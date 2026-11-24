// Package meshstate reads hold records and live NetBird state
// (peers + setup keys) to assemble the Access view for one
// member, as rendered on the console's user-detail page.
package meshstate

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// HoldRecord is one line of the hold-records JSONL file
// (ops/lifecycle/schema/hold-record.v1.schema.json).
type HoldRecord struct {
	Version  int     `json:"v"`
	Lane     string  `json:"lane"`
	Site     string  `json:"site"`
	PeerID   string  `json:"peer_id"`
	PeerName string  `json:"peer_name"`
	Handle   *string `json:"handle"`
	// EnteredAt is when the hold was placed, UTC.
	EnteredAt   time.Time `json:"entered_at"`
	HoldGroup   string    `json:"hold_group"`
	PriorGroups []string  `json:"prior_groups"`
	Actor       string    `json:"actor"`
	Reason      string    `json:"reason,omitempty"`
	// Phase is "releasing" while a release of this record is in
	// progress (the releaser rewrites the line before it touches any
	// group). A reader treats such a record as a release to finish,
	// never as a hold. Absent = held.
	Phase string `json:"phase,omitempty"`
}

// Records is the parsed hold-record set plus duplicate flags and
// the count of lines the loader skipped.
type Records struct {
	All []HoldRecord
	// DuplicatePeerIDs marks peer_ids that appeared on more than
	// one line. The resolver treats these as inconsistent.
	DuplicatePeerIDs map[string]bool
	// Skipped counts bad lines the loader passed over; SkippedLines
	// names them, 1-based, for the operator. What a bad line is is
	// defined once, in ops/lifecycle/README.md ("A bad line is
	// skipped, never fatal, and always counted"), and implemented
	// twice: here (parseHoldRecordLine) and in the sweep
	// (_is_bad_hold_record in ops/scripts/lifecycle-sweep.py). The
	// shared fixture ops/lifecycle/fixtures/hold-records-bad-lines.jsonl
	// pins both readers to the same count. One bad line degrades that
	// one peer to "no record" (a security hold until repaired), never
	// the whole file.
	Skipped      int
	SkippedLines []int
}

// holdRecordRequiredFields mirrors "required" in
// ops/lifecycle/schema/hold-record.v1.schema.json; a test pins the two
// together. A line missing any of them is a bad line.
var holdRecordRequiredFields = []string{
	"v", "lane", "site", "peer_id", "peer_name", "handle",
	"entered_at", "hold_group", "prior_groups", "actor",
}

// maxHoldRecordLine bounds one line of the file. A real record is a few
// hundred bytes; a line past this bound is ONE bad line (skipped and
// counted like any other), never a read error that blanks the section
// for every member.
const maxHoldRecordLine = 1 << 20 // 1 MiB

// LoadHoldRecords parses a hold-record JSONL file. A bad line (see
// parseHoldRecordLine) is skipped and counted in Skipped / SkippedLines;
// it never fails the load. Blank lines are ignored and not counted.
// Only an unreadable file returns an error. Duplicate peer_id lines
// flag a per-peer DuplicatePeerIDs entry but keep every line.
func LoadHoldRecords(path string) (Records, error) {
	var recs Records
	recs.DuplicatePeerIDs = make(map[string]bool)

	f, err := os.Open(path)
	if err != nil {
		// Callers prefix the path themselves; an *os.PathError repeats it,
		// so strip to the operation and cause. errors.Is(err, fs.ErrNotExist)
		// still holds through the %w.
		var pe *os.PathError
		if errors.As(err, &pe) {
			return recs, fmt.Errorf("hold-records: %s: %w", pe.Op, pe.Err)
		}
		return recs, err
	}
	defer f.Close()

	rd := bufio.NewReaderSize(f, 64<<10)
	line := 0
	seen := make(map[string]bool)
	for {
		raw, tooLong, err := readBoundedLine(rd, maxHoldRecordLine)
		if err != nil && !errors.Is(err, io.EOF) {
			return recs, fmt.Errorf("hold-records: read: %w", err)
		}
		if raw == "" && !tooLong && errors.Is(err, io.EOF) {
			break
		}
		line++
		if !tooLong && strings.TrimSpace(raw) == "" {
			if errors.Is(err, io.EOF) {
				break
			}
			continue
		}
		var (
			r  HoldRecord
			ok bool
		)
		if !tooLong {
			r, ok = parseHoldRecordLine(raw)
		}
		if !ok {
			recs.Skipped++
			recs.SkippedLines = append(recs.SkippedLines, line)
		} else {
			if seen[r.PeerID] {
				recs.DuplicatePeerIDs[r.PeerID] = true
			} else {
				seen[r.PeerID] = true
			}
			recs.All = append(recs.All, r)
		}
		if errors.Is(err, io.EOF) {
			break
		}
	}
	return recs, nil
}

// readBoundedLine reads one line without its terminator. When the line
// exceeds limit bytes the rest of it is drained and tooLong is true;
// the returned text is then empty. err is io.EOF on the last line
// (which may still carry text) and nil otherwise.
func readBoundedLine(rd *bufio.Reader, limit int) (text string, tooLong bool, err error) {
	var buf []byte
	for {
		frag, isPrefix, rerr := rd.ReadLine()
		if rerr != nil {
			if errors.Is(rerr, io.EOF) && (len(buf) > 0 || tooLong) {
				return string(buf), tooLong, io.EOF
			}
			return "", tooLong, rerr
		}
		if !tooLong {
			if len(buf)+len(frag) > limit {
				tooLong = true
				buf = nil
			} else {
				buf = append(buf, frag...)
			}
		}
		if !isPrefix {
			return string(buf), tooLong, nil
		}
	}
}

// parseHoldRecordLine reports whether raw is a v1 hold record under the
// contract both readers implement (ops/lifecycle/README.md, and
// _is_bad_hold_record in ops/scripts/lifecycle-sweep.py). A line is bad
// when it is not a JSON object, is not v: 1, lacks a required v1 field,
// has an empty peer_id, carries a wrong-typed field, names a lane
// outside the enum, names a lane that disagrees with its hold_group
// (dormancy ⇒ <SITE>-PROBATION-<role>, security ⇒ <SITE>-QUARANTINED),
// or carries a phase other than "releasing". Presence of required keys
// is checked on the raw object because a struct decode cannot tell an
// absent field from its zero value (entered_at absent would otherwise
// read as year 0001).
func parseHoldRecordLine(raw string) (HoldRecord, bool) {
	var obj map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &obj); err != nil || obj == nil {
		return HoldRecord{}, false
	}
	for _, k := range holdRecordRequiredFields {
		if _, present := obj[k]; !present {
			return HoldRecord{}, false
		}
	}
	var r HoldRecord
	if err := json.Unmarshal([]byte(raw), &r); err != nil {
		return HoldRecord{}, false
	}
	if r.Version != 1 || strings.TrimSpace(r.PeerID) == "" || r.EnteredAt.IsZero() {
		return HoldRecord{}, false
	}
	if r.PriorGroups == nil {
		// "prior_groups": null decodes to a nil slice; the schema says
		// array, and the sweep rejects a non-list.
		return HoldRecord{}, false
	}
	hg := strings.ToUpper(strings.TrimSpace(r.HoldGroup))
	switch r.Lane {
	case "dormancy":
		if !strings.Contains(hg, "-PROBATION-") {
			return HoldRecord{}, false
		}
	case "security":
		if !strings.HasSuffix(hg, "-QUARANTINED") {
			return HoldRecord{}, false
		}
	default:
		return HoldRecord{}, false
	}
	if _, present := obj["phase"]; present && r.Phase != "releasing" {
		return HoldRecord{}, false
	}
	return r, true
}

// Group is the subset of a NetBird group shape this surface needs.
type Group struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

// Peer is the subset of NetBird's peer shape this surface reads.
// It matches the ops/smoketest/internal/netbird Peer subset the
// repository already consumes elsewhere.
type Peer struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Connected bool   `json:"connected"`
	// IP is the peer's mesh address, the join key to the host's own
	// per-peer handshake view (handshakes.go). The probation ForwardAuth
	// seam uses an exact parsed-address match against Traefik's connection-
	// derived client address; names and caller-supplied identity headers are
	// never identity inputs there.
	IP       string    `json:"ip"`
	LastSeen time.Time `json:"last_seen"`
	// LastSeenUnreadable distinguishes a timestamp the API supplied but
	// that could not be parsed from a peer the API simply has never seen.
	// The peer and its security-relevant group membership remain usable.
	LastSeenUnreadable bool    `json:"-"`
	Groups             []Group `json:"groups"`
	// OS is NetBird's free-text operating system string for the peer, e.g.
	// "Oracle Linux Server 8.10", "Microsoft Windows 11 Pro", "Darwin".
	OS string `json:"os"`
}

// SetupKey is the subset of NetBird's setup-key shape this surface
// reads to assess validity and usage. The plaintext key value never
// appears here.
type SetupKey struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	UsedTimes int    `json:"used_times"`
	Revoked   bool   `json:"revoked"`
	Valid     bool   `json:"valid"`
	// Expires is the expiry timestamp, when present. Zero time
	// means "no explicit expiry recorded".
	Expires time.Time `json:"-"`
	// ExpiresUnreadable is true when the API sent an expiry the
	// client could not parse. The key's validity is then unknown:
	// it must never read as valid by inference.
	ExpiresUnreadable bool   `json:"-"`
	State             string `json:"state"`
}

// Client talks to the NetBird management HTTP API for this surface.
// MgmtURL is the full base URL (e.g. https://netbird.example.net),
// Token is the NetBird PAT, and HTTP is the client to use.
type Client struct {
	MgmtURL string
	Token   string
	HTTP    *http.Client
}

// httpClient returns the client to use with redirects refused. The
// request carries the NetBird PAT, and Go keeps Authorization on a
// redirect to the same host or a subdomain, https to http included; a
// redirect is returned as its own 3xx response, which Peers and
// SetupKeys report as an error.
func (c *Client) httpClient() *http.Client {
	cl := http.Client{}
	if c.HTTP != nil {
		cl = *c.HTTP
	}
	cl.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	return &cl
}

// HTTPStatusError records a failed response from NetBird without reading its body.
type HTTPStatusError struct {
	Status int
}

func (e *HTTPStatusError) Error() string {
	return fmt.Sprintf("NetBird returned HTTP %d", e.Status)
}

// Peers lists all peers from GET /api/peers with a strict 5 s
// per-call timeout. Non-200 responses are returned as errors that
// carry the status code; no partial values are returned.
func (c *Client) Peers(ctx context.Context) ([]Peer, error) {
	if strings.TrimSpace(c.MgmtURL) == "" {
		return nil, errors.New("NetBird mgmt URL is empty")
	}
	if strings.TrimSpace(c.Token) == "" {
		return nil, errors.New("NetBird token is empty")
	}
	u := strings.TrimSuffix(c.MgmtURL, "/") + "/api/peers"
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Token "+c.Token)
	req.Header.Set("Accept", "application/json")

	resp, err := c.httpClient().Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, &HTTPStatusError{Status: resp.StatusCode}
	}
	var raw []struct {
		ID        string          `json:"id"`
		Name      string          `json:"name"`
		Connected bool            `json:"connected"`
		IP        string          `json:"ip"`
		LastSeen  json.RawMessage `json:"last_seen"`
		Groups    []Group         `json:"groups"`
		OS        string          `json:"os"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return nil, fmt.Errorf("decode peers: %w", err)
	}
	peers := make([]Peer, 0, len(raw))
	for _, r := range raw {
		p := Peer{
			ID:        r.ID,
			Name:      r.Name,
			Connected: r.Connected,
			IP:        r.IP,
			Groups:    r.Groups,
			OS:        r.OS,
		}
		lastSeenJSON := bytes.TrimSpace(r.LastSeen)
		if len(lastSeenJSON) > 0 && !bytes.Equal(lastSeenJSON, []byte("null")) {
			var lastSeen string
			if err := json.Unmarshal(lastSeenJSON, &lastSeen); err != nil {
				p.LastSeenUnreadable = true
			} else if t, err := time.Parse(time.RFC3339, lastSeen); err == nil {
				p.LastSeen = t
			} else {
				p.LastSeenUnreadable = true
			}
		}
		peers = append(peers, p)
	}
	return peers, nil
}

// Groups lists the live group vocabulary. Release preview uses this to refuse
// a deleted recorded destination before anything is queued for mutation.
func (c *Client) Groups(ctx context.Context) ([]Group, error) {
	if strings.TrimSpace(c.MgmtURL) == "" {
		return nil, errors.New("NetBird mgmt URL is empty")
	}
	if strings.TrimSpace(c.Token) == "" {
		return nil, errors.New("NetBird token is empty")
	}
	u := strings.TrimSuffix(c.MgmtURL, "/") + "/api/groups"
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Token "+c.Token)
	req.Header.Set("Accept", "application/json")
	resp, err := c.httpClient().Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, &HTTPStatusError{Status: resp.StatusCode}
	}
	var groups []Group
	if err := json.NewDecoder(resp.Body).Decode(&groups); err != nil {
		return nil, fmt.Errorf("decode groups: %w", err)
	}
	return groups, nil
}

// SetupKeys lists all setup keys from GET /api/setup-keys with a
// strict 5 s per-call timeout. Non-200 responses are returned as
// errors that carry the status code; no partial values are returned.
func (c *Client) SetupKeys(ctx context.Context) ([]SetupKey, error) {
	if strings.TrimSpace(c.MgmtURL) == "" {
		return nil, errors.New("NetBird mgmt URL is empty")
	}
	if strings.TrimSpace(c.Token) == "" {
		return nil, errors.New("NetBird token is empty")
	}
	u := strings.TrimSuffix(c.MgmtURL, "/") + "/api/setup-keys"
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Token "+c.Token)
	req.Header.Set("Accept", "application/json")

	resp, err := c.httpClient().Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, &HTTPStatusError{Status: resp.StatusCode}
	}

	var raw []struct {
		ID        string `json:"id"`
		Name      string `json:"name"`
		UsedTimes int    `json:"used_times"`
		Revoked   bool   `json:"revoked"`
		Valid     bool   `json:"valid"`
		Expires   string `json:"expires"`
		State     string `json:"state"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return nil, fmt.Errorf("decode setup keys: %w", err)
	}

	out := make([]SetupKey, 0, len(raw))
	for _, r := range raw {
		sk := SetupKey{
			ID:        r.ID,
			Name:      r.Name,
			UsedTimes: r.UsedTimes,
			Revoked:   r.Revoked,
			Valid:     r.Valid,
			State:     r.State,
		}
		if strings.TrimSpace(r.Expires) != "" {
			if t, err := time.Parse(time.RFC3339, r.Expires); err == nil {
				sk.Expires = t
			} else {
				sk.ExpiresUnreadable = true
			}
		}
		out = append(out, sk)
	}
	return out, nil
}

// OwnsName reports whether handle is the right owner of an
// artifact called name, given every handle the console knows
// about.
//
// WHY THIS IS NOT A PREFIX TEST. The obvious rule —
// does the name start with `peer-<handle>-` — credits the WRONG
// PERSON whenever one handle extends another by a hyphenated
// segment. `peer-dave-2-acme-site-2026-08-04` starts with
// `peer-dave-`, so `dave` would inherit `dave-2`'s used key and
// be marked enrolled having never enrolled.
//
// The name alone cannot settle it: the grammar is
// `peer-<handle>[-<org>]-<goatnet>-YYYY-MM-DD[-device]` and an org
// tag sits between the handle and the goatnet, so there is no
// fixed segment count to anchor on and `2` is indistinguishable
// from an org from the string alone.
//
// What DOES settle it is the roster: among all known handles that
// prefix this name, the LONGEST is the owner. `dave-2` beats
// `dave`, and a name nobody claims is credited to nobody.
func OwnsName(name, handle string, known map[string]struct{}) bool {
	if !strings.HasPrefix(name, "peer-"+handle+"-") {
		return false
	}
	for k := range known {
		if len(k) > len(handle) && strings.HasPrefix(name, "peer-"+k+"-") {
			return false // a longer known handle claims this name
		}
	}
	return true
}

// KnownHandles is every handle in the onboarding store, used only
// to disambiguate name ownership. On a store error it returns the
// empty set AND the error: the caller must surface it, because an
// empty set is weaker in the dangerous direction — fewer known
// handles means fewer names are contested, so an artifact can be
// credited to a handle that merely prefixes it.
func KnownHandles(ctx context.Context, store *onboardstore.Store, log *slog.Logger) (map[string]struct{}, error) {
	out := map[string]struct{}{}
	if store == nil {
		return out, errors.New("onboarding store is not configured")
	}
	if log == nil {
		log = slog.Default()
	}
	all, err := store.ListProposals(ctx, "")
	if err != nil {
		log.Warn("meshstate: could not list all handles; name ownership is disambiguated against a partial set", slog.Any("error", err))
		return out, fmt.Errorf("list handles: %w", err)
	}
	for _, p := range all {
		if p.Handle != "" {
			out[p.Handle] = struct{}{}
		}
	}
	return out, nil
}

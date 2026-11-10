// Package netbirdkeys mints NetBird setup keys whose auto_groups are DERIVED
// from a machine's node-class, rather than typed by whoever is at the keyboard.
//
// ADR 1116 §9: "the setup key is the enforcement point, not this document."
// Class is assigned when a key is minted — `auto_groups` decides it, and a wrong
// choice persists until somebody reads membership. A taxonomy with nothing that
// fits and a mint path with no opinion puts every kind of machine into the same
// group.
//
// Separate package from netbirdpoll deliberately. That one polls; this one
// WRITES to the mesh control plane, and the two deserve to be distinguishable in
// an import list.
package netbirdkeys

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"
	"time"
)

var (
	// ErrDisabled means the client has no mgmt URL, token or site prefix. It is
	// returned rather than silently skipped: a caller that asked for a mesh key
	// and got nothing must find out, not proceed with an unenrolled machine.
	ErrDisabled = errors.New("netbird setup-key minting is not configured")
	// ErrGroupMissing means a derived group suffix has no group on the live
	// mgmt server. Fail-closed: minting a key with the OTHER groups would
	// quietly under-enroll the machine, and an under-enrolled peer looks
	// configured while reaching nothing.
	ErrGroupMissing = errors.New("derived group does not exist on the mgmt server")
)

// Client talks to one goatnet's netbird-mgmt API.
type Client struct {
	mgmtURL    string
	token      string
	sitePrefix string // e.g. "SITE-PROD"; group name = <prefix>-<suffix>
	http       *http.Client
}

// New returns a Client. sitePrefix is the goatnet's group-name prefix, the same
// one ops/ansible/netbird_acl_taxonomy.yml declares; without it a suffix cannot
// be resolved to a real group and the client reports itself disabled.
func New(mgmtURL, token, sitePrefix string) *Client {
	return &Client{
		mgmtURL:    strings.TrimRight(mgmtURL, "/"),
		token:      token,
		sitePrefix: strings.TrimRight(strings.TrimSpace(sitePrefix), "-"),
		http:       &http.Client{Timeout: 15 * time.Second},
	}
}

// Enabled reports whether the client has everything it needs to mint.
func (c *Client) Enabled() bool {
	return c != nil && c.mgmtURL != "" && c.token != "" && c.sitePrefix != ""
}

// SitePrefix exposes the configured prefix so callers can name the full group
// in operator-facing output without re-deriving it.
func (c *Client) SitePrefix() string {
	if c == nil {
		return ""
	}
	return c.sitePrefix
}

// GroupName renders the full mgmt-server group name for a suffix under the
// goatnet's own site prefix.
func (c *Client) GroupName(suffix string) string {
	return c.GroupNameScoped("", suffix)
}

// GroupNameScoped renders the full group name for a suffix under an explicit
// scope prefix — an exercise's EX-* prefix (ADR 1116; the taxonomy's
// netbird_exercises are synthetic sites on this goatnet's mgmt server). An
// empty prefix means the goatnet's own site prefix.
func (c *Client) GroupNameScoped(prefix, suffix string) string {
	if prefix == "" {
		if c == nil || c.sitePrefix == "" {
			return suffix
		}
		prefix = c.sitePrefix
	}
	return prefix + "-" + suffix
}

type group struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

// resolveGroupIDs maps suffixes (under the given scope prefix; "" = the site
// prefix) to live group UUIDs. Every suffix must resolve; a partial result is
// an error, never a smaller grant.
func (c *Client) resolveGroupIDs(ctx context.Context, prefix string, suffixes []string) ([]string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.mgmtURL+"/api/groups", nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Token "+c.token)
	req.Header.Set("Accept", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("netbird mgmt GET /api/groups: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("netbird mgmt GET /api/groups returned %d: %s", resp.StatusCode, truncate(string(body), 200))
	}
	var groups []group
	if err := json.Unmarshal(body, &groups); err != nil {
		return nil, fmt.Errorf("decode /api/groups: %w", err)
	}
	byName := make(map[string]string, len(groups))
	for _, g := range groups {
		byName[g.Name] = g.ID
	}

	ids := make([]string, 0, len(suffixes))
	var missing []string
	for _, s := range suffixes {
		name := c.GroupNameScoped(prefix, s)
		id, ok := byName[name]
		if !ok {
			missing = append(missing, name)
			continue
		}
		ids = append(ids, id)
	}
	if len(missing) > 0 {
		sort.Strings(missing)
		// Name BOTH halves. Telling the operator only to run the bootstrap is
		// half an answer that reads as a whole one: the bootstrap CREATES the
		// groups and creates no policy that USES them, so a site with groups
		// and no matrix entry still runs the Default All→All and the gear
		// lands wearing correct-looking labels that carry no reach of their
		// own. A custodian who follows a one-line remediation to the letter and still
		// gets a flat mesh has been mis-directed by the error, not by their own care.
		return nil, fmt.Errorf("%w: %s — the site's gear groups do not exist on this mgmt server. "+
			"Fix BOTH halves or the enrollment is only half-configured: (1) create the groups — "+
			"`netbird-acl-bootstrap.yml -e target_site=<site>`; (2) make sure the site has an entry in "+
			"ops/ansible/netbird_acl_matrix.yml and that its permits have been applied, or the groups "+
			"exist while the mesh still runs the Default All→All and the class you just chose grants "+
			"nothing. See docs/operations/gear-enrollment-sop.md",
			ErrGroupMissing, strings.Join(missing, ", "))
	}
	return ids, nil
}

// SetupKey is what a machine needs to join the mesh, plus the record of what the
// key was scoped to. Groups carries the FULL group names so audit and
// operator-facing output never have to re-apply the prefix.
type SetupKey struct {
	ID        string   `json:"id"`
	Key       string   `json:"key"`
	Name      string   `json:"name"`
	Groups    []string `json:"groups"`
	ExpiresIn int      `json:"expires_in_seconds"`
}

type createSetupKeyRequest struct {
	Name       string   `json:"name"`
	Type       string   `json:"type"`
	ExpiresIn  int      `json:"expires_in"`
	AutoGroups []string `json:"auto_groups"`
	UsageLimit int      `json:"usage_limit"`
	Ephemeral  bool     `json:"ephemeral"`
	// AutoPeerName is the label prefix the patched goat mgmt image combines
	// with the joining peer's hostname as "<auto_peer_name>-<hostname>". Send
	// it or the peer registers under its bare hostname, and a bare hostname is
	// not a name anyone can trace back to a person or a machine record.
	//
	// Without this field every machine enrolled through this path lands as
	// `<hostname>` — invisible to the
	// revoke, which selects peers by matching `^peer-<handle>-<site>-`
	// (operator-onboard.yml, Phase R1.2). A cut that cannot see a peer does
	// not report failing to cut it; it reports success. See F-345 and
	// ops/scripts/peer-attribution.py, which recovers the link from the
	// management server's own enrolment events when the name is already lost.
	//
	// Upstream NetBird ignores unknown fields, so sending this against an
	// unpatched mgmt server is inert rather than an error.
	AutoPeerName string `json:"auto_peer_name"`
}

type createSetupKeyResponse struct {
	ID  string `json:"id"`
	Key string `json:"key"`
}

// MintForGroups creates a ONE-OFF, single-use setup key carrying exactly the
// groups derived from the machine's node-class.
//
// One-off and usage_limit 1 on purpose. A machine key is minted for one machine
// at a mint event we have already recorded a custody edge for; a reusable key is
// a standing invitation whose membership nobody re-reads, which is the shape
// that let three unlike things land in one group in a single sitting.
func (c *Client) MintForGroups(ctx context.Context, name string, groupSuffixes []string, expiresIn time.Duration) (*SetupKey, error) {
	return c.MintScoped(ctx, name, "", groupSuffixes, expiresIn)
}

// MintScoped is MintForGroups under an explicit scope prefix — the door for
// exercise-scoped gear (EX-<name>-<class> groups on this goatnet's mgmt
// server). An empty prefix means the goatnet's own site prefix. Fail-closed
// exactly like the site scope: a prefixed group missing from the live mgmt
// server refuses the whole key, which is also what enforces "the exercise has
// declared its gear groups" without a second registry of exercises here.
func (c *Client) MintScoped(ctx context.Context, name, prefix string, groupSuffixes []string, expiresIn time.Duration) (*SetupKey, error) {
	if !c.Enabled() {
		return nil, ErrDisabled
	}
	if len(groupSuffixes) == 0 {
		// Belt and braces: MeshGroupsForNodeClass already refuses an unmapped
		// class, but a key with no groups is a peer that enrols into nothing and
		// reads as configured, so refuse it here too.
		return nil, errors.New("refusing to mint a setup key with no auto_groups")
	}
	ids, err := c.resolveGroupIDs(ctx, prefix, groupSuffixes)
	if err != nil {
		return nil, err
	}

	payload := createSetupKeyRequest{
		Name:       name,
		Type:       "one-off",
		ExpiresIn:  int(expiresIn.Seconds()),
		AutoGroups: ids,
		UsageLimit: 1,
		Ephemeral:  false,
		// The key name IS the attribution: callers build it from the machine's
		// registry anchor, so reusing it as the peer-name prefix makes the peer
		// traceable to the same record without a second naming scheme to keep
		// in sync.
		AutoPeerName: name,
	}
	buf, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.mgmtURL+"/api/setup-keys", bytes.NewReader(buf))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Token "+c.token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("netbird mgmt POST /api/setup-keys: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		return nil, fmt.Errorf("netbird mgmt POST /api/setup-keys returned %d: %s", resp.StatusCode, truncate(string(body), 200))
	}
	var out createSetupKeyResponse
	if err := json.Unmarshal(body, &out); err != nil {
		return nil, fmt.Errorf("decode /api/setup-keys: %w", err)
	}
	if strings.TrimSpace(out.Key) == "" {
		return nil, errors.New("netbird returned an empty setup key")
	}

	full := make([]string, len(groupSuffixes))
	for i, s := range groupSuffixes {
		full[i] = c.GroupNameScoped(prefix, s)
	}
	return &SetupKey{
		ID:        out.ID,
		Key:       out.Key,
		Name:      name,
		Groups:    full,
		ExpiresIn: payload.ExpiresIn,
	}, nil
}

// --- the teardown half -------------------------------------------------
//
// Minting a machine onto the mesh has an inverse: without it nothing could
// retire a machine.
//
// ADR 1031 puts the authoritative cut at NetBird, not at the Zenoh ACL — in
// per-uuid slot-pool mode the router bakes a grant for every slot
// unconditionally, so mesh removal is the ONLY authorization barrier a revoke
// actually moves. That makes these two calls the load-bearing part of a
// machine revoke, and the registry teardown the bookkeeping.

// MeshCut records what a machine revoke actually removed from the mgmt server.
// Every field is a list of names/ids rather than a count, because "we cut
// something" is not a claim an operator can check and "we deleted peer X" is.
type MeshCut struct {
	// PeersDeleted are the peer ids removed (peer name == machine handle).
	PeersDeleted []string
	// KeysDeleted are the ids of UNCONSUMED setup keys removed.
	KeysDeleted []string
	// KeysAlreadySpent are the ids of keys found with used_times > 0. Left
	// alone deliberately: a spent one-off cannot admit anything else, and
	// deleting it would erase the record of how the peer got there.
	KeysAlreadySpent []string
}

type peerRef struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type setupKeyRef struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	UsedTimes int    `json:"used_times"`
	Revoked   bool   `json:"revoked"`
}

// CutMachine performs the mesh half of a machine revoke: it deletes any
// UNCONSUMED setup key named keyName, then deletes every peer BELONGING TO
// peerName — which is not the same as named peerName, and that difference is
// the whole of F-400.
//
// `auto_peer_name` is a PREFIX: the patched mgmt image renders the joining
// peer as "<auto_peer_name>-<hostname>" (see the field's doc above). So a
// machine minted as "machine-<handle>" NEVER appears as "<handle>", and an
// equality match here found nothing, deleted the key, and reported a clean
// cut while the peer kept its groups and its reach. It matched only a peer a
// human had renamed by hand, which is not a contract.
//
// Matching the documented shape instead is what the PERSON path already does
// — check-node-class-membership.sh recognises people by the prefix regex
// ^peer-<handle>-<env>-<date>-, never by equality. This is the machine half
// of the same rule.
//
// Order is load-bearing. The key goes first so the door is shut before the
// occupant is evicted; deleting the peer first leaves a live single-use key
// that a box could re-join on in the gap. It is also why an unconsumed key is
// deleted at all rather than left to expire: a standing invitation for a
// retired machine is the exact shape ADR 1116 §9 exists to eliminate, and its
// TTL is a 30-day join window (D4), not a trust window.
//
// A machine with no peer and no key is the normal case for a revoke that
// follows a never-joined mint — it returns an empty MeshCut and no error, not
// a not-found.
func (c *Client) CutMachine(ctx context.Context, keyName, peerName string) (*MeshCut, error) {
	if !c.Enabled() {
		return nil, ErrDisabled
	}
	cut := &MeshCut{}

	var keys []setupKeyRef
	if err := c.getJSON(ctx, "/api/setup-keys", &keys); err != nil {
		return nil, err
	}
	for _, k := range keys {
		if k.Name != keyName {
			continue
		}
		if k.UsedTimes > 0 {
			cut.KeysAlreadySpent = append(cut.KeysAlreadySpent, k.ID)
			continue
		}
		if err := c.delete(ctx, "/api/setup-keys/"+k.ID); err != nil {
			return nil, err
		}
		cut.KeysDeleted = append(cut.KeysDeleted, k.ID)
	}

	var peers []peerRef
	if err := c.getJSON(ctx, "/api/peers", &peers); err != nil {
		return nil, err
	}
	for _, p := range peers {
		if !ownedPeerName(p.Name, keyName, peerName) {
			continue
		}
		if err := c.delete(ctx, "/api/peers/"+p.ID); err != nil {
			return nil, err
		}
		cut.PeersDeleted = append(cut.PeersDeleted, p.ID)
	}
	return cut, nil
}

// ownedPeerName reports whether a live peer name belongs to the machine whose
// peer-name base is base. Two shapes are accepted and no others:
//
//	handle               the machine was renamed to its bare handle by hand,
//	                     the documented F-400 workaround. Kept so a cut still
//	                     finds every peer renamed before this fix landed.
//	keyName              auto_peer_name with no hostname appended.
//	keyName + "-" + host the shape the mgmt image actually produces.
//
// The trailing hyphen is load-bearing: without it keyName "machine-gw-1"
// would also claim "machine-gw-11-host", cutting a different machine.
func ownedPeerName(name, keyName, handle string) bool {
	return name == handle || name == keyName || strings.HasPrefix(name, keyName+"-")
}

func (c *Client) getJSON(ctx context.Context, path string, out any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.mgmtURL+path, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Token "+c.token)
	req.Header.Set("Accept", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("netbird mgmt GET %s: %w", path, err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<22))
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("netbird mgmt GET %s returned %d: %s", path, resp.StatusCode, truncate(string(body), 200))
	}
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("decode %s: %w", path, err)
	}
	return nil
}

// delete treats 404 as success: the caller's goal is that the object is gone,
// and a concurrent delete having already achieved that is not a failure.
func (c *Client) delete(ctx context.Context, path string) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodDelete, c.mgmtURL+path, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Token "+c.token)
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("netbird mgmt DELETE %s: %w", path, err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<16))
	switch resp.StatusCode {
	case http.StatusOK, http.StatusNoContent, http.StatusAccepted, http.StatusNotFound:
		return nil
	}
	return fmt.Errorf("netbird mgmt DELETE %s returned %d: %s", path, resp.StatusCode, truncate(string(body), 200))
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "…"
}

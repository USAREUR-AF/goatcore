// Package audittail polls the Block 82S audit-subscriber's MinIO
// bucket and drives L3/L4 of the vendor-progress swimlane (Block 88).
//
// One mechanism, two purposes:
//   - First boot: cursor is empty → ListObjectsV2 walks every existing
//     NDJSON from the start of the bucket. That IS the "MinIO replay"
//     half of the design.
//   - Steady state: cursor advances as new audit-subscriber batches
//     land; only new objects are pulled.
//
// The audit-subscriber writes objects under key shape
// `{audit_subscriber_id}/{YYYY}/{MM}/{DD}/{HH}/{ms}-{n}.ndjson`, which
// is lex-monotonic in time; ListObjectsV2's StartAfter consumes it
// cleanly so we don't need a separate cursor format.
//
// Auth: minimal AWS SigV4 implementation (stdlib only) targeting MinIO.
// We do not pull aws-sdk-go-v2 because the surface we need is two
// requests (ListObjectsV2 + GetObject) and adding a transitive go.mod
// dependency this big is disproportionate. If we ever need richer S3,
// swap in aws-sdk-go-v2 then.
package audittail

import (
	"bufio"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Config holds the MinIO/S3 endpoint + credentials.
type Config struct {
	Endpoint     string // e.g. "http://minio.goat.example:9000"
	Bucket       string // e.g. "audit-integrity"
	Region       string // e.g. "us-east-1"
	AccessKey    string
	SecretKey    string
	SubscriberID string // e.g. "site-A" — also the top-level object-key prefix
	// PollInterval defaults to 30s when zero.
	PollInterval time.Duration
}

// Tailer pulls NDJSON from MinIO and feeds events into the registry.
type Tailer struct {
	cfg Config
	reg *registry.Registry
	cli *http.Client
}

// New builds a Tailer. Caller should call Enabled() before scheduling
// it.
func New(cfg Config, reg *registry.Registry) *Tailer {
	if cfg.Region == "" {
		cfg.Region = "us-east-1"
	}
	if cfg.SubscriberID == "" {
		cfg.SubscriberID = "site-A"
	}
	if cfg.PollInterval <= 0 {
		cfg.PollInterval = 30 * time.Second
	}
	return &Tailer{
		cfg: cfg,
		reg: reg,
		cli: &http.Client{Timeout: 60 * time.Second},
	}
}

// Enabled is true when the tailer has the credentials it needs.
func (t *Tailer) Enabled() bool {
	return t != nil && t.cfg.Endpoint != "" && t.cfg.Bucket != "" && t.cfg.AccessKey != "" && t.cfg.SecretKey != ""
}

// Run launches the tail loop in a goroutine. Exits when ctx is canceled.
// Errors log at Warn — degradation is graceful (L3/L4 columns show "—"
// for everyone until MinIO is reachable again).
func (t *Tailer) Run(ctx context.Context) {
	if !t.Enabled() {
		slog.Info("audit tailer disabled (no S3 endpoint or credentials); L3/L4 columns will show '—'")
		return
	}
	go func() {
		// First tick on a short delay so startup logs settle.
		first := time.NewTimer(8 * time.Second)
		select {
		case <-ctx.Done():
			first.Stop()
			return
		case <-first.C:
		}
		t.tick(ctx)

		ticker := time.NewTicker(t.cfg.PollInterval)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				t.tick(ctx)
			}
		}
	}()
}

func (t *Tailer) tick(ctx context.Context) {
	tickCtx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	n, lastKey, err := t.PullOnce(tickCtx)
	if err != nil {
		slog.Warn("audit tail failed", "err", err, "last_key", lastKey)
		return
	}
	slog.Debug("audit tail ok", "events", n, "last_key", lastKey)
}

// PullOnce reads from the cursor forward and processes every NDJSON
// object found. Returns (event_count, new_cursor, err). Each object is
// applied atomically WITH its cursor advance (one transaction per
// object, registry.ApplyAuditObject), so replay is exactly-once at
// object grain: a failed object rolls back whole, cursor included, and
// the next tick re-applies it from scratch without double-counting.
func (t *Tailer) PullOnce(ctx context.Context) (int, string, error) {
	cursor, err := t.reg.GetAuditCursor(ctx, t.cfg.SubscriberID)
	if err != nil {
		return 0, "", fmt.Errorf("get audit cursor: %w", err)
	}

	// Build prefix + StartAfter.
	prefix := t.cfg.SubscriberID + "/"
	startAfter := cursor.LastKey

	// Snapshot the current vendor set (prefix → synthetic) once per
	// pull so we don't re-query for every event.
	vendors, err := t.reg.ListVendors(ctx)
	if err != nil {
		return 0, "", fmt.Errorf("list vendors: %w", err)
	}
	vSet := make(map[string]bool, len(vendors))
	for _, v := range vendors {
		// Skip synthetic — the progress page deliberately filters them.
		if v.Synthetic {
			continue
		}
		vSet[v.Prefix] = true
	}

	totalEvents := 0
	keys, err := t.listObjects(ctx, prefix, startAfter)
	if err != nil {
		return 0, startAfter, fmt.Errorf("list objects: %w", err)
	}
	for _, key := range keys {
		n, err := t.processObject(ctx, key, vSet)
		if err != nil {
			// Every completed object already committed its own cursor
			// advance, so a partial pull resumes right after the last
			// object that applied; the failed object replays whole on
			// the next tick.
			return totalEvents, startAfter, fmt.Errorf("process %s: %w", key, err)
		}
		totalEvents += n
		startAfter = key
	}
	return totalEvents, startAfter, nil
}

// listObjects pages ListObjectsV2 and returns every key (in order)
// matching prefix and strictly after StartAfter. Caps per-pull at
// listCap to bound work; subsequent ticks pick up the rest.
const listCap = 2000

func (t *Tailer) listObjects(ctx context.Context, prefix, startAfter string) ([]string, error) {
	return listObjectKeys(ctx, t.cli, t.cfg, prefix, startAfter, listCap)
}

// listObjectKeys pages ListObjectsV2 and returns every `.ndjson` key (in
// order) matching prefix and strictly after startAfter, capped at maxKeys.
// Parameterized by Config + client so the Tailer and the Presigner share it.
func listObjectKeys(ctx context.Context, cli *http.Client, cfg Config, prefix, startAfter string, maxKeys int) ([]string, error) {
	var (
		out               []string
		continuationToken string
	)
	for len(out) < maxKeys {
		q := url.Values{}
		q.Set("list-type", "2")
		q.Set("prefix", prefix)
		q.Set("max-keys", "1000")
		if startAfter != "" && continuationToken == "" {
			q.Set("start-after", startAfter)
		}
		if continuationToken != "" {
			q.Set("continuation-token", continuationToken)
		}
		reqURL := fmt.Sprintf("%s/%s?%s", strings.TrimRight(cfg.Endpoint, "/"), cfg.Bucket, q.Encode())
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, reqURL, nil)
		if err != nil {
			return out, err
		}
		if err := signV4Request(cfg, req, nil); err != nil {
			return out, err
		}
		resp, err := cli.Do(req)
		if err != nil {
			return out, err
		}
		body, err := io.ReadAll(io.LimitReader(resp.Body, 8*1024*1024))
		_ = resp.Body.Close()
		if err != nil {
			return out, err
		}
		if resp.StatusCode >= 400 {
			return out, fmt.Errorf("ListObjectsV2 returned %d: %s", resp.StatusCode, truncate(string(body), 200))
		}
		var parsed listV2Resp
		if err := xml.Unmarshal(body, &parsed); err != nil {
			return out, fmt.Errorf("parse list response: %w", err)
		}
		for _, c := range parsed.Contents {
			if !strings.HasSuffix(c.Key, ".ndjson") {
				continue
			}
			out = append(out, c.Key)
			if len(out) >= maxKeys {
				break
			}
		}
		if parsed.IsTruncated == "true" || parsed.IsTruncated == "TRUE" {
			continuationToken = parsed.NextContinuationToken
			continue
		}
		break
	}
	return out, nil
}

type listV2Resp struct {
	XMLName               xml.Name `xml:"ListBucketResult"`
	IsTruncated           string   `xml:"IsTruncated"`
	NextContinuationToken string   `xml:"NextContinuationToken"`
	Contents              []struct {
		Key string `xml:"Key"`
	} `xml:"Contents"`
}

// processObject GETs one NDJSON file, aggregates its events in memory,
// and applies the whole object — progress, topic activity, and the
// cursor advance to key — in one registry transaction. Nothing is
// written until the body has been read to the end, so a mid-body
// failure leaves the registry (cursor included) untouched and the next
// tick retries the object from scratch.
func (t *Tailer) processObject(ctx context.Context, key string, vSet map[string]bool) (int, error) {
	reqURL := fmt.Sprintf("%s/%s/%s", strings.TrimRight(t.cfg.Endpoint, "/"), t.cfg.Bucket, key)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, reqURL, nil)
	if err != nil {
		return 0, err
	}
	if err := t.signV4(req, nil); err != nil {
		return 0, err
	}
	resp, err := t.cli.Do(req)
	if err != nil {
		return 0, err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 1024))
		return 0, fmt.Errorf("GET %s returned %d: %s", key, resp.StatusCode, string(body))
	}
	scanner := bufio.NewScanner(resp.Body)
	// audit NDJSON lines stay well under 1 MiB even with inline payload.
	buf := make([]byte, 0, 64*1024)
	scanner.Buffer(buf, 4*1024*1024)
	agg := newObjectAgg()
	n := 0
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var ev auditEvent
		if err := json.Unmarshal(line, &ev); err != nil {
			// One malformed line shouldn't poison the whole object.
			slog.Debug("skip malformed audit line", "key", key, "err", err)
			continue
		}
		agg.add(&ev, vSet)
		n++
	}
	if err := scanner.Err(); err != nil {
		return 0, err
	}
	updates := agg.flatten()
	if err := t.reg.ApplyAuditObject(ctx, t.cfg.SubscriberID, key, updates); err != nil {
		return 0, err
	}
	return n, nil
}

// auditEvent matches the audit-subscriber NDJSON shape.
type auditEvent struct {
	Vendor          string `json:"vendor"`
	KeyExpr         string `json:"key_expr"`
	TS              string `json:"ts"`
	PayloadHash     string `json:"payload_hash"`
	PayloadLen      int    `json:"payload_len"`
	PayloadBytesB64 string `json:"payload_bytes_b64"`
}

// objectAgg accumulates one NDJSON object's events in memory so the
// registry can apply the whole object in a single transaction
// (ApplyAuditObject) instead of two autocommit writes per event.
//
// The aggregate is keyed by ORG and nothing else. It deliberately does not
// keep a per-(org, topic) entry: that was the shape that wrote one registry
// row per distinct key expression ever observed, which is unbounded by
// publisher behaviour — 86,810 rows against 353 declared topics, one org
// holding 61,540 of them for 74,427 publishes. How many distinct
// topics an org publishes under is answered by internal/activity from the
// audit-subscriber's own Prometheus counters, which are labelled by DECLARED
// pattern and therefore bounded at source.
type objectAgg struct {
	progress map[string]*registry.AuditProgressUpdate
	// First-seen order, so writes land in stream order — deterministic
	// transactions are easier to reason about in tests and traces.
	progressOrder []string
}

func newObjectAgg() *objectAgg {
	return &objectAgg{progress: map[string]*registry.AuditProgressUpdate{}}
}

// add fingerprints one event and folds it into the aggregate. Skips
// events whose vendor isn't in vSet (unknown prefixes, retired vendors,
// or synthetic vendors filtered upstream).
func (a *objectAgg) add(ev *auditEvent, vSet map[string]bool) {
	prefix := vendorOfKey(ev.KeyExpr)
	if prefix == "" || !vSet[prefix] {
		return
	}
	ts := parseTS(ev.TS)
	if ts == 0 {
		// Same fallback MarkPublish applied per event: an unparseable
		// timestamp counts as "now".
		ts = time.Now().Unix()
	}
	u := a.progress[prefix]
	if u == nil {
		u = &registry.AuditProgressUpdate{Prefix: prefix}
		a.progress[prefix] = u
		a.progressOrder = append(a.progressOrder, prefix)
	}
	if isHelloWorld(ev, prefix) {
		if u.HelloFirstAt == 0 || ts < u.HelloFirstAt {
			u.HelloFirstAt = ts
		}
	} else if u.OtherFirstAt == 0 || ts < u.OtherFirstAt {
		u.OtherFirstAt = ts
	}
	if ts > u.LastPublishAt {
		u.LastPublishAt = ts
		// last_topic tracks last_publish_at rather than stream order, so an
		// out-of-order replayed event cannot rewind the name the surface
		// shows — the same monotonic rule the timestamp itself follows.
		u.LastTopic = ev.KeyExpr
	}
	u.PublishDelta++
}

// flatten returns the aggregates in first-seen order, ready for
// ApplyAuditObject.
func (a *objectAgg) flatten() []registry.AuditProgressUpdate {
	updates := make([]registry.AuditProgressUpdate, 0, len(a.progressOrder))
	for _, p := range a.progressOrder {
		updates = append(updates, *a.progress[p])
	}
	return updates
}

// vendorOfKey returns the first '/' segment of a Zenoh key, or "".
func vendorOfKey(k string) string {
	if i := strings.IndexByte(k, '/'); i > 0 {
		return k[:i]
	}
	return ""
}

// helloRE matches first-publisher.py's payload: "hello from <prefix> at <unix-float-3dp>".
var helloRE = regexp.MustCompile(`^hello from [a-z0-9-]+ at \d+\.\d+$`)

// isHelloWorld returns true when ev matches the bundle's
// first-publisher.py fingerprint (key + payload). Both halves must
// match — a misaligned key with a "hello from …" payload counts as L4
// (it's not what we shipped).
func isHelloWorld(ev *auditEvent, prefix string) bool {
	if ev.KeyExpr != prefix+"/hello/v1" {
		return false
	}
	if ev.PayloadBytesB64 == "" {
		// Audit-subscriber inlines payloads ≤16 KiB; our hello payload
		// is ~30 bytes, so absence here is structural drift. Treat as
		// non-hello to avoid false positives.
		return false
	}
	body, err := base64.StdEncoding.DecodeString(ev.PayloadBytesB64)
	if err != nil {
		return false
	}
	s := strings.TrimSpace(string(body))
	if !strings.HasPrefix(s, "hello from "+prefix+" at ") {
		return false
	}
	return helloRE.MatchString(s)
}

// parseTS converts the audit-subscriber's ISO-8601 timestamp into Unix
// seconds. Returns 0 on parse failure (MarkPublish falls back to now).
func parseTS(s string) int64 {
	if s == "" {
		return 0
	}
	for _, layout := range []string{
		time.RFC3339Nano,
		time.RFC3339,
		"2006-01-02T15:04:05.999Z",
		"2006-01-02T15:04:05Z",
	} {
		if t, err := time.Parse(layout, s); err == nil {
			return t.Unix()
		}
	}
	return 0
}

// --- SigV4 (S3 path-style, query-string for ListObjectsV2) ------------

// signV4 adds Authorization + x-amz-date + x-amz-content-sha256 headers
// to req. body may be nil for GET. Path-style addressing only.
func (t *Tailer) signV4(req *http.Request, body []byte) error {
	return signV4Request(t.cfg, req, body)
}

// signV4Request is the header-form SigV4 signer, parameterized by Config so
// both the Tailer (live poll) and the Presigner share one implementation.
func signV4Request(cfg Config, req *http.Request, body []byte) error {
	if body == nil {
		body = []byte{}
	}
	bodyHash := sha256.Sum256(body)
	bodyHashHex := hex.EncodeToString(bodyHash[:])
	req.Header.Set("x-amz-content-sha256", bodyHashHex)

	now := time.Now().UTC()
	amzDate := now.Format("20060102T150405Z")
	dateStamp := now.Format("20060102")
	req.Header.Set("x-amz-date", amzDate)

	// Canonical headers: lowercase host + sorted x-amz-* + content sha.
	host := req.URL.Host
	req.Host = host

	signedHeaders := "host;x-amz-content-sha256;x-amz-date"
	canonicalHeaders := fmt.Sprintf("host:%s\nx-amz-content-sha256:%s\nx-amz-date:%s\n",
		host, bodyHashHex, amzDate)

	canonicalQuery := canonicalQueryString(req.URL.Query())

	canonicalURI := req.URL.EscapedPath()
	if canonicalURI == "" {
		canonicalURI = "/"
	}

	canonicalRequest := strings.Join([]string{
		req.Method,
		canonicalURI,
		canonicalQuery,
		canonicalHeaders,
		signedHeaders,
		bodyHashHex,
	}, "\n")
	crHash := sha256.Sum256([]byte(canonicalRequest))

	scope := fmt.Sprintf("%s/%s/s3/aws4_request", dateStamp, cfg.Region)
	stringToSign := strings.Join([]string{
		"AWS4-HMAC-SHA256",
		amzDate,
		scope,
		hex.EncodeToString(crHash[:]),
	}, "\n")

	kDate := hmacSHA256([]byte("AWS4"+cfg.SecretKey), []byte(dateStamp))
	kRegion := hmacSHA256(kDate, []byte(cfg.Region))
	kService := hmacSHA256(kRegion, []byte("s3"))
	kSigning := hmacSHA256(kService, []byte("aws4_request"))
	sig := hex.EncodeToString(hmacSHA256(kSigning, []byte(stringToSign)))

	auth := fmt.Sprintf("AWS4-HMAC-SHA256 Credential=%s/%s, SignedHeaders=%s, Signature=%s",
		cfg.AccessKey, scope, signedHeaders, sig)
	req.Header.Set("Authorization", auth)
	return nil
}

func hmacSHA256(key, data []byte) []byte {
	m := hmac.New(sha256.New, key)
	m.Write(data)
	return m.Sum(nil)
}

// canonicalQueryString builds the SigV4 canonical query string from a
// parsed url.Values. Keys + values are RFC3986-escaped and sorted by
// (key, value).
func canonicalQueryString(q url.Values) string {
	if len(q) == 0 {
		return ""
	}
	type kv struct{ k, v string }
	flat := make([]kv, 0, len(q))
	for k, vs := range q {
		ek := awsURIEscape(k)
		for _, v := range vs {
			flat = append(flat, kv{ek, awsURIEscape(v)})
		}
	}
	// Sort by encoded key, then value.
	for i := 1; i < len(flat); i++ {
		for j := i; j > 0 && (flat[j-1].k > flat[j].k || (flat[j-1].k == flat[j].k && flat[j-1].v > flat[j].v)); j-- {
			flat[j-1], flat[j] = flat[j], flat[j-1]
		}
	}
	var b strings.Builder
	for i, p := range flat {
		if i > 0 {
			b.WriteByte('&')
		}
		b.WriteString(p.k)
		b.WriteByte('=')
		b.WriteString(p.v)
	}
	return b.String()
}

// awsURIEscape encodes per AWS SigV4 rules (RFC3986 unreserved are kept,
// everything else percent-encoded; '/' is also escaped).
func awsURIEscape(s string) string {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		c := s[i]
		if (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') ||
			c == '-' || c == '_' || c == '.' || c == '~' {
			b.WriteByte(c)
			continue
		}
		b.WriteString(fmt.Sprintf("%%%02X", c))
	}
	return b.String()
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "…"
}

// ensure error sentinels look right in coverage runs.
var _ = errors.New

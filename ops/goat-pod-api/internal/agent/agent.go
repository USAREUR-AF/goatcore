// Package agent is the pod-side carriage runtime: it pulls and verifies the pod's rule
// generation, folds in local rules from the ledger, renders and applies the router ACL, uploads
// the ledger, and reports status.
package agent

import (
	"bufio"
	"bytes"
	"context"
	"crypto"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/canon"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/router"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/rules"
)

// Errors the API maps to status codes.
var (
	ErrInvalid   = errors.New("invalid request")
	ErrNotFound  = errors.New("not found")
	ErrForbidden = errors.New("not authorized")
	ErrConflict  = errors.New("conflict")
)

// Agent holds the pod's carriage state.
type Agent struct {
	cfg     *config.Config
	signer  *x509.Certificate
	podCert *x509.Certificate
	podKey  crypto.Signer
	Ledger  *ledger.Ledger
	applier *router.Applier
	client  *http.Client
	Now     func() time.Time
	Log     *log.Logger

	op sync.Mutex // serializes changes, pulls and applies
	mu sync.Mutex // guards the fields below

	gen          *generation.Generation
	genRaw       []byte
	archive      map[string]rules.Archived
	lastContact  time.Time
	lastPull     time.Time
	lastPullErr  string
	lastApply    time.Time
	lastApplyErr string
	failedSHA    string
	rejectedGen  int64
	lastUpload   time.Time
	uploadErr    string
	reserved     []string
}

// New loads the configuration's keys, certificates, ledger and saved state.
func New(cfg *config.Config, rt router.Runtime) (*Agent, error) {
	signer, err := canon.LoadCertificate(cfg.PortalSignerCert)
	if err != nil {
		return nil, err
	}
	podCert, err := canon.LoadCertificate(cfg.PodCert)
	if err != nil {
		return nil, err
	}
	podKey, err := canon.LoadPrivateKey(cfg.PodKey)
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(cfg.StateDir, 0o700); err != nil {
		return nil, fmt.Errorf("state dir: %w", err)
	}
	led, err := ledger.Open(filepath.Join(cfg.StateDir, "ledger.jsonl"), podKey, podCert, cfg.PodSlot)
	if err != nil {
		return nil, err
	}
	a := &Agent{
		cfg: cfg, signer: signer, podCert: podCert, podKey: podKey, Ledger: led,
		applier: &router.Applier{
			Files:   router.Files{Config: cfg.Router.Config, LastGood: cfg.Router.LastGood, AppliedSHA: cfg.Router.AppliedSHA},
			Runtime: rt, Timeout: time.Duration(cfg.Router.HealthTimeoutSeconds) * time.Second,
		},
		Now:     time.Now,
		Log:     log.New(os.Stderr, "goat-pod-api: ", log.LstdFlags|log.LUTC),
		archive: map[string]rules.Archived{},
	}
	a.reserved = []string{podCert.Subject.CommonName}
	if cfg.FabricRouterCN != "" {
		a.reserved = append(a.reserved, cfg.FabricRouterCN)
	}
	if err := a.loadState(); err != nil {
		return nil, err
	}
	if cfg.PortalURL != "" {
		if a.client, err = portalClient(cfg); err != nil {
			return nil, err
		}
	}
	return a, nil
}

func portalClient(cfg *config.Config) (*http.Client, error) {
	pair, err := tls.LoadX509KeyPair(cfg.PodCert, cfg.PodKey)
	if err != nil {
		return nil, fmt.Errorf("pod certificate: %w", err)
	}
	tc := &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{pair}}
	if cfg.PortalCA != "" {
		raw, err := os.ReadFile(cfg.PortalCA)
		if err != nil {
			return nil, fmt.Errorf("portal CA: %w", err)
		}
		pool := x509.NewCertPool()
		if !pool.AppendCertsFromPEM(raw) {
			return nil, fmt.Errorf("portal CA %s holds no certificate", cfg.PortalCA)
		}
		tc.RootCAs = pool
	}
	return &http.Client{Timeout: 30 * time.Second, Transport: &http.Transport{TLSClientConfig: tc}}, nil
}

func (a *Agent) genPath() string     { return filepath.Join(a.cfg.StateDir, "generation.json") }
func (a *Agent) archivePath() string { return filepath.Join(a.cfg.StateDir, "central-archive.json") }

func (a *Agent) loadState() error {
	raw, err := os.ReadFile(a.genPath())
	switch {
	case errors.Is(err, os.ErrNotExist):
	case err != nil:
		return err
	default:
		g, err := generation.Verify(raw, a.signer, a.cfg.PodSlot)
		if err != nil {
			return fmt.Errorf("saved generation %s no longer verifies: %w", a.genPath(), err)
		}
		a.gen, a.genRaw = g, raw
	}
	raw, err = os.ReadFile(a.archivePath())
	switch {
	case errors.Is(err, os.ErrNotExist):
	case err != nil:
		return err
	default:
		if err := json.Unmarshal(raw, &a.archive); err != nil {
			return fmt.Errorf("archive %s: %w", a.archivePath(), err)
		}
	}
	return nil
}

func writeAtomic(path string, data []byte) error {
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// Compute returns the current rule set.
func (a *Agent) Compute() rules.Result {
	a.mu.Lock()
	gen, archive := a.gen, a.archive
	a.mu.Unlock()
	return rules.Compute(rules.Inputs{Gen: gen, Archive: archive, Ledger: a.Ledger.Entries(), Ack: a.Ledger.Acked(), Now: a.Now()})
}

func (a *Agent) fabricCN(base []byte) string {
	if a.cfg.FabricRouterCN != "" {
		return a.cfg.FabricRouterCN
	}
	return SubjectCN(base, acl.FabricSubject)
}

// SubjectCN reads the first certificate CN of subject id from a rendered router configuration.
func SubjectCN(conf []byte, id string) string {
	sc := bufio.NewScanner(bytes.NewReader(conf))
	found := false
	for sc.Scan() {
		line := sc.Text()
		if strings.Contains(line, `id: "`+id+`"`) {
			found = true
			continue
		}
		if found && strings.Contains(line, "cert_common_names:") {
			parts := strings.Split(line, `"`)
			if len(parts) > 1 {
				return parts[1]
			}
			return ""
		}
	}
	return ""
}

// Render produces the router configuration for the current rule set.
func (a *Agent) Render() ([]byte, rules.Result, error) {
	res := a.Compute()
	base, err := os.ReadFile(a.cfg.Router.BaseConfig)
	if err != nil {
		return nil, res, fmt.Errorf("base router configuration: %w", err)
	}
	reserved := append([]string(nil), a.reserved...)
	if cn := a.fabricCN(base); cn != "" {
		reserved = append(reserved, cn)
	}
	a.mu.Lock()
	var read []string
	if a.gen != nil {
		read = a.gen.Read
	}
	a.mu.Unlock()
	out, err := acl.Render(base, res.Effective, read, acl.Options{
		PodSlot: a.cfg.PodSlot, HealthKey: a.cfg.HealthKey(), Reserved: reserved,
	})
	return out, res, err
}

// Reconcile renders and applies. A rendering the router already rejected is not retried until
// the inputs change.
func (a *Agent) Reconcile(ctx context.Context) error {
	a.op.Lock()
	defer a.op.Unlock()
	return a.reconcileLocked(ctx)
}

func (a *Agent) reconcileLocked(ctx context.Context) error {
	out, _, err := a.Render()
	if err != nil {
		a.setApply(err)
		return err
	}
	sha := sha(out)
	a.mu.Lock()
	failed, lastErr := a.failedSHA, a.lastApplyErr
	a.mu.Unlock()
	if sha == failed {
		return errors.New(lastErr)
	}
	res, err := a.applier.Apply(ctx, out)
	a.mu.Lock()
	defer a.mu.Unlock()
	if err != nil {
		a.failedSHA = sha
		a.lastApplyErr = err.Error()
		a.Log.Printf("apply: %v", err)
		return err
	}
	a.failedSHA, a.lastApplyErr = "", ""
	if res.Changed {
		a.lastApply = a.Now()
		a.Log.Printf("router configuration applied")
	}
	return nil
}

func (a *Agent) setApply(err error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.lastApplyErr = err.Error()
}

func sha(b []byte) string {
	h := crypto.SHA256.New()
	h.Write(b)
	return hex.EncodeToString(h.Sum(nil))
}

// Adopt verifies raw as a new generation and applies it. A lower generation than the one held
// is refused; the same number must carry the same document.
func (a *Agent) Adopt(ctx context.Context, raw []byte) error {
	g, err := generation.Verify(raw, a.signer, a.cfg.PodSlot)
	if err != nil {
		return err
	}
	a.op.Lock()
	defer a.op.Unlock()
	a.mu.Lock()
	prev, prevRaw, rejected := a.gen, a.genRaw, a.rejectedGen
	a.mu.Unlock()
	if prev != nil {
		switch {
		case g.Generation < prev.Generation:
			return fmt.Errorf("%w: generation %d is lower than the %d this pod holds", ErrConflict, g.Generation, prev.Generation)
		case g.Generation == prev.Generation:
			if a.sameDocument(raw, prevRaw) {
				return nil
			}
			return fmt.Errorf("%w: generation %d differs from the copy this pod holds", ErrConflict, g.Generation)
		}
	}
	if g.Generation == rejected {
		return fmt.Errorf("generation %d was rejected by the router earlier; waiting for a newer one", g.Generation)
	}
	now := a.Now()
	newArchive := rules.NextArchive(a.archive, prev, g, now, a.cfg.Reactivation())
	a.mu.Lock()
	oldArchive := a.archive
	a.gen, a.genRaw, a.archive = g, raw, newArchive
	a.mu.Unlock()
	if err := a.reconcileLocked(ctx); err != nil {
		a.mu.Lock()
		a.gen, a.genRaw, a.archive, a.rejectedGen = prev, prevRaw, oldArchive, g.Generation
		a.mu.Unlock()
		// Bring the router back in line with the generation still held.
		if rerr := a.reconcileLocked(ctx); rerr != nil {
			a.Log.Printf("re-render with generation still held: %v", rerr)
		}
		return fmt.Errorf("generation %d not applied: %w", g.Generation, err)
	}
	if err := writeAtomic(a.genPath(), raw); err != nil {
		return fmt.Errorf("persist generation: %w", err)
	}
	arch, err := json.Marshal(newArchive)
	if err != nil {
		return err
	}
	if err := writeAtomic(a.archivePath(), arch); err != nil {
		return fmt.Errorf("persist archive: %w", err)
	}
	a.Log.Printf("generation %d adopted (%d sources)", g.Generation, len(g.Sources))
	return nil
}

func (a *Agent) sameDocument(x, y []byte) bool {
	cx, err1 := canon.FromJSON(x)
	cy, err2 := canon.FromJSON(y)
	return err1 == nil && err2 == nil && bytes.Equal(cx, cy)
}

func (a *Agent) podURL(suffix string) string {
	return strings.TrimRight(a.cfg.PortalURL, "/") + "/api/carriage/pods/" + url.PathEscape(a.cfg.PodSlot) + suffix
}

// Pull fetches and adopts the generation.
func (a *Agent) Pull(ctx context.Context) error {
	if a.client == nil {
		return errors.New("no portal_url configured")
	}
	err := a.pull(ctx)
	a.mu.Lock()
	a.lastPull = a.Now()
	if err != nil {
		a.lastPullErr = err.Error()
	} else {
		a.lastPullErr = ""
	}
	a.mu.Unlock()
	return err
}

func (a *Agent) pull(ctx context.Context) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, a.podURL("/generation"), nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "application/json")
	resp, err := a.client.Do(req)
	if err != nil {
		return fmt.Errorf("pull generation: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil {
		return fmt.Errorf("pull generation: %w", err)
	}
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("pull generation: portal answered %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	a.mu.Lock()
	a.lastContact = a.Now()
	a.mu.Unlock()
	return a.Adopt(ctx, body)
}

// Upload posts unacknowledged ledger lines to the portal.
func (a *Agent) Upload(ctx context.Context) error {
	if a.client == nil {
		return errors.New("no portal_url configured")
	}
	err := a.upload(ctx)
	a.mu.Lock()
	if err != nil {
		a.uploadErr = err.Error()
	} else {
		a.uploadErr = ""
		a.lastUpload = a.Now()
	}
	a.mu.Unlock()
	return err
}

func (a *Agent) upload(ctx context.Context) error {
	body, n := a.Ledger.Unsent()
	if n == 0 {
		return nil
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, a.podURL("/ledger"), bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/x-ndjson")
	resp, err := a.client.Do(req)
	if err != nil {
		return fmt.Errorf("upload ledger: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return fmt.Errorf("upload ledger: %w", err)
	}
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("upload ledger: portal answered %d: %s", resp.StatusCode, strings.TrimSpace(string(raw)))
	}
	var ack struct {
		Accepted int   `json:"accepted"`
		LastSeq  int64 `json:"last_seq"`
	}
	if err := json.Unmarshal(raw, &ack); err != nil {
		return fmt.Errorf("upload ledger: portal answer: %w", err)
	}
	a.mu.Lock()
	held := int64(0)
	if a.gen != nil {
		held = a.gen.Generation
	}
	a.lastContact = a.Now()
	a.mu.Unlock()
	if err := a.Ledger.SetAck(ack.LastSeq, a.Now(), held); err != nil {
		return err
	}
	a.Log.Printf("ledger uploaded: %d lines sent, portal holds up to %d", n, ack.LastSeq)
	return nil
}

// Run pulls, uploads, reconciles and publishes health until ctx ends.
func (a *Agent) Run(ctx context.Context) {
	tick := func() {
		if a.client != nil {
			if err := a.Upload(ctx); err != nil {
				a.Log.Printf("%v", err)
			}
			if err := a.Pull(ctx); err != nil {
				a.Log.Printf("%v", err)
			}
		}
		// Expiry of local and central rules takes effect here.
		if err := a.Reconcile(ctx); err != nil {
			a.Log.Printf("reconcile: %v", err)
		}
		a.PublishHealth(ctx)
	}
	tick()
	t := time.NewTicker(a.cfg.PullInterval())
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			tick()
		}
	}
}

// --- local changes (pod-carriage-and-provenance.md §7.4) ---------------------------------------

// Change is a local change request.
type Change struct {
	Source     string    `json:"source"`
	Until      time.Time `json:"until"`
	Reason     string    `json:"reason"`
	Principals []string  `json:"principals,omitempty"`
}

// Authorize returns the local authority entry for a client certificate CN.
func (a *Agent) Authorize(cn string) (generation.Authority, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.gen == nil {
		return generation.Authority{}, fmt.Errorf("%w: this pod holds no rule generation, so it names no local authority", ErrForbidden)
	}
	for _, la := range a.gen.LocalAuthority {
		if la.CertCN == cn {
			return la, nil
		}
	}
	return generation.Authority{}, fmt.Errorf("%w: %s is not named as local authority for this pod", ErrForbidden, cn)
}

func (a *Agent) checkChange(c Change) error {
	now := a.Now()
	if strings.TrimSpace(c.Reason) == "" {
		return fmt.Errorf("%w: a reason is required", ErrInvalid)
	}
	if len(c.Reason) > 500 {
		return fmt.Errorf("%w: the reason is longer than 500 characters", ErrInvalid)
	}
	if c.Until.IsZero() {
		return fmt.Errorf("%w: an end date (until) is required", ErrInvalid)
	}
	if !c.Until.After(now) {
		return fmt.Errorf("%w: until %s is not in the future", ErrInvalid, c.Until.Format(time.RFC3339))
	}
	if limit := now.Add(a.cfg.MaxRule()); c.Until.After(limit) {
		return fmt.Errorf("%w: until is later than the longest rule length (%d days, until %s)",
			ErrInvalid, a.cfg.MaxRuleDays, limit.Format(time.RFC3339))
	}
	return nil
}

func (a *Agent) held() int64 {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.gen == nil {
		return 0
	}
	return a.gen.Generation
}

func newRuleID() (string, error) {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return "lr_" + hex.EncodeToString(b), nil
}

// parseSource accepts a slot or a certificate CN of the form <slot>.g<N>.
func parseSource(s string) (slot, cn string, err error) {
	if generation.ValidSlot(s) {
		return s, "", nil
	}
	if i := strings.LastIndex(s, ".g"); i > 0 && generation.ValidSlot(s[:i]) && generation.ValidCN(s) {
		return s[:i], s, nil
	}
	return "", "", fmt.Errorf("%w: source %q is neither a slot nor a certificate name <slot>.g<N>", ErrInvalid, s)
}

// Allow adds a local rule letting a source use this pod.
func (a *Agent) Allow(ctx context.Context, who generation.Authority, c Change) (ledger.Entry, error) {
	if err := a.checkChange(c); err != nil {
		return ledger.Entry{}, err
	}
	slot, cn, err := parseSource(c.Source)
	if err != nil {
		return ledger.Entry{}, err
	}
	principals := c.Principals
	write := generation.DefaultWrite(slot)
	if cn != "" {
		principals = append(principals, cn)
	}
	if len(principals) == 0 {
		known, kw, ok := a.Compute().KnownSource(slot)
		if !ok {
			return ledger.Entry{}, fmt.Errorf("%w: this pod has never carried %s; name its certificate (<slot>.g<N>) as the source", ErrInvalid, slot)
		}
		principals, write = known, kw
	}
	for _, p := range principals {
		if !generation.ValidCN(p) {
			return ledger.Entry{}, fmt.Errorf("%w: principal %q is not a certificate name", ErrInvalid, p)
		}
		for _, r := range a.reserved {
			if p == r {
				return ledger.Entry{}, fmt.Errorf("%w: %q is this pod's own identity", ErrInvalid, p)
			}
		}
	}
	id, err := newRuleID()
	if err != nil {
		return ledger.Entry{}, err
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionAllow, RuleID: id, Source: slot, Principals: principals,
		Write: write, Until: c.Until.UTC(), Reason: c.Reason, Authority: who})
}

// Extend moves the end date of an active rule.
func (a *Agent) Extend(ctx context.Context, who generation.Authority, id string, c Change) (ledger.Entry, error) {
	if err := a.checkChange(c); err != nil {
		return ledger.Entry{}, err
	}
	r, ok := a.Compute().Find(id)
	if !ok {
		return ledger.Entry{}, fmt.Errorf("%w: no rule %s on this pod", ErrNotFound, id)
	}
	if r.State != rules.StateActive {
		return ledger.Entry{}, fmt.Errorf("%w: rule %s is %s; reactivate it instead", ErrConflict, id, r.State)
	}
	if r.Kind == "org" {
		return ledger.Entry{}, fmt.Errorf("%w: rule %s is an org rule and has no end date", ErrConflict, id)
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionExtend, RuleID: id, Source: r.Source, Until: c.Until.UTC(),
		Reason: c.Reason, Authority: who})
}

// Reactivate brings back an archived, expired or revoked rule inside the reactivation window.
func (a *Agent) Reactivate(ctx context.Context, who generation.Authority, id string, c Change) (ledger.Entry, error) {
	if err := a.checkChange(c); err != nil {
		return ledger.Entry{}, err
	}
	r, ok := a.Compute().Find(id)
	if !ok {
		return ledger.Entry{}, fmt.Errorf("%w: no rule %s on this pod", ErrNotFound, id)
	}
	switch r.State {
	case rules.StateActive:
		return ledger.Entry{}, fmt.Errorf("%w: rule %s is active; extend it instead", ErrConflict, id)
	case rules.StateSettled:
		return ledger.Entry{}, fmt.Errorf("%w: rule %s was decided by the portal; ask an org lead", ErrConflict, id)
	}
	if r.EndedAt != nil && a.Now().Sub(*r.EndedAt) > a.cfg.Reactivation() {
		return ledger.Entry{}, fmt.Errorf("%w: rule %s ended %s, outside the %d-day reactivation window; approve it as a new rule",
			ErrConflict, id, r.EndedAt.Format(time.RFC3339), a.cfg.ReactivationDays)
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionReactivate, RuleID: id, Source: r.Source, Until: c.Until.UTC(),
		Reason: c.Reason, Authority: who})
}

// Revoke cuts a source off this pod until c.Until.
func (a *Agent) Revoke(ctx context.Context, who generation.Authority, slot string, c Change) (ledger.Entry, error) {
	if err := a.checkChange(c); err != nil {
		return ledger.Entry{}, err
	}
	if !generation.ValidSlot(slot) {
		return ledger.Entry{}, fmt.Errorf("%w: %q is not a slot", ErrInvalid, slot)
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionRevoke, RuleID: "", Source: slot, Until: c.Until.UTC(),
		Reason: c.Reason, Authority: who})
}

func (a *Agent) record(ctx context.Context, e ledger.Entry) (ledger.Entry, error) {
	a.op.Lock()
	defer a.op.Unlock()
	e.Generation = a.held()
	stored, err := a.Ledger.Append(e)
	if err != nil {
		return stored, err
	}
	a.Log.Printf("local %s %s source %s until %s by %s", stored.Action, stored.RuleID, stored.Source,
		stored.Until.Format(time.RFC3339), stored.Authority.Identity)
	if err := a.reconcileLocked(ctx); err != nil {
		return stored, fmt.Errorf("change recorded (ledger line %d) but the router did not take it: %w", stored.Seq, err)
	}
	return stored, nil
}

// --- status -------------------------------------------------------------------------------------

// SourceHealth is the carriage health of one source on this pod.
type SourceHealth struct {
	Source     string     `json:"source"`
	State      string     `json:"state"` // carried | revoked | expired | archived
	Origin     string     `json:"origin"`
	Rules      []string   `json:"rules"`
	Until      *time.Time `json:"until"`
	Principals []string   `json:"principals"`
}

// Status is GET /v1/status.
type Status struct {
	PodSlot           string                 `json:"pod_slot"`
	State             string                 `json:"state"` // connected | cut off | no portal
	Connected         bool                   `json:"connected"`
	LastContact       *time.Time             `json:"last_contact"`
	LastPullError     string                 `json:"last_pull_error,omitempty"`
	Generation        int64                  `json:"generation"`
	GenerationIssued  *time.Time             `json:"generation_issued_at"`
	GenerationAgeSecs *int64                 `json:"generation_age_seconds"`
	LastApply         *time.Time             `json:"last_apply"`
	LastApplyError    string                 `json:"last_apply_error,omitempty"`
	SourcesCarried    int                    `json:"sources_carried"`
	SourcesConnected  *int                   `json:"sources_connected"`
	Sources           []SourceHealth         `json:"sources"`
	Pending           int                    `json:"pending"`
	LedgerUnsent      int                    `json:"ledger_unsent"`
	LastUploadError   string                 `json:"last_upload_error,omitempty"`
	LocalAuthority    []generation.Authority `json:"local_authority"`
	MaxRuleDays       int                    `json:"max_rule_days"`
	ReactivationDays  int                    `json:"reactivation_days"`
}

func tp(t time.Time) *time.Time {
	if t.IsZero() {
		return nil
	}
	return &t
}

// Status reports the pod's carriage state.
func (a *Agent) Status() Status {
	res := a.Compute()
	now := a.Now()
	a.mu.Lock()
	defer a.mu.Unlock()
	st := Status{
		PodSlot: a.cfg.PodSlot, LastContact: tp(a.lastContact), LastPullError: a.lastPullErr,
		LastApply: tp(a.lastApply), LastApplyError: a.lastApplyErr, LastUploadError: a.uploadErr,
		MaxRuleDays: a.cfg.MaxRuleDays, ReactivationDays: a.cfg.ReactivationDays,
		LocalAuthority: []generation.Authority{},
	}
	switch {
	case a.cfg.PortalURL == "":
		st.State = "no portal"
	case !a.lastContact.IsZero() && now.Sub(a.lastContact) <= a.cfg.CutoffAfter():
		st.State, st.Connected = "connected", true
	default:
		st.State = "cut off"
	}
	if a.gen != nil {
		st.Generation = a.gen.Generation
		issued := a.gen.IssuedAt
		st.GenerationIssued = &issued
		age := int64(now.Sub(issued).Seconds())
		st.GenerationAgeSecs = &age
		st.LocalAuthority = a.gen.LocalAuthority
	}
	bySource := map[string]*SourceHealth{}
	var order []string
	for _, r := range res.Rules {
		state := map[string]string{
			rules.StateActive: "carried", rules.StateRevoked: "revoked", rules.StateExpired: "expired",
			rules.StateArchive: "archived", rules.StateSettled: "settled",
		}[r.State]
		h, ok := bySource[r.Source]
		if !ok {
			h = &SourceHealth{Source: r.Source, State: state, Origin: r.Origin}
			bySource[r.Source] = h
			order = append(order, r.Source)
		} else if state == "carried" || (h.State != "carried" && state == "revoked") {
			h.State, h.Origin = state, r.Origin
		}
		h.Rules = append(h.Rules, r.ID)
		if r.Pending {
			st.Pending++
		}
	}
	for _, e := range res.Effective {
		if h, ok := bySource[e.Slot]; ok {
			h.Until = e.Until
			h.Principals = e.Principals
		}
	}
	for _, slot := range order {
		st.Sources = append(st.Sources, *bySource[slot])
	}
	if st.Sources == nil {
		st.Sources = []SourceHealth{}
	}
	st.SourcesCarried = len(res.Effective)
	_, st.LedgerUnsent = a.Ledger.Unsent()
	return st
}

// Pending lists ledger lines the portal has not acknowledged.
func (a *Agent) Pending() []ledger.Entry {
	ack := a.Ledger.Acked()
	out := []ledger.Entry{}
	for _, e := range a.Ledger.Entries() {
		if e.Seq > ack.LastSeq {
			out = append(out, e)
		}
	}
	return out
}

// HealthEnvelope is the JSON form of EnvHealthV1 with domain "pod".
type HealthEnvelope struct {
	Domain       string `json:"domain"`
	Scope        string `json:"scope"`
	TSUnixMs     int64  `json:"ts_unix_ms"`
	OverallColor string `json:"overall_color"`
	SampleCount  int    `json:"sample_count"`
	WindowSecs   int    `json:"window_secs"`
	Detail       struct {
		Pod Status `json:"pod"`
	} `json:"detail"`
}

// Health returns the carriage health envelope.
func (a *Agent) Health() HealthEnvelope {
	st := a.Status()
	h := HealthEnvelope{Domain: "pod", Scope: a.cfg.PodSlot, TSUnixMs: a.Now().UnixMilli(),
		SampleCount: st.SourcesCarried, WindowSecs: a.cfg.CutoffAfterSeconds}
	switch {
	case st.Generation == 0:
		h.OverallColor = "unmetered"
	case st.LastApplyError != "":
		h.OverallColor = "red"
	case !st.Connected:
		h.OverallColor = "amber"
	default:
		h.OverallColor = "green"
	}
	h.Detail.Pod = st
	return h
}

// PublishHealth runs the configured publish command with the health envelope.
func (a *Agent) PublishHealth(ctx context.Context) {
	key := a.cfg.HealthKey()
	if key == "" {
		return
	}
	payload, err := json.Marshal(a.Health())
	if err != nil {
		return
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	args := append(append([]string(nil), a.cfg.HealthPublish.Command[1:]...), key, "--payload", string(payload), "--encoding", "json")
	cmd := exec.CommandContext(ctx, a.cfg.HealthPublish.Command[0], args...) // #nosec G204 -- configured command
	cmd.Env = os.Environ()
	for k, v := range a.cfg.HealthPublish.Env {
		cmd.Env = append(cmd.Env, k+"="+v)
	}
	if out, err := cmd.CombinedOutput(); err != nil {
		a.Log.Printf("publish health on %s: %v: %s", key, err, strings.TrimSpace(string(out)))
	}
}

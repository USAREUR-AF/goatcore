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
	"maps"
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

// APIError is a refusal the local API returns as {"error": {code, message, what, why, fix}}.
type APIError struct {
	Status int    `json:"-"`
	Code   string `json:"code"`
	What   string `json:"what"`
	Why    string `json:"why,omitempty"`
	Fix    string `json:"fix,omitempty"`
}

func (e *APIError) Error() string { return e.Code + ": " + e.What }

// Err builds an APIError.
func Err(status int, code, what, why, fix string) error {
	return &APIError{Status: status, Code: code, What: what, Why: why, Fix: fix}
}

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

	// wake tells watchEndDates that the rules changed, so it recomputes the next end date.
	wake chan struct{}

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
	// retryAt is when a failed configuration is tried again; zero means not until it changes.
	retryAt time.Time
	// suspended: a removal could not be applied, so no source is carried.
	suspended   bool
	rejectedGen int64
	lastUpload  time.Time
	uploadErr   string
	reserved    []string
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
		wake:    make(chan struct{}, 1),
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
	tc := &tls.Config{MinVersion: tls.VersionTLS12}
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
	if _, err := os.Stat(a.suspendedPath()); err == nil {
		a.suspended = true
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
	return rules.Compute(rules.Inputs{Gen: gen, Archive: archive, Ledger: a.Ledger.Records(), Now: a.Now()})
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
	// A change must not be left half done because the caller went away; the applier has its own
	// time limits.
	ctx = context.WithoutCancel(ctx)
	out, computed, renderErr := a.Render()
	live, err := a.applier.Current()
	if err != nil {
		a.setApply(err)
		return err
	}
	if renderErr != nil && !a.removesUnrendered(live, computed) {
		a.setApply(renderErr)
		return renderErr
	}
	sha := sha(out)
	if renderErr != nil {
		sha = "unrendered: " + renderErr.Error()
	}
	a.mu.Lock()
	failed, lastErr, retryAt, suspended := a.failedSHA, a.lastApplyErr, a.retryAt, a.suspended
	a.mu.Unlock()
	if sha == failed && (retryAt.IsZero() || a.Now().Before(retryAt)) {
		return errors.New(lastErr)
	}
	var changed bool
	if renderErr != nil || suspended || acl.Removes(live, out) {
		err = a.applyRemoval(ctx, out, renderErr)
		changed = true
	} else {
		var res router.Result
		res, err = a.applier.Apply(ctx, out)
		changed = res.Changed
	}
	suspended = errors.Is(err, router.ErrSuspended)
	if serr := a.setSuspended(suspended); serr != nil {
		a.Log.Printf("%v", serr)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if err != nil {
		a.failedSHA, a.lastApplyErr, a.retryAt = sha, err.Error(), time.Time{}
		if suspended {
			// Tried again at the next tick; a rejected addition is not.
			a.retryAt = a.Now().Add(a.cfg.PullInterval() / 2)
		}
		a.Log.Printf("apply: %v", err)
		return err
	}
	a.failedSHA, a.lastApplyErr, a.retryAt = "", "", time.Time{}
	if changed {
		a.lastApply = a.Now()
		a.Log.Printf("router configuration applied")
	}
	return nil
}

// applyRemoval applies out, falling back to the base configuration and then to a stopped router.
func (a *Agent) applyRemoval(ctx context.Context, out []byte, renderErr error) error {
	base, err := os.ReadFile(a.cfg.Router.BaseConfig)
	if err != nil || acl.Carries(base) {
		base = nil
	}
	if renderErr != nil {
		return fmt.Errorf("%w (%w)", a.applier.ApplyRemoval(ctx, nil, base), renderErr)
	}
	return a.applier.ApplyRemoval(ctx, out, base)
}

// removesUnrendered reports whether the live configuration admits a certificate the current
// rules do not carry, when those rules cannot be rendered.
func (a *Agent) removesUnrendered(live []byte, res rules.Result) bool {
	grants, err := acl.Grants(live)
	if err != nil {
		return true
	}
	carried := map[string]bool{}
	for _, s := range res.Effective {
		for _, p := range s.Principals {
			carried[p] = true
		}
	}
	for g := range grants {
		cn, _, _ := strings.Cut(g, " ")
		if !strings.HasPrefix(cn, "id:") && !carried[cn] {
			return true
		}
	}
	return false
}

func (a *Agent) suspendedPath() string { return filepath.Join(a.cfg.StateDir, "suspended") }

// setSuspended records the suspended state, so that a restart of the service keeps retrying.
func (a *Agent) setSuspended(on bool) error {
	a.mu.Lock()
	was := a.suspended
	a.suspended = on
	a.mu.Unlock()
	if was == on {
		return nil
	}
	if on {
		return writeAtomic(a.suspendedPath(), []byte("see last_apply_error\n"))
	}
	if err := os.Remove(a.suspendedPath()); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("clear suspended marker: %w", err)
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
			return fmt.Errorf("generation %d is lower than the %d this pod holds", g.Generation, prev.Generation)
		case g.Generation == prev.Generation:
			if a.sameDocument(raw, prevRaw) {
				return nil
			}
			return fmt.Errorf("generation %d differs from the copy this pod holds", g.Generation)
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
	if a.takesAway() {
		// Kept even when the router rejects it: going back to the previous generation would
		// admit a source this one removed.
		if err := a.persist(raw, newArchive); err != nil {
			a.Log.Printf("generation %d: %v", g.Generation, err)
		}
		a.signalRules()
		if err := a.reconcileLocked(ctx); err != nil {
			return fmt.Errorf("generation %d adopted, not running: %w", g.Generation, err)
		}
		a.Log.Printf("generation %d adopted (%d sources)", g.Generation, len(g.Sources))
		return nil
	}
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
	if err := a.persist(raw, newArchive); err != nil {
		return err
	}
	a.signalRules()
	a.Log.Printf("generation %d adopted (%d sources)", g.Generation, len(g.Sources))
	return nil
}

// takesAway reports whether the current rules remove a grant the live configuration holds.
func (a *Agent) takesAway() bool {
	live, err := a.applier.Current()
	if err != nil {
		return true
	}
	out, computed, err := a.Render()
	if err != nil {
		return a.removesUnrendered(live, computed)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.suspended || acl.Removes(live, out)
}

func (a *Agent) persist(raw []byte, archive map[string]rules.Archived) error {
	if err := writeAtomic(a.genPath(), raw); err != nil {
		return fmt.Errorf("persist generation: %w", err)
	}
	arch, err := json.Marshal(archive)
	if err != nil {
		return err
	}
	if err := writeAtomic(a.archivePath(), arch); err != nil {
		return fmt.Errorf("persist archive: %w", err)
	}
	return nil
}

// sameDocument compares two generations without their signatures: the portal signs on every
// fetch, so the same document carries a different signature each time. Both copies have already
// been verified.
func (a *Agent) sameDocument(x, y []byte) bool {
	cx, _, err1 := canon.WithoutKey(x, "signature")
	cy, _, err2 := canon.WithoutKey(y, "signature")
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
	if err := a.authorize(req); err != nil {
		return err
	}
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
	certPEM, err := os.ReadFile(a.cfg.PodCert)
	if err != nil {
		return fmt.Errorf("upload ledger: pod certificate: %w", err)
	}
	body, n, err := a.Ledger.UploadBody(certPEM)
	if err != nil || n == 0 {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, a.podURL("/ledger"), bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	if err := a.authorize(req); err != nil {
		return err
	}
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
		Accepted  int                  `json:"accepted"`
		Duplicate int                  `json:"duplicate"`
		Pending   int                  `json:"pending"`
		Lapsed    int                  `json:"lapsed"`
		LastSeq   int64                `json:"last_seq"`
		Entries   []ledger.PortalEntry `json:"entries"`
	}
	if err := json.Unmarshal(raw, &ack); err != nil {
		return fmt.Errorf("upload ledger: portal answer: %w", err)
	}
	a.mu.Lock()
	a.lastContact = a.Now()
	a.mu.Unlock()
	if err := a.Ledger.SetAck(ack.LastSeq, a.Now(), ack.Entries); err != nil {
		return err
	}
	a.Log.Printf("ledger uploaded: %d sent, %d accepted, %d lapsed on arrival; portal holds up to %d",
		n, ack.Accepted, ack.Lapsed, ack.LastSeq)
	return nil
}

// authorize adds the pod's portal API key (issued at its machine enrollment).
func (a *Agent) authorize(req *http.Request) error {
	raw, err := os.ReadFile(a.cfg.PortalTokenFile)
	if err != nil {
		return fmt.Errorf("portal API key: %w", err)
	}
	token := strings.TrimSpace(string(raw))
	if token == "" {
		return fmt.Errorf("portal API key file %s is empty", a.cfg.PortalTokenFile)
	}
	req.Header.Set("Authorization", "Bearer "+token)
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
	watched := make(chan struct{})
	go func() { defer close(watched); a.watchEndDates(ctx) }()
	defer func() { <-watched }()
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

// watchEndDates re-renders when the earliest end date of a carried rule passes, connected or
// not. It is apart from the pull loop because a pull to an unreachable portal can take its whole
// timeout. The pull loop's own reconcile covers a clock that was set during the wait.
func (a *Agent) watchEndDates(ctx context.Context) {
	for {
		var expiry <-chan time.Time
		var t *time.Timer
		if next, ok := a.Compute().NextExpiry(a.Now()); ok {
			t = time.NewTimer(max(next.Sub(a.Now()), 0))
			expiry = t.C
		}
		select {
		case <-ctx.Done():
		case <-a.wake:
		case <-expiry:
			if err := a.Reconcile(ctx); err != nil {
				a.Log.Printf("reconcile at a rule's end date: %v", err)
			}
		}
		if t != nil {
			t.Stop()
		}
		if ctx.Err() != nil {
			return
		}
	}
}

func (a *Agent) signalRules() {
	select {
	case a.wake <- struct{}{}:
	default:
	}
}

// --- local changes (pod-carriage-and-provenance.md §7.4) ---------------------------------------

// Change is a local change request. Source may also arrive as source_slot.
type Change struct {
	Source     string     `json:"source"`
	SourceSlot string     `json:"source_slot"`
	Until      *time.Time `json:"until"`
	Reason     string     `json:"reason"`
	Principals []string   `json:"principals,omitempty"`
}

func (c Change) source() string {
	if c.Source != "" {
		return c.Source
	}
	return c.SourceSlot
}

// Authorize returns the local authority entry for a client certificate CN.
func (a *Agent) Authorize(cn string) (generation.Authority, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.gen == nil {
		return generation.Authority{}, Err(403, "carriage_not_authorized",
			"This pod holds no rule generation, so it names no local authority.",
			"Local authority is named in the pod's signed rules, which this pod has not received yet.",
			"Connect the pod to the portal once, or adopt a generation file with goat-pod-api adopt.")
	}
	for _, la := range a.gen.LocalAuthority {
		if la.CertCN == cn {
			return la, nil
		}
	}
	return generation.Authority{}, Err(403, "carriage_not_authorized",
		cn+" is not local authority for this pod.",
		"Only identities the pod's signed rules name as local authority may change its rules.",
		"Ask an org lead to name you as local authority for this pod, or ask someone who is.")
}

// MayChange reports whether cn may change rules.
func (a *Agent) MayChange(cn string) bool {
	_, err := a.Authorize(cn)
	return err == nil
}

func (a *Agent) checkUntil(until *time.Time) error {
	now := a.Now()
	if until == nil || until.IsZero() {
		return Err(400, "carriage_until_required", "An end date (until) is required.",
			"Every local change ends; goat's own rules decide after that.",
			"Send until as an RFC 3339 time, for example 4, 24 or 72 hours from now.")
	}
	if !until.After(now) {
		return Err(400, "carriage_until_past", "until "+until.UTC().Format(time.RFC3339)+" is not in the future.",
			"A change that has already ended would do nothing.", "Choose a later end date.")
	}
	if limit := now.Add(a.cfg.MaxRule()); until.After(limit) {
		return Err(400, "carriage_over_max_length",
			fmt.Sprintf("until is later than the longest rule length (%d days, until %s).", a.cfg.MaxRuleDays, limit.Format(time.RFC3339)),
			"A local rule runs at most the goatnet's longest rule length.", "Choose an end date on or before "+limit.Format(time.RFC3339)+".")
	}
	return nil
}

func (a *Agent) checkReason(reason string) error {
	if strings.TrimSpace(reason) == "" {
		return Err(400, "carriage_reason_required", "A reason is required.",
			"An org lead reads the reason when ratifying this change.", "Say in a few words why this change is needed.")
	}
	if len(reason) > 500 {
		return Err(400, "invalid_request", "The reason is longer than 500 characters.", "", "Shorten the reason.")
	}
	return nil
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
	return "", "", Err(400, "invalid_request", fmt.Sprintf("Source %q is neither a slot nor a certificate name <slot>.g<N>.", s),
		"", "Give the device's slot, or the common name of its goat certificate.")
}

// Result of a local change.
type Result struct {
	Rule    *rules.Rule   `json:"rule"`
	Entry   ledger.Record `json:"entry"`
	Warning string        `json:"warning,omitempty"`
}

// Allow adds a local rule letting a source use this pod.
func (a *Agent) Allow(ctx context.Context, who generation.Authority, c Change) (Result, error) {
	if err := a.checkUntil(c.Until); err != nil {
		return Result{}, err
	}
	if err := a.checkReason(c.Reason); err != nil {
		return Result{}, err
	}
	slot, cn, err := parseSource(c.source())
	if err != nil {
		return Result{}, err
	}
	principals := c.Principals
	write := generation.DefaultWrite(slot)
	if cn != "" {
		principals = append(principals, cn)
	}
	if len(principals) == 0 {
		known, kw, ok := a.Compute().KnownSource(slot)
		if !ok {
			return Result{}, Err(400, "invalid_request", "This pod has never carried "+slot+".",
				"The pod needs the device's certificate name to admit it.", "Give the source as <slot>.g<N>, the common name on the device's goat certificate.")
		}
		principals, write = known, kw
	}
	for _, p := range principals {
		if !generation.ValidCN(p) {
			return Result{}, Err(400, "invalid_request", fmt.Sprintf("Principal %q is not a certificate name.", p), "", "")
		}
		for _, r := range a.reserved {
			if p == r {
				return Result{}, Err(400, "invalid_request", fmt.Sprintf("%q is this pod's or its fabric router's own identity.", p), "", "")
			}
		}
	}
	id, err := newRuleID()
	if err != nil {
		return Result{}, err
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionAllow, RuleID: id, SourceSlot: slot,
		Until: c.Until.UTC().Format(time.RFC3339), Reason: c.Reason, Authority: who},
		ledger.Local{Principals: principals, Write: write})
}

// Extend moves the end date of an active rule.
func (a *Agent) Extend(ctx context.Context, who generation.Authority, id string, c Change) (Result, error) {
	if err := a.checkUntil(c.Until); err != nil {
		return Result{}, err
	}
	if err := a.checkReason(c.Reason); err != nil {
		return Result{}, err
	}
	r, ok := a.Compute().Find(id)
	if !ok {
		return Result{}, Err(404, "not_found", "No rule "+id+" on this pod.", "", "List the rules with goat pod rules ls.")
	}
	switch {
	case r.State == rules.StateExpired:
		return Result{}, Err(409, "carriage_rule_expired", "Rule "+id+" has ended.", "An ended rule is reactivated, not extended.", "Use goat pod rules reactivate "+id+".")
	case r.State == rules.StateArchive || r.State == rules.StateRevoked:
		return Result{}, Err(409, "carriage_rule_archived", "Rule "+id+" is "+r.State+".", "Only an active rule is extended.", "Use goat pod rules reactivate "+id+".")
	case r.Kind == "org":
		return Result{}, Err(409, "conflict", "Rule "+id+" is an org rule and has no end date.", "", "")
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionExtend, RuleID: id, SourceSlot: r.Source,
		Until: c.Until.UTC().Format(time.RFC3339), Reason: c.Reason, Authority: who}, ledger.Local{})
}

// Reactivate brings back an archived, expired or revoked rule inside the reactivation window.
func (a *Agent) Reactivate(ctx context.Context, who generation.Authority, id string, c Change) (Result, error) {
	if err := a.checkUntil(c.Until); err != nil {
		return Result{}, err
	}
	if err := a.checkReason(c.Reason); err != nil {
		return Result{}, err
	}
	r, ok := a.Compute().Find(id)
	if !ok {
		return Result{}, Err(404, "not_found", "No rule "+id+" on this pod.", "", "List the rules with goat pod rules ls.")
	}
	if r.State == rules.StateActive {
		return Result{}, Err(409, "carriage_not_archived", "Rule "+id+" is active.", "Only an ended or revoked rule is reactivated.", "Use goat pod rules extend "+id+".")
	}
	if r.Ratification != nil && (*r.Ratification == rules.RatRatified || *r.Ratification == rules.RatLapsed) {
		return Result{}, Err(409, "conflict", "Rule "+id+" was "+*r.Ratification+" by an org lead.",
			"The portal has decided this local rule.", "Ask an org lead, or allow the source again as a new local rule.")
	}
	if r.EndedAt != nil && a.Now().Sub(*r.EndedAt) > a.cfg.Reactivation() {
		return Result{}, Err(409, "carriage_beyond_reactivation_window",
			fmt.Sprintf("Rule %s ended %s, outside the %d-day reactivation window.", id, r.EndedAt.Format(time.RFC3339), a.cfg.ReactivationDays),
			"An old rule is approved again like a new one.", "Allow the source as a new local rule.")
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionReactivate, RuleID: id, SourceSlot: r.Source,
		Until: c.Until.UTC().Format(time.RFC3339), Reason: c.Reason, Authority: who}, ledger.Local{})
}

// Revoke cuts a source off this pod at once. It holds until the portal decides it or a later
// local allow or reactivate lifts it; it takes no end date.
func (a *Agent) Revoke(ctx context.Context, who generation.Authority, slot string, c Change) (Result, error) {
	if err := a.checkReason(c.Reason); err != nil {
		return Result{}, err
	}
	if c.Until != nil {
		return Result{}, Err(400, "invalid_request", "A revoke takes no end date.",
			"A local revoke holds until an org lead decides it or a later local allow or reactivate lifts it.",
			"Send only the reason.")
	}
	if !generation.ValidSlot(slot) {
		return Result{}, Err(400, "invalid_request", fmt.Sprintf("%q is not a slot.", slot), "", "")
	}
	return a.record(ctx, ledger.Entry{Action: ledger.ActionRevoke, SourceSlot: slot, Reason: c.Reason, Authority: who}, ledger.Local{})
}

func (a *Agent) record(ctx context.Context, e ledger.Entry, local ledger.Local) (Result, error) {
	a.op.Lock()
	defer a.op.Unlock()
	local.Generation = a.held()
	stored, err := a.Ledger.Append(e, local)
	if err != nil {
		return Result{}, err
	}
	a.Log.Printf("local %s %s source %s until %q by %s", stored.Entry.Action, stored.Entry.RuleID,
		stored.Entry.SourceSlot, stored.Entry.Until, stored.Entry.Authority.Identity)
	res := Result{Entry: stored}
	defer a.signalRules()
	ruleID := stored.Entry.RuleID
	if err := a.reconcileLocked(ctx); err != nil {
		res.Warning = fmt.Sprintf("change recorded (ledger entry %d) but the router did not take it: %v",
			stored.Entry.Seq, err)
	}
	if ruleID != "" {
		if r, ok := a.Compute().Find(ruleID); ok {
			res.Rule = &r
		}
	}
	return res, nil
}

func (a *Agent) held() int64 {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.gen == nil {
		return 0
	}
	return a.gen.Generation
}

// --- status -------------------------------------------------------------------------------------

// SourceHealth is the carriage health of one source on this pod.
type SourceHealth struct {
	Slot    string `json:"slot"`
	Source  string `json:"source"`
	Carried bool   `json:"carried"`
	// Carried is what the running router admits; Change is what the rules want and it does not
	// do yet: adding, removing, changing (other certificates or keys), or empty.
	Change string `json:"change,omitempty"`
	// Carriage: ok (carried), expiring (carried, ends within 24 hours or is being removed),
	// refused (not carried).
	Carriage string     `json:"carriage"`
	State    string     `json:"state"` // carried | revoked | expired | archived
	Origin   string     `json:"origin"`
	RuleID   string     `json:"rule_id"`
	Rules    []string   `json:"rules"`
	Until    *time.Time `json:"until"`
	// Connected is null: which devices hold a session now is not measured.
	Connected  *bool    `json:"connected"`
	Principals []string `json:"principals"`
}

// You describes the signed-in caller.
type You struct {
	Identity  string `json:"identity"`
	CertCN    string `json:"cert_cn"`
	MayChange bool   `json:"may_change"`
}

// Status is GET /v1/status.
type Status struct {
	PodSlot           string     `json:"pod_slot"`
	State             string     `json:"state"` // connected | cut off | no portal
	Connected         bool       `json:"connected"`
	CutOffSince       *time.Time `json:"cut_off_since"`
	LastContact       *time.Time `json:"last_contact"`
	LastPullError     string     `json:"last_pull_error,omitempty"`
	Generation        int64      `json:"generation"`
	GenerationIssued  *time.Time `json:"generation_issued_at"`
	GenerationAgeSecs *int64     `json:"generation_age_seconds"`
	LastApply         *time.Time `json:"last_apply"`
	LastApplyError    string     `json:"last_apply_error,omitempty"`
	// Suspended: a removal could not be applied, so the pod carries no source.
	Suspended bool `json:"suspended"`
	// SourcesCarried counts the sources the running router admits; null when what it runs is not
	// known. SourcesWanted counts those the rules carry now.
	SourcesCarried *int `json:"sources_carried"`
	SourcesWanted  int  `json:"sources_wanted"`
	// ChangePending: the running router does not enforce the current rules yet.
	ChangePending    bool                   `json:"change_pending"`
	SourcesConnected *int                   `json:"sources_connected"`
	Sources          []SourceHealth         `json:"sources"`
	Pending          int                    `json:"pending"`
	Lapsed           int                    `json:"lapsed"`
	LedgerUnsent     int                    `json:"ledger_unsent"`
	LastUploadError  string                 `json:"last_upload_error,omitempty"`
	LocalAuthority   []generation.Authority `json:"local_authority"`
	You              *You                   `json:"you,omitempty"`
	MaxRuleDays      int                    `json:"max_rule_days"`
	MaxRuleHours     int                    `json:"max_rule_hours"`
	ReactivationDays int                    `json:"reactivation_days"`
}

func tp(t time.Time) *time.Time {
	if t.IsZero() {
		return nil
	}
	return &t
}

// Status reports the pod's carriage state.
func (a *Agent) Status() Status {
	rendered, res, renderErr := a.Render()
	pending := a.Pending()
	now := a.Now()
	conf, known := a.applier.Running()
	running, runErr := acl.Writers(conf)
	wanted, wantErr := acl.Writers(rendered)
	a.mu.Lock()
	defer a.mu.Unlock()
	st := Status{
		PodSlot: a.cfg.PodSlot, LastContact: tp(a.lastContact), LastPullError: a.lastPullErr,
		LastApply: tp(a.lastApply), LastApplyError: a.lastApplyErr, LastUploadError: a.uploadErr, Suspended: a.suspended,
		MaxRuleDays: a.cfg.MaxRuleDays, MaxRuleHours: a.cfg.MaxRuleDays * 24, ReactivationDays: a.cfg.ReactivationDays,
		LocalAuthority: []generation.Authority{},
	}
	switch {
	case a.cfg.PortalURL == "":
		st.State = "no portal"
	case !a.lastContact.IsZero() && now.Sub(a.lastContact) <= a.cfg.CutoffAfter():
		st.State, st.Connected = "connected", true
	default:
		st.State = "cut off"
		st.CutOffSince = tp(a.lastContact)
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
			rules.StateArchive: "archived",
		}[r.State]
		h, ok := bySource[r.Source]
		if !ok {
			h = &SourceHealth{Slot: r.Source, Source: r.Source, State: state, Origin: r.Origin, RuleID: r.ID}
			bySource[r.Source] = h
			order = append(order, r.Source)
		} else if state == "carried" || (h.State != "carried" && state == "revoked") {
			h.State, h.Origin, h.RuleID = state, r.Origin, r.ID
		}
		h.Rules = append(h.Rules, r.ID)
	}
	for _, e := range res.Effective {
		if h, ok := bySource[e.Slot]; ok {
			h.Until = e.Until
			h.Principals = e.Principals
		}
	}
	// A slot the router still admits and no rule names any more is shown all the same.
	for slot := range running {
		if bySource[slot] == nil {
			bySource[slot] = &SourceHealth{Slot: slot, Source: slot, State: "archived", Origin: "none"}
			order = append(order, slot)
		}
	}
	for _, slot := range order {
		h := bySource[slot]
		h.Carried = len(running[slot]) > 0
		switch {
		case maps.Equal(running[slot], wanted[slot]):
		case len(wanted[slot]) == 0:
			h.Change = "removing"
		case len(running[slot]) == 0:
			h.Change = "adding"
		default:
			h.Change = "changing"
		}
		switch {
		case !h.Carried:
			h.Carriage = "refused"
		case h.Change == "removing", h.Until != nil && h.Until.Sub(now) < 24*time.Hour:
			h.Carriage = "expiring"
		default:
			h.Carriage = "ok"
		}
		st.Sources = append(st.Sources, *h)
	}
	if st.Sources == nil {
		st.Sources = []SourceHealth{}
	}
	for _, p := range pending {
		if p.Ratification == rules.RatLapsed {
			st.Lapsed++
		} else {
			st.Pending++
		}
	}
	if known && runErr == nil {
		n := len(running)
		st.SourcesCarried = &n
	}
	st.SourcesWanted = len(res.Effective)
	st.ChangePending = !known || runErr != nil || renderErr != nil || wantErr != nil || acl.Removes(conf, rendered) || acl.Removes(rendered, conf)
	st.LedgerUnsent = a.Ledger.Unsent()
	return st
}

// PendingItem is one local change in GET /v1/pending.
type PendingItem struct {
	ledger.Entry
	ID           string `json:"id"`
	Source       string `json:"source"`
	At           string `json:"at"`
	Uploaded     bool   `json:"uploaded"`
	PortalState  string `json:"portal_state,omitempty"`
	PortalNote   string `json:"portal_note,omitempty"`
	Ratification string `json:"ratification"` // pending | lapsed
}

// Pending lists local changes the portal has not ratified: those not yet decided, and those
// it let lapse (on arrival or by an org lead).
func (a *Agent) Pending() []PendingItem {
	ack := a.Ledger.Acked()
	res := a.Compute()
	out := []PendingItem{}
	for _, r := range a.Ledger.Records() {
		e := r.Entry
		p := PendingItem{Entry: e, ID: e.RuleID, Source: e.SourceSlot, At: e.TS, Uploaded: e.Seq <= ack.LastSeq,
			Ratification: rules.RatPending}
		if s, ok := ack.States[e.Seq]; ok {
			p.PortalState, p.PortalNote = s.State, s.Note
			if s.State == rules.RatLapsed {
				p.Ratification = rules.RatLapsed
			}
		}
		switch res.Decisions[e.Seq] {
		case rules.RatRatified:
			continue
		case rules.RatLapsed:
			p.Ratification = rules.RatLapsed
		}
		out = append(out, p)
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
		WindowSecs: a.cfg.CutoffAfterSeconds}
	if st.SourcesCarried != nil {
		h.SampleCount = *st.SourcesCarried
	}
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

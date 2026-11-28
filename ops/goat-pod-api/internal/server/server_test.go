package server

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/router"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/testpki"
)

const (
	podSlot = "0123456789abcdef0123456789abcdef"
	srcA    = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	srcB    = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	baseCfg = `{
  access_control: {
    rules: [
      // @carriage-rules
    ],
    subjects: [
      { id: "pod-peer", cert_common_names: ["` + podSlot + `"] },
      {
        id: "fabric-peer",
        cert_common_names: ["fabric-router"],
      },
      // @carriage-subjects
    ],
    policies: [
      // @carriage-policies
    ],
  },
}`
)

// fakeRuntime refuses (stops) a configuration naming a "broken" CN.
type fakeRuntime struct {
	mu       sync.Mutex
	path     string
	restarts int
	stops    int
	fail     func(conf string) bool
	needCtx  bool
	// duringRestart runs while the old process is still the one that enforces.
	duringRestart func()
	restartErr    error
}

func (f *fakeRuntime) Restart(ctx context.Context) error {
	if f.needCtx && ctx.Err() != nil {
		return ctx.Err()
	}
	if f.duringRestart != nil {
		f.duringRestart()
	}
	if f.restartErr != nil {
		return f.restartErr
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	f.restarts++
	return nil
}

func (f *fakeRuntime) Stop(context.Context) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.stops++
	return nil
}

func (f *fakeRuntime) Healthy(context.Context) error {
	raw, _ := os.ReadFile(f.path)
	f.mu.Lock()
	defer f.mu.Unlock()
	if strings.Contains(string(raw), "broken") || (f.fail != nil && f.fail(string(raw))) {
		return fmt.Errorf("%w: test", router.ErrStopped)
	}
	return nil
}

type env struct {
	t        *testing.T
	ca       *testpki.CA
	signer   *testpki.Leaf
	podLeaf  *testpki.Leaf
	cfg      *config.Config
	agent    *agent.Agent
	rt       *fakeRuntime
	api      *httptest.Server
	portal   *httptest.Server
	gen      []byte
	mu       sync.Mutex
	uploaded [][]byte
	lastSeq  int64
}

func doc(n int, sources ...map[string]any) map[string]any {
	srcs := []any{}
	for _, s := range sources {
		srcs = append(srcs, s)
	}
	return map[string]any{
		"schema": generation.Schema, "pod_slot": podSlot, "generation": n,
		"issued_at":       time.Now().UTC().Format(time.RFC3339),
		"local_authority": []any{map[string]any{"identity": "us-dod:1234567890", "cert_cn": "lead.g1"}},
		"read":            []any{"*/**"}, "sources": srcs, "signer": "portal-signer",
	}
}

func source(slot, id string) map[string]any {
	return map[string]any{"slot": slot, "rule_id": id, "kind": "org", "until": nil,
		"principals": []any{slot + ".g1"}, "write": []any{slot + "/**", "volatile/" + slot + "/**", slot + "/**/@adv/**"}}
}

func newEnv(t *testing.T) *env {
	t.Helper()
	dir := t.TempDir()
	e := &env{t: t, ca: testpki.NewCA(t, "zagros-test-ca")}
	e.signer = e.ca.Leaf(t, "portal-signer")
	e.podLeaf = e.ca.Leaf(t, podSlot)

	// The portal: requires the pod's client certificate.
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/carriage/pods/{slot}/generation", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer test-pod-api-key" || r.PathValue("slot") != podSlot {
			http.Error(w, "not this pod", http.StatusForbidden)
			return
		}
		e.mu.Lock()
		defer e.mu.Unlock()
		_, _ = w.Write(e.gen)
	})
	mux.HandleFunc("POST /api/carriage/pods/{slot}/ledger", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer test-pod-api-key" {
			http.Error(w, "no key", http.StatusUnauthorized)
			return
		}
		body, _ := io.ReadAll(r.Body)
		var up struct {
			PodCert string         `json:"pod_cert"`
			Entries []ledger.Entry `json:"entries"`
		}
		dec := json.NewDecoder(bytes.NewReader(body))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&up); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		rows := []ledger.PortalEntry{}
		e.mu.Lock()
		defer e.mu.Unlock()
		for _, en := range up.Entries {
			if err := ledger.Check(en, e.podLeaf.Cert, podSlot, e.lastSeq+1); err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
				return
			}
			e.lastSeq = en.Seq
			rows = append(rows, ledger.PortalEntry{Seq: en.Seq, LocalRuleID: en.RuleID, State: "pending"})
		}
		e.uploaded = append(e.uploaded, body)
		_ = json.NewEncoder(w).Encode(map[string]any{"accepted": len(up.Entries), "last_seq": e.lastSeq, "entries": rows})
	})
	e.portal = httptest.NewUnstartedServer(mux)
	pool := x509.NewCertPool()
	pool.AddCert(e.ca.Cert)
	e.portal.TLS = &tls.Config{MinVersion: tls.VersionTLS12}
	portalLeaf := e.ca.Leaf(t, "localhost")
	pair, _ := tls.X509KeyPair(portalLeaf.CertPEM, portalLeaf.KeyPEM)
	e.portal.TLS.Certificates = []tls.Certificate{pair}
	e.portal.StartTLS()
	t.Cleanup(e.portal.Close)

	zenoh := filepath.Join(dir, "zenoh")
	_ = os.MkdirAll(zenoh, 0o700)
	e.cfg = &config.Config{
		PodSlot: podSlot, StateDir: filepath.Join(dir, "pod-api"), PortalURL: e.portal.URL,
		PortalCA:            testpki.Write(t, dir, "ca.pem", e.ca.PEM),
		PortalSignerCert:    testpki.Write(t, dir, "signer.pem", e.signer.CertPEM),
		PortalTokenFile:     testpki.Write(t, dir, "portal.token", []byte("test-pod-api-key\n")),
		PodCert:             testpki.Write(t, dir, "pod.pem", e.podLeaf.CertPEM),
		PodKey:              testpki.Write(t, dir, "pod.key", e.podLeaf.KeyPEM),
		CARoots:             testpki.Write(t, dir, "roots.pem", e.ca.PEM),
		PullIntervalSeconds: 60, CutoffAfterSeconds: 300, MaxRuleDays: 90, ReactivationDays: 183,
		Router: config.Router{
			BaseConfig: testpki.Write(t, zenoh, "config.base.json5", []byte(baseCfg)),
			Config:     filepath.Join(zenoh, "config.json5"), LastGood: filepath.Join(zenoh, "config.last-known-good.json5"),
			AppliedSHA: filepath.Join(zenoh, ".config.applied.sha256"), Container: "unused", HealthTimeoutSeconds: 1,
		},
	}
	e.rt = &fakeRuntime{path: e.cfg.Router.Config}
	a, err := agent.New(e.cfg, e.rt)
	if err != nil {
		t.Fatal(err)
	}
	a.Log.SetOutput(io.Discard)
	e.agent = a

	tc, err := TLSConfig(e.cfg.PodCert, e.cfg.PodKey, e.cfg.CARoots)
	if err != nil {
		t.Fatal(err)
	}
	e.api = httptest.NewUnstartedServer((&Server{Agent: a}).Handler())
	e.api.TLS = tc
	e.api.StartTLS()
	t.Cleanup(e.api.Close)
	return e
}

func (e *env) publish(d map[string]any) {
	raw, err := generation.Sign(d, e.signer.Key)
	if err != nil {
		e.t.Fatal(err)
	}
	e.mu.Lock()
	e.gen = raw
	e.mu.Unlock()
}

func (e *env) client(cn string) *http.Client {
	pool := x509.NewCertPool()
	pool.AddCert(e.ca.Cert)
	tc := &tls.Config{RootCAs: pool, MinVersion: tls.VersionTLS12, ServerName: "localhost"}
	if cn != "" {
		l := e.ca.Leaf(e.t, cn)
		pair, _ := tls.X509KeyPair(l.CertPEM, l.KeyPEM)
		tc.Certificates = []tls.Certificate{pair}
	}
	return &http.Client{Transport: &http.Transport{TLSClientConfig: tc}}
}

func (e *env) call(c *http.Client, method, path string, body any) (int, map[string]any) {
	e.t.Helper()
	var rd io.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rd = bytes.NewReader(raw)
	}
	req, _ := http.NewRequestWithContext(context.Background(), method, e.api.URL+path, rd)
	resp, err := c.Do(req)
	if err != nil {
		e.t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	raw, _ := io.ReadAll(resp.Body)
	out := map[string]any{}
	_ = json.Unmarshal(raw, &out)
	if len(out) == 0 {
		out["raw"] = string(raw)
	}
	return resp.StatusCode, out
}

func (e *env) liveConfig() string {
	raw, _ := os.ReadFile(e.cfg.Router.Config)
	return string(raw)
}

func TestEndToEnd(t *testing.T) {
	e := newEnv(t)
	ctx := context.Background()
	lead := e.client("lead.g1")

	// Before any generation nobody may change rules.
	if code, _ := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "until": time.Now().Add(time.Hour), "reason": "x"}); code != http.StatusForbidden {
		t.Fatalf("change without a generation: %d", code)
	}

	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(e.liveConfig(), `"cert_common_names":["`+srcA+`.g1"]`) {
		t.Fatalf("source a not rendered:\n%s", e.liveConfig())
	}
	if strings.Contains(e.liveConfig(), `"cert_common_names":["fabric-router"]`) {
		t.Fatal("fabric router admitted as a carried principal")
	}

	// Status needs a client certificate; any trusted one reads it.
	if code, _ := e.call(e.client(""), "GET", "/v1/status", nil); code != http.StatusUnauthorized {
		t.Fatalf("status without certificate: %d", code)
	}
	code, st := e.call(e.client("someone.g1"), "GET", "/v1/status", nil)
	if code != 200 || st["state"] != "connected" || st["generation"].(float64) != 1 || st["sources_carried"].(float64) != 1 {
		t.Fatalf("status: %d %v", code, st)
	}

	// A lower generation is refused; the same number with other contents is refused.
	e.publish(doc(1, source(srcA, "cr_a"), source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "differs") {
		t.Fatalf("same-number different generation: %v", err)
	}

	// Only local authority may change rules; until and reason are bounded.
	someone := e.client("someone.g1")
	if code, _ := e.call(someone, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "until": time.Now().Add(time.Hour), "reason": "x"}); code != http.StatusForbidden {
		t.Fatalf("non-authority allow: %d", code)
	}
	if code, out := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "until": time.Now().Add(91 * 24 * time.Hour), "reason": "x"}); code != 400 || out["error"].(map[string]any)["code"] != "carriage_over_max_length" {
		t.Fatalf("over 90 days: %d %v", code, out)
	}
	if code, out := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "until": time.Now().Add(time.Hour)}); code != 400 || out["error"].(map[string]any)["code"] != "carriage_reason_required" {
		t.Fatalf("no reason: %d %v", code, out)
	}
	if code, out := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "reason": "x"}); code != 400 || out["error"].(map[string]any)["code"] != "carriage_until_required" {
		t.Fatalf("no until: %d %v", code, out)
	}
	if code, _ := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB, "until": time.Now().Add(time.Hour), "reason": "x"}); code != 400 {
		t.Fatalf("unknown bare slot: %d", code)
	}
	code, out := e.call(lead, "POST", "/v1/rules", map[string]any{"source": srcB + ".g1", "until": time.Now().Add(4 * time.Hour), "reason": "met in the field"})
	if code != http.StatusCreated {
		t.Fatalf("allow: %d %v", code, out)
	}
	ruleID := out["rule"].(map[string]any)["id"].(string)
	if out["rule"].(map[string]any)["ratification"] != "pending" {
		t.Fatalf("new local rule: %v", out["rule"])
	}
	if !strings.Contains(e.liveConfig(), `"cert_common_names":["`+srcB+`.g1"]`) {
		t.Fatal("local rule not rendered")
	}
	if a := out["entry"].(map[string]any)["entry"].(map[string]any)["authority"].(map[string]any); a["identity"] != "us-dod:1234567890" {
		t.Fatalf("ledger authority %v", a)
	}

	// Extend, then revoke, then reactivate.
	if code, out := e.call(lead, "POST", "/v1/rules/"+ruleID+"/extend", map[string]any{"until": time.Now().Add(24 * time.Hour), "reason": "longer"}); code != 200 {
		t.Fatalf("extend: %d %v", code, out)
	}
	if code, out := e.call(lead, "POST", "/v1/rules/cr_a/extend", map[string]any{"until": time.Now().Add(24 * time.Hour), "reason": "x"}); code != http.StatusConflict {
		t.Fatalf("extend org rule: %d %v", code, out)
	}
	if code, out := e.call(lead, "POST", "/v1/sources/"+srcA+"/revoke", map[string]any{"reason": "x", "until": time.Now().Add(time.Hour)}); code != 400 {
		t.Fatalf("revoke with until: %d %v", code, out)
	}
	if code, out := e.call(lead, "POST", "/v1/sources/"+srcA+"/revoke", map[string]any{"reason": "lost device"}); code != 200 {
		t.Fatalf("revoke: %d %v", code, out)
	}
	if strings.Contains(e.liveConfig(), `"cert_common_names":["`+srcA+`.g1"]`) {
		t.Fatal("revoked source still rendered")
	}
	if code, out := e.call(lead, "POST", "/v1/rules/cr_a/reactivate", map[string]any{"until": time.Now().Add(24 * time.Hour), "reason": "found"}); code != 200 {
		t.Fatalf("reactivate: %d %v", code, out)
	}
	if !strings.Contains(e.liveConfig(), `"cert_common_names":["`+srcA+`.g1"]`) {
		t.Fatal("reactivated source not rendered")
	}

	// A change the router refuses is recorded, rolled back and reported.
	before := e.liveConfig()
	code, out = e.call(lead, "POST", "/v1/rules", map[string]any{"source": "cccccccccccccccccccccccccccccccc", "principals": []string{"broken.g1"}, "until": time.Now().Add(time.Hour), "reason": "x"})
	if code != http.StatusAccepted || out["warning"] == nil {
		t.Fatalf("refused change: %d %v", code, out)
	}
	if e.liveConfig() != before {
		t.Fatal("router config not rolled back")
	}

	// Pending, then upload: the portal verifies every line and acknowledges.
	_, pend := e.call(lead, "GET", "/v1/pending", nil)
	if n := len(pend["pending"].([]any)); n != 5 {
		t.Fatalf("pending %d", n)
	}
	if err := e.agent.Upload(ctx); err != nil {
		t.Fatal(err)
	}
	_, pend = e.call(lead, "GET", "/v1/pending", nil)
	if first := pend["pending"].([]any)[0].(map[string]any); first["uploaded"] != true || first["portal_state"] != "pending" {
		t.Fatalf("pending after upload: %v", first)
	}

	// The next generation decides the acknowledged local rules: b ratified as a named rule.
	until := time.Now().Add(48 * time.Hour).UTC().Format(time.RFC3339)
	named := source(srcB, "cr_b")
	named["kind"], named["until"] = "named", until
	g2 := doc(2, source(srcA, "cr_a"), named)
	// The allow the router refused (seq 5) lapses.
	g2["local_decisions"] = []any{
		map[string]any{"seq": 1, "local_rule_id": ruleID, "state": "ratified"},
		map[string]any{"seq": 5, "state": "lapsed"},
	}
	e.publish(g2)
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	_, rl := e.call(lead, "GET", "/v1/rules", nil)
	states := map[string]string{}
	for _, r := range rl["rules"].([]any) {
		m := r.(map[string]any)
		states[m["id"].(string)] = fmt.Sprint(m["state"], "/", m["ratification"])
	}
	if states[ruleID] != "archived/ratified" || states["cr_b"] != "active/<nil>" {
		t.Fatalf("after ratification: %v", states)
	}
	_, pend = e.call(lead, "GET", "/v1/pending", nil)
	if n := len(pend["pending"].([]any)); n != 4 {
		t.Fatalf("pending after ratification %d", n)
	}
	if last := pend["pending"].([]any)[3].(map[string]any); last["ratification"] != "lapsed" {
		t.Fatalf("lapsed entry not shown: %v", last)
	}
	if code, st := e.call(lead, "GET", "/v1/status", nil); code != 200 || st["lapsed"].(float64) != 1 ||
		st["you"].(map[string]any)["may_change"] != true {
		t.Fatalf("status after decisions: %v", st)
	}
	// The local revoke of a still holds until its end; local revokes are not handed over.
	if states["cr_a"] != "active/pending" {
		// a was reactivated after the revoke, so it is carried.
		t.Fatalf("cr_a state %s", states["cr_a"])
	}

	// Gatepost placeholder at /.
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, e.api.URL+"/", nil)
	resp, err := e.client("").Do(req)
	if err != nil {
		t.Fatal(err)
	}
	page, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if !bytes.Equal(page, gatepostPage) || !strings.Contains(string(page), `data-cli="pod rules allow"`) {
		t.Fatal("Gatepost page not served at /")
	}

	// A restart of the service keeps the generation and the ledger.
	again, err := agent.New(e.cfg, e.rt)
	if err != nil {
		t.Fatal(err)
	}
	if again.Status().Generation != 2 || len(again.Ledger.Records()) != 5 {
		t.Fatal("state not kept across restart")
	}
}

func TestLowerGenerationRefused(t *testing.T) {
	e := newEnv(t)
	ctx := context.Background()
	e.publish(doc(5, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	e.publish(doc(4))
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "lower") {
		t.Fatalf("lower generation: %v", err)
	}
	if e.agent.Status().Generation != 5 {
		t.Fatal("lower generation adopted")
	}
	// A bad signature is refused and changes nothing.
	e.mu.Lock()
	e.gen = bytes.Replace(e.gen, []byte(`"generation":4`), []byte(`"generation":9`), 1)
	e.mu.Unlock()
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "signature") {
		t.Fatalf("tampered generation: %v", err)
	}
}

func TestRejectedGenerationRollsBack(t *testing.T) {
	e := newEnv(t)
	ctx := context.Background()
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	good := e.liveConfig()
	bad := source(srcB, "cr_b")
	bad["principals"] = []any{"broken.g1"}
	e.publish(doc(2, source(srcA, "cr_a"), bad))
	if err := e.agent.Pull(ctx); err == nil {
		t.Fatal("generation the router refused was reported as adopted")
	}
	if e.liveConfig() != good || e.agent.Status().Generation != 1 {
		t.Fatal("not rolled back to generation 1")
	}
	restarts := e.rt.restarts
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "rejected") {
		t.Fatalf("second pull of the rejected generation: %v", err)
	}
	if e.rt.restarts != restarts {
		t.Fatal("rejected generation retried")
	}
}

// The portal signs the document on every fetch and ECDSA signatures are randomized, so the same
// generation arrives with a different signature each time. Only the signed body identifies it,
// and the signature on each copy is checked before any comparison.
func TestSameGenerationAgain(t *testing.T) {
	e := newEnv(t)
	ctx := context.Background()
	body := doc(3, source(srcA, "cr_a"))
	e.publish(body)
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	first, config, restarts := string(e.gen), e.liveConfig(), e.rt.restarts

	unchanged := func(name string) {
		t.Helper()
		if e.liveConfig() != config || e.rt.restarts != restarts || e.agent.Status().Generation != 3 {
			t.Fatalf("%s changed the running state", name)
		}
	}

	e.publish(body)
	if string(e.gen) == first {
		t.Fatal("the re-signed copy is byte-identical; this test needs a new signature")
	}
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatalf("re-signed same body refused: %v", err)
	}
	if msg := e.agent.Status().LastPullError; msg != "" {
		t.Fatalf("re-signed same body left an error: %s", msg)
	}
	unchanged("a re-signed same body")

	// The same body signed by a key other than the portal signer's.
	other := e.ca.Leaf(t, "portal-signer")
	forged, err := generation.Sign(body, other.Key)
	if err != nil {
		t.Fatal(err)
	}
	e.gen = forged
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "signature") {
		t.Fatalf("same body, bad signature: %v", err)
	}
	if msg := e.agent.Status().LastPullError; !strings.Contains(msg, "signature") {
		t.Fatalf("bad signature not reported: %q", msg)
	}
	unchanged("a bad signature")

	// The same number with another, correctly signed body.
	e.publish(doc(3, source(srcA, "cr_a"), source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "differs") {
		t.Fatalf("same number, different body: %v", err)
	}
	if msg := e.agent.Status().LastPullError; !strings.Contains(msg, "differs") {
		t.Fatalf("different body not reported: %q", msg)
	}
	unchanged("a different body")

	// The held copy again clears the reported error.
	e.publish(body)
	if err := e.agent.Pull(ctx); err != nil || e.agent.Status().LastPullError != "" {
		t.Fatalf("held copy after refusals: %v %q", err, e.agent.Status().LastPullError)
	}
}

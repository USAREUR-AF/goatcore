package api

// ADR 1251 pod carriage: rules, the signed generation, keyless enrollment and
// a pod's local-authority ledger, end to end over HTTP against Postgres.

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/carriage"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
)

const carriageOpKey = "carriage-op-key"

// routerConfig is the shape of the router template's carriage part: the slot
// subject markers and one carriage policy per pool slot, bound to carry-<a>
// (or, for a role render that predates the fragment's carry- subjects, to the
// inert carry-<a>-none).
func routerConfig(salt string, n int, none bool) string {
	var b strings.Builder
	b.WriteString("access_control: {\n  subjects: [\n      // BEGIN: slot subjects\n      // END: slot subjects\n  ],\n  policies: [\n")
	for k := 0; k < n; k++ {
		a := registry.SlotAnchor(salt, k)
		suffix := ""
		if none {
			suffix = "-none"
		}
		b.WriteString("      {\n        rules: [\"slot-w-" + a + "\"],\n        subjects: [\"carry-" + a + suffix + "\"],\n      },\n")
	}
	b.WriteString("  ],\n}\n")
	return b.String()
}

type carriageFixture struct {
	t          *testing.T
	configPath string
	ts         *httptest.Server
	reg        *registry.Registry
	signerCert *x509.Certificate
	caCert     *x509.Certificate
	caKey      *ecdsa.PrivateKey
	alice, bob string // bearer keys of two custodians: alice in org-a, bob in org-b
}

func ecKey(t *testing.T) *ecdsa.PrivateKey {
	t.Helper()
	k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return k
}

func issueCert(t *testing.T, cn string, key *ecdsa.PrivateKey, parent *x509.Certificate, parentKey *ecdsa.PrivateKey, isCA bool) *x509.Certificate {
	t.Helper()
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(time.Now().UnixNano()), Subject: pkix.Name{CommonName: cn},
		NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(24 * time.Hour),
		BasicConstraintsValid: true, IsCA: isCA,
		KeyUsage:    x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
	}
	if parent == nil {
		parent, parentKey = tmpl, key
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, parent, &key.PublicKey, parentKey)
	if err != nil {
		t.Fatal(err)
	}
	c, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func certPEM(c *x509.Certificate) []byte {
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: c.Raw})
}

func newCarriageFixture(t *testing.T) *carriageFixture {
	t.Helper()
	dir := t.TempDir()
	write := func(name string, b []byte) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, b, 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}
	signerKey := ecKey(t)
	signerCert := issueCert(t, "portal-carriage-signer", signerKey, nil, nil, false)
	der, _ := x509.MarshalPKCS8PrivateKey(signerKey)
	caKey := ecKey(t)
	caCert := issueCert(t, "zagros-test-ca", caKey, nil, nil, true)
	roster := `{"orgs":[
	  {"id":"org-a","members":[{"handle":"alice","type":"person","state":"active","vendor_tag":"alice"}]},
	  {"id":"org-b","members":[{"handle":"bob","type":"person","state":"active","vendor_tag":"bob"}]}]}`
	cfg := &config.Config{
		OperatorAPIKey:      carriageOpKey,
		IdentityModel:       config.IdentityModelPerUUID,
		SlotPoolSalt:        "carriage-test",
		SlotPoolSize:        16,
		ZenohRestFake:       true,
		ACLConfigPath:       write("config.json5", []byte(routerConfig("carriage-test", 16, false))),
		ACLReloadFake:       true,
		CertSignFake:        true,
		VendorBundleDir:     filepath.Join(dir, "bundles"),
		SchemaDir:           filepath.Join(dir, "schemas"),
		CertSignURIPrefix:   "urn:goat:zagros:org=",
		RosterPath:          write("roster.json", []byte(roster)),
		GovernBootstrapOpen: true,
		CarriageSignerKey:   write("signer.key", pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der})),
		CarriageSignerCert:  write("signer.pem", certPEM(signerCert)),
		CARootCert:          write("root.pem", certPEM(caCert)),
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	if _, err := reg.InitializeSlotPool(t.Context(), cfg.SlotPoolSalt, cfg.SlotPoolSize); err != nil {
		t.Fatal(err)
	}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatal(err)
	}
	minter := &certmint.Minter{Fake: true, VendorBundleDir: cfg.VendorBundleDir, URIPrefix: cfg.CertSignURIPrefix}
	srv := New(cfg, reg, zenohproxy.New("", true), minter, &acl.Renderer{ConfigPath: cfg.ACLConfigPath, ReloadFake: true}, schemas, nil, nil)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)
	ctx := t.Context()
	for _, g := range []struct{ who, org string }{{"alice", "org-a"}, {"bob", "org-b"}} {
		if err := reg.SetMachineGrant(ctx, g.who, 8, g.org, "", "op"); err != nil {
			t.Fatal(err)
		}
	}
	return &carriageFixture{
		t: t, ts: ts, reg: reg, configPath: cfg.ACLConfigPath, signerCert: signerCert, caCert: caCert, caKey: caKey,
		alice: seedPrincipal(t, reg, "alice", false, ""),
		bob:   seedPrincipal(t, reg, "bob", false, ""),
	}
}

// call sends a JSON request and returns status and decoded body.
func (f *carriageFixture) call(method, path, bearer string, body any) (int, map[string]any, []byte) {
	f.t.Helper()
	var rd io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		rd = bytes.NewReader(b)
	}
	req, _ := http.NewRequest(method, f.ts.URL+path, rd)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+bearer)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		f.t.Fatal(err)
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	var m map[string]any
	_ = json.Unmarshal(raw, &m)
	return resp.StatusCode, m, raw
}

func (f *carriageFixture) mustCall(want int, method, path, bearer string, body any) map[string]any {
	f.t.Helper()
	code, m, raw := f.call(method, path, bearer, body)
	if code != want {
		f.t.Fatalf("%s %s: status %d, want %d: %s", method, path, code, want, raw)
	}
	return m
}

func codeOf(m map[string]any) string { s, _ := m["code"].(string); return s }

// enroll enrolls a machine for custodian (operator on behalf) and returns its
// slot and API key.
func (f *carriageFixture) enroll(handle, custodian string, extra map[string]any) (string, string, map[string]any) {
	f.t.Helper()
	body := map[string]any{"machine_handle": handle, "node_class": "SENSOR-FEED", "autonomy": "supervised", "custodian": custodian}
	for k, v := range extra {
		body[k] = v
	}
	m := f.mustCall(http.StatusCreated, "POST", "/api/machines/enroll", carriageOpKey, body)
	slot, _ := m["slot_id"].(string)
	key, _ := m["api_key"].(string)
	return slot, key, m
}

func (f *carriageFixture) registerPod(pod, org string, authority []map[string]any) map[string]any {
	f.t.Helper()
	return f.mustCall(http.StatusCreated, "POST", "/api/carriage/pods", carriageOpKey,
		map[string]any{"pod_slot": pod, "org": org, "local_authority": authority})
}

func (f *carriageFixture) generation(pod, key string) *carriage.Generation {
	f.t.Helper()
	code, _, raw := f.call("GET", "/api/carriage/pods/"+pod+"/generation", key, nil)
	if code != http.StatusOK {
		f.t.Fatalf("generation: %d %s", code, raw)
	}
	g, err := carriage.VerifyGeneration(raw, f.signerCert)
	if err != nil {
		f.t.Fatalf("generation does not verify: %v\n%s", err, raw)
	}
	return g
}

func sourceSlots(g *carriage.Generation) map[string]carriage.Source {
	out := map[string]carriage.Source{}
	for _, s := range g.Sources {
		out[s.Slot] = s
	}
	return out
}

func (f *carriageFixture) cn(slot string) string {
	f.t.Helper()
	s, err := f.reg.SlotByID(f.t.Context(), slot)
	if err != nil {
		f.t.Fatal(err)
	}
	return registry.LeafCN(slot, s.Generation)
}

func days(n int) string {
	return time.Now().Add(time.Duration(n) * 24 * time.Hour).UTC().Format(time.RFC3339)
}

func TestCarriage_OrgRuleGenerationSignedMonotonicOptOut(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	dev1, dev1Key, _ := f.enroll("dev-1", "alice", nil)
	dev2, _, _ := f.enroll("dev-2", "alice", nil)
	bobDev, _, _ := f.enroll("bob-dev", "bob", nil)

	reg := f.registerPod(pod, "org-a", []map[string]any{{"identity": "us-dod:1234567890", "cert_cn": "lead-a"}})
	rule, _ := reg["org_rule"].(map[string]any)
	if rule == nil || rule["kind"] != "org" || rule["state"] != "active" || rule["until"] != nil {
		t.Fatalf("org rule not created on pod registration: %v", reg)
	}

	g := f.generation(pod, podKey)
	if g.Generation != 1 || g.PodSlot != pod || g.Signer != "portal-carriage-signer" {
		t.Fatalf("generation header wrong: %+v", g)
	}
	if len(g.Read) != 1 || g.Read[0] != "*/**" {
		t.Fatalf("read = %v, want [*/**]", g.Read)
	}
	if len(g.LocalAuthority) != 1 || g.LocalAuthority[0].CertCN != "lead-a" {
		t.Fatalf("local_authority = %+v", g.LocalAuthority)
	}
	src := sourceSlots(g)
	if _, ok := src[pod]; ok {
		t.Fatal("the pod's own slot is listed as a carried source")
	}
	if _, ok := src[bobDev]; ok {
		t.Fatal("an org-b device is carried by an org-a pod without a rule")
	}
	s1, ok := src[dev1]
	if !ok || s1.Kind != "org" || s1.Until != nil || len(s1.Principals) != 1 || s1.Principals[0] != f.cn(dev1) {
		t.Fatalf("dev-1 entry wrong: %+v", s1)
	}
	if s1.Write[0] != dev1+"/**" || len(s1.Write) != 3 {
		t.Fatalf("dev-1 write = %v", s1.Write)
	}
	if _, ok := src[dev2]; !ok {
		t.Fatal("dev-2 missing")
	}

	// Unchanged rules: same generation number.
	if g2 := f.generation(pod, podKey); g2.Generation != 1 || g2.IssuedAt != g.IssuedAt {
		t.Fatalf("generation moved without a change: %d", g2.Generation)
	}
	// Opt-out by the custodian changes the document: number goes up, dev-2 leaves.
	f.mustCall(http.StatusOK, "PUT", "/api/carriage/sources/"+dev2+"/org-rule", f.alice, map[string]any{"opt_out": true})
	g3 := f.generation(pod, podKey)
	if g3.Generation != 2 {
		t.Fatalf("generation after opt-out = %d, want 2", g3.Generation)
	}
	if _, ok := sourceSlots(g3)[dev2]; ok {
		t.Fatal("an opted-out source is still carried under the org rule")
	}
	// Bob is not dev-1's custodian.
	if code, m, _ := f.call("PUT", "/api/carriage/sources/"+dev1+"/org-rule", f.bob, map[string]any{"opt_out": true}); code != http.StatusForbidden || codeOf(m) != "carriage_not_authorized" {
		t.Fatalf("opt-out by a stranger: %d %v", code, m)
	}

	// Only the pod's own key fetches its generation.
	for _, k := range []string{carriageOpKey, dev1Key, f.alice} {
		if code, _, _ := f.call("GET", "/api/carriage/pods/"+pod+"/generation", k, nil); code != http.StatusForbidden {
			t.Fatalf("generation served to a key that is not the pod's: %d", code)
		}
	}
	// Re-registering for another org is refused.
	if code, m, _ := f.call("POST", "/api/carriage/pods", carriageOpKey, map[string]any{"pod_slot": pod, "org": "org-b"}); code != http.StatusConflict || codeOf(m) != "carriage_pod_org_mismatch" {
		t.Fatalf("re-register for another org: %d %v", code, m)
	}
	// Pods are listed for signed-in callers.
	if m := f.mustCall(http.StatusOK, "GET", "/api/carriage/pods", f.bob, nil); len(m["pods"].([]any)) != 1 {
		t.Fatalf("pods list: %v", m)
	}
}

func TestCarriage_NamedRuleConsentAndCap(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	dev1, _, _ := f.enroll("dev-1", "alice", nil)
	bobDev, _, _ := f.enroll("bob-dev", "bob", nil)
	f.registerPod(pod, "org-a", nil)

	rule := map[string]any{"source_slot": bobDev, "pod_slot": pod, "kind": "named", "until": days(91), "reason": "exercise support"}
	if code, m, _ := f.call("POST", "/api/carriage/rules", f.bob, rule); code != http.StatusBadRequest || codeOf(m) != "carriage_over_max_length" {
		t.Fatalf("91-day rule: %d %v", code, m)
	}
	delete(rule, "until")
	if code, m, _ := f.call("POST", "/api/carriage/rules", f.bob, rule); code != http.StatusBadRequest || codeOf(m) != "carriage_until_required" {
		t.Fatalf("rule without until: %d %v", code, m)
	}
	rule["until"] = days(30)
	// Alice is not bob-dev's custodian: she cannot grant it.
	if code, m, _ := f.call("POST", "/api/carriage/rules", f.alice, rule); code != http.StatusForbidden || codeOf(m) != "carriage_not_authorized" {
		t.Fatalf("grant by the pod side: %d %v", code, m)
	}
	m := f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules", f.bob, rule)
	if m["state"] != "proposed" {
		t.Fatalf("a rule the pod's custodian has not accepted is %v", m["state"])
	}
	id := m["id"].(string)
	if _, ok := sourceSlots(f.generation(pod, podKey))[bobDev]; ok {
		t.Fatal("a proposed rule is rendered")
	}
	if code, _, _ := f.call("POST", "/api/carriage/rules/"+id+"/accept", f.bob, nil); code != http.StatusForbidden {
		t.Fatal("the source side accepted for the pod")
	}
	if m := f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+id+"/accept", f.alice, nil); m["state"] != "active" {
		t.Fatalf("accept: %v", m)
	}
	s, ok := sourceSlots(f.generation(pod, podKey))[bobDev]
	if !ok || s.Kind != "named" || s.Until == nil || s.RuleID != id {
		t.Fatalf("named rule not rendered: %+v", s)
	}
	// A rule granted by someone who holds both sides is active at once.
	m = f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules", f.alice,
		map[string]any{"source_slot": dev1, "pod_slot": pod, "kind": "temporary", "until": days(3), "reason": "drill"})
	if m["state"] != "active" {
		t.Fatalf("both-custodian grant is %v", m["state"])
	}
	// The org rule still wins for dev-1 (no end beats an end).
	if s := sourceSlots(f.generation(pod, podKey))[dev1]; s.Kind != "org" {
		t.Fatalf("dev-1 rendered under %s, want the org rule", s.Kind)
	}
	if m := f.mustCall(http.StatusOK, "GET", "/api/carriage/rules?pod="+pod+"&state=active", f.bob, nil); len(m["rules"].([]any)) != 3 {
		t.Fatalf("active rules for the pod: %v", m)
	}
}

func TestCarriage_ExpiryArchivesAndReactivationWindow(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	bobDev, _, _ := f.enroll("bob-dev", "bob", nil)
	f.registerPod(pod, "org-a", nil)
	t.Cleanup(func() { registry.CarriageClock = time.Now })

	newRule := func() string {
		m := f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules", f.bob,
			map[string]any{"source_slot": bobDev, "pod_slot": pod, "until": days(10), "reason": "liaison"})
		f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+m["id"].(string)+"/accept", f.alice, nil)
		return m["id"].(string)
	}
	id := newRule()
	registry.CarriageClock = func() time.Time { return time.Now().Add(11 * 24 * time.Hour) }
	m := f.mustCall(http.StatusOK, "GET", "/api/carriage/rules?state=archived", f.bob, nil)
	rules := m["rules"].([]any)
	if len(rules) != 1 || rules[0].(map[string]any)["archive_reason"] != "expired" {
		t.Fatalf("expired rule not archived: %v", m)
	}
	if _, ok := sourceSlots(f.generation(pod, podKey))[bobDev]; ok {
		t.Fatal("an expired rule is rendered")
	}
	if code, m, _ := f.call("POST", "/api/carriage/rules/"+id+"/extend", f.bob, map[string]any{"until": days(20)}); code != http.StatusConflict || codeOf(m) != "carriage_rule_expired" {
		t.Fatalf("extend an expired rule: %d %v", code, m)
	}
	n := f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules/"+id+"/reactivate", f.bob, nil)
	if n["reactivated_from"] != id || n["state"] != "active" {
		t.Fatalf("reactivate: %v", n)
	}
	if code, m, _ := f.call("POST", "/api/carriage/rules/"+id+"/reactivate", f.bob, nil); code != http.StatusConflict || codeOf(m) != "carriage_already_reactivated" {
		t.Fatalf("second reactivation: %d %v", code, m)
	}
	// Archived rows are kept: the original is still there.
	if _, err := f.reg.CarriageRule(t.Context(), id); err != nil {
		t.Fatalf("archived rule deleted: %v", err)
	}

	registry.CarriageClock = time.Now
	id2 := newRule()
	registry.CarriageClock = func() time.Time { return time.Now().Add(200 * 24 * time.Hour) }
	if code, m, _ := f.call("POST", "/api/carriage/rules/"+id2+"/reactivate", f.bob, nil); code != http.StatusConflict || codeOf(m) != "carriage_beyond_reactivation_window" {
		t.Fatalf("reactivation past the window: %d %v", code, m)
	}
}

func TestCarriage_KeylessEnrollMintsNothingAndPodsRequest(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	adapter, _, _ := f.enroll("asterix-adapter", "alice", nil)
	f.registerPod(pod, "org-a", nil)

	code, m, _ := f.call("POST", "/api/machines/enroll", carriageOpKey, map[string]any{
		"machine_handle": "radar-1", "node_class": "SENSOR-FEED", "autonomy": "autonomous", "custodian": "alice", "keyless": true,
	})
	if code != http.StatusBadRequest || codeOf(m) != "invalid_keyless" {
		t.Fatalf("keyless without adapters: %d %v", code, m)
	}
	radar, key, resp := f.enroll("radar-1", "alice", map[string]any{
		"keyless": true, "adapters": []string{adapter},
		"receive": map[string]any{"interface": "udp", "port": 10009, "sender": "192.0.2.10:5000"},
	})
	if key != "" || resp["bundle_url"] != "" || resp["leaf_cn"] != "" || resp["keyless"] != true {
		t.Fatalf("keyless enrollment minted something: %v", resp)
	}
	leaves, err := f.reg.SlotLeaves(t.Context(), radar)
	if err != nil || len(leaves) != 0 {
		t.Fatalf("keyless slot has leaves: %v %v", leaves, err)
	}
	if q, _ := f.reg.MachineQuotaFor(t.Context(), "alice"); q.Used != 3 {
		t.Fatalf("keyless source should hold a slot under its custodian: used %d", q.Used)
	}
	s, ok := sourceSlots(f.generation(pod, podKey))[radar]
	if !ok || len(s.Principals) != 1 || s.Principals[0] != f.cn(adapter) {
		t.Fatalf("keyless source principals = %+v, want the adapter's leaf CN", s)
	}

	// pods at enrollment: alice holds both sides, so the named rule is active.
	_, _, resp = f.enroll("dev-3", "alice", map[string]any{"pods": []string{pod}})
	_ = resp
	code, m, raw := f.call("POST", "/api/machines/enroll", f.bob, map[string]any{
		"machine_handle": "bob-4", "node_class": "SENSOR-FEED", "autonomy": "manual", "pods": []string{pod},
	})
	if code != http.StatusCreated {
		t.Fatalf("bob enroll: %d %s", code, raw)
	}
	cr := m["carriage"].([]any)[0].(map[string]any)
	if cr["state"] != "proposed" || cr["rule_id"] == "" {
		t.Fatalf("bob's pods request should be proposed for the pod's custodian: %v", cr)
	}
}

// ledgerFor signs entries with the pod's key and wraps them in an upload.
type podSigner struct {
	key  *ecdsa.PrivateKey
	cert *x509.Certificate
	slot string
}

func (f *carriageFixture) podIdentity(slot string) podSigner {
	k := ecKey(f.t)
	return podSigner{key: k, cert: issueCert(f.t, f.cn(slot), k, f.caCert, f.caKey, false), slot: slot}
}

func (p podSigner) upload(t *testing.T, entries ...carriage.LedgerEntry) map[string]any {
	t.Helper()
	var raws []json.RawMessage
	for _, e := range entries {
		e.V, e.PodSlot = 1, p.slot
		if e.TS == "" {
			e.TS = time.Now().UTC().Format(time.RFC3339)
		}
		if e.Reason == "" {
			e.Reason = "field change"
		}
		if e.Authority.Identity == "" {
			e.Authority = carriage.Authority{Identity: "us-dod:1234567890", CertCN: "lead-a"}
		}
		s, err := carriage.SignLedgerEntry(p.key, e)
		if err != nil {
			t.Fatal(err)
		}
		b, _ := json.Marshal(s)
		raws = append(raws, b)
	}
	return map[string]any{"pod_cert": string(certPEM(p.cert)), "entries": raws}
}

func TestCarriage_RatifiedLocalRevokeIsAnExclusion(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	dev1, _, _ := f.enroll("dev-1", "alice", nil)
	f.registerPod(pod, "org-a", []map[string]any{{"identity": "us-dod:1234567890", "cert_cn": "lead-a"}})
	ps := f.podIdentity(pod)
	res := f.mustCall(http.StatusOK, "POST", "/api/carriage/pods/"+pod+"/ledger", podKey,
		ps.upload(t, carriage.LedgerEntry{Seq: 1, Action: "revoke", SourceSlot: dev1, Reason: "device lost"}))
	lid := res["entries"].([]any)[0].(map[string]any)["local_rule_id"].(string)
	r := f.mustCall(http.StatusOK, "POST", "/api/carriage/local-rules/"+lid+"/ratify", f.alice, nil)
	exID, _ := r["ratified_rule_id"].(string)
	ex, err := f.reg.CarriageRule(t.Context(), exID)
	if err != nil || ex.Kind != registry.CarriageKindExclusion || deref(ex.SourceSlot) != dev1 {
		t.Fatalf("ratified revoke did not record an exclusion: %+v %v", ex, err)
	}
	if _, ok := sourceSlots(f.generation(pod, podKey))[dev1]; ok {
		t.Fatal("an excluded source is still carried under the org rule")
	}
	listed := false
	for _, raw := range f.mustCall(http.StatusOK, "GET", "/api/carriage/rules?pod="+pod, f.bob, nil)["rules"].([]any) {
		if raw.(map[string]any)["kind"] == "exclusion" {
			listed = true
		}
	}
	if !listed {
		t.Fatal("the exclusion is not visible in the rules list")
	}
	if code, m, _ := f.call("POST", "/api/carriage/rules/"+exID+"/extend", f.alice, map[string]any{"until": days(5)}); code != http.StatusBadRequest || codeOf(m) != "carriage_exclusion_no_end" {
		t.Fatalf("extend an exclusion: %d %v", code, m)
	}
	if code, _, _ := f.call("POST", "/api/carriage/rules/"+exID+"/revoke", f.bob, nil); code != http.StatusForbidden {
		t.Fatal("someone other than the source's custodian lifted the exclusion")
	}
	f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+exID+"/revoke", f.alice, nil)
	if _, ok := sourceSlots(f.generation(pod, podKey))[dev1]; !ok {
		t.Fatal("lifting the exclusion did not bring the source back under the org rule")
	}
}

func TestCarriage_LedgerRatifyLapseAndRevokeWins(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	bobDev, _, _ := f.enroll("bob-dev", "bob", nil)
	bobDev2, _, _ := f.enroll("bob-dev-2", "bob", nil)
	f.registerPod(pod, "org-a", []map[string]any{{"identity": "us-dod:1234567890", "cert_cn": "lead-a"}})
	ps := f.podIdentity(pod)
	path := "/api/carriage/pods/" + pod + "/ledger"
	tomorrow := days(1)

	// A central rule for bob-dev-2, so a later revoke has something to win over.
	m := f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules", f.bob,
		map[string]any{"source_slot": bobDev2, "pod_slot": pod, "until": days(30), "reason": "liaison"})
	central := m["id"].(string)
	f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+central+"/accept", f.alice, nil)

	body := ps.upload(t,
		carriage.LedgerEntry{Seq: 1, Action: "allow", RuleID: "lr_a", SourceSlot: bobDev, Until: tomorrow},
		carriage.LedgerEntry{Seq: 2, Action: "allow", RuleID: "lr_b", SourceSlot: bobDev, Until: tomorrow,
			Authority: carriage.Authority{Identity: "us-dod:9999999999", CertCN: "someone"}},
		carriage.LedgerEntry{Seq: 3, Action: "allow", RuleID: "lr_c", SourceSlot: bobDev, Until: days(100)},
		carriage.LedgerEntry{Seq: 4, Action: "extend", RuleID: central, SourceSlot: bobDev2, Until: tomorrow},
	)
	if code, _, _ := f.call("POST", path, carriageOpKey, body); code != http.StatusForbidden {
		t.Fatal("ledger accepted from a key that is not the pod's")
	}
	res := f.mustCall(http.StatusOK, "POST", path, podKey, body)
	if res["pending"].(float64) != 2 || res["lapsed"].(float64) != 2 {
		t.Fatalf("ingest: %v", res)
	}
	again := f.mustCall(http.StatusOK, "POST", path, podKey, body)
	if again["duplicate"].(float64) != 4 || again["accepted"].(float64) != 0 {
		t.Fatalf("re-upload is not idempotent: %v", again)
	}
	conflict := ps.upload(t, carriage.LedgerEntry{Seq: 1, Action: "allow", SourceSlot: bobDev2, Until: tomorrow})
	if code, m, _ := f.call("POST", path, podKey, conflict); code != http.StatusConflict || codeOf(m) != "ledger_conflict" {
		t.Fatalf("different entry at a used seq: %d %v", code, m)
	}
	forged := ps.upload(t, carriage.LedgerEntry{Seq: 9, Action: "allow", SourceSlot: bobDev, Until: tomorrow})
	forged["entries"].([]json.RawMessage)[0] = json.RawMessage(strings.Replace(string(forged["entries"].([]json.RawMessage)[0]), "field change", "field chance", 1))
	if code, m, _ := f.call("POST", path, podKey, forged); code != http.StatusBadRequest || codeOf(m) != "ledger_invalid" {
		t.Fatalf("tampered entry: %d %v", code, m)
	}
	stranger := f.podIdentity(bobDev) // signed by a leaf that is not the pod's
	stranger.slot = pod
	if code, m, _ := f.call("POST", path, podKey, stranger.upload(t, carriage.LedgerEntry{Seq: 10, Action: "allow", SourceSlot: bobDev, Until: tomorrow})); code != http.StatusBadRequest || codeOf(m) != "ledger_invalid" {
		t.Fatalf("entry under another leaf: %d %v", code, m)
	}

	// Org leads of the pod's org see the pending list; others see nothing.
	pending := f.mustCall(http.StatusOK, "GET", "/api/carriage/local-rules?state=pending", f.alice, nil)["local_rules"].([]any)
	if len(pending) != 2 {
		t.Fatalf("pending for alice: %v", pending)
	}
	if got := f.mustCall(http.StatusOK, "GET", "/api/carriage/local-rules", f.bob, nil)["local_rules"].([]any); len(got) != 0 {
		t.Fatalf("bob sees another org's pending local rules: %v", got)
	}
	ids := map[float64]string{}
	for _, raw := range pending {
		lr := raw.(map[string]any)
		ids[lr["seq"].(float64)] = lr["id"].(string)
	}

	// Goat's revocations win: revoking the central rule lapses seq 4 at once…
	rv := f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+central+"/revoke", f.bob, nil)
	if rv["local_rules_lapsed"].(float64) != 1 {
		t.Fatalf("revoke did not lapse the pending local rule: %v", rv)
	}
	// …and a local rule arriving after the revoke lapses on arrival.
	late := f.mustCall(http.StatusOK, "POST", path, podKey,
		ps.upload(t, carriage.LedgerEntry{Seq: 5, Action: "allow", RuleID: "lr_d", SourceSlot: bobDev2, Until: tomorrow}))
	if e := late["entries"].([]any)[0].(map[string]any); e["state"] != "lapsed" || !strings.Contains(e["note"].(string), "revoked") {
		t.Fatalf("local rule after a central revoke: %v", e)
	}

	// Bob does not lead org-a: he cannot ratify.
	if code, _, _ := f.call("POST", "/api/carriage/local-rules/"+ids[1]+"/ratify", f.bob, nil); code != http.StatusForbidden {
		t.Fatal("ratified by someone outside the pod's org")
	}
	r := f.mustCall(http.StatusOK, "POST", "/api/carriage/local-rules/"+ids[1]+"/ratify", f.alice, nil)
	if r["state"] != "ratified" || r["ratified_rule_id"] == nil {
		t.Fatalf("ratify: %v", r)
	}
	nr, err := f.reg.CarriageRule(t.Context(), r["ratified_rule_id"].(string))
	if err != nil || nr.State != registry.CarriageActive || nr.Origin != "ratified" || deref(nr.SourceSlot) != bobDev {
		t.Fatalf("ratified rule: %+v %v", nr, err)
	}
	gr := f.generation(pod, podKey)
	if s, ok := sourceSlots(gr)[bobDev]; !ok || s.Until == nil || *s.Until != tomorrow {
		t.Fatalf("a ratified local rule is not in the next generation with its own end date: %+v", s)
	}
	seen := map[int64]string{}
	for _, d := range gr.LocalDecisions {
		seen[d.Seq] = d.State
	}
	if seen[1] != "ratified" || seen[2] != "lapsed" || seen[4] != "lapsed" || seen[5] != "lapsed" {
		t.Fatalf("local_decisions = %+v", gr.LocalDecisions)
	}
	if code, m, _ := f.call("POST", "/api/carriage/local-rules/"+ids[1]+"/lapse", f.alice, nil); code != http.StatusConflict || codeOf(m) != "local_rule_not_pending" {
		t.Fatalf("lapse after ratify: %d %v", code, m)
	}

	// Lapse: a decision issues a new generation even when the sources are
	// unchanged (the pod reads the next higher generation as the answer).
	l := f.mustCall(http.StatusOK, "POST", path, podKey,
		ps.upload(t, carriage.LedgerEntry{Seq: 6, Action: "reactivate", RuleID: "lr_a", SourceSlot: bobDev, Until: tomorrow}))
	if l["last_seq"].(float64) != 6 {
		t.Fatalf("last_seq = %v, want 6", l["last_seq"])
	}
	lid := l["entries"].([]any)[0].(map[string]any)["local_rule_id"].(string)
	before := f.generation(pod, podKey).Generation
	if m := f.mustCall(http.StatusOK, "POST", "/api/carriage/local-rules/"+lid+"/lapse", f.alice, nil); m["state"] != "lapsed" {
		t.Fatalf("lapse: %v", m)
	}
	if after := f.generation(pod, podKey).Generation; after != before+1 {
		t.Fatalf("generation after a lapse = %d, want %d", after, before+1)
	}

	// A revoke may carry no end date and is not held to the rule length.
	rv7 := f.mustCall(http.StatusOK, "POST", path, podKey,
		ps.upload(t, carriage.LedgerEntry{Seq: 7, Action: "revoke", SourceSlot: bobDev}))
	if e := rv7["entries"].([]any)[0].(map[string]any); e["state"] != "pending" {
		t.Fatalf("revoke without until: %v", e)
	}
	if got := f.mustCall(http.StatusOK, "GET", "/api/carriage/local-rules", f.alice, nil)["local_rules"].([]any); got[0].(map[string]any)["pod_org"] != "org-a" {
		t.Fatalf("local rule list lacks pod_org: %v", got)
	}
}

// carryOf reads the carry-<slot> CNs from the live router config.
func (f *carriageFixture) carryOf(slot string) []string {
	f.t.Helper()
	b, err := os.ReadFile(f.configPath)
	if err != nil {
		f.t.Fatal(err)
	}
	m := regexp.MustCompile(`id: "carry-` + slot + `",\n\s*cert_common_names: \[([^\]]*)\]`).FindStringSubmatch(string(b))
	if m == nil {
		f.t.Fatalf("no carry-%s subject in the live config", slot)
	}
	var out []string
	for _, c := range regexp.MustCompile(`"([^"]+)"`).FindAllStringSubmatch(m[1], -1) {
		if !strings.HasSuffix(c[1], ".unassigned") {
			out = append(out, c[1])
		}
	}
	return out
}

func TestCarriage_CoreCarriersFollowRulesAndDriftNamesDifferences(t *testing.T) {
	f := newCarriageFixture(t)
	pod, podKey, _ := f.enroll("pod-1", "alice", nil)
	dev1, _, _ := f.enroll("dev-1", "alice", nil)
	bobDev, _, _ := f.enroll("bob-dev", "bob", nil)
	adapter, _, _ := f.enroll("asterix-adapter", "alice", nil)
	radar, _, _ := f.enroll("radar-1", "alice", map[string]any{
		"keyless": true, "adapters": []string{adapter},
		"receive": map[string]any{"interface": "udp", "port": 10009, "sender": "192.0.2.10:5000"},
	})
	// Before any pod: only the adapter carries the keyless source.
	if got := f.carryOf(radar); len(got) != 1 || got[0] != f.cn(adapter) {
		t.Fatalf("keyless source carriers before any pod = %v, want the adapter", got)
	}
	if got := f.carryOf(dev1); len(got) != 0 {
		t.Fatalf("dev-1 carried before any rule: %v", got)
	}
	f.registerPod(pod, "org-a", []map[string]any{{"identity": "us-dod:1234567890", "cert_cn": "lead-a"}})
	podCN := f.cn(pod)
	if got := f.carryOf(dev1); len(got) != 1 || got[0] != podCN {
		t.Fatalf("org rule: carry-dev1 = %v, want [%s]", got, podCN)
	}
	want := []string{f.cn(adapter), podCN}
	sort.Strings(want)
	if got := f.carryOf(radar); strings.Join(got, ",") != strings.Join(want, ",") {
		t.Fatalf("carry-radar = %v, want %v", got, want)
	}
	if got := f.carryOf(bobDev); len(got) != 0 {
		t.Fatalf("an org-b device is carried by an org-a pod: %v", got)
	}
	if got := f.carryOf(pod); len(got) != 0 {
		t.Fatalf("the pod carries itself: %v", got)
	}

	// Health: never pulled, then current, then behind after a rule change.
	podHealth := func() map[string]any {
		for _, p := range f.mustCall(http.StatusOK, "GET", "/api/carriage/pods", f.bob, nil)["pods"].([]any) {
			pm := p.(map[string]any)
			if pm["pod_slot"] == pod {
				return pm["health"].(map[string]any)
			}
		}
		t.Fatal("pod missing from the list")
		return nil
	}
	if h := podHealth(); h["state"] != "never_pulled" {
		t.Fatalf("health before any pull: %v", h)
	}
	f.generation(pod, podKey)
	if h := podHealth(); h["state"] != "current" || h["last_pulled_generation"].(float64) != 1 || h["pull_age_seconds"] == nil {
		t.Fatalf("health after a pull: %v", h)
	}

	// Opt-out reaches the core fragment at once.
	f.mustCall(http.StatusOK, "PUT", "/api/carriage/sources/"+dev1+"/org-rule", f.alice, map[string]any{"opt_out": true})
	if got := f.carryOf(dev1); len(got) != 0 {
		t.Fatalf("an opted-out source is still carried at the core: %v", got)
	}
	if h := podHealth(); h["state"] != "behind" || h["rules_changed_since_pull"] != true {
		t.Fatalf("health after a rule change: %v", h)
	}

	// A named rule for bob's device, accepted by the pod side.
	m := f.mustCall(http.StatusCreated, "POST", "/api/carriage/rules", f.bob,
		map[string]any{"source_slot": bobDev, "pod_slot": pod, "until": days(10), "reason": "liaison"})
	if got := f.carryOf(bobDev); len(got) != 0 {
		t.Fatal("a proposed rule reached the core")
	}
	f.mustCall(http.StatusOK, "POST", "/api/carriage/rules/"+m["id"].(string)+"/accept", f.alice, nil)
	if got := f.carryOf(bobDev); len(got) != 1 || got[0] != podCN {
		t.Fatalf("accepted named rule: carry-bobdev = %v", got)
	}

	// Drift: in sync now; a hand edit of the config is named.
	drift := f.mustCall(http.StatusOK, "GET", "/api/carriage/drift", carriageOpKey, nil)
	if drift["in_sync"] != true {
		t.Fatalf("drift right after a render: %v", drift)
	}
	if code, _, _ := f.call("GET", "/api/carriage/drift", f.alice, nil); code != http.StatusForbidden {
		t.Fatal("drift served to a non-operator")
	}
	b, _ := os.ReadFile(f.configPath)
	edited := strings.Replace(string(b), `id: "carry-`+bobDev+`",
        cert_common_names: ["`+podCN+`"],`, `id: "carry-`+bobDev+`",
        cert_common_names: ["`+f.cn(dev1)+`"],`, 1)
	if edited == string(b) {
		t.Fatal("test setup: carry-bobdev not found in the config")
	}
	if err := os.WriteFile(f.configPath, []byte(edited), 0o644); err != nil {
		t.Fatal(err)
	}
	rep, err := CarriageDrift(t.Context(), f.reg, &config.Config{RosterPath: filepath.Join(filepath.Dir(f.configPath), "roster.json")}, f.configPath)
	if err != nil {
		t.Fatal(err)
	}
	kinds := map[string]bool{}
	for _, d := range rep.Differences {
		kinds[d.Kind+":"+d.CN] = d.Slot == bobDev
	}
	if rep.InSync || !kinds["missing:"+podCN] || !kinds["extra:"+f.cn(dev1)] || len(rep.Differences) != 2 {
		t.Fatalf("drift after a hand edit = %+v", rep.Differences)
	}

	// A role render older than the fragment's carry- subjects: the policies
	// bind -none, which the drift check names for every slot someone carries.
	if err := os.WriteFile(f.configPath, []byte(routerConfig("carriage-test", 16, true)), 0o644); err != nil {
		t.Fatal(err)
	}
	f.mustCall(http.StatusOK, "PUT", "/api/carriage/sources/"+dev1+"/org-rule", f.alice, map[string]any{"opt_out": false})
	rep, err = CarriageDrift(t.Context(), f.reg, &config.Config{RosterPath: filepath.Join(filepath.Dir(f.configPath), "roster.json")}, f.configPath)
	if err != nil {
		t.Fatal(err)
	}
	inert := map[string]bool{}
	for _, d := range rep.Differences {
		if d.Kind == DriftInert {
			inert[d.Slot] = true
		}
	}
	if !inert[dev1] || !inert[bobDev] || !inert[radar] || inert[pod] {
		t.Fatalf("inert bindings named = %v (differences %+v)", inert, rep.Differences)
	}

	// The ledger upload time is recorded for health.
	ps := f.podIdentity(pod)
	f.mustCall(http.StatusOK, "POST", "/api/carriage/pods/"+pod+"/ledger", podKey,
		ps.upload(t, carriage.LedgerEntry{Seq: 1, Action: "revoke", SourceSlot: dev1}))
	if h := podHealth(); h["last_ledger_at"] == nil || h["pending_local_rules"].(float64) != 1 {
		t.Fatalf("health after a ledger upload: %v", h)
	}
}

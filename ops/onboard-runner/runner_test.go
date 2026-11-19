package main

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// fakePortal is an in-memory stand-in for the goat-zenoh-portal onboard contract:
// it serves the approved/onboarded queues + claim + result, tracking state so a
// test can assert what the runner did.
type fakePortal struct {
	mu          sync.Mutex
	proposals   map[string]*proposal // id -> proposal (status mutated by claim/result)
	claims      []string             // ids claimed, in order
	results     map[string]string    // id -> terminal status written back
	resultKeys  map[string]string    // id -> setup_key written back on the result
	resultBun   map[string]string    // id -> bundle_path written back on the result
	resultColA  map[string]string    // id -> collab_account written back
	resultColL  map[string]string    // id -> collab_setup_link written back
	resultColOn map[string]bool      // id -> collab_activated written back
	claimFail   map[string]bool      // ids where claim should 409 (race loser)
	resultErrs  map[string]string    // id -> error written back on the result
	resultAudit map[string]string    // id -> operator_audit + " " + operator_audit_error written back

	offboardClaims    []string        // ids offboard-claimed, in order
	offboardClaimFail map[string]bool // ids where offboard-claim should 409
}

func newFakePortal() *fakePortal {
	return &fakePortal{
		proposals:   map[string]*proposal{},
		results:     map[string]string{},
		resultKeys:  map[string]string{},
		resultBun:   map[string]string{},
		resultColA:  map[string]string{},
		resultColL:  map[string]string{},
		resultColOn: map[string]bool{},
		claimFail:   map[string]bool{},
		resultErrs:  map[string]string{},
		resultAudit: map[string]string{},

		offboardClaimFail: map[string]bool{},
	}
}

func (f *fakePortal) add(p proposal) {
	f.mu.Lock()
	defer f.mu.Unlock()
	cp := p
	f.proposals[p.ID] = &cp
}

func (f *fakePortal) handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/onboard/proposals", func(w http.ResponseWriter, r *http.Request) {
		status := r.URL.Query().Get("status")
		f.mu.Lock()
		var out []proposal
		for _, p := range f.proposals {
			if status == "" || p.Status == status {
				out = append(out, *p)
			}
		}
		f.mu.Unlock()
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("POST /api/onboard/proposals/{id}/claim", func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		f.mu.Lock()
		defer f.mu.Unlock()
		p, ok := f.proposals[id]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		if f.claimFail[id] || p.Status != "approved" {
			w.WriteHeader(http.StatusConflict)
			return
		}
		p.Status = "onboarding"
		f.claims = append(f.claims, id)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": id, "status": "onboarding"})
	})
	mux.HandleFunc("POST /api/onboard/proposals/{id}/offboard-claim", func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		f.mu.Lock()
		defer f.mu.Unlock()
		p, ok := f.proposals[id]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		if f.offboardClaimFail[id] || p.Status != "offboard-requested" {
			w.WriteHeader(http.StatusConflict)
			return
		}
		p.Status = "offboarding"
		f.offboardClaims = append(f.offboardClaims, id)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": id, "status": "offboarding"})
	})
	mux.HandleFunc("POST /api/onboard/proposals/{id}/result", func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		var body struct {
			Status          string `json:"status"`
			Error           string `json:"error"`
			SetupKey        string `json:"setup_key"`
			BundlePath      string `json:"bundle_path"`
			CollabAccount   string `json:"collab_account"`
			CollabSetupLink string `json:"collab_setup_link"`
			CollabActivated bool   `json:"collab_activated"`
			OperatorAudit   string `json:"operator_audit"`
			OperatorAuditEr string `json:"operator_audit_error"`
		}
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &body)
		f.mu.Lock()
		defer f.mu.Unlock()
		p, ok := f.proposals[id]
		if !ok || (p.Status != "onboarding" && p.Status != "offboarding") {
			w.WriteHeader(http.StatusConflict)
			return
		}
		p.Status = body.Status
		f.results[id] = body.Status
		f.resultErrs[id] = body.Error
		f.resultAudit[id] = strings.TrimSpace(body.OperatorAudit + " " + body.OperatorAuditEr)
		f.resultKeys[id] = body.SetupKey
		f.resultBun[id] = body.BundlePath
		f.resultColA[id] = body.CollabAccount
		f.resultColL[id] = body.CollabSetupLink
		f.resultColOn[id] = body.CollabActivated
		_ = json.NewEncoder(w).Encode(map[string]string{"id": id, "status": body.Status})
	})
	return mux
}

func testRunner(t *testing.T, f *fakePortal) (*runner, *httptest.Server) {
	t.Helper()
	srv := httptest.NewServer(f.handler())
	t.Cleanup(srv.Close)
	return &runner{
		pc:       newPortalClient(srv.URL, "test-key", 5*time.Second),
		repoRoot: t.TempDir(),
		runnerID: "onboard-runner-test",
		log:      slog.New(slog.NewTextHandler(io.Discard, nil)),
	}, srv
}

const testManifest = `{"identity":{"handle":"jane","email":"person-55@example.org"},"zenoh":{"enabled":true,"trial":{"org":"jane"}},"netbird":{"usage_limit":5}}`

func TestRunOnce_HappyPath(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "person-55@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)

	var minted []string
	r.mint = func(_ context.Context, p proposal, path string) (mintResult, error) {
		if !strings.HasSuffix(path, "ops/operators/zagros/jane.yml") {
			t.Errorf("manifest path = %q", path)
		}
		minted = append(minted, p.Handle)
		return mintResult{}, nil
	}

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if len(minted) != 1 || minted[0] != "jane" {
		t.Errorf("minted = %v, want [jane]", minted)
	}
	if f.results["prop-1"] != "onboarded" {
		t.Errorf("result = %q, want onboarded", f.results["prop-1"])
	}
}

func TestRunOnce_CollabWriteback(t *testing.T) {
	// A manifest that opted into collab: the runner provisions collab BEFORE the
	// writeback and carries the account + one-time link + activated flag in it, so
	// the console's welcome email includes collab access.
	f := newFakePortal()
	m := `{"identity":{"handle":"jane","email":"person-55@example.org"},"collab":{"enabled":true},"netbird":{"usage_limit":5}}`
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "person-55@example.org", Status: "approved", ManifestJSON: m})
	r, _ := testRunner(t, f)
	r.mint = func(context.Context, proposal, string) (mintResult, error) {
		return mintResult{SetupKey: "SETUP-KEY"}, nil
	}
	r.collab = &stubCollab{} // Provision → (handle, ".../ui/reset?token=T", nil)

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if f.results["prop-1"] != "onboarded" {
		t.Fatalf("result = %q, want onboarded", f.results["prop-1"])
	}
	if f.resultColA["prop-1"] != "jane" {
		t.Errorf("collab_account = %q, want jane", f.resultColA["prop-1"])
	}
	if !strings.Contains(f.resultColL["prop-1"], "reset?token=T") {
		t.Errorf("collab_setup_link = %q, want a reset link", f.resultColL["prop-1"])
	}
	if !f.resultColOn["prop-1"] {
		t.Error("collab_activated should be true")
	}
}

func TestRunOnce_NoCollabWhenNotOptedIn(t *testing.T) {
	// Provider configured, but the manifest didn't opt in → no collab in the writeback.
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "person-55@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.mint = func(context.Context, proposal, string) (mintResult, error) { return mintResult{SetupKey: "K"}, nil }
	r.collab = &stubCollab{}
	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if f.resultColA["prop-1"] != "" || f.resultColOn["prop-1"] {
		t.Errorf("collab must be absent when not opted in: acct=%q active=%v", f.resultColA["prop-1"], f.resultColOn["prop-1"])
	}
}

func TestRunOnce_DryRunTouchesNothing(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "person-55@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.dryRun = true
	minted := 0
	r.mint = func(context.Context, proposal, string) (mintResult, error) { minted++; return mintResult{}, nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if minted != 0 {
		t.Errorf("dry-run minted %d, want 0", minted)
	}
	if len(f.claims) != 0 {
		t.Errorf("dry-run claimed %v, want none", f.claims)
	}
	if f.proposals["prop-1"].Status != "approved" {
		t.Errorf("dry-run changed status to %q", f.proposals["prop-1"].Status)
	}
}

func TestRunOnce_ClaimLostSkipsMint(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Status: "approved", ManifestJSON: testManifest})
	f.claimFail["prop-1"] = true // another runner won the race
	r, _ := testRunner(t, f)
	minted := 0
	r.mint = func(context.Context, proposal, string) (mintResult, error) { minted++; return mintResult{}, nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if minted != 0 {
		t.Errorf("claim-lost minted %d, want 0", minted)
	}
}

func TestRunOnce_DuplicateEmailSkipsMint(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-old", Env: "zagros", Handle: "jane1", Email: "person-55@example.org", Status: "onboarded"})
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane2", Email: "person-55@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	minted := 0
	r.mint = func(context.Context, proposal, string) (mintResult, error) { minted++; return mintResult{}, nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if minted != 0 {
		t.Errorf("duplicate email minted %d, want 0 (case-insensitive match)", minted)
	}
	if f.results["prop-1"] != "onboarded" {
		t.Errorf("duplicate result = %q, want onboarded", f.results["prop-1"])
	}
}

// TestRunOnce_DuplicateEmailCopiesOriginalKey: when a duplicate is finalized
// without minting, it must carry the ORIGINAL's setup key + bundle path so its
// welcome package points at the same enrollment — not a keyless onboarded row.
func TestRunOnce_DuplicateEmailCopiesOriginalKey(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-old", Env: "zagros", Handle: "jane1", Email: "person-55@example.org",
		Status: "onboarded", SetupKey: "ORIG-KEY", BundlePath: "/api/org-bundle/abc/tok.tar.gz"})
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane2", Email: "person-55@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	minted := 0
	r.mint = func(context.Context, proposal, string) (mintResult, error) { minted++; return mintResult{}, nil }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if minted != 0 {
		t.Fatalf("duplicate minted %d, want 0", minted)
	}
	if f.results["prop-1"] != "onboarded" {
		t.Fatalf("duplicate result = %q, want onboarded", f.results["prop-1"])
	}
	if f.resultKeys["prop-1"] != "ORIG-KEY" {
		t.Errorf("duplicate setup_key = %q, want the original's ORIG-KEY (not keyless)", f.resultKeys["prop-1"])
	}
	if f.resultBun["prop-1"] != "/api/org-bundle/abc/tok.tar.gz" {
		t.Errorf("duplicate bundle_path = %q, want the original's", f.resultBun["prop-1"])
	}
}

func TestRunOnce_MintFailureRecordsFailed(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "j@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.mint = func(context.Context, proposal, string) (mintResult, error) { return mintResult{}, errBoom }

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if f.results["prop-1"] != "failed" {
		t.Errorf("result = %q, want failed", f.results["prop-1"])
	}
}

func TestRunOnce_OnlyEnvFilter(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-nova", Env: "nova", Handle: "x", Status: "approved", ManifestJSON: testManifest})
	f.add(proposal{ID: "prop-zagros", Env: "zagros", Handle: "y", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.onlyEnv = "zagros"
	var minted []string
	r.mint = func(_ context.Context, p proposal, _ string) (mintResult, error) {
		minted = append(minted, p.Env)
		return mintResult{}, nil
	}

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if len(minted) != 1 || minted[0] != "zagros" {
		t.Errorf("only-env minted %v, want [zagros]", minted)
	}
}

func TestManifestJSONToYAML_IntsAndShape(t *testing.T) {
	y, err := manifestJSONToYAML(testManifest)
	if err != nil {
		t.Fatal(err)
	}
	s := string(y)
	if !strings.Contains(s, "usage_limit: 5") || strings.Contains(s, "usage_limit: 5.0") {
		t.Errorf("usage_limit not rendered as int:\n%s", s)
	}
	if !strings.Contains(s, "handle: jane") {
		t.Errorf("missing handle:\n%s", s)
	}
}

var errBoom = &mintErr{}

type mintErr struct{}

func (*mintErr) Error() string { return "native mint: boom" }

// The mint prints both credentials; both must be scraped. The portal key is the
// one that was being dropped on the floor: minted into the bundle, never carried
// to the console, so the welcome email told people to go find it in the Portal —
// which is the thing the key unlocks.
func TestParseMintResult(t *testing.T) {
	out := []byte(`onboarded ` + "`user9x`" + ` (native, no Ansible) — NetBird + Ory + Zenoh
  write anchor: 4f250e30ac723662f668cbe941f4b0b6
  uri_san:      urn:goat:zagros:org=4f250e30ac723662f668cbe941f4b0b6
  api_key:      vp_4f250e30ac723662f668cbe941f4b0b6_175df7a58fb7d647cea509c1
  setup key:    FF8E395A-4BD8-49FA-998C-C3AEFDF71003  (key ` + "`user9x`" + `)
  ory login:    person-3@example.org  (password ` + "`hunter2`" + `)
`)
	got := parseMintResult(out)
	if got.SetupKey != "FF8E395A-4BD8-49FA-998C-C3AEFDF71003" {
		t.Errorf("setup key = %q", got.SetupKey)
	}
	if got.PortalAPIKey != "vp_4f250e30ac723662f668cbe941f4b0b6_175df7a58fb7d647cea509c1" {
		t.Errorf("portal api key = %q", got.PortalAPIKey)
	}
}

// A manifest with no fabric identity mints no portal key. The scrape must come
// back empty rather than grabbing a neighbouring field — a wrong key is worse
// than none, because it fails at the Portal with no clue why.
func TestParseMintResult_noFabricLegYieldsNoPortalKey(t *testing.T) {
	out := []byte(`onboarded ` + "`user364x`" + ` (native, no Ansible) — NetBird + Ory
  setup key:    0576A667-8A4E-46C2-9013-51281748EAB2  (key ` + "`user364x`" + `)
  ory login:    person-3@example.org  (password ` + "`hunter2`" + `)
`)
	got := parseMintResult(out)
	if got.SetupKey == "" {
		t.Error("setup key should still parse")
	}
	if got.PortalAPIKey != "" {
		t.Errorf("expected no portal key, got %q", got.PortalAPIKey)
	}
}

// The mint STATES its write anchor; the runner carries it to the console rather
// than leaving the console to re-derive it from the bundle path. That derivation
// yields nothing for a slot with no bundle — which is what mint-on-collect
// produces — and yields it silently.
func TestParseMintResult_ScrapesTheStatedWriteAnchor(t *testing.T) {
	out := []byte("onboarded `fra-land-bde` (native, no Ansible) — Zenoh + NetBird\n" +
		"  write anchor: 8aa78aa40cc102ecaa5bd5c1f5d02e15\n" +
		"  uri_san:      urn:goat:zagros:org=fra-land-bde\n" +
		"  api_key:      vp_8aa78aa40cc102ecaa5bd5c1f5d02e15_deadbeef\n" +
		"  setup key:    ABC123  (key `fra-land-bde`)\n")
	r := parseMintResult(out)
	if r.SlotAnchor != "8aa78aa40cc102ecaa5bd5c1f5d02e15" {
		t.Errorf("SlotAnchor = %q, want the stated anchor", r.SlotAnchor)
	}
	if r.SetupKey != "ABC123" || r.PortalAPIKey != "vp_8aa78aa40cc102ecaa5bd5c1f5d02e15_deadbeef" {
		t.Errorf("scraping the anchor disturbed the other fields: %+v", r)
	}
}

// A deferred mint prints its anchor and NO api_key. The anchor must still be
// carried — it is the only way the person's slot ever gets linked to them.
func TestParseMintResult_AnchorSurvivesADeferredMint(t *testing.T) {
	out := []byte("onboarded `isr-recon-det` (native, no Ansible) — Zenoh\n" +
		"  write anchor: 11223344556677889900112233445566\n" +
		"  setup key:    XYZ789  (key `isr-recon-det`)\n")
	r := parseMintResult(out)
	if r.SlotAnchor != "11223344556677889900112233445566" {
		t.Errorf("SlotAnchor = %q, want the anchor from a mint that issued no key", r.SlotAnchor)
	}
	if r.PortalAPIKey != "" {
		t.Errorf("PortalAPIKey = %q, want empty — the mint issued none", r.PortalAPIKey)
	}
}

// A per-vendor mint's anchor is a human prefix, not a slot. Reporting it as a
// slot anchor would have the console PUT /api/slots/<prefix>/identity, which
// addresses a slot that does not exist.
func TestParseMintResult_IgnoresANonSlotAnchor(t *testing.T) {
	out := []byte("onboarded `acme` (native, no Ansible) — Zenoh\n" +
		"  write anchor: acme\n" +
		"  api_key:      vp_acme_deadbeef\n")
	if r := parseMintResult(out); r.SlotAnchor != "" {
		t.Errorf("SlotAnchor = %q, want empty for a per-vendor prefix", r.SlotAnchor)
	}
}

// F-512: the deploy rendered the management API's address wrong and every claim
// turned into a failed row. With the address unreachable the runner must claim
// nothing — the proposal stays approved for the next healthy cycle.
func TestRunOnce_MgmtUnreachableClaimsNothing(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "j@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	minted := false
	r.mint = func(context.Context, proposal, string) (mintResult, error) {
		minted = true
		return mintResult{SetupKey: "k"}, nil
	}
	dead := httptest.NewServer(http.NotFoundHandler())
	dead.Close() // a closed port: the F-512 shape (connect refused)
	r.mgmtURL = dead.URL

	err := r.runOnce(context.Background())
	if err == nil {
		t.Fatal("runOnce must report the preflight failure")
	}
	if minted {
		t.Error("mint ran with the management API unreachable")
	}
	if got := f.results["prop-1"]; got != "" {
		t.Errorf("proposal result = %q, want untouched (stays approved)", got)
	}
}

// A 401 from the management API is the PASS: reached, demanding auth. The mint
// proceeds exactly as before.
func TestRunOnce_MgmtAnswers401ThenMints(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "j@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.mint = func(context.Context, proposal, string) (mintResult, error) { return mintResult{SetupKey: "k"}, nil }
	mgmt := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.URL.Path != "/api/groups" {
			t.Errorf("probe path = %s", req.URL.Path)
		}
		w.WriteHeader(http.StatusUnauthorized)
	}))
	t.Cleanup(mgmt.Close)
	r.mgmtURL = mgmt.URL

	if err := r.runOnce(context.Background()); err != nil {
		t.Fatal(err)
	}
	if f.results["prop-1"] != "onboarded" {
		t.Errorf("result = %q, want onboarded", f.results["prop-1"])
	}
}

// An address that answers but is not the management API (an nginx, say) is
// refused the same way: only 200/401/403 count as reached.
func TestRunOnce_MgmtWrongServiceClaimsNothing(t *testing.T) {
	f := newFakePortal()
	f.add(proposal{ID: "prop-1", Env: "zagros", Handle: "jane", Email: "j@example.org", Status: "approved", ManifestJSON: testManifest})
	r, _ := testRunner(t, f)
	r.mint = func(context.Context, proposal, string) (mintResult, error) { return mintResult{SetupKey: "k"}, nil }
	nginx := httptest.NewServer(http.NotFoundHandler())
	t.Cleanup(nginx.Close)
	r.mgmtURL = nginx.URL

	if err := r.runOnce(context.Background()); err == nil {
		t.Fatal("a 404 from the mapped address must fail the preflight")
	}
	if got := f.results["prop-1"]; got != "" {
		t.Errorf("proposal result = %q, want untouched", got)
	}
}

// The proposal note must carry goat's terminal `error:` line, not the warnings
// that precede it — the exact output shape of the zagros failures.
func TestMintErrSnippet_LastErrorLineWins(t *testing.T) {
	out := []byte("warning: operator-action audit emit failed (/home/runner/.goat/contexts/default/profile.toml missing router_endpoint — re-run `goat profile init`); recording to local fallback log\n" +
		"warning: operator-action audit emit failed (/home/runner/.goat/contexts/default/profile.toml missing router_endpoint — re-run `goat profile init`); recording to local fallback log\n" +
		"error: netbird: NetBird GET /api/groups failed: network or parse error: error sending request for url (https://netbird.zagros.example/api/groups)\n")
	got := mintErrSnippet(out)
	if !strings.HasPrefix(got, "error: netbird: NetBird GET /api/groups failed") {
		t.Errorf("snippet = %q, want the error line", got)
	}
	if strings.Contains(got, "warning:") {
		t.Errorf("snippet carries the warning, not the error: %q", got)
	}
	// No error line: the tail of the output, not the head.
	long := []byte(strings.Repeat("x", 400) + "TAIL")
	if got := mintErrSnippet(long); !strings.HasSuffix(got, "TAIL") {
		t.Errorf("no-error-line snippet = %q, want the tail", got[:20])
	}
}

// The handset key (ADR 1175) is a second line under its own label. Both keys
// must be scraped from a mint that prints both, and a mint that prints only the
// device key must leave the handset key empty rather than duplicating it.
func TestParseMintResult_HandsetKeyIsScrapedBesideTheDeviceKey(t *testing.T) {
	out := []byte("onboarded `user2x` (native, no Ansible) — NetBird + Ory\n" +
		"  setup key:    ABC123  (key `peer-user2x-zagros-2026-09-10`)\n" +
		"  handset key:  HAND456  (key `peer-user2x-zagros-2026-09-10-handset`)\n" +
		"  ory login:    person-3@example.org  (password `hunter2`)\n")
	r := parseMintResult(out)
	if r.SetupKey != "ABC123" {
		t.Errorf("SetupKey = %q", r.SetupKey)
	}
	if r.HandsetKey != "HAND456" {
		t.Errorf("HandsetKey = %q, want HAND456", r.HandsetKey)
	}
	only := parseMintResult([]byte("  setup key:    ABC123  (key `x`)\n"))
	if only.HandsetKey != "" {
		t.Errorf("HandsetKey = %q on a mint with no handset line, want empty", only.HandsetKey)
	}
}

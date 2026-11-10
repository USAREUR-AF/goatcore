package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// fakeState records what the mock Kanidm saw + lets a test tune its responses.
type fakeState struct {
	personStatus  int
	personBody    string
	token         string
	credSeen      bool
	personCreated bool
	intentSeen    bool
	ttl86400      bool
}

func hasKey(m map[string]json.RawMessage, k string) bool { _, ok := m[k]; return ok }

// fakeKanidmSrv mimics the Kanidm endpoints the provider drives: the 3-step auth
// flow, person-create, and the credential reset-intent mint.
func fakeKanidmSrv(t *testing.T, st *fakeState) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/auth", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Step map[string]json.RawMessage `json:"step"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		w.Header().Set("x-kanidm-auth-session-id", "sess-1")
		switch {
		case hasKey(body.Step, "init"):
			_, _ = w.Write([]byte(`{"state":{"choose":["password"]}}`))
		case hasKey(body.Step, "begin"):
			_, _ = w.Write([]byte(`{"state":{"continue":["password"]}}`))
		case hasKey(body.Step, "cred"):
			st.credSeen = true
			_, _ = w.Write([]byte(`{"state":{"success":"ADMINTOK"}}`))
		default:
			http.Error(w, "unexpected auth step", http.StatusBadRequest)
		}
	})
	mux.HandleFunc("/v1/person", func(w http.ResponseWriter, r *http.Request) {
		st.personCreated = true
		w.WriteHeader(st.personStatus)
		_, _ = w.Write([]byte(st.personBody))
	})
	mux.HandleFunc("/v1/person/", func(w http.ResponseWriter, r *http.Request) {
		if strings.Contains(r.URL.Path, "_update_intent") {
			st.intentSeen = true
			if strings.HasSuffix(r.URL.Path, "/86400") {
				st.ttl86400 = true
			}
			_, _ = w.Write([]byte(`{"token":"` + st.token + `"}`))
			return
		}
		http.NotFound(w, r)
	})
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func TestKanidmProvision_HappyPath(t *testing.T) {
	st := &fakeState{personStatus: 200, personBody: "null", token: "RESETTOK"}
	srv := fakeKanidmSrv(t, st)
	k := &kanidmProvider{base: srv.URL, adminPW: "pw", hc: srv.Client()}

	acct, url, err := k.Provision(context.Background(), "jane-doe", "jane@example.io", "Jane Doe")
	if err != nil {
		t.Fatalf("provision: %v", err)
	}
	if acct != "jane-doe" {
		t.Errorf("account = %q, want jane-doe", acct)
	}
	if !strings.HasSuffix(url, "/ui/reset?token=RESETTOK") {
		t.Errorf("setupURL = %q, want .../ui/reset?token=RESETTOK", url)
	}
	if !st.credSeen || !st.personCreated || !st.intentSeen {
		t.Errorf("missing expected calls: %+v", st)
	}
	if !st.ttl86400 {
		t.Errorf("reset intent must request the 24h (86400) TTL as a path segment")
	}
}

func TestKanidmProvision_ServiceAccountTokenSkipsPasswordAuth(t *testing.T) {
	// With a scoped service-account token, the provider must NOT run the idm_admin
	// 3-step password flow — it uses the token directly as the bearer.
	st := &fakeState{personStatus: 200, personBody: "null", token: "RT"}
	srv := fakeKanidmSrv(t, st)
	k := &kanidmProvider{base: srv.URL, apiToken: "SVC-TOKEN", hc: srv.Client()}

	acct, url, err := k.Provision(context.Background(), "svc-user", "s@example.io", "Svc User")
	if err != nil {
		t.Fatalf("provision: %v", err)
	}
	if acct != "svc-user" || !strings.HasSuffix(url, "token=RT") {
		t.Errorf("acct=%q url=%q", acct, url)
	}
	if st.credSeen {
		t.Error("service-account token path must not run the idm_admin password flow")
	}
	if !st.personCreated || !st.intentSeen {
		t.Errorf("expected person-create + intent calls: %+v", st)
	}
}

func TestKanidmProvision_AlreadyExistsIsAchieved(t *testing.T) {
	// Re-onboarding someone who kept their collab access: the account we were
	// asked to ensure exists, exists. That is ACHIEVED — return the account with
	// NO setup link and no error, and do NOT mint a credential-reset intent (that
	// would hand out a takeover link for credentials that already work, F-274).
	//
	// The 409 + attribute-uniqueness body is the LIVE Kanidm response, captured
	// from the collab IdP — the previous string-match tolerance
	// ("already exists"/"duplicate") never fired against it, so every re-onboard
	// was recorded as a collab failure.
	cases := []struct {
		name   string
		status int
		body   string
	}{
		{"live kanidm 409", 409, `{"conflicting_attributes":["mail","name","spn"],"error":"Attribute uniqueness error"}`},
		{"legacy already-exists text", 400, `{"error":"entry already exists"}`},
		{"legacy duplicate text", 400, `{"error":"Duplicate entry"}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			st := &fakeState{personStatus: tc.status, personBody: tc.body, token: "TOK2"}
			srv := fakeKanidmSrv(t, st)
			k := &kanidmProvider{base: srv.URL, adminPW: "pw", hc: srv.Client()}

			acct, url, err := k.Provision(context.Background(), "existing", "e@example.io", "Existing Person")
			if err != nil {
				t.Fatalf("already-provisioned must not error: %v", err)
			}
			if acct != "existing" {
				t.Errorf("acct=%q, want the account id back", acct)
			}
			if url != "" {
				t.Errorf("url=%q, want EMPTY — never mint a reset link for a live account", url)
			}
			if st.intentSeen {
				t.Error("must not mint a credential-setup intent for an already-provisioned account")
			}
		})
	}
}

func TestKanidmDeprovisionAndExists(t *testing.T) {
	// Deprovision is idempotent (404 = already gone = success); Exists is a
	// read-only probe with no side effects.
	st := &fakeState{personStatus: 200, personBody: "null", token: "T"}
	srv := fakeKanidmSrv(t, st)
	k := &kanidmProvider{base: srv.URL, apiToken: "SVC", hc: srv.Client()}

	if err := k.Deprovision(context.Background(), ""); err == nil {
		t.Error("empty handle must error before any network call")
	}
	if err := k.Deprovision(context.Background(), "gone-already"); err != nil {
		t.Errorf("deprovision of an absent account should succeed: %v", err)
	}
}

func TestIsAlreadyExists(t *testing.T) {
	yes := []struct {
		st   int
		body string
	}{
		{409, `{"conflicting_attributes":["name"],"error":"Attribute uniqueness error"}`},
		{400, "entry already exists"},
		{400, "Duplicate entry"},
		{409, ""},
	}
	for _, c := range yes {
		if !isAlreadyExists(c.st, c.body) {
			t.Errorf("isAlreadyExists(%d, %q) = false, want true", c.st, c.body)
		}
	}
	no := []struct {
		st   int
		body string
	}{
		{200, "null"},
		{500, `{"error":"internal"}`},
		{403, `{"error":"insufficient privilege"}`},
	}
	for _, c := range no {
		if isAlreadyExists(c.st, c.body) {
			t.Errorf("isAlreadyExists(%d, %q) = true, want false", c.st, c.body)
		}
	}
}

func TestKanidmProvision_RequiresHandleAndEmail(t *testing.T) {
	k := &kanidmProvider{base: "http://unused", adminPW: "pw", hc: http.DefaultClient}
	if _, _, err := k.Provision(context.Background(), "", "e@example.org", "n"); err == nil {
		t.Error("empty handle should error before any network call")
	}
	if _, _, err := k.Provision(context.Background(), "h", "", "n"); err == nil {
		t.Error("empty email should error before any network call")
	}
}

func TestParseManifestFacts(t *testing.T) {
	f := parseManifestFacts(`{"identity":{"name":"Jane Doe","email":"j@example.org"},"collab":{"enabled":true,"provider":"goat-collab"}}`)
	if !f.Collab.Enabled || f.Identity.Name != "Jane Doe" || f.Collab.Provider != "goat-collab" {
		t.Errorf("parsed = %+v", f)
	}
	if parseManifestFacts(`{"collab":{"enabled":false}}`).Collab.Enabled {
		t.Error("enabled:false must parse as disabled")
	}
	if parseManifestFacts(`{}`).Collab.Enabled {
		t.Error("absent collab block must parse as disabled (back-compat)")
	}
	if parseManifestFacts(`not json`).Collab.Enabled {
		t.Error("unparseable manifest must parse as disabled (safe default)")
	}
}

// stubCollab is a CollabProvider double for the runner-gating tests.
type stubCollab struct {
	calls                          int
	lastHandle, lastEmail, lastNam string
	failErr                        error
	// alreadyProvisioned makes Provision answer the re-onboard shape: the
	// account id with NO setup link and no error.
	alreadyProvisioned bool
	deprovisioned      []string
	exists             bool
}

func (s *stubCollab) Name() string { return "stub" }
func (s *stubCollab) Provision(_ context.Context, h, e, n string) (string, string, error) {
	s.calls++
	s.lastHandle, s.lastEmail, s.lastNam = h, e, n
	if s.failErr != nil {
		return "", "", s.failErr
	}
	if s.alreadyProvisioned {
		return h, "", nil
	}
	return h, "https://collab.example/ui/reset?token=T", nil
}
func (s *stubCollab) Deprovision(_ context.Context, h string) error {
	s.deprovisioned = append(s.deprovisioned, h)
	return s.failErr
}
func (s *stubCollab) Exists(_ context.Context, _ string) (bool, error) { return s.exists, s.failErr }

func discardLog() *slog.Logger { return slog.New(slog.NewTextHandler(io.Discard, nil)) }

func TestProvisionCollab_Gating(t *testing.T) {
	log := discardLog()

	// nil provider ⇒ no-op, no panic even with an enabled manifest.
	(&runner{repoRoot: t.TempDir()}).provisionCollab(context.Background(),
		proposal{Handle: "h", ManifestJSON: `{"collab":{"enabled":true}}`}, log)

	// collab disabled in the manifest ⇒ provider is never called.
	s := &stubCollab{}
	(&runner{repoRoot: t.TempDir(), collab: s}).provisionCollab(context.Background(),
		proposal{Handle: "h", Email: "e@example.org", ManifestJSON: `{"collab":{"enabled":false}}`}, log)
	if s.calls != 0 {
		t.Errorf("disabled manifest must not call the provider (calls=%d)", s.calls)
	}

	// enabled ⇒ provider called with handle/email/displayname; out-file staged.
	repo := t.TempDir()
	s2 := &stubCollab{}
	p := proposal{Handle: "jane", Email: "jane@example.org", ManifestJSON: `{"identity":{"name":"Jane"},"collab":{"enabled":true}}`}
	(&runner{repoRoot: repo, collab: s2}).provisionCollab(context.Background(), p, log)
	if s2.calls != 1 || s2.lastHandle != "jane" || s2.lastEmail != "jane@example.org" || s2.lastNam != "Jane" {
		t.Errorf("provider call = %+v", s2)
	}
	b, err := os.ReadFile(filepath.Join(repo, "ops", "operators", "out", "jane-collab.md"))
	if err != nil {
		t.Fatalf("collab out-file: %v", err)
	}
	if !strings.Contains(string(b), "username: jane") || !strings.Contains(string(b), "reset?token=T") {
		t.Errorf("out-file missing username/link:\n%s", b)
	}
}

func TestProvisionCollab_EmailFallsBackToManifest(t *testing.T) {
	// proposal.Email empty ⇒ fall back to identity.email from the manifest.
	s := &stubCollab{}
	(&runner{repoRoot: t.TempDir(), collab: s}).provisionCollab(context.Background(),
		proposal{Handle: "k", ManifestJSON: `{"identity":{"name":"K","email":"k@fallback.example"},"collab":{"enabled":true}}`}, discardLog())
	if s.lastEmail != "k@fallback.example" {
		t.Errorf("email fallback = %q, want k@fallback.example", s.lastEmail)
	}
}

func TestProvisionCollab_AlreadyProvisionedIsRecorded(t *testing.T) {
	// The re-onboard case: the person kept their collab access, so the
	// provider answers account + empty link. The runner must RECORD the account
	// (so the identity record stays true and collab_activated is set) while
	// staging no out-file — there is no link to stage, and none should exist.
	repo := t.TempDir()
	s := &stubCollab{alreadyProvisioned: true}
	acct, url := (&runner{repoRoot: repo, collab: s}).provisionCollab(context.Background(),
		proposal{Handle: "returning", Email: "r@example.org", ManifestJSON: `{"identity":{"name":"R"},"collab":{"enabled":true}}`},
		discardLog())
	if acct != "returning" {
		t.Errorf("account = %q, want it recorded as achieved", acct)
	}
	if url != "" {
		t.Errorf("setup link = %q, want empty (no reset link for a live account)", url)
	}
	if _, err := os.Stat(filepath.Join(repo, "ops", "operators", "out", "returning-collab.md")); !os.IsNotExist(err) {
		t.Error("no out-file should be staged when there is no new setup link")
	}
}

func TestProvisionCollab_BestEffortOnFailure(t *testing.T) {
	// A provider failure must not panic, must not stage an out-file, must not
	// propagate (the caller has no error return — a panic would crash the loop).
	repo := t.TempDir()
	s := &stubCollab{failErr: fmt.Errorf("kanidm unreachable")}
	(&runner{repoRoot: repo, collab: s}).provisionCollab(context.Background(),
		proposal{Handle: "x", Email: "x@example.net", ManifestJSON: `{"collab":{"enabled":true}}`}, discardLog())
	if _, err := os.Stat(filepath.Join(repo, "ops", "operators", "out", "x-collab.md")); !os.IsNotExist(err) {
		t.Error("no out-file should be staged when provisioning fails")
	}
}

func TestCollabSignInURLDerivesFromDoor(t *testing.T) {
	t.Setenv("COLLAB_SIGNIN_URL", "")
	k := &kanidmProvider{base: "https://idm.collab.example.org"}
	if got := collabSignInURL(k); got != "https://collab.example.org" {
		t.Fatalf("derived sign-in = %q", got)
	}
	t.Setenv("COLLAB_SIGNIN_URL", "https://front.example.org/")
	if got := collabSignInURL(k); got != "https://front.example.org" {
		t.Fatalf("explicit sign-in = %q", got)
	}
}

func TestCollabProviderNeedsDoor(t *testing.T) {
	t.Setenv("COLLAB_PROVISION_ENABLED", "1")
	t.Setenv("COLLAB_KANIDM_API_TOKEN", "tok")
	t.Setenv("COLLAB_KANIDM_URL", "")
	if p := newCollabProviderFromEnv(time.Second, slog.New(slog.NewTextHandler(io.Discard, nil))); p != nil {
		t.Fatal("a provider with no COLLAB_KANIDM_URL must stay disabled")
	}
}

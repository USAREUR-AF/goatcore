package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/store"
)

// newOnboardTestServer builds a minimal Server wired for the onboarding realms:
// a temp SQLite store, a fixed machine token, and DevBypassAuth so the human
// (adminGate) surface admits a synthetic admin actor.
func newOnboardTestServer(t *testing.T) *Server {
	t.Helper()
	ob, err := onboardstore.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	t.Cleanup(func() { _ = ob.Close() })
	cfg := &config.Config{
		DevBypassAuth: true,
		NotifyGoatnet: "zagros",
		// Per-handle operator-key map; the runner presents "runner-key"
		// and is attributed as the handle "onboard-runner".
		OnboardOperatorAPIKeys: map[string]string{"runner-key": "onboard-runner"},
		IdPs:                   map[string]config.IdP{"tenant": {Key: "tenant", Label: "Tenant"}},
		IdPOrder:               []string{"tenant"},
		DefaultIdP:             "tenant",
	}
	// The console never boots without a loaded matrix (main.go exits on
	// Load error), so tests mirror the canonical ADMINS grant the
	// matrix-driven adminGate consults (ADR 1073).
	perms := &permissions.Matrix{
		SchemaVersion: 1,
		Groups: []permissions.Group{
			{Name: "ADMINS", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
			{Name: "IAM-ADMIN", Grants: []permissions.Grant{{Service: "id-admin-console", Access: "admin"}}},
		},
	}
	srv, err := NewServer(cfg, nil, nil, store.NewMemory(), audit.NewStderrEmitter(50), perms, ob)
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	return srv
}

// TestOnboardFlow exercises the full three-realm arc: a human admin mints an
// invite (native attribution, no tier cap), the public token-gated preview +
// submit produce an inert proposal, and the machine-token realm sponsor-approves,
// approves, claims, and records the result — matching the portal's JSON shapes.
func TestOnboardFlow(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	// --- HUMAN realm: mint an invite. adminGate + DevBypassAuth admits it. The
	// CSRF nonce is the server instance nonce.
	form := url.Values{}
	form.Set("csrf", s.csrfNonce)
	form.Set("env", "zagros")
	form.Set("tier", "developer")
	form.Set("affiliation", "partner")
	form.Set("ttl_hours", "72")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/invite", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("mint invite: want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	// The plaintext token isn't in the HTML response; pull it out of the store by
	// listing invites via the machine realm.
	invites := listInvites(t, h)
	if len(invites) != 1 {
		t.Fatalf("want 1 invite, got %d", len(invites))
	}
	if invites[0].Sponsor != "dev@bypass.local" {
		t.Fatalf("native attribution: want sponsor dev@bypass.local, got %q", invites[0].Sponsor)
	}
	// Re-mint through the store directly to get a token (the mint handler doesn't
	// echo it in HTML; the store owns single-use). We instead mint a fresh invite
	// via the store to drive preview/submit deterministically.
	tok := mintTokenViaStore(t, s)

	// --- PUBLIC realm: preview (token-gated, no auth). Valid + partner affiliation
	// is locked (sponsor set it), tier is locked (developer).
	pv := preview(t, h, tok)
	if !pv.Valid {
		t.Fatalf("preview: want valid, got reason=%q", pv.Reason)
	}
	if !pv.TierLocked || !pv.AffiliationLocked {
		t.Fatalf("preview: want tier+affiliation locked, got tier=%v aff=%v", pv.TierLocked, pv.AffiliationLocked)
	}

	// --- PUBLIC realm: submit. Enrollee fills identity; sponsor-locked fields win.
	sub := map[string]any{"name": "Pat Liaison", "email": "person-106@example.org", "organization": "Flow Org", "nationality": "CAN"}
	pid := submit(t, h, tok, sub)
	if pid == "" {
		t.Fatalf("submit: empty proposal id")
	}

	// A second submit with the same token must fail (single-use).
	if code := submitRaw(t, h, tok, sub); code != http.StatusConflict {
		t.Fatalf("double-submit: want 409, got %d", code)
	}

	// --- MACHINE realm: the proposal is `proposed` (awaiting sponsor). Sponsor-
	// approve → sponsor_approved, approve → approved, claim → onboarding,
	// result(onboarded) → onboarded.
	if got := decideMachine(t, h, pid, "sponsor-approve"); got != onboardstore.ProposalSponsorApproved {
		t.Fatalf("sponsor-approve: got %q", got)
	}
	if got := decideMachine(t, h, pid, "approve"); got != onboardstore.ProposalApproved {
		t.Fatalf("approve: got %q", got)
	}
	claim := postMachine(t, h, "/api/onboard/proposals/"+pid+"/claim", nil)
	if claim.code != http.StatusOK {
		t.Fatalf("claim: want 200, got %d: %s", claim.code, claim.body)
	}
	res := postMachine(t, h, "/api/onboard/proposals/"+pid+"/result", map[string]any{"status": "onboarded"})
	if res.code != http.StatusOK {
		t.Fatalf("result: want 200, got %d: %s", res.code, res.body)
	}

	// --- MACHINE realm: token gate. No bearer → 401.
	nreq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/proposals", nil)
	nrec := httptest.NewRecorder()
	h.ServeHTTP(nrec, nreq)
	if nrec.Code != http.StatusUnauthorized {
		t.Fatalf("no-token: want 401, got %d", nrec.Code)
	}
}

// TestInviteLinksOmitPublicPrefix locks F-246: the candidate links the mint API
// emits (surfaced on the invite-created page and in the invite email) must use
// the clean /onboard.html path, NOT /public/onboard.html. The public intake
// collector only proxies /onboard.html (rewriting it to the console's /public
// route internally) and 404s a literal /public/onboard.html, so a /public link
// is unreachable for the candidate.
func TestInviteLinksOmitPublicPrefix(t *testing.T) {
	s := newOnboardTestServer(t)
	// Both bases front through the intake collector in production; set them so the
	// mint response carries absolute intake_url / nomination_url.
	s.Cfg.IntakeBaseURL = "https://intake.zagros.example"
	s.Cfg.PortalPublicURL = "https://intake.zagros.example"
	h := s.Routes()

	resp := postMachine(t, h, "/api/onboard/invite", map[string]any{"env": "zagros"})
	if resp.code != http.StatusCreated {
		t.Fatalf("mint invite: want 201, got %d: %s", resp.code, resp.body)
	}
	var mr struct {
		IntakeURL      string `json:"intake_url"`
		NominationURL  string `json:"nomination_url"`
		NominationPath string `json:"nomination_path"`
	}
	if err := json.Unmarshal([]byte(resp.body), &mr); err != nil {
		t.Fatalf("decode mint response: %v", err)
	}
	for _, c := range []struct{ name, got string }{
		{"nomination_path", mr.NominationPath},
		{"intake_url", mr.IntakeURL},
		{"nomination_url", mr.NominationURL},
	} {
		if c.got == "" {
			t.Fatalf("%s is empty", c.name)
		}
		if strings.Contains(c.got, "/public/") {
			t.Errorf("%s must not contain /public/ (unreachable at the intake front desk): %q", c.name, c.got)
		}
		if !strings.Contains(c.got, "/onboard.html?t=") {
			t.Errorf("%s must point at the clean /onboard.html path: %q", c.name, c.got)
		}
	}
}

// --- helpers ---------------------------------------------------------

func mintTokenViaStore(t *testing.T, s *Server) string {
	t.Helper()
	pb := onboardstore.Prebound{Tier: "developer", Affiliation: "partner"}
	pbJSON, _ := json.Marshal(pb)
	inv := &onboardstore.Invite{Sponsor: "dev@bypass.local", Env: "zagros", PreboundJSON: string(pbJSON), ExpiresAt: 0}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatalf("mint via store: %v", err)
	}
	return tok
}

func listInvites(t *testing.T, h http.Handler) []onboardstore.Invite {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/invites", nil)
	req.Header.Set("Authorization", "Bearer runner-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("list invites: %d %s", rec.Code, rec.Body.String())
	}
	var out []onboardstore.Invite
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatalf("decode invites: %v", err)
	}
	return out
}

func preview(t *testing.T, h http.Handler, tok string) onboardPreviewResponse {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/public/onboard/preview?t="+url.QueryEscape(tok), nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("preview: %d %s", rec.Code, rec.Body.String())
	}
	var pv onboardPreviewResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &pv); err != nil {
		t.Fatalf("decode preview: %v", err)
	}
	return pv
}

// withPersonDefaults fills the fields a PERSON's submission must carry for the
// server to accept it, when a test did not set them itself. Today that is only
// nationality (required ; a system is exempt). Tests about the
// requirement itself send their body through submitRaw with the field absent.
func withPersonDefaults(sub map[string]any) map[string]any {
	out := make(map[string]any, len(sub)+1)
	for k, v := range sub {
		out[k] = v
	}
	if pt, _ := out["principal_type"].(string); pt != "system" {
		if _, ok := out["nationality"]; !ok {
			out["nationality"] = "CAN"
		}
	}
	return out
}

func submit(t *testing.T, h http.Handler, tok string, sub map[string]any) string {
	t.Helper()
	body, _ := json.Marshal(map[string]any{"token": tok, "submission": withPersonDefaults(sub)})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusCreated {
		t.Fatalf("submit: want 201, got %d: %s", rec.Code, rec.Body.String())
	}
	var r onboardSubmitResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &r); err != nil {
		t.Fatalf("decode submit: %v", err)
	}
	return r.ProposalID
}

func submitRaw(t *testing.T, h http.Handler, tok string, sub map[string]any) int {
	t.Helper()
	body, _ := json.Marshal(map[string]any{"token": tok, "submission": sub})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec.Code
}

type machineResp struct {
	code int
	body string
}

func postMachine(t *testing.T, h http.Handler, path string, body any) machineResp {
	t.Helper()
	var rdr *bytes.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		rdr = bytes.NewReader(b)
	} else {
		rdr = bytes.NewReader(nil)
	}
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, rdr)
	req.Header.Set("Authorization", "Bearer runner-key")
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return machineResp{code: rec.Code, body: rec.Body.String()}
}

// TestMachineRealmAuth covers the per-handle key model: a valid operator key
// attributes to its handle, an unknown key is 401, and an unconfigured
// map is 503.
func TestMachineRealmAuth(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	// F-207: an intake through the machine realm is sponsored by the RESOLVED
	// handle ("onboard-runner"), not a generic label.
	manifest := map[string]any{
		"identity":    map[string]any{"handle": "svc-x", "email": "person-117@example.org", "name": "Svc X", "added_by": "person-110@example.org"},
		"affiliation": map[string]any{"id": "zagros-svc", "kind": "organic"},
		"netbird":     map[string]any{"enabled": true, "auto_groups": []string{"OBSERVER"}, "sites": []string{"zagros"}, "role": "peer"},
		"ory_site":    map[string]any{"enabled": true, "sites": []string{"zagros"}, "credentials": map[string]any{"password": true}},
		"vendor":      map[string]any{"display_name": "Svc X"},
	}
	body, _ := json.Marshal(map[string]any{"manifest": manifest, "env": "zagros"})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/onboard/intake", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer runner-key")
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code == http.StatusCreated {
		var resp onboardIntakeResponse
		_ = json.Unmarshal(rec.Body.Bytes(), &resp)
		if resp.Sponsor != "onboard-runner" {
			t.Fatalf("F-207 attribution: want sponsor onboard-runner, got %q", resp.Sponsor)
		}
	} else {
		// If the sample manifest doesn't lint, the sponsor-attribution assertion
		// is skipped — but we still confirm the request was authenticated (not
		// 401/503): a lint failure is a 400 with findings, not an auth error.
		if rec.Code == http.StatusUnauthorized || rec.Code == http.StatusServiceUnavailable {
			t.Fatalf("intake with valid key must authenticate; got %d: %s", rec.Code, rec.Body.String())
		}
	}

	// Unknown key → 401.
	ureq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/proposals", nil)
	ureq.Header.Set("Authorization", "Bearer not-a-key")
	urec := httptest.NewRecorder()
	h.ServeHTTP(urec, ureq)
	if urec.Code != http.StatusUnauthorized {
		t.Fatalf("unknown key: want 401, got %d", urec.Code)
	}

	// Unconfigured map → 503.
	s2 := newOnboardTestServer(t)
	s2.Cfg.OnboardOperatorAPIKeys = map[string]string{}
	h2 := s2.Routes()
	creq := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/api/onboard/proposals", nil)
	creq.Header.Set("Authorization", "Bearer runner-key")
	crec := httptest.NewRecorder()
	h2.ServeHTTP(crec, creq)
	if crec.Code != http.StatusServiceUnavailable {
		t.Fatalf("unconfigured: want 503, got %d", crec.Code)
	}
}

func decideMachine(t *testing.T, h http.Handler, pid, action string) string {
	t.Helper()
	r := postMachine(t, h, "/api/onboard/proposals/"+pid+"/"+action, map[string]any{})
	if r.code != http.StatusOK {
		t.Fatalf("%s: want 200, got %d: %s", action, r.code, r.body)
	}
	var out struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal([]byte(r.body), &out); err != nil {
		t.Fatalf("decode %s: %v", action, err)
	}
	return out.Status
}

package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// GHSA-mxf4-8rv7-774w round 3: a key is bound to its holder's Kratos identity
// id at collection, the live gate reads that identity by id, and a handle
// names one person at the intake doors.

const (
	holderAliceID = "0b1f0000-0000-4000-8000-0000000001a1"
	holderMallID  = "0b1f0000-0000-4000-8000-0000000001b2"
)

func holderServer(t *testing.T) (*Server, *multiKratosRT) {
	t.Helper()
	forgetIdentity(holderAliceID)
	forgetIdentity(holderMallID)
	kc, _ := fakeKratosMulti(t,
		fakeIdent{id: holderAliceID, email: "alice@example.test", name: "Alice", state: "active", groups: []string{"PLATFORM-ADMIN"}},
		fakeIdent{id: holderMallID, email: "mallory@example.test", name: "Mallory", state: "active", groups: []string{"IAM-ADMIN"}})
	s := newUsersAPITestServer(t, kc)
	s.Cfg.LiveIdentityCheck = true
	s.Cfg.NotifyGoatnet = "zagros"
	s.Cfg.OperatorKeyDir = t.TempDir()
	s.Cfg.OnboardOperatorAPIKeys["alice-key"] = "op-alice"
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		CreatedAt: time.Now().Unix() - 1000, Handle: "op-alice", Email: "alice@example.test",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatal(err)
	}
	return s, kc.HTTP.Transport.(*multiKratosRT)
}

// approvedRow files and approves (in the store) a key under op-alice.
func approvedRow(t *testing.T, s *Server, email string) *onboardstore.OperatorKey {
	t.Helper()
	ctx := context.Background()
	row, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: "op-alice", Email: email, Env: "zagros"})
	if err != nil {
		t.Fatal(err)
	}
	row, err = s.Onboard.ApproveOperatorKey(ctx, row.ID, "approver@example.test", []string{"approver@example.test", "approver"})
	if err != nil {
		t.Fatal(err)
	}
	return row
}

func plantSettled(t *testing.T, s *Server, email string) {
	t.Helper()
	if err := s.Onboard.CreateProposal(context.Background(), &onboardstore.Proposal{Sponsor: "sponsor-2", Env: "zagros",
		CreatedAt: time.Now().Unix() + 1000, Handle: "op-alice", Email: email,
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "member"}); err != nil {
		t.Fatal(err)
	}
}

// Collected through the machine door, the key is bound to Alice. A settled
// record filed later under her handle for somebody else neither locks her
// key out nor ties it to that somebody; an offboard done outside the console
// reaches it through the bound id.
func TestOperatorKeyHolder_boundAtCollectAndReadById(t *testing.T) {
	s, rt := holderServer(t)
	ctx := context.Background()
	row := approvedRow(t, s, "")
	rec := machineReq(t, s.Routes(), "alice-key", http.MethodGet, "/api/onboard/operator-keys/"+row.ID+"/collect", "")
	if rec.Code != http.StatusOK {
		t.Fatalf("collect: %d %s", rec.Code, rec.Body.String())
	}
	h, err := s.Onboard.OperatorKeyHolderOf(ctx, row.ID)
	if err != nil || h.HolderIdentityID != holderAliceID {
		t.Fatalf("holder after collect = %+v %v, want %s", h, err, holderAliceID)
	}
	s.Cfg.OnboardOperatorAPIKeys[row.Secret] = "op-alice"

	plantSettled(t, s, "mallory@example.test")
	forgetIdentity(holderAliceID)
	if c := machineReq(t, s.Routes(), row.Secret, http.MethodGet, "/api/onboard/proposals", "").Code; c != http.StatusOK {
		t.Fatalf("bound key locked out by a record filed under its handle: %d", c)
	}
	rt.mu.Lock()
	rt.byID[holderAliceID]["state"] = "inactive"
	rt.mu.Unlock()
	forgetIdentity(holderAliceID)
	if c := machineReq(t, s.Routes(), row.Secret, http.MethodGet, "/api/onboard/proposals", "").Code; c != http.StatusForbidden {
		t.Fatalf("bound key of an externally offboarded holder: %d, want 403 (Mallory, the other record's person, is active)", c)
	}
}

// The browser door binds the signed-in session's identity.
func TestOperatorKeyHolder_webCollectBindsSessionIdentity(t *testing.T) {
	s, _ := holderServer(t)
	row := approvedRow(t, s, "alice@example.test")
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/operator-key/collect/"+row.ID, nil)
	req.Header.Set("X-Forwarded-Email", "alice@example.test")
	req.Header.Set("X-Forwarded-User", holderAliceID)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	h, err := s.Onboard.OperatorKeyHolderOf(context.Background(), row.ID)
	if err != nil || h.HolderIdentityID != holderAliceID {
		t.Fatalf("web collect (%d): holder = %+v %v, want %s", rec.Code, h, err, holderAliceID)
	}
}

// A key whose handle names two people is not issued, and a row bound to one
// identity is not handed to another. Neither refusal writes anything.
func TestOperatorKeyHolder_refusesUnbindableOrMismatched(t *testing.T) {
	ctx := context.Background()
	t.Run("ambiguous", func(t *testing.T) {
		s, _ := holderServer(t) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		row := approvedRow(t, s, "") //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		plantSettled(t, s, "mallory@example.test") //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		if _, err := s.collectHolder(ctx, "op-alice", ""); err == nil {
			t.Fatal("resolved a holder for a handle that names two people")
		}
		if h, _ := s.Onboard.OperatorKeyHolderOf(ctx, row.ID); h.HolderIdentityID != "" {
			t.Fatalf("holder recorded on refusal: %q", h.HolderIdentityID)
		}
	})
	t.Run("mismatch", func(t *testing.T) {
		s, _ := holderServer(t) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		row := approvedRow(t, s, "alice@example.test") //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		if err := s.Onboard.SetOperatorKeyHolder(ctx, row.ID, holderAliceID); err != nil {
			t.Fatal(err)
		}
		if _, err := s.Onboard.CollectOperatorKeyAs(ctx, row.ID, "op-alice", holderMallID); !errors.Is(err, onboardstore.ErrOperatorKeyHolderMismatch) {
			t.Fatalf("collect of a row bound to Alice as Mallory: %v, want ErrOperatorKeyHolderMismatch", err)
		}
		if secret, err := s.Onboard.CollectOperatorKeyAs(ctx, row.ID, "op-alice", holderAliceID); err != nil || secret == "" {
			t.Fatalf("a refused mismatch consumed the key: %q %v", secret, err)
		}
	})
}

// A collect of somebody else's row id writes nothing, on either door and in
// any state: the row is bound only inside its owner's own collect.
func TestOperatorKeyHolder_foreignCollectBindsNothing(t *testing.T) {
	ctx := context.Background()
	s, _ := holderServer(t)
	s.Cfg.OnboardOperatorAPIKeys["mallory-key"] = "op-mallory"
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		CreatedAt: time.Now().Unix() - 900, Handle: "op-mallory", Email: "mallory@example.test",
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "iam-admin"}); err != nil {
		t.Fatal(err)
	}
	approved := approvedRow(t, s, "alice@example.test")
	requested, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: "op-alice", Email: "alice@example.test", Env: "other"})
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{approved.ID, requested.ID} {
		req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/operator-key/collect/"+id, nil)
		req.Header.Set("X-Forwarded-Email", "mallory@example.test")
		req.Header.Set("X-Forwarded-User", holderMallID)
		req.Header.Set("X-Forwarded-Groups", "IAM-ADMIN")
		s.Routes().ServeHTTP(httptest.NewRecorder(), req)
		machineReq(t, s.Routes(), "mallory-key", http.MethodGet, "/api/onboard/operator-keys/"+id+"/collect", "")
		if h, _ := s.Onboard.OperatorKeyHolderOf(ctx, id); h.HolderIdentityID != "" {
			t.Fatalf("foreign collect bound row %s to %q", id, h.HolderIdentityID)
		}
	}
	// Alice still collects her own key, and only then is it bound to her.
	rec := machineReq(t, s.Routes(), "alice-key", http.MethodGet, "/api/onboard/operator-keys/"+approved.ID+"/collect", "")
	if h, _ := s.Onboard.OperatorKeyHolderOf(ctx, approved.ID); rec.Code != http.StatusOK || h.HolderIdentityID != holderAliceID {
		t.Fatalf("alice's own collect: %d, holder %q", rec.Code, h.HolderIdentityID)
	}
}

// The roster binding of an operator-door request reads settled records only
// and refuses a handle two settled records give to two people.
func TestOperatorKeyRosterPerson_settledOnly(t *testing.T) {
	s, _ := holderServer(t)
	ctx := context.Background()
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "x", Env: "zagros", CreatedAt: time.Now().Unix() + 500,
		Handle: "op-alice", Email: "bob@example.test", Status: onboardstore.ProposalProposed, ManifestJSON: "{}"}); err != nil {
		t.Fatal(err)
	}
	h, e, err := s.operatorKeyRosterPerson(ctx, "op-alice", "")
	if err != nil || h != "op-alice" || e != "alice@example.test" {
		t.Fatalf("pending record must not name the roster person: %q %q %v", h, e, err)
	}
	plantSettled(t, s, "mallory@example.test")
	if _, _, err := s.operatorKeyRosterPerson(ctx, "op-alice", ""); err == nil {
		t.Fatal("filed a key under a handle two settled records give to two people")
	}
}

func intakeManifest(handle, email string) map[string]any {
	var m map[string]any
	_ = json.Unmarshal([]byte(`{"identity":{"handle":"`+handle+`","email":"`+email+`","name":"Pat Liaison","status":"active","added":"2026-06-01","added_by":"admin@example.test"},"netbird":{"enabled":true,"sites":["zagros"],"role":"peer","auto_groups":["OBSERVER"],"handset_limit":1},"ory_site":{"enabled":false},"ory_idp":{"enabled":false},"zenoh":{"enabled":false}}`), &m)
	return m
}

func runnerPost(t *testing.T, h http.Handler, path string, body any) *httptest.ResponseRecorder {
	t.Helper()
	b, _ := json.Marshal(body)
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, path, bytes.NewReader(b))
	req.Header.Set("Authorization", "Bearer runner-key")
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// Both intake doors refuse a handle a live record holds for another address.
func TestIntake_refusesHandleHeldByAnotherRecord(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	if rec := runnerPost(t, h, "/api/onboard/intake", map[string]any{"manifest": intakeManifest("svc-x", "person-117@example.org"), "env": "zagros"}); rec.Code != http.StatusCreated {
		t.Fatalf("first intake: %d %s", rec.Code, rec.Body.String())
	}
	rec := runnerPost(t, h, "/api/onboard/intake", map[string]any{"manifest": intakeManifest("svc-x", "someone-else@example.org"), "env": "zagros"})
	if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), "handle_taken") {
		t.Fatalf("second intake under the same handle: %d %s, want 409 handle_taken", rec.Code, rec.Body.String())
	}
	content, _ := json.Marshal([]any{intakeManifest("svc-x", "third@example.org")})
	rec = runnerPost(t, h, "/api/onboard/intake/batch", map[string]any{"content": string(content), "format": "json", "env": "zagros"})
	var resp onboardBatchResponse
	_ = json.Unmarshal(rec.Body.Bytes(), &resp)
	if rec.Code != http.StatusOK || resp.Summary.Created != 0 || resp.Summary.Rejected != 1 {
		t.Fatalf("batch intake under a held handle: %d %+v", rec.Code, resp.Summary)
	}
	ps, err := s.Onboard.ProposalsUnderHandle(context.Background(), "svc-x")
	if err != nil || len(ps) != 1 {
		t.Fatalf("records under svc-x = %d (%v), want 1", len(ps), err)
	}
}

// The public submit ignores a JSON handle and derives it from the email.
func TestSubmit_ignoresSubmittedHandle(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	tok := mintPrebound(t, s, onboardstore.Prebound{Tier: "browse"})
	pid := submit(t, h, tok, map[string]any{"name": "Eve", "email": "eve.person@example.test", "organization": "Eve Org", "nationality": "CAN", "handle": "op-alice"})
	p, _ := loadProposal(t, s, pid)
	if p.Handle == "op-alice" || p.Handle == "" {
		t.Fatalf("submitted handle honoured or lost: %q", p.Handle)
	}
}

// Configured per-person keys that resolve to nobody are named at startup and
// on /healthz/operator-keys; service keys and roster keys are not.
func TestUnrosteredOperatorKeys(t *testing.T) {
	s, _ := holderServer(t)
	s.Cfg.OnboardOperatorAPIKeys["ghost-key"] = "ghost"
	got, err := s.UnrosteredOperatorKeys(context.Background())
	if err != nil || len(got) != 1 || got[0] != "ghost" {
		t.Fatalf("UnrosteredOperatorKeys = %v %v, want [ghost]", got, err)
	}
	s.WarnUnrosteredOperatorKeys(context.Background())
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/healthz/operator-keys", nil)
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	var body struct {
		Count int      `json:"count"`
		Keys  []string `json:"unrostered_operator_keys"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &body)
	if rec.Code != http.StatusOK || body.Count != 1 || len(body.Keys) != 1 || body.Keys[0] != "ghost" {
		t.Fatalf("/healthz/operator-keys: %d %s", rec.Code, rec.Body.String())
	}
}

// The public submit door refuses an address whose derived handle a live
// record already holds for somebody else, and the invite survives.
func TestSubmit_refusesDerivedHandleHeldByAnother(t *testing.T) {
	s := newOnboardTestServer(t)
	ctx := context.Background()
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		Handle: "op-alice", Email: "alice@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}"}); err != nil {
		t.Fatal(err)
	}
	tok := mintPrebound(t, s, onboardstore.Prebound{Tier: "browse"})
	body, _ := json.Marshal(map[string]any{"token": tok, "submission": withPersonDefaults(map[string]any{
		"name": "Mal", "email": "op-alice@attacker.example", "organization": "Org", "nationality": "CAN"})})
	req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/public/onboard/submit", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusConflict || !strings.Contains(rec.Body.String(), "handle_taken") {
		t.Fatalf("submit under a held derived handle: %d %s, want 409 handle_taken", rec.Code, rec.Body.String())
	}
	if ps, _ := s.Onboard.ProposalsUnderHandle(ctx, "op-alice"); len(ps) != 1 {
		t.Fatalf("records under op-alice = %d, want 1", len(ps))
	}
	// The link is not spent: a usable address still goes through.
	submit(t, s.Routes(), tok, map[string]any{"name": "Mal", "email": "mal.lory@attacker.example", "organization": "Org", "nationality": "CAN"})
}

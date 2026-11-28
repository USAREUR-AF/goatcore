package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// siteRunnerHandle is a per-site runner key, a service key under
// isServiceKeyHandle (onboard-runner-<site>). Admin-bearing machine changes in
// these tests use it: 7788/vr6w refuse a per-person key on them.
const siteRunnerHandle = "onboard-runner-zagros"

// An offboard takes the person's operator keys with it, on every offboard
// door: deactivation from the console and from the machine realm, the decide
// flow's "offboard", and the queued cut. Somebody else's key is untouched.

type opKeyFixture struct {
	s       *Server
	mine    *onboardstore.OperatorKey
	pending *onboardstore.OperatorKey
	theirs  *onboardstore.OperatorKey
	dir     string
	read    func() ([]string, string)
}

func newOpKeyFixture(t *testing.T, status string) *opKeyFixture {
	t.Helper()
	ctx := context.Background()
	// Alice is the person being offboarded; Bob is the admin acting on the
	// machine realm with his own roster-bound key (bob-key → op-bob). Admin-
	// bearing machine changes go through the runner service key, which the
	// admin rules of 7788/vr6w allow and a per-person key is refused.
	kc, readID := fakeKratosMulti(t,
		fakeIdent{id: "id-alice", email: "alice@example.test", name: "Alice", state: "active", groups: []string{"IAM-ADMIN"}},
		fakeIdent{id: "id-bob", email: "bob@example.test", name: "Bob", state: "active", groups: []string{"PLATFORM-ADMIN"}})
	read := func() ([]string, string) { return readID("id-alice") }
	s := newUsersAPITestServer(t, kc)
	s.Cfg.NotifyGoatnet = "zagros"
	s.Cfg.OperatorKeyDir = t.TempDir()
	s.Cfg.OnboardOperatorAPIKeys["bob-key"] = "op-bob"
	s.Cfg.OnboardOperatorAPIKeys["site-runner-key"] = siteRunnerHandle
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
		Handle: "op-alice", Email: "alice@example.test", Status: status,
		ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
		t.Fatal(err)
	}
	seedRosterPerson(t, s, "op-bob", "bob@example.test")
	mk := func(handle, email string, approve bool) *onboardstore.OperatorKey {
		row, err := s.Onboard.RequestOperatorKey(ctx, &onboardstore.OperatorKey{Handle: handle, Email: email, Env: "zagros"})
		if err != nil {
			t.Fatal(err)
		}
		if !approve {
			return row
		}
		row, err = s.Onboard.ApproveOperatorKey(ctx, row.ID, "approver@example.test", []string{"approver@example.test", "approver"})
		if err != nil {
			t.Fatal(err)
		}
		if err := s.operatorKeyDir().Write(row.Handle, row.Secret); err != nil {
			t.Fatal(err)
		}
		return row
	}
	f := &opKeyFixture{s: s, dir: s.Cfg.OperatorKeyDir, read: read}
	f.mine = mk("op-alice", "", true)                             // handle-only row, joined through the onboarding record
	f.pending = mk("op-alice-2", "Alice+ops@example.test", false) // second spelling of her address
	f.theirs = mk("op-bob", "bob@example.test", true)
	return f
}

func (f *opKeyFixture) assertCut(t *testing.T) {
	t.Helper()
	ctx := context.Background()
	for _, k := range []*onboardstore.OperatorKey{f.mine, f.pending} {
		row, err := f.s.Onboard.OperatorKey(ctx, k.ID)
		if err != nil || row.Status != onboardstore.OperatorKeyRevoked {
			t.Errorf("key %s (%s) after offboard = %+v, %v; want revoked", k.ID, k.Handle, row, err)
		}
	}
	if secretOnDisk(t, f.dir, "op-alice", f.mine.Secret) {
		t.Errorf("op-alice's issued key is still on disk after offboard")
	}
	row, err := f.s.Onboard.OperatorKey(ctx, f.theirs.ID)
	if err != nil || row.Status != onboardstore.OperatorKeyActive {
		t.Errorf("someone else's key after offboard = %+v, %v; want still active", row, err)
	}
	if !secretOnDisk(t, f.dir, "op-bob", f.theirs.Secret) {
		t.Errorf("op-bob's key was removed by someone else's offboard")
	}
	after := auditAfter(t, f.s, "operator_key_revoked_at_offboard")
	if after["status"] != onboardstore.OperatorKeyRevoked {
		t.Errorf("revocation audit after = %v", after)
	}
}

func TestOffboard_webDeactivationRevokesOperatorKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	rec := postState(t, f.s, "id-alice", "other-admin@example.test", "inactive")
	if _, st := f.read(); st != "inactive" {
		t.Fatalf("deactivation did not land: status %d state %q body=%s", rec.Code, st, rec.Body.String())
	}
	f.assertCut(t)
}

func TestOffboard_apiDeactivationRevokesOperatorKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	body, _ := json.Marshal(map[string]string{"email": "alice@example.test", "state": "inactive"})
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/api/users/state", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer site-runner-key")
	rec := httptest.NewRecorder()
	f.s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("api deactivation: %d %s", rec.Code, rec.Body.String())
	}
	f.assertCut(t)
}

func TestOffboard_decideOffboardRevokesOperatorKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	ps, err := f.s.Onboard.ListProposalsFiltered(context.Background(), "", "alice@example.test")
	if err != nil || len(ps) != 1 {
		t.Fatalf("proposal lookup: %v %d", err, len(ps))
	}
	r := ceilingReq(adminActor("other-admin@example.test", ""))
	if _, err := f.s.decideProposal(r, ps[0].ID, "offboard", "other-admin@example.test", "cut by hand", ""); err != nil {
		t.Fatalf("decide offboard: %v", err)
	}
	f.assertCut(t)
}

func TestOffboard_queuedCutRevokesOperatorKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	f.s.Cfg.StepUpEnforced = false
	ps, err := f.s.Onboard.ListProposalsFiltered(context.Background(), "", "alice@example.test")
	if err != nil || len(ps) != 1 {
		t.Fatalf("proposal lookup: %v %d", err, len(ps))
	}
	r := ceilingReq(adminActor("other-admin@example.test", ""))
	r.Form = url.Values{"reason_class": {"left-service"}}
	rec := httptest.NewRecorder()
	f.s.offboardCutRevokingKeys(rec, r, ps[0].ID, adminActor("other-admin@example.test", ""))
	if rec.Code != http.StatusSeeOther {
		t.Fatalf("queued cut: %d %s", rec.Code, rec.Body.String())
	}
	f.assertCut(t)
}

// A cut that only issued a step-up challenge has not happened yet, so the keys
// stay until it does.
func TestOffboard_cutChallengedByStepUpLeavesKeys(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	f.s.Permissions.StepUpRequired = append(f.s.Permissions.StepUpRequired, "offboard_identity")
	ps, _ := f.s.Onboard.ListProposalsFiltered(context.Background(), "", "alice@example.test")
	r := ceilingReq(adminActor("other-admin@example.test", testActorID))
	r = r.WithContext(context.WithValue(r.Context(), ctxKeyIdP, f.s.Cfg.IdPs["tenant"]))
	r.Form = url.Values{"reason_class": {"left-service"}}
	rec := httptest.NewRecorder()
	f.s.offboardCutRevokingKeys(rec, r, ps[0].ID, adminActor("other-admin@example.test", testActorID))
	if rec.Code != http.StatusSeeOther || rec.Header().Get("Location")[:len("/step-up?nonce=")] != "/step-up?nonce=" {
		t.Fatalf("challenged cut: %d %q", rec.Code, rec.Header().Get("Location"))
	}
	row, err := f.s.Onboard.OperatorKey(context.Background(), f.mine.ID)
	if err != nil || row.Status != onboardstore.OperatorKeyActive {
		t.Fatalf("key revoked by a cut that never ran: %+v %v", row, err)
	}
}

// Self-reactivation through the machine realm is refused, including the real
// case where the holder's onboarding record is already offboarded (the active
// lookups skip it). Another operator still reactivates.
func TestApiPatchUserState_refusesSelfReactivation(t *testing.T) {
	ctx := context.Background()
	for _, status := range []string{onboardstore.ProposalOnboarded, onboardstore.ProposalOffboarded} {
		t.Run(status, func(t *testing.T) {
			// IAM-ADMIN, not PLATFORM-ADMIN: under the ADR 1071 amendment no
			// operator key reactivates a platform-tier login, so the service
			// key's reactivation below is only allowed below that tier.
			kc, read := fakeKratosMeta(t, "id-alice", "alice@example.test", "Alice", []string{"IAM-ADMIN"}, "inactive")
			s := newUsersAPITestServer(t, kc) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
			s.Cfg.NotifyGoatnet = "zagros"
			s.Cfg.OnboardOperatorAPIKeys["alice-key"] = "op-alice"
			s.Cfg.OnboardOperatorAPIKeys["bob-key"] = "op-bob"
			s.Cfg.OnboardOperatorAPIKeys["site-runner-key"] = siteRunnerHandle
			seedRosterPerson(t, s, "op-bob", "bob@example.test") //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
			if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros",
				Handle: "op-alice", Email: "alice@example.test", Status: status,
				ManifestJSON: "{}", Tier: "platform-admin"}); err != nil {
				t.Fatal(err)
			}
			call := func(key string) *httptest.ResponseRecorder {
				body, _ := json.Marshal(map[string]string{"email": "alice@example.test", "state": "active"})
				req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/api/users/state", bytes.NewReader(body))
				req.Header.Set("Authorization", "Bearer "+key)
				rec := httptest.NewRecorder()
				s.Routes().ServeHTTP(rec, req)
				return rec
			}
			if rec := call("alice-key"); rec.Code != http.StatusForbidden {
				t.Fatalf("self reactivation by operator key: %d %s", rec.Code, rec.Body.String())
			}
			if _, st := read(); st != "inactive" {
				t.Fatalf("state after refused self reactivation = %q", st)
			}
			// The other actor is a service key: a per-person key may not act
			// on an admin-bearing identity at all once 7788/vr6w are stacked,
			// so only a service key's answer is the same alone and stacked.
			if rec := call("site-runner-key"); rec.Code != http.StatusOK {
				t.Fatalf("a service key reactivating: %d %s", rec.Code, rec.Body.String())
			}
			if _, st := read(); st != "active" {
				t.Fatalf("state after another operator's reactivation = %q", st)
			}
		})
	}
}

// secretOnDisk reports whether the key file for handle still grants secret.
// It asserts what holds whatever a revoke leaves behind (nothing, or a
// tombstone in the key's place): the issued secret is no longer there.
func secretOnDisk(t *testing.T, dir, handle, secret string) bool {
	t.Helper()
	if strings.TrimSpace(secret) == "" {
		return false
	}
	b, err := os.ReadFile(filepath.Join(dir, handle+".key"))
	if os.IsNotExist(err) {
		return false
	}
	if err != nil {
		t.Fatalf("read key file for %s: %v", handle, err)
	}
	return strings.Contains(string(b), secret)
}

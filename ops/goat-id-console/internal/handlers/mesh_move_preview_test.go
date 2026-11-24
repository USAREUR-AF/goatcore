package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func TestMeshMovePreview(t *testing.T) {
	ctx := context.Background()
	const email = "preview-target@example.test"
	const peer = "p-preview"
	const from = "ZAGROS-DEVELOPER"
	const to = "ZAGROS-OBSERVER"
	const token = "fixture-netbird-token"
	s := newAccessTestServer(t, email, "Preview Target", nil)
	s.Cfg.MembershipAuthority = "ungoverned"
	s.Cfg.DevBypassAuth = true
	s.Cfg.EnrollWatchToken = token
	if err := s.Onboard.CreateProposal(ctx, &onboardstore.Proposal{
		Sponsor: "sponsor-1", Env: "zagros", Handle: "preview-target", Email: email,
		Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer",
	}); err != nil {
		t.Fatal(err)
	}

	type netbirdFailure struct {
		path   string
		status int
	}
	var failNetBird atomic.Pointer[netbirdFailure]
	nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if fail := failNetBird.Load(); fail != nil && r.URL.Path == fail.path {
			http.Error(w, "private upstream detail "+token, fail.status)
			return
		}
		switch r.URL.Path {
		case "/api/peers":
			_, _ = w.Write([]byte(`[
			  {"id":"p-preview","name":"peer-preview-target-acme-zagros-20260604-laptop","groups":[{"name":"ZAGROS-DEVELOPER"},{"name":"ZAGROS-P2P"}]},
			  {"id":"p-done","name":"peer-preview-target-acme-zagros-20260604-phone","groups":[{"name":"ZAGROS-OBSERVER"}]},
			  {"id":"p-other","name":"peer-other-person-acme-zagros-20260604-laptop","groups":[{"name":"ZAGROS-DEVELOPER"}]}
			]`))
		case "/api/setup-keys":
			_, _ = w.Write([]byte(`[]`))
		case "/api/groups":
			_, _ = w.Write([]byte(`[{"name":"All"},{"name":"ZAGROS-DEVELOPER"},{"name":"ZAGROS-OBSERVER"},{"name":"ZAGROS-P2P"},{"name":"ZAGROS-INFRA"},{"name":"ZAGROS-QUARANTINED"}]`))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(nb.Close)
	s.Cfg.NotifyMgmtURL = nb.URL
	s.Cfg.MovePlannerReadURL = nb.URL

	const good = `printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-preview","name":"Fixture laptop","other_groups":["ZAGROS-P2P"]}],"already_at_destination":0}'`
	tests := []struct {
		name, role, csrf, peer, from, to, planner, want string
		status                                          int
		callsPlanner                                    bool
	}{
		{"plan", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to, good, `"other_groups":["ZAGROS-P2P"]`, 200, true},
		{"already there", "PLATFORM-ADMIN", s.csrfNonce, "p-done", from, to,
			`printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[],"already_at_destination":1}'`,
			`"already_at_destination":1`, 200, true},
		{"IAM Admin refused", "IAM-ADMIN", s.csrfNonce, peer, from, to, "", "permission", 403, false},
		{"missing CSRF", "PLATFORM-ADMIN", "", peer, from, to, "", "CSRF token mismatch", 400, false},
		{"missing device", "PLATFORM-ADMIN", s.csrfNonce, "", from, to, "", "Select a device", 400, false},
		{"other user device", "PLATFORM-ADMIN", s.csrfNonce, "p-other", from, to, "", "unavailable for this user", 404, false},
		{"same groups", "PLATFORM-ADMIN", s.csrfNonce, peer, from, from, "", "two different", 400, false},
		{"system group", "PLATFORM-ADMIN", s.csrfNonce, peer, from, "All", "", "two different", 400, false},
		{"unknown destination", "PLATFORM-ADMIN", s.csrfNonce, peer, from, "missing", "", "no longer available", 409, false},
		{"stale source membership", "PLATFORM-ADMIN", s.csrfNonce, "p-done", from, "ZAGROS-INFRA", "", "no longer in the source group", 409, false},
		{"planner refusal", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to, `printf '%s' 'error: COVERAGE GATE: destination is uncovered (/private/config fixture-netbird-token)' >&2; exit 1`, "No enabled network policy covers this destination group. Choose another group or ask a network administrator to review the policy.", 422, true},
		{"unrecognized planner error", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' 'unexpected error from https://netbird.example.test/private/config (fixture-netbird-token)' >&2; exit 1`,
			"Move preview could not be completed. Try again. If the problem continues, contact an administrator with the request ID. Request ID:", 502, true},
		{"bad output", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to, `printf '%s' 'invalid json'`, "The console could not verify the move plan.", 502, true},
		{"wrong peer", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-other"}]}'`,
			"The console could not verify the move plan.", 502, true},
		{"multiple matches", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-preview"},{"peer_id":"p-other"}],"already_at_destination":0}'`,
			"The console could not verify the move plan.", 502, true},
		{"wrong group", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-P2P","movers":[{"peer_id":"p-preview"}]}'`,
			"The console could not verify the move plan.", 502, true},
		{"planner credentials rejected", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' 'error: NetBird mgmt API returned HTTP 401 (fixture-netbird-token)' >&2; exit 1`,
			"check the console’s NetBird credentials. Reference:", 503, true},
		{"planner service unavailable", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' 'error: NetBird mgmt API returned HTTP 503 (fixture-netbird-token)' >&2; exit 1`,
			"The network service could not be reached or returned an error. Try again.", 502, true},
		{"stale after validation", "PLATFORM-ADMIN", s.csrfNonce, peer, from, to,
			`printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[],"already_at_destination":0}'`,
			"group membership changed", 409, true},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			bin := filepath.Join(dir, "goat")
			argsPath := filepath.Join(dir, "args")
			mgmtPath := filepath.Join(dir, "mgmt")
			script := "#!/bin/sh\n" +
				"printf '%s\\n' \"$@\" > args\n" +
				"printf '%s' \"$GOAT_NB_MGMT_URL\" > mgmt\n" +
				"[ \"$GOAT_NB_API_TOKEN\" = fixture-netbird-token ] || exit 2\n" + tc.planner + "\n"
			if err := os.WriteFile(bin, []byte(script), 0700); err != nil {
				t.Fatal(err)
			}
			s.Cfg.MovePlannerBin, s.Cfg.MovePlannerProfile = bin, dir
			form := url.Values{"csrf": {tc.csrf}, "peer": {tc.peer}, "from": {tc.from}, "to": {tc.to}}
			req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/users/id-1/mesh-move/preview?idp=tenant", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			req.Header.Set("X-Forwarded-Email", "admin@example.test")
			req.Header.Set("X-Forwarded-Groups", tc.role)
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			if rec.Code != tc.status || !strings.Contains(rec.Body.String(), tc.want) {
				t.Fatalf("status = %d, want %d with %q; body: %s", rec.Code, tc.status, tc.want, rec.Body)
			}
			for _, private := range []string{token, "/private/config", "netbird.example.test", "COVERAGE GATE:"} {
				if strings.Contains(rec.Body.String(), private) {
					t.Fatal("response included raw planner details")
				}
			}
			args, err := os.ReadFile(argsPath)
			if !tc.callsPlanner {
				if !os.IsNotExist(err) {
					t.Fatalf("unexpected planner invocation: %q, error %v", args, err)
				}
				return
			}
			if got, err := os.ReadFile(mgmtPath); err != nil || string(got) != nb.URL {
				t.Fatalf("planner did not get the configured NetBird URL through its environment: %v", err)
			}
			wantArgs := []string{"--profile-dir=" + dir, "--context=zagros", "admin", "members", "move",
				"--env=zagros", "--planes=mesh", "--json",
				"--peer-id=" + tc.peer, "--from=" + tc.from, "--to=" + tc.to}
			if err != nil || string(args) != strings.Join(wantArgs, "\n")+"\n" {
				t.Fatalf("planner argv = %q, want %q (error %v)", args, wantArgs, err)
			}
		})
	}
	for _, tc := range []struct {
		name, path, message string
		upstream, status    int
	}{
		{"peer credentials rejected", "/api/peers", "Ask a network administrator to check the console’s NetBird credentials. Reference:", 401, 503},
		{"group credentials rejected", "/api/groups", "Ask a network administrator to check the console’s NetBird credentials. Reference:", 403, 503},
		{"peer service unavailable", "/api/peers", "The network service could not be reached or returned an error. Try again.", 503, 502},
		{"group service unavailable", "/api/groups", "The network service could not be reached or returned an error. Try again.", 502, 502},
	} {
		t.Run(tc.name, func(t *testing.T) {
			form := url.Values{"csrf": {s.csrfNonce}, "peer": {peer}, "from": {from}, "to": {to}}
			req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/users/id-1/mesh-move/preview?idp=tenant", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			req.Header.Set("X-Forwarded-Email", "admin@example.test")
			req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
			req.Header.Set("X-Request-ID", "test-preview-reference")
			failNetBird.Store(&netbirdFailure{path: tc.path, status: tc.upstream})
			t.Cleanup(func() { failNetBird.Store(nil) })
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			body := rec.Body.String()
			if rec.Code != tc.status || !strings.Contains(body, tc.message) || !strings.Contains(body, "test-preview-reference") {
				t.Fatalf("status = %d, want %d with %q and reference; body: %s", rec.Code, tc.status, tc.message, body)
			}
			for _, private := range []string{token, "private upstream detail", nb.URL, "policy covers"} {
				if strings.Contains(body, private) {
					t.Fatalf("unexpected failure detail %q in %q", private, body)
				}
			}
		})
	}

	t.Run("review binding and changed plan", func(t *testing.T) {
		dir := t.TempDir()
		bin := filepath.Join(dir, "goat")
		if err := os.WriteFile(bin, []byte("#!/bin/sh\nexec /bin/cat plan.json\n"), 0700); err != nil {
			t.Fatal(err)
		}
		s.Cfg.MovePlannerBin, s.Cfg.MovePlannerProfile = bin, dir
		writePlan := func(group string) {
			t.Helper()
			body := `{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-preview","name":"Fixture laptop","other_groups":["` + group + `"]}],"already_at_destination":0}`
			if err := os.WriteFile(filepath.Join(dir, "plan.json"), []byte(body), 0600); err != nil {
				t.Fatal(err)
			}
		}
		post := func(action, role, actor, device, proof string, want int) *httptest.ResponseRecorder {
			t.Helper()
			form := url.Values{"csrf": {s.csrfNonce}, "peer": {device}, "from": {from}, "to": {to}, "confirmation": {proof}}
			req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/users/id-1/mesh-move/"+action+"?idp=tenant", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			req.Header.Set("X-Forwarded-Email", actor)
			req.Header.Set("X-Forwarded-Groups", role)
			res := httptest.NewRecorder()
			s.Routes().ServeHTTP(res, req)
			if res.Code != want {
				t.Fatalf("%s status = %d, want %d; body: %s", action, res.Code, want, res.Body)
			}
			return res
		}
		decode := func(rec *httptest.ResponseRecorder) meshMovePlan {
			t.Helper()
			var plan meshMovePlan
			if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
				t.Fatal(err)
			}
			return plan
		}

		const admin = "admin@example.test"
		const platform = "PLATFORM-ADMIN"
		writePlan("ZAGROS-P2P")
		initial := decode(post("preview", platform, admin, peer, "", 200))
		if initial.ConfirmationToken == "" {
			t.Fatal("missing confirmation binding from preview")
		}
		post("confirm", platform, admin, peer, "", 409)
		post("confirm", platform, admin, peer, initial.ConfirmationToken+"x", 409)
		post("confirm", platform, "another-admin@example.test", peer, initial.ConfirmationToken, 409)
		post("confirm", platform, admin, "p-done", initial.ConfirmationToken, 409)
		post("confirm", "IAM-ADMIN", admin, peer, initial.ConfirmationToken, 403)

		expired := s.issueMeshMoveConfirmation(meshMoveBinding{
			ActorEmail: admin, Realm: "tenant", Identity: "id-1", Site: "zagros", Peer: peer, From: from, To: to,
		}, initial, time.Now().Add(-meshMoveConfirmationTTL))
		post("confirm", platform, admin, peer, expired, 409)

		writePlan("ZAGROS-OTHER")
		changed := decode(post("confirm", platform, admin, peer, initial.ConfirmationToken, 409))
		if changed.ConfirmationState != "changed" || changed.ConfirmationToken == "" ||
			changed.Movers[0].OtherGroups[0] != "ZAGROS-OTHER" {
			t.Fatalf("changed response does not contain a reviewable updated plan: %+v", changed)
		}
		post("confirm", platform, admin, peer, initial.ConfirmationToken, 409)
		confirmed := decode(post("confirm", platform, admin, peer, changed.ConfirmationToken, 200))
		if confirmed.ConfirmationState != "confirmed" || confirmed.ConfirmationToken != "" {
			t.Fatalf("confirmation response = %+v", confirmed)
		}

		scope := filepath.Join(dir, "_scope.yaml")
		if err := os.WriteFile(scope, []byte("version: 1\nscope: zagros\nconverge: enforce\n"), 0600); err != nil {
			t.Fatal(err)
		}
		s.Cfg.MembershipAuthority, s.Cfg.MembershipScopePath = "scope", scope
		const actorID = "aaaaaaaa-1111-4111-8111-111111111111"
		postGoverned := func(action, proof string, want int) meshMovePlan {
			t.Helper()
			form := url.Values{"csrf": {s.csrfNonce}, "peer": {peer}, "from": {from}, "to": {to}, "confirmation": {proof}}
			req := httptest.NewRequestWithContext(ctx, http.MethodPost, "/users/id-1/mesh-move/"+action+"?idp=tenant", strings.NewReader(form.Encode()))
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			req.Header.Set("X-Forwarded-Email", admin)
			req.Header.Set("X-Forwarded-Groups", platform)
			req.Header.Set("X-Forwarded-User", actorID)
			res := httptest.NewRecorder()
			s.Routes().ServeHTTP(res, req)
			if res.Code != want {
				t.Fatalf("%s status = %d, want %d; body: %s", action, res.Code, want, res.Body)
			}
			if want == http.StatusBadGateway {
				return meshMovePlan{}
			}
			return decode(res)
		}
		withDeclaration := func(fragment, expiry string) {
			t.Helper()
			script := `#!/bin/sh
for arg in "$@"; do
  case "$arg" in --apply|--write-entry) exit 2 ;; esac
done
case "$*" in
  *--prepare-declaration=*)
    case "$*" in *--owner=kratos-aaaaaaaa-1111-4111-8111-111111111111*) ;; *) exit 2 ;; esac
    printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-preview","name":"Fixture laptop","other_groups":["ZAGROS-OTHER"]}],"declaration":{"fragment":"` + fragment + `","replaces_existing":true,"effective":{"add":["ZAGROS-OBSERVER"],"remove":["ZAGROS-DEVELOPER"],"expiry_before":null,"expiry_after":"` + expiry + `"},"yaml":"  - peer: Fixture laptop\n    peer_id: p-preview\n    grant: [ZAGROS-OBSERVER]\n    expires: ` + expiry + `\n"}}'
    ;;
  *) exec /bin/cat plan.json ;;
esac
`
			if err := os.WriteFile(bin, []byte(script), 0700); err != nil {
				t.Fatal(err)
			}
		}
		withDeclaration("orgs/fixture.yaml", "2099-01-01")
		governedPreview := postGoverned("preview", "", 200)
		if governedPreview.Declaration == nil || governedPreview.Declaration.Effective == nil {
			t.Fatal("preview did not return the effective warrant declaration")
		}
		withDeclaration("orgs/fixture.yaml", "2099-02-01")
		changedDeclaration := postGoverned("confirm", governedPreview.ConfirmationToken, 409)
		if changedDeclaration.ConfirmationState != "changed" || changedDeclaration.ConfirmationToken == "" ||
			changedDeclaration.Declaration == nil || *changedDeclaration.Declaration.Effective.ExpiryAfter != "2099-02-01" {
			t.Fatalf("changed declaration was not returned for a fresh review: %+v", changedDeclaration)
		}
		prepared := postGoverned("confirm", changedDeclaration.ConfirmationToken, 200)
		if prepared.ConfirmationState != "declaration_ready" || prepared.Declaration == nil ||
			prepared.Declaration.Fragment != "ops/warrant/membership/zagros/orgs/fixture.yaml" ||
			!prepared.Declaration.ReplacesExisting || !strings.Contains(prepared.Declaration.YAML, "grant:") {
			t.Fatalf("invalid declaration handoff: %+v", prepared)
		}
		userPage := func(role, id string) string {
			t.Helper()
			req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/users/id-1?idp=tenant", nil)
			req.Header.Set("X-Forwarded-Email", admin)
			req.Header.Set("X-Forwarded-Groups", role)
			req.Header.Set("X-Forwarded-User", id)
			rec := httptest.NewRecorder()
			s.Routes().ServeHTTP(rec, req)
			if rec.Code != http.StatusOK {
				t.Fatalf("reload status = %d", rec.Code)
			}
			return rec.Body.String()
		}
		savedID := meshMoveOperationID(changedDeclaration.ConfirmationToken)
		if body := userPage(platform, actorID); !strings.Contains(body, savedID) ||
			!strings.Contains(body, "ops/warrant/membership/zagros/orgs/fixture.yaml") {
			t.Fatal("reloaded page did not contain the saved declaration")
		}
		for _, body := range []string{userPage("IAM-ADMIN", actorID), userPage(platform, "bbbbbbbb-1111-4111-8111-111111111111")} {
			if strings.Contains(body, savedID) {
				t.Fatal("saved declaration was disclosed outside the originating administrator's session")
			}
		}

		auditCount := func(operationID, outcome string) int {
			t.Helper()
			count := 0
			for _, event := range s.Audit.Recent(50) {
				if event.Action != "mesh_move_declaration_review" || event.After["operation_id"] != operationID {
					continue
				}
				if event.Actor.IdentityID != actorID || event.Target.IdentityID != "id-1" ||
					event.After["peer_id"] != peer || event.Result != outcome {
					t.Fatalf("audit record has the wrong requester, target, peer, or outcome: %+v", event)
				}
				count++
			}
			return count
		}

		outagePreview := postGoverned("preview", "", http.StatusOK)
		failNetBird.Store(&netbirdFailure{path: "/api/peers", status: 503})
		outageReceipt := postGoverned("confirm", changedDeclaration.ConfirmationToken, http.StatusOK)
		if outageReceipt.ConfirmationState != "saved_review" || outageReceipt.Declaration != nil {
			t.Fatalf("saved handoff must replay as history during an outage: %+v", outageReceipt)
		}
		if body := userPage(platform, actorID); !strings.Contains(body, "Saved group move for device "+peer) ||
			!strings.Contains(body, savedID) || strings.Contains(body, "orgs/fixture.yaml") {
			t.Fatal("reload must show the saved receipt without copyable YAML during an outage")
		}
		postGoverned("confirm", outagePreview.ConfirmationToken, http.StatusBadGateway)
		postGoverned("confirm", outagePreview.ConfirmationToken, http.StatusBadGateway)
		outageID := meshMoveOperationID(outagePreview.ConfirmationToken)
		if body := userPage(platform, actorID); !strings.Contains(body, outageID) ||
			!strings.Contains(body, "The network service could not be reached") ||
			!strings.Contains(body, "Preview again once the device is available") {
			t.Fatal("reload did not show the saved outage failure and recovery step")
		}
		if count := auditCount(outageID, "error"); count != 1 {
			t.Fatalf("outage and replay emitted %d audit events; want one", count)
		}
		failNetBird.Store(nil)
		postGoverned("confirm", outagePreview.ConfirmationToken, http.StatusBadGateway)

		withDeclaration("../unrelated.yaml", "2099-02-01")
		replayed := postGoverned("confirm", changedDeclaration.ConfirmationToken, 200)
		if replayed.Declaration == nil || replayed.Declaration.Fragment != prepared.Declaration.Fragment {
			t.Fatal("duplicate confirmation did not replay the saved declaration")
		}
		if count := auditCount(savedID, "ok"); count != 1 {
			t.Fatalf("duplicate confirmation emitted %d audit records; want one", count)
		}
		withDeclaration("orgs/fixture.yaml", "2099-02-01")
		newReview := postGoverned("preview", "", 200)
		withDeclaration("../unrelated.yaml", "2099-02-01")
		postGoverned("confirm", newReview.ConfirmationToken, 502)
		postGoverned("confirm", newReview.ConfirmationToken, 502)
		failedID := meshMoveOperationID(newReview.ConfirmationToken)
		if count := auditCount(failedID, "error"); count != 1 {
			t.Fatalf("failed confirmation emitted %d audit records; want one", count)
		}
		if body := userPage(platform, actorID); !strings.Contains(body, failedID) ||
			!strings.Contains(body, "The console could not verify where to put the proposed declaration.") ||
			strings.Contains(body, savedID) {
			t.Fatal("reloaded page did not show the latest saved failure")
		}
		if err := os.WriteFile(bin, []byte("#!/bin/sh\nexec /bin/cat plan.json\n"), 0700); err != nil {
			t.Fatal(err)
		}

		s.Cfg.MembershipScopePath = ""
		unverified := postGoverned("confirm", governedPreview.ConfirmationToken, 409)
		if unverified.Authority != authorityUnknown || unverified.ConfirmationToken != "" {
			t.Fatalf("unverified authority offered confirmation: %+v", unverified)
		}
	})

}

func TestSavedMeshMoveSuccessCanBeReplayedWithoutYAML(t *testing.T) {
	payload, err := json.Marshal(meshMoveSavedResult{
		From: "ZAGROS-DEVELOPER", To: "ZAGROS-OBSERVER",
		Declaration: &meshMoveDeclaration{
			YAML:      "- peer: laptop\n  peer_id: peer-a\n",
			Effective: &meshMoveEffectiveReview{Add: []string{"ZAGROS-OBSERVER"}},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	rec := httptest.NewRecorder()
	writeSavedMeshMoveReview(rec, onboardstore.MeshMoveReview{
		ID: "move-fixture", PeerID: "peer-a", Status: http.StatusOK, Result: string(payload),
	}, meshMoveLifecycleRefusal)
	var result struct {
		meshMoveSavedResult
		State  string `json:"confirmation_state"`
		Notice string `json:"notice"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusOK || rec.Header().Get("X-Move-Operation-ID") != "move-fixture" ||
		result.Declaration != nil || result.State != "saved_review" || result.Notice != meshMoveLifecycleRefusal {
		t.Fatalf("unexpected historical replay: %d %s", rec.Code, rec.Body.String())
	}
}

func TestMeshMovePlannerRequiresReadEndpoint(t *testing.T) {
	for _, tc := range []struct {
		endpoint  string
		dev, want bool
	}{
		{"https://mgmt-read.site.example.org", false, true},
		{"https://mgmt.site.example.org", false, false},
		{"http://mgmt-read.site.example.org", false, false},
		{"https://mgmt-read.site.example.org?alternate=true", false, false},
		{"http://127.0.0.1:8081/fixture", false, false},
		{"http://127.0.0.1:8081/fixture", true, true},
		{"http://service.example.org", true, false},
	} {
		t.Run(tc.endpoint, func(t *testing.T) {
			server := &Server{Cfg: &config.Config{MovePlannerReadURL: tc.endpoint, DevBypassAuth: tc.dev}}
			if got := server.movePlannerReadURL() != ""; got != tc.want {
				t.Fatalf("read endpoint accepted = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestMeshMoveRejectsManagedGroupsBeforePlanner(t *testing.T) {
	server := &Server{}
	for _, tc := range []struct{ from, to, refusal string }{
		{"ZAGROS-DEVELOPER", "zagros-dev-quarantined", meshMoveQuarantineRefusal},
		{"ZAGROS-QUARANTINED", "ZAGROS-OBSERVER", meshMoveQuarantineRefusal},
		{"ZAGROS-DEV-PROBATION-DEVELOPER", "ZAGROS-DEV-DEVELOPER", meshMoveLifecycleRefusal},
		{"ZAGROS-DEVELOPER", "ZAGROS-P2P", meshMoveP2PRefusal},
		{"ZAGROS-DEVELOPER", "ZAGROS-DEV-PLATFORM-ADMIN", meshMovePrivilegedRefusal},
		{"ZAGROS-DEV-BREAKGLASS", "ZAGROS-DEVELOPER", meshMovePrivilegedRefusal},
		{"ZAGROS-DEVELOPER", "ZAGROS-DEV-EXTERNAL-INFRA", meshMovePrivilegedRefusal},
	} {
		for _, confirm := range []bool{false, true} {
			req := httptest.NewRequest(http.MethodPost, "/", nil)
			req.PostForm = url.Values{"peer": {"peer-a"}, "from": {tc.from}, "to": {tc.to}}
			req = req.WithContext(context.WithValue(req.Context(), ctxKeyActor, Actor{Groups: []string{platformAdminGroup}}))
			rec := httptest.NewRecorder()
			server.handleMeshMoveReview(rec, req, confirm)
			if rec.Code != http.StatusUnprocessableEntity || !strings.Contains(rec.Body.String(), tc.refusal) {
				t.Fatalf("%v confirm=%v: status %d body %q", tc, confirm, rec.Code, rec.Body.String())
			}
		}
	}
}

func TestMeshMoveConfirmationBindsDeclarationAndEffectiveChanges(t *testing.T) {
	plan := meshMovePlan{Declaration: &meshMoveDeclaration{
		Fragment:  "people.yaml",
		YAML:      "reviewed YAML",
		Effective: &meshMoveEffectiveReview{Add: []string{"ZAGROS-OBSERVER"}},
	}}
	reviewedHash, err := meshMovePlanHash(plan)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name   string
		change func(*meshMoveDeclaration)
	}{
		{"YAML", func(d *meshMoveDeclaration) { d.YAML = "updated YAML" }},
		{"effective group", func(d *meshMoveDeclaration) { d.Effective.Add = []string{"ZAGROS-P2P"} }},
		{"effective expiry", func(d *meshMoveDeclaration) {
			date := "2099-01-01"
			d.Effective.ExpiryAfter = &date
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			changed := *plan.Declaration
			effective := *changed.Effective
			changed.Effective = &effective
			tc.change(&changed)
			updated := plan
			updated.Declaration = &changed
			hash, err := meshMovePlanHash(updated)
			if err != nil || hash == reviewedHash {
				t.Fatalf("review hash did not change, error: %v", err)
			}
		})
	}
}

func TestMeshMoveDeclarationBindsPeer(t *testing.T) {
	for _, tc := range []struct {
		name            string
		yaml            string
		createsFragment bool
		want            bool
	}{
		{name: "entry", yaml: "- peer: laptop\n  peer_id: '123'\n", want: true},
		{name: "fragment", yaml: "version: 1\nentries:\n- peer: laptop\n  peer_id: '123'\n", createsFragment: true, want: true},
		{name: "older planner", yaml: "- peer: laptop\n"},
		{name: "other peer", yaml: "- peer: laptop\n  peer_id: another-id\n"},
		{name: "numeric ID", yaml: "- peer: laptop\n  peer_id: 123\n"},
		{name: "additional entry", yaml: "- peer: laptop\n  peer_id: '123'\n- peer: other\n"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			declaration := meshMoveDeclaration{YAML: tc.yaml, CreatesFragment: tc.createsFragment}
			if got := declaration.bindsPeer("123"); got != tc.want {
				t.Fatalf("bindsPeer = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestClassifyMeshMovePlannerError(t *testing.T) {
	for _, tc := range []struct {
		stderr string
		reason string
		status int
	}{
		{"error: COVERAGE GATE: destination not covered", "destination_policy", 422},
		{"error: REFUSED security-quarantine transition: private owner details", "quarantine", 422},
		{"error: no group named `unknown`", "stale_selection", 409},
		{"error: group/peer not found", "stale_selection", 409},
		{"error: --from and --to resolve to the same group", "same_group", 400},
		{"error: NetBird mgmt API unreachable: private host", "network_unavailable", 502},
		{"error: NetBird mgmt API returned HTTP 401", "network_credentials", 503},
		{"error: NetBird mgmt API returned HTTP 403", "network_credentials", 503},
		{"error: NetBird mgmt API returned HTTP 503", "network_unavailable", 502},
		{"error: NetBird mgmt API returned HTTP 429", "network_unavailable", 502},
		{"error: NetBird mgmt client not configured", "network_access", 503},
		{"error: not admin\n  help: NetBird mgmt API denied this request", "network_credentials", 503},
		{"error: no profile for context\n  code: E_PROFILE_MISSING", "network_access", 503},
		{"", "unclassified_planner_error", 502},
		{"error: an unexpected diagnostic including COVERAGE GATE: as data", "unclassified_planner_error", 502},
	} {
		t.Run(tc.reason+"/"+tc.stderr, func(t *testing.T) {
			got := classifyMeshMovePlannerError(tc.stderr)
			if got.reason != tc.reason || got.status != tc.status {
				t.Fatalf("classification = %s (%d), want %s (%d)", got.reason, got.status, tc.reason, tc.status)
			}
		})
	}
}

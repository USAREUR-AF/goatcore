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
			_, _ = w.Write([]byte(`[{"name":"All"},{"name":"ZAGROS-DEVELOPER"},{"name":"ZAGROS-OBSERVER"},{"name":"ZAGROS-P2P"},{"name":"ZAGROS-QUARANTINED"}]`))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(nb.Close)
	s.Cfg.NotifyMgmtURL = nb.URL

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
		{"stale source membership", "PLATFORM-ADMIN", s.csrfNonce, peer, to, "ZAGROS-QUARANTINED", "", "no longer in the source group", 409, false},
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
				"--handle=" + tc.peer, "--from=" + tc.from, "--to=" + tc.to}
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
		governed := decode(post("confirm", platform, admin, peer, changed.ConfirmationToken, 409))
		if governed.Authority != authorityEnforce || governed.ConfirmationState != "changed" || governed.ConfirmationToken == "" {
			t.Fatalf("authority change did not require review: %+v", governed)
		}
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
		withDeclaration := func(fragment string) {
			t.Helper()
			script := `#!/bin/sh
for arg in "$@"; do
  case "$arg" in --apply|--write-entry) exit 2 ;; esac
done
case "$*" in
  *--prepare-declaration=*)
    case "$*" in *--owner=kratos-aaaaaaaa-1111-4111-8111-111111111111*) ;; *) exit 2 ;; esac
    printf '%s' '{"from_group":"ZAGROS-DEVELOPER","to_group":"ZAGROS-OBSERVER","movers":[{"peer_id":"p-preview","name":"Fixture laptop","other_groups":["ZAGROS-OTHER"]}],"declaration":{"fragment":"` + fragment + `","replaces_existing":true,"yaml":"  - peer: fixture-laptop\n    grant: [ZAGROS-OBSERVER]\n"}}'
    ;;
  *) exec /bin/cat plan.json ;;
esac
`
			if err := os.WriteFile(bin, []byte(script), 0700); err != nil {
				t.Fatal(err)
			}
		}
		withDeclaration("orgs/fixture.yaml")
		governedPreview := postGoverned("preview", "", 200)
		if governedPreview.Declaration != nil {
			t.Fatal("preview returned a declaration before confirmation")
		}
		prepared := postGoverned("confirm", governedPreview.ConfirmationToken, 200)
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
		savedID := meshMoveOperationID(governedPreview.ConfirmationToken)
		if body := userPage(platform, actorID); !strings.Contains(body, savedID) ||
			!strings.Contains(body, "ops/warrant/membership/zagros/orgs/fixture.yaml") {
			t.Fatal("reloaded page did not contain the saved declaration")
		}
		for _, body := range []string{userPage("IAM-ADMIN", actorID), userPage(platform, "bbbbbbbb-1111-4111-8111-111111111111")} {
			if strings.Contains(body, savedID) {
				t.Fatal("saved declaration was disclosed outside the originating administrator's session")
			}
		}
		failNetBird.Store(&netbirdFailure{path: "/api/peers", status: 503})
		if body := userPage(platform, actorID); !strings.Contains(body, "Saved group move for device "+peer) ||
			!strings.Contains(body, savedID) {
			t.Fatal("saved declaration disappeared while the peer list was unavailable")
		}
		failNetBird.Store(nil)

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

		withDeclaration("../unrelated.yaml")
		replayed := postGoverned("confirm", governedPreview.ConfirmationToken, 200)
		if replayed.Declaration == nil || replayed.Declaration.Fragment != prepared.Declaration.Fragment {
			t.Fatal("duplicate confirmation did not replay the saved declaration")
		}
		if count := auditCount(savedID, "ok"); count != 1 {
			t.Fatalf("duplicate confirmation emitted %d audit records; want one", count)
		}
		newReview := postGoverned("preview", "", 200)
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
		unverified := decode(post("confirm", platform, admin, peer, governed.ConfirmationToken, 409))
		if unverified.Authority != authorityUnknown || unverified.ConfirmationToken != "" {
			t.Fatalf("unverified authority offered confirmation: %+v", unverified)
		}
	})

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

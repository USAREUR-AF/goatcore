package handlers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/repoaccess"
)

// getMachine issues a machine-realm GET with the runner operator key.
func getMachine(t *testing.T, h http.Handler, path string) machineResp {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
	req.Header.Set("Authorization", "Bearer runner-key")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return machineResp{code: rec.Code, body: rec.Body.String()}
}

// writeRepoPolicy writes a minimal policy file granting a developer one repo and
// points the server at it. Returns the proposal id created for a developer with a
// github login.
func seedRepoAccessProposal(t *testing.T, s *Server) string {
	t.Helper()
	dir := t.TempDir()
	policy := "" +
		"version: 1\n" +
		"default_owner: usareur-af\n" +
		"rules:\n" +
		"  - name: developers-moon-pod\n" +
		"    match:\n" +
		"      tier: [developer]\n" +
		"    grant:\n" +
		"      - {repo: zagros-moon-pod, permission: pull}\n"
	path := filepath.Join(dir, "repo-access.yaml")
	if err := os.WriteFile(path, []byte(policy), 0o600); err != nil {
		t.Fatalf("write policy: %v", err)
	}
	s.Cfg.RepoAccessConfigPath = path
	s.Cfg.GitHubOnboardToken = "test-token" // has_token = true

	manifest := map[string]any{
		"identity":    map[string]any{"handle": "dev-x", "email": "dev-x@example.org", "name": "Dev X", "github": "octocat"},
		"affiliation": map[string]any{"id": "zagros-dev", "kind": "partner"},
	}
	mj, _ := json.Marshal(manifest)
	p := &onboardstore.Proposal{
		Sponsor:      "onboard-runner",
		Env:          "zagros",
		Handle:       "dev-x",
		Email:        "person-35@example.org",
		Status:       onboardstore.ProposalSponsorApproved,
		ManifestJSON: string(mj),
		Tier:         "developer",
		Complete:     true,
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}
	return p.ID
}

func TestRepoAccessGetReflectsPolicy(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedRepoAccessProposal(t, s)

	res := getMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access")
	if res.code != http.StatusOK {
		t.Fatalf("GET repo-access: want 200, got %d: %s", res.code, res.body)
	}
	var v repoAccessView
	if err := json.Unmarshal([]byte(res.body), &v); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if !v.HasToken || v.GitHub != "octocat" {
		t.Fatalf("view: has_token/github wrong: %+v", v)
	}
	if len(v.Effective) != 1 || v.Effective[0].Repo != "zagros-moon-pod" || v.Effective[0].Source != repoaccess.SourcePolicy {
		t.Fatalf("effective should reflect the policy grant: %+v", v.Effective)
	}
}

func TestRepoAccessSetAddAndRemove(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedRepoAccessProposal(t, s)

	// Add an audible repo AND remove the policy repo in one desired-set POST.
	body := map[string]any{
		"add":    []map[string]any{{"repo": "usareur-af/extra-repo", "permission": "push"}},
		"remove": []map[string]any{{"repo": "zagros-moon-pod"}},
	}
	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access", body)
	if res.code != http.StatusOK {
		t.Fatalf("POST repo-access: want 200, got %d: %s", res.code, res.body)
	}
	var v repoAccessView
	if err := json.Unmarshal([]byte(res.body), &v); err != nil {
		t.Fatalf("decode: %v", err)
	}
	// The policy repo is removed; the audible add is present, tagged audible.
	if findEffective(v.Effective, "extra-repo") == nil {
		t.Fatalf("audible add missing: %+v", v.Effective)
	}
	if g := findEffective(v.Effective, "extra-repo"); g.Source != repoaccess.SourceAudible || g.Permission != "push" {
		t.Fatalf("audible add wrong: %+v", g)
	}
	if findEffective(v.Effective, "zagros-moon-pod") != nil {
		t.Fatalf("remove should have dropped the policy repo: %+v", v.Effective)
	}

	// A fresh GET reflects the stored override (persisted).
	res2 := getMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access")
	var v2 repoAccessView
	_ = json.Unmarshal([]byte(res2.body), &v2)
	if len(v2.Overrides.Add) != 1 || len(v2.Overrides.Remove) != 1 {
		t.Fatalf("stored overrides not reflected on GET: %+v", v2.Overrides)
	}
	if findEffective(v2.Effective, "extra-repo") == nil || findEffective(v2.Effective, "zagros-moon-pod") != nil {
		t.Fatalf("GET effective should match the stored override: %+v", v2.Effective)
	}
}

func TestRepoAccessSetRejectsBadPermission(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	id := seedRepoAccessProposal(t, s)
	body := map[string]any{"add": []map[string]any{{"repo": "usareur-af/x", "permission": "godmode"}}}
	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access", body)
	if res.code != http.StatusBadRequest {
		t.Fatalf("bad permission: want 400, got %d: %s", res.code, res.body)
	}
}

// A person already onboarded can be given another repo, and the grant fires now.
// Dropping a repo is refused: it would change the set and leave GitHub as it was.
func TestRepoAccessAfterOnboardedAddsAndRefusesRemoval(t *testing.T) {
	s := newOnboardTestServer(t)
	rec := withDeveloperRepoPolicy(t, s)
	h := s.Routes()
	id := seedRepoAccessProposal(t, s)
	if err := s.Onboard.SetProposalStatus(context.Background(), id, onboardstore.ProposalOnboarded); err != nil {
		t.Fatalf("set onboarded: %v", err)
	}
	body := map[string]any{"add": []map[string]any{{"repo": "usareur-af/x", "permission": "pull"}}}
	res := postMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access", body)
	if res.code != http.StatusOK {
		t.Fatalf("add after onboarded: want 200, got %d: %s", res.code, res.body)
	}
	if !strings.Contains(strings.Join(rec.calls(), " "), "/repos/usareur-af/x/collaborators/octocat") {
		t.Fatalf("add after onboarded did not fire a grant; calls: %v", rec.calls())
	}
	if !strings.Contains(res.body, `"grants"`) {
		t.Fatalf("response does not report grant outcomes: %s", res.body)
	}
	for name, b := range map[string]map[string]any{
		"new remove":  {"add": []map[string]any{{"repo": "usareur-af/x"}}, "remove": []map[string]any{{"repo": "zagros-moon-pod"}}},
		"dropped add": {"add": []map[string]any{}},
	} {
		res := postMachine(t, h, "/api/onboard/proposals/"+id+"/repo-access", b)
		if res.code != http.StatusConflict || !strings.Contains(res.body, "remove_on_github") {
			t.Errorf("%s: want 409 remove_on_github, got %d: %s", name, res.code, res.body)
		}
	}
}

// browserRepoAccessPost posts as a browser admin session, the way the panel does.
func browserRepoAccessPost(t *testing.T, s *Server, id, csrf, groups, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodPost, "/onboarding/"+id+"/repo-access", strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	req.Header.Set("X-Forwarded-Groups", groups)
	if csrf != "" {
		req.Header.Set("X-CSRF-Token", csrf)
	}
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec
}

// The review card saves through a browser session, which holds no bearer key.
func TestRepoAccessBrowserSave(t *testing.T) {
	s := newOnboardTestServer(t)
	id := seedRepoAccessProposal(t, s)
	add := func(perm string) string {
		return `{"add":[{"repo":"usareur-af/x","permission":"` + perm + `"}]}`
	}
	if rec := browserRepoAccessPost(t, s, id, "", "IAM-ADMIN", add("pull")); rec.Code != http.StatusBadRequest {
		t.Errorf("no CSRF: want 400, got %d: %s", rec.Code, rec.Body.String())
	}
	if rec := browserRepoAccessPost(t, s, id, s.csrfNonce, "IAM-ADMIN", add("admin")); rec.Code != http.StatusForbidden {
		t.Errorf("IAM admin granting GitHub admin: want 403, got %d: %s", rec.Code, rec.Body.String())
	}
	if rec := browserRepoAccessPost(t, s, id, s.csrfNonce, "IAM-ADMIN", add("pull")); rec.Code != http.StatusOK {
		t.Errorf("IAM admin adding pull: want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	if rec := browserRepoAccessPost(t, s, id, s.csrfNonce, "PLATFORM-ADMIN", add("admin")); rec.Code != http.StatusOK {
		t.Errorf("platform admin granting GitHub admin: want 200, got %d: %s", rec.Code, rec.Body.String())
	}
}

// TestRepoAccessSetIsOperatorOnly is the H1 regression: the mutating POST is
// operator-only. A sponsorship-authorized participant key — even on a proposal
// carrying its OWN sponsorship, which the GET scoping admits — must be refused,
// because the POST body accepts a free-text repo and `admin` permission and
// would otherwise let a participant grant themselves GitHub admin on any repo.
// An operator key is allowed through the guard.
func TestRepoAccessSetIsOperatorOnly(t *testing.T) {
	s := newOnboardTestServer(t)
	// A participant sponsor whose org owns the proposal below. The portal
	// delegation seam resolves this key read-only to its org handle + cap.
	portalURL, _ := fakeSponsorPortal(t, map[string]sponsorLookupResponse{
		"sponsora-key": {Org: "0ff404dd", Alias: "pat-example", SponsorAuthorized: true, SponsorMaxTier: "developer"},
	}, nil)
	s.Cfg.PortalInternalURL = portalURL
	h := s.Routes()

	// A proposal sponsored by the participant's own org (so the GET/allowsSponsor
	// scoping would admit them) with a github login so a grant could really fire.
	dir := t.TempDir()
	policy := "" +
		"version: 1\n" +
		"default_owner: usareur-af\n" +
		"rules: []\n"
	path := filepath.Join(dir, "repo-access.yaml")
	if err := os.WriteFile(path, []byte(policy), 0o600); err != nil {
		t.Fatalf("write policy: %v", err)
	}
	s.Cfg.RepoAccessConfigPath = path
	manifest := map[string]any{
		"identity":    map[string]any{"handle": "mine-x", "email": "person-58@example.org", "name": "Mine X", "github": "octomine"},
		"affiliation": map[string]any{"id": "zagros-dev", "kind": "partner"},
	}
	mj, _ := json.Marshal(manifest)
	p := &onboardstore.Proposal{
		ID: "p-own-repo", Sponsor: "0ff404dd", Env: "zagros", Handle: "mine-x",
		Email: "person-58@example.org", Status: onboardstore.ProposalSponsorApproved,
		ManifestJSON: string(mj), Tier: "developer", Complete: true,
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	// The escalation POST: grant admin on an arbitrary repo. The participant
	// owns this proposal (GET would 200), but the mutation must be refused 403.
	body := `{"add":[{"repo":"usareur-af/anything","permission":"admin"}]}`
	w := machineReq(t, h, "sponsora-key", "POST", "/api/onboard/proposals/p-own-repo/repo-access", body)
	if w.Code != http.StatusForbidden {
		t.Fatalf("participant repo-access POST = %d, want 403; %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "operator_only") {
		t.Fatalf("participant refusal should be operator_only: %s", w.Body.String())
	}

	// The participant may still READ the effective set for its own invitee.
	if w := machineReq(t, h, "sponsora-key", "GET", "/api/onboard/proposals/p-own-repo/repo-access", ""); w.Code != http.StatusOK {
		t.Fatalf("participant GET repo-access = %d, want 200; %s", w.Code, w.Body.String())
	}

	// The operator key passes the guard (200 — the grant is accepted).
	w = machineReq(t, h, "runner-key", "POST", "/api/onboard/proposals/p-own-repo/repo-access", body)
	if w.Code != http.StatusOK {
		t.Fatalf("operator repo-access POST = %d, want 200; %s", w.Code, w.Body.String())
	}
}

// findEffective returns the effective grant for a repo (case-insensitive), or nil.
func findEffective(gs []repoaccess.EffectiveGrant, repo string) *repoaccess.EffectiveGrant {
	for i := range gs {
		if gs[i].Repo == repo {
			return &gs[i]
		}
	}
	return nil
}

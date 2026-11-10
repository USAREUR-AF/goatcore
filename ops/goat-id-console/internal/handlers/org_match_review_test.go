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

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
)

func withOrgs(t *testing.T, s *Server, enabled bool) *orgstore.Store {
	t.Helper()
	orgs, err := orgstore.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open orgstore: %v", err)
	}
	t.Cleanup(func() { _ = orgs.Close() })
	s.Orgs = orgs
	s.Cfg.OrgMatchEnabled = enabled
	return orgs
}

// storeDraftWithAffiliation creates a minimal-burden draft (the
// candidate-intake/v-draft record in manifest_json, complete=0) and then
// adds an affiliation block so the org-match handlers can see the typed org
// while draft completion still reads the intake record.
func storeDraftWithAffiliation(t *testing.T, s *Server, pb onboardstore.Prebound, sub map[string]any, affID, nation string) string {
	t.Helper()
	id := storeDraft(t, s, pb, sub)
	ctx := context.Background()
	p, err := s.Onboard.GetProposal(ctx, id)
	if err != nil {
		t.Fatalf("GetProposal(%s): %v", id, err)
	}
	var rec map[string]any
	if err := json.Unmarshal([]byte(p.ManifestJSON), &rec); err != nil {
		t.Fatalf("unmarshal draft manifest: %v", err)
	}
	rec["affiliation"] = map[string]any{"id": affID, "nation": nation}
	updated, err := json.Marshal(rec)
	if err != nil {
		t.Fatalf("marshal draft manifest with affiliation: %v", err)
	}
	if err := s.Onboard.UpdateProposalManifest(ctx, id, string(updated)); err != nil {
		t.Fatalf("UpdateProposalManifest: %v", err)
	}
	return id
}

// seedOrgMatchProposal stages a proposal whose org the CANDIDATE typed — the
// ordinary intake without a pre-bound org, and the case #1086 turns on.
func seedOrgMatchProposal(t *testing.T, s *Server, affID, nation string) string {
	t.Helper()
	return seedOrgProposalFrom(t, s, affID, nation, onboardstore.OrgSourceTyped)
}

// seedSponsorBoundProposal stages the other provenance: the invite pinned this
// org at mint, so nothing about it is the candidate's to claim.
func seedSponsorBoundProposal(t *testing.T, s *Server, affID, nation string) string {
	t.Helper()
	return seedOrgProposalFrom(t, s, affID, nation, onboardstore.OrgSourceSponsor)
}

func seedOrgProposalFrom(t *testing.T, s *Server, affID, nation, orgSource string) string {
	t.Helper()
	manifest := map[string]any{
		"identity":    map[string]any{"handle": "test-enrollee", "email": "enrollee@example.test", "type": "person", "status": "active"},
		"affiliation": map[string]any{"id": affID, "kind": "partner", "nation": nation},
	}
	mj, _ := json.Marshal(manifest)
	p := &onboardstore.Proposal{
		Sponsor: "op-handle", Env: "zagros", Handle: "test-enrollee",
		Email: "enrollee@example.test", Status: onboardstore.ProposalSponsorApproved,
		ManifestJSON: string(mj), OrgSource: orgSource,
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}
	return p.ID
}

func countOrgAttachAudits(s *Server, id string) int {
	n := 0
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_org_attach" && e.Target.InviteTokenPrefix == id {
			n++
		}
	}
	return n
}

func countOrgNewAudits(s *Server, id string) int {
	n := 0
	for _, e := range s.Audit.Recent(50) {
		if e.Action == "onboard_org_new" && e.Target.InviteTokenPrefix == id {
			n++
		}
	}
	return n
}

func TestSuggestEndpointFlagGate(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, false) // wired but DISABLED — the §13.2 rollback lever
	h := s.Routes()
	rr := orgReq(t, h, "GET", "/api/org-registry/suggest?q=anything", "runner-key", nil)
	if rr.Code != http.StatusNotFound {
		t.Fatalf("flag off: got %d want 404", rr.Code)
	}
	s.Cfg.OrgMatchEnabled = true
	if _, err := s.Orgs.CreateOrg(orgstore.Org{ID: "ruritanian-air-force", DisplayName: "Ruritanian Air Force", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	rr = orgReq(t, h, "GET", "/api/org-registry/suggest?q=Ruritanian+Air+Force", "runner-key", nil)
	if rr.Code != http.StatusOK {
		t.Fatalf("flag on: got %d body %s", rr.Code, rr.Body.String())
	}
	var resp struct {
		Candidates []orgstore.Candidate `json:"candidates"`
	}
	_ = json.Unmarshal(rr.Body.Bytes(), &resp)
	if len(resp.Candidates) == 0 || resp.Candidates[0].ID != "ruritanian-air-force" {
		t.Fatalf("candidates: %+v", resp.Candidates)
	}
}

// The browser-realm suggest endpoint behind the review panel's parent-org
// input: resolve status (direct + via alias) and candidates, honoring the
// same §13.2 rollback lever as the rest of the affordance. Fixture is the
// live case: a Control and Reporting Centre entered under the Ruritanian Air
// Force (registry alias "org124x").
//
//nolint:misspell // keep "Centre" spelling from the live org name
func TestOnboardOrgSuggestBrowserEndpoint(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, false) // wired but disabled
	h := s.Routes()

	get := func(q string) *httptest.ResponseRecorder {
		req := httptest.NewRequestWithContext(context.Background(), "GET", "/onboarding/org-suggest?q="+url.QueryEscape(q), nil)
		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)
		return rr
	}

	if rr := get("anything"); rr.Code != http.StatusNotFound {
		t.Fatalf("flag off: got %d want 404", rr.Code)
	}

	s.Cfg.OrgMatchEnabled = true
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-air-force", DisplayName: "Ruritanian Air Force", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	if err := orgs.AddAlias("org124x", "ruritanian-air-force", "abbreviation", "test"); err != nil {
		t.Fatal(err)
	}

	type resp struct {
		Resolves   bool                 `json:"resolves"`
		Canonical  string               `json:"canonical"`
		Candidates []orgstore.Candidate `json:"candidates"`
	}
	decode := func(rr *httptest.ResponseRecorder) resp {
		t.Helper()
		if rr.Code != http.StatusOK {
			t.Fatalf("got %d body %s", rr.Code, rr.Body.String())
		}
		var d resp
		if err := json.Unmarshal(rr.Body.Bytes(), &d); err != nil {
			t.Fatalf("decode: %v", err)
		}
		return d
	}

	// Direct id resolves.
	if d := decode(get("ruritanian-air-force")); !d.Resolves || d.Canonical != "ruritanian-air-force" {
		t.Fatalf("direct: %+v", d)
	}
	// Alias resolves to the canonical id.
	if d := decode(get("org124x")); !d.Resolves || d.Canonical != "ruritanian-air-force" {
		t.Fatalf("alias: %+v", d)
	}
	// A new lower org (the CRC) does not resolve — no silent create ever.
	if d := decode(get("control-and-reporting-centre")); d.Resolves {
		t.Fatalf("crc must not resolve: %+v", d)
	}
	// A near-miss surfaces the registered org as a candidate.
	if d := decode(get("ruritanian air force")); len(d.Candidates) == 0 || d.Candidates[0].ID != "ruritanian-air-force" {
		t.Fatalf("near-miss candidates: %+v", d)
	}
	// Empty query is a calm no-op, not an error.
	if d := decode(get("")); d.Resolves || len(d.Candidates) != 0 {
		t.Fatalf("empty query: %+v", d)
	}
}

// Render smoke: the review page with an active match panel must execute the
// template — grid rows for candidates, and the parent-org autocomplete field
// wired on the create-new form.
func TestOnboardingPageRendersOrgGrid(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-air-force", DisplayName: "Ruritanian Air Force", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	id := seedOrgMatchProposal(t, s, "ruritanian air force crc", "SVN")

	req := httptest.NewRequestWithContext(context.Background(), "GET", "/onboarding?status=sponsor_approved&id="+id, nil)
	rr := httptest.NewRecorder()
	s.Routes().ServeHTTP(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("render: got %d body %s", rr.Code, rr.Body.String())
	}
	body := rr.Body.String()
	for _, want := range []string{"ob-orggrid", "ruritanian-air-force", "org-parent-input", "org-suggest.js", "org-new"} {
		if !strings.Contains(body, want) {
			t.Fatalf("rendered page missing %q", want)
		}
	}
}

// The live gap this covers (screenshot): a candidate whose org IS
// registered — org131x under ruritanian-air-force under ruritanian-armed-forces —
// showed nothing at review; nobody could tell the hierarchy was captured.
// The review card must render the org + full ancestor chain, and the parent
// autocomplete's resolved response must carry the ancestors.
func TestOrgResolvedChainOnReviewCard(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	for _, o := range []orgstore.Org{
		{ID: "ruritanian-armed-forces", DisplayName: "Ruritanian Armed Forces", Kind: "partner", Nation: "SVN"},
		{ID: "ruritanian-air-force", DisplayName: "Ruritanian Air Force", Kind: "partner", Nation: "SVN"},
		{ID: "org131x", DisplayName: "Control and Reporting Centre", Kind: "partner", Nation: "SVN"}, //nolint:misspell // keep "Centre" spelling from the live org name
	} {
		if _, err := orgs.CreateOrg(o); err != nil {
			t.Fatal(err)
		}
	}
	if err := orgs.SetParent("ruritanian-air-force", "ruritanian-armed-forces", "test", "chain"); err != nil {
		t.Fatal(err)
	}
	if err := orgs.SetParent("org131x", "ruritanian-air-force", "test", "chain"); err != nil {
		t.Fatal(err)
	}

	id := seedSponsorBoundProposal(t, s, "org131x", "SVN")
	p, _ := s.Onboard.GetProposal(context.Background(), id)

	// No decision panel — the org resolves and the sponsor bound it.
	if f, cs := s.orgMatchesForProposal(*p); f.AffID != "" || cs != nil {
		t.Fatalf("resolved org must not render the decision panel: %q %+v", f.AffID, cs)
	}
	// The chain: self, parent, root — nearest-first.
	chain := s.orgChainForProposal(*p)
	if len(chain) != 3 || chain[0].ID != "org131x" || chain[1].ID != "ruritanian-air-force" || chain[2].ID != "ruritanian-armed-forces" {
		t.Fatalf("chain: %+v", chain)
	}

	// The rendered review card carries it.
	req := httptest.NewRequestWithContext(context.Background(), "GET", "/onboarding?status=sponsor_approved&id="+id, nil)
	rr := httptest.NewRecorder()
	s.Routes().ServeHTTP(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("render: got %d", rr.Code)
	}
	body := rr.Body.String()
	for _, want := range []string{"ob-orgchain", "org131x", "ruritanian-air-force", "ruritanian-armed-forces", "Control and Reporting Centre"} { //nolint:misspell // keep "Centre" spelling from the live org name
		if !strings.Contains(body, want) {
			t.Fatalf("rendered card missing %q", want)
		}
	}

	// The parent autocomplete's resolved response names the ancestors too.
	sreq := httptest.NewRequestWithContext(context.Background(), "GET", "/onboarding/org-suggest?q=org131x", nil)
	srr := httptest.NewRecorder()
	s.Routes().ServeHTTP(srr, sreq)
	var d struct {
		Resolves  bool     `json:"resolves"`
		Ancestors []string `json:"ancestors"`
	}
	if err := json.Unmarshal(srr.Body.Bytes(), &d); err != nil || !d.Resolves {
		t.Fatalf("suggest: %v %s", err, srr.Body.String())
	}
	if len(d.Ancestors) != 2 || d.Ancestors[0] != "ruritanian-air-force" || d.Ancestors[1] != "ruritanian-armed-forces" {
		t.Fatalf("ancestors: %+v", d.Ancestors)
	}
}

func TestOrgMatchesForProposalPanelLogic(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-land-component", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}

	// Unregistered typed org → panel with candidates, manifest facts riding
	// along for the create form's admin-confirmable prefills.
	id := seedOrgMatchProposal(t, s, "ruritanian-land-forces", "SVN")
	p, _ := s.Onboard.GetProposal(context.Background(), id)
	f, cs := s.orgMatchesForProposal(*p)
	if f.AffID != "ruritanian-land-forces" || len(cs) == 0 {
		t.Fatalf("panel: typed=%q candidates=%+v", f.AffID, cs)
	}
	if f.Kind != "partner" || f.Nation != "SVN" {
		t.Fatalf("manifest facts must ride along for the prefills: %+v", f)
	}

	// Registered org the SPONSOR bound → no panel, nothing to decide.
	id2 := seedSponsorBoundProposal(t, s, "ruritanian-land-component", "SVN")
	p2, _ := s.Onboard.GetProposal(context.Background(), id2)
	if f2, cs := s.orgMatchesForProposal(*p2); f2.AffID != "" || cs != nil {
		t.Fatalf("sponsor-bound org must render no panel: typed=%q cs=%+v", f2.AffID, cs)
	}

	// Same registered org, TYPED by the candidate → the panel renders, marked
	// as a claim, with the org itself attachable (#1086).
	id3 := seedOrgMatchProposal(t, s, "ruritanian-land-component", "SVN")
	p3, _ := s.Onboard.GetProposal(context.Background(), id3)
	f3, cs3 := s.orgMatchesForProposal(*p3)
	if f3.AffID != "ruritanian-land-component" || !f3.Registered {
		t.Fatalf("typed org that resolves must raise the panel as a claim: %+v", f3)
	}
	if len(cs3) == 0 || cs3[0].ID != "ruritanian-land-component" || cs3[0].Band != orgstore.BandExact {
		t.Fatalf("the claimed org must be attachable from its own panel: %+v", cs3)
	}

	// Flag off → never a panel.
	s.Cfg.OrgMatchEnabled = false
	if off, _ := s.orgMatchesForProposal(*p); off.AffID != "" {
		t.Fatalf("flag off must render no panel")
	}
}

func TestOrgUnregisteredErrNotFoundClearsWhenOrgRegistered(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	manifest := map[string]any{
		"identity":    map[string]any{"handle": "test-enrollee", "email": "enrollee@example.test", "type": "person", "status": "active"},
		"affiliation": map[string]any{"id": "unknown-org", "kind": "partner", "nation": "CAN"},
	}
	mj, _ := json.Marshal(manifest)
	p := &onboardstore.Proposal{
		Sponsor: "op-handle", Env: "zagros", Handle: "test-enrollee",
		Email: "enrollee@example.test", Status: onboardstore.ProposalProposed,
		ManifestJSON: string(mj),
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	if !s.orgUnregistered(*p) {
		t.Fatalf("orgUnregistered must be true for unknown affiliation id")
	}

	if _, err := orgs.CreateOrg(orgstore.Org{ID: "unknown-org", Kind: "partner", Nation: "CAN", Provenance: "operator-verified"}); err != nil {
		t.Fatalf("register org: %v", err)
	}
	if s.orgUnregistered(*p) {
		t.Fatalf("orgUnregistered must clear once org is registered")
	}
}

func webForm(t *testing.T, s *Server, path string, form url.Values) *httptest.ResponseRecorder {
	t.Helper()
	form.Set("csrf", s.csrfNonce)
	req := httptest.NewRequestWithContext(context.Background(), "POST", path, strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rr := httptest.NewRecorder()
	s.Routes().ServeHTTP(rr, req)
	return rr
}

func webGet(t *testing.T, s *Server, path string) *httptest.ResponseRecorder {
	t.Helper()
	rr := httptest.NewRecorder()
	s.Routes().ServeHTTP(rr, httptest.NewRequestWithContext(context.Background(), "GET", path, nil))
	return rr
}

// A draft whose candidate typed a non-Latin organisation reads back the same
// id the assembler will write, so the org panel and org-new can act on it
// instead of reporting that the manifest names no organization.
func TestOrgFactsFromManifest_DraftNonLatinOrgDerivesTheAssemblerID(t *testing.T) {
	s := newOnboardTestServer(t)
	const org = "Збройні сили"
	if got := slug(org); got != "" {
		t.Fatalf("slug(%q) = %q, want empty so the hash path is what is under test", org, got)
	}
	id := storeDraft(t, s, onboardstore.Prebound{}, map[string]any{"name": "Pat", "email": "pat@example.test", "organization": org})
	p, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatal(err)
	}
	want := affiliationIDFromOrg(org)
	if f := orgFactsFromManifest(p.ManifestJSON); want == "" || f.AffID != want {
		t.Fatalf("draft AffID = %q, want the assembler's id %q", f.AffID, want)
	}
}

func TestOrgAttachRewritesManifestAndAccretesAlias(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-land-component", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	id := seedOrgMatchProposal(t, s, "ruritanian-land-forces", "SVN")

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"ruritanian-land-component"}})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("attach: got %d body %s", rr.Code, rr.Body.String())
	}
	p, _ := s.Onboard.GetProposal(context.Background(), id)
	if f := orgFactsFromManifest(p.ManifestJSON); f.AffID != "ruritanian-land-component" {
		t.Fatalf("manifest not rewritten: %q", f.AffID)
	}
	// The typed form accreted as an alias → future Suggest resolves it.
	if canonical, err := orgs.Resolve("ruritanian-land-forces"); err != nil || canonical != "ruritanian-land-component" {
		t.Fatalf("alias accretion: %q err %v", canonical, err)
	}
}

func TestOrgNewCreatesWithNegativePairsAndChain(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-armed-forces", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "ruritanian-air-force", Kind: "partner", Nation: "SVN"}); err != nil {
		t.Fatal(err)
	}
	id := seedOrgMatchProposal(t, s, "ruritanian-air-forces-support", "SVN")

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"Ruritanian Air Forces Support"},
		"parent":       {"ruritanian-armed-forces"},
		"despite":      {"ruritanian-air-force"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
	}
	v, err := orgs.GetOrg("ruritanian-air-forces-support")
	if err != nil || v.Parent != "ruritanian-armed-forces" || v.Provenance != "operator-verified" {
		t.Fatalf("created org: %+v err %v", v, err)
	}
	// Human-judged-distinct: the rejected candidate is demoted for this name.
	cs, _ := orgs.Suggest("ruritanian-air-forces-support", "", 5)
	for _, c := range cs {
		if c.ID == "ruritanian-air-force" && (c.Band == orgstore.BandExact || c.Band == orgstore.BandHigh) {
			t.Fatalf("negative pair not demoted: %+v", c)
		}
	}
	// Unknown parent is an honest refusal, not a silent root.
	id2 := seedOrgMatchProposal(t, s, "another-new-org", "SVN")
	rr = webForm(t, s, "/onboarding/"+id2+"/org-new", url.Values{"parent": {"does-not-exist"}})
	if rr.Code != http.StatusConflict {
		t.Fatalf("unknown parent: got %d", rr.Code)
	}
}

// Org facts at create are ADMIN-CONFIRMED: the form's kind/nation win over
// the manifest's candidate self-description; absent fields fall back to the
// manifest (never to a kindless org). The `origin` row was minted
// kind=partner precisely because the create inherited one candidate's
// self-pick with no admin visibility.
func TestOrgNewAdminOverridesKindNation(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)

	// The seeded manifest says kind=partner; the admin corrects to vendor.
	id := seedOrgMatchProposal(t, s, "origin", "LVA")
	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"kind": {"vendor"}, "nation": {"lva"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new with overrides: got %d body %s", rr.Code, rr.Body.String())
	}
	v, err := orgs.GetOrg("origin")
	if err != nil || v.Kind != "vendor" || v.Nation != "LVA" {
		t.Fatalf("admin overrides must win (nation uppercased): %+v err %v", v, err)
	}

	// No form fields → NOTHING is inherited from the candidate's manifest
	// (ADR 1171). This assertion is the inverse of what it was: the manifest
	// fallback existed so an older cached form still produced a kinded org, and
	// it is exactly the inheritance that put a Canadian candidate's `CAN` on
	// offer as the controlling nation of a United States command .
	// An org fact is typed by the admin or it is absent.
	id2 := seedOrgMatchProposal(t, s, "second-org", "SVN")
	if rr := webForm(t, s, "/onboarding/"+id2+"/org-new", url.Values{}); rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new with no fields: got %d", rr.Code)
	}
	v2, err := orgs.GetOrg("second-org")
	if err != nil {
		t.Fatalf("GetOrg: %v", err)
	}
	if v2.Nation != "" {
		t.Errorf("nation = %q, want empty — a candidate's answer is not the org's nation", v2.Nation)
	}
	if v2.Kind != "" {
		t.Errorf("kind = %q, want empty — nothing was typed and nothing is derivable", v2.Kind)
	}
}

// After an org attach on a draft that CAN assemble, the draft must complete
// immediately and lose its INCOMPLETE marker, with the org change itself
// still succeeding.
func TestOrgAttach_CompletesDraftWhenCompletable(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraft(t, s, pb, sub)
	before, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(before): %v", err)
	}
	if before.Complete {
		t.Fatalf("seeded proposal must be an incomplete draft, got complete note=%q", before.Note)
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-attach: got %d body %s", rr.Code, rr.Body.String())
	}
	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if !after.Complete {
		t.Fatalf("proposal must complete after org attach, note=%q", after.Note)
	}
	if strings.Contains(after.Note, "INCOMPLETE (") {
		t.Fatalf("note %q still carries INCOMPLETE marker after completion", after.Note)
	}
	if !strings.Contains(after.Note, "— draft completed") {
		t.Fatalf("note %q must mark draft completed", after.Note)
	}
	var finalManifest map[string]any
	if err := json.Unmarshal([]byte(after.ManifestJSON), &finalManifest); err != nil {
		t.Fatalf("unmarshal final manifest: %v", err)
	}
	if finalAff, _ := finalManifest["affiliation"].(map[string]any); finalAff["id"] != "demoforge" {
		t.Fatalf("final assembled manifest affiliation.id = %v, want demoforge", finalAff["id"])
	}
}

// When a draft stays blocked after org attach (e.g. missing tier), the org
// change still succeeds and the note is rewritten to name the CURRENT
// blocker, replacing the previous INCOMPLETE reason.
func TestOrgAttach_BlockedDraftUpdatesNoteAndRedirects(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	// No prebound tier and no enrollee tier → errTierRequired at completion.
	pb := onboardstore.Prebound{}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraft(t, s, pb, sub)
	before, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(before): %v", err)
	}
	if before.Complete {
		t.Fatalf("seeded proposal must be an incomplete draft, got complete note=%q", before.Note)
	}
	originalNote := before.Note

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-attach (blocked): got %d body %s", rr.Code, rr.Body.String())
	}
	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if after.Complete {
		t.Fatalf("blocked draft must remain incomplete, got complete note=%q", after.Note)
	}
	if strings.Contains(after.Note, "affiliation") {
		t.Fatalf("note %q must replace the old INCOMPLETE reason, not keep %q", after.Note, originalNote)
	}
	if !strings.Contains(after.Note, "INCOMPLETE (") {
		t.Fatalf("note %q must still carry an INCOMPLETE marker", after.Note)
	}
	if strings.Count(after.Note, " — INCOMPLETE (") != 1 {
		t.Fatalf("note %q must have exactly one INCOMPLETE clause", after.Note)
	}
	if !strings.Contains(after.Note, "missing tier") {
		t.Fatalf("note %q must name the current blocker (missing tier)", after.Note)
	}
}

// A blocked attach clicked twice must not grow a second INCOMPLETE clause
// even when the blocker reason itself carries parentheses (ADR 1137 in the
// tier message).
func TestOrgAttach_BlockedDraftNoteDedupesOnSecondClick(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	// No prebound tier and no enrollee tier → errTierRequired at completion.
	pb := onboardstore.Prebound{}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraft(t, s, pb, sub)

	// First blocked click.
	_ = webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	afterFirst, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after first): %v", err)
	}
	if strings.Count(afterFirst.Note, " — INCOMPLETE (") != 1 {
		t.Fatalf("after first click, note %q must have exactly one INCOMPLETE clause", afterFirst.Note)
	}

	// Second blocked click — still one clause.
	_ = webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	afterSecond, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after second): %v", err)
	}
	if strings.Count(afterSecond.Note, " — INCOMPLETE (") != 1 {
		t.Fatalf("after second click, note %q must have exactly one INCOMPLETE clause", afterSecond.Note)
	}
}

// A draft that is initially blocked (missing tier) and later made completable
// must lose its INCOMPLETE marker when org-attach completes it, even when the
// blocker reason contained parentheses.
func TestOrgAttach_BlockedThenCompletedClearsIncompleteNote(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	// Start with a tierless draft so the first completion attempt blocks.
	pb := onboardstore.Prebound{}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraft(t, s, pb, sub)

	// First click: blocked, note carries an INCOMPLETE clause.
	_ = webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	afterBlocked, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after blocked): %v", err)
	}
	if !strings.Contains(afterBlocked.Note, "INCOMPLETE (") {
		t.Fatalf("blocked note %q must carry an INCOMPLETE marker", afterBlocked.Note)
	}

	// Make the draft completable by setting a tier in the stored intake record.
	var rec map[string]any
	if err := json.Unmarshal([]byte(afterBlocked.ManifestJSON), &rec); err != nil {
		t.Fatalf("unmarshal draft manifest: %v", err)
	}
	pre, _ := rec["prebound"].(map[string]any)
	if pre == nil {
		pre = map[string]any{}
	}
	pre["tier"] = "developer"
	rec["prebound"] = pre
	updated, err := json.Marshal(rec)
	if err != nil {
		t.Fatalf("marshal updated intake record: %v", err)
	}
	if err := s.Onboard.UpdateProposalManifest(context.Background(), id, string(updated)); err != nil {
		t.Fatalf("UpdateProposalManifest: %v", err)
	}

	// Second click: completion succeeds and the note loses the INCOMPLETE marker.
	_ = webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	afterComplete, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after complete): %v", err)
	}
	if !afterComplete.Complete {
		t.Fatalf("draft must complete after second click, note=%q", afterComplete.Note)
	}
	if strings.Contains(afterComplete.Note, "INCOMPLETE (") {
		t.Fatalf("note %q must drop the INCOMPLETE marker after completion", afterComplete.Note)
	}
	if !strings.Contains(afterComplete.Note, "— draft completed") {
		t.Fatalf("note %q must mark draft completed", afterComplete.Note)
	}
}

// Already complete proposals skip draft completion logic entirely, leaving the
// note intact and generating no draft-completion audit row, but the org attach
// itself still succeeds.
func TestOrgAttach_AlreadyCompleteProposal(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "demoforge-typed", "USA")

	// Force the proposal to be marked complete.
	if _, err := s.Onboard.DB().Exec(`UPDATE onboard_proposals SET complete = 1, note = 'original note' WHERE id = $1`, id); err != nil {
		t.Fatal(err)
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-attach: got %d", rr.Code)
	}

	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if after.Note != "original note" {
		t.Fatalf("note %q was altered, should have been skipped", after.Note)
	}

	if n := countDraftCompletionAudits(s, id); n != 0 {
		t.Fatalf("want zero completion audit rows for already complete proposal, got %d", n)
	}
}

// Already complete proposals skip draft completion logic entirely, leaving the
// note intact and generating no draft-completion audit row, but the org create
// itself still succeeds.
func TestOrgNew_AlreadyCompleteProposal(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "New Org",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "new-org", "USA")

	// Force the proposal to be marked complete.
	if _, err := s.Onboard.DB().Exec(`UPDATE onboard_proposals SET complete = 1, note = 'original note' WHERE id = $1`, id); err != nil {
		t.Fatal(err)
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
		"nation":       {"USA"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d", rr.Code)
	}

	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if after.Note != "original note" {
		t.Fatalf("note %q was altered, should have been skipped", after.Note)
	}

	if n := countDraftCompletionAudits(s, id); n != 0 {
		t.Fatalf("want zero completion audit rows for already complete proposal, got %d", n)
	}
}

// After an org create on a draft that CAN assemble, the draft must complete
// immediately and lose its INCOMPLETE marker, with the org registration
// itself still succeeding.
func TestOrgNew_CompletesDraftWhenCompletable(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	// Sponsor prebound a registered org id that does not match the slug of the
	// candidate's typed organization. Completion after org-new must assemble
	// against the CREATED org id, not re-slug the typed name.
	pb := onboardstore.Prebound{Tier: "developer", Org: "new-org-draft"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "New Org",
	}
	id := storeDraft(t, s, pb, sub)
	before, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(before): %v", err)
	}
	if before.Complete {
		t.Fatalf("seeded proposal must be an incomplete draft, got complete note=%q", before.Note)
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
		"nation":       {"USA"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
	}
	// The org itself must exist under the prebound id.
	if v, err := orgs.GetOrg("new-org-draft"); err != nil || v.DisplayName != "New Org" {
		t.Fatalf("created org: %+v err=%v", v, err)
	}
	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if !after.Complete {
		t.Fatalf("proposal must complete after org create, note=%q", after.Note)
	}
	if strings.Contains(after.Note, "INCOMPLETE (") {
		t.Fatalf("note %q still carries INCOMPLETE marker after completion", after.Note)
	}
	if !strings.Contains(after.Note, "— draft completed") {
		t.Fatalf("note %q must mark draft completed", after.Note)
	}
	var finalManifest map[string]any
	if err := json.Unmarshal([]byte(after.ManifestJSON), &finalManifest); err != nil {
		t.Fatalf("unmarshal final manifest: %v", err)
	}
	if finalAff, _ := finalManifest["affiliation"].(map[string]any); finalAff["id"] != "new-org-draft" {
		t.Fatalf("final assembled manifest affiliation.id = %v, want new-org-draft", finalAff["id"])
	}
}

// When a draft stays blocked after org create (e.g. missing tier), the org
// registration still succeeds and the note is rewritten to name the CURRENT
// blocker, replacing the previous INCOMPLETE reason.
func TestOrgNew_BlockedDraftUpdatesNoteAndRedirects(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	// No prebound tier and no enrollee tier → errTierRequired at completion.
	pb := onboardstore.Prebound{}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "New Org",
	}
	id := storeDraft(t, s, pb, sub)
	before, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(before): %v", err)
	}
	if before.Complete {
		t.Fatalf("seeded proposal must be an incomplete draft, got complete note=%q", before.Note)
	}
	originalNote := before.Note

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new (blocked): got %d body %s", rr.Code, rr.Body.String())
	}
	// The org registration itself must still succeed.
	if _, err := orgs.GetOrg("new-org"); err != nil {
		t.Fatalf("GetOrg(new-org-draft-blocked): %v", err)
	}
	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if after.Complete {
		t.Fatalf("blocked draft must remain incomplete, got complete note=%q", after.Note)
	}
	if strings.Contains(after.Note, "affiliation") {
		t.Fatalf("note %q must replace the old INCOMPLETE reason, not keep %q", after.Note, originalNote)
	}
	if !strings.Contains(after.Note, "INCOMPLETE (") {
		t.Fatalf("note %q must still carry an INCOMPLETE marker", after.Note)
	}
	if strings.Count(after.Note, " — INCOMPLETE (") != 1 {
		t.Fatalf("note %q must have exactly one INCOMPLETE clause", after.Note)
	}
	if !strings.Contains(after.Note, "missing tier") {
		t.Fatalf("note %q must name the current blocker (missing tier)", after.Note)
	}
}

func TestMintInviteSponsorAttributionHeader(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.OnboardOperatorAPIKeys = map[string]string{
		"legacy-key": "operator", // the generic label the region C invites exposed
		"person-key": "user367x",
	}
	h := s.Routes()
	mint := func(key, actorHeader string) (sponsor string) {
		body, _ := json.Marshal(map[string]any{"env": "zagros"})
		req := httptest.NewRequestWithContext(context.Background(), "POST", "/api/onboard/invite", bytes.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+key)
		if actorHeader != "" {
			req.Header.Set("X-Goat-Operator-Actor", actorHeader)
		}
		rr := httptest.NewRecorder()
		h.ServeHTTP(rr, req)
		if rr.Code != http.StatusOK && rr.Code != http.StatusCreated {
			t.Fatalf("mint: got %d body %s", rr.Code, rr.Body.String())
		}
		var resp struct {
			Sponsor string `json:"sponsor"`
		}
		_ = json.Unmarshal(rr.Body.Bytes(), &resp)
		return resp.Sponsor
	}
	// Generic label + header → the person is recorded.
	if got := mint("legacy-key", "maintainer-maintainer"); got != "maintainer-maintainer" {
		t.Fatalf("legacy+header: sponsor %q want maintainer-maintainer", got)
	}
	// Generic label, no header → stays generic (no invention).
	if got := mint("legacy-key", ""); got != "operator" {
		t.Fatalf("legacy no header: sponsor %q want operator", got)
	}
	// A real per-handle identity is NEVER overridden by the header.
	if got := mint("person-key", "someone-else"); got != "user367x" {
		t.Fatalf("per-handle key: sponsor %q want user367x (header must not override)", got)
	}
}

// --- ADR 1151 steps 3-4: the org-registration form asks two questions --------

// The form used to offer one four-value `kind` select that asked "what sort of
// thing is this" AND "is it ours" at once. It now asks sector (+ gov_branch for
// state) and DERIVES kind. These assert the derivation on the real handler, not
// on the library in isolation.
func TestOrgNewDerivesKindFromSector(t *testing.T) {
	cases := []struct {
		name              string
		sector, govBranch string
		nation, sovereign string
		wantKind          string
		wantBranch        string
	}{
		{
			name: "a foreign defense contractor — the case with no correct old answer",
			// vendor dropped the foreignness, partner dropped the commercialness.
			sector: "commercial", nation: "EST", sovereign: "USA",
			wantKind: "vendor",
		},
		{
			name:   "our own army",
			sector: "state", govBranch: "military", nation: "USA", sovereign: "USA",
			wantKind: "organic", wantBranch: "military",
		},
		{
			name:   "an allied foreign army is partner the ENTITY TYPE",
			sector: "state", govBranch: "military", nation: "EST", sovereign: "USA",
			wantKind: "partner", wantBranch: "military",
		},
		{
			name:   "a university",
			sector: "civil-society", nation: "USA", sovereign: "USA",
			wantKind: "civil",
		},
		{
			name: "a branch under a non-state sector is dropped, not stored — a " +
				"company has no branch of government",
			sector: "commercial", govBranch: "military", nation: "USA", sovereign: "USA",
			wantKind: "vendor", wantBranch: "",
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			orgs := withOrgs(t, s, true)
			s.Cfg.OperatingSovereign = c.sovereign
			id := seedOrgMatchProposal(t, s, "some-new-org", c.nation)

			rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
				"sector":     {c.sector},
				"gov_branch": {c.govBranch},
				"nation":     {c.nation},
			})
			if rr.Code != http.StatusSeeOther {
				t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
			}
			v, err := orgs.GetOrg("some-new-org")
			if err != nil {
				t.Fatalf("GetOrg: %v", err)
			}
			if v.Kind != c.wantKind {
				t.Errorf("kind = %q, want %q (derived from sector %q)", v.Kind, c.wantKind, c.sector)
			}
			if v.Sector != c.sector {
				t.Errorf("sector = %q, want %q", v.Sector, c.sector)
			}
			if v.GovBranch != c.wantBranch {
				t.Errorf("gov_branch = %q, want %q", v.GovBranch, c.wantBranch)
			}
		})
	}
}

// An explicit kind is an OVERRIDE and must beat the derivation — otherwise an
// admin correcting a wrong derivation would be silently overruled.
func TestOrgNewExplicitKindBeatsDerivation(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	s.Cfg.OperatingSovereign = "USA"
	id := seedOrgMatchProposal(t, s, "odd-one", "USA")

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"sector": {"commercial"}, // would derive vendor
		"kind":   {"civil"},      // admin says otherwise
		"nation": {"USA"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
	}
	v, _ := orgs.GetOrg("odd-one")
	if v.Kind != "civil" {
		t.Errorf("kind = %q, want the explicit civil — the admin's value must win", v.Kind)
	}
}

// With no operating sovereign declared, a STATE sector cannot decide organic vs
// partner. ADR 1151 step 3 is explicit about what must happen then: KEEP WHAT
// THE MANIFEST CARRIES, never invent a value. The seeded proposal declares
// kind=partner, so that is what the org must get — from the manifest, not from
// a guessed sovereign.
func TestOrgNewStateSectorWithoutSovereignLeavesKindUnknown(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	s.Cfg.OperatingSovereign = ""
	id := seedOrgMatchProposal(t, s, "undecidable-org", "EST") // manifest kind=partner

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"sector":     {"state"},
		"gov_branch": {"military"},
		"nation":     {"EST"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
	}
	v, err := orgs.GetOrg("undecidable-org")
	if err != nil {
		t.Fatalf("GetOrg: %v", err)
	}
	if v.Sector != "state" || v.GovBranch != "military" {
		t.Errorf("target block not stored: sector=%q gov_branch=%q", v.Sector, v.GovBranch)
	}
	// The derivation could not decide (no operating sovereign), and there is no
	// longer a manifest kind to fall through to — ADR 1171 removed that
	// inheritance. Unknown must survive to the row rather than being filled in
	// from the candidate's own description.
	if v.Kind != "" {
		t.Errorf("kind = %q, want empty — undecidable must stay unknown, not "+
			"inherit the candidate's self-description", v.Kind)
	}
}

// The same undecidable case, but with a manifest that carries NO kind either.
// Now there is genuinely nothing to fall back to, and the org must be created
// with an empty kind rather than a manufactured one. This is the case that
// proves "unknown" survives all the way to the row.
func TestOrgNewUndecidableAndNoManifestKindLeavesKindEmpty(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	s.Cfg.OperatingSovereign = ""

	manifest := map[string]any{
		"identity":    map[string]any{"handle": "test-enrollee", "email": "enrollee@example.test", "type": "person", "status": "active"},
		"affiliation": map[string]any{"id": "kindless-org", "nation": "EST"}, // no kind at all
	}
	mj, _ := json.Marshal(manifest)
	p := &onboardstore.Proposal{
		Sponsor: "op-handle", Env: "zagros", Handle: "test-enrollee",
		Email: "enrollee@example.test", Status: onboardstore.ProposalSponsorApproved,
		ManifestJSON: string(mj),
	}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	rr := webForm(t, s, "/onboarding/"+p.ID+"/org-new", url.Values{
		"sector": {"state"},
		"nation": {"EST"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d body %s", rr.Code, rr.Body.String())
	}
	v, err := orgs.GetOrg("kindless-org")
	if err != nil {
		t.Fatalf("GetOrg: %v", err)
	}
	if v.Kind != "" {
		t.Errorf("kind = %q, want empty — nothing declared it and nothing could "+
			"derive it, so the row must say so rather than guess", v.Kind)
	}
	if v.Sector != "state" {
		t.Errorf("sector = %q, want state — the fact that WAS declared must persist", v.Sector)
	}
}

// The live-derive endpoint behind the review form's read-only panel. What it
// returns must be what the create path records — the panel promising `member`
// and the row storing `partner` would be worse than showing nothing.
func TestAPIAffiliationDerive(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	s.Cfg.OperatingSovereign = "USA"
	s.Coalition = testCoalitionRegistry(t)

	get := func(nation, sector string) map[string]any {
		t.Helper()
		rr := webGet(t, s, "/onboarding/affiliation-derive?nation="+nation+"&sector="+sector)
		if rr.Code != 200 {
			t.Fatalf("derive %s/%s: got %d body %s", nation, sector, rr.Code, rr.Body.String())
		}
		var out map[string]any
		if err := json.Unmarshal(rr.Body.Bytes(), &out); err != nil {
			t.Fatalf("decode: %v", err)
		}
		return out
	}

	t.Run("our own nation reads ours", func(t *testing.T) {
		d := get("USA", "state")
		if d["organic_known"] != true || d["organic"] != true {
			t.Errorf("USA should be ours: %+v", d)
		}
		if d["organic_label"] != "ours" {
			t.Errorf("organic_label = %v, want ours", d["organic_label"])
		}
	})

	t.Run("an allied nation is foreign AND a member — the two are different axes", func(t *testing.T) {
		d := get("FRA", "state")
		if d["organic"] != false {
			t.Errorf("FRA is not the operating sovereign, so it is foreign: %+v", d)
		}
		if d["alliance_standing"] != "member" {
			t.Errorf("alliance_standing = %v, want member — FRA is in the fixture framework", d["alliance_standing"])
		}
	})

	t.Run("a non-member nation is partner", func(t *testing.T) {
		if d := get("UKR", "state"); d["alliance_standing"] != "partner" {
			t.Errorf("alliance_standing = %v, want partner", d["alliance_standing"])
		}
	})

	t.Run("an unresolvable nation fails toward partner, never membership", func(t *testing.T) {
		if d := get("ZZZ", "state"); d["alliance_standing"] != "partner" {
			t.Errorf("alliance_standing = %v, want partner for an unknown nation", d["alliance_standing"])
		}
	})

	t.Run("no nation says so rather than guessing", func(t *testing.T) {
		d := get("", "state")
		if _, ok := d["alliance_standing"]; ok {
			t.Errorf("no nation must not produce a standing: %+v", d)
		}
		if d["organic_known"] != false {
			t.Errorf("organic must be unknown without a nation: %+v", d)
		}
	})

	t.Run("with no operating sovereign, ours-vs-foreign is unknown, not foreign", func(t *testing.T) {
		s.Cfg.OperatingSovereign = ""
		defer func() { s.Cfg.OperatingSovereign = "USA" }()
		d := get("FRA", "state")
		if d["organic_known"] != false {
			t.Errorf("without a sovereign the question has no answer: %+v", d)
		}
		if _, ok := d["organic"]; ok {
			t.Errorf("organic must be absent, not false: %+v", d)
		}
		// Standing is independent of the sovereign and must still resolve.
		if d["alliance_standing"] != "member" {
			t.Errorf("standing should still resolve: %+v", d)
		}
	})
}

// --- the env/site hardcode class (F-4xx) -----------------------------------

// A console must stamp the goatnet it SERVES onto an invite, never a value the
// request supplied. A wrongly stamped invite's proposal can never onboard: the
// scoped runner skips it every 30 seconds, forever, while the console displays
// it as `approved`.
func TestInviteMintStampsTheConsolesOwnGoatnet(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.NotifyGoatnet = "zagros-prod"

	// No env in the form at all — the console supplies it.
	rr := webForm(t, s, "/onboarding/invite", url.Values{
		"candidate_email": {"someone@example.test"},
		"ttl_hours":       {"72"},
	})
	if rr.Code >= 400 {
		t.Fatalf("mint refused: %d %s", rr.Code, rr.Body.String())
	}
	invs, err := s.Onboard.ListInvites(context.Background(), "")
	if err != nil || len(invs) == 0 {
		t.Fatalf("no invite minted: %v", err)
	}
	if invs[0].Env != "zagros-prod" {
		t.Errorf("invite env = %q, want zagros-prod — the console's own goatnet", invs[0].Env)
	}
}

// A request naming a DIFFERENT goatnet is refused loudly rather than silently
// corrected or silently honoured. If this ever fires the form is stale or
// someone is minting across environments; both deserve to be seen at mint time
// instead of discovered when the person is still not onboarded weeks later.
func TestInviteMintRefusesAForeignGoatnet(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.NotifyGoatnet = "zagros-prod"

	rr := webForm(t, s, "/onboarding/invite", url.Values{
		"env":             {"zagros"},
		"candidate_email": {"someone@example.test"},
		"ttl_hours":       {"72"},
	})
	if rr.Code != 400 {
		t.Fatalf("expected a refusal, got %d %s", rr.Code, rr.Body.String())
	}
	if body := rr.Body.String(); !strings.Contains(body, "zagros-prod") || !strings.Contains(body, "zagros") {
		t.Errorf("the refusal must name BOTH goatnets so the reader can see the mismatch: %s", body)
	}
}

// A realistic caller payload from the org-new form includes all rendered fields,
// empty or not, and any despite (checkbox) values. The handler must process it
// and successfully complete the draft.
func TestOrgNew_RealisticCallerPayload(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Unknown Defense",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "unknown-defense-typed", "ISL")

	// The web template (onboarding.html) sends these fields.
	payload := url.Values{
		"composition":  {"national"},
		"sector":       {"state"},
		"gov_branch":   {" MiLiTaRy "}, // Human free text with mixed case/spaces
		"nation":       {" iSl "},
		"display_name": {"Unknown Defense Ltd"},
		"parent":       {""},
		"despite":      {"some-other-org"}, // despite checkboxes, if any were shown
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", payload)
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new realistic payload: got %d body %s", rr.Code, rr.Body.String())
	}

	// Validate it hit the orgstore with the derived/normalized facts.
	org, err := orgs.GetOrg("unknown-defense-typed")
	if err != nil {
		t.Fatalf("GetOrg: %v", err)
	}
	if org.GovBranch != "military" || org.Nation != "ISL" || org.Composition != "national" || org.Sector != "state" {
		t.Errorf("normalized org facts = %+v; want military, ISL, national, state", org)
	}

	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if !after.Complete {
		t.Fatalf("proposal must complete after org create, note=%q", after.Note)
	}
}

// Past the review stages the manifest is frozen. A concurrent completion or a
// late action returns 409 instead of rewriting the manifest (org-attach).
func TestOrgAttach_FrozenAnswers409(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}

	for _, status := range []string{
		onboardstore.ProposalApproved,
		onboardstore.ProposalOnboarded,
		onboardstore.ProposalRejected,
	} {
		pb := onboardstore.Prebound{Tier: "developer"}
		sub := map[string]any{"name": "Pat Draft", "email": "pat@example.test", "organization": "Unknown"}
		id := storeDraftWithAffiliation(t, s, pb, sub, "unknown-"+status, "USA")

		// Force the proposal into the frozen status
		_, err := s.Onboard.DB().Exec(`UPDATE onboard_proposals SET status = $1, complete = 1 WHERE id = $2`, status, id)
		if err != nil {
			t.Fatalf("force status: %v", err)
		}

		rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
		if rr.Code != http.StatusConflict {
			t.Errorf("status %s: want 409, got %d %s", status, rr.Code, rr.Body.String())
		}
		if !strings.Contains(rr.Body.String(), "frozen") {
			t.Errorf("status %s: 409 should mention frozen manifest, got %s", status, rr.Body.String())
		}
	}
}

// Past the review stages the manifest is frozen. A concurrent completion or a
// late action returns 409 instead of creating the org (org-new).
func TestOrgNew_FrozenAnswers409(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)

	for _, status := range []string{
		onboardstore.ProposalApproved,
		onboardstore.ProposalOnboarded,
		onboardstore.ProposalRejected,
	} {
		pb := onboardstore.Prebound{Tier: "developer"}
		sub := map[string]any{"name": "Pat Draft", "email": "pat@example.test", "organization": "Unknown"}
		id := storeDraftWithAffiliation(t, s, pb, sub, "unknown-"+status, "USA")

		// Force the proposal into the frozen status
		_, err := s.Onboard.DB().Exec(`UPDATE onboard_proposals SET status = $1, complete = 1 WHERE id = $2`, status, id)
		if err != nil {
			t.Fatalf("force status: %v", err)
		}

		rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
			"display_name": {"New Org"},
			"nation":       {"USA"},
		})
		if rr.Code != http.StatusConflict {
			t.Errorf("status %s: want 409, got %d %s", status, rr.Code, rr.Body.String())
		}
		if !strings.Contains(rr.Body.String(), "frozen") {
			t.Errorf("status %s: 409 should mention frozen manifest, got %s", status, rr.Body.String())
		}
	}
}

// A raced completion returns 409: when completeDraft reports errDraftChanged,
// the org-attach handler must answer 409 rather than 200/303.
func TestOrgAttach_RacedCompletionReturns409(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraft(t, s, pb, sub)

	// Force completeDraft to behave as if another admin completed the draft
	// first: every call reports errDraftChanged so the handler must surface a
	// 409 and no completion audit row is written here.
	s.testCompleteDraftOverride = func(ctx context.Context, p *onboardstore.Proposal, by draftCompleter) (draftOutcome, string, error) {
		return draftBlocked, "", errDraftChanged
	}
	defer func() { s.testCompleteDraftOverride = nil }()

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	if rr.Code != http.StatusConflict {
		t.Fatalf("raced completion: want 409, got %d body %s", rr.Code, rr.Body.String())
	}
	if n := countOrgAttachAudits(s, id); n != 1 {
		t.Fatalf("want exactly one org-attach audit row despite raced completion, got %d", n)
	}
}

// A raced completion returns 409 on org-new as well.
func TestOrgNew_RacedCompletionReturns409(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "New Org",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "new-org", "USA")

	s.testCompleteDraftOverride = func(ctx context.Context, p *onboardstore.Proposal, by draftCompleter) (draftOutcome, string, error) {
		return draftBlocked, "", errDraftChanged
	}
	defer func() { s.testCompleteDraftOverride = nil }()

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
		"nation":       {"USA"},
	})
	if rr.Code != http.StatusConflict {
		t.Fatalf("raced completion (org-new): want 409, got %d body %s", rr.Code, rr.Body.String())
	}
	if n := countOrgNewAudits(s, id); n != 1 {
		t.Fatalf("want exactly one org-new audit row despite raced completion, got %d", n)
	}
}

// The real caller payload for org-attach includes only the org ID.
func TestOrgAttach_RealisticCallerPayload(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "demoforge-typed", "USA")

	// The web template (onboarding.html) sends only csrf and org for org-attach.
	payload := url.Values{
		"org": {"demoforge"},
	}

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", payload)
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-attach realistic payload: got %d body %s", rr.Code, rr.Body.String())
	}

	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if !after.Complete {
		t.Fatalf("proposal must complete after org attach, note=%q", after.Note)
	}
}

// Completing a draft through org-attach must write an onboard_draft_complete audit row.
func TestOrgAttach_EmitAuditOnCompletion(t *testing.T) {
	s := newOnboardTestServer(t)
	orgs := withOrgs(t, s, true)
	if _, err := orgs.CreateOrg(orgstore.Org{ID: "demoforge", DisplayName: "Demoforge", Kind: "partner", Nation: "USA"}); err != nil {
		t.Fatal(err)
	}
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Demoforge",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "demoforge-typed", "USA")

	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"demoforge"}})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-attach: got %d", rr.Code)
	}

	if n := countDraftCompletionAudits(s, id); n != 1 {
		t.Fatalf("want exactly one completion audit row, got %d", n)
	}
}

// Completing a draft through org-new must write an onboard_draft_complete audit row.
func TestOrgNew_EmitAuditOnCompletion(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "New Org",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "new-org", "USA")

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
		"nation":       {"USA"},
	})
	if rr.Code != http.StatusSeeOther {
		t.Fatalf("org-new: got %d", rr.Code)
	}

	if n := countDraftCompletionAudits(s, id); n != 1 {
		t.Fatalf("want exactly one completion audit row, got %d", n)
	}
}

// A user input that fails the callee's validation (CreateOrg) halts the process
// and returns the 409 conflict, leaving the draft incomplete.
func TestOrgNew_CalleeValidationFailure(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Invalid Nation",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "invalid-nation-typed", "USA")

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"Invalid Nation"},
		"kind":         {"invalid-kind"},
	})
	if rr.Code != http.StatusConflict {
		t.Fatalf("org-new invalid nation: want 409, got %d", rr.Code)
	}
	if !strings.Contains(rr.Body.String(), "refused") {
		t.Errorf("expected refusal message in body, got %q", rr.Body.String())
	}

	after, err := s.Onboard.GetProposal(context.Background(), id)
	if err != nil {
		t.Fatalf("GetProposal(after): %v", err)
	}
	if after.Complete {
		t.Fatalf("draft must not complete when org validation fails")
	}
}

// Unit test for rewriteManifestAffiliation when the intake record has no
// prebound object at all (it must create one so the canonical org ID is
// pinned).
func TestRewriteManifestAffiliation_IntakeRecordWithoutPrebound(t *testing.T) {
	manifestJSON := `{"schema_version": "candidate-intake/v-draft", "submission": "..."}`
	updated, err := rewriteManifestAffiliation(manifestJSON, "new-canonical-org")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	var m map[string]any
	if err := json.Unmarshal([]byte(updated), &m); err != nil {
		t.Fatalf("unmarshal updated manifest: %v", err)
	}
	pre, ok := m["prebound"].(map[string]any)
	if !ok || pre == nil {
		t.Fatalf("expected prebound object to be created, got %v", pre)
	}
	if pre["org"] != "new-canonical-org" {
		t.Errorf("prebound.org = %v, want new-canonical-org", pre["org"])
	}
	aff, ok := m["affiliation"].(map[string]any)
	if !ok || aff == nil {
		t.Fatalf("expected affiliation object to be created, got %v", aff)
	}
	if aff["id"] != "new-canonical-org" {
		t.Errorf("affiliation.id = %v, want new-canonical-org", aff["id"])
	}
}

// Missing org parameter returns 400.
func TestOrgAttach_RequiresOrg(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	id := storeDraft(t, s, onboardstore.Prebound{}, map[string]any{})
	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{})
	if rr.Code != http.StatusBadRequest {
		t.Errorf("want 400 for missing org, got %d", rr.Code)
	}
}

// Missing AffID in the derived org facts returns 400.
func TestOrgNew_RequiresManifestOrganization(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	// Create a draft with no prebound org and no organization in submission.
	id := storeDraft(t, s, onboardstore.Prebound{}, map[string]any{"organization": "   "})
	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"New Org"},
		"nation":       {"USA"},
	})
	if rr.Code != http.StatusBadRequest {
		t.Errorf("want 400 when manifest names no organization, got %d %s", rr.Code, rr.Body.String())
	}
}

// 404 when org-attach targets an unknown org
func TestOrgAttach_OrgNotFound(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	id := storeDraft(t, s, onboardstore.Prebound{}, map[string]any{})
	rr := webForm(t, s, "/onboarding/"+id+"/org-attach", url.Values{"org": {"does-not-exist"}})
	if rr.Code != http.StatusNotFound {
		t.Errorf("want 404 for unknown org, got %d", rr.Code)
	}
}

// 409 when org-new requests a parent that does not exist
func TestOrgNew_ParentAttachRefused(t *testing.T) {
	s := newOnboardTestServer(t)
	withOrgs(t, s, true)
	pb := onboardstore.Prebound{Tier: "developer"}
	sub := map[string]any{
		"name":         "Pat Draft",
		"email":        "pat@example.test",
		"organization": "Unknown Defense",
	}
	id := storeDraftWithAffiliation(t, s, pb, sub, "unknown-defense-typed", "ISL")

	rr := webForm(t, s, "/onboarding/"+id+"/org-new", url.Values{
		"display_name": {"Unknown Defense Ltd"},
		"composition":  {"national"},
		"sector":       {"state"},
		"gov_branch":   {"military"},
		"nation":       {"ISL"},
		"parent":       {"does-not-exist"},
	})
	if rr.Code != http.StatusConflict {
		t.Fatalf("want 409 for missing parent, got %d body %s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "parent attach refused") {
		t.Errorf("expected parent attach refused message, got %q", rr.Body.String())
	}
}

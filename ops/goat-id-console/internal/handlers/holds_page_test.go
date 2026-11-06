package handlers

import (
	"context"
	"encoding/json"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/web"
)

// kratosDirectoryRoundTripper serves a fixed set of identities: the paged
// list the roster join reads, and the by-id GET the member page reads.
type kratosDirectoryRoundTripper struct {
	identities []map[string]any
}

func (rt *kratosDirectoryRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	switch {
	case req.Method == http.MethodGet && req.URL.Path == "/admin/identities":
		if req.URL.Query().Get("page") == "0" {
			_ = json.NewEncoder(w).Encode(rt.identities)
		} else {
			_ = json.NewEncoder(w).Encode([]any{})
		}
	case req.Method == http.MethodGet && strings.HasPrefix(req.URL.Path, "/admin/identities/"):
		id := strings.TrimPrefix(req.URL.Path, "/admin/identities/")
		for _, i := range rt.identities {
			if i["id"] == id {
				_ = json.NewEncoder(w).Encode(i)
				return w.Result(), nil
			}
		}
		w.WriteHeader(http.StatusNotFound)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
	return w.Result(), nil
}

// holdsFailingRoundTripper is a directory that answers 502 to everything.
type holdsFailingRoundTripper struct{}

func (holdsFailingRoundTripper) RoundTrip(*http.Request) (*http.Response, error) {
	return &http.Response{StatusCode: http.StatusBadGateway, Body: http.NoBody, Header: http.Header{}}, nil
}

func holdsIdentity(id, email, name, orgTag, orgName string) map[string]any {
	return map[string]any{
		"id": id, "state": "active",
		"traits": map[string]any{
			"email": email, "name": name, "groups": []any{"MEMBERS"},
			"org": map[string]any{"short_tag": orgTag, "display_name": orgName},
		},
	}
}

// holdsFixtureServer is a console on a test goatnet with a roster of three
// onboarded members (jane-doe, john-roe, kim-lee) and their logins.
func holdsFixtureServer(t *testing.T) *Server {
	t.Helper()
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: &kratosDirectoryRoundTripper{identities: []map[string]any{
		holdsIdentity("id-jane", "jane@example.test", "Jane Doe", "ACME", "Acme Corp"),
		holdsIdentity("id-john", "john@example.test", "John Roe", "ACME", "Acme Corp"),
		holdsIdentity("id-kim", "kim@example.test", "Kim Lee", "BLUE", "Blue Ltd"),
		// ned has a login here; whether the page links it is the join's call.
		holdsIdentity("id-ned", "ned@example.test", "Ned Hue", "BLUE", "Blue Ltd"),
		// tia has a login here; two same-second proposals decide which handle links it.
		holdsIdentity("id-tia", "tia@example.test", "Tia Sol", "BLUE", "Blue Ltd"),
	}}}
	s := newUsersAPITestServer(t, cli)
	s.Cfg.NotifyGoatnet = "zagros"
	ctx := context.Background()
	for _, m := range [][2]string{{"jane-doe", "jane@example.test"}, {"john-roe", "john@example.test"}, {"kim-lee", "kim@example.test"}} {
		p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: m[0], Email: m[1], Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatalf("CreateProposal %s: %v", m[0], err)
		}
	}
	return s
}

const holdsFakeNetBirdPAT = "NETBIRD-TOKEN-SECRET-VALUE"

// holdsFakeNetBird serves GET /api/peers with the given peers and an
// empty setup-key list, and points the server at it.
func holdsFakeNetBird(t *testing.T, s *Server, peers []map[string]any) {
	t.Helper()
	nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/api/peers":
			_ = json.NewEncoder(w).Encode(peers)
		case "/api/setup-keys":
			_ = json.NewEncoder(w).Encode([]any{})
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	t.Cleanup(nb.Close)
	s.Cfg.NotifyMgmtURL = nb.URL
	s.Cfg.EnrollWatchToken = holdsFakeNetBirdPAT
}

func holdsPeerJSON(id, name, lastSeen string, groups ...string) map[string]any {
	gs := []map[string]any{{"id": "g-all", "name": "All"}}
	for _, g := range groups {
		gs = append(gs, map[string]any{"id": "g-" + g, "name": g})
	}
	return map[string]any{"id": id, "name": name, "connected": false, "last_seen": lastSeen, "groups": gs}
}

// holdsFixturePeers is the live mesh that goes with the shared fixture:
// jane's laptop in its PROBATION group, john's desktop in QUARANTINED, plus
// a no-record QUARANTINED peer of kim's, an inconsistent peer of kim's, and
// a hold-group peer no roster handle owns.
func holdsFixturePeers() []map[string]any {
	return []map[string]any{
		holdsPeerJSON("d0f1e2a3b4c5d6e7f8091a2b", "peer-jane-doe-acme-zagros-20260601-laptop", "2026-09-10T10:00:00Z", "ZAGROS-PROBATION-DEVELOPER"),
		holdsPeerJSON("0a1b2c3d4e5f60718293a4b5", "peer-john-roe-acme-zagros-20260415-desktop", "2026-09-02T10:00:00Z", "ZAGROS-QUARANTINED"),
		holdsPeerJSON("p-kim-1", "peer-kim-lee-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
		holdsPeerJSON("p-kim-2", "peer-kim-lee-blue-zagros-20260701-phone", "2026-09-21T10:00:00Z", "ZAGROS-PROBATION-OBSERVER", "ZAGROS-OBSERVER"),
		holdsPeerJSON("p-unowned", "peer-nobody-zagros-20260701-box", "2026-09-21T10:00:00Z", "ZAGROS-QUARANTINED"),
		holdsPeerJSON("p-free", "peer-kim-lee-blue-zagros-20260701-tablet", "2026-09-22T10:00:00Z", "ZAGROS-OBSERVER"),
	}
}

func getHolds(t *testing.T, s *Server, target string, groups string) (int, string) {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, target, nil)
	req.Header.Set("X-Forwarded-Groups", groups)
	req.Header.Set("X-Forwarded-Email", "admin@example.test")
	req.Header.Set("X-Forwarded-User", "admin-id")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec.Code, rec.Body.String()
}

// tbodyFor returns the <tbody> of the row for one peer id.
func tbodyFor(t *testing.T, body, peerID string) string {
	t.Helper()
	i := strings.Index(body, `<code class="muted">`+peerID+`</code>`)
	if i < 0 {
		t.Fatalf("no row for peer %s", peerID)
	}
	start := strings.LastIndex(body[:i], "<tbody")
	end := strings.Index(body[i:], "</tbody>")
	if start < 0 || end < 0 {
		t.Fatalf("row for %s is not inside a tbody", peerID)
	}
	return body[start : i+end]
}

var holdBadgeRe = regexp.MustCompile(`<span class="status-badge hold-(kind|state)[^"]*">([^<]*)</span>`)

func badgeTexts(s string) []string {
	var out []string
	for _, m := range holdBadgeRe.FindAllStringSubmatch(s, -1) {
		out = append(out, m[1]+"="+m[2])
	}
	return out
}

func TestReleaseOutcomePresentation(t *testing.T) {
	for _, tc := range []struct {
		outcome string
		label   string
		class   string
	}{
		{"released", "released", "success"},
		{"already_released", "already released", "success"},
		{"refused", "refused", "error"},
		{"retryable_partial", "retryable partial progress", "warn"},
		{"unexpected", "", ""},
	} {
		t.Run(tc.outcome, func(t *testing.T) {
			label, class := releaseOutcomePresentation(tc.outcome)
			if label != tc.label || class != tc.class {
				t.Fatalf("releaseOutcomePresentation(%q) = %q, %q; want %q, %q", tc.outcome, label, class, tc.label, tc.class)
			}
		})
	}
}

func TestHoldsPageReleaseResultBannerIsRecentAndIdentified(t *testing.T) {
	seedResult := func(t *testing.T, finished time.Time) *Server {
		t.Helper()
		s := holdsFixtureServer(t)
		s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
		holdsFakeNetBird(t, s, holdsFixturePeers())
		entered := time.Date(2026, time.September, 2, 11, 24, 0, 0, time.UTC)
		params, err := json.Marshal(releaseHoldParams{Site: "zagros", PeerID: "b1c2d3e4f5a60718293b4c5d", PeerName: "release-target-laptop", HoldEnteredAt: entered.Format(time.RFC3339)})
		if err != nil {
			t.Fatal(err)
		}
		a, err := s.Onboard.EnqueueHoldAction(context.Background(), "zagros", "b1c2d3e4f5a60718293b4c5d", entered, "release-target", "admin@example.test", string(params))
		if err != nil {
			t.Fatal(err)
		}
		if err := s.Onboard.ClaimAction(context.Background(), a.ID, "runner"); err != nil {
			t.Fatal(err)
		}
		claimed, err := s.Onboard.GetAction(context.Background(), a.ID)
		if err != nil {
			t.Fatal(err)
		}
		if err := s.Onboard.SetReleaseActionResult(context.Background(), a.ID, claimed.ClaimToken, onboardstore.ActionFailed,
			`{"release":{"outcome":"refused","audit_outcome":"recorded"}}`, "refused", "b1c2d3e4f5a60718293b4c5d", entered, "admin@example.test", false); err != nil {
			t.Fatal(err)
		}
		if _, err := s.Onboard.DB().Exec(`UPDATE onboard_actions SET finished_at = $1 WHERE id = $2`, finished.Unix(), a.ID); err != nil {
			t.Fatal(err)
		}
		return s
	}

	recent := time.Now().UTC().Add(-time.Hour).Truncate(time.Second)
	s := seedResult(t, recent)
	_, body := getHolds(t, s, "/holds", "ADMINS")
	want := "Hold release for release-target-laptop finished at " + recent.Format("2006-01-02 15:04:05 UTC") + ": refused."
	if !strings.Contains(body, want) {
		t.Fatalf("recent release banner missing %q", want)
	}
	_, queued := getHolds(t, s, "/holds?release=queued", "ADMINS")
	if strings.Contains(queued, `id="release-terminal"`) {
		t.Fatal("terminal result appears beside the release-queued banner")
	}

	old := seedResult(t, time.Now().UTC().Add(-25*time.Hour))
	_, body = getHolds(t, old, "/holds", "ADMINS")
	if strings.Contains(body, `id="release-terminal"`) {
		t.Fatal("release result older than 24 hours rendered")
	}
}

// The populated page: the fixture's two lanes side by side with reason and
// actor, a no-record containment, an inconsistent peer, an unowned peer,
// member links from the roster join, the counts, and nothing credential-
// shaped in the body.
func TestHoldsPage_PopulatedFromFixtureAndMesh(t *testing.T) {
	s := holdsFixtureServer(t)
	s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
	holdsFakeNetBird(t, s, holdsFixturePeers())

	code, body := getHolds(t, s, "/holds", "ADMINS")
	if code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body: %s", code, body)
	}
	if !strings.Contains(body, "Mesh holds — <code>zagros</code>") {
		t.Error("heading does not name the goatnet")
	}
	if strings.Contains(body, holdsFakeNetBirdPAT) {
		t.Fatal("response leaked the NetBird token")
	}
	if strings.Contains(strings.ToLower(body), `method="post"`) || strings.Contains(body, `type="submit"`) {
		t.Error("read-only page renders a form or a submit control")
	}

	jane := tbodyFor(t, body, "d0f1e2a3b4c5d6e7f8091a2b")
	if got := badgeTexts(jane); strings.Join(got, ",") != "kind=dormancy,state=held" {
		t.Errorf("jane badges = %v", got)
	}
	for _, want := range []string{`href="/users/id-jane"`, "jane-doe", "Acme Corp", "ACME", "2026-09-01", "ZAGROS-DEVELOPER", "ZAGROS-P2P", "Reason: inactive 17d (person threshold 14d)", "Actor: sweep", `data-band="dormancy"`} {
		if !strings.Contains(jane, want) {
			t.Errorf("jane row missing %q:\n%s", want, jane)
		}
	}

	john := tbodyFor(t, body, "0a1b2c3d4e5f60718293a4b5")
	if got := badgeTexts(john); strings.Join(got, ",") != "kind=security,state=held" {
		t.Errorf("john badges = %v", got)
	}
	for _, want := range []string{`href="/users/id-john"`, "Reason: credential exposure suspected", "Actor: jane-admin", "ZAGROS-OBSERVER", `data-band="security"`} {
		if !strings.Contains(john, want) {
			t.Errorf("john row missing %q:\n%s", want, john)
		}
	}

	kim1 := tbodyFor(t, body, "p-kim-1")
	if got := badgeTexts(kim1); strings.Join(got, ",") != "kind=security,state=held" {
		t.Errorf("no-record containment badges = %v (the contract: a hold group with no record is a security hold)", got)
	}
	if !strings.Contains(kim1, `href="/users/id-kim"`) || !strings.Contains(kim1, "no hold record") {
		t.Errorf("no-record row: %s", kim1)
	}
	kim2 := tbodyFor(t, body, "p-kim-2")
	if got := badgeTexts(kim2); strings.Join(got, ",") != "kind=security,state=inconsistent, investigate" {
		t.Errorf("inconsistent badges = %v", got)
	}
	if !strings.Contains(kim2, "ZAGROS-OBSERVER") || !strings.Contains(kim2, `data-band="attention"`) {
		t.Errorf("inconsistent row must name the groups and sit in the attention band: %s", kim2)
	}
	// Kind and State are two facts on the row: the quick filters read
	// data-kind / data-attention, so kim's quarantined-but-inconsistent
	// phone answers to "Security" as well as to "Needs attention".
	if !strings.Contains(kim2, `data-kind="security"`) || !strings.Contains(kim2, `data-attention="1"`) {
		t.Errorf("inconsistent security row must carry data-kind=security and data-attention=1: %s", kim2)
	}
	if !strings.Contains(jane, `data-kind="dormancy"`) || !strings.Contains(jane, `data-attention="0"`) {
		t.Errorf("held dormancy row must carry data-kind=dormancy and data-attention=0: %s", jane)
	}
	unowned := tbodyFor(t, body, "p-unowned")
	if !strings.Contains(unowned, "not attributed to a member") || strings.Contains(unowned, `href="/users/`) {
		t.Errorf("unowned row: %s", unowned)
	}
	if !regexp.MustCompile(`data-filter="[^"]*p-unowned[^"]*"`).MatchString(unowned) {
		t.Errorf("unowned row filter does not contain its peer ID: %s", unowned)
	}
	if strings.Contains(body, "p-free") {
		t.Error("a peer in no hold group is not a row")
	}

	// Summary counts: 5 on hold; by kind 1 dormancy + 4 security (kim's
	// inconsistent phone is a security hold too); by state 1 attention.
	// The kind counts partition the total, attention overlaps them, and
	// the legend says so.
	for _, want := range []string{`data-band="all" aria-pressed="true">On hold <b>5</b>`, `data-band="dormancy" aria-pressed="false">Dormancy <b>1</b>`, `data-band="security" aria-pressed="false">Security <b>4</b>`, `data-band="attention" aria-pressed="false">Needs attention <b>1</b>`, `id="holds-legend"`, "the counts overlap rather than add up"} {
		if !strings.Contains(body, want) {
			t.Errorf("summary missing %q", want)
		}
	}
	if strings.Contains(body, `data-band="unknown"`) {
		t.Error("Kind unknown button must not render when every kind was read")
	}
	// Attention first, then security, then dormancy.
	if strings.Index(body, "p-kim-2") >= strings.Index(body, "p-kim-1") || strings.Index(body, "p-kim-1") >= strings.Index(body, "d0f1e2a3b4c5d6e7f8091a2b") {
		t.Error("default order is not attention, security, dormancy")
	}
	if !strings.Contains(body, "6 peers checked") || !strings.Contains(body, "Checked at") {
		t.Error("checked-at line missing")
	}
	if !strings.Contains(body, `<a href="/holds?idp=tenant">Mesh holds</a>`) {
		t.Error("nav entry missing")
	}
	if !strings.Contains(body, `<td class="hold-action"></td>`) {
		t.Error("trailing empty action cell missing")
	}
	if !strings.Contains(body, `data-filter="jane-doe acme corp acme peer-jane-doe-acme-zagros-20260601-laptop d0f1e2a3b4c5d6e7f8091a2b"`) {
		t.Error("filter haystack must carry handle, org name, org tag, device name and peer ID")
	}
}

// The member link follows the proposal on THIS goatnet: a newer proposal
// for the same handle on another goatnet, with another email, must not
// redirect the link.
func TestHoldsPage_LinkFollowsThisGoatnetsProposal(t *testing.T) {
	s := holdsFixtureServer(t)
	p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros-prod", Handle: "jane-doe", Email: "kim@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer", CreatedAt: time.Now().Unix() + 100}
	if err := s.Onboard.CreateProposal(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
	holdsFakeNetBird(t, s, holdsFixturePeers())
	_, body := getHolds(t, s, "/holds", "ADMINS")
	jane := tbodyFor(t, body, "d0f1e2a3b4c5d6e7f8091a2b")
	if !strings.Contains(jane, `href="/users/id-jane"`) || strings.Contains(jane, "id-kim") {
		t.Errorf("jane's link followed a foreign-goatnet proposal: %s", jane)
	}
	// The same proposal moved kim's EMAIL: the newest active onboarding for it is
	// now on another goatnet, so kim's member page would refuse to derive the
	// handle (resolveAccess reads ActiveProposalForEmail across goatnets).
	// This page must not link to a page that disowns the handle, even
	// though the directory knows the email.
	kim := tbodyFor(t, body, "p-kim-1")
	if strings.Contains(kim, `href="/users/`) || !strings.Contains(kim, "the newest onboarding of its login is on zagros-prod") {
		t.Errorf("kim's link must be withheld and the row must say why: %s", kim)
	}
}

// Two held devices of one member render as adjacent rows, each linking to
// the same member page.
func TestHoldsPage_TwoDevicesOneMemberAdjacent(t *testing.T) {
	s := holdsFixtureServer(t)
	fixture, err := os.ReadFile(filepath.Clean(holdRecordsFixturePath))
	if err != nil {
		t.Fatal(err)
	}
	extra := `{"v":1,"lane":"dormancy","site":"zagros","peer_id":"p-jane-2","peer_name":"peer-jane-doe-acme-zagros-20260601-phone","handle":null,"entered_at":"2026-09-05T10:23:00Z","hold_group":"ZAGROS-PROBATION-DEVELOPER","prior_groups":["ZAGROS-DEVELOPER"],"actor":"sweep"}` + "\n"
	path := filepath.Join(t.TempDir(), "hold-records.jsonl")
	if err := os.WriteFile(path, append(fixture, []byte(extra)...), 0o600); err != nil { //nolint:gosec // path is inside t.TempDir()
		t.Fatal(err)
	}
	s.Cfg.HoldRecordsPath = path
	peers := holdsFixturePeers()
	peers = append(peers, holdsPeerJSON("p-jane-2", "peer-jane-doe-acme-zagros-20260601-phone", "2026-09-04T10:00:00Z", "ZAGROS-PROBATION-DEVELOPER"))
	holdsFakeNetBird(t, s, peers)

	code, body := getHolds(t, s, "/holds", "ADMINS")
	if code != http.StatusOK {
		t.Fatalf("status = %d; %s", code, body)
	}
	a := strings.Index(body, `<code class="muted">p-jane-2</code>`)
	b := strings.Index(body, `<code class="muted">d0f1e2a3b4c5d6e7f8091a2b</code>`)
	if a < 0 || b < 0 {
		t.Fatal("jane's two rows not both rendered")
	}
	between := body[min(a, b):max(a, b)]
	if strings.Count(between, "<tbody") != 1 {
		t.Errorf("jane's two devices are not adjacent (%d tbody starts between them)", strings.Count(between, "<tbody"))
	}
	if strings.Count(body, `href="/users/id-jane"`) != 2 {
		t.Errorf("both rows must link the same member page, got %d links", strings.Count(body, `href="/users/id-jane"`))
	}
}

// The same peer renders identical Kind and State badge text on the Holds
// page and on the member page's Access section: one shared define.
func TestHoldsPage_BadgesMatchMemberPage(t *testing.T) {
	s := holdsFixtureServer(t)
	s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
	holdsFakeNetBird(t, s, holdsFixturePeers())

	_, holds := getHolds(t, s, "/holds", "ADMINS")
	fromHolds := badgeTexts(tbodyFor(t, holds, "0a1b2c3d4e5f60718293a4b5"))

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/users/id-john?idp=tenant", nil)
	req.Header.Set("X-Forwarded-Groups", "ADMINS")
	req.Header.Set("X-Forwarded-Email", "admin@example.test")
	req.Header.Set("X-Forwarded-User", "admin-id")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("member page = %d; %s", rec.Code, rec.Body.String())
	}
	fromMember := badgeTexts(rec.Body.String())
	if len(fromHolds) == 0 || strings.Join(fromHolds, ",") != strings.Join(fromMember, ",") {
		t.Fatalf("badge text differs: holds %v, member page %v", fromHolds, fromMember)
	}
	if !strings.Contains(rec.Body.String(), `href="/holds?idp=tenant"`) || !strings.Contains(rec.Body.String(), "See all holds") {
		t.Error("member page Access section lacks the See all holds link")
	}
}

// One unreadable informational timestamp must not hide the rest of the mesh
// or the affected peer's hold groups on either administrative surface.
func TestUnreadablePeerLastSeenKeepsHoldsAndMemberAccess(t *testing.T) {
	s := holdsFixtureServer(t)
	s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
	peers := holdsFixturePeers()
	peers[0]["last_seen"] = ""
	holdsFakeNetBird(t, s, peers)

	code, holds := getHolds(t, s, "/holds", "ADMINS")
	if code != http.StatusOK {
		t.Fatalf("holds status = %d; %s", code, holds)
	}
	if strings.Contains(holds, `id="holds-banner-mesh"`) {
		t.Fatalf("unreadable last_seen made the mesh unavailable: %s", holds)
	}
	jane := tbodyFor(t, holds, "d0f1e2a3b4c5d6e7f8091a2b")
	if !strings.Contains(jane, "last seen unreadable") || !strings.Contains(jane, "ZAGROS-DEVELOPER") {
		t.Fatalf("holds row does not preserve the peer with an unreadable note: %s", jane)
	}
	if !strings.Contains(holds, "p-kim-1") {
		t.Fatal("a bad timestamp hid another peer from the Holds page")
	}

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/users/id-jane?idp=tenant", nil)
	req.Header.Set("X-Forwarded-Groups", "ADMINS")
	req.Header.Set("X-Forwarded-Email", "admin@example.test")
	req.Header.Set("X-Forwarded-User", "admin-id")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("member status = %d; %s", rec.Code, rec.Body.String())
	}
	member := rec.Body.String()
	if strings.Contains(member, "mesh state unavailable") || !strings.Contains(member, "last seen unreadable") || !strings.Contains(member, "ZAGROS-PROBATION-DEVELOPER") {
		t.Fatalf("member Access section does not preserve the peer with an unreadable note: %s", member)
	}
}

// A duplicated record for a peer the mesh does not list is graded
// inconsistent; its Inactive cell still reads "—", not "never seen", which
// is reserved for a peer the mesh lists but has never seen.
func TestHoldsPage_InactiveCellNeverSeenOnlyForListedPeers(t *testing.T) {
	s := holdsFixtureServer(t)
	line := `{"v":1,"lane":"security","site":"zagros","peer_id":"p-gone","peer_name":"peer-john-roe-acme-zagros-20260415-desktop","handle":"john-roe","entered_at":"2026-09-01T10:23:00Z","hold_group":"ZAGROS-QUARANTINED","prior_groups":["ZAGROS-DEVELOPER"],"actor":"sweep","reason":"x"}` + "\n"
	path := filepath.Join(t.TempDir(), "hold-records.jsonl")
	if err := os.WriteFile(path, []byte(line+line), 0o600); err != nil {
		t.Fatal(err)
	}
	s.Cfg.HoldRecordsPath = path
	// A peer the mesh lists but has never seen carries no last_seen at all.
	never := holdsPeerJSON("p-never", "peer-kim-lee-blue-zagros-20260701-phone", "", "ZAGROS-QUARANTINED")
	delete(never, "last_seen")
	holdsFakeNetBird(t, s, []map[string]any{never})
	code, body := getHolds(t, s, "/holds", "ADMINS")
	if code != http.StatusOK {
		t.Fatalf("status = %d; %s", code, body)
	}
	gone := tbodyFor(t, body, "p-gone")
	if !strings.Contains(gone, "duplicate hold records") || strings.Contains(gone, "never seen") {
		t.Errorf("duplicated orphan must be inconsistent and read —, not never seen:\n%s", gone)
	}
	if never := tbodyFor(t, body, "p-never"); !strings.Contains(never, "never seen") {
		t.Errorf("a listed peer with no last-seen must read never seen:\n%s", never)
	}
}

// Empty and unavailable states are visibly different from each other.
func TestHoldsPage_EmptyAndUnavailableStates(t *testing.T) {
	empty := func(t *testing.T) *Server {
		s := holdsFixtureServer(t)
		path := filepath.Join(t.TempDir(), "hold-records.jsonl")
		if err := os.WriteFile(path, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		s.Cfg.HoldRecordsPath = path
		return s
	}
	noHolds := []map[string]any{holdsPeerJSON("p-free", "peer-kim-lee-blue-zagros-20260701-tablet", "2026-09-22T10:00:00Z", "ZAGROS-OBSERVER")}

	t.Run("nobody on hold", func(t *testing.T) {
		s := empty(t)
		holdsFakeNetBird(t, s, noHolds)
		_, body := getHolds(t, s, "/holds", "ADMINS")
		for _, want := range []string{`id="holds-empty">Nobody is on hold.`, "1 peer checked", "On hold <b>0</b>", "Dormancy <b>0</b>", "Security <b>0</b>", "Needs attention <b>0</b>"} {
			if !strings.Contains(body, want) {
				t.Errorf("missing %q", want)
			}
		}
		for _, banner := range []string{"holds-banner-records", "holds-banner-mesh", "holds-banner-roster", "holds-bad-lines"} {
			if strings.Contains(body, banner) {
				t.Errorf("empty state shows %s", banner)
			}
		}
	})

	t.Run("record file missing", func(t *testing.T) {
		s := holdsFixtureServer(t)
		s.Cfg.HoldRecordsPath = filepath.Join(t.TempDir(), "absent.jsonl")
		holdsFakeNetBird(t, s, holdsFixturePeers())
		_, body := getHolds(t, s, "/holds", "ADMINS")
		if !strings.Contains(body, `id="holds-banner-records"`) || !strings.Contains(body, "Hold records unavailable") {
			t.Error("records banner missing")
		}
		if strings.Contains(body, "Nobody is on hold") || strings.Contains(body, `id="holds-banner-mesh"`) {
			t.Error("records-missing state must not read as empty or as mesh-down")
		}
		// Mesh alone: the hold-group peers render, kind unknown, counted
		// under their own quick filter and never as security. All five
		// are kind unknown (kim's inconsistent phone included: it is one
		// Kind unknown AND one Needs attention).
		jane := tbodyFor(t, body, "d0f1e2a3b4c5d6e7f8091a2b")
		if got := badgeTexts(jane); strings.Join(got, ",") != "kind=kind unknown,state=held" {
			t.Errorf("mesh-alone badges = %v", got)
		}
		if !strings.Contains(body, `data-band="unknown" aria-pressed="false">Kind unknown <b>5</b>`) || !strings.Contains(body, "Security <b>0</b>") || !strings.Contains(body, "Needs attention <b>1</b>") {
			t.Error("kind-unknown rows must be counted as unknown, not security, with attention counted by state")
		}
	})

	t.Run("mesh unavailable", func(t *testing.T) {
		s := holdsFixtureServer(t)
		s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
		nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusBadGateway) }))
		t.Cleanup(nb.Close)
		s.Cfg.NotifyMgmtURL = nb.URL
		s.Cfg.EnrollWatchToken = holdsFakeNetBirdPAT
		_, body := getHolds(t, s, "/holds", "ADMINS")
		if !strings.Contains(body, `id="holds-banner-mesh"`) || !strings.Contains(body, "Mesh unavailable") || !strings.Contains(body, "mesh did not answer") {
			t.Error("mesh banner / checked-at wording missing")
		}
		if strings.Contains(body, `id="holds-banner-records"`) || strings.Contains(body, "Nobody is on hold") {
			t.Error("mesh-down state must not read as records-missing or empty")
		}
		if strings.Contains(body, `href="/users/`) {
			t.Error("with the mesh down nothing is attributed, so no member link")
		}
		// Records alone: kind, held since, prior groups; Inactive is "—".
		jane := tbodyFor(t, body, "d0f1e2a3b4c5d6e7f8091a2b")
		if got := badgeTexts(jane); strings.Join(got, ",") != "kind=dormancy,state=held" {
			t.Errorf("records-alone badges = %v", got)
		}
		if !strings.Contains(jane, "2026-09-01") || !strings.Contains(jane, "ZAGROS-P2P") || !strings.Contains(jane, `data-inactive=""`) {
			t.Errorf("records-alone row: %s", jane)
		}
		if strings.Contains(body, holdsFakeNetBirdPAT) {
			t.Fatal("token leaked")
		}
	})

	t.Run("roster partial", func(t *testing.T) {
		s := holdsFixtureServer(t)
		s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
		holdsFakeNetBird(t, s, holdsFixturePeers())
		s.Onboard = nil
		_, body := getHolds(t, s, "/holds", "ADMINS")
		// The roster is absent here, not partial: the banner must not call
		// it partial, and must still carry the cause.
		if !strings.Contains(body, `id="holds-banner-roster"`) || !strings.Contains(body, "Handle roster unavailable — onboarding store is not configured") || strings.Contains(body, "Handle roster partial") {
			t.Error("roster banner missing or misworded")
		}
		if strings.Contains(body, `href="/users/`) {
			t.Error("a partial roster must attribute nothing")
		}
		if strings.Count(body, "not attributed to a member") != 5 {
			t.Errorf("want every one of the 5 rows unattributed, got %d", strings.Count(body, "not attributed to a member"))
		}
	})

	t.Run("goatnet unset is unavailable, not empty", func(t *testing.T) {
		s := holdsFixtureServer(t)
		s.Cfg.NotifyGoatnet = ""
		s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
		calls := 0
		nb := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls++; w.WriteHeader(http.StatusOK) }))
		t.Cleanup(nb.Close)
		s.Cfg.NotifyMgmtURL = nb.URL
		s.Cfg.EnrollWatchToken = holdsFakeNetBirdPAT
		code, body := getHolds(t, s, "/holds", "ADMINS")
		if code != http.StatusOK || strings.Contains(body, "Nobody is on hold") {
			t.Fatalf("unset goatnet rendered the empty claim (code %d)", code)
		}
		if !strings.Contains(body, "NOTIFY_GOATNET is unset") || !strings.Contains(body, `id="holds-banner-records"`) || !strings.Contains(body, `id="holds-banner-mesh"`) {
			t.Error("unset goatnet must name the cause on both banners")
		}
		if !strings.Contains(body, `id="holds-empty">Neither source answered`) || strings.Contains(body, "the source that answered") {
			t.Error("with neither source answering, the empty line must not claim one did")
		}
		if calls != 0 {
			t.Error("no mesh call when there is no goatnet to grade")
		}
	})

	t.Run("bad lines counted on one line", func(t *testing.T) {
		s := holdsFixtureServer(t)
		s.Cfg.HoldRecordsPath = filepath.Clean("../../../lifecycle/fixtures/hold-records-bad-lines.jsonl")
		holdsFakeNetBird(t, s, holdsFixturePeers())
		_, body := getHolds(t, s, "/holds", "ADMINS")
		if !strings.Contains(body, `id="holds-bad-lines"`) || !strings.Contains(body, "bad hold-record line(s) skipped") || !strings.Contains(body, "validate-hold-records.py") {
			t.Error("bad-lines line missing")
		}
		if strings.Count(body, "bad hold-record line(s) skipped") != 1 {
			t.Error("bad lines must be reported once")
		}
	})
}

// Admin-gated like the other admin pages: a sponsor session is refused, the
// identity-plane selector is ignored, and the nav entry disappears in
// sponsor mode.
func TestHoldsPage_GatingAndIdP(t *testing.T) {
	s := holdsFixtureServer(t)
	s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
	holdsFakeNetBird(t, s, holdsFixturePeers())

	if code, _ := getHolds(t, s, "/holds", "PARTNER"); code != http.StatusForbidden {
		t.Errorf("non-admin session = %d, want 403", code)
	}
	// A second plane: a known selector is kept for the nav, an unknown one
	// is dropped (default plane) rather than refused; the page is the same.
	s.Cfg.IdPs["operators"] = config.IdP{Key: "operators", Label: "Operators"}
	s.Cfg.IdPOrder = append(s.Cfg.IdPOrder, "operators")
	for target, plane := range map[string]string{"/holds": "tenant", "/holds?idp=operators": "operators", "/holds?idp=bogus": "tenant", "/holds?idp=tenant": "tenant"} {
		code, body := getHolds(t, s, target, "ADMINS")
		if code != http.StatusOK || !strings.Contains(body, "Mesh holds — <code>zagros</code>") {
			t.Errorf("%s = %d; the idp selector must not change or refuse the page", target, code)
		}
		for _, want := range []string{`href="/?idp=` + plane + `"`, `href="/holds?idp=` + plane + `"`} {
			if !strings.Contains(body, want) {
				t.Errorf("%s: nav must keep plane %s (missing %s)", target, plane, want)
			}
		}
	}

	base, err := loadTemplates()
	if err != nil {
		t.Fatal(err)
	}
	tm, _ := base.Clone()
	data, _ := fs.ReadFile(web.FS, "templates/users_list.html")
	if _, err := tm.New("users_list").Parse(string(data)); err != nil {
		t.Fatal(err)
	}
	var b strings.Builder
	if err := tm.ExecuteTemplate(&b, "layout", &pageVars{SponsorMode: true}); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(b.String(), "Mesh holds") {
		t.Error("sponsor-mode nav shows Mesh holds")
	}
}

// An attributed handle with no member link says WHY, one wording per cause,
// so the Release action (goat#1044) can tell them apart: onboarded here but
// no login carries the email; onboarded on other goatnets only; no active
// onboarding record at all. With the directory down the banner speaks and
// the rows do not guess.
func TestHoldsPage_NoLoginReasons(t *testing.T) {
	newServer := func(t *testing.T) *Server {
		t.Helper()
		s := holdsFixtureServer(t)
		ctx := context.Background()
		// kay: onboarded here, but no login carries the proposal's email.
		p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "kay-roe", Email: "kay@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatal(err)
		}
		// lou: onboarded on two other goatnets, never here.
		for _, env := range []string{"zagros-prod", "vega"} {
			p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: env, Handle: "lou-ray", Email: "lou@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatal(err)
			}
		}
		// max: the only proposal was offboarded, so no active record anywhere.
		p = &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "max-fox", Email: "max@example.test", Status: onboardstore.ProposalOffboarded, ManifestJSON: "{}", Tier: "developer"}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatal(err)
		}
		// ops: filed through the machine door — a handle and no email — so
		// there is an active record HERE and nothing to link (goat#210).
		p = &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: "ops-key", Email: "", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer"}
		if err := s.Onboard.CreateProposal(ctx, p); err != nil {
			t.Fatal(err)
		}
		// ned: onboarded here, then re-onboarded on another goatnet with the SAME
		// email; the member page derives the handle from the newest
		// proposal for the email, which is now foreign, so no link.
		for i, env := range []string{"zagros", "zagros-prod"} {
			p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: env, Handle: "ned-hue", Email: "ned@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer", CreatedAt: time.Now().Unix() + int64(i*100)}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatal(err)
			}
		}
		// vic: onboarded here with an email, then the SAME email filed
		// again here through the machine door (no handle). The newest
		// proposal derives NO handle, which is not "another handle".
		for i, h := range []string{"vic-doe", ""} {
			p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: h, Email: "vic@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer", CreatedAt: time.Now().Unix() + int64(i*100)}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatal(err)
			}
		}
		// tia: two active proposals here for ONE email in the SAME second
		// under two handles. Whichever the store's own lookup returns is
		// the handle the member page derives; the holds page must agree.
		for _, h := range []string{"tia-one", "tia-two"} {
			p := &onboardstore.Proposal{Sponsor: "sponsor-1", Env: "zagros", Handle: h, Email: "tia@example.test", Status: onboardstore.ProposalOnboarded, ManifestJSON: "{}", Tier: "developer", CreatedAt: 1_800_000_000}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatal(err)
			}
		}
		s.Cfg.HoldRecordsPath = filepath.Clean(holdRecordsFixturePath)
		holdsFakeNetBird(t, s, []map[string]any{
			holdsPeerJSON("p-vic", "peer-vic-doe-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-tia1", "peer-tia-one-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-tia2", "peer-tia-two-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-kay", "peer-kay-roe-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-lou", "peer-lou-ray-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-max", "peer-max-fox-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-ops", "peer-ops-key-blue-zagros-20260701-box", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
			holdsPeerJSON("p-ned", "peer-ned-hue-blue-zagros-20260701-laptop", "2026-09-20T10:00:00Z", "ZAGROS-QUARANTINED"),
		})
		return s
	}

	t.Run("directory answered", func(t *testing.T) {
		s := newServer(t)
		code, body := getHolds(t, s, "/holds", "ADMINS")
		if code != http.StatusOK {
			t.Fatalf("status = %d; %s", code, body)
		}
		for peer, want := range map[string]string{
			"p-kay": "onboarded on this goatnet, no login carries its email",
			"p-lou": "onboarded on vega, zagros-prod only, not this goatnet",
			"p-max": "no active onboarding record",
			"p-ops": "onboarded on this goatnet without an email (machine door), no login to link",
			"p-ned": "onboarded on this goatnet, but the newest onboarding of its login is on zagros-prod, so the member page would not derive this handle",
			"p-vic": "onboarded on this goatnet, but the newest onboarding of its login carries no handle (machine door), so the member page would not derive this handle",
		} {
			row := tbodyFor(t, body, peer)
			if !strings.Contains(row, want) || strings.Contains(row, `href="/users/`) {
				t.Errorf("%s: want reason %q and no link:\n%s", peer, want, row)
			}
		}
		if row := tbodyFor(t, body, "p-vic"); strings.Contains(row, "derives another handle") {
			t.Errorf("p-vic: a handle-less proposal derives no handle, not another one:\n%s", row)
		}
		// Equal created_at: the page's pick must be the store's pick, so a
		// link from this page never lands on a member page that disowns
		// the handle.
		newest, ok, err := s.Onboard.ActiveProposalForEmail(context.Background(), "tia@example.test")
		if err != nil || !ok {
			t.Fatalf("ActiveProposalForEmail: ok=%v err=%v", ok, err)
		}
		linked, disowned := "p-tia1", "p-tia2"
		if newest.Handle == "tia-two" {
			linked, disowned = disowned, linked
		}
		if row := tbodyFor(t, body, linked); !strings.Contains(row, `href="/users/`) {
			t.Errorf("%s (store derives %s): want a member link:\n%s", linked, newest.Handle, row)
		}
		if row := tbodyFor(t, body, disowned); strings.Contains(row, `href="/users/`) || !strings.Contains(row, "its login now derives another handle") {
			t.Errorf("%s (store derives %s): want no link and the other-handle reason:\n%s", disowned, newest.Handle, row)
		}
		if strings.Contains(body, "no login on this goatnet") {
			t.Error("the one-size wording must be gone")
		}
	})

	t.Run("directory down", func(t *testing.T) {
		s := newServer(t)
		s.Kratos[tenantIdPKey].HTTP = &http.Client{Transport: holdsFailingRoundTripper{}}
		_, body := getHolds(t, s, "/holds", "ADMINS")
		if !strings.Contains(body, `id="holds-banner-directory"`) {
			t.Fatal("directory banner missing")
		}
		for _, peer := range []string{"p-kay", "p-lou", "p-max", "p-ops", "p-ned"} {
			row := tbodyFor(t, body, peer)
			if !strings.Contains(row, "directory unavailable, no link") || strings.Contains(row, "onboarded on") || strings.Contains(row, "no active onboarding record") {
				t.Errorf("%s: with the directory down the row must not guess a cause:\n%s", peer, row)
			}
		}
	})
}

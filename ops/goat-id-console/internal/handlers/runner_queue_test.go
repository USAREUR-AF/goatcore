package handlers

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The runner band: a row the onboard-runner has left waiting must be
// visible on the queue page itself, with a verdict the store can prove —
// "the runner claimed other work after this was queued" is the one fact that
// separates a runner that is down from a build that does not read the queue.

func unixPtr(v int64) *int64 { return &v }

func TestRunnerQueue_NothingLateRendersNoBand(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	fresh := now.Add(-time.Minute).Unix()
	all := []onboardstore.Proposal{
		{ID: "a", Status: onboardstore.ProposalApproved, DecidedAt: unixPtr(fresh)},
		{ID: "b", Status: onboardstore.ProposalOffboardRequested, OffboardRequestedAt: fresh},
		{ID: "c", Status: onboardstore.ProposalOnboarded, DecidedAt: unixPtr(now.Add(-48 * time.Hour).Unix())},
	}
	if v := runnerQueue(all, 0, now, 0); v != nil {
		t.Fatalf("nothing waited past the window, want no band, got %+v", v)
	}
}

// A runner that claimed work AFTER the cut was queued was alive and polling
// and left the cut behind: the band says so, and says every late row is a cut.
func TestRunnerQueue_BlindWhenRunnerClaimedAfterTheQueue(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	queued := now.Add(-7 * time.Hour).Unix()
	lastClaim := now.Add(-20 * time.Minute).Unix()
	all := []onboardstore.Proposal{
		{ID: "p1", Handle: "rtexample5768", Env: "zagros", Status: onboardstore.ProposalOffboardRequested, OffboardRequestedAt: queued},
	}
	v := runnerQueue(all, lastClaim, now, 0)
	if v == nil {
		t.Fatal("a cut queued 7h ago is late; want the band")
	}
	if !v.Blind {
		t.Errorf("runner claimed 20m ago, after the 7h-old queue: want Blind=true")
	}
	if !v.AllCuts {
		t.Errorf("the only late row is a cut: want AllCuts=true")
	}
	if len(v.Rows) != 1 || v.Rows[0].Handle != "rtexample5768" || v.Rows[0].Waited != "7h" || v.Rows[0].Stage != "Cut queued" || !v.Rows[0].Cut {
		t.Errorf("row = %+v, want the queued cut with its wait and tab label", v.Rows)
	}
	if v.LastClaim != "20m ago" || v.Oldest != "7h" {
		t.Errorf("LastClaim=%q Oldest=%q, want 20m ago / 7h", v.LastClaim, v.Oldest)
	}
}

// No claim since the row was queued: the store cannot tell a dead runner from a
// blind one, and the band must not pretend otherwise.
func TestRunnerQueue_NotBlindWhenNothingClaimedSince(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	queued := now.Add(-time.Hour).Unix()
	all := []onboardstore.Proposal{
		{ID: "p1", Handle: "x", Status: onboardstore.ProposalApproved, DecidedAt: unixPtr(queued)},
		{ID: "p2", Handle: "y", Status: onboardstore.ProposalOffboardRequested, OffboardRequestedAt: queued},
	}
	v := runnerQueue(all, now.Add(-2*time.Hour).Unix(), now, 0)
	if v == nil || v.Blind {
		t.Fatalf("last claim predates the queue: want a band with Blind=false, got %+v", v)
	}
	if v.AllCuts {
		t.Errorf("an approved mint is also late: want AllCuts=false")
	}
	if v2 := runnerQueue(all, 0, now, 0); v2 == nil || v2.Blind || v2.LastClaim != "never" {
		t.Errorf("no claim ever: want Blind=false and LastClaim=never, got %+v", v2)
	}
}

// A row is judged from the moment it was handed to the runner, never from its
// creation — and a row with no such clock is left out rather than guessed at.
func TestRunnerQueue_KeysOnHandoverNotCreation(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	old := now.Add(-30 * 24 * time.Hour).Unix()
	all := []onboardstore.Proposal{
		{ID: "a", Status: onboardstore.ProposalApproved, CreatedAt: old, DecidedAt: unixPtr(now.Add(-time.Minute).Unix())},
		{ID: "b", Status: onboardstore.ProposalApproved, CreatedAt: old}, // no decision clock
		{ID: "c", Status: onboardstore.ProposalOffboardRequested, CreatedAt: old},
	}
	if v := runnerQueue(all, 0, now, 0); v != nil {
		t.Fatalf("a month-old row decided a minute ago is not late, and clockless rows are skipped; got %+v", v)
	}
}

// The admin's queue page carries the band for a stuck cut, names the row, and
// gives the verdict and the fix; the detail panel offers the settle door.
func TestRunnerBand_RendersOnTheQueuePageForAStuckCut(t *testing.T) {
	s := newOnboardTestServer(t)
	s.runnerLateAfter = time.Nanosecond // a freshly queued row is already "late"
	h := s.Routes()
	pid := offboardRowTo(t, s, h, "person-115@example.org", onboardstore.ProposalOffboardRequested)

	body := adminGet(t, h, "/onboarding?status="+onboardstore.ProposalOffboardRequested)
	for _, want := range []string{
		"ob-runner",
		"waited for the onboard-runner",
		"/onboarding?status=offboard-requested&id=" + pid,
		"docker logs onboard-runner",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("queue page missing %q", want)
		}
	}
	// The row was claimed once on its way to onboarded (driveTo), BEFORE the
	// cut was queued, so the store shows no claim since: not blind.
	if strings.Contains(body, "not reading the offboard queue") {
		t.Errorf("no runner claim since the cut was queued: the band must not call the runner blind")
	}
	// The row's own badge clock travels with the row.
	if !strings.Contains(body, `data-queued="`) || strings.Contains(body, `data-queued="0" data-status="offboard-requested"`) {
		t.Errorf("queued cut row must carry its offboard_requested_at for the runner? badge")
	}

	detail := adminGet(t, h, "/onboarding?status="+onboardstore.ProposalOffboardRequested+"&id="+pid)
	if !strings.Contains(detail, `action="/onboarding/`+pid+`/offboard"`) || !strings.Contains(detail, "settle this record") {
		t.Errorf("detail panel of a queued cut must offer the settle door (Mark offboarded)")
	}
}

// The band calls the runner blind when it claimed other work after the cut
// was queued: a live runner whose build has no offboard leg.
func TestRunnerBand_CallsTheRunnerBlindWhenItClaimedOtherWorkSince(t *testing.T) {
	s := newOnboardTestServer(t)
	s.runnerLateAfter = time.Nanosecond
	h := s.Routes()
	pid := offboardRowTo(t, s, h, "person-115@example.org", onboardstore.ProposalOffboardRequested)
	// Another person's mint is claimed afterwards: the runner is alive. The
	// store keeps whole seconds and the verdict is strictly "claimed AFTER
	// queued", so the claim has to land in a later second than the click.
	time.Sleep(1100 * time.Millisecond)
	onboardedProposalInState(t, s, h, "person-73@example.org", onboardstore.ProposalOnboarded)

	body := adminGet(t, h, "/onboarding?status="+onboardstore.ProposalOffboardRequested)
	for _, want := range []string{
		"not reading the offboard queue",
		"built before 2026-09-26",
		"deploy a current onboard-runner",
		"Mark offboarded",
		pid,
	} {
		if !strings.Contains(body, want) {
			t.Errorf("blind verdict missing %q", want)
		}
	}
}

// Without a band payload the page carries no band markup at all — the handler
// sets RunnerQueue for admin sessions only, so a sponsor's page is this case.
func TestRunnerBand_AbsentWhenNoPayload(t *testing.T) {
	out := renderLayoutFor(t, "onboarding", &pageVars{SponsorMode: true, OnboardEnabled: true})
	if strings.Contains(out, "ob-runner") {
		t.Errorf("page without a RunnerQueue payload rendered the runner band")
	}
}

// The settle door: a never-claimed cut is marked offboarded by an admin
// who cut the access by hand — with a note, behind the login-plane precheck.
func TestSettleQueuedCut_RequiresNoteAndInactiveLogin(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	email := "person-113@example.org"
	pid := offboardRowTo(t, s, h, email, onboardstore.ProposalOffboardRequested)

	// Login still active: refused, row does not move.
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "active")}
	rec := postBrowserOffboardNote(t, h, s, pid, "cut by hand: goat admin onboard --revoke")
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, flashOffboardLive) {
		t.Fatalf("active login: want refusal %q, got %d %q", flashOffboardLive, rec.Code, loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboardRequested {
		t.Fatalf("refused settle moved the row to %q", got)
	}

	// Login gone, but no note: refused — the note is the record of how.
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}
	rec = postBrowserOffboardNote(t, h, s, pid, "")
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, flashSettleNoteRequired) {
		t.Fatalf("blank note: want refusal %q, got %d %q", flashSettleNoteRequired, rec.Code, loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboardRequested {
		t.Fatalf("blank-note settle moved the row to %q", got)
	}

	// Login gone and a note: settled. The email is free for a fresh invite and
	// the runner's claim now conflicts — the cut is nobody else's to run.
	postBrowserOffboardNote(t, h, s, pid, "cut by hand: goat admin onboard --revoke")
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboarded {
		t.Fatalf("want offboarded after a verified hand cut, got %q", got)
	}
	if _, found, _ := s.Onboard.ActiveProposalForEmail(t.Context(), email); found {
		t.Errorf("settled row still blocks the email for a fresh invite")
	}
	if c := postMachine(t, h, "/api/onboard/proposals/"+pid+"/offboard-claim", nil); c.code != http.StatusConflict {
		t.Errorf("runner claim on a settled row: want 409, got %d %s", c.code, c.body)
	}
}

// A cut the runner HAS claimed is the runner's to finish: the settle door
// still refuses `offboarding`.
func TestSettleQueuedCut_RefusesAClaimedCut(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	email := "person-26@example.org"
	pid := offboardRowTo(t, s, h, email, onboardstore.ProposalOffboarding)
	s.Kratos = map[string]*kratos.Client{"tenant": fakeKratosIdentityList(t, email, "inactive")}
	rec := postBrowserOffboardNote(t, h, s, pid, "cut by hand")
	if loc := rec.Header().Get("Location"); !strings.Contains(loc, flashOffboardState) {
		t.Fatalf("claimed cut: want refusal %q, got %d %q", flashOffboardState, rec.Code, loc)
	}
	if got := statusOf(t, s, pid); got != onboardstore.ProposalOffboarding {
		t.Fatalf("refused settle moved a claimed cut to %q", got)
	}
}

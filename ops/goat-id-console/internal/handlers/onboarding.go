package handlers

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"html/template"
	"log/slog"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/meshstate"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// onboarding.go — the HUMAN (admin) surface of the three-stage delegated candidate
// onboarding (ADR 1062/1071), ported from the data-fabric portal into the identity
// plane per docs/design/onboarding-extraction-plan.md (slice 2b).
//
// oauth2-proxy ADMINS gated. An admin is OPERATOR-EQUIVALENT → ADR 1071
// sponsorship-authorized by definition with NO tier cap (no portal call). Sponsor
// and decided_by are the console's own oauth2-proxy identity — native attribution,
// no X-Goat-Operator-Actor header. Approve records `approved` only; the runner
// (machine realm) claims + mints — the YAML/command response is dropped.

// onboardStages are the review buckets shown as filter tabs. "invited" is the
// pre-submission funnel stage (minted links awaiting the candidate — an invite
// row, not a proposal status); "proposed" is the sponsor gate;
// "sponsor_approved" is the admin gate (the default).
const onboardStageInvited = "invited"

var onboardStageDefs = []struct{ Key, Label string }{
	{onboardStageInvited, "Invited"},
	{"proposed", "Awaiting sponsor"},
	{"sponsor_approved", "Awaiting admin"},
	{"approved", "Approved"},
	{"onboarding", "Provisioning"},
	{"onboarded", "Onboarded"},
	// The real-cut states (goat#372), between Onboarded and Offboarded
	// because that is the journey: queued → cutting → landed or failed.
	// Cut failed is the loud one — the person still has access — and its
	// panel is where the re-queue lives.
	{"offboard-requested", "Cut queued"},
	{"offboarding", "Cutting"},
	{"offboard-failed", "Cut failed"},
	{"offboarded", "Offboarded"},
	// `enrolled` is OBSERVED, not decided: the enrollment watcher saw the person
	// consume their mesh setup key. It sits after Onboarded because that is the
	// question an operator asks next — the package was sent, did it land?
	{"enrolled", "Enrolled"},
	// `archived` is COLD STORAGE, not a failure: credentials were sent, the key
	// was never redeemed, and the ADR 1136 purge lane revoked it. The row keeps
	// both gate approvals so Restore re-mints and re-sends in one action. It
	// sits after Enrolled because that is the pair an operator reads together —
	// who joined, and who never did and has since been shelved.
	{"archived", "Archived"},
	{"failed", "Failed"},
	{"rejected", "Rejected"},
}

// Flash codes for the onboarding queue. The redirect carries the code; the
// operator-facing wording lives here, server-side, so it cannot be authored
// by whoever composed the URL.
const (
	flashOffboardAlready = "offboard_already"
	flashOffboardState   = "offboard_state"
	flashOffboardFailed  = "offboard_failed"
	// Refusals from the browser button's assertion check (offboardPrecheck).
	flashOffboardLive       = "offboard_live"
	flashOffboardUnverified = "offboard_unverified"
	// Refusal of an archived-row offboard that carries no note: the
	// transition is one-way, so the reason must be written down.
	flashOffboardNoteRequired = "offboard_note_required"
	// flashSettleNoteRequired: a never-claimed queued cut settled by hand
	// must say HOW the access was cut.
	flashSettleNoteRequired = "offboard_settle_note_required"
	// Restore from the Archived card (#1037). Both messages name facts read
	// from the selected row at render time (its email, its current stage),
	// never from the URL, so the code stays a code: restoreFlash below
	// builds them once the row is loaded, and they render nothing without
	// a selected row.
	flashRestored     = "restored"
	flashRestoreState = "restore_state"
	// Handset-key reissue (ADR 1175): queued for the runner, or refused.
	flashHandsetQueued  = "handset_queued"
	flashHandsetPending = "handset_pending"
	flashHandsetState   = "handset_state"
	flashHandsetFailed  = "handset_failed"
	// Device-key reissue: an expired or used-up setup key replaced.
	flashDeviceKeyQueued  = "device_key_queued"
	flashDeviceKeyPending = "device_key_pending"
	flashDeviceKeyState   = "device_key_state"
	flashDeviceKeyFailed  = "device_key_failed"
	flashDeviceKeyUses    = "device_key_uses"
	// The real-cut button (goat#372): queue outcomes, not bookkeeping marks.
	flashOffboardCutQueued = "offboard_cut_queued"
	flashOffboardCutState  = "offboard_cut_state"
	flashOffboardCutFailed = "offboard_cut_failed"
	// GitHub username set or cleared after onboarding (github_retro.go).
	flashGitHubSet       = "github_set"
	flashGitHubCleared   = "github_cleared"
	flashGitHubRequested = "github_requested"
)

var onboardFlashes = map[string]flash{
	flashOffboardAlready:      {Level: "info", Message: "That proposal was already marked offboarded. The queue below shows its current state."},
	flashOffboardState:        {Level: "error", Message: "Only a proposal that is " + onboardstore.OffboardableProse() + " can be marked offboarded. Nothing was changed — the queue below shows its current state."},
	flashOffboardFailed:       {Level: "error", Message: "Could not record the offboard. Nothing was changed; try again, and check the console logs if it keeps failing."},
	flashOffboardLive:         {Level: "error", Message: "Not recorded: this person's login is still ACTIVE, so their access has not been cut. Marking the row would release their email for a fresh enrolment while they still hold credentials. Run `goat admin onboard <handle> --env <env> --revoke` — it cuts the mesh peer, the login and the slot AND records this row — then reload."},
	flashHandsetQueued:        {Level: "success", Message: "Handset key reissue queued. The onboarding runner revokes the person's current handset keys, mints a fresh one, and re-sends the welcome package with it — usually within a minute. The phone that had the old key is off the goatnet once the runner has run."},
	flashHandsetPending:       {Level: "info", Message: "A handset key reissue is already queued or in progress for this person. Nothing new was queued."},
	flashHandsetState:         {Level: "error", Message: "Only an onboarded person can have a handset key reissued. Nothing was changed."},
	flashHandsetFailed:        {Level: "error", Message: "Could not queue the handset key reissue. Nothing was changed; try again, and check the console logs if it keeps failing."},
	flashDeviceKeyQueued:      {Level: "success", Message: "Device key reissue queued. The onboarding runner revokes any of the person's device keys still live, mints a fresh one with the reuse count you chose, and re-sends the welcome package with it — usually within a minute. Devices already on the goatnet stay on."},
	flashDeviceKeyPending:     {Level: "info", Message: "A device key reissue is already queued or in progress for this person. Nothing new was queued."},
	flashDeviceKeyState:       {Level: "error", Message: "Only an onboarded or enrolled person can have a device key reissued. Nothing was changed."},
	flashDeviceKeyFailed:      {Level: "error", Message: "Could not queue the device key reissue. Nothing was changed; try again, and check the console logs if it keeps failing."},
	flashDeviceKeyUses:        {Level: "error", Message: "The number of uses must be a whole number from 1 to 100. Nothing was queued."},
	flashOffboardUnverified:   {Level: "error", Message: "Not recorded: the login plane could not be reached, so whether access was actually cut is unknown. Nothing was changed. Try again, or run `goat admin onboard <handle> --env <env> --revoke`, which performs the cut and records it."},
	flashOffboardNoteRequired: {Level: "error", Message: offboardNoteRequiredMsg},
	flashSettleNoteRequired:   {Level: "error", Message: settleNoteRequiredMsg},
	flashOffboardCutQueued:    {Level: "info", Message: "The cut is queued. The runner will revoke this person's access (mesh peer, login, slot, portal keys); this row shows offboarded when it lands, or the failure reason if it does not."},
	flashOffboardCutState:     {Level: "error", Message: "Nothing was queued: the row is not in an offboardable state, or a cut is already queued or running. The queue below shows its current state."},
	flashOffboardCutFailed:    {Level: "error", Message: "Could not queue the cut. Nothing was changed; try again, and check the console logs if it keeps failing."},
	flashGitHubSet:            {Level: "success", Message: "GitHub username saved and the repo-access grant re-run. The Repo access panel below shows what GitHub was asked to grant; the person accepts each invitation from GitHub."},
	flashGitHubCleared:        {Level: "info", Message: "GitHub username removed. Repositories the person was already added to are not revoked by this; remove them on GitHub or through offboarding."},
	flashGitHubRequested:      {Level: "success", Message: "Request emailed. It links the person to /github, where they add their username signed in as themselves; saving it re-runs the repo-access grant. The request is in the audit log as onboard_github_requested."},
}

// offboardNoteRequiredMsg is the one sentence both realms return when an
// archived row is offboarded without a note: the browser flash and the
// machine realm's 400 body carry it verbatim.
const offboardNoteRequiredMsg = "Not recorded: this archived record was not changed. Offboarding an archived record is one-way — it cannot be restored afterwards — so a note saying why it is being closed for good is required. Add a note and try again."

// settleNoteRequiredMsg is its counterpart for a queued cut the runner never
// ran: marking it offboarded asserts the access was cut by hand, and
// the note is the only record of how.
const settleNoteRequiredMsg = "Not recorded: this queued cut was not changed. Marking it offboarded asserts you cut the access yourself, and the note is the only record of how — say what you ran (for example `goat admin onboard <handle> --env <env> --revoke`) and try again."

// proposalToView flattens a store proposal into the template shape.
func proposalToView(p onboardstore.Proposal) proposalView {
	decidedBy := ""
	if p.DecidedBy != nil {
		decidedBy = *p.DecidedBy
	}
	decidedAt := ""
	decidedAtUnix := int64(0)
	if p.DecidedAt != nil {
		decidedAt = time.Unix(*p.DecidedAt, 0).UTC().Format(time.RFC3339)
		decidedAtUnix = *p.DecidedAt
	}
	restoredAt, restoredBy := "", p.RestoredBy
	if p.RestoredAt != 0 {
		restoredAt = time.Unix(p.RestoredAt, 0).UTC().Format(time.RFC3339)
		// The timestamp is the fact that a restore happened; the actor can be
		// empty (no forwarded email under auth bypass). Never let an empty
		// name hide the mark and make the row read as a fresh approval.
		if restoredBy == "" {
			restoredBy = "an unnamed operator"
		}
	}
	offboardRequestedAt := ""
	if p.OffboardRequestedAt != 0 {
		offboardRequestedAt = time.Unix(p.OffboardRequestedAt, 0).UTC().Format(time.RFC3339)
	}
	return proposalView{
		RestoredBy:              restoredBy,
		RestoredAt:              restoredAt,
		ID:                      p.ID,
		Handle:                  p.Handle,
		Email:                   p.Email,
		Env:                     p.Env,
		Status:                  p.Status,
		Escalated:               p.Escalated,
		Petitions:               p.Petitions,
		DecidedBy:               decidedBy,
		DecidedAt:               decidedAt,
		DecidedAtUnix:           decidedAtUnix,
		Note:                    p.Note,
		OffboardRequestedBy:     p.OffboardRequestedBy,
		OffboardRequestedAt:     offboardRequestedAt,
		OffboardRequestedAtUnix: p.OffboardRequestedAt,
		Created:                 p.CreatedAt,
	}
}

// inviteToView flattens a store invite into the template shape. Lifecycle
// fields are always rendered; access fields are withheld when the pre-bound
// metadata is unreadable. The display state folds TTL in: ListInvites does not
// lazy-expire, so an 'invited' row past its expires_at must still read
// "expired" here. archiveAfter is the configured dead-invite archive window;
// 0 means "no automatic archiving".
func inviteToView(inv onboardstore.Invite, archiveAfter time.Duration) onboardInviteView {
	pb, perr := inv.Prebound()
	state := ""
	switch inv.Status {
	case onboardstore.InviteInvited:
		state = "open"
		if inv.ExpiresAt != 0 && time.Now().Unix() >= inv.ExpiresAt {
			state = "expired"
		}
	case onboardstore.InviteSubmitted:
		state = "submitted"
	case onboardstore.InviteExpired:
		state = "expired"
	case onboardstore.InviteRevoked:
		state = "revoked"
	case onboardstore.InviteArchived:
		state = "archived"
	}
	sponsor := inv.SponsorName
	if sponsor == "" {
		sponsor = inv.Sponsor
	}
	created, expires := time.Unix(inv.CreatedAt, 0), time.Unix(inv.ExpiresAt, 0)
	v := onboardInviteView{
		ID: inv.ID, Sponsor: sponsor, Env: inv.Env,
		State: state, Note: inv.Note,
		Created:    created.UTC().Format(time.RFC3339),
		CreatedAgo: humanSince(created),
		Expires:    humanSince(expires),
		ExpiresAt:  expires.UTC().Format(time.RFC3339),
		TTL:        humanTTL(expires.Sub(created)),

		CandidateEmail: inv.CandidateEmail,
		SentVia:        inv.SentVia,
	}
	if perr != nil {
		v.MetadataError = "access metadata unreadable — mint a fresh invite to re-nominate"
		if state == "open" {
			v.MetadataError = "access metadata unreadable — revoke and re-invite"
		}
		// Access fields stay zero and Escalated stays false. The template
		// hides them, because a zero SponsorCap reads as "unrestricted".
	} else {
		v.Tier, v.Affiliation, v.Sector, v.Site = pb.Tier, pb.Affiliation, pb.Sector, pb.Site
		v.MachineQuota, v.Collab, v.SponsorCap = pb.MachineQuota, pb.Collab, pb.SponsorMaxTier
		v.Escalated = isEscalatedTier(pb.Tier)
	}
	if inv.ExpiresAt == 0 {
		v.ExpiresAt, v.TTL = "—", "—"
	}
	if inv.SentAt != nil && *inv.SentAt != 0 {
		sent := time.Unix(*inv.SentAt, 0)
		v.Sent, v.SentAgo = sent.UTC().Format(time.RFC3339), humanSince(sent)
	}
	if inv.ConsumedAt != nil && *inv.ConsumedAt != 0 {
		v.Consumed = time.Unix(*inv.ConsumedAt, 0).UTC().Format(time.RFC3339)
	}
	if inv.ProposalID != nil {
		v.ProposalID = *inv.ProposalID
	}
	// Lifecycle: when this invite died and when it archives.
	// The death anchor is StaleSince's answer so the UI, the sweep and the
	// CLI share one definition of "dead since when". For revoked rows we only
	// render a death moment when the revocation time was actually recorded;
	// legacy rows that predate revoked_at fall back to the TTL for archiving
	// but do not claim a future revocation.
	if state == "expired" || state == "revoked" {
		if at, ok := onboardstore.StaleSince(inv); ok {
			dead := time.Unix(at, 0).UTC()
			if state == "expired" || (state == "revoked" && inv.RevokedAt != nil && *inv.RevokedAt > 0) {
				v.DeadAt = dead.Format(time.RFC3339)
				v.DeadAtShort = dead.Format("2006-01-02")
				v.DeadAgo = humanSince(dead)
			}
			if archiveAfter > 0 {
				archiveAt := dead.Add(archiveAfter).UTC()
				v.ArchivesAt = archiveAt.Format(time.RFC3339)
				v.ArchivesAtShort = archiveAt.Format("2006-01-02")
				if d := time.Until(archiveAt); d <= 0 {
					v.ArchivesIn = ""
					v.ArchivesDueNextSweep = true
				} else {
					v.ArchivesIn = twoUnit(d)
					v.ArchivesDueNextSweep = false
				}
			}
		}
	}
	if inv.ArchivedAt != nil && *inv.ArchivedAt != 0 {
		archived := time.Unix(*inv.ArchivedAt, 0).UTC()
		v.ArchivedAt = archived.Format(time.RFC3339)
		v.ArchivedAgo = humanSince(archived)
	}
	return v
}

// humanSince renders how far a moment is from now, TWO units deep ("in 1d 23h",
// "3h 12m ago"). The console's one-unit humanizer truncates, so a link with
// 47h left reads "in 1d" — a whole day of runway the reader never sees. On an
// invite the remaining time is the decision (resend now, or leave it), so it is
// stated at the precision the decision needs.
func humanSince(t time.Time) string {
	if t.IsZero() || t.Unix() == 0 {
		return "—"
	}
	d := time.Until(t)
	if d >= 0 {
		return "in " + twoUnit(d)
	}
	return twoUnit(-d) + " ago"
}

// twoUnit renders a duration as at most two descending units.
func twoUnit(d time.Duration) string {
	switch {
	case d < time.Minute:
		return strconv.Itoa(int(d.Seconds())) + "s"
	case d < time.Hour:
		return strconv.Itoa(int(d.Minutes())) + "m"
	case d < 24*time.Hour:
		h := int(d.Hours())
		if m := int(d.Minutes()) % 60; m != 0 {
			return strconv.Itoa(h) + "h " + strconv.Itoa(m) + "m"
		}
		return strconv.Itoa(h) + "h"
	default:
		days := int(d.Hours()) / 24
		if h := int(d.Hours()) % 24; h != 0 {
			return strconv.Itoa(days) + "d " + strconv.Itoa(h) + "h"
		}
		return strconv.Itoa(days) + "d"
	}
}

// humanTTL renders the lifetime a sponsor chose for an invite (created →
// expires) as the sponsor picked it: whole days read as days, anything else as
// hours. This is the invite's total budget, not what is left — the remaining
// time is Expires.
func humanTTL(d time.Duration) string {
	// Rounded first: created_at is stamped a beat after expires_at is computed,
	// so a 48h invite measures 47h59m and would otherwise read "47h".
	if d >= time.Hour {
		d = d.Round(time.Hour)
	} else {
		d = d.Round(time.Minute)
	}
	switch {
	case d <= 0:
		return "—"
	case d < time.Hour:
		return strconv.Itoa(int(d.Minutes())) + "m"
	case d >= 24*time.Hour && int(d.Hours())%24 == 0:
		return strconv.Itoa(int(d.Hours())/24) + "d"
	default:
		return strconv.Itoa(int(d.Hours())) + "h"
	}
}

// humanArchiveWindow renders the dead-invite archive window for copy: whole
// days read as "N days"; otherwise hours. It is DISPLAY-ONLY and never
// drives behavior — the sweep reads the duration itself.
func humanArchiveWindow(d time.Duration) string {
	if d <= 0 {
		return ""
	}
	if d%(24*time.Hour) == 0 {
		days := int(d / (24 * time.Hour))
		if days == 1 {
			return "1 day"
		}
		return fmt.Sprintf("%d days", days)
	}
	if d%time.Hour == 0 {
		hours := int(d / time.Hour)
		if hours == 1 {
			return "1 hour"
		}
		return fmt.Sprintf("%d hours", hours)
	}
	// Fallback for a window that is not a whole number of hours (rare but
	// legal): the same two-unit form the rest of the page uses ("1h 30m"),
	// not Go's "1h30m0s".
	return twoUnit(d)
}

// splitInviteViews returns the Invited-tab rows in two lists — the WORKING list
// (open first, then recently dead, each newest-first) and the ARCHIVE — plus the
// count of OPEN invites for the tab badge.
//
// The split is the whole point of archiving. A revoked or expired link is dead
// the moment it dies, but it stays on the working list for a week because an
// operator who revoked something on Friday still wants to see it on Monday.
// After that it is noise: the sweep files it, and it reads here as a folded
// section rather than padding the list an operator scans for live work.
//
// Submitted invites appear in neither: they became proposals and live on in the
// later stages. Sponsor-scoped exactly like the proposal lists (ADR 1072), so a
// sponsor's archive holds only their own invitees.
//
// The handler makes one ListInvites pull and shares it with the tab badge and
// cross-stage find index (#1306). Malformed rows are logged and kept visible.
func splitInviteViews(ctx context.Context, invs []onboardstore.Invite, sponsorKeys map[string]struct{}, archiveAfter time.Duration) (working, archived []onboardInviteView, open int) {
	live := make([]onboardInviteView, 0, len(invs))
	dead := make([]onboardInviteView, 0, len(invs))
	archived = make([]onboardInviteView, 0, len(invs))
	for _, inv := range invs {
		// A submitted invite lives on as a proposal in the later stages; it is
		// never listed here, which is why the switch below has no arm for it.
		if inv.Status == onboardstore.InviteSubmitted {
			continue
		}
		if sponsorKeys != nil && !sponsorAllows(sponsorKeys, inv.Sponsor) {
			continue
		}
		v := inviteToView(inv, archiveAfter)
		if v.MetadataError != "" {
			slog.ErrorContext(ctx, "onboarding: invite access metadata unreadable",
				slog.String("invite_id", inv.ID))
		}
		switch v.State {
		case "open":
			live = append(live, v)
		case "archived":
			archived = append(archived, v)
		default:
			dead = append(dead, v)
		}
	}
	return append(live, dead...), archived, len(live)
}

// stageCounts buckets every proposal by status, sponsor-scoped exactly like the
// lists. One all-status pull — the same read the tab badges have always made,
// shared with the counts endpoint so the badge and the page can never disagree.
func (s *Server) stageCounts(ctx context.Context, sponsorKeys map[string]struct{}) (map[string]int, error) {
	all, err := s.Onboard.ListProposals(ctx, "")
	if err != nil {
		return nil, err
	}
	return countStages(all, sponsorKeys), nil
}

// countStages is the bucketing behind stageCounts, pure over the rows so the
// review page can feed it the pull it already holds.
func countStages(all []onboardstore.Proposal, sponsorKeys map[string]struct{}) map[string]int {
	counts := map[string]int{}
	for _, p := range all {
		if sponsorKeys != nil && !sponsorAllows(sponsorKeys, p.Sponsor) {
			continue
		}
		counts[p.Status]++
	}
	return counts
}

// handleOnboardCounts is the poll target behind the live tab badges: the two
// gate counts, nothing else. Gated by onboardGate (the same door as the queue
// page), so a sponsor session sees only their own invitees — the badge must
// never reveal a queue depth the page itself would not show them.
func (s *Server) handleOnboardCounts(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		onboardError(w, http.StatusServiceUnavailable, "not_configured", "onboarding is not configured")
		return
	}
	var sponsorKeys map[string]struct{}
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		sponsorKeys = sp.keys()
	}
	counts, err := s.stageCounts(r.Context(), sponsorKeys)
	if err != nil {
		onboardError(w, http.StatusBadGateway, "counts_error", "onboarding queue unavailable")
		return
	}
	onboardJSON(w, http.StatusOK, map[string]int{
		onboardstore.ProposalProposed:        counts[onboardstore.ProposalProposed],
		onboardstore.ProposalSponsorApproved: counts[onboardstore.ProposalSponsorApproved],
	})
}

func (s *Server) handleOnboarding(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	// Sponsor sessions (ADR 1072) see only proposals carrying their own
	// sponsorship; the same keys filter the tab counts below.
	var sponsorKeys map[string]struct{}
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		sponsorKeys = sp.keys()
	}
	status := r.URL.Query().Get("status")
	if status == "" {
		// Land each role on its own actionable stage: a sponsor's gate is the
		// vouch (proposed); the admin gate is sponsor_approved.
		if sponsorKeys != nil {
			status = onboardstore.ProposalProposed
		} else {
			status = onboardstore.ProposalSponsorApproved
		}
	}

	// Optional flash banner carried on the redirect back from the decision
	// handlers. The query param carries a CODE, never the message text: this
	// page is reachable by URL, so reflecting caller-supplied prose would let
	// anyone who can get an operator to follow a link put arbitrary words
	// inside the trusted console chrome. An unrecognized code renders no
	// banner at all.
	var flashMsg *flash
	flashCode := strings.TrimSpace(r.URL.Query().Get("flash"))
	if f, ok := onboardFlashes[flashCode]; ok {
		flashMsg = &f
	}

	// One all-status pull each for proposals and invites. The tab badges, the
	// Invited tab and the cross-stage find index (#1306) are bucketed from
	// these two slices, so a badge and a search hit come from the same read.
	// The open proposal tab's own list is still the separate
	// ListProposals(status) pull below.
	allProposals, err := s.Onboard.ListProposals(r.Context(), "")
	if err != nil {
		s.renderError(w, r, 502, "onboarding queue unavailable: "+err.Error())
		return
	}
	allInvites, inviteErr := s.Onboard.ListInvites(r.Context(), "")
	if inviteErr != nil {
		// The proposal tabs render without the invite badge, as they always
		// have; the index says invites are missing rather than pretending it
		// searched them. The Invited tab itself still refuses below.
		slog.WarnContext(r.Context(), "onboarding: invites unavailable; page renders without them",
			slog.String("err", inviteErr.Error()))
		allInvites = nil
	}
	inviteViews, archivedInviteViews, inviteCount := splitInviteViews(r.Context(), allInvites, sponsorKeys, s.Cfg.InviteArchiveAfter)

	// The Invited tab lists invite rows; every other tab lists proposals
	// (a "" store filter = all; the queue tabs pass a concrete status).
	var views []proposalView
	if status == onboardStageInvited {
		if inviteErr != nil {
			s.renderError(w, r, 502, "onboarding queue unavailable: "+inviteErr.Error())
			return
		}
	} else {
		rows, err := s.Onboard.ListProposals(r.Context(), status)
		if err != nil {
			s.renderError(w, r, 502, "onboarding queue unavailable: "+err.Error())
			return
		}
		views = make([]proposalView, 0, len(rows))
		// One parse of the intake hints for every draft the editor prefills.
		hints := sync.OnceValues(loadIntakeHints)
		for _, p := range rows {
			if sponsorKeys != nil && !sponsorAllows(sponsorKeys, p.Sponsor) {
				continue
			}
			v := proposalToView(p)
			// Org match panel: pending review stages only, ADMIN sessions
			// only (a sponsor vouches for the person; org structure is an
			// admin act, so sponsors don't see attach/create controls).
			if p.Status == onboardstore.ProposalProposed || p.Status == onboardstore.ProposalSponsorApproved {
				v.OrgUnregistered = s.orgUnregistered(p)
			}
			if sponsorKeys == nil && (p.Status == onboardstore.ProposalProposed || p.Status == onboardstore.ProposalSponsorApproved) {
				f, cs := s.orgMatchesForProposal(p)
				v.OrgTyped, v.OrgCandidates, v.OrgKind, v.OrgNation = f.AffID, cs, f.Kind, f.Nation
				v.OrgTypedName, v.OrgClaimed = f.Typed, f.Registered
				s.fillEditable(&v, p, hints)
			}
			// Registered-org chain: read-only proof of capture, all
			// stages, sponsor sessions included.
			v.OrgResolved = s.orgChainForProposal(p)
			v.seedNationFromRegisteredOrg()
			v.Relationship = s.relationshipChoiceFor(p)
			views = append(views, v)
		}
	}

	// Stage tab counts, bucketed from the pull above.
	counts := countStages(allProposals, sponsorKeys)
	counts[onboardStageInvited] = inviteCount
	stages := make([]onboardStage, 0, len(onboardStageDefs))
	for _, d := range onboardStageDefs {
		stages = append(stages, onboardStage{Key: d.Key, Label: d.Label, Count: counts[d.Key]})
	}

	// Cross-stage find index (#1306): scoped before it is built, rendered
	// hidden, filtered in the browser. The typed term travels in the URL
	// fragment, which never reaches the server, so nothing about it is read
	// from the request here: no filter, no prefill.
	findRows, findTruncated := buildFindIndex(allProposals, allInvites, sponsorKeys)

	vars := &pageVars{
		Title:                  "Onboarding — review",
		OnboardEnabled:         true,
		Proposals:              views,
		OnboardInvites:         inviteViews,
		OnboardInvitesArchived: archivedInviteViews,
		OnboardStatus:          status,
		OnboardStages:          stages,
		OnboardFindRows:        findRows,
		OnboardFindTruncated:   findTruncated,
		OnboardFindPartial:     inviteErr != nil,
		Flashes:                flashes(flashMsg),
		RequireRegisteredOrg:   s.Cfg.RequireRegisteredOrg,
	}
	if s.Cfg.InviteArchiveAfter > 0 {
		vars.InviteArchiveSweepEnabled = true
		vars.InviteArchiveWindow = humanArchiveWindow(s.Cfg.InviteArchiveAfter)
	}
	// The runner band: admin sessions only — a sponsor neither sees
	// the runner's stages nor can act on them. A store error here costs the
	// band, not the page: the rows are still listed, and the "runner?" row
	// badge still fires on its own clock.
	if sponsorKeys == nil {
		lastClaim, cerr := s.Onboard.LastRunnerClaim(r.Context())
		if cerr != nil {
			slog.WarnContext(r.Context(), "onboarding: last runner claim unavailable; runner band not rendered",
				slog.String("err", cerr.Error()))
		} else {
			vars.RunnerQueue = runnerQueue(allProposals, lastClaim, time.Now(), s.runnerLateAfter)
		}
	}

	// Operator-keys tab (#210): admin sessions only — it sits AFTER a divider
	// at the end of the strip because the stage tabs read as one candidate's
	// chronological journey, and an operator key is not a step each candidate
	// passes through. Sponsors get no tab in the markup at all: the pending
	// count is not their information, and the predicate here is the same
	// sponsor resolution the queue's adminGate direction enforces. A count
	// error keeps the tab with a quiet zero rather than dropping the surface.
	if sponsorKeys == nil {
		vars.OperatorKeysTab = true
		if ks, kerr := s.Onboard.ListOperatorKeys(r.Context(), onboardstore.OperatorKeyRequested); kerr == nil {
			vars.OperatorKeysPending = len(ks)
		}
	}

	// Detail pane: ?id=<proposal|invite>. A proposal is loaded via GetProposal
	// (carries manifest_json) and serialized for the client-side identity card;
	// an Invited-tab id selects the invite row for the invite pane.
	if id := r.URL.Query().Get("id"); id != "" {
		if status == onboardStageInvited {
			inv, err := s.Onboard.GetInvite(r.Context(), id)
			if err != nil {
				s.renderError(w, r, 502, "load invite: "+err.Error())
				return
			}
			// A foreign invite reads as absent for a sponsor session.
			if sponsorKeys != nil && !sponsorAllows(sponsorKeys, inv.Sponsor) {
				s.renderError(w, r, 404, "invite not found")
				return
			}
			v := inviteToView(*inv, s.Cfg.InviteArchiveAfter)
			if v.MetadataError != "" {
				slog.ErrorContext(r.Context(), "onboarding: invite access metadata unreadable",
					slog.String("invite_id", inv.ID))
			}
			vars.OnboardInvite = &v
		} else {
			p, err := s.Onboard.GetProposal(r.Context(), id)
			if err != nil {
				s.renderError(w, r, 502, "load proposal: "+err.Error())
				return
			}
			// A foreign proposal reads as absent for a sponsor session.
			if sponsorKeys != nil && !sponsorAllows(sponsorKeys, p.Sponsor) {
				s.renderError(w, r, 404, "proposal not found")
				return
			}
			v := proposalToView(*p)
			// Org match panel on the detail card (ADR 1112): admin sessions,
			// pending review stages only. Never auto-applied — a human clicks.
			if p.Status == onboardstore.ProposalProposed || p.Status == onboardstore.ProposalSponsorApproved {
				v.OrgUnregistered = s.orgUnregistered(*p)
			}
			if sponsorKeys == nil && (p.Status == onboardstore.ProposalProposed || p.Status == onboardstore.ProposalSponsorApproved) {
				f, cs := s.orgMatchesForProposal(*p)
				v.OrgTyped, v.OrgCandidates, v.OrgKind, v.OrgNation = f.AffID, cs, f.Kind, f.Nation
				v.OrgTypedName, v.OrgClaimed = f.Typed, f.Registered
				s.fillEditable(&v, *p, loadIntakeHints)
			}
			// Post-mint GitHub username: admin sessions on an onboarded or
			// enrolled person, the only stages SetProposalGitHub accepts.
			if sponsorKeys == nil && (p.Status == onboardstore.ProposalOnboarded || p.Status == onboardstore.ProposalEnrolled) {
				v.GitHubPostMint = true
				_, v.GitHub = manifestIdentity(p.ManifestJSON)
				// Device reads belong to the selected card, never the list.
				// Load after HTML arrives so NetBird cannot block navigation (#2384).
				v.DevicesURL = "/onboarding/" + url.PathEscape(p.ID) + "/devices"
			}
			// Registered-org chain: read-only proof of capture, all
			// stages, sponsor sessions included.
			v.OrgResolved = s.orgChainForProposal(*p)
			v.seedNationFromRegisteredOrg()
			v.Relationship = s.relationshipChoiceFor(*p)
			vars.Proposal = &v
			// The restore banners are built from THIS row, so they can only be
			// set once it is loaded — after vars was assembled above.
			if f := restoreFlash(flashCode, *p); f != nil {
				vars.Flashes = flashes(f)
			}
			b, err := json.Marshal(p)          // the FULL store row (manifest_json/petitions/…) for the card
			_ = err                            // store rows are JSON-marshalable by construction; encode failures are not expected
			vars.ProposalJSON = template.JS(b) //nolint:gosec // store-sourced, HTML-escaped by the card via textContent
		}
	}
	s.render(w, r, "onboarding", vars)
}

// handleOnboardInviteRevoke kills an open invite from the queue's Invited tab —
// the human-web counterpart of POST /api/onboard/invites/{id}/revoke. Sponsor
// sessions may revoke only their own invites (ADR 1072); the store refuses a
// non-open invite, so a consumed link is never silently "revoked".
func (s *Server) handleOnboardInviteRevoke(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	id := r.PathValue("id")
	actor := actorFromCtx(r.Context())
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		inv, gerr := s.Onboard.GetInvite(r.Context(), id)
		if gerr != nil || !sponsorAllows(sp.keys(), inv.Sponsor) {
			s.renderError(w, r, 404, "invite not found")
			return
		}
	}
	err := s.Onboard.RevokeInvite(r.Context(), id)
	switch {
	case err == nil:
		// ok
	case errors.Is(err, onboardstore.ErrNotFound):
		s.renderError(w, r, 404, "invite not found")
		return
	case errors.Is(err, onboardstore.ErrInviteConsumed):
		s.emitAuditWithInviteToken(r, actor, "onboard_revoke_invite", id, "", nil, nil, false, "error", "invite is no longer open")
		s.renderError(w, r, 409, "invite is no longer open — cannot revoke")
		return
	default:
		s.emitAuditWithInviteToken(r, actor, "onboard_revoke_invite", id, "", nil, nil, false, "error", err.Error())
		s.renderError(w, r, 502, "revoke invite failed: "+err.Error())
		return
	}
	s.emitAuditWithInviteToken(r, actor, "onboard_revoke_invite", id, "", nil,
		map[string]interface{}{"status": onboardstore.InviteRevoked}, false, "ok", "")
	http.Redirect(w, r, "/onboarding?status="+onboardStageInvited, http.StatusSeeOther)
}

// handleOnboardInviteForm renders the "sponsor a candidate" form (mint a one-time
// onboarding link). Everything is optional — the enrollee fills what the sponsor
// leaves open.
func (s *Server) handleOnboardInviteForm(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	s.render(w, r, "onboard_invite", &pageVars{Title: "Sponsor a candidate", OnboardEnabled: true, MailerEnabled: s.Mailer.Enabled()})
}

// Invite TTL bounds (mirrors the portal): unset defaults to 72h; capped at 720h.
const (
	defaultInviteTTLHours = 72
	minInviteTTLHours     = 1
	maxInviteTTLHours     = 720
	// shortInviteTTLHours — at or below this the mint page warns and the invite
	// list flags the row. Short links are legitimate (a hand-over happening in
	// the same call), but they are also what an unintended TTL looks like, and
	// the candidate may believe they have days. Warn, never refuse.
	shortInviteTTLHours = 12
)

// inviteExpiryLabel renders an expiry the way the mint page and the invite list
// need it: relative first (how long the candidate has), absolute second (the
// deadline anyone can check a clock against).
func inviteExpiryLabel(expiresAt int64) string {
	if expiresAt <= 0 {
		return ""
	}
	exp := time.Unix(expiresAt, 0).UTC()
	d := time.Until(exp)
	if d <= 0 {
		return "expired — " + exp.Format("2006-01-02 15:04") + " UTC"
	}
	return "in " + humanSince(exp) + " — " + exp.Format("2006-01-02 15:04") + " UTC"
}

// handleOnboardInviteCreate mints the invite locally + shows the link. The admin
// is operator-equivalent (ADR 1071): sponsorship-authorized with no tier cap, so
// the pre-bound tier rides the invite unbounded (SponsorMaxTier "").
func (s *Server) handleOnboardInviteCreate(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	actor := actorFromCtx(r.Context())
	sponsor := actor.Email

	// env decides WHICH onboard-runner will ever pick this proposal up. The
	// runner is scoped to one goatnet and skips anything else forever, so a
	// wrong env is not a slow onboard — it is a permanent one, and it looks like
	// success to everybody: the proposal reads `approved`, complete, lint-clean.
	//
	// The console knows which goatnet it serves. That is the authority, and the
	// request cannot override it. A form-supplied value is unsafe: a hardcoded value
	// in a template would stamp every invite minted on any other console with the
	// wrong goatnet, and those proposals could never onboard. Checking only that env
	// is non-BLANK misses that wrong-value case.
	env := strings.TrimSpace(s.Cfg.NotifyGoatnet)
	if env == "" {
		// No declared goatnet: fall back to the request, still validated, so a
		// deployment that has not set NOTIFY_GOATNET keeps working rather than
		// losing the ability to mint.
		env = strings.TrimSpace(r.FormValue("env"))
	} else if want := strings.TrimSpace(r.FormValue("env")); want != "" && want != env {
		// A caller asking for a different goatnet is refused LOUDLY rather than
		// silently corrected: if this ever fires, either the form is stale or
		// somebody is trying to mint across environments, and both deserve to
		// be seen at mint time instead of discovered weeks later.
		s.renderError(w, r, 400, fmt.Sprintf(
			"this console serves %q, so it cannot mint an invite for %q — a proposal only onboards where its runner runs",
			env, want))
		return
	}
	if env == "" {
		s.renderError(w, r, 400, "env is required — a blank env strands the proposal for a scoped onboard-runner")
		return
	}
	if !envPattern.MatchString(env) {
		s.renderError(w, r, 400, "env must be a lowercase goatnet slug (e.g. galaxy)")
		return
	}
	quotaStr := strings.TrimSpace(r.FormValue("machine_quota"))
	quota := 0
	if quotaStr != "" {
		q, err := strconv.Atoi(quotaStr)
		if err != nil {
			s.renderError(w, r, 400, "machine_quota must be a non-negative integer")
			return
		}
		if q < 0 {
			s.renderError(w, r, 400, "machine_quota must be a non-negative integer")
			return
		}
		quota = q
	}
	ttlStr := strings.TrimSpace(r.FormValue("ttl_hours"))
	ttl := defaultInviteTTLHours
	if ttlStr != "" {
		var err error
		ttl, err = strconv.Atoi(ttlStr)
		if err != nil {
			s.renderError(w, r, 400, "ttl_hours must be an integer number of hours")
			return
		}
	}
	if ttl < minInviteTTLHours || ttl > maxInviteTTLHours {
		s.renderError(w, r, 400, fmt.Sprintf("ttl_hours must be between %d and %d", minInviteTTLHours, maxInviteTTLHours))
		return
	}

	prebound := onboardstore.Prebound{
		Tier: strings.TrimSpace(r.FormValue("tier")),
		// ADR 1151: the mint form asks for SECTOR, not the deprecated kind. The
		// `affiliation` form value is still read so an older cached form keeps
		// working, but nothing renders a control that sets it.
		Affiliation:  strings.TrimSpace(r.FormValue("affiliation")),
		Sector:       strings.ToLower(strings.TrimSpace(r.FormValue("sector"))),
		MachineQuota: quota,
		// collab: the form checkbox (value "1") is checked by default, so the
		// sponsor unticks to exclude. Absent (checkbox unchecked, or an older form
		// without the field) → false, a safe default that never provisions collab
		// without the decision. Captured into the manifest as collab.enabled.
		Collab: r.FormValue("collab") == "1",
		// ADMINS minter → no tier cap (ADR 1071 "operators sponsor by
		// default"). A sponsor session (ADR 1072) carries its annotated cap
		// instead — set just below.
		SponsorMaxTier: "",
	}
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		prebound.SponsorMaxTier = sp.MaxTier
		if prebound.Tier != "" && sp.MaxTier != "" {
			h, herr := loadIntakeHints()
			if herr != nil {
				s.renderError(w, r, 500, "onboarding config unavailable")
				return
			}
			if !h.tierWithin(prebound.Tier, sp.MaxTier) {
				s.renderError(w, r, 403, fmt.Sprintf("you may sponsor up to %q — %q exceeds your sponsorship level", sp.MaxTier, prebound.Tier))
				return
			}
		}
	}
	// Recommended org (ADR 1112). The sponsor names a REGISTERED org; the invite
	// carries its canonical id so the candidate is shown their organization
	// instead of free-typing one, and assembly writes that id to affiliation.id
	// rather than slugging a typed string. Fail-closed on both arms: an org the
	// registry does not know, and an org named on a goatnet with no registry, are
	// both refused at mint. Silently dropping the value would mint an invite that
	// looks pinned and behaves un-pinned — the failure shape the whole match-or-
	// create step exists to avoid.
	if orgIn := strings.TrimSpace(r.FormValue("org")); orgIn != "" {
		if s.Orgs == nil {
			s.renderError(w, r, 400, "this goatnet has no org registry, so an organization cannot be pinned onto an invite — leave it blank")
			return
		}
		canonical, oerr := s.Orgs.Resolve(orgIn)
		if oerr != nil {
			s.renderError(w, r, 400, fmt.Sprintf("%q is not a registered org id or alias — register the org first, or leave the field blank", orgIn))
			return
		}
		prebound.Org = canonical
	}
	preboundJSON, err := json.Marshal(prebound)
	_ = err // Prebound is a plain struct; JSON encode cannot fail here in practice.

	// The candidate address is recorded on the invite at mint, before any send
	// is attempted: who a link was meant for is part of the record even when
	// delivery is out of band or fails (the sandbox case just below).
	candidateEmail := strings.TrimSpace(r.FormValue("candidate_email"))
	if refusal := s.checkInviteRecipient(r, candidateEmail); refusal != nil {
		s.renderError(w, r, refusal.status, refusal.message)
		return
	}
	inv := &onboardstore.Invite{
		Sponsor:        sponsor,
		SponsorName:    sponsor, // oauth2-proxy email — already the human label
		Env:            env,
		PreboundJSON:   string(preboundJSON),
		ExpiresAt:      time.Now().Add(time.Duration(ttl) * time.Hour).Unix(),
		Note:           strings.TrimSpace(r.FormValue("note")),
		CandidateEmail: candidateEmail,
	}
	token, err := s.Onboard.MintInvite(r.Context(), inv)
	if err != nil {
		s.emitAuditWithInviteToken(r, actor, "onboard_mint_invite", "", "", nil, nil, false, "error", err.Error())
		s.renderError(w, r, 502, "mint invite failed: "+err.Error())
		return
	}

	escalated := isEscalatedTier(prebound.Tier)
	res := &inviteMintedView{
		ID:           inv.ID,
		Sponsor:      inv.Sponsor,
		Env:          inv.Env,
		Escalated:    escalated,
		ExpiresAt:    inv.ExpiresAt,
		ExpiresLabel: inviteExpiryLabel(inv.ExpiresAt),
		ShortTTL:     ttl <= shortInviteTTLHours,
	}
	// Build the candidate links. The public front-desk link (INTAKE_BASE_URL) is
	// preferred; the mesh-side nomination link (PORTAL_PUBLIC_URL) is the fallback
	// the sponsor can use today. Both bases front through the public intake
	// collector (or its equivalent), whose candidate-facing path is the clean
	// /onboard.html - nginx rewrites that to the console's /public/onboard.html
	// internally. Emitting /public here 404s at the front desk.
	nomPath := "/onboard.html?t=" + token
	if base := strings.TrimRight(s.Cfg.IntakeBaseURL, "/"); base != "" {
		res.IntakeURL = base + nomPath
	}
	if base := strings.TrimRight(s.Cfg.PortalPublicURL, "/"); base != "" {
		res.NominationURL = base + nomPath
	}

	s.emitAuditWithInviteToken(r, actor, "onboard_mint_invite", inv.ID, "", nil,
		map[string]interface{}{"env": inv.Env, "escalated": escalated}, false, "ok", "")

	// Email the candidate the link when an address is given + the mailer is
	// configured. Sandbox-aware (ADR 1062 delivery increment): while the SES
	// account is in the sandbox, an unverified recipient would 554 — so instead
	// of a blind send, the page offers the verification controls. Best-effort
	// throughout: any failure still shows the link to hand over.
	vars := &pageVars{Title: "Invite created", OnboardEnabled: true, InviteResult: res, MailerEnabled: s.Mailer.Enabled()}
	link := res.IntakeURL
	if link == "" {
		link = res.NominationURL
	}
	to := candidateEmail
	vars.InviteCandidateEmail = to
	if to != "" && s.Mailer.Enabled() {
		send := true
		if sandbox, verified, ok := s.sesSandboxState(r, to); ok && sandbox {
			vars.InviteSandbox, vars.InviteVerified = true, verified
			send = verified // unverified sandbox recipient → offer controls, don't 554
		}
		if send {
			if err := s.Mailer.SendInvite(to, link, res.Sponsor, res.Env, s.Cfg.NotifyGoatnetTagline, s.Cfg.NotifyGoatnetPurpose, res.ExpiresAt); err != nil {
				vars.InviteEmailErr = err.Error()
				s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", res.ID, "", nil, map[string]interface{}{"to": to}, false, "error", err.Error())
			} else {
				vars.InviteEmailedTo = to
				s.recordInviteSend(r, res.ID, to, onboardstore.SentViaEmail)
				s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", res.ID, "", nil, map[string]interface{}{"to": to}, false, "ok", "")
			}
		}
	}
	s.render(w, r, "onboard_invite", vars)
}

// recordInviteSend stamps the delivery onto the invite row so the queue's invite
// record shows who the link went to and when it left. Best-effort: the send
// already happened, so a store error must never fail the response — the audit
// trail carries the same fact either way.
func (s *Server) recordInviteSend(r *http.Request, id, to, via string) {
	if s.Onboard == nil || id == "" {
		return
	}
	_ = s.Onboard.RecordInviteSend(r.Context(), id, to, via, time.Now().Unix()) //nolint:errcheck // delivery already happened; audit carries the same fact
}

// addressInvite records the intended recipient without claiming a send. Same
// best-effort posture as recordInviteSend.
func (s *Server) addressInvite(r *http.Request, id, to string) {
	if s.Onboard == nil || id == "" {
		return
	}
	_ = s.Onboard.AddressInvite(r.Context(), id, to) //nolint:errcheck // address metadata must not block delivery
}

// sesSandboxState reports (sandbox, recipientVerified, ok). ok=false when the
// SES API is unconfigured or errors — callers then behave as before (send and
// let SMTP report), never blocking delivery on a broken side-channel.
func (s *Server) sesSandboxState(r *http.Request, to string) (sandbox, verified, ok bool) {
	if !s.SES.Enabled() {
		return false, false, false
	}
	prod, err := s.SES.ProductionAccess(r.Context())
	if err != nil {
		return false, false, false
	}
	if prod {
		return false, true, true
	}
	v, err := s.SES.IdentityVerified(r.Context(), to)
	if err != nil {
		return true, false, true
	}
	return true, v, true
}

// inviteFromForm rebuilds the minted-invite view from the posted form fields.
// The one-time link exists only in the invite-created page (never stored), so
// the email-delivery actions round-trip it through hidden fields.
func inviteFromForm(r *http.Request) (*inviteMintedView, string, string) {
	m := &inviteMintedView{
		ID:            r.FormValue("invite_id"),
		Sponsor:       r.FormValue("sponsor"),
		Env:           r.FormValue("env"),
		IntakeURL:     r.FormValue("intake_url"),
		NominationURL: r.FormValue("nomination_url"),
		Escalated:     r.FormValue("escalated") == "true",
	}
	// The deadline rides the form so a send from this page states the same
	// expiry the mint showed. Unparseable/absent → 0, which degrades the email
	// to the old vague wording rather than inventing a deadline.
	m.ExpiresAt, _ = strconv.ParseInt(r.FormValue("expires_at"), 10, 64) //nolint:errcheck // invalid optional deadline intentionally degrades to zero
	m.ExpiresLabel = inviteExpiryLabel(m.ExpiresAt)
	m.ShortTTL = m.ExpiresAt > 0 && time.Until(time.Unix(m.ExpiresAt, 0)) <= shortInviteTTLHours*time.Hour
	link := m.IntakeURL
	if link == "" {
		link = m.NominationURL
	}
	return m, link, strings.TrimSpace(r.FormValue("candidate_email"))
}

// handleOnboardInviteEmailVerify triggers SES's verification email to the
// candidate (the sandbox prerequisite for that address to receive mail).
func (s *Server) handleOnboardInviteEmailVerify(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	m, _, to := inviteFromForm(r)
	actor := actorFromCtx(r.Context())
	vars := &pageVars{Title: "Invite created", OnboardEnabled: true, InviteResult: m,
		MailerEnabled: s.Mailer.Enabled(), InviteCandidateEmail: to, InviteSandbox: true}
	if to == "" {
		vars.InviteEmailErr = "enter the candidate's email address first"
	} else {
		// Addressed, not sent. Recording the intended recipient here is the
		// point of the step: verification is what an unverified sandbox
		// address needs BEFORE a send, so this is often the only moment the
		// record learns who the link was meant for.
		s.addressInvite(r, m.ID, to)
		if err := s.SES.VerifyIdentity(r.Context(), to); err != nil {
			vars.InviteEmailErr = "could not trigger the verification email: " + err.Error()
			s.emitAuditWithInviteToken(r, actor, "onboard_email_verify", m.ID, "", nil, map[string]interface{}{"to": to}, false, "error", err.Error())
		} else {
			vars.InviteVerifySent = true
			s.emitAuditWithInviteToken(r, actor, "onboard_email_verify", m.ID, "", nil, map[string]interface{}{"to": to}, false, "ok", "")
		}
	}
	s.render(w, r, "onboard_invite", vars)
}

// handleOnboardInviteEmailSend checks verification (sandbox) then sends the
// invite email. In production (or when the SES API is unconfigured) it just
// sends and lets SMTP report.
func (s *Server) handleOnboardInviteEmailSend(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	m, link, to := inviteFromForm(r)
	if refusal := s.checkInviteRecipient(r, to); refusal != nil {
		s.renderError(w, r, refusal.status, refusal.message)
		return
	}
	actor := actorFromCtx(r.Context())
	vars := &pageVars{Title: "Invite created", OnboardEnabled: true, InviteResult: m,
		MailerEnabled: s.Mailer.Enabled(), InviteCandidateEmail: to}
	switch {
	case to == "":
		vars.InviteEmailErr = "enter the candidate's email address first"
	case !s.Mailer.Enabled():
		vars.InviteEmailErr = "email sending is not configured on this console"
	default:
		// Address the row before attempting: a send that fails, or a sandbox
		// address that has not verified yet, still tells the record who this
		// link is for.
		s.addressInvite(r, m.ID, to)
		if sandbox, verified, ok := s.sesSandboxState(r, to); ok && sandbox {
			vars.InviteSandbox, vars.InviteVerified = true, verified
			if !verified {
				vars.InviteEmailErr = "recipient has not completed verification yet — have them click the link in the verification email, then try again"
				s.render(w, r, "onboard_invite", vars)
				return
			}
		}
		if err := s.Mailer.SendInvite(to, link, m.Sponsor, m.Env, s.Cfg.NotifyGoatnetTagline, s.Cfg.NotifyGoatnetPurpose, m.ExpiresAt); err != nil {
			vars.InviteEmailErr = err.Error()
			s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", m.ID, "", nil, map[string]interface{}{"to": to}, false, "error", err.Error())
		} else {
			vars.InviteEmailedTo = to
			s.recordInviteSend(r, m.ID, to, onboardstore.SentViaEmail)
			s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", m.ID, "", nil, map[string]interface{}{"to": to}, false, "ok", "")
		}
	}
	s.render(w, r, "onboard_invite", vars)
}

// handleOnboardInviteEml serves the composed invite as a downloadable .eml so
// the admin can deliver it out-of-band from their own mail client — available
// regardless of SES sandbox/production state (and even with SMTP unconfigured).
func (s *Server) handleOnboardInviteEml(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	m, link, to := inviteFromForm(r)
	if refusal := s.checkInviteRecipient(r, to); refusal != nil {
		s.renderError(w, r, refusal.status, refusal.message)
		return
	}
	actor := actorFromCtx(r.Context())
	msg := s.Mailer.ComposeInvite(to, link, m.Sponsor, m.Env, s.Cfg.NotifyGoatnetTagline, s.Cfg.NotifyGoatnetPurpose, m.ExpiresAt)
	// Hand-over, not delivery: the operator sends this from their own client, so
	// the record says "eml" rather than claiming the console mailed it.
	s.recordInviteSend(r, m.ID, to, onboardstore.SentViaEML)
	s.emitAuditWithInviteToken(r, actor, "onboard_email_download", m.ID, "", nil, map[string]interface{}{"to": to}, false, "ok", "")
	name := "invite"
	if m.ID != "" {
		name = "invite-" + m.ID
	}
	w.Header().Set("Content-Type", "message/rfc822")
	w.Header().Set("Content-Disposition", `attachment; filename="`+name+`.eml"`)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write([]byte(msg))
}

// handleOnboardingDecide records approve | reject | sponsor-approve | sponsor-reject
// on the local store, then redirects back to the queue. The two-gate order is
// enforced (proposed → sponsor_approved → approved; reject from either gate). The
// admin is operator-equivalent, so the sponsor/tier caps never bind here.
func (s *Server) handleOnboardingDecide(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	id := r.PathValue("id")
	action := r.PathValue("action")
	switch action {
	case "approve", "reject", "sponsor-approve", "sponsor-reject", "retry", "offboard", "offboard-cut":
	default:
		s.renderError(w, r, 400, "unknown action: "+action)
		return
	}
	// Sponsor sessions (ADR 1072): only the sponsor-stage actions, only on
	// their own invitees, only within their tier cap. Admin decisions stay
	// with ADMINS sessions.
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		if action != "sponsor-approve" && action != "sponsor-reject" {
			s.renderError(w, r, 403, "sponsors can approve or decline their own invitees; admin decisions require an ADMINS session")
			return
		}
		p, gerr := s.Onboard.GetProposal(r.Context(), id)
		if gerr != nil || !sponsorAllows(sp.keys(), p.Sponsor) {
			s.renderError(w, r, 404, "proposal not found")
			return
		}
		if sp.MaxTier != "" && strings.TrimSpace(p.Tier) != "" {
			h, herr := loadIntakeHints()
			if herr != nil {
				s.renderError(w, r, 500, "onboarding config unavailable")
				return
			}
			if !h.tierWithin(p.Tier, sp.MaxTier) {
				s.renderError(w, r, 403, fmt.Sprintf("you may vouch up to %q — this proposal carries %q", sp.MaxTier, p.Tier))
				return
			}
		}
	}
	note := r.FormValue("note")
	actor := actorFromCtx(r.Context())

	// The REAL cut (goat#372) queues work for the onboard-runner and is
	// step-up-gated; it shares nothing with the decide flow below (whose
	// offboard action asserts the cut already happened elsewhere).
	if action == "offboard-cut" {
		s.handleOffboardCut(w, r, id, actor)
		return
	}

	// The browser button ASSERTS that access was already cut elsewhere; verify
	// that rather than trust it. Recording the mark releases the email-keyed
	// duplicate guard (ActiveProposalForEmail), so a false assertion lets a
	// SECOND credential set be minted for someone still holding their first —
	// the collision-safety failure this goatnet has already been bitten by
	// (ADR 1048). The MACHINE path (apiOffboardProposal) is deliberately NOT
	// gated: that is `goat admin onboard --revoke`'s R0 leg, which marks the
	// row and then performs the cut in the same run, so the login is still
	// active by design when it calls.
	// For offboard, capture the pre-decision state for the audit trail: with
	// three eligible source states, "from enrolled credential-holder" vs
	// "from never-joined archived" is a distinction every offboard audit row
	// (refusal included) must keep. Best-effort read (nil Before on error);
	// the guarded store UPDATE stays the enforcement — this read is context,
	// never a gate.
	var auditBefore map[string]interface{}
	if action == "offboard" {
		p0, gerr := s.Onboard.GetProposal(r.Context(), id)
		if gerr == nil {
			auditBefore = map[string]interface{}{"status": p0.Status}
		}
		// The archived-row reason rule (decideProposal enforces it in both
		// realms) is checked here first, before the login-plane read: the
		// browser's required-field check lets a whitespace-only reason
		// through, and an operator should learn the reason is missing at
		// once, not after waiting on Kratos and dealing with its answer
		// (#1283). The redirect matches the decide-side refusal — back to
		// the archived card with this row selected — and so does the audit
		// row's err text: one rule, one sentence, whichever leg caught it.
		if gerr == nil && (p0.Status == onboardstore.ProposalArchived || p0.Status == onboardstore.ProposalOffboardRequested) && strings.TrimSpace(note) == "" {
			msg, code := offboardNoteRequiredMsg, flashOffboardNoteRequired
			if p0.Status == onboardstore.ProposalOffboardRequested {
				msg, code = settleNoteRequiredMsg, flashSettleNoteRequired
			}
			s.emitAuditWithInviteToken(r, actor, "onboard_offboard", id, "", auditBefore, nil, false, "refused", msg)
			q := url.Values{}
			q.Set("flash", code)
			q.Set("status", p0.Status)
			q.Set("id", id)
			http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
			return
		}
		if code := s.offboardPrecheck(r, id); code != "" {
			s.emitAuditWithInviteToken(r, actor, "onboard_offboard", id, "", auditBefore, nil, false, "refused", code)
			q := url.Values{}
			q.Set("flash", code)
			http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
			return
		}
	}

	// ADR 1171: the membership relationship is declared at the moment of
	// approval, by the one person who knows it. decideProposal records it
	// inside the approve arm, after a draft has assembled and before the
	// decision is written, so a failure refuses the approval rather than
	// minting an identity whose relationship nobody ever states — the field
	// exists to end an inference, and a blank one silently re-opens it.
	status, err := s.decideProposal(r, id, action, actor.Email, note, r.FormValue("relationship"))
	if err != nil {
		s.emitAuditWithInviteToken(r, actor, "onboard_"+action, id, "", auditBefore, nil, false, "error", err.Error())
		// Offboard failures return the operator to the queue with a banner
		// rather than replacing the page with an error, so they keep the
		// list they were working. Only a known code is carried; anything
		// unrecognized falls through to the normal error page rather than
		// silently showing no explanation.
		if action == "offboard" {
			var de *decideError
			if errors.As(err, &de) {
				if _, known := onboardFlashes[de.code]; known {
					q := url.Values{}
					q.Set("flash", de.code)
					// A refused archived offboard lands back on the refused
					// card (the archived filter, this row selected) rather
					// than the default queue, where the row would be off
					// screen and the operator would have to re-find it.
					if de.code == flashOffboardNoteRequired {
						q.Set("status", onboardstore.ProposalArchived)
						q.Set("id", id)
					}
					http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
					return
				}
			}
		}
		s.renderError(w, r, decideErrStatus(err), fmt.Sprintf("onboarding %s failed: %v", action, err))
		return
	}
	s.emitAuditWithInviteToken(r, actor, "onboard_"+action, id, "", auditBefore,
		map[string]interface{}{"status": status}, false, "ok", "")

	// Back to the queue at the stage the item is now most relevant to.
	next := onboardstore.ProposalSponsorApproved
	switch action {
	case "sponsor-approve":
		next = onboardstore.ProposalSponsorApproved
	case "approve", "retry":
		next = onboardstore.ProposalApproved
	case "reject", "sponsor-reject":
		next = onboardstore.ProposalRejected
	case "offboard":
		next = onboardstore.ProposalOffboarded
	}
	loc := "/onboarding?status=" + next
	if id != "" {
		loc = loc + "&id=" + url.QueryEscape(id)
	}
	http.Redirect(w, r, loc, http.StatusSeeOther)
}

// onboardStageLabel is the tab label for a proposal status ("Provisioning"
// for `onboarding`), falling back to the raw status for one the tabs do not
// list, so a flash never names a stage the operator cannot find.
func onboardStageLabel(status string) string {
	for _, d := range onboardStageDefs {
		if d.Key == status {
			return d.Label
		}
	}
	return status
}

// restoreFlash builds the two restore banners from the SELECTED row: the
// email the welcome package goes to, and the stage a refused row is actually
// in. Both facts come from the store, so the URL still carries only a code.
// nil for any other code, for these codes when no row is selected, and when
// the row does not bear the code out (a stale or edited link): the banner
// must never claim a credential is on its way unless the row was restored and
// is still in a stage where that is true (Approved or Provisioning).
func restoreFlash(code string, p onboardstore.Proposal) *flash {
	switch code {
	case flashRestored:
		if p.RestoredAt == 0 || (p.Status != onboardstore.ProposalApproved && p.Status != onboardstore.ProposalOnboarding) {
			return nil
		}
		return &flash{Level: "success", Message: fmt.Sprintf(
			"Restored. A fresh key will be minted and the welcome package sent to %s. Follow this row through Provisioning to Onboarded.", p.Email)}
	case flashRestoreState:
		if p.Status == onboardstore.ProposalArchived {
			// Still archived, yet the guarded write matched nothing: the row
			// moved and came back between the two reads, or the re-read
			// failed. Say so rather than land silently on an unchanged card.
			return &flash{Level: "error", Message: "Not restored: this record changed underneath the click. Nothing was changed — reload and try again."}
		}
		return &flash{Level: "error", Message: fmt.Sprintf(
			"Not restored: this record is no longer archived — it is in %s now, so there is nothing to bring back. Nothing was changed.", onboardStageLabel(p.Status))}
	}
	return nil
}

// handleOnboardingRestore is the Archived card's "Restore and resend key"
// button (#1037): the browser caller of the same guarded store write the
// operator API uses (apiRestoreProposal). The row returns to `approved`, the
// runner claims it, mints a FRESH setup key and the welcome package goes out
// again — neither human gate is repeated, because the row never lost them.
//
// Admin sessions only: a sponsor session is admitted by onboardGate for the
// sponsor-stage decisions, and restore is not one of them — it puts a live
// credential in an inbox, the same weight as approve. The audit event is the
// one the API emits, with the session identity as actor.
func (s *Server) handleOnboardingRestore(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	if sponsorFromCtx(r.Context()) != nil {
		s.renderError(w, r, 403, "restoring an archived person is an admin decision; sponsors approve or decline their own invitees at the sponsor stage")
		return
	}
	id := r.PathValue("id")
	actor := actorFromCtx(r.Context())
	p0, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		s.renderError(w, r, 404, "proposal not found")
		return
	}
	if err != nil {
		// A store error is not "no such row": say so, rather than telling
		// the operator the person does not exist.
		s.renderError(w, r, 502, "load proposal: "+err.Error())
		return
	}
	before := map[string]interface{}{"status": p0.Status}
	ok, err := s.Onboard.RestoreProposal(r.Context(), id, actor.Email)
	if err != nil {
		s.emitAudit(r, actor, "onboard_restore_proposal", nil, "", before, nil, false, "error", err.Error())
		if errors.Is(err, onboardstore.ErrInvalidManifest) {
			s.renderError(w, r, http.StatusConflict, err.Error())
			return
		}
		s.renderError(w, r, 500, "could not restore proposal")
		return
	}
	q := url.Values{}
	q.Set("id", id)
	if !ok {
		// Refused: the row is not archived any more (restored by somebody else
		// meanwhile, or offboarded). Land on the row where it IS, and let the
		// banner name that stage; the row is unchanged. Re-read so a race that
		// moved it between our two reads still lands on the truth.
		cur := p0.Status
		if p1, gerr := s.Onboard.GetProposal(r.Context(), id); gerr == nil {
			cur = p1.Status
		}
		s.emitAudit(r, actor, "onboard_restore_proposal", nil, "", map[string]interface{}{"status": cur}, nil,
			false, "refused", "only an archived proposal can be restored — it is "+cur)
		q.Set("flash", flashRestoreState)
		q.Set("status", cur)
		http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
		return
	}
	s.emitAudit(r, actor, "onboard_restore_proposal", nil, "",
		map[string]interface{}{"status": onboardstore.ProposalArchived},
		map[string]interface{}{"status": onboardstore.ProposalApproved, "proposal_id": id},
		false, "ok", "")
	q.Set("flash", flashRestored)
	q.Set("status", onboardstore.ProposalApproved)
	http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
}

// decideError carries an HTTP status for the human + machine decide paths.
type decideError struct {
	status int
	msg    string
	// code is a stable machine token for the cases a caller must branch
	// on. Branching on msg couples control flow to prose: a copy-edit to
	// the operator-facing wording would silently change behavior.
	code string
}

func (e *decideError) Error() string { return e.msg }

func decideErrStatus(err error) int {
	var de *decideError
	if errors.As(err, &de) {
		return de.status
	}
	return 502
}

// decideProposal applies one two-gate decision to a proposal on the local store,
// enforcing the transition order. Returns the new status. Shared by the human web
// handler and the machine-token JSON handlers so the state machine lives in one
// place. The admin/machine realms are both operator-equivalent, so no sponsorship
// tier cap is applied here (that gate lives only on the participant self-mint path,
// which is deferred — see the extraction plan).
// maxDecisionNoteBytes bounds the free-text note persisted with a proposal
// decision. The note is optional operator annotation, not load-bearing state,
// so an oversized one is truncated rather than made a reason to fail the
// decision — this keeps a machine caller from writing an unbounded blob
// straight into the onboarding store's SQLite row.
const maxDecisionNoteBytes = 4096

// truncateNote clamps a note to maxDecisionNoteBytes on a UTF-8 rune boundary,
// so a multi-byte rune is never split.
func truncateNote(s string) string {
	if len(s) <= maxDecisionNoteBytes {
		return s
	}
	n := 0
	for i := range s {
		if i > maxDecisionNoteBytes {
			break
		}
		n = i
	}
	return s[:n]
}

// requireRegisteredOrg refuses an approval while the candidate's organisation is
// not in the registry (ADR 1171 decision 7). No-op unless the goatnet has armed
// ONBOARD_REQUIRE_REGISTERED_ORG.
//
// WHY IT IS A GATE AT ALL. Without it, a candidate can be approved and
// provisioned carrying an affiliation.id that exists nowhere. Once the identity
// exists, its organisation is a string nothing resolves, and every downstream
// org-keyed decision silently has no row.
//
// WHY IT SHIPS OFF. Armed against a registry that does not yet carry the orgs a
// goatnet's live manifests already name, it refuses every onboarding. Arming is
// a per-goatnet decision taken when that goatnet is ready, never a fleet-wide
// flip.
//
// The refusal names the create action, because the admin is one click from
// fixing it and a generic conflict would send them looking elsewhere.
func (s *Server) requireRegisteredOrg(p *onboardstore.Proposal) *decideError {
	if s.Cfg == nil || !s.Cfg.RequireRegisteredOrg || s.Orgs == nil {
		return nil
	}
	f := orgFactsFromManifest(p.ManifestJSON)
	if f.AffID == "" {
		return &decideError{status: http.StatusBadRequest,
			msg: "this proposal names no organisation, and this goatnet requires a registered one before approval"}
	}
	if _, err := s.Orgs.Resolve(f.AffID); err != nil {
		return &decideError{status: http.StatusConflict,
			msg: fmt.Sprintf("%q is not a registered organisation on this goatnet. Register it with the create action on this page — its facts are typed by you, not carried over from the candidate — then approve.", f.AffID)}
	}
	return nil
}

// requireAttachedOrg refuses an approval while the proposal's organisation is
// still only the candidate's claim (#1086). The admin attaches it from the org
// panel on the same page, and that click is the membership decision.
//
// SCOPE, deliberately narrow. It fires only for an org that RESOLVES: that is
// the case with no gate of its own, because resolution reads as settlement
// everywhere downstream. An org the registry does not know is already governed
// by requireRegisteredOrg, which ships OFF by goatnet decision — firing here
// would arm that gate fleet-wide through the back door.
//
// No-op when org matching is off: the attach action lives behind the same flag,
// so a refusal would name a fix the admin cannot reach.
//
// Fails closed (503) when the registry cannot be read. The gate exists to
// refuse an unverified membership, so "could not tell whether the claimed org
// resolves" holds the approval; a retry clears it once the registry answers.
func (s *Server) requireAttachedOrg(p *onboardstore.Proposal) *decideError {
	if !s.orgMatchActive() || !orgClaimedByCandidate(*p) {
		return nil
	}
	f := orgFactsFromManifest(p.ManifestJSON)
	if f.AffID == "" {
		return nil
	}
	resolves, err := s.orgResolves(f.AffID)
	if err != nil {
		slog.Error("onboarding approve held: org registry read failed",
			slog.String("proposal", p.ID), slog.String("org", f.AffID), slog.Any("err", err))
		return &decideError{status: http.StatusServiceUnavailable,
			msg: fmt.Sprintf("The org registry could not be read, so this console cannot tell whether %q is a registered "+
				"organisation the candidate only claimed. The approval was refused; retry.", f.AffID)}
	}
	if !resolves {
		return nil
	}
	return &decideError{status: http.StatusConflict,
		msg: fmt.Sprintf("%q is the organisation this candidate typed, not one a sponsor bound them to. "+
			"It is registered, so attach the proposal to it on this page before approving — that click is what "+
			"records the membership. Approving without it provisions this person as a member of that organisation "+
			"on nobody's say-so.", f.AffID)}
}

// approvalCeiling enforces the two rules the admin gate was missing,
// both fail-closed and both audited by the caller's normal decision audit:
//
//  1. an admin may approve UP TO THEIR OWN TIER, never above it;
//  2. an approver may not be the applicant.
//
// WHY. The approve arm must read both the proposal's tier and the approving
// actor, or approving `browse` and approving `platform-admin` run identical code.
// ADR 1071's at-most-own-tier ceiling governs the SPONSOR stage and returns early
// for operators, so it does not cover this path. Without these rules an IAM-ADMIN
// could self-submit a second proposal under a second mailbox and approve it,
// granting a tier above their own. ADR 1062's two-gate protection collapses to
// one gate whenever the second human is the applicant.
//
// The same console already states rule 2 for operator KEYS ("never about your
// own: a request is decided by a different admin") and enforces it there;
// onboarding approval grants strictly more, so it enforces both.
//
// An OPERATOR principal (service token, no browser groups) is exempt from the
// CEILING only — it sits at the top of the tier ladder, so the comparison is
// vacuous — but is NOT exempt from rule 2.
// validRelationships is ADR 1171's closed membership vocabulary. It mirrors
// grammar.yaml affiliation.relationship and both manifest schemas; the parity
// gate in ops/identity/generate.py holds those three together.
var validRelationships = map[string]bool{"constituent": true, "assigned": true, "liaison": true}

// parseRelationship folds an admin-declared relationship and checks it against
// the closed vocabulary. It is the validate half of recordRelationship, split
// out so the approve arm can refuse a bad value BEFORE anything is written:
// a draft's completion is persisted and audited ahead of the relationship, and
// a refusal that lands after it leaves a completed draft behind a 400.
func parseRelationship(rel string) (string, error) {
	rel = strings.ToLower(strings.TrimSpace(rel))
	if !validRelationships[rel] {
		return "", fmt.Errorf("unknown membership relationship %q — expected constituent, assigned or liaison", rel)
	}
	return rel, nil
}

// recordRelationship writes affiliation.relationship into the proposal's
// manifest at the admin gate.
//
// It is a manifest edit rather than a separate column because affiliation.id IS
// the membership assertion, so the membership's qualifier belongs beside it and
// travels with it into the roster projection unchanged.
func (s *Server) recordRelationship(r *http.Request, id, rel string) error {
	rel, err := parseRelationship(rel)
	if err != nil {
		return err
	}
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if err != nil {
		return fmt.Errorf("could not load proposal: %w", err)
	}
	var m map[string]any
	if err := json.Unmarshal([]byte(p.ManifestJSON), &m); err != nil {
		return fmt.Errorf("proposal manifest is not readable, so the relationship cannot be recorded: %w", err)
	}
	aff, _ := m["affiliation"].(map[string]any)
	if aff == nil {
		aff = map[string]any{}
	}
	aff["relationship"] = rel
	m["affiliation"] = aff
	out, err := json.Marshal(m)
	if err != nil {
		return fmt.Errorf("could not re-encode the manifest: %w", err)
	}
	return s.Onboard.UpdateProposalManifest(r.Context(), id, string(out))
}

func (s *Server) approvalCeiling(r *http.Request, p *onboardstore.Proposal, decidedBy string) *decideError {
	// Rule 2 first: it needs no tier data and applies to every principal.
	if e := strings.TrimSpace(strings.ToLower(decidedBy)); e != "" &&
		e == strings.TrimSpace(strings.ToLower(p.Email)) {
		return &decideError{status: http.StatusForbidden,
			msg: "you cannot approve your own onboarding — a request is decided by a different admin, the same rule the operator-key queue already applies"}
	}

	tier := strings.TrimSpace(p.Tier)
	if tier == "" {
		return nil // no tier to compare; the manifest lint governs completeness
	}
	hints, err := loadIntakeHints()
	if err != nil {
		// Fail CLOSED. Without the tier table there is no ceiling, and "no
		// ceiling" is exactly the state this function exists to end.
		return &decideError{status: http.StatusServiceUnavailable,
			msg: "onboarding tier definitions unavailable — refusing to decide without a ceiling to check against"}
	}

	// A SPONSOR's ceiling is their sponsorship annotation, not their groups —
	// a sponsor is not a console admin and holds none. ADR 1071 already caps
	// this value at invite MINT (onboarding.go:632); it was never applied to
	// the sponsor's own vouch on the web path, which is the second half of the
	// operator's rule: sponsors sponsor up to their own level.
	//
	// An empty MaxTier means operator-minted sponsorship, i.e. unrestricted —
	// the same reading the mint path already uses, so this changes no existing
	// sponsor's reach.
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		if sp.MaxTier != "" && !hints.tierWithin(tier, sp.MaxTier) {
			return &decideError{status: http.StatusForbidden,
				msg: fmt.Sprintf("your sponsorship allows up to %q and this proposal is %q — a sponsor may vouch up to their own level, not above it", sp.MaxTier, tier)}
		}
		return nil
	}

	actor := actorFromCtx(r.Context())
	if len(actor.Groups) == 0 {
		// No sponsor session and no browser groups: an operator/service
		// principal that already passed requireOperatorPrincipal. Top of the
		// ladder, so the ceiling is vacuous — but the self rule above applied.
		return nil
	}
	own, ownRank, ok := hints.actorTier(actor.Groups)
	if !ok {
		return &decideError{status: http.StatusForbidden,
			msg: "your groups match no onboarding tier, so there is no ceiling to decide within — ask a platform admin to decide this one"}
	}
	askRank, ok := hints.tierRank(tier)
	if !ok {
		return &decideError{status: http.StatusForbidden,
			msg: fmt.Sprintf("proposal names tier %q, which is not in the tier table — refusing rather than approving an unrankable grant", tier)}
	}
	if askRank > ownRank {
		return &decideError{status: http.StatusForbidden,
			msg: fmt.Sprintf("you hold %q and this proposal grants %q — an admin may approve up to their own tier, not above it", own, tier)}
	}
	return nil
}

// decideProposal applies action to proposal id on behalf of decidedBy.
// relationship is the ADR 1171 membership relationship the admin declared at
// the approve gate; blank on every other action and on the machine realm.
func (s *Server) decideProposal(r *http.Request, id, action, decidedBy, note, relationship string) (string, error) {
	// Bound the free-text note before it can reach the store (any realm).
	note = truncateNote(note)
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		return "", &decideError{status: http.StatusNotFound, msg: "proposal not found"}
	}
	if err != nil {
		return "", &decideError{status: http.StatusInternalServerError, msg: "could not load proposal"}
	}
	// An incomplete draft (manifest never assembled) may be rejected, but never
	// advanced as-is: approving one lets the runner claim it, then the native
	// mint hard-fails on the unusable manifest with a misleading error. The
	// two advancing arms below try to complete it under the current rules
	// (onboard_draft.go) once their status check has passed, so a decision
	// that is refused anyway never writes a completion first. Only a draft
	// that still cannot assemble is refused, and the refusal names the gap.
	switch action {
	case "offboard":
		// Offboard marks exit for proposals that have made it through
		// provisioning (onboarded, enrolled, archived): credentials have
		// already been minted and delivered, so this leg turns off the
		// onboarding rail while retaining the proposal row for audit.
		if p.Status == onboardstore.ProposalOffboarded {
			return "", &decideError{status: http.StatusConflict, msg: "this enrollee was already marked offboarded — reload the queue to see current state", code: flashOffboardAlready}
		}
		if !onboardstore.IsOffboardable(p.Status) && p.Status != onboardstore.ProposalOffboardRequested {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q — only a proposal that is %s can be marked offboarded", p.Status, onboardstore.OffboardableProse()), code: flashOffboardState}
		}
		// Archived → offboarded is the one-way transition of the three
		// (RestoreProposal is guarded on archived), so it must carry a
		// reason. The store trims the note and keeps the existing one when
		// it is blank, which is right for onboarded/enrolled but would let
		// an archived row close for good under its old archive reason.
		// A queued cut the runner never ran is settled through the
		// same door: the admin is asserting they cut the access by hand, so
		// the note must say how, and the login-plane precheck has already
		// refused if the person's login is still active.
		if p.Status == onboardstore.ProposalArchived && strings.TrimSpace(note) == "" {
			return "", &decideError{status: http.StatusBadRequest, msg: offboardNoteRequiredMsg, code: flashOffboardNoteRequired}
		}
		if p.Status == onboardstore.ProposalOffboardRequested && strings.TrimSpace(note) == "" {
			return "", &decideError{status: http.StatusBadRequest, msg: settleNoteRequiredMsg, code: flashSettleNoteRequired}
		}
		ok, err := s.Onboard.MarkOffboardedFrom(r.Context(), id, decidedBy, note)
		if err != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not record offboard", code: flashOffboardFailed}
		}
		if ok {
			return onboardstore.ProposalOffboarded, nil
		}
		// Zero rows matched: a concurrent transition moved the row out of the
		// offboardable set, or the id no longer exists. Re-read so the caller
		// sees the same conflicts the pre-update checks enforce.
		p2, gerr := s.Onboard.GetProposal(r.Context(), id)
		if errors.Is(gerr, onboardstore.ErrNotFound) {
			return "", &decideError{status: http.StatusNotFound, msg: "proposal not found"}
		}
		if gerr != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not load proposal"}
		}
		if p2.Status == onboardstore.ProposalOffboarded {
			return "", &decideError{status: http.StatusConflict, msg: "this enrollee was already marked offboarded — reload the queue to see current state", code: flashOffboardAlready}
		}
		if !onboardstore.IsOffboardable(p2.Status) {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q — only a proposal that is %s can be marked offboarded", p2.Status, onboardstore.OffboardableProse()), code: flashOffboardState}
		}
		// The store also refuses the terminal transition while a reissue or
		// credential-producing notification is still running.
		return "", &decideError{status: http.StatusConflict, msg: "a key reissue or notification is still running — wait for its result before marking offboarded", code: flashOffboardState}
	case "retry":
		// Re-run the MINT for a proposal whose provisioning failed. Both human
		// gates were passed to reach `failed`, so nothing is re-decided here —
		// the row returns to `approved` and the runner claims it again. The
		// alternative is a hand-written UPDATE against the console's SQLite file,
		// which is exactly the out-of-band surgery onboarding-rails.md forbids.
		if p.Status != onboardstore.ProposalFailed {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q, not failed — only a failed mint can be retried", p.Status)}
		}
		if err := s.Onboard.RetryProposal(r.Context(), id); err != nil {
			if errors.Is(err, onboardstore.ErrInvalidManifest) {
				return "", &decideError{status: http.StatusConflict, msg: err.Error(), code: "invalid_manifest"}
			}
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not requeue the proposal for another mint attempt"}
		}
		return onboardstore.ProposalApproved, nil
	case "sponsor-approve":
		if p.Status != onboardstore.ProposalProposed {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q, not awaiting sponsor review", p.Status)}
		}
		// Same two rules as the admin gate. sponsorVouchGate already caps
		// a CAPPED sponsor's tier, but it returns early for operators and for a
		// console admin acting at this stage — so a browser admin vouching here
		// had no ceiling either, and nothing anywhere stopped a person vouching
		// for themselves.
		if derr := s.approvalCeiling(r, p, decidedBy); derr != nil {
			return "", derr
		}
		if derr := s.advanceDraft(r, p); derr != nil {
			return "", derr
		}
		if err := s.Onboard.DecideProposal(r.Context(), id, onboardstore.ProposalSponsorApproved, decidedBy, note); err != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not record sponsor approval"}
		}
		// Stage-2 provenance: decided_by gets overwritten by the admin approve,
		// so persist the sponsor-gate actor separately (best-effort — the
		// approval itself already succeeded).
		if err := s.Onboard.RecordSponsorApproval(r.Context(), id, decidedBy); err != nil {
			slog.Warn("record sponsor-approval provenance failed", "id", id, "err", err)
		}
		s.notifyAdminVouched(r, p, decidedBy)
		return onboardstore.ProposalSponsorApproved, nil
	case "sponsor-reject":
		// A sponsor may decline before vouching, or withdraw a vouch the admin
		// has not yet decided on (ADR 1207). Once the admin decides, the
		// proposal is no longer the sponsor's to change.
		if p.Status != onboardstore.ProposalProposed && p.Status != onboardstore.ProposalSponsorApproved {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q; a sponsor can decline or withdraw only before the admin decides", p.Status)}
		}
		if err := s.Onboard.DecideProposal(r.Context(), id, onboardstore.ProposalRejected, decidedBy, note); err != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not record rejection"}
		}
		s.notifyCandidateRejected(r, p, note)
		return onboardstore.ProposalRejected, nil
	case "approve":
		if p.Status != onboardstore.ProposalSponsorApproved {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q, not sponsor-approved — a sponsor must vouch for it first", p.Status)}
		}
		if derr := s.approvalCeiling(r, p, decidedBy); derr != nil {
			return "", derr
		}
		// The relationship is CHECKED before the draft completes and WRITTEN
		// after. Checked first, because completion is a persisted, audited
		// state change and a value the vocabulary refuses must not leave one
		// behind. Written after, because assembly rebuilds the manifest from
		// the intake record, so a relationship written first would be rebuilt
		// away and the approved row would carry none, which is the inference
		// ADR 1171 exists to end.
		// The organisation decides what may be declared: a governmental org is
		// asked, a commercial or civil-society org implies `assigned`, and
		// `constituent` exists only on a multinational org (relationship_scope.go).
		rel, derr := s.relationshipChoiceFor(*p).resolveRelationship(relationship)
		if derr != nil {
			return "", derr
		}
		if derr := s.advanceDraft(r, p); derr != nil {
			return "", derr
		}
		if rel != "" {
			if err := s.recordRelationship(r, id, rel); err != nil {
				return "", &decideError{status: http.StatusBadRequest, msg: err.Error()}
			}
			if p, err = s.Onboard.GetProposal(r.Context(), id); err != nil {
				return "", &decideError{status: http.StatusInternalServerError, msg: "could not reload proposal after recording the relationship"}
			}
		}
		// The ADMIN gate only. A sponsor vouches for the PERSON; whether their
		// organisation is registered is an admin concern and an admin's click to
		// fix, so refusing it at the sponsor gate would block a stage that cannot
		// act on the refusal.
		if derr := s.requireRegisteredOrg(p); derr != nil {
			return "", derr
		}
		if derr := s.requireAttachedOrg(p); derr != nil {
			return "", derr
		}
		// Re-validate before approving — guards a schema bump between submit and
		// approve. A proposal that no longer lints is refused, not silently passed.
		if lint := onboardLint([]byte(p.ManifestJSON), "json"); !lint.OK {
			return "", &decideError{status: http.StatusBadRequest,
				msg: "proposal no longer passes the onboard lint gate: " + lintFindingsSummary(lint.Findings)}
		}
		if err := s.Onboard.DecideProposal(r.Context(), id, onboardstore.ProposalApproved, decidedBy, note); err != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not record approval"}
		}
		return onboardstore.ProposalApproved, nil
	case "reject":
		// Either pre-mint gate can reject (proposed or sponsor_approved).
		if p.Status != onboardstore.ProposalProposed && p.Status != onboardstore.ProposalSponsorApproved {
			return "", &decideError{status: http.StatusConflict, msg: fmt.Sprintf("proposal is %q — cannot reject (already decided/minted)", p.Status)}
		}
		if err := s.Onboard.DecideProposal(r.Context(), id, onboardstore.ProposalRejected, decidedBy, note); err != nil {
			return "", &decideError{status: http.StatusInternalServerError, msg: "could not record rejection"}
		}
		s.notifyCandidateRejected(r, p, note)
		return onboardstore.ProposalRejected, nil
	default:
		return "", &decideError{status: http.StatusBadRequest, msg: "unknown action: " + action}
	}
}

// offboardPrecheck verifies the assertion the browser's "Mark offboarded"
// button makes — that this person's access was already cut elsewhere. It
// returns a flash code to refuse with, or "" to proceed.
//
// A real revoke (`goat admin onboard --revoke`, phase R2) leaves the Kratos
// identity present but state=inactive, or removes it entirely under --purge.
// So an ACTIVE login for the proposal's email is positive evidence that no cut
// has happened, and the assertion is false. Fail-closed on an unreachable
// login plane: an unverifiable claim is refused rather than recorded, because
// the mark is what releases the duplicate-email guard.
//
// The IdP key mirrors mintRecoveryForEmail's "tenant" — the site realm the
// onboarding rail mints into.
func (s *Server) offboardPrecheck(r *http.Request, id string) string {
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if err != nil || strings.TrimSpace(p.Email) == "" {
		// Nothing to check against; decideProposal's state guard still applies.
		return ""
	}
	// Only proposals eligible for offboard (onboarded, enrolled, archived) can
	// be marked. Deferring every other status to decideProposal keeps its
	// conflict reporting ("already offboarded" / "only a proposal that is
	// <OffboardableProse> can be marked offboarded") intact instead of masking
	// it with a verification refusal, and spares a pointless login-plane read.
	if !onboardstore.IsOffboardable(p.Status) && p.Status != onboardstore.ProposalOffboardRequested {
		return ""
	}
	// No login plane to check against is not "nothing to worry about" — it is
	// an unverifiable claim, and this console administers Kratos for a living,
	// so a missing client means a degraded deployment rather than a goatnet
	// that legitimately has no identities. Refuse; the CLI still cuts.
	cli := s.Kratos["tenant"]
	if cli == nil {
		return flashOffboardUnverified
	}
	ids, lerr := cli.ListIdentities(r.Context())
	if lerr != nil {
		slog.Warn("offboard precheck: list identities failed",
			"proposal", id, "err", lerr)
		return flashOffboardUnverified
	}
	for _, i := range ids {
		if strings.EqualFold(i.Email(), p.Email) && strings.EqualFold(i.State, "active") {
			return flashOffboardLive
		}
	}
	return ""
}

// devicesForHandle reads the NetBird peers and returns the ones this handle
// owns, with their OS. It reuses NOTIFY_MGMT_URL + ENROLL_WATCH_NB_TOKEN, the
// same pair the Access section reads; when either is unset, or the call fails,
// it returns the reason instead, and the card says it could not tell.
func (s *Server) devicesForHandle(ctx context.Context, handle string) (*meshstate.DeviceSummary, string) {
	mgmtURL := strings.TrimSpace(s.Cfg.NotifyMgmtURL)
	token := strings.TrimSpace(s.Cfg.EnrollWatchToken)
	if mgmtURL == "" || token == "" {
		return nil, "mesh device list unavailable: NOTIFY_MGMT_URL or ENROLL_WATCH_NB_TOKEN is unset"
	}
	peers, err := (&meshstate.Client{MgmtURL: mgmtURL, Token: token}).Peers(ctx)
	if err != nil {
		slog.WarnContext(ctx, "onboarding card NetBird peer list failed", slog.String("handle", handle), slog.String("err", err.Error()))
		return nil, "mesh device list unavailable: " + err.Error()
	}
	known, kerr := meshstate.KnownHandles(ctx, s.Onboard, nil)
	if kerr != nil {
		return nil, "mesh device list unavailable: onboarding roster unreadable, so peer names cannot be attributed safely"
	}
	// The management host's own handshake view backs the control-only flag.
	// Unavailable (unset, missing, stale) leaves the flag off: unknown is
	// never shown as a fault.
	hs, herr := meshstate.ReadHandshakes(s.Cfg.PeerHandshakesPath, time.Now())
	if herr != nil {
		slog.DebugContext(ctx, "onboarding card peer handshake summary unavailable", slog.String("err", herr.Error()))
	}
	d := meshstate.DevicesFor(peers, handle, known, hs)
	return &d, ""
}

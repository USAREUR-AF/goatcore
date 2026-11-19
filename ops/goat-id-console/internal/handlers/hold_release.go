package handlers

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/meshstate"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

const incidentCloseout = "Security holds are released only through the ADR 1091 incident close-out path"

type releaseHoldParams struct {
	Site              string   `json:"site"`
	PeerID            string   `json:"peer_id"`
	PeerName          string   `json:"peer_name"`
	HoldEnteredAt     string   `json:"hold_entered_at"`
	HoldGroup         string   `json:"hold_group"`
	PriorGroups       []string `json:"prior_groups"`
	RequestedByHandle string   `json:"requested_by_handle"`
	StepUpVerified    bool     `json:"step_up_verified"`
}

type releaseHoldPreview struct {
	Record             meshstate.HoldRecord
	Authority          membershipAuthority
	OpenRequest        bool
	AlreadyReleased    bool
	HoldStarted        string
	PriorGroupsDisplay string
	ReturnTo           string
	Error              string
}

// sameGoatnet compares canonical site slugs exactly. The runner and lifecycle
// engine filter case-sensitively, so accepting a case variant in the console
// would enqueue work the site runner must refuse.
func sameGoatnet(a, b string) bool {
	return strings.TrimSpace(a) == strings.TrimSpace(b)
}

func equalGroupSet(got, want []string) bool {
	norm := func(in []string) []string {
		out := make([]string, 0, len(in))
		for _, g := range in {
			g = strings.TrimSpace(g)
			if g == "" || strings.EqualFold(g, "All") {
				continue
			}
			out = append(out, strings.ToLower(g))
		}
		sort.Strings(out)
		return out
	}
	a, b := norm(got), norm(want)
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// validateReleaseHold is the common GET-preview and POST-enqueue gate. It
// re-reads authority, record, group vocabulary and peers every time, so a
// direct endpoint call cannot bypass a hidden button or act on a stale preview.
func (s *Server) validateReleaseHold(r *http.Request, peerID string, enteredAt time.Time) (releaseHoldPreview, error) {
	p := releaseHoldPreview{Authority: s.membershipAuthority(), ReturnTo: "/holds"}
	if !p.Authority.AllowsLiveRelease() {
		if p.Authority.Mode == authorityEnforce {
			return p, fmt.Errorf("%s", p.Authority.Detail)
		}
		return p, fmt.Errorf("membership authority is unknown; live release is disabled: %s", p.Authority.Detail)
	}
	records, err := meshstate.LoadHoldRecords(strings.TrimSpace(s.Cfg.HoldRecordsPath))
	if err != nil {
		return p, fmt.Errorf("hold records unavailable: %w", err)
	}
	if records.DuplicatePeerIDs[peerID] {
		return p, errors.New("peer identity is ambiguous: duplicate hold records")
	}
	var matches []meshstate.HoldRecord
	var otherSite bool
	for _, rec := range records.All {
		if rec.PeerID != peerID || !rec.EnteredAt.Equal(enteredAt) {
			continue
		}
		if !sameGoatnet(rec.Site, s.Cfg.NotifyGoatnet) {
			otherSite = true
			continue
		}
		matches = append(matches, rec)
	}
	if len(matches) == 0 {
		if otherSite {
			return p, errors.New("the hold record belongs to a different goatnet")
		}
		return p, errors.New("no matching dormancy hold record")
	}
	if len(matches) != 1 {
		return p, errors.New("peer identity is ambiguous: more than one matching hold record")
	}
	p.Record = matches[0]
	if p.Record.Lane == meshstate.HoldKindSecurity {
		return p, errors.New(incidentCloseout) //nolint:staticcheck // User-facing sentence is intentionally capitalized.
	}
	if p.Record.Lane != meshstate.HoldKindDormancy {
		return p, errors.New("no matching dormancy hold record")
	}
	if s.Cfg.NotifyMgmtURL == "" || s.Cfg.EnrollWatchToken == "" {
		return p, errors.New("live peer is unavailable")
	}
	cli := &meshstate.Client{MgmtURL: s.Cfg.NotifyMgmtURL, Token: s.Cfg.EnrollWatchToken}
	peers, err := cli.Peers(r.Context())
	if err != nil {
		return p, fmt.Errorf("live peer is unavailable: %w", err)
	}
	var found []meshstate.Peer
	for _, peer := range peers {
		if peer.ID == peerID {
			found = append(found, peer)
		}
	}
	if len(found) == 0 {
		return p, errors.New("live peer is unavailable")
	}
	if len(found) != 1 {
		return p, errors.New("peer identity is ambiguous in live mesh state")
	}
	live := make([]string, 0, len(found[0].Groups))
	for _, g := range found[0].Groups {
		live = append(live, g.Name)
	}
	allowed := append([]string{p.Record.HoldGroup}, p.Record.PriorGroups...)
	if p.Record.Phase == "releasing" {
		for _, g := range live {
			if strings.EqualFold(g, "All") {
				continue
			}
			ok := false
			for _, a := range allowed {
				if strings.EqualFold(g, a) {
					ok = true
					break
				}
			}
			if !ok {
				return p, fmt.Errorf("live state is inconsistent: unexpected group %s", g)
			}
		}
	} else if equalGroupSet(live, []string{p.Record.HoldGroup}) {
		// clean held state
	} else if equalGroupSet(live, p.Record.PriorGroups) {
		p.AlreadyReleased = true
	} else {
		return p, errors.New("live state is inconsistent with the hold record")
	}
	groups, err := cli.Groups(r.Context())
	if err != nil {
		return p, fmt.Errorf("group vocabulary unavailable: %w", err)
	}
	known := map[string]bool{}
	for _, g := range groups {
		known[strings.ToLower(strings.TrimSpace(g.Name))] = true
	}
	for _, g := range p.Record.PriorGroups {
		if !known[strings.ToLower(strings.TrimSpace(g))] {
			return p, fmt.Errorf("recorded destination group no longer exists: %s", g)
		}
	}
	if s.Onboard == nil {
		return p, errors.New("reactivation request store is unavailable")
	}
	p.OpenRequest, err = s.Onboard.HasOpenReactivationRequest(r.Context(), peerID, enteredAt)
	if err != nil {
		return p, fmt.Errorf("reactivation request lookup unavailable: %w", err)
	}
	p.HoldStarted = p.Record.EnteredAt.UTC().Format("2006-01-02 15:04:05 UTC")
	if len(p.Record.PriorGroups) == 0 {
		p.PriorGroupsDisplay = "empty list (no groups)"
	} else {
		p.PriorGroupsDisplay = strings.Join(p.Record.PriorGroups, ", ")
	}
	return p, nil
}

func releaseEnteredAt(r *http.Request) (time.Time, error) {
	raw := r.FormValue("entered_at")
	sec, err := strconv.ParseInt(raw, 10, 64)
	if err != nil || sec <= 0 {
		return time.Time{}, errors.New("invalid hold identity")
	}
	return time.Unix(sec, 0).UTC(), nil
}

func (s *Server) handleReleaseHoldPreview(w http.ResponseWriter, r *http.Request) {
	entered, err := releaseEnteredAt(r)
	if err != nil {
		s.renderError(w, r, http.StatusBadRequest, err.Error())
		return
	}
	p, err := s.validateReleaseHold(r, r.PathValue("peer"), entered)
	if err != nil {
		p.Error = err.Error()
	}
	s.render(w, r, "release_hold", &pageVars{Title: "Release dormancy hold", ReleaseHold: &p})
}

func (s *Server) handleReleaseHoldConfirm(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, http.StatusServiceUnavailable, "request store is not configured")
		return
	}
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	entered, err := releaseEnteredAt(r)
	if err != nil {
		s.renderError(w, r, http.StatusBadRequest, err.Error())
		return
	}
	p, err := s.validateReleaseHold(r, r.PathValue("peer"), entered)
	actor := actorFromCtx(r.Context())
	if err != nil {
		s.emitAudit(r, actor, "release_hold", nil, s.Cfg.NotifyGoatnet, nil, map[string]interface{}{"peer_id": r.PathValue("peer"), "hold_entered_at": entered.Format(time.RFC3339)}, false, "refused", err.Error())
		s.renderError(w, r, http.StatusConflict, err.Error())
		return
	}
	step := s.requireStepUp(w, r, actor, "release_hold")
	if step.Handled {
		s.emitAudit(r, actor, "release_hold", nil, p.Record.Site, nil, nil, false, "refused", stepUpChallengeAudit)
		return
	}
	// The operator-action actor comes ONLY from the authenticated session.
	// It used to be handleForLoginEmail, the newest onboarding record carrying
	// the admin's email, which an intake key could plant with any handle and
	// so attribute this release to someone else (GHSA-3m4r review).
	actorHandle := sessionOperatorActor(actor)
	if actorHandle == "" {
		s.emitAudit(r, actor, "release_hold", nil, p.Record.Site, nil, nil, step.Verified, "refused", "session carries no identity id")
		s.renderError(w, r, http.StatusConflict, "Your session carries no identity id, so the fabric audit record cannot be attributed safely.")
		return
	}
	params := releaseHoldParams{Site: p.Record.Site, PeerID: p.Record.PeerID, PeerName: p.Record.PeerName,
		HoldEnteredAt: p.Record.EnteredAt.UTC().Format(time.RFC3339), HoldGroup: p.Record.HoldGroup,
		PriorGroups: append([]string(nil), p.Record.PriorGroups...), RequestedByHandle: actorHandle, StepUpVerified: step.Verified}
	raw, err := json.Marshal(params)
	if err != nil {
		s.emitAudit(r, actor, "release_hold", nil, p.Record.Site, nil, nil, step.Verified, "error", "release scope could not be encoded")
		s.renderError(w, r, http.StatusInternalServerError, "Release scope could not be encoded.")
		return
	}
	a, err := s.Onboard.EnqueueHoldAction(r.Context(), p.Record.Site, p.Record.PeerID, p.Record.EnteredAt, actorHandle, actor.Email, string(raw))
	if err != nil {
		outcome := "error"
		if errors.Is(err, onboardstore.ErrActionPending) {
			outcome = "refused"
		}
		s.emitAudit(r, actor, "release_hold", nil, p.Record.Site, nil, map[string]interface{}{"peer_id": p.Record.PeerID, "hold_entered_at": params.HoldEnteredAt, "prior_groups": p.Record.PriorGroups}, step.Verified, outcome, err.Error())
		s.renderError(w, r, http.StatusConflict, err.Error())
		return
	}
	s.emitAudit(r, actor, "release_hold", nil, p.Record.Site, nil, map[string]interface{}{"action_id": a.ID, "peer_id": p.Record.PeerID, "hold_entered_at": params.HoldEnteredAt, "prior_groups": p.Record.PriorGroups, "state": "queued"}, step.Verified, "ok", "")
	q := url.Values{"release": []string{"queued"}}
	http.Redirect(w, r, "/holds?"+q.Encode(), http.StatusSeeOther)
}

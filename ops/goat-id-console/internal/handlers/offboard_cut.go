package handlers

import (
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// handleOffboardCut is the Offboard button that does the REAL cut (goat#372):
// it queues the row as offboard-requested for the onboard-runner, which
// executes the reversible native revoke and writes the result back. No access
// changes in this handler — its whole job is an attributed, step-up-gated,
// double-click-safe queue entry. Contrast the "offboard" decide action, which
// asserts the cut already happened elsewhere (and prechecks that assertion).
func (s *Server) handleOffboardCut(w http.ResponseWriter, r *http.Request, id string, actor Actor) {
	var auditBefore map[string]interface{}
	if p0, err := s.Onboard.GetProposal(r.Context(), id); err == nil {
		auditBefore = map[string]interface{}{"status": p0.Status}
		// Admin tier ceiling (GHSA-vr6w): nobody queues the access cut of a
		// person above their own tier.
		if s.personAboveTierVr6w(r, p0) {
			s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
				auditBefore, nil, false, "refused", tierCeilingRefusal)
			s.renderError(w, r, 403, tierCeilingRefusal)
			return
		}
	}

	// The departure (ADR 1272): why the person left and when. Refused before
	// the step-up and before anything is queued, so no cut exists without it.
	departure := onboardstore.Departure{
		Class:     r.FormValue("reason_class"),
		Text:      r.FormValue("reason_text"),
		Effective: r.FormValue("effective"),
	}
	// A re-queue after a failed cut is the same departure: the recorded
	// reason stands unless the form gives a new one.
	if p0, err := s.Onboard.GetProposal(r.Context(), id); err == nil &&
		p0.Status == onboardstore.ProposalOffboardFailed && strings.TrimSpace(departure.Class) == "" {
		departure = onboardstore.Departure{Class: p0.OffboardReasonClass,
			Text: p0.OffboardReasonText, Effective: p0.OffboardEffective}
	}
	if strings.TrimSpace(departure.Effective) == "" {
		departure.Effective = time.Now().UTC().Format("2006-01-02")
	}
	if err := departure.Validate(); err != nil {
		s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
			auditBefore, nil, false, "refused", err.Error())
		s.renderError(w, r, http.StatusBadRequest, err.Error())
		return
	}

	stepUpVerified := false
	if s.stepUpRequired("offboard_identity") {
		out := s.requireStepUp(w, r, actor, "offboard_identity")
		if out.Handled {
			s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
				auditBefore, nil, false, "refused", stepUpChallengeAudit)
			return
		}
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}

	redirect := func(code string) {
		q := url.Values{}
		q.Set("flash", code)
		http.Redirect(w, r, "/onboarding?"+q.Encode(), http.StatusSeeOther)
	}

	// The approver's identity comes ONLY from the authenticated session (the
	// OIDC sub, a Kratos identity id), never from a lookup by email: any
	// onboarding record can carry an admin's email, so a lookup could be
	// steered to another person's handle (GHSA-3m4r review). The runner names
	// this identity in the admin.offboard operator-action record. A session
	// with no identity id still cuts access; the runner then records the cut
	// under its own name and says the approver identity is missing.
	approverIdentity := strings.ToLower(actor.IdentityID)
	ok, err := s.Onboard.RequestOffboardDeparture(r.Context(), id, actor.Email, approverIdentity, departure)
	if err != nil {
		s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
			auditBefore, nil, stepUpVerified, "error", err.Error())
		redirect(flashOffboardCutFailed)
		return
	}
	if !ok {
		// Unknown id, a pre-provisioning row, or a cut already queued/running
		// (the two-admins double-click) — refused loudly, nothing moved.
		s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
			auditBefore, nil, stepUpVerified, "refused", flashOffboardCutState)
		redirect(flashOffboardCutState)
		return
	}
	s.emitAuditWithInviteToken(r, actor, "onboard_offboard_cut", id, "",
		auditBefore, map[string]interface{}{"status": onboardstore.ProposalOffboardRequested, "approver_identity": approverIdentity,
			"reason_class": departure.Class, "effective": departure.Effective},
		stepUpVerified, "ok", "")
	redirect(flashOffboardCutQueued)
}

// sessionOperatorActor is the operator-action actor for the signed-in person,
// "kratos-<identity id>", derived only from the authenticated session's
// identity id (the OIDC sub that oauth2-proxy forwards). It is a kebab handle,
// as `goat audit emit --actor` requires. "" when the session has no identity
// id. Never derive it from an email lookup: any onboarding record can carry
// an admin's email.
func sessionOperatorActor(a Actor) string {
	id := strings.TrimSpace(a.IdentityID)
	if !identityUUIDRe.MatchString(id) {
		return ""
	}
	return "kratos-" + strings.ToLower(id)
}

package handlers

// Shared Ory group-change core. Both surfaces that can flip a tenant identity's
// login groups — the session-authenticated web UI (POST /users/{id}/groups) and
// the operator-key machine endpoint (POST /api/users/groups, driven by
// `goat admin`) — funnel through applyGroupChange. That is the whole point: the
// vocabulary fence, last-admin guard, step-up policy, the Kratos patch, the
// operator audit, and the best-effort role-change email live in ONE place, so
// the CLI and the web UI can never drift in what they enforce, audit, or email.
// See docs/design/onboarding-notifications.md §"CLI/web parity".

import (
	"errors"
	"log/slog"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

// groupChangeError carries an HTTP status alongside the message so each caller
// renders it in its own idiom (web → renderError, machine → onboardError).
type groupChangeError struct {
	status int
	msg    string
}

func (e *groupChangeError) Error() string { return e.msg }

// errStepUpHandled signals that the step-up gate already wrote the response (a
// redirect into the re-authentication flow). Callers must return immediately
// and render nothing — it is not an error to surface to the operator.
var errStepUpHandled = errors.New("step-up challenge issued; response already written")

// groupChangeOpts tunes the two surfaces that share applyGroupChange.
type groupChangeOpts struct {
	// enforceStepUp asks for an AAL2 session check on an admin grant/revoke.
	// The web UI sets it (its actor carries a browser-session AAL). The
	// operator-key machine path leaves it false ON PURPOSE: the operator key is
	// itself the high-trust authorization (ADR 1071 — operator keys are
	// uncapped), and there is no browser session to step up. The audit still
	// records stepUp=false with the operator handle as actor, so a
	// platform-admin grant via `goat admin` is fully attributed, not silent.
	enforceStepUp bool
	// revokeSessions force-logs-out the identity after the change (used when a
	// reduction should not leave live sessions holding the old claims).
	revokeSessions bool
}

// applyGroupChange is THE single implementation of an Ory login-group patch.
// It returns the patched identity, or a *groupChangeError carrying the HTTP
// status the caller should surface. Every exit path (refusal, error, success)
// emits the same `patch_groups` audit shape as before; a successful change
// fires the role-change email best-effort.
//
// w is used ONLY by the step-up gate, which may take over the response to
// redirect a browser operator into re-authentication. When it does,
// applyGroupChange returns errStepUpHandled and the caller must return without
// writing anything further. The operator-key machine path never triggers that
// leg (enforceStepUp is false there), so it can pass its ResponseWriter safely.
func (s *Server) applyGroupChange(w http.ResponseWriter, r *http.Request, idpKey, id string, newGroups []string, actor Actor, opts groupChangeOpts) (*kratos.Identity, error) {
	ctx := r.Context()
	cli := s.Kratos[idpKey]
	if cli == nil {
		return nil, &groupChangeError{http.StatusServiceUnavailable, "realm " + idpKey + " is unavailable"}
	}
	before, err := cli.GetIdentity(ctx, id)
	if err != nil {
		return nil, &groupChangeError{http.StatusBadGateway, "Kratos get failed: " + err.Error()}
	}
	beforeGroups := map[string]interface{}{"groups": before.Groups()}
	newAfter := func() map[string]interface{} { return map[string]interface{}{"groups": newGroups} }

	// Admin tier ceiling (GHSA-vr6w), on every path. The browser session is
	// held to its own tier; a per-person operator key may not touch the
	// platform tier (tier_ceiling.go); a service key is uncapped (ADR 1071).
	ceilMsg := ""
	if opts.enforceStepUp {
		beforeMeta, _ := metadataGroupsVr6w(before)
		ceilMsg = groupChangeCeiling(actor, id, beforeMeta, targetTierRankVr6w(before), newGroups)
	} else if perPersonOperatorKeyVr6w(r) && (targetTierRankVr6w(before) >= platformTier || adminTierRank(newGroups) >= platformTier) {
		ceilMsg = perPersonTierRefusal
	} else if adminTierRank(newGroups) >= platformTier && metadataRankBelowPlatformVr6w(before) {
		// ADR 1071 as amended: no service key grants the platform tier.
		ceilMsg = operatorKeyPlatformRefusalVr6w
	}
	if ceilMsg != "" {
		s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), false, "refused", ceilMsg)
		return nil, &groupChangeError{http.StatusForbidden, ceilMsg}
	}

	// Vocabulary fence (tenant realm): refuse mesh-role names + unknown groups
	// before they land in Kratos, where the schema would silently accept them.
	if idpKey == tenantIdPKey {
		if err := s.validateGroupVocabulary(newGroups); err != nil {
			s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), false, "refused", err.Error())
			return nil, &groupChangeError{http.StatusBadRequest, err.Error()}
		}
	}

	// wasAdmin/willBeAdmin are platform-admin (the last-admin guard's
	// subject). The step-up decision covers every admin-bearing group,
	// IAM-ADMIN included: IAM-ADMIN passes adminGate.
	wasAdmin := before.HasGroup(platformAdminGroup) || before.HasGroup(legacyAdminGroup)
	willBeAdmin := isAdminGroupList(newGroups)
	// The delta is taken against the admin-managed groups (an unmigrated
	// identity has none, so any admin group is a grant) and against the
	// groups the identity presents, and either one counts.
	beforeMeta, _ := targetMetadataGroups7788(before)
	gMeta, rMeta := s.adminBearingDelta(beforeMeta, newGroups)
	gShown, rShown := s.adminBearingDelta(before.Groups(), newGroups)
	granted, removed := gMeta || gShown, rMeta || rShown

	// Browser session: nobody grants themselves an admin-bearing group
	// (another admin must), and nobody grants, revokes or edits groups at an
	// admin tier above their own. Operator-key path: a per-person key never
	// grants or revokes an admin-bearing group and never edits an identity
	// holding one (admin_operator_key.go); a service key is uncapped (ADR
	// 1071).
	msg := ""
	if opts.enforceStepUp {
		switch {
		case granted && actorIsTarget(actor, before):
			msg = "refused: you cannot grant an admin group to your own identity; another admin must make this change"
		case s.adminChangeAboveTierRank7788(actor, s.targetAdminRank7788(before), newGroups):
			msg = adminTierRefusal
		}
	} else if perPersonOperatorKey7788(r) && (granted || removed || s.targetAdminBearing7788(before)) {
		msg = perPersonAdminRefusal
	} else if grantsPlatformTier7788(beforeMeta, newGroups) {
		// Service keys (ADR 1071, as amended for this release) keep their
		// scope except this one: no service key grants the platform tier
		// through the API. The runner never uses this route; removing the
		// platform tier (offboarding) is unchanged.
		msg = servicePlatformGrantRefusal7788
	}
	if msg != "" {
		s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), false, "refused", msg)
		return nil, &groupChangeError{http.StatusForbidden, msg}
	}

	stepUpAction := ""
	switch {
	case granted:
		stepUpAction = "grant_ADMINS"
	case removed:
		stepUpAction = "revoke_ADMINS"
	}
	// stepUpVerified must claim ONLY what actually happened: a fresh second
	// factor was presented and verified for THIS action.
	//
	// It was previously `opts.enforceStepUp && stepUpAction != ""` — which
	// consults whether the gate RAN, never whether it PASSED. Under
	// STEP_UP_ENFORCED=false that audited step_up_verified:true on every
	// web-UI admin grant/revoke while no second factor existed anywhere on the
	// goatnet (#174 Story C; the same falsehood the reset_2fa path already
	// avoids). Now it is the gate's own verdict.
	stepUpVerified := false
	if opts.enforceStepUp && stepUpAction != "" && s.stepUpRequired(stepUpAction) {
		out := s.requireStepUp(w, r, actor, stepUpAction)
		if out.Handled {
			s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), false, "refused", stepUpChallengeAudit)
			return nil, errStepUpHandled
		}
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}

	// Last-admin guard.
	if err := s.guardLastAdmin(ctx, idpKey, id, wasAdmin, willBeAdmin); err != nil {
		s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), stepUpVerified, "refused", err.Error())
		return nil, &groupChangeError{http.StatusConflict, err.Error()}
	}

	after, err := cli.PatchGroups(ctx, id, newGroups)
	if err != nil {
		s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups, newAfter(), stepUpVerified, "error", err.Error())
		return nil, &groupChangeError{http.StatusBadGateway, "Kratos patch failed: " + err.Error()}
	}

	if opts.revokeSessions {
		if err := cli.RevokeIdentitySessions(ctx, id); err != nil {
			// The Kratos patch above already landed, but the identity's live
			// sessions still carry the OLD groups (e.g. a demoted admin keeps
			// ADMINS in X-Forwarded-Groups until logout). Reporting this as
			// "ok" would hide exactly the condition revokeSessions exists to
			// prevent, so audit it as an error with revoked_sessions:false and
			// surface it the way every other Kratos failure here does (cf.
			// handleUserRevokeSessions). The operator retries the revoke; the
			// group change itself does not need to be repeated.
			slog.Warn("revoke sessions after group change failed", slog.String("id", id), slog.String("err", err.Error()))
			msg := "groups were changed, but revoking the identity's live sessions failed: " + err.Error() +
				". Existing sessions still carry the previous groups — revoke sessions for this identity and verify."
			s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups,
				map[string]interface{}{"groups": newGroups, "revoked_sessions": false}, stepUpVerified, "error", msg)
			return nil, &groupChangeError{http.StatusBadGateway, msg}
		}
	}

	s.emitAudit(r, actor, "patch_groups", before, idpKey, beforeGroups,
		map[string]interface{}{"groups": newGroups, "revoked_sessions": opts.revokeSessions}, stepUpVerified, "ok", "")

	// Role-change email (best-effort): the person whose access changed gets an
	// upgrade-framed, complete reference for their new role — never a
	// new-provisioning welcome, never silence. No-op on an effective no-change.
	s.sendAccessChangeEmail(r, idpKey, before, newGroups)

	return after, nil
}

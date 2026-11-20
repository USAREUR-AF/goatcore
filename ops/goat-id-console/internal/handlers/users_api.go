package handlers

// Machine-realm user mutation endpoints — the operator-key siblings of the
// admin-gated web routes POST /users/{id}/groups and POST /users/{id}/state.
// They exist so `goat admin` (and the operator-onboard playbook) can change a
// person's login groups / activation state through the SAME console logic the
// web UI uses — one audit shape, one last-admin guard, one role-change email —
// instead of a parallel Kratos reconcile that skips all three. Operator-key
// only (onboardAPIGate → IsOperator); a sponsorship-capped participant key may
// read the queue but never grant groups.

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// resolveTenantIdentity looks up a tenant-realm identity by Kratos id (if given)
// else by email (case-insensitive, the dedup key onboarding uses everywhere).
func (s *Server) resolveTenantIdentity(ctx context.Context, id, email string) (string, *kratos.Identity, *groupChangeError) {
	cli := s.Kratos[tenantIdPKey]
	if cli == nil {
		return "", nil, &groupChangeError{http.StatusServiceUnavailable, "tenant realm unavailable"}
	}
	id = strings.TrimSpace(id)
	email = strings.TrimSpace(email)
	if id != "" {
		ident, err := cli.GetIdentity(ctx, id)
		if err != nil {
			return "", nil, &groupChangeError{http.StatusNotFound, "no tenant identity with that id"}
		}
		return id, ident, nil
	}
	if email == "" {
		return "", nil, &groupChangeError{http.StatusBadRequest, `body must set "email" or "id"`}
	}
	ids, err := cli.ListIdentities(ctx)
	if err != nil {
		return "", nil, &groupChangeError{http.StatusBadGateway, "list identities failed"}
	}
	for i := range ids {
		if strings.EqualFold(strings.TrimSpace(ids[i].Email()), email) {
			return ids[i].ID, &ids[i], nil
		}
	}
	return "", nil, &groupChangeError{http.StatusNotFound, "no tenant identity with that email"}
}

// cleanGroups trims and drops empties from a JSON group array.
func cleanGroups(gs []string) []string {
	out := make([]string, 0, len(gs))
	for _, g := range gs {
		if t := strings.TrimSpace(g); t != "" {
			out = append(out, t)
		}
	}
	return out
}

// codeForStatus maps an HTTP status to the short machine error code the
// onboard-API JSON envelope carries.
func codeForStatus(status int) string {
	switch status {
	case http.StatusBadRequest:
		return "bad_request"
	case http.StatusForbidden:
		return "forbidden"
	case http.StatusNotFound:
		return "not_found"
	case http.StatusConflict:
		return "conflict"
	case http.StatusServiceUnavailable:
		return "not_configured"
	default:
		return "error"
	}
}

// principalOwnsIdentity reports whether the operator principal behind this
// request IS the person the request is about — by login email, or by the
// onboarding handle that email resolves to (operator keys are named by handle,
// so both spellings have to be compared).
//
// Unresolvable cases answer false: a service key (onboard-runner) resolves to
// no person, and an admin who predates the onboarding store resolves to no
// handle. Answering true there would refuse legitimate machine work; the
// escalation this guards is a PERSON changing their own groups, and that person
// always has a row to match.
func (s *Server) principalOwnsIdentity(r *http.Request, pr onboardPrincipal, ident *kratos.Identity) bool {
	if ident == nil || strings.TrimSpace(pr.Handle) == "" {
		return false
	}
	email := strings.TrimSpace(ident.Email())
	if email == "" {
		return false
	}
	if strings.EqualFold(email, pr.Handle) || selfcheck.AnySame(identityAddresses(ident), []string{pr.Handle, pr.SponsorEmail}) {
		return true
	}
	if h, err := s.sshKeyHandleFor(r, email); err == nil && strings.TrimSpace(h) != "" && selfcheck.Same(h, pr.Handle) {
		return true
	}
	// The active lookups above skip an offboarded record, which is exactly
	// the record an offboarded key holder has. Ask for the newest record in
	// any status, under the key's handle and under each of the identity's
	// addresses. A store failure answers true: this only ever refuses.
	if s.Onboard != nil {
		addrs := identityAddresses(ident)
		for _, who := range append([]string{pr.Handle}, addrs...) {
			p, ok, err := s.Onboard.LatestProposalFor(r.Context(), who)
			if err != nil {
				return true
			}
			if ok && (selfcheck.Same(p.Handle, pr.Handle) && selfcheck.AnySame(addrs, []string{p.Email})) {
				return true
			}
		}
	}
	return false
}

// apiPatchUserGroups (POST /api/users/groups) — operator-key group change on a
// tenant identity, funneled through applyGroupChange so it is byte-for-byte
// consistent with the web UI (audit + last-admin guard + role-change email).
//
// Body: {"email": "...", "groups": ["IAM-ADMIN", ...], "revoke_sessions": bool}
// or {"id": "<kratos-uuid>", ...}. "groups" is the FULL desired set (send [] to
// clear). No AAL2 step-up: the operator key is the authorization (ADR 1071);
// the change is attributed to the operator handle in the audit.
func (s *Server) apiPatchUserGroups(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	if !pr.IsOperator {
		onboardError(w, http.StatusForbidden, "forbidden", "operator key required to change login groups")
		return
	}
	var req struct {
		Email          string   `json:"email"`
		ID             string   `json:"id"`
		Groups         []string `json:"groups"`
		RevokeSessions bool     `json:"revoke_sessions"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
		return
	}
	if req.Groups == nil {
		onboardError(w, http.StatusBadRequest, "bad_request", `body must set "groups" (use [] to clear all)`)
		return
	}
	id, ident, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email)
	if gErr != nil {
		onboardError(w, gErr.status, codeForStatus(gErr.status), gErr.msg)
		return
	}
	// SELF-CHANGE REFUSED. An operator key authorizes this route on its
	// own — no step-up, no second person — so without this a key holder could
	// add PLATFORM-ADMIN to their own login and the only record would be their
	// own handle on both halves. Every legitimate use of this route is an admin
	// changing SOMEBODY ELSE, so refusing self costs nothing and closes the
	// escalation. The web path is unaffected: it runs under adminGate with a
	// browser session and its own step-up.
	if s.principalOwnsIdentity(r, pr, ident) {
		onboardError(w, http.StatusForbidden, "forbidden",
			"an operator key may not change its own holder's login groups — another admin must make this change")
		return
	}
	actor := Actor{Email: pr.Handle}
	// enforceStepUp:false — the operator key IS the authorization (ADR 1071)
	// and there is no browser to redirect, so the step-up gate never fires on
	// this path and `w` is never taken over by it.
	after, err := s.applyGroupChange(w, r, tenantIdPKey, id, cleanGroups(req.Groups), actor,
		groupChangeOpts{enforceStepUp: false, revokeSessions: req.RevokeSessions})
	if err != nil {
		status := http.StatusBadGateway
		var gce *groupChangeError
		if errors.As(err, &gce) {
			status = gce.status
		}
		onboardError(w, status, codeForStatus(status), err.Error())
		return
	}
	if kerr := s.afterGroupChange(r, actor, tenantIdPKey, id); kerr != nil {
		onboardError(w, http.StatusInternalServerError, "error",
			"the groups changed, but the identity's operator keys could not all be revoked: "+kerr.Error())
		return
	}
	w.WriteHeader(http.StatusOK)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]interface{}{
		"id":     id,
		"email":  after.Email(),
		"groups": after.Groups(),
	})
}

// apiPatchUserState (POST /api/users/state) — operator-key activation toggle,
// the machine sibling of POST /users/{id}/state. Mirrors the web handler's
// sensitive-path guard (deactivating an admin runs the last-admin guard) and
// emits the same patch_state audit. Used for reactivating a returning
// principal before a grant.
//
// Body: {"email"|"id": "...", "state": "active"|"inactive"}.
func (s *Server) apiPatchUserState(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	if !pr.IsOperator {
		onboardError(w, http.StatusForbidden, "forbidden", "operator key required to change identity state")
		return
	}
	var req struct {
		Email string `json:"email"`
		ID    string `json:"id"`
		State string `json:"state"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
		return
	}
	if req.State != "active" && req.State != "inactive" {
		onboardError(w, http.StatusBadRequest, "bad_request", `"state" must be "active" or "inactive"`)
		return
	}
	id, target, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email)
	if gErr != nil {
		onboardError(w, gErr.status, codeForStatus(gErr.status), gErr.msg)
		return
	}
	actor := Actor{Email: pr.Handle}
	before := map[string]interface{}{"state": target.State}
	after := map[string]interface{}{"state": req.State}

	// SELF-REACTIVATION REFUSED, as on the web path: an operator key works
	// without a session or step-up, so its holder switching their own
	// identity back on would undo an offboard with one call. Another admin
	// must do it.
	if req.State == "active" && target.State != "active" && s.reactivatorUnprovenOther(r, pr) {
		msg := "this operator key cannot be tied to a person, so it cannot be shown not to be the identity's own — reactivate with a key that names its holder, or from the console"
		s.emitAudit(r, actor, "patch_state", target, tenantIdPKey, before, after, false, "refused", msg)
		onboardError(w, http.StatusForbidden, "forbidden", msg)
		return
	}
	if req.State == "active" && target.State != "active" && s.principalOwnsIdentity(r, pr, target) {
		msg := "an operator key may not reactivate its own holder's identity — another admin must"
		s.emitAudit(r, actor, "patch_state", target, tenantIdPKey, before, after, false, "refused", msg)
		onboardError(w, http.StatusForbidden, "forbidden", msg)
		return
	}

	// Deactivating an admin identity is sensitive (locks them out) — run the
	// last-admin guard, same as the web handler. No AAL2: operator key is the
	// authorization on the machine path.
	if req.State == "inactive" && (target.HasGroup(platformAdminGroup) || target.HasGroup(legacyAdminGroup)) {
		if err := s.guardLastAdmin(r.Context(), tenantIdPKey, id, true, false); err != nil {
			s.emitAudit(r, actor, "patch_state", target, tenantIdPKey, before, after, false, "refused", err.Error())
			onboardError(w, http.StatusConflict, "conflict", err.Error())
			return
		}
	}

	updated, err := s.Kratos[tenantIdPKey].PatchState(r.Context(), id, req.State)
	if err != nil {
		s.emitAudit(r, actor, "patch_state", target, tenantIdPKey, before, after, false, "error", err.Error())
		onboardError(w, http.StatusBadGateway, "error", "Kratos patch failed: "+err.Error())
		return
	}
	s.emitAudit(r, actor, "patch_state", target, tenantIdPKey, before, after, false, "ok", "")
	forgetIdentity(id)
	if req.State == "inactive" {
		if _, kerr := s.revokeOperatorKeysAtOffboard(r, actor, "api_patch_state", target, "", ""); kerr != nil {
			onboardError(w, http.StatusInternalServerError, "error",
				"the identity is now inactive, but its operator keys could not all be revoked: "+kerr.Error())
			return
		}
	}
	w.WriteHeader(http.StatusOK)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]interface{}{
		"id":    id,
		"email": updated.Email(),
		"state": req.State,
	})
}

// apiCheckReinstatement authorizes and audits the explicit recovery request.
// It grants no SQL access and changes no identity state. The CLI next uses the
// operator's existing privileged SSH path to append the ledger decision, then
// calls the normal state endpoint, which independently verifies persistence.
func (s *Server) apiCheckReinstatement(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	if !pr.IsOperator {
		onboardError(w, http.StatusForbidden, "forbidden", "operator key required for reinstatement")
		return
	}
	var req struct {
		Email  string `json:"email"`
		Env    string `json:"env"`
		Reason string `json:"reason"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
		return
	}
	expectedEnv := strings.TrimSpace(s.Cfg.NotifyGoatnet)
	if expectedEnv == "" {
		onboardError(w, http.StatusServiceUnavailable, "unconfigured", "console goatnet identity is not configured")
		return
	}
	if strings.TrimSpace(req.Env) != expectedEnv {
		onboardError(w, http.StatusConflict, "wrong_environment", "reinstatement must use this console's goatnet")
		return
	}
	req.Reason = strings.TrimSpace(req.Reason)
	if req.Reason == "" || len(req.Reason) > 512 || strings.ContainsAny(req.Reason, "\r\n") {
		onboardError(w, http.StatusBadRequest, "bad_request", "a single-line reason of at most 512 bytes is required")
		return
	}
	id, target, gErr := s.resolveTenantIdentity(r.Context(), "", req.Email)
	if gErr != nil {
		onboardError(w, gErr.status, codeForStatus(gErr.status), gErr.msg)
		return
	}
	s.emitAudit(r, Actor{Email: pr.Handle}, "reinstate_identity_requested", target, tenantIdPKey,
		map[string]interface{}{"state": target.State}, map[string]interface{}{"reason": req.Reason}, false, "requested", "")
	onboardJSON(w, http.StatusOK, map[string]string{"id": id, "actor": pr.Handle, "env": expectedEnv})
}

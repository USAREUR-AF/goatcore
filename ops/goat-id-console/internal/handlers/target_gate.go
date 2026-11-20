package handlers

// One target-identity gate (GHSA-vr6w). Routes() wraps every admin route and
// every machine-realm route in it; it acts on the routes listed in
// browserTargetRoutes and machineTargetRoutes, and passes every other route
// through untouched. A route that acts on a target identity and is missing
// from these tables is a bug: TestTargetGate_coversEveryTargetRoute fails.
//
// For a listed route the gate enforces, before the handler runs:
//   - browser: the session's rank is at least the target's (its own
//     identity, by identity id, is exempt); machine: a per-person operator
//     key does not act on an identity holding the platform tier at all, and
//     a service key is uncapped (ADR 1071);
//   - step-up (browser) before an action that mints or returns a credential
//     or recovery link, when the policy lists it.
// Step-up before the other changes to an admin identity (groups, state,
// delete, revoke sessions) stays in those handlers, after their own self
// and last-admin refusals, so the gate never pre-empts a refusal with a
// step-up redirect.
// Self-elevation is refused by groupChangeCeiling in applyGroupChange, which
// both the browser and machine group routes reach.

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

type targetRoute struct {
	// action names the audit row and the step-up policy entry.
	action string
	// credential: the route mints or returns a credential or a recovery
	// link; step-up when the policy lists action.
	credential bool
}

// browserTargetRoutes are the admin routes whose {id} is a target identity.
var browserTargetRoutes = map[string]targetRoute{
	"POST /users/{id}/groups":            {action: "patch_groups"},
	"POST /users/{id}/revoke-sessions":   {action: "revoke_sessions"},
	"POST /users/{id}/reset-2fa":         {action: "reset_2fa", credential: true},
	"POST /users/{id}/rotate-credential": {action: "rotate_credential", credential: true},
	"POST /users/{id}/delete":            {action: "delete_identity"},
	"POST /users/{id}/recovery-link":     {action: "mint_recovery_link", credential: true},
	"POST /users/{id}/email-reference":   {action: "email_reference"},
	"POST /users/{id}/state":             {action: "patch_state"},
}

type machineRoute struct {
	action string
	fields []string
}

// machineTargetRoutes are the operator-key routes that act on a tenant
// identity named in the JSON body, with the fields each accepts. POST
// /api/users/groups is capped inside applyGroupChange, which parses its body
// once and also sees the requested groups.
var machineTargetRoutes = map[string]machineRoute{
	"POST /api/users/state": {"patch_state", []string{"email", "id", "state"}},
}

// machineUngatedRoutes act on a tenant identity but are capped elsewhere or
// change nothing: POST /api/users/groups is capped in applyGroupChange;
// reinstatement-check only audits a request, and the state change it
// precedes goes through the gate.
var machineUngatedRoutes = map[string]bool{
	"POST /api/users/groups":              true,
	"POST /api/users/reinstatement-check": true,
}

// machineProposalRoutes act on the person an onboarding proposal is for: a
// per-person operator key may not act when that person is at the platform
// tier (personTierRankVr6w: what the proposal grants, or what their tenant
// identity holds). Approvals are capped in approvalCeiling; the proposal
// reads redact credentials in the handlers (redactAboveTierVr6w).
var machineProposalRoutes = map[string]string{
	"POST /api/onboard/proposals/{id}/offboard": "onboard_offboard",
}

// machineActionRoutesVr6w are the follow-up action queue routes (mesh-key
// reissue, hold release). Claiming an action and posting its result write
// the person's setup or handset key and re-send their package, so a
// per-person operator key may not claim or report one whose person holds or
// is granted an admin tier (personTierRankVr6w > 0, the same rule as
// GHSA-7788's per-person cap). A lookup failure refuses. Service keys (the
// runner, ADR 1071) are uncapped; an action with no proposal (a peer hold)
// has no person to rank.
var machineActionRoutesVr6w = map[string]string{
	"POST /api/onboard/actions/{id}/claim":  "onboard_action_claim",
	"POST /api/onboard/actions/{id}/result": "onboard_action_result",
}

const perPersonFollowUpRefusalVr6w = "refused: this follow-up action writes the mesh keys of a person holding an admin tier, which a per-person operator key may not claim or report; the runner's service key runs it"

// machineQueueRoutesVr6w are the proposal lifecycle actions held to the
// admin tier ceiling of an approve (queueCeilingVr6w): restore and retry
// move a row back into the runner's mint queue without an approve; claim
// hands over the manifest to mint; result records the mint and its mesh
// keys; archive is the step before a restore. A per-person operator key may
// not run them on a row that grants an admin tier; a service key (the
// runner, ADR 1071) is uncapped. The browser restore and the decide route's
// retry apply the same ceiling in their handlers.
var machineQueueRoutesVr6w = map[string]string{
	"POST /api/onboard/proposals/{id}/restore": "onboard_restore_proposal",
	"POST /api/onboard/proposals/{id}/retry":   "onboard_retry_proposal",
	"POST /api/onboard/proposals/{id}/claim":   "onboard_claim_proposal",
	"POST /api/onboard/proposals/{id}/result":  "onboard_proposal_result",
	"POST /api/onboard/proposals/{id}/archive": "onboard_archive_proposal",
}

// machineRouteClassified reports whether the machine gate knows pattern.
func machineRouteClassified(pattern string) bool {
	if _, ok := machineTargetRoutes[pattern]; ok {
		return true
	}
	if _, ok := machineProposalRoutes[pattern]; ok {
		return true
	}
	if machineUngatedRoutes[pattern] {
		return true
	}
	if _, ok := machineQueueRoutesVr6w[pattern]; ok {
		return true
	}
	if _, ok := machineActionRoutesVr6w[pattern]; ok {
		return true
	}
	// Every other machine route is listed by name (machine_routes_vr6w.go);
	// nothing is admitted by prefix.
	_, ok := machineRecordRoutesVr6w[pattern]
	return ok
}

const unclassifiedRouteRefusal = "refused: this route is not classified by the target-identity gate (target_gate.go)"

// targetGate is the browser half: wrap an admin-gated handler.
func (s *Server) targetGate(next http.HandlerFunc) http.HandlerFunc {
	s.noteGateWrap("targetGate")
	return func(w http.ResponseWriter, r *http.Request) {
		route, ok := browserTargetRoutes[r.Pattern]
		if !ok {
			// Default deny by name (route_classes_vr6w.go): an admin route
			// that is not a target route passes only when it is listed with
			// a reason, whatever the shape of its path.
			if _, known := routeNotTargetVr6w[r.Pattern]; known {
				next(w, r)
				return
			}
			s.renderError(w, r, 403, unclassifiedRouteRefusal)
			return
		}
		if err := s.validateCSRF(r); err != nil {
			s.renderError(w, r, 400, err.Error())
			return
		}
		idp := idpFromCtx(r.Context())
		cli := s.Kratos[idp.Key]
		if cli == nil {
			s.renderError(w, r, 503, "realm "+idp.Key+" is unavailable")
			return
		}
		target, err := cli.GetIdentity(r.Context(), r.PathValue("id"))
		if err != nil {
			var httpErr *kratos.HTTPError
			if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
				s.renderError(w, r, 404, "identity not found")
				return
			}
			s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
			return
		}
		actor := actorFromCtx(r.Context())
		if targetOutranksActor(actor, target) {
			s.emitAudit(r, actor, route.action, target, idp.Key, nil, nil, false, "refused", tierCeilingRefusal)
			s.renderError(w, r, 403, tierCeilingRefusal)
			return
		}
		// Returning a platform-tier login to service takes a fresh step-up
		// (review F2), whatever the policy says.
		if route.action == "patch_state" && strings.TrimSpace(r.FormValue("state")) == "active" && targetTierRankVr6w(target) >= platformTier && actor.IdentityID == "" {
			s.emitAudit(r, actor, route.action, target, idp.Key, nil, nil, false, "refused", reactivateNoIdentityRefusalVr6w)
			s.renderError(w, r, 403, reactivateNoIdentityRefusalVr6w)
			return
		}
		// Nobody returns their own platform-tier login to service, by
		// identity id (any case) or by the session's address (round 2 F1).
		if route.action == "patch_state" && strings.TrimSpace(r.FormValue("state")) == "active" && targetTierRankVr6w(target) >= platformTier &&
			(actorIsIdentity(actor, target.ID) || strings.EqualFold(strings.TrimSpace(actor.Email), strings.TrimSpace(target.Email()))) {
			s.emitAudit(r, actor, route.action, target, idp.Key, nil, nil, false, "refused", selfReactivationRefusalVr6w)
			s.renderError(w, r, 403, selfReactivationRefusalVr6w)
			return
		}
		if route.action == "patch_state" && strings.TrimSpace(r.FormValue("state")) == "active" && !actorIsIdentity(actor, target.ID) && targetTierRankVr6w(target) >= platformTier {
			if out := s.requireStepUp(w, r, actor, "reactivate_platform_tier"); out.Handled {
				s.emitAudit(r, actor, route.action, target, idp.Key, nil, nil, false, "refused", stepUpChallengeAudit)
				return
			}
		}
		if route.credential && s.stepUpRequired(route.action) {
			if out := s.requireStepUp(w, r, actor, route.action); out.Handled {
				s.emitAudit(r, actor, route.action, target, idp.Key, nil, nil, false, "refused", stepUpChallengeAudit)
				return
			}
		}
		next(w, r)
	}
}

// machineTargetGate is the operator-key half: wrap an onboardAPIGate
// handler. On a listed route it parses the body once, strictly
// (decodeMachineBodyVr6w: no unknown fields, no trailing data), refuses a
// per-person key acting on an identity holding the platform tier, and hands the
// handler a canonical re-encoding of the value it checked, so the gate and
// the handler cannot read different requests.
func (s *Server) machineTargetGate(next http.HandlerFunc) http.HandlerFunc {
	s.noteGateWrap("machineTargetGate")
	return func(w http.ResponseWriter, r *http.Request) {
		route, ok := machineTargetRoutes[r.Pattern]
		if !ok {
			if !machineRouteClassified(r.Pattern) {
				onboardError(w, http.StatusForbidden, "unclassified_route", unclassifiedRouteRefusal)
				return
			}
			if action, ok := machineProposalRoutes[r.Pattern]; ok && s.Onboard != nil {
				pr := onboardPrincipalFromCtx(r.Context())
				if pr.IsOperator && perPersonOperatorKeyVr6w(r) {
					if p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id")); err == nil && s.personTierRankVr6w(r, p, nil) >= platformTier {
						s.emitAuditWithInviteToken(r, Actor{Email: pr.Handle}, action, p.ID, "", nil, nil, false, "refused", perPersonTierRefusal)
						onboardError(w, http.StatusForbidden, "forbidden", perPersonTierRefusal)
						return
					}
				}
			}
			if audit, ok := machineActionRoutesVr6w[r.Pattern]; ok && s.Onboard != nil {
				pr := onboardPrincipalFromCtx(r.Context())
				if pr.IsOperator && perPersonOperatorKeyVr6w(r) {
					a, err := s.Onboard.GetAction(r.Context(), r.PathValue("id"))
					if err != nil && !errors.Is(err, onboardstore.ErrNotFound) {
						onboardError(w, http.StatusServiceUnavailable, "get_error", "could not load the action to check whose keys it writes")
						return
					}
					if err == nil && a.ProposalID != "" {
						p, perr := s.Onboard.GetProposal(r.Context(), a.ProposalID)
						if perr != nil || s.personTierRankVr6w(r, p, nil) > 0 {
							s.emitAuditWithInviteToken(r, Actor{Email: pr.Handle}, audit, a.ID, "", nil, nil, false, "refused", perPersonFollowUpRefusalVr6w)
							onboardError(w, http.StatusForbidden, "forbidden", perPersonFollowUpRefusalVr6w)
							return
						}
					}
				}
			}
			if action, ok := machineQueueRoutesVr6w[r.Pattern]; ok && s.Onboard != nil {
				p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
				if err != nil && !errors.Is(err, onboardstore.ErrNotFound) {
					onboardError(w, http.StatusServiceUnavailable, "get_error", "could not load the proposal to check what it would mint")
					return
				}
				if err == nil {
					entersQueue := strings.HasSuffix(r.Pattern, "/restore") || strings.HasSuffix(r.Pattern, "/retry")
					if derr := s.queueCeilingVr6w(r, p, entersQueue); derr != nil {
						s.emitAuditWithInviteToken(r, Actor{Email: onboardPrincipalFromCtx(r.Context()).Handle}, action, p.ID, "", nil, nil, false, "refused", derr.msg)
						onboardError(w, derr.status, "forbidden", derr.msg)
						return
					}
				}
			}
			next(w, r)
			return
		}
		raw, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
		if err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
			return
		}
		var probe map[string]json.RawMessage
		var req machineTargetBodyVr6w
		if err := decodeMachineBodyVr6w(raw, &probe); err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: "+err.Error())
			return
		}
		for k := range probe {
			if !containsString(route.fields, k) {
				onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: unknown field "+k)
				return
			}
		}
		if err := decodeMachineBodyVr6w(raw, &req); err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: "+err.Error())
			return
		}
		if onboardPrincipalFromCtx(r.Context()).IsOperator && perPersonOperatorKeyVr6w(r) {
			if _, target, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email); gErr == nil && targetTierRankVr6w(target) >= platformTier {
				s.emitAudit(r, Actor{Email: onboardPrincipalFromCtx(r.Context()).Handle}, route.action, target, tenantIdPKey, nil, nil, false, "refused", perPersonTierRefusal)
				onboardError(w, http.StatusForbidden, "forbidden", perPersonTierRefusal)
				return
			}
		} else if onboardPrincipalFromCtx(r.Context()).IsOperator && route.action == "patch_state" && strings.TrimSpace(req.State) == "active" {
			// ADR 1071 as amended: a service key may not return a
			// platform-tier login to service. Deactivation is unchanged; a
			// target that cannot be read is refused.
			_, target, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email)
			if gErr != nil || (targetTierRankVr6w(target) >= platformTier && target.State != "active") {
				s.emitAudit(r, Actor{Email: onboardPrincipalFromCtx(r.Context()).Handle}, route.action, target, tenantIdPKey, nil, nil, false, "refused", operatorKeyPlatformRefusalVr6w)
				onboardError(w, http.StatusForbidden, "forbidden", operatorKeyPlatformRefusalVr6w)
				return
			}
		}
		canon, _ := json.Marshal(req) //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		r.Body = io.NopCloser(bytes.NewReader(canon))
		r.ContentLength = int64(len(canon))
		next(w, r)
	}
}

// decodeMachineBodyVr6w decodes one JSON value into v, refusing unknown
// fields and anything after the value.
func decodeMachineBodyVr6w(raw []byte, v interface{}) error {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return err
	}
	if err := dec.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("trailing data after the JSON body")
	}
	return nil
}

// machineTargetBodyVr6w is the fenced route's request; a route accepts only
// its own fields (machineRoute.fields).
type machineTargetBodyVr6w struct {
	Email string `json:"email,omitempty"`
	ID    string `json:"id,omitempty"`
	State string `json:"state,omitempty"`
}

// welcomeGate fronts the welcome-package routes. A sponsor session reaches
// only packages for proposals it sponsors (as every other /onboarding/{id}
// handler is scoped), and downloading a package, which carries a freshly
// minted recovery link, takes the mint_recovery_link step-up.
func (s *Server) welcomeGate(download bool, next http.HandlerFunc) http.HandlerFunc {
	s.noteGateWrap("welcomeGate")
	return func(w http.ResponseWriter, r *http.Request) {
		if s.Onboard == nil {
			next(w, r)
			return
		}
		p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
		if err != nil {
			next(w, r)
			return
		}
		if sp := sponsorFromCtx(r.Context()); sp != nil && !sponsorAllows(sp.keys(), p.Sponsor) {
			s.renderError(w, r, 404, "proposal not found")
			return
		}
		// A resend that carries new mesh keys overwrites the person's stored
		// ones; nobody does that for a person above their own tier.
		if !download && (strings.TrimSpace(r.FormValue("setup_key")) != "" || strings.TrimSpace(r.FormValue("handset_key")) != "") &&
			s.personAboveTierVr6w(r, p) {
			s.emitAuditWithInviteToken(r, actorFromCtx(r.Context()), "onboard_welcome_resend", p.Handle, "", nil,
				map[string]interface{}{"env": p.Env, "email": p.Email}, false, "refused", tierCeilingRefusal)
			s.renderError(w, r, 403, tierCeilingRefusal)
			return
		}
		if download && s.stepUpRequired("mint_recovery_link") {
			actor := actorFromCtx(r.Context())
			if out := s.requireStepUp(w, r, actor, "mint_recovery_link"); out.Handled {
				s.emitAuditWithInviteToken(r, actor, "onboard_welcome_package", p.Handle, "", nil,
					map[string]interface{}{"env": p.Env, "email": p.Email}, false, "refused", stepUpChallengeAudit)
				return
			}
		}
		next(w, r)
	}
}

// reissueGate fronts the mesh-key reissue routes. Reissuing revokes the
// person's live device or handset keys, so a sponsor reaches only proposals
// it sponsors, and nobody reissues for a person above their own tier.
func (s *Server) reissueGate(next http.HandlerFunc) http.HandlerFunc {
	s.noteGateWrap("reissueGate")
	return func(w http.ResponseWriter, r *http.Request) {
		if s.Onboard == nil {
			next(w, r)
			return
		}
		p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
		if err != nil {
			next(w, r)
			return
		}
		if sp := sponsorFromCtx(r.Context()); sp != nil && !sponsorAllows(sp.keys(), p.Sponsor) {
			s.renderError(w, r, 404, "proposal not found")
			return
		}
		if s.personAboveTierVr6w(r, p) {
			s.renderError(w, r, 403, tierCeilingRefusal)
			return
		}
		next(w, r)
	}
}

// --- route recording ----------------------------------------------------

// routeEntry is one registered pattern, whether a target gate was built
// for its handler, and which gates (targetGate, machineTargetGate,
// welcomeGate, reissueGate).
type routeEntry struct {
	pattern string
	gated   bool
	gates   []string
}

var routeAudit = struct {
	sync.Mutex
	wraps   map[*Server]int
	pending map[*Server][]string
	routes  map[*Server][]routeEntry
}{wraps: map[*Server]int{}, pending: map[*Server][]string{}, routes: map[*Server][]routeEntry{}}

// noteGateWrap records gate being built for s. A handler's arguments are
// built just before its HandleFunc call, so the gates noted between two
// registrations belong to the second one.
func (s *Server) noteGateWrap(gate string) {
	routeAudit.Lock()
	routeAudit.wraps[s]++
	routeAudit.pending[s] = append(routeAudit.pending[s], gate)
	routeAudit.Unlock()
}

// routeMux is the ServeMux Routes() registers on; it records every pattern.
type routeMux struct {
	*http.ServeMux
	s    *Server
	seen int
}

func (s *Server) newRouteMux() *routeMux {
	routeAudit.Lock()
	routeAudit.routes[s] = nil
	routeAudit.pending[s] = nil
	n := routeAudit.wraps[s]
	routeAudit.Unlock()
	return &routeMux{ServeMux: http.NewServeMux(), s: s, seen: n}
}

func (m *routeMux) record(pattern string) {
	routeAudit.Lock()
	n := routeAudit.wraps[m.s]
	routeAudit.routes[m.s] = append(routeAudit.routes[m.s], routeEntry{pattern: pattern, gated: n != m.seen, gates: routeAudit.pending[m.s]})
	routeAudit.pending[m.s] = nil
	m.seen = n
	routeAudit.Unlock()
}

func (m *routeMux) HandleFunc(pattern string, h func(http.ResponseWriter, *http.Request)) {
	m.record(pattern)
	if !routeClassifiedVr6w(pattern) {
		h = refuseUnclassifiedRoute
	}
	m.ServeMux.Handle(pattern, withListedActions(pattern, http.HandlerFunc(h)))
}

func (m *routeMux) Handle(pattern string, h http.Handler) {
	m.record(pattern)
	if !routeClassifiedVr6w(pattern) {
		h = http.HandlerFunc(refuseUnclassifiedRoute)
	}
	m.ServeMux.Handle(pattern, withListedActions(pattern, h))
}

// refuseUnclassifiedRoute answers every request to a registered pattern that
// no table classifies (default deny, browser and machine alike).
func refuseUnclassifiedRoute(w http.ResponseWriter, r *http.Request) {
	http.Error(w, unclassifiedRouteRefusal, http.StatusForbidden)
}

// registeredRoutes returns the patterns s's last Routes() call registered.
func (s *Server) registeredRoutes() []routeEntry {
	routeAudit.Lock()
	defer routeAudit.Unlock()
	return append([]routeEntry(nil), routeAudit.routes[s]...)
}

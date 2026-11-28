package handlers

// Route classification (GHSA-vr6w). Every pattern Routes() registers is
// classified by name, browser and machine alike: a target route the gates
// check (browserTargetRoutes, machineTargetRoutes, machineProposalRoutes,
// machineQueueRoutesVr6w, proposalGatedRoutesVr6w), a machine record route
// (machineRecordRoutesVr6w), a capped-elsewhere machine route
// (machineUngatedRoutes), or a route listed here with the reason it needs no
// target check. Nothing is admitted by prefix or by the shape of the path.
// routeMux registers an unclassified pattern as a 403 (default deny), and
// TestTargetGate_coversEveryTargetRoute fails on it.

import "net/http"

// stackOnlyVr6w marks an entry for a route another embargo patch registers
// (GHSA-mxf4), absent when this patch applies alone.
const stackOnlyVr6w = "[registered by another patch] "

// proposalGatedRoutesVr6w are the /onboarding/{id}/ routes fronted by
// welcomeGate or reissueGate, which scope and cap them by the proposal.
var proposalGatedRoutesVr6w = map[string]string{
	"GET /onboarding/{id}/welcome":              "welcomeGate",
	"POST /onboarding/{id}/welcome/resend":      "welcomeGate",
	"POST /onboarding/{id}/handset-key/reissue": "reissueGate",
	"POST /onboarding/{id}/device-key/reissue":  "reissueGate",
}

// expectedGateVr6w is the gate a target route must be wrapped in. The
// coverage test asserts it, so a target route wrapped in the wrong gate (or
// only in some other gate) fails.
func expectedGateVr6w(pattern string) string {
	if _, ok := browserTargetRoutes[pattern]; ok {
		return "targetGate"
	}
	if g, ok := proposalGatedRoutesVr6w[pattern]; ok {
		return g
	}
	_, mt := machineTargetRoutes[pattern]
	_, mp := machineProposalRoutes[pattern]
	_, mq := machineQueueRoutesVr6w[pattern]
	_, ma := machineActionRoutesVr6w[pattern]
	if mt || mp || mq || ma {
		return "machineTargetGate"
	}
	return ""
}

// wildcardActionsVr6w lists, for each route whose last segment is an
// {action} wildcard, every action its handler implements. routeMux answers
// any other action with 404 before the handler runs, so a new action needs
// an entry here (and the coverage test enumerates these lists).
var wildcardActionsVr6w = map[string][]string{
	"POST /onboarding/{id}/{action}":    {"approve", "reject", "sponsor-approve", "sponsor-reject", "retry", "offboard", "offboard-cut"},
	"POST /operator-keys/{id}/{action}": {"approve", "reject", "revoke"},
	"POST /ssh-keys/{id}/{action}":      {"approve", "reject", "revoke"},
}

// withListedActions answers an {action} the route's list does not name with
// 404.
func withListedActions(pattern string, h http.Handler) http.Handler {
	actions, ok := wildcardActionsVr6w[pattern]
	if !ok {
		return h
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		for _, a := range actions {
			if r.PathValue("action") == a {
				h.ServeHTTP(w, r)
				return
			}
		}
		http.NotFound(w, r)
	})
}

// routeNotTargetVr6w lists every other registered route with the reason it
// needs no target-identity check. targetGate passes an admin route listed
// here and refuses an admin route listed nowhere.
var routeNotTargetVr6w = map[string]string{ //nolint:gosec // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	// Unauthenticated or infrastructure.
	"GET /healthz":                   "health check, no data",
	"GET /healthz/operator-keys":     stackOnlyVr6w + "health check: counts operator keys, acts on no identity",
	"GET /internal/probation/ready":  "probation readiness probe, no identity",
	"/internal/probation/auth":       "probation forward-auth check, acts on no identity",
	"POST /__goat/probation/request": "probation access request from the requester's own session",
	"GET /static/":                   "static assets",
	"GET /api/onboard/capabilities":  "unauthenticated capability list, no data",
	"POST /api/notify":               "token-gated stage notification source, acts on no identity",
	"GET /docs/onboarding":           "documentation page",
	// Public intake and invite redemption.
	"GET /invites/{token}/accept":      "public redemption; the preset was capped when the invite was minted",
	"POST /invites/{token}/accept":     "public redemption; the preset was capped when the invite was minted",
	"GET /public/onboard.html":         "public intake form",
	"GET /public/onboarding-guide":     "public guide page",
	"GET /public/onboard/preview":      "public intake preview, no identity",
	"GET /public/network-requirements": "public page",
	"POST /public/onboard/submit":      "public intake submit; creates a proposed row that approval caps",
	// Self-service: the session's own identity only.
	"GET /identity":                  "self-service: the session's own identity only",
	"GET /fabric-credential":         "self-service: the session's own identity only",
	"POST /fabric-credential":        "self-service: the session's own identity only",
	"GET /setup-code":                "self-service: the session's own setup code",
	"POST /setup-code":               "self-service: the session's own setup code",
	"GET /ssh-key":                   "self-service: the session's own SSH key request",
	"POST /ssh-key":                  "self-service: the session's own SSH key request",
	"GET /github":                    "self-service: the session's own GitHub handle",
	"POST /github":                   "self-service: the session's own GitHub handle",
	"GET /operator-key":              "self-service: the session's own key request",
	"POST /operator-key":             "self-service: the session's own key request",
	"GET /operator-key/collect/{id}": "self-service: collects the session's own approved key",
	"GET /step-up":                   "step-up challenge for the session itself",
	"GET /step-up/return":            "step-up return for the session itself",
	"POST /step-up/lock":             "step-up lock for the session itself",
	// Admin pages and records that are not a login identity.
	"GET /{$}":                                 "user list, view only",
	"GET /users/new":                           "create form, view only",
	"POST /users/new":                          "create; capped in handleUserCreate (groupsOutrankActor) with step-up",
	"GET /users/{id}":                          "view only",
	"GET /invites":                             "invite list, view only",
	"POST /invites":                            "invite mint; capped in handleInviteCreate (groupsOutrankActor)",
	"POST /invites/{token}/revoke":             "revokes an invite, not an identity",
	"GET /holds":                               "mesh peer holds, view only",
	"GET /holds/{peer}/release":                "view only (mesh peer hold preview)",
	"POST /holds/{peer}/release":               "releases a mesh peer hold, not a login identity",
	"GET /reactivation-requests":               "view only",
	"GET /reactivation-requests/{id}/decline":  "view only (decline preview)",
	"POST /reactivation-requests/{id}/decline": "declines a reactivation request; the identity is not changed",
	"GET /offboard-verify":                     "view only",
	"GET /departures":                          "view only",
	"GET /access-review":                       "view only",
	"GET /permissions":                         "view only",
	"GET /audit":                               "view only",
	"GET /onboarding/operator-keys":            "view only (operator-key queue)",
	"POST /operator-keys/{id}/{action}":        "operator-key request decision; two-person rule in the store",
	"GET /onboarding/ssh-keys":                 "view only (SSH key queue)",
	"POST /ssh-keys/{id}/{action}":             "SSH key request decision, not a login identity",
	// Onboarding queue (onboardGate: admins and sponsor-scoped sponsors).
	"GET /onboarding":                      "queue page, view only",
	"GET /api/onboard/proposals/counts":    "queue badge counts, sponsor-scoped, no credential",
	"GET /onboarding/{id}/devices":         "view only",
	"GET /onboarding/{id}/repo-access":     "view only",
	"GET /onboarding/invite":               "invite form, view only",
	"POST /onboarding/invite":              "invite mint; tier capped in handleOnboardInviteCreate (sponsor cap, or inviteTierRankVr6w for an admin)",
	"POST /onboarding/invite/email/verify": "checks an address for an invite, no identity",
	"POST /onboarding/invite/email/send":   "mails an invite already minted under the cap",
	"POST /onboarding/invite/email/eml":    "renders an invite mail already minted under the cap",
	"POST /onboarding/invite/{id}/revoke":  "revokes an invite, not an identity",
	"GET /onboarding/org-suggest":          "organisation lookup, no identity",
	"GET /onboarding/affiliation-derive":   "affiliation lookup, no identity",
	"POST /onboarding/{id}/org-attach":     "proposal record (organisation), not a login identity",
	"POST /onboarding/{id}/fields":         "proposal record fields, not a login identity",
	"POST /onboarding/{id}/org-new":        "proposal record (organisation), not a login identity",
	"POST /onboarding/{id}/{action}":       "proposal decision; approvalCeiling and the retry arm cap the tier, offboard-cut is capped in handleOffboardCut",
	"POST /onboarding/{id}/restore":        "restore; capped in handleOnboardingRestore (queueCeilingVr6w)",
	"POST /onboarding/{id}/github":         "proposal record (GitHub handle), not a login identity",
	"POST /onboarding/{id}/github-request": "proposal record (GitHub request), not a login identity",
	"POST /onboarding/{id}/repo-access":    "proposal record (repository access), not a login identity",
}

// routeClassifiedVr6w reports whether pattern is classified by name.
func routeClassifiedVr6w(pattern string) bool {
	if _, ok := browserTargetRoutes[pattern]; ok {
		return true
	}
	if _, ok := routeNotTargetVr6w[pattern]; ok {
		return true
	}
	if _, ok := proposalGatedRoutesVr6w[pattern]; ok {
		return true
	}
	return machineRouteClassified(pattern)
}

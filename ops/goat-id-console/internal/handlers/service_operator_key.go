package handlers

import "strings"

// isServiceKeyHandle is THE rule for whether an operator key belongs
// to a service rather than a person (GHSA-mxf4-8rv7-774w). A service key has
// no identity to tie it to, so the machine-realm owner gate, the admin
// self/tier rules and the roster requirement for new keys do not apply to it;
// every other operator key is a person's.
//
// A handle is a service handle when, after trimming, it is exactly:
//   - one of the built-in service labels (isServiceSponsorLabel:
//     "operator", "onboard-api", "id-console"; the empty handle is NOT a
//     service key),
//   - "onboard-runner", or "onboard-runner-" followed by anything (the
//     runner id the onboard-runner role provisions, onboard-runner-<site>),
//   - or one of the configured handles (ONBOARD_SERVICE_OPERATOR_HANDLES,
//     config.Config.ServiceOperatorHandles).
//
// Exact match, no case folding: this is a trust decision and must not widen.
func isServiceKeyHandle(handle string, configured []string) bool {
	h := strings.TrimSpace(handle)
	if h == "" {
		// An empty handle is only the defensive fallback for an unset
		// principal; it names no service, so it fails closed.
		return false
	}
	if isServiceSponsorLabel(h) || h == "onboard-runner" || strings.HasPrefix(h, "onboard-runner-") {
		return true
	}
	for _, c := range configured {
		if strings.TrimSpace(c) == h {
			return true
		}
	}
	return false
}

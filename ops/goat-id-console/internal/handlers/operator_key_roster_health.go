package handlers

import (
	"context"
	"log/slog"
	"net/http"
	"sort"
	"strings"
)

// Per-person keys pinned in ONBOARD_OPERATOR_API_KEYS are tied to their
// holder through the roster like any other per-person key. One whose handle
// no key row and no settled onboarding record names is refused by the live
// machine-realm gate (operatorOwnerGate), so a key that worked before the
// gate stops working on upgrade. The console says so at startup and on
// /healthz/operator-keys rather than leaving the refusal to be found in the
// audit log. Service keys (isServiceKeyHandle) are not checked and not listed.

// UnrosteredOperatorKeys returns the handles of the configured per-person
// operator keys that resolve to no person, sorted. Empty when the live
// identity check is off (the gate is off too) or nothing is configured. A
// store error is returned; the handle list is then incomplete.
func (s *Server) UnrosteredOperatorKeys(ctx context.Context) ([]string, error) {
	if s.Cfg == nil || !s.Cfg.LiveIdentityCheck || s.Cfg.DevBypassAuth {
		return nil, nil
	}
	seen := map[string]bool{}
	var out []string
	for _, handle := range s.Cfg.OnboardOperatorAPIKeys {
		h := strings.TrimSpace(handle)
		if h == "" || seen[h] || s.isServiceOperatorPrincipal(h) {
			continue
		}
		seen[h] = true
		ev, err := s.operatorOwnerEvidenceFor(ctx, h)
		if err != nil {
			return out, err
		}
		if ev.empty() {
			out = append(out, h)
		}
	}
	sort.Strings(out)
	return out, nil
}

// WarnUnrosteredOperatorKeys logs, once at startup, each configured
// per-person operator key the machine-realm gate will refuse.
func (s *Server) WarnUnrosteredOperatorKeys(ctx context.Context) {
	hs, err := s.UnrosteredOperatorKeys(ctx)
	if err != nil {
		slog.Warn("could not check ONBOARD_OPERATOR_API_KEYS against the onboarding roster", "err", err)
	}
	for _, h := range hs {
		slog.Warn("operator key in ONBOARD_OPERATOR_API_KEYS names no person on the onboarding roster; the machine realm refuses it",
			"handle", h,
			"fix", "file it under the holder's roster handle, or list it in ONBOARD_SERVICE_OPERATOR_HANDLES if it belongs to a service")
	}
}

// handleHealthzOperatorKeys (GET /healthz/operator-keys) reports the same
// list. Unauthenticated like /healthz: it names operator handles, which are
// roster names, never key material.
func (s *Server) handleHealthzOperatorKeys(w http.ResponseWriter, r *http.Request) {
	hs, err := s.UnrosteredOperatorKeys(r.Context())
	body := map[string]any{
		"live_identity_check":      s.Cfg != nil && s.Cfg.LiveIdentityCheck,
		"unrostered_operator_keys": hs,
		"count":                    len(hs),
	}
	if hs == nil {
		body["unrostered_operator_keys"] = []string{}
	}
	if err != nil {
		body["error"] = "the roster could not be read; the list may be incomplete"
	}
	onboardJSON(w, http.StatusOK, body)
}

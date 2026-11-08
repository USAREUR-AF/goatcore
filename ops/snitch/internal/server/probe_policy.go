// Package server — probe-policy handlers.
//
// Two endpoints (Block 61):
//
//   GET  /api/probe-policy        — public; probes poll every cycle
//   POST /api/probe-policy/set    — admin-auth; sets the fleet policy
//
// Defaults to PolicyDefault. Operators set focused-research windows
// via the admin endpoint, e.g.:
//
//   curl -X POST -H "Authorization: Bearer $ADMIN_KEY" \
//     -d '{"mode":"focused-research","expires_at":"2026-05-04T15:10:00Z","reason":"weekly net-health snapshot","set_by":"operator:maintainer"}' \
//     <collector-url>/api/probe-policy/set
//
// To abort an in-progress focused-research window early:
//
//   curl -X POST -H "Authorization: Bearer $ADMIN_KEY" \
//     -d '{"mode":"default","reason":"abort","set_by":"operator:maintainer"}' \
//     <collector-url>/api/probe-policy/set
//
// To stop probes fleet-wide:
//
//   curl -X POST -H "Authorization: Bearer $ADMIN_KEY" \
//     -d '{"mode":"off","reason":"operational disruption","set_by":"operator:maintainer"}' \
//     <collector-url>/api/probe-policy/set
//
// Default at boot is read from env var SNITCH_DEFAULT_POLICY (one of
// off / minimal / default / focused-research). If unset, defaults to
// "default".

package server

import (
	"encoding/json"
	"net/http"
	"os"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

// probePolicyStore holds the current fleet policy. Process-local; no
// persistence — collector restart re-reads SNITCH_DEFAULT_POLICY env.
// In a multi-collector deployment, the policy can drift between
// collectors during failover; that's acceptable for a probe-throttle
// (probes will re-converge on the next cycle).
type probePolicyStore struct {
	mu      sync.RWMutex
	current types.ProbePolicy
}

var policyStore = func() *probePolicyStore {
	return &probePolicyStore{
		current: bootPolicy(),
	}
}()

// bootPolicy resolves the policy at collector boot.
func bootPolicy() types.ProbePolicy {
	mode := os.Getenv("SNITCH_DEFAULT_POLICY")
	if mode == "" {
		mode = types.PolicyDefault
	}
	return types.ProbePolicy{
		Mode:  mode,
		SetBy: "boot:env",
		SetAt: time.Now(),
	}
}

// handleProbePolicy serves the current policy. Public; probes poll.
func (s *Server) handleProbePolicy(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	policyStore.mu.RLock()
	current := policyStore.current
	policyStore.mu.RUnlock()
	// Auto-revert if expired (so operators can rely on the natural
	// expiry without polling /set themselves).
	if !current.ExpiresAt.IsZero() && time.Now().After(current.ExpiresAt) {
		policyStore.mu.Lock()
		current = types.ProbePolicy{
			Mode:   types.PolicyDefault,
			Reason: "auto-reverted (previous policy expired at " + current.ExpiresAt.Format(time.RFC3339) + ")",
			SetBy:  "auto",
			SetAt:  time.Now(),
		}
		policyStore.current = current
		policyStore.mu.Unlock()
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(current)
}

// handleSetProbePolicy lets admins set the fleet policy.
func (s *Server) handleSetProbePolicy(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost && r.Method != http.MethodPut {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var p types.ProbePolicy
	if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
		http.Error(w, "invalid JSON: "+err.Error(), http.StatusBadRequest)
		return
	}
	if !validPolicyMode(p.Mode) {
		http.Error(w, "invalid mode (off|minimal|default|focused-research)", http.StatusBadRequest)
		return
	}
	if p.SetAt.IsZero() {
		p.SetAt = time.Now()
	}
	policyStore.mu.Lock()
	policyStore.current = p
	policyStore.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(p)
}

func validPolicyMode(m string) bool {
	switch m {
	case types.PolicyOff, types.PolicyMinimal, types.PolicyDefault, types.PolicyFocusedResearch:
		return true
	}
	return false
}

package handlers

import (
	"bytes"
	"errors"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// handleOnboardingDevices is the selected admin card's read-only device
// fragment. There is no peer request while rendering /onboarding (#2384).
func (s *Server) handleOnboardingDevices(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	if s.Onboard == nil {
		s.renderError(w, r, http.StatusServiceUnavailable, "onboarding is not configured")
		return
	}
	p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
	if errors.Is(err, onboardstore.ErrNotFound) {
		http.NotFound(w, r)
		return
	}
	if err != nil {
		s.renderError(w, r, http.StatusBadGateway, "could not load proposal")
		return
	}
	if p.Status != onboardstore.ProposalOnboarded && p.Status != onboardstore.ProposalEnrolled {
		http.NotFound(w, r)
		return
	}
	v := &proposalView{}
	v.Devices, v.DevicesErr = s.devicesForHandle(r.Context(), p.Handle)
	// Execute a clone: executing the shared template set would prevent the
	// normal page renderer from cloning it on subsequent requests.
	t, err := s.Templates.Clone()
	if err != nil {
		s.renderFailure(w, r, "onboard_devices", err)
		return
	}
	var buf bytes.Buffer
	if err := t.ExecuteTemplate(&buf, "devices_block", v); err != nil {
		s.renderFailure(w, r, "onboard_devices", err)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Referrer-Policy", "no-referrer")
	//nolint:errcheck // Response delivery is best-effort after the fragment is rendered.
	_, _ = buf.WriteTo(w)
}

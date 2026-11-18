package api

import (
	"errors"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Identity-keyed rotation (goat#320): the console's door onto ADR 1044's
// in-place re-issue. Same trust boundary as collection (collect.go): the
// caller — goat-id-console, holding the shared operator bearer and naming the
// admin via X-Goat-Operator-Actor — says WHO the person is, never
// which anchor. The portal resolves identity → slot itself, so a caller
// cannot rotate someone else's anchor by asking; it never gets to name one.
//
// The rotation performed is rotateAnchor, the exact machinery behind the
// CLI's POST /api/orgs/{anchor}/rotate — one implementation, two doors.

// rotateByIdentityRequest names the person whose credential rotates, and
// nothing else. Deliberately no anchor/prefix/alias field — see above.
type rotateByIdentityRequest struct {
	IdentityID string `json:"identity_id"`
}

// rotateByIdentity handles POST /api/orgs/rotate (operator-gated).
func (s *Server) rotateByIdentity(w http.ResponseWriter, r *http.Request) {
	var req rotateByIdentityRequest
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	identityID := registry.NormalizeIdentityID(req.IdentityID)
	if identityID == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "identity_id is required")
		return
	}
	if err := registry.ValidIdentityID(identityID); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_identity_id", err.Error())
		return
	}

	// Slot-pool goatnets only, same refusal as collect: per-vendor mode has no
	// person→slot linkage to resolve, so there is no "this person's credential"
	// to rotate; refuse plainly rather than half-work.
	if s.cfg.IdentityModel != config.IdentityModelPerUUID {
		writeError(w, http.StatusNotImplemented, "not_slot_mode",
			"identity-keyed rotation requires the per-uuid slot-pool identity model (ADR 1025); this goatnet runs per-vendor identities — rotate by anchor with POST /api/orgs/{anchor}/rotate")
		return
	}

	slot, err := s.reg.SlotForIdentity(r.Context(), identityID)
	switch {
	case errors.Is(err, registry.ErrNotFound):
		writeError(w, http.StatusNotFound, "no_slot",
			"no live slot is recorded for this identity — either the person was never onboarded to this goatnet, "+
				"their slot was revoked, or they were onboarded before the person→slot linkage was recorded. "+
				"An operator can stamp the linkage with PUT /api/slots/{anchor}/identity.")
		return
	case errors.Is(err, registry.ErrAmbiguous):
		writeError(w, http.StatusConflict, "ambiguous_identity",
			"more than one live slot is recorded for this identity; refusing to guess which one to rotate. An operator must fix the duplicate linkage.")
		return
	case err != nil:
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}

	// No minted credential means nothing to rotate. Minting here would be a
	// first issue through the wrong door: it would stamp the slot minted while
	// collected_at stays NULL, burning the person's one-time collection
	// (fabric-credential-collection.md) on a plaintext only the admin sees.
	if slot.CertMintedAt == nil {
		writeError(w, http.StatusConflict, "not_minted",
			"this identity's slot holds no minted credential, so there is nothing to rotate. "+
				"The person collects their first credential themselves via the console's fabric-credential page (POST /api/orgs/collect).")
		return
	}

	s.rotateAnchor(w, r, slot.SlotID)
}

package api

// Machine revoke — the inverse of the mint (machine-mesh-alignment follow-on;
// the M-107 §5 remainder).
//
// A machine has four doors in (enroll, enroll-bulk, mesh-key, GET) and needs an
// exit: without one, a machine brought into existence — own slot, own cert,
// custody edge, one-off setup key — could never be retired.
//
// DELETE /api/machines/{anchor} is that exit. Three properties are deliberate:
//
//   - MESH-FIRST (ADR 1031). The authoritative cut lives at NetBird, not at
//     the Zenoh ACL: in per-uuid slot-pool mode the router bakes a
//     `slot-w-<anchor>` grant for EVERY slot unconditionally, so the registry
//     teardown moves no authorization at all and mesh removal is the only
//     barrier that actually falls. The mesh cut therefore runs FIRST and a
//     failure there aborts with the registry untouched — a half-cut that
//     reported success would leave a live peer behind a row that says retired,
//     which is worse than a clean refusal to retry.
//   - THE UNCONSUMED KEY GOES TOO. A machine's setup key is one-off with a
// 30-day join window (D4). An unspent one left standing for a retired machine
// is a standing invitation carrying gear groups that nobody re-reads.
//   - QUOTA RETURNS BY CONSTRUCTION. Remaining quota is derived
//     (quota − COUNT(live enrolled slots)), never stored, so stamping
//     revoked_at IS the refund. Nothing here adds to a counter.
//
// Not reversible in the restore sense, and the response says so: re-enrolling
// the handle allocates a FRESH slot with a new anchor. What survives is the
// record — the slot row keeps its born-as stamps and custody edge alongside
// revoked_at, and the audit event carries the full inventory of what was cut.

import (
	"errors"
	"fmt"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

type machineRevokeResponse struct {
	SlotID       string `json:"slot_id"`
	SystemHandle string `json:"system_handle"`
	NodeClass    string `json:"node_class"`
	Custodian    string `json:"custodian"`

	// MeshCut reports whether the portal actually reached the mgmt server and
	// cut. False is a real answer, not an error: a portal with no mgmt
	// credential never minted the key either, so the peer (if any) was made by
	// hand and has to be cut by hand. MeshNote says which case this was.
	MeshCut          bool     `json:"mesh_cut"`
	PeersDeleted     []string `json:"peers_deleted,omitempty"`
	KeysDeleted      []string `json:"keys_deleted,omitempty"`
	KeysAlreadySpent []string `json:"keys_already_spent,omitempty"`
	MeshNote         string   `json:"mesh_note"`

	SlotFreed bool `json:"slot_freed"`
	// FabricRevocation says whether the router still admits the machine's
	// leaf (GHSA-rr3p): live, pending_router_reload, unknown or none.
	FabricRevocation string `json:"fabric_revocation"`
	FabricNote       string `json:"fabric_note"`
	QuotaRemaining   int64  `json:"quota_remaining"`
	Note             string `json:"note"`
}

// machineRevoke retires one enrolled machine. Gate mirrors the re-mint
// (machine_mesh_key.go): an operator for any machine, a vendor-scoped caller
// only for machines under its own custody edge — retiring a machine is the
// same custodial authority as re-minting its key, and a custodian who may
// enroll must be able to un-enroll or the quota is a one-way ratchet.
func (s *Server) machineRevoke(w http.ResponseWriter, r *http.Request) {
	if s.cfg.IdentityModel != config.IdentityModelPerUUID {
		writeError(w, http.StatusPreconditionFailed, "not_slot_pool",
			"machine revoke requires the per-uuid slot-pool identity model (ADR 1025)")
		return
	}
	p := PrincipalFromContext(r.Context())
	if p == nil {
		writeError(w, http.StatusUnauthorized, "auth", "missing principal")
		return
	}

	// Accept the handle the enroll issued as well as the anchor — the same
	// affordance DELETE /api/vendors grew after a handle-keyed delete 404'd and
	// left an orphan row (the partner-onboard failure). Handles are
	// lowercase aliases and anchors are 32-hex, so the two cannot collide.
	anchor := r.PathValue("anchor")
	if resolved, err := s.reg.AnchorForHandle(r.Context(), anchor); err == nil {
		anchor = resolved
	}

	m, err := s.reg.MachineByAnchor(r.Context(), anchor)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "not_a_machine",
			"no live custody-enrolled machine at this anchor — revoke retires machines the mint enrolled; a person's slot or an already-revoked machine is not one. `DELETE /api/vendors/{anchor}` is the door for a person's slot.")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "read_machine", err.Error())
		return
	}
	custodian := derefStr(m.EnrolledBy)
	if !p.Operator && (p.Vendor == "" || p.Vendor != custodian) {
		writeError(w, http.StatusForbidden, "custodian_mismatch",
			"a non-operator may revoke only machines under its own custody edge")
		return
	}
	handle := derefStr(m.AssignedTo)
	nodeClass := derefStr(m.NodeClass)

	// --- mesh first (ADR 1031) ------------------------------------------
	resp := &machineRevokeResponse{
		SlotID:       anchor,
		SystemHandle: handle,
		NodeClass:    nodeClass,
		Custodian:    custodian,
	}
	if s.nbKeys.Enabled() {
		// Peer name == machine handle is the provenance guard's join key
		// (machine-enrollment.md); it is also what makes the peer findable
		// here. A peer renamed off its handle survives this cut and then
		// grades as a violation in check-node-class-membership.sh — which is
		// the designed outcome: the drift becomes visible rather than
		// silently un-cuttable.
		cut, cerr := s.nbKeys.CutMachine(r.Context(), "machine-"+handle, handle)
		if cerr != nil {
			writeError(w, http.StatusBadGateway, "mesh_cut",
				fmt.Sprintf("could not cut %q from the mesh: %v. NOTHING was changed — the registry row, slot and cert are untouched; retry once the mgmt server answers. Revoking the registry while the peer stays live would report a cut that did not happen.", handle, cerr))
			return
		}
		resp.MeshCut = true
		resp.PeersDeleted = cut.PeersDeleted
		resp.KeysDeleted = cut.KeysDeleted
		resp.KeysAlreadySpent = cut.KeysAlreadySpent
		resp.MeshNote = fmt.Sprintf("mesh cut: %d peer(s) deleted, %d unconsumed setup key(s) deleted, %d already-spent key(s) left as the record of how the peer joined.",
			len(cut.PeersDeleted), len(cut.KeysDeleted), len(cut.KeysAlreadySpent))
	} else {
		resp.MeshNote = fmt.Sprintf("mesh NOT cut: this portal holds no netbird mgmt credential, so it never minted this machine's key either. Delete the peer named %q and any setup key named %q by hand — until you do, the machine is retired in the registry and still on the mesh.", handle, "machine-"+handle)
	}

	// --- then the registry ----------------------------------------------
	// Audit before the destructive part so the row is reconstructible from the
	// audit trail even after the vendor row is gone (the deleteOrRetireTopic
	// discipline).
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "machine-revoke", handle,
		fmt.Sprintf(`{"anchor":%q,"custodian":%q,"node_class":%q,"mesh_cut":%t,"peers_deleted":%q,"keys_deleted":%q,"keys_already_spent":%q}`,
			anchor, custodian, nodeClass, resp.MeshCut,
			strings.Join(resp.PeersDeleted, ","), strings.Join(resp.KeysDeleted, ","), strings.Join(resp.KeysAlreadySpent, ",")))

	if derr := s.reg.DeleteVendor(r.Context(), anchor); derr != nil && !errors.Is(derr, registry.ErrNotFound) {
		if strings.Contains(derr.Error(), "non-retired topic") {
			writeError(w, http.StatusConflict, "conflict",
				derr.Error()+" — the mesh cut above already happened; retire the topics and re-run to finish the registry side.")
			return
		}
		writeError(w, http.StatusInternalServerError, "db", derr.Error())
		return
	}
	// The free advances the slot's generation; re-render the router's slot
	// subjects so the machine's leaf stops matching (GHSA-rr3p), and say
	// whether the router has loaded that yet.
	revokedCN := ""
	if sl, err := s.reg.SlotByID(r.Context(), anchor); err == nil && sl.AssignedTo != nil && sl.RevokedAt == nil {
		revokedCN = registry.LeafCN(anchor, sl.Generation)
	}
	freed, serr := s.reg.RevokeAssignedSlot(r.Context(), anchor)
	s.syncSlotFragment(r.Context())
	if serr != nil {
		writeError(w, http.StatusInternalServerError, "revoke_slot", serr.Error())
		return
	}
	resp.SlotFreed = freed
	if !freed {
		revokedCN = ""
	}
	resp.FabricRevocation, resp.FabricNote = s.revocationStatus(revokedCN)

	resp.QuotaRemaining = -1
	if q, qerr := s.reg.MachineQuotaFor(r.Context(), custodian); qerr == nil {
		resp.QuotaRemaining = q.Remaining
	}
	resp.Note = "The slot row keeps its born-as class, autonomy and custody edge alongside revoked_at — the record of what this machine was survives. The slot index returns to the free pool, so re-enrolling this handle mints a NEW anchor; the old cert is not restorable and its anchor may later belong to a different tenant."
	writeJSON(w, http.StatusOK, resp)
}

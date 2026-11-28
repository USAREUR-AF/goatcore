package api

// Bulk machine enrollment (machine-mesh-alignment plan, decision D7).
//
// POST /api/machines/enroll-bulk enrolls N machines under ONE custodian in one
// operator act. Semantics: N independent mints, one authorization, one summary
// audit act. Each entry runs the same enrollOne core as the single door — same
// grammar refusals, same rollback discipline, same one-off single-use mesh key
// per machine — so an entry that fails rolls back only itself and the rest
// keep going; the atomic in-tx quota ceiling means over-quota entries fail
// cleanly with quota_exhausted rather than racing past the grant.
//
// Deliberately NOT a reusable key handed to N boxes: that shape is a standing
// invitation whose membership nobody re-reads (how unrelated gear ends up sharing a group).
// Bulk here means N single-use keys minted at once, each traceable to its own
// custody edge.

import (
	"fmt"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// bulkEnrollMax bounds one act. Higher volumes are repeated acts, not one
// unbounded request — the cap keeps a runaway script from consuming a whole
// slot pool in one call.
const bulkEnrollMax = 100

type bulkEnrollRequest struct {
	// Custodian for every machine in the act (operator names it; a vendor-scoped
	// caller may only use its own identity, same as the single door).
	Custodian string `json:"custodian,omitempty"`
	// Exercise scopes every minted key in the act; empty = site scope.
	Exercise string        `json:"exercise,omitempty"`
	Machines []machineSpec `json:"machines"`
}

type bulkEnrollEntry struct {
	MachineHandle string `json:"machine_handle"`
	Status        string `json:"status"` // "minted" | "failed"
	ErrorCode     string `json:"error_code,omitempty"`
	Error         string `json:"error,omitempty"`
	// Result is the full single-enroll response for a minted entry (slot,
	// api_key, bundle, mesh key) — the caller distributes per machine.
	Result *enrollMachineResponse `json:"result,omitempty"`
}

type bulkEnrollResponse struct {
	Custodian      string            `json:"custodian"`
	Minted         int               `json:"minted"`
	Failed         int               `json:"failed"`
	QuotaRemaining int64             `json:"quota_remaining"`
	Entries        []bulkEnrollEntry `json:"entries"`
}

func (s *Server) enrollMachinesBulk(w http.ResponseWriter, r *http.Request) {
	if s.cfg.IdentityModel != config.IdentityModelPerUUID {
		writeError(w, http.StatusPreconditionFailed, "not_slot_pool",
			"machine enrollment requires the per-uuid slot-pool identity model (ADR 1025)")
		return
	}
	var req bulkEnrollRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	if len(req.Machines) == 0 {
		writeError(w, http.StatusBadRequest, "empty_bulk", "machines[] is empty")
		return
	}
	if len(req.Machines) > bulkEnrollMax {
		writeError(w, http.StatusBadRequest, "bulk_too_large",
			fmt.Sprintf("machines[] holds %d entries, cap is %d per act — split the standup into repeated acts", len(req.Machines), bulkEnrollMax))
		return
	}
	exercisePrefix, mf := exercisePrefixFor(s.cfg.MeshExercisePrefixes, req.Exercise)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}

	p := PrincipalFromContext(r.Context())
	if p == nil {
		writeError(w, http.StatusUnauthorized, "auth", "missing principal")
		return
	}
	// One custodian per act. The decision-#1 gate runs per entry (an act mixing
	// a KID with a MGMT-HOST must not ride the weaker gate), but every entry
	// must resolve to the SAME custodian — one authorization, one drawdown.
	actCustodian := ""
	for _, spec := range req.Machines {
		c, mf := s.resolveEnrollCustodian(r.Context(), p, spec.NodeClass, req.Custodian)
		if mf != nil {
			s.writeMintFailure(w, mf)
			return
		}
		if actCustodian == "" {
			actCustodian = c
		} else if c != actCustodian {
			writeError(w, http.StatusBadRequest, "mixed_custodians",
				"a bulk act draws down one custodian's grant; entries resolved to different custodians")
			return
		}
	}

	grant, mf := s.grantForEnroll(r.Context(), actCustodian)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}

	actor := actorOf(r)
	resp := bulkEnrollResponse{Custodian: actCustodian, Entries: make([]bulkEnrollEntry, 0, len(req.Machines))}
	for _, spec := range req.Machines {
		entry := bulkEnrollEntry{MachineHandle: spec.MachineHandle}
		meshGroups, mf := validateMachineSpec(spec)
		if mf == nil {
			var one *enrollMachineResponse
			one, mf = s.enrollOne(r.Context(), actor, actCustodian, grant, spec, req.Exercise, exercisePrefix, meshGroups)
			if mf == nil {
				s.requestEnrollCarriage(r, one, spec.Pods)
				entry.Status = "minted"
				entry.Result = one
				resp.Minted++
			}
		}
		if mf != nil {
			entry.Status = "failed"
			entry.ErrorCode = mf.code
			entry.Error = mf.msg
			resp.Failed++
		}
		resp.Entries = append(resp.Entries, entry)
	}

	resp.QuotaRemaining = -1
	if q, qerr := s.reg.MachineQuotaFor(r.Context(), actCustodian); qerr == nil {
		resp.QuotaRemaining = q.Remaining
	}
	for i := range resp.Entries {
		if resp.Entries[i].Result != nil {
			resp.Entries[i].Result.QuotaRemaining = resp.QuotaRemaining
		}
	}

	_ = s.reg.LogAuditEvent(r.Context(), actor, "machine-enroll-bulk", actCustodian,
		fmt.Sprintf(`{"custodian":%q,"requested":%d,"minted":%d,"failed":%d,"mesh_scope":%q}`,
			actCustodian, len(req.Machines), resp.Minted, resp.Failed, req.Exercise))

	// 200 even with partial failures — the per-entry statuses are the result of
	// the act; a caller that wants all-or-nothing checks failed==0 and revokes.
	writeJSON(w, http.StatusOK, &resp)
}

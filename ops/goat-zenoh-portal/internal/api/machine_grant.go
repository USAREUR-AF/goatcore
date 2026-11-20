package api

// POST /api/machines/grant — set (create or adjust) a custodian's machine
// enrollment quota. This is the OPERATOR management path for the standing
// enrollment_grant (ADR 1096) that person-onboard seeds from the manifest
// `machine_quota`: without it, a custodian's quota can only ever be set at
// onboarding, so bumping "give Morgan 5 machines" meant a hand-edit of the
// registry. This endpoint is the shared backend for `goat machine grant` and
// the onboarding/permissions console.
//
// Two shapes, both operator-only:
//   - {custodian, quota, affiliation[, site]} → SetMachineGrant: seed or replace
//     the FULL grant. Use when the custodian has no grant yet (a machine inherits
//     its custodian's unit, so affiliation is required to enroll).
//   - {custodian, quota}                       → SetMachineQuota: adjust only the
//     quota, preserving the existing affiliation/site. Use to bump an existing
//     custodian up or down.
//
// Either shape may additionally carry {network_admin: true|false} to set or
// clear the ADR 1113 delegated network-admin entitlement. It is a pointer field:
// omitting it leaves the entitlement untouched, so a routine quota bump can
// never silently revoke someone's authority to enroll bearer gear.
//
// Returns the resulting derived quota view (quota/used/remaining) so the caller
// renders exactly what the custodian will see next.

import (
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// defaultAdminMachineQuota is the working allowance handed to a custodian who
// is granted the network-admin entitlement with no standing quota. Mirrors the
// invite-time default for the platform-admin tier in goat-cli.
const defaultAdminMachineQuota = 10

type machineGrantRequest struct {
	Custodian   string `json:"custodian"`
	Quota       int64  `json:"quota"`
	Affiliation string `json:"affiliation,omitempty"`
	Site        string `json:"site,omitempty"`
	// NetworkAdmin sets/clears the ADR 1113 delegated network-admin entitlement
	// (may this custodian enroll NETWORK-class bearer gear for their own site
	// without the operator). A POINTER so "absent" is distinguishable from
	// "false": a routine quota bump must leave the entitlement exactly as it
	// was, never silently revoke it.
	NetworkAdmin *bool `json:"network_admin,omitempty"`
}

func (s *Server) grantMachineQuota(w http.ResponseWriter, r *http.Request) {
	p := PrincipalFromContext(r.Context())
	if p == nil {
		writeError(w, http.StatusUnauthorized, "auth", "missing principal")
		return
	}
	// Setting someone's quota is an operator act — a custodian cannot raise its
	// own ceiling (mirrors the enroll gate for infrastructure classes).
	if !p.Operator {
		writeError(w, http.StatusForbidden, "operator_only",
			"setting a machine quota requires an operator token")
		return
	}

	var req machineGrantRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	if req.Custodian == "" {
		writeError(w, http.StatusBadRequest, "missing_custodian", "custodian is required")
		return
	}
	if req.Quota < 0 {
		writeError(w, http.StatusBadRequest, "invalid_quota", "quota must be non-negative")
		return
	}

	grantedBy := p.Actor
	if grantedBy == "" {
		grantedBy = "operator"
	}

	// audit appends the change to audit_events. The grant row's granted_by is
	// overwritten by the next change, so without this row there is no record
	// of who set a quota or granted or withdrew the network-admin entitlement
	// once anyone touches the grant again. Written for every outcome that
	// changed state, including a failure after the grant itself landed.
	audit := func(quota *registry.MachineQuota, errCode string) string {
		d := map[string]any{"requested_quota": req.Quota, "network_admin": req.NetworkAdmin}
		if req.Affiliation != "" {
			d["affiliation"] = req.Affiliation
			d["site"] = req.Site
		}
		if quota == nil {
			// A failure after the grant write: read back what did land so the
			// row says whether the quota or affiliation changed.
			quota, _ = s.reg.MachineQuotaFor(r.Context(), req.Custodian) //nolint:errcheck // best-effort context for the audit row
		}
		if quota != nil {
			d["quota"] = quota.Quota
		}
		if errCode != "" {
			d["error"] = errCode
		}
		b, _ := json.Marshal(d)
		if err := s.reg.LogAuditEvent(r.Context(), actorOf(r), "machine.grant", req.Custodian, string(b)); err != nil {
			slog.Error("machine.grant audit record NOT written; the grant change is unrecorded",
				"custodian", req.Custodian, "actor", actorOf(r), "details", string(b), "err", err)
			return "failed"
		}
		return "recorded"
	}

	// Affiliation present → full grant (seed/replace). Absent → quota-only,
	// preserving whatever affiliation the grant already carries. Guard the
	// quota-only path against a first-time grant that would land with no
	// affiliation (an enroll against it would then fail-close), so the operator
	// gets a clear 400 instead of a grant that silently can't enroll.
	if req.Affiliation != "" {
		if err := s.reg.SetMachineGrant(r.Context(), req.Custodian, req.Quota, req.Affiliation, req.Site, grantedBy); err != nil {
			// A near-miss spelling of an existing unit is the caller's error,
			// not the server's: 400 with the on-file spelling in the message.
			if errors.Is(err, registry.ErrAffiliationVariant) {
				writeError(w, http.StatusBadRequest, "affiliation_variant", err.Error())
				return
			}
			writeError(w, http.StatusInternalServerError, "set_grant", err.Error())
			return
		}
	} else {
		if _, err := s.reg.MachineGrantFor(r.Context(), req.Custodian); errors.Is(err, registry.ErrNotFound) {
			writeError(w, http.StatusBadRequest, "affiliation_required",
				"this custodian has no grant yet; include an affiliation to seed one (a machine inherits its custodian's unit)")
			return
		}
		if err := s.reg.SetMachineQuota(r.Context(), req.Custodian, req.Quota, grantedBy); err != nil {
			writeError(w, http.StatusInternalServerError, "set_quota", err.Error())
			return
		}
	}

	// ADR 1113 — the delegated network-admin entitlement, applied only when the
	// caller said something about it. Ordered AFTER the grant write so it can
	// rely on the grant (and its site) existing.
	if req.NetworkAdmin != nil {
		if err := s.reg.SetNetworkAdmin(r.Context(), req.Custodian, *req.NetworkAdmin, grantedBy); err != nil {
			_ = audit(nil, "set_network_admin")
			writeError(w, http.StatusBadRequest, "set_network_admin", err.Error())
			return
		}
		// Granting network-admin with a zero ceiling is a contradiction: the
		// entitlement says "you may enroll NETWORK-class bearer gear for your
		// own site without the operator", and quota 0 means the first such
		// enrollment fails `no_enrollment_grant`. Whoever grants the authority
		// has already made the decision the quota would be asking about, so
		// seed the working default rather than hand out an authority that
		// cannot be exercised (operator decision).
		//
		// Only when the caller did not state a quota in THIS request and the
		// standing one is 0 — an explicit quota always wins, including a
		// deliberate 0 alongside network_admin:false.
		if *req.NetworkAdmin && req.Quota == 0 {
			if g, err := s.reg.MachineGrantFor(r.Context(), req.Custodian); err == nil && g.Quota == 0 {
				if err := s.reg.SetMachineQuota(r.Context(), req.Custodian, defaultAdminMachineQuota, grantedBy); err != nil {
					_ = audit(nil, "seed_quota")
					writeError(w, http.StatusInternalServerError, "seed_quota", err.Error())
					return
				}
			}
		}
	}

	q, err := s.reg.MachineQuotaFor(r.Context(), req.Custodian)
	if err != nil {
		_ = audit(nil, "")
		writeError(w, http.StatusInternalServerError, "quota_lookup", err.Error())
		return
	}
	// The grant happened, so the answer stays 200; "audit" says whether the
	// required machine.grant record was written, so a caller is never told
	// an unqualified success for an unrecorded change.
	writeJSON(w, http.StatusOK, map[string]any{"quota": q, "audit": audit(q, "")})
}

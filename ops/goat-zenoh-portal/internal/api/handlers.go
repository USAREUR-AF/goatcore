package api

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/decoder"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
)

// --- Health ----------------------------------------------------------

// healthProbeTimeout bounds the registry probe so a locked or wedged SQLite file
// answers "unhealthy" quickly instead of hanging the probe until the client's own
// timeout — a hung healthcheck reads as "no answer", which is a weaker signal than
// an explicit failure.
const healthProbeTimeout = 3 * time.Second

// handleHealth is the portal's liveness signal. It PROBES the registry (one 1-row
// read) rather than returning a constant: the previous static "ok" could not fail,
// so a portal with an unreadable registry — or any portal-shaped process at all —
// answered healthy, and actual portal death surfaced only through two unrelated
// ≥30-minute warning-tier proxies (identity fragility audit §1.3
// violation 5). 503 + a reason when the probe fails, so a container healthcheck,
// a blackbox target, and an operator all learn the same thing at the same time.
func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	body := map[string]any{
		"uptime":  time.Since(s.startedAt).String(),
		"sunset":  s.cfg.SunsetDate,
		"version": APIVersion,
		// Deprecated-/api/vendors* surface use since boot (ADR 1052 Phase 1).
		// The Phase-4 removal gate: drop the vendor routes only once this stays
		// at zero across the deprecation window. See
		// docs/design/vendor-to-org-machine-rename.md.
		"legacy_vendor_surface_hits": s.legacyVendorHits.Load(),
		// F-371 convergence. api_key_bcrypt_compares counts authentications
		// that still fell back to the per-row bcrypt leg since boot; the
		// awaiting count below says how many rows can still cause one. When
		// the awaiting count is 0 fleet-wide and the compare count stops
		// climbing, the fallback leg in resolvePrincipal is dead code and can
		// go. A climbing compare count with a 0 awaiting count would mean
		// something is presenting keys for rows that no longer exist.
		"api_key_bcrypt_compares": APIKeyBcryptCompares(),
		// #1351. Count of operator handles whose directory key is overridden
		// by an env key. Console revocation of those handles has no effect
		// until the env pin is removed. A count only: which handles is in the
		// portal's startup log, not on an unauthenticated endpoint.
		"operator_keys_shadowed": s.operatorKeysShadowed(),
	}
	ctx, cancel := context.WithTimeout(r.Context(), healthProbeTimeout)
	defer cancel()
	// Both reads take a pooled connection of their own, so a write burst on
	// another connection cannot hold the probe past its deadline.
	if n, err := s.reg.CountVendorsAwaitingAPIKeyDigest(ctx); err == nil {
		body["api_key_digest_awaiting"] = n
	}
	if err := s.reg.Ping(ctx); err != nil {
		// The probe error stays in the log (newProblem logs it against the
		// request id); an unauthenticated caller sees a constant reason.
		p := newProblem(w, http.StatusServiceUnavailable, "registry_unavailable", err.Error())
		out := p.members()
		for k, v := range body {
			out[k] = v
		}
		// status is the HTTP status here (RFC 9457); the health word moves to
		// its own member so a reader of the 200 shape still finds it.
		out["health"] = "unhealthy"
		out["registry"] = "unavailable"
		out["reason"] = p.Detail
		writeProblem(w, http.StatusServiceUnavailable, out)
		return
	}
	body["status"] = "ok"
	body["registry"] = "ok"
	writeJSON(w, http.StatusOK, body)
}

// operatorKeysShadowed is the /api/health gauge behind operator_keys_shadowed.
func (s *Server) operatorKeysShadowed() int {
	if s.cfg.OperatorKeys == nil {
		return 0
	}
	return len(s.cfg.OperatorKeys.Shadowed())
}

// --- Vendors ---------------------------------------------------------

type vendorListEntry struct {
	Prefix      string `json:"prefix"`
	DisplayName string `json:"display_name"`
	Approved    bool   `json:"approved"`
	Synthetic   bool   `json:"synthetic"`
	CreatedAt   int64  `json:"created_at"`
	// Sponsorship annotation (ADR 1071) — surfaced so admin views can show
	// who may sponsor and to what level.
	SponsorAuthorized bool   `json:"sponsor_authorized"`
	SponsorMaxTier    string `json:"sponsor_max_tier,omitempty"`
	// Producer data labels (ADR 1075). Raw lanes plus the effective access lane
	// (unlabeled resolves to the per-goatnet default) so the catalog can badge
	// each producer without a second call.
	Fidelity        string `json:"fidelity,omitempty"`
	Context         string `json:"context,omitempty"`
	Access          string `json:"access,omitempty"`
	EffectiveAccess string `json:"effective_access"`
	// DeclaredTopics is the org's live (non-retired) topic-row count. A
	// catalog is a discovery surface: an org with nothing declared has nothing
	// to discover, and on a per-identity slot pool most rows are people who
	// registered and declared nothing. Carried here so the catalog can fold
	// those rows by default without a per-org fan-out.
	DeclaredTopics int64 `json:"declared_topics"`
}

func (s *Server) listVendors(w http.ResponseWriter, r *http.Request) {
	vs, err := s.reg.ListVendors(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	topicCounts, err := s.reg.DeclaredTopicCounts(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	out := make([]vendorListEntry, 0, len(vs))
	for _, v := range vs {
		out = append(out, vendorListEntry{
			Prefix:            v.Prefix,
			DisplayName:       v.DisplayName,
			Approved:          v.ApprovedAt != nil,
			Synthetic:         v.Synthetic,
			CreatedAt:         v.CreatedAt,
			DeclaredTopics:    topicCounts[v.Prefix],
			SponsorAuthorized: v.SponsorAuthorized,
			SponsorMaxTier:    v.SponsorMaxTier,
			Fidelity:          registry.EffectiveFidelity(v.LabelFidelity, v.Synthetic),
			Context:           v.LabelContext,
			Access:            v.LabelAccess,
			EffectiveAccess:   registry.EffectiveAccess(v.LabelAccess, s.cfg.DataLabelAccessDefault),
		})
	}
	// Dual-emit the org listing alongside the deprecated vendors key (ADR 1052
	// Phase 1 — vendor→org machine rename). A migrated client reads `orgs`; an
	// un-migrated one keeps reading `vendors`. Same slice, two keys.
	writeJSON(w, http.StatusOK, map[string]any{"vendors": out, "orgs": out})
}

type createVendorRequest struct {
	Prefix      string `json:"prefix"`
	DisplayName string `json:"display_name"`
	AutoApprove bool   `json:"auto_approve"` // operator-controlled override of OperatorApprovalRequired
	Synthetic   bool   `json:"synthetic"`    // operator-set; flags this vendor as demo/illustrative
	// Optional declared throughput envelope (ADR 1045). When present + valid,
	// stored as the party's edited declaration; otherwise the actor gets the
	// rough class-baseline default. Never gates registration.
	Envelope *envelopeInput `json:"envelope,omitempty"`
	// Sponsorship annotation decided at onboarding (ADR 1071): the onboard
	// path passes the manifest's sponsorship block here so the identity can
	// mint invites / vouch at stage 2 from day one. Operator-set only (this
	// route is operator-gated); defaults to not-authorized.
	SponsorAuthorized bool   `json:"sponsor_authorized"`
	SponsorMaxTier    string `json:"sponsor_max_tier"`
	// SponsorEmail optionally binds the sponsor's Ory login for
	// sponsor-scoped console access (ADR 1072); typically the
	// person-manifest identity.email.
	SponsorEmail string `json:"sponsor_email"`
	// Optional producer data labels (ADR 1075). When present, validated and
	// stamped after registration; the justification-on-restriction gate applies,
	// so a restrictive label with no justification refuses the WHOLE
	// registration (validated before the row is created). Absent = unlabeled,
	// resolving to the per-goatnet access default at read time.
	Labels *setLabelsRequest `json:"labels,omitempty"`
	// Machine-enrollment grant seeded at onboard (ADR 1096): a person onboarded
	// with a machine_quota > 0 can enroll that many machines without a separate
	// operator grant. Affiliation is the custodian's unit every enrolled machine
	// inherits (from the manifest affiliation.id); a quota with no affiliation is
	// skipped (a machine can't inherit an empty unit). Only the slot-pool onboard
	// path seeds it; a bare `POST /api/vendors` leaves it unset.
	MachineQuota int64  `json:"machine_quota,omitempty"`
	Affiliation  string `json:"affiliation,omitempty"`
	Site         string `json:"site,omitempty"`
	// PartnerLanes requests a slot from the PRE-CUT PARTNER BLOCK bound to this
	// lane-set (ADR 1138) instead of the lowest free slot. e.g.
	// ["REL-ALL","REL-PARTNER"]. Order-insensitive.
	//
	// EXPLICIT ON PURPOSE — never inferred from affiliation. Which lanes a
	// partner may read is a release decision with an owner; deriving it from a
	// nation or an affiliation.kind would let a roster edit silently widen who
	// receives data. The caller states it, and the mint is refused if no block
	// carries that exact lane-set (see onboardSlotSystem).
	//
	// Absent = ordinary slot. A partner minted WITHOUT this lands outside every
	// block, matches no partner-<anchor> subject, and reads the whole fabric
	// until someone hand-edits zenoh_partner_read_lanes and restarts the router.
	PartnerLanes []string `json:"partner_lanes,omitempty"`
	// IdentityID is the Kratos identity UUID of the person this slot is being
	// onboarded FOR — the linkage the onboard flow already holds and used to
	// discard, leaving every downstream consumer to re-derive ownership by
	// parsing names (identity fragility audit §1.2). Recorded on the
	// slot; validated as a UUID, never an email or a handle. Optional: an onboard
	// that genuinely has no Ory identity (a system with no person behind it)
	// omits it and the slot keeps an honest NULL. Slot-pool path only.
	IdentityID string `json:"identity_id,omitempty"`
	// DeferCredential onboards the slot WITHOUT minting its fabric credential
	// (mint-on-collect, docs/design/fabric-credential-collection.md). The slot is
	// allocated, the person linkage stamped and the org row created — everything
	// that makes the identity exist — but no api_key and no cert. Their owner
	// mints them later by collecting, which is the only way the plaintext can be
	// handed to the person themselves rather than carried to them in an email.
	//
	// PEOPLE ONLY. A machine cannot complete an Ory login and so can never
	// collect; deferring a machine's credential would strand it. Slot-pool path
	// only — there is no collection surface in per-vendor mode.
	DeferCredential bool `json:"defer_credential,omitempty"`
}

type createVendorResponse struct {
	Prefix      string `json:"prefix"`
	APIKey      string `json:"api_key"`      // one-time return; bcrypt hash kept in DB
	BundleURL   string `json:"bundle_url"`   // one-time tarball download URL
	BundleToken string `json:"bundle_token"` // raw token so the operator can rebuild the URL if needed
	URISAN      string `json:"uri_san"`      // per-vendor URI-SAN minted into the cert
	Approved    bool   `json:"approved"`
	NoteToShare string `json:"note_to_share"`
	// Per-uuid slot-pool mode (ADR 1025) only. SlotID is the assigned anchor
	// (= cert CN = the namespace the system writes under); SystemHandle is the
	// human handle the operator onboarded. Omitted in per-vendor mode.
	SlotID       string `json:"slot_id,omitempty"`
	SystemHandle string `json:"system_handle,omitempty"`
	// IdentityID echoes the person linkage recorded on the slot (empty when the
	// caller supplied none) so the onboarding client can confirm what was written
	// rather than assume it.
	IdentityID string `json:"identity_id,omitempty"`
	// Reused is true when a deferred onboard found the handle already holding a
	// live slot and returned that slot instead of allocating one (#1576). The
	// caller must not roll it back on a later failure: it existed before the call.
	Reused bool `json:"reused,omitempty"`
}

// reuseDeferredSlot answers a deferred onboard whose handle already holds a live
// slot: the person is coming back (a restored archive row, a retried mint), and
// the slot from their first onboarding is still theirs. A deferred onboard never
// returns a credential, so handing back the existing slot mints nothing and
// invalidates nothing — a credential its owner already collected keeps working.
// Refusing with alias_taken, as before #1576, failed every such restore.
//
// It never relinks: a slot already linked to a different identity than the
// caller names is refused with identity_mismatch. The response carries the
// slot's current identity_id so a caller that learns the identity later (the
// native onboard runs the login leg after the slot) can make the same check.
func (s *Server) reuseDeferredSlot(w http.ResponseWriter, r *http.Request, handle, identityID string) {
	slot, err := s.reg.LiveSlotForAlias(r.Context(), handle)
	if err != nil {
		// The allocation guard just saw a live slot for this alias; not finding
		// exactly one now is a race or a duplicate, and guessing is how a person
		// ends up in someone else's namespace.
		writeError(w, http.StatusConflict, "alias_taken",
			fmt.Sprintf("alias %q is already assigned, and its live slot could not be resolved for reuse (%v); revoke the existing one and retry", handle, err))
		return
	}
	anchor := slot.SlotID
	current := ""
	if slot.IdentityID != nil {
		current = *slot.IdentityID
	}
	if identityID != "" && current != "" && current != identityID {
		writeError(w, http.StatusConflict, "identity_mismatch",
			fmt.Sprintf("alias %q holds slot %s linked to identity %s, not %s; refusing to hand one person's slot to another", handle, anchor, current, identityID))
		return
	}
	if identityID != "" && current == "" {
		if err := s.reg.StampSlotIdentity(r.Context(), anchor, identityID); err != nil {
			writeError(w, http.StatusInternalServerError, "stamp_identity", err.Error())
			return
		}
		current = identityID
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "slot-onboard-deferred-reused", handle,
		fmt.Sprintf(`{"slot_idx":%d,"anchor":%q,"identity_id":%q}`, slot.Idx, anchor, current))
	writeJSON(w, http.StatusOK, &createVendorResponse{
		Prefix:       anchor,
		Approved:     true,
		SlotID:       anchor,
		SystemHandle: handle,
		IdentityID:   current,
		Reused:       true,
		NoteToShare: "This handle already held a live slot, so the deferred onboard reused it rather than allocating another. Nothing was minted: " +
			"a fabric credential its owner already collected still works, and an uncollected one is still theirs to collect from the console.",
	})
}

func (s *Server) createVendor(w http.ResponseWriter, r *http.Request) {
	var req createVendorRequest
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	if err := registry.ValidVendorPrefix(req.Prefix); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	if err := capped(fieldCap{"display_name", req.DisplayName, maxDisplayName}); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	// ADR 1075 labels: validate any supplied label block BEFORE creating the
	// producer row, so a restrictive lane with no justification refuses the
	// whole registration rather than leaving a labeled-but-half-created vendor.
	var reqLabels *registry.Labels
	if req.Labels != nil {
		l := registry.Labels{Fidelity: req.Labels.Fidelity, Context: req.Labels.Context, Access: req.Labels.Access, Justification: req.Labels.Justification}
		if err := l.Validate(); err != nil {
			writeError(w, http.StatusBadRequest, "validate", err.Error())
			return
		}
		reqLabels = &l
	}

	// Per-uuid slot-pool mode (ADR 1025): onboarding is allocate-a-slot +
	// mint-CN=anchor, not register-a-vendor + render-ACL. Diverges enough to
	// warrant its own path; req.Prefix is the human system handle there.
	if s.cfg.IdentityModel == config.IdentityModelPerUUID {
		s.onboardSlotSystem(w, r, req)
		return
	}

	apiKey, err := generateAPIKey(req.Prefix)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "gen_key", err.Error())
		return
	}
	hash, digest, err := hashAPIKey(apiKey)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "hash", err.Error())
		return
	}

	display := req.DisplayName
	if display == "" {
		display = req.Prefix
	}
	v := &registry.Vendor{
		Prefix:            req.Prefix,
		APIKeyHash:        hash,
		APIKeySHA256:      digest,
		DisplayName:       display,
		Synthetic:         req.Synthetic,
		SponsorAuthorized: req.SponsorAuthorized,
		SponsorMaxTier:    strings.ToLower(strings.TrimSpace(req.SponsorMaxTier)),
		SponsorEmail:      strings.ToLower(strings.TrimSpace(req.SponsorEmail)),
	}
	approved := req.AutoApprove || !s.cfg.OperatorApprovalRequired
	if approved {
		now := time.Now().Unix()
		approver := "operator"
		v.ApprovedAt = &now
		v.Approver = &approver
	}

	if err := s.reg.CreateVendor(r.Context(), v); err != nil {
		if errors.Is(err, registry.ErrVendorExists) {
			writeError(w, http.StatusConflict, "exists", "org prefix already exists")
			return
		}
		writeError(w, http.StatusInternalServerError, "create", err.Error())
		return
	}

	// Declare the actor's throughput envelope (ADR 1045) — supplied block or
	// rough class-baseline default. Best-effort; never gates registration.
	s.assignInitialEnvelope(r.Context(), req.Prefix, req.Envelope)

	// Stamp the producer's data labels (ADR 1075) when supplied — already
	// validated above, so this only fails on a transient DB error, which we
	// surface rather than swallow (the label set was part of the request).
	if reqLabels != nil {
		if err := s.reg.SetVendorLabels(r.Context(), req.Prefix, *reqLabels); err != nil {
			writeError(w, http.StatusInternalServerError, "labels", err.Error())
			return
		}
		_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.label.set", req.Prefix, labelAuditDetails(*reqLabels))
	}

	// Mint the cert tarball. apiKey is the vendor's portal token — it
	// gets baked into the goat-cli .cbor inside the tarball so the vendor
	// has a one-command `goat profile init` with no separate file.
	res, err := s.minter.Mint(r.Context(), req.Prefix, apiKey, certmint.MintOptions{})
	if err != nil {
		writeError(w, http.StatusInternalServerError, "mint", err.Error())
		return
	}
	if err := s.reg.MarkBundle(r.Context(), req.Prefix, res.BundleToken, res.TarballPath); err != nil {
		writeError(w, http.StatusInternalServerError, "mark_bundle", err.Error())
		return
	}

	// Re-render the ACL when the vendor is approved on creation.
	if approved {
		if err := s.renderACL(r.Context()); err != nil {
			// Don't fail the response; just log. ACL drift is operator-recoverable.
			writeJSON(w, http.StatusOK, &createVendorResponse{
				Prefix:      req.Prefix,
				APIKey:      apiKey,
				BundleURL:   bundleURL(req.Prefix, res.BundleToken),
				BundleToken: res.BundleToken,
				URISAN:      res.URISAN,
				Approved:    approved,
				NoteToShare: "ACL render failed; vendor created but writes will be denied at the router until the next render. Operator: re-render manually. Underlying: " + err.Error(),
			})
			return
		}
	}

	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-register", req.Prefix, "")
	writeJSON(w, http.StatusCreated, &createVendorResponse{
		Prefix:      req.Prefix,
		APIKey:      apiKey,
		BundleURL:   bundleURL(req.Prefix, res.BundleToken),
		BundleToken: res.BundleToken,
		URISAN:      res.URISAN,
		Approved:    approved,
		NoteToShare: "Distribute the cert tarball to the vendor OOB. Share the api_key separately; the inspector portal authenticates with it as a Bearer token. The tarball URL is one-time — fetch within 24 h.",
	})
}

// onboardSlotSystem is the per-uuid-slot-pool (ADR 1025) onboarding path.
// req.Prefix is the human system handle (e.g. "fra-land-bde"). It allocates a
// free pre-baked slot, mints a cert whose CN *is* the slot anchor, and records
// the assignment. The router already enforces the slot rule, so there is NO ACL
// render and NO restart — the whole dynamic step is the cert mint.
//
// The slot anchor doubles as the portal "vendor prefix": topics, the inspector,
// and panoscope all key on the namespace the system writes under (<anchor>/**),
// so a vendor row keyed by the anchor is created with the human handle as its
// display name.
func (s *Server) onboardSlotSystem(w http.ResponseWriter, r *http.Request, req createVendorRequest) {
	handle := req.Prefix

	// The handle is the slot alias (ADR 1039): validate its shape (charset, not
	// UUID-shaped, not a reserved namespace word) before claiming a slot.
	if err := registry.ValidAlias(handle); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_alias", err.Error())
		return
	}
	// Validate the person linkage BEFORE claiming a slot, so a malformed UUID
	// costs nothing rather than burning a slot on a request that must be re-sent.
	identityID := strings.TrimSpace(req.IdentityID)
	if identityID != "" {
		if err := registry.ValidIdentityID(identityID); err != nil {
			writeError(w, http.StatusBadRequest, "invalid_identity_id", err.Error())
			return
		}
	}

	// ADR 1138 — a partner is allocated from the pre-cut block bound to its
	// lane-set, so the CN it is about to be minted with already matches a
	// rendered partner-<anchor> subject: no ACL edit, no router restart.
	//
	// FAIL CLOSED at every branch. Falling back to an ordinary slot is the
	// over-sharing bug (the partner would read the whole fabric, silently), so
	// an undeclared lane-set, an unreadable block file, or a full block all
	// refuse the mint instead.
	var slot *registry.Slot
	var err error
	if len(req.PartnerLanes) > 0 {
		if s.cfg.PartnerSlotBlocksPath == "" {
			writeError(w, http.StatusBadRequest, "partner_blocks_not_configured",
				"partner_lanes was requested but this goatnet declares no partner slot blocks (--partner-slot-blocks-path unset). Refusing rather than issuing an ordinary slot: a partner outside a pre-cut block reads the whole fabric until the ACL is hand-edited and the router restarted.")
			return
		}
		blocks, lerr := access.LoadPartnerSlotBlocks(s.cfg.PartnerSlotBlocksPath)
		if lerr != nil {
			writeError(w, http.StatusInternalServerError, "partner_blocks_unreadable", lerr.Error())
			return
		}
		blk, ok := access.MatchPartnerBlock(blocks, req.PartnerLanes)
		if !ok {
			writeError(w, http.StatusBadRequest, "no_such_partner_lane_set",
				fmt.Sprintf("no pre-cut partner block declares the lane-set %v on this goatnet. Declared: %s. A new lane-set is a governance act: declare it in zenoh_partner_slot_blocks, re-render the router and restart it, then retry.",
					req.PartnerLanes, access.DeclaredLaneSets(blocks)))
			return
		}
		slot, err = s.reg.AllocateSlotIn(r.Context(), handle, blk.From, blk.To)
		if errors.Is(err, registry.ErrPartnerBlockExhausted) {
			writeError(w, http.StatusServiceUnavailable, "partner_block_exhausted",
				fmt.Sprintf("every slot in the partner block [%s] (%d-%d) is taken. Widen the block in zenoh_partner_slot_blocks and take one planned router restart; do NOT issue an ordinary slot to a partner.",
					strings.Join(blk.Lanes, "+"), blk.From, blk.To))
			return
		}
	} else {
		slot, err = s.reg.AllocateSlot(r.Context(), handle)
	}
	if errors.Is(err, registry.ErrAliasTaken) {
		// A partner slot is only reusable if it sits in the block for the lane-set
		// now requested; that check is not made here, so a partner keeps the 409.
		if req.DeferCredential && len(req.PartnerLanes) == 0 {
			s.reuseDeferredSlot(w, r, handle, identityID)
			return
		}
		writeError(w, http.StatusConflict, "alias_taken",
			fmt.Sprintf("alias %q is already assigned to a live slot; pick a different handle or revoke the existing one", handle))
		return
	}
	if errors.Is(err, registry.ErrSlotPoolExhausted) {
		writeError(w, http.StatusServiceUnavailable, "pool_exhausted",
			"slot pool exhausted; onboarding is paused until a planned router restart refills the pool with a larger size (ADR 1025 §6)")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "allocate_slot", err.Error())
		return
	}
	anchor := slot.SlotID

	// Record the person→slot linkage while we still hold it. Fatal on failure:
	// the slot allocation was a write to this same registry, so a failure here is
	// a broken registry, not a partial-success worth keeping — and a slot minted
	// without its linkage is exactly the debt this closes. Nothing is minted yet,
	// so the rollback is clean.
	if identityID != "" {
		if err := s.reg.StampSlotIdentity(r.Context(), anchor, identityID); err != nil {
			_ = s.reg.RevokeSlot(r.Context(), anchor)
			writeError(w, http.StatusInternalServerError, "stamp_identity", err.Error())
			return
		}
	} else {
		// INFO, and phrased as the transient it usually is. The native onboard
		// runs NetBird -> Zenoh -> Ory LAST, deliberately, so at slot allocation
		// the Kratos identity does not exist yet and there is no UUID to send;
		// goat-cli then stamps the linkage in a second call once the login leg
		// returns (cmd/admin.rs, "recorded person->slot linkage").
		//
		// It is INFO rather than WARN because a line saying "ownership for this slot
		// WILL be derived by name join" states as settled a condition the very next
		// step normally repairs. A log line that describes a two-step handshake's first
		// step as a finished outcome costs someone an investigation every time they
		// meet it.
		//
		// The genuine failure — a caller that never stamps — is still visible:
		// it is this line with no "recorded person->slot linkage" following it,
		// and the collect + rotate paths refuse loudly with the repair command.
		slog.Info("slot allocated without an identity linkage yet — expected when the login leg runs after allocation, which the native onboard does; the caller stamps it next with PUT /api/slots/<anchor>/identity. If nothing does, ownership falls back to the alias↔handle name join",
			"anchor", anchor, "handle", handle)
	}

	// Mint-on-collect: a deferred slot must read "no credential exists", and on
	// a pre-minted pool the allocated slot does not — the pre-cut provisioning
	// pass can stamp cert_minted_at on free slots long before anyone owns them,
	// and the collection guard reads any non-NULL stamp as "minted for this person
	// at onboarding" and refuses with `already_minted`. Erase the pool-era stamps
	// so the slot's state says what the
	// deferral means. Fatal on failure for the same reason as the linkage stamp
	// above: a deferred slot that keeps a stale stamp is a person who can never
	// collect, and nothing is minted yet so the rollback is clean.
	if req.DeferCredential {
		if err := s.reg.ClearSlotCert(r.Context(), anchor); err != nil {
			_ = s.reg.RevokeSlot(r.Context(), anchor)
			writeError(w, http.StatusInternalServerError, "clear_slot_cert", err.Error())
			return
		}
	}

	// Mint-on-collect: with the credential deferred there is no key to generate
	// and no hash to store. The org row is created with an EMPTY api_key_hash,
	// which resolvePrincipal already skips — so a deferred identity exists in the
	// registry and authenticates to nothing until its owner collects.
	var apiKey, hash, digest string
	if !req.DeferCredential {
		var err error
		apiKey, err = generateAPIKey(anchor)
		if err != nil {
			_ = s.reg.RevokeSlot(r.Context(), anchor) // give the slot back; nothing minted yet
			writeError(w, http.StatusInternalServerError, "gen_key", err.Error())
			return
		}
		hash, digest, err = hashAPIKey(apiKey)
		if err != nil {
			_ = s.reg.RevokeSlot(r.Context(), anchor)
			writeError(w, http.StatusInternalServerError, "hash", err.Error())
			return
		}
	}

	display := req.DisplayName
	if display == "" {
		display = handle
	}
	now := time.Now().Unix()
	approver := "operator"
	v := &registry.Vendor{
		Prefix:            anchor,
		APIKeyHash:        hash,
		APIKeySHA256:      digest,
		DisplayName:       display,
		Synthetic:         req.Synthetic,
		SponsorAuthorized: req.SponsorAuthorized,
		SponsorMaxTier:    strings.ToLower(strings.TrimSpace(req.SponsorMaxTier)),
		SponsorEmail:      strings.ToLower(strings.TrimSpace(req.SponsorEmail)),
		// Pre-baked ACL admits the slot the moment its cert exists — there is no
		// per-onboard approval gate, so the anchor row is approved on creation.
		ApprovedAt: &now,
		Approver:   &approver,
	}
	if err := s.reg.CreateVendor(r.Context(), v); err != nil {
		_ = s.reg.RevokeSlot(r.Context(), anchor)
		writeError(w, http.StatusInternalServerError, "create", err.Error())
		return
	}

	// Declare the slot system's throughput envelope (ADR 1045). Best-effort.
	s.assignInitialEnvelope(r.Context(), anchor, req.Envelope)

	// Deferred: stop here. The identity exists — slot assigned, person linked,
	// org row approved, envelope declared — and holds no credential. The lazy
	// per-slot store is NOT added either: it is created when the cert first
	// exists, which on this path is at collection. Nothing to roll back, because
	// nothing was minted.
	if req.DeferCredential {
		// The grant is an authorization the sponsor approved at intake; it does
		// not depend on the credential, so it is seeded here too.
		s.seedOnboardMachineGrant(r.Context(), anchor, req, actorOf(r))
		_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "slot-onboard-deferred", handle,
			fmt.Sprintf(`{"slot_idx":%d,"anchor":%q,"identity_id":%q}`, slot.Idx, anchor, identityID))
		writeJSON(w, http.StatusCreated, &createVendorResponse{
			Prefix:       anchor,
			Approved:     true,
			SlotID:       anchor,
			SystemHandle: handle,
			IdentityID:   identityID,
			NoteToShare: "Slot onboarded WITHOUT a fabric credential (mint-on-collect). There is no api_key and no bundle to distribute: " +
				"this person mints their own from the console's fabric-credential page once they can sign in, which is the only way the key " +
				"reaches them rather than passing through an operator. Their mesh setup key and login link are unaffected.",
		})
		return
	}

	// Mint the cert tarball with CN = the slot anchor so the leaf matches the
	// router's pre-baked cert_common_names subject. prefix=anchor also makes the
	// publisher namespace + tarball examples write under <anchor>/**.
	res, err := s.minter.Mint(r.Context(), anchor, apiKey, certmint.MintOptions{CommonName: anchor})
	if err != nil {
		_ = s.reg.DeleteVendor(r.Context(), anchor)
		_ = s.reg.RevokeSlot(r.Context(), anchor)
		writeError(w, http.StatusInternalServerError, "mint", err.Error())
		return
	}
	if err := s.reg.MarkBundle(r.Context(), anchor, res.BundleToken, res.TarballPath); err != nil {
		writeError(w, http.StatusInternalServerError, "mark_bundle", err.Error())
		return
	}
	if err := s.reg.StampSlotCert(r.Context(), anchor, res.BundleToken, res.TarballPath); err != nil {
		writeError(w, http.StatusInternalServerError, "stamp_slot", err.Error())
		return
	}

	// Lazy slot-pool storage (zenoh-storage-taxonomy-sharding.md, F-186 opt.2):
	// when the router no longer pre-cuts a store per pool slot, live-add this
	// slot's <anchor>/** rocksdb store via the admin REST API so its data
	// persists + replays for late-joining subscribers. Best-effort + non-fatal:
	// the slot's live pub/sub already works off the pre-baked ACL, and if the
	// live add fails the next router config render (which declares stores for the
	// active anchors) picks it up. No-op when the store is pre-cut (flag off) or
	// AdminREST is not wired.
	if s.cfg.LazySlotStorage && s.aclRender != nil && s.aclRender.AdminREST != nil {
		ttl := s.aclRender.StorageTTLMs
		if ttl == 0 {
			ttl = 604800000 // 7d default, matches the role
		}
		dir := filepath.Join(s.aclRender.StorageDirPrefix, anchor)
		if err := s.aclRender.AdminREST.AddStorage(r.Context(), anchor, dir, ttl); err != nil {
			slog.Warn("lazy slot storage: live store-add failed (non-fatal; next render persists it)",
				"anchor", anchor, "err", err)
		} else {
			slog.Info("lazy slot storage: added <anchor>/** store at mint", "anchor", anchor)
		}
	}

	s.seedOnboardMachineGrant(r.Context(), anchor, req, actorOf(r))

	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "slot-onboard", handle,
		fmt.Sprintf(`{"slot_idx":%d,"anchor":%q,"identity_id":%q}`, slot.Idx, anchor, identityID))
	writeJSON(w, http.StatusCreated, &createVendorResponse{
		Prefix:       anchor,
		APIKey:       apiKey,
		BundleURL:    bundleURL(anchor, res.BundleToken),
		BundleToken:  res.BundleToken,
		URISAN:       res.URISAN,
		Approved:     true,
		SlotID:       anchor,
		SystemHandle: handle,
		IdentityID:   identityID,
		NoteToShare: "Per-uuid slot onboarded. The system's write namespace, cert CN, and portal prefix are all the slot anchor (slot_id). " +
			"Distribute the cert tarball OOB; share the api_key separately as the inspector Bearer token. No router restart was needed — the slot rule was pre-baked.",
	})
}

// rotateVendor handles POST /api/vendors/{vendor}/rotate (operator-gated). It
// re-issues a peer's leaf cert IN PLACE — no slot allocation, no new anchor.
// This is the "rotate" sibling of revocation (ADR 1031): the answer to expiry
// renewal, post-compromise rotation, and retrofitting a URI-SAN scheme onto
// already-issued certs without tearing down identity.
//
// {vendor} is the existing anchor (= vendor prefix). Everything identity-bearing
// stays put — anchor / CN / alias / slot assignment / write namespace / the
// pre-baked ACL are all unchanged; only the leaf cert (and, necessarily, the
// portal api_key — the old plaintext is unrecoverable) is fresh. Mirrors the
// onboard mint exactly, minus AllocateSlot + CreateVendor.
func (s *Server) rotateVendor(w http.ResponseWriter, r *http.Request) {
	prefix := r.PathValue("vendor")
	if prefix == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing org prefix")
		return
	}
	s.rotateAnchor(w, r, prefix)
}

// rotateAnchor is the rotation itself, shared by the anchor-keyed CLI path
// (rotateVendor) and the identity-keyed console path (rotateByIdentity) so the
// two doors cannot drift: same mint, same registry stamps, same audit action.
func (s *Server) rotateAnchor(w http.ResponseWriter, r *http.Request, prefix string) {
	// The peer must already exist — rotation re-issues, it does not create.
	v, err := s.reg.GetVendor(r.Context(), prefix)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "notfound",
			"no such peer; rotation re-issues an existing cert in place — use POST /api/vendors to onboard a new one")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}

	// Fresh portal token + hash; swap the hash in place. Identity is untouched;
	// only the credential rotates. The new key rides the re-minted .cbor bundle.
	apiKey, err := generateAPIKey(prefix)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "gen_key", err.Error())
		return
	}
	hash, digest, err := hashAPIKey(apiKey)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "hash", err.Error())
		return
	}
	if err := s.reg.UpdateVendorAPIKey(r.Context(), prefix, hash, digest); err != nil {
		writeError(w, http.StatusInternalServerError, "update_key", err.Error())
		return
	}

	// Mint a fresh leaf for the SAME anchor. In per-uuid-slot-pool mode the CN
	// must be the bare anchor (matches the router's pre-baked cert_common_names);
	// per-vendor mode keeps the default "<prefix>-trial-peer" CN. Either way the
	// URI-SAN is re-derived from the CURRENT minter config — that's what carries
	// the new scheme onto the cert.
	var mintOpts certmint.MintOptions
	slotMode := s.cfg.IdentityModel == config.IdentityModelPerUUID
	if slotMode {
		mintOpts.CommonName = prefix
	}
	// A machine's leaf carries serverAuth (see machine enroll: it doubles as
	// the pod router's listen cert). Rotation re-issues in place and must not
	// narrow the EKU set, or the rotated pod stops handshaking.
	if _, mErr := s.reg.MachineByAnchor(r.Context(), prefix); mErr == nil {
		mintOpts.ServerAuth = true
	} else if !errors.Is(mErr, registry.ErrNotFound) {
		writeError(w, http.StatusInternalServerError, "db", mErr.Error())
		return
	}
	res, err := s.minter.Mint(r.Context(), prefix, apiKey, mintOpts)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "mint", err.Error())
		return
	}
	if err := s.reg.MarkBundle(r.Context(), prefix, res.BundleToken, res.TarballPath); err != nil {
		writeError(w, http.StatusInternalServerError, "mark_bundle", err.Error())
		return
	}
	// In slot mode, point the slot row's bundle at the fresh tarball too.
	// StampSlotCert is re-mint-aware: it overwrites the bundle pointer +
	// minted timestamps and leaves the assignment intact.
	if slotMode {
		if err := s.reg.StampSlotCert(r.Context(), prefix, res.BundleToken, res.TarballPath); err != nil {
			writeError(w, http.StatusInternalServerError, "stamp_slot", err.Error())
			return
		}
	}

	// No ACL render and no router restart: the anchor/CN the ACL keys on is
	// unchanged, so the existing rule already admits the fresh leaf.
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-rotate", prefix,
		fmt.Sprintf(`{"uri_san":%q,"slot_mode":%t}`, res.URISAN, slotMode))
	resp := &createVendorResponse{
		Prefix:      prefix,
		APIKey:      apiKey,
		BundleURL:   bundleURL(prefix, res.BundleToken),
		BundleToken: res.BundleToken,
		URISAN:      res.URISAN,
		Approved:    v.ApprovedAt != nil,
		NoteToShare: "Cert re-issued in place — same anchor, alias, write namespace, and ACL; only the leaf cert + portal api_key are new. " +
			"Swap the tarball's cert+key on the peer and restart it. The tarball URL is one-time — fetch within 24 h.",
	}
	if slotMode {
		resp.SlotID = prefix
		resp.SystemHandle = v.DisplayName
	}
	writeJSON(w, http.StatusOK, resp)
}

// handleListAliases serves the slot_id↔alias map the display layer caches
// (ADR 1039). Operator-gated read; reveals names only, not access. The UUID
// (slot_id) stays canonical — this is a presentation projection.
func (s *Server) handleListAliases(w http.ResponseWriter, r *http.Request) {
	aliases, err := s.reg.ListAliases(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"aliases": aliases})
}

// --- Topics ----------------------------------------------------------

type topicEntry struct {
	ID int64 `json:"id"`
	// OrgPrefix is dual-emitted alongside the deprecated VendorPrefix (ADR 1052
	// Phase 1 — vendor→org machine rename). Same anchor value, two keys, so a
	// migrated client can read `org_prefix` while an un-migrated one keeps
	// reading `vendor_prefix`. See docs/design/vendor-to-org-machine-rename.md.
	OrgPrefix        string `json:"org_prefix"`
	VendorPrefix     string `json:"vendor_prefix"` // deprecated alias of org_prefix
	KeyPattern       string `json:"key_pattern"`
	Format           string `json:"format"`
	SchemaRef        string `json:"schema_ref,omitempty"`
	Description      string `json:"description,omitempty"`
	PublishToStorage bool   `json:"publish_to_storage"`
	LastSeenAt       *int64 `json:"last_seen_at,omitempty"`
	PublishCount     int64  `json:"publish_count"`
	CreatedAt        int64  `json:"created_at"`
}

func toEntry(t *registry.Topic) topicEntry {
	return topicEntry{
		ID:               t.ID,
		OrgPrefix:        t.VendorPrefix,
		VendorPrefix:     t.VendorPrefix,
		KeyPattern:       t.KeyPattern,
		Format:           t.Format,
		SchemaRef:        t.SchemaRef,
		Description:      t.Description,
		PublishToStorage: t.PublishToStorage,
		LastSeenAt:       t.LastSeenAt,
		PublishCount:     t.PublishCount,
		CreatedAt:        t.CreatedAt,
	}
}

func (s *Server) listTopics(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	ts, err := s.reg.ListVendorTopics(r.Context(), vendor)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	out := make([]topicEntry, 0, len(ts))
	for i := range ts {
		out = append(out, toEntry(&ts[i]))
	}
	writeJSON(w, http.StatusOK, map[string]any{"topics": out})
}

type createTopicRequest struct {
	KeyPattern       string `json:"key_pattern"`
	Format           string `json:"format"`
	SchemaRef        string `json:"schema_ref"`
	Description      string `json:"description"`
	PublishToStorage bool   `json:"publish_to_storage"`
}

func (s *Server) createTopic(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	var req createTopicRequest
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	if err := capped(
		fieldCap{"key_pattern", req.KeyPattern, maxKeyPattern},
		fieldCap{"format", req.Format, maxFormat},
		fieldCap{"schema_ref", req.SchemaRef, maxSchemaRef},
		fieldCap{"description", req.Description, maxDescription},
	); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	// A topic belongs to an org; asking for one under an org that does not exist
	// is a 404 the caller can act on, not the registry's foreign-key failure
	// surfacing as a 500 (found by the e2e goat suite).
	if _, err := s.reg.GetVendor(r.Context(), vendor); err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			writeError(w, http.StatusNotFound, "notfound", fmt.Sprintf("no org with prefix %q — create it first (goat admin org create %s)", vendor, vendor))
			return
		}
		writeRegistryError(w, err)
		return
	}
	t := &registry.Topic{
		VendorPrefix:     vendor,
		KeyPattern:       req.KeyPattern,
		Format:           req.Format,
		SchemaRef:        req.SchemaRef,
		Description:      req.Description,
		PublishToStorage: req.PublishToStorage,
	}
	id, err := s.reg.CreateTopic(r.Context(), t)
	if err != nil {
		if errors.Is(err, registry.ErrTopicExists) {
			writeError(w, http.StatusConflict, "exists", "topic with this key_pattern already exists for this vendor")
			return
		}
		writeRegistryError(w, err)
		return
	}
	t.ID = id
	_ = s.reg.LogAuditEvent(r.Context(), vendor, "topic-create", req.KeyPattern, "")
	writeJSON(w, http.StatusCreated, toEntry(t))
}

func (s *Server) getTopic(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil {
		writeError(w, http.StatusBadRequest, "decode", "id must be integer")
		return
	}
	t, err := s.reg.GetTopic(r.Context(), id)
	if err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			writeError(w, http.StatusNotFound, "notfound", "topic not found")
			return
		}
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if t.VendorPrefix != vendor {
		writeError(w, http.StatusNotFound, "notfound", "topic not found under this vendor")
		return
	}
	writeJSON(w, http.StatusOK, toEntry(t))
}

func (s *Server) updateTopic(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil {
		writeError(w, http.StatusBadRequest, "decode", "id must be integer")
		return
	}
	t, err := s.reg.GetTopic(r.Context(), id)
	if err != nil {
		writeError(w, http.StatusNotFound, "notfound", "topic not found")
		return
	}
	if t.VendorPrefix != vendor {
		writeError(w, http.StatusNotFound, "notfound", "topic not found under this vendor")
		return
	}
	var req struct {
		Format           *string `json:"format"`
		SchemaRef        *string `json:"schema_ref"`
		Description      *string `json:"description"`
		PublishToStorage *bool   `json:"publish_to_storage"`
	}
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	if req.Format == nil && req.SchemaRef == nil && req.Description == nil && req.PublishToStorage == nil {
		writeError(w, http.StatusBadRequest, "validate",
			"request body must include at least one mutable field (format, schema_ref, description, publish_to_storage)")
		return
	}
	if err := capped(
		fieldCap{"format", deref(req.Format), maxFormat},
		fieldCap{"schema_ref", deref(req.SchemaRef), maxSchemaRef},
		fieldCap{"description", deref(req.Description), maxDescription},
	); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	if req.Format != nil {
		// Write-time format ↔ FDS check. If a schema is already uploaded
		// for this topic and the operator is moving format to
		// "protobuf:<Name>", reject the PATCH unless <Name> is a
		// top-level message in the latest .fds. Mirrors the read-side
		// stethoscope schema-body check and closes the DRS-RCA loop.
		if verr := s.validateFormatAgainstLatestSchema(r.Context(), id, *req.Format); verr != nil {
			var fme *decoder.FormatFDSMismatchError
			if errors.As(verr, &fme) {
				detail := verr.Error()
				if sugg := fme.Suggestion(); sugg != "" {
					detail += fmt.Sprintf(" — most likely correct: PATCH topic format to %q, or rebuild the .fds with %q at top-level.", sugg, fme.DeclaredName)
				}
				writeError(w, http.StatusUnprocessableEntity, "format_mismatch", detail)
				return
			}
			writeError(w, http.StatusBadRequest, "fds_decode", verr.Error())
			return
		}
		t.Format = *req.Format
	}
	if req.SchemaRef != nil {
		t.SchemaRef = *req.SchemaRef
	}
	if req.Description != nil {
		t.Description = *req.Description
	}
	if req.PublishToStorage != nil {
		t.PublishToStorage = *req.PublishToStorage
	}
	if err := s.reg.UpdateTopic(r.Context(), t); err != nil {
		writeRegistryError(w, err)
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), vendor, "topic-update", t.KeyPattern, "")
	writeJSON(w, http.StatusOK, toEntry(t))
}

// validateFormatAgainstLatestSchema enforces the same `protobuf:<Name>`
// ↔ FDS-top-level-message rule the schema-upload path enforces, against
// the topic's latest already-uploaded schema body. Returns nil when no
// schema is uploaded yet (the gate is deferred to upload-time) or when
// the format is not a named protobuf format.
func (s *Server) validateFormatAgainstLatestSchema(ctx context.Context, topicID int64, format string) error {
	if s.schemas == nil {
		return nil
	}
	pbKind, pbName := decoder.ParseProtobufFormat(format)
	xmlKind, xmlName := decoder.ParseXMLFormat(format)
	// Only the schema-bound colon-suffix formats gate against a body;
	// bare "xml"/json/raw/… carry no symbol to match.
	if (pbKind != "protobuf" || pbName == "") && (xmlKind != "xml" || xmlName == "") {
		return nil
	}
	row, err := s.reg.GetLatestSchemaForTopic(ctx, topicID)
	if err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			return nil
		}
		return nil
	}
	rc, err := s.schemas.Open(row.FilePath)
	if err != nil {
		return nil
	}
	defer rc.Close()
	body, err := io.ReadAll(rc)
	if err != nil {
		return nil
	}
	if xmlKind == "xml" && xmlName != "" {
		return decoder.ValidateFormatAgainstXSD(format, body)
	}
	return decoder.ValidateFormatAgainstFDS(format, body)
}

// deleteVendor removes a fully-tombstoned vendor.
//
// Refuses with 409 if the vendor still owns any non-retired topics — the
// operator must retire (or purge) those via
// DELETE /api/vendors/{v}/topics/{id}[?purge=true] first.
// Schema FK is ON DELETE CASCADE, so once topics are retired the cascade
// cleans up the retired-topic rows + their schemas + code-example rows in
// one shot.
//
// Operator-scoped (not vendor-scoped) — only the bearer of the operator
// API key can prune the registry. A vendor cannot delete itself.
type updateVendorRequest struct {
	DisplayName *string `json:"display_name"`
}

// updateVendor patches mutable fields on an existing vendor row. Today
// the only mutable field is display_name; expand the request struct +
// the registry method as new safely-mutable fields appear. Operator-
// scoped (api_key_hash, approval state, bundle token, synthetic flag
// stay immutable from this endpoint by design).
func (s *Server) updateVendor(w http.ResponseWriter, r *http.Request) {
	prefix := r.PathValue("vendor")
	if prefix == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing org prefix")
		return
	}
	var req updateVendorRequest
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	if req.DisplayName == nil {
		writeError(w, http.StatusBadRequest, "validate", "request body must include at least one mutable field (display_name)")
		return
	}
	if *req.DisplayName == "" {
		writeError(w, http.StatusBadRequest, "validate", "display_name must be non-empty")
		return
	}
	if err := capped(fieldCap{"display_name", *req.DisplayName, maxDisplayName}); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	if err := s.reg.UpdateVendorDisplayName(r.Context(), prefix, *req.DisplayName); err != nil {
		if err == registry.ErrNotFound {
			writeError(w, http.StatusNotFound, "notfound", "vendor not found")
			return
		}
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	v, err := s.reg.GetVendor(r.Context(), prefix)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-update", prefix, "display_name")
	writeJSON(w, http.StatusOK, vendorListEntry{
		Prefix:      v.Prefix,
		DisplayName: v.DisplayName,
		Approved:    v.ApprovedAt != nil,
		Synthetic:   v.Synthetic,
		CreatedAt:   v.CreatedAt,
	})
}

type setSponsorshipRequest struct {
	Authorized bool   `json:"authorized"`
	MaxTier    string `json:"max_tier"`
	// Email optionally binds the sponsor's Ory login so the id-admin-console
	// can resolve their session to this identity (sponsor-scoped console
	// access, ADR 1072). Cleared on revoke along with the rest.
	Email string `json:"email,omitempty"`
}

// setVendorSponsorship grants or revokes an identity's sponsorship
// authorization (ADR 1071) — the case-by-case lever for identities that
// didn't get it by tier default at onboarding. Operator-scoped. Granting
// requires a valid max_tier (the level cap is part of the decision, never
// implicit); revoking clears both fields.
func (s *Server) setVendorSponsorship(w http.ResponseWriter, r *http.Request) {
	prefix := r.PathValue("vendor")
	if prefix == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing org prefix")
		return
	}
	var req setSponsorshipRequest
	if err := readJSON(w, r, &req); err != nil {
		writeDecodeError(w, err)
		return
	}
	maxTier := strings.ToLower(strings.TrimSpace(req.MaxTier))
	email := strings.ToLower(strings.TrimSpace(req.Email))
	if req.Authorized {
		h, herr := s.intakeHints()
		if herr != nil {
			writeError(w, http.StatusInternalServerError, "hints_error", "onboarding config unavailable")
			return
		}
		if _, ok := h.tierRank(maxTier); !ok {
			writeError(w, http.StatusBadRequest, "validate",
				"granting sponsorship requires a valid max_tier (the highest tier this identity may sponsor)")
			return
		}
		if email != "" && !strings.Contains(email, "@") {
			writeError(w, http.StatusBadRequest, "validate", "email must be a login email address")
			return
		}
	} else {
		maxTier = ""
		email = ""
	}
	// Accept either the uuid anchor OR the human handle (per-uuid slot-pool
	// mode keys the row by anchor; mirror the deleteVendor resolution).
	if s.cfg.IdentityModel == config.IdentityModelPerUUID {
		if anchor, err := s.reg.AnchorForHandle(r.Context(), prefix); err == nil {
			prefix = anchor
		}
	}
	if err := s.reg.UpdateVendorSponsorship(r.Context(), prefix, req.Authorized, maxTier, email); err != nil {
		if err == registry.ErrNotFound {
			writeError(w, http.StatusNotFound, "notfound", "identity not found")
			return
		}
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor.sponsorship.set", prefix,
		fmt.Sprintf(`{"authorized":%t,"max_tier":%q,"email_bound":%t}`, req.Authorized, maxTier, email != ""))
	writeJSON(w, http.StatusOK, map[string]any{
		"prefix": prefix, "sponsor_authorized": req.Authorized, "sponsor_max_tier": maxTier,
		"sponsor_email_bound": email != "",
	})
}

func (s *Server) deleteVendor(w http.ResponseWriter, r *http.Request) {
	prefix := r.PathValue("vendor")
	if prefix == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing org prefix")
		return
	}
	// Accept either the uuid anchor OR the human handle the onboard POST issued.
	// In per-uuid-slot-pool mode the vendor row + slot are keyed by the anchor,
	// so a DELETE /api/vendors/<handle> would match nothing — 404, leaving an
	// orphaned org row that 409s the next onboard (the partner
	// onboard failure). Resolve a live handle to its anchor first; an anchor (or
	// an unknown id) passes through unchanged and 404s only when truly absent.
	if s.cfg.IdentityModel == config.IdentityModelPerUUID {
		if anchor, err := s.reg.AnchorForHandle(r.Context(), prefix); err == nil {
			prefix = anchor
		}
	}
	delErr := s.reg.DeleteVendor(r.Context(), prefix)
	if delErr != nil && delErr != registry.ErrNotFound {
		// Vendor still exists (active topics, or a DB error) — do NOT touch
		// the slot. Distinguishes "has active topics" from generic DB errors.
		if strings.Contains(delErr.Error(), "non-retired topic") {
			writeError(w, http.StatusConflict, "conflict", delErr.Error())
			return
		}
		writeError(w, http.StatusInternalServerError, "db", delErr.Error())
		return
	}

	// F-190 — per-uuid-slot-pool mode (ADR 1025): the ACL is a pre-baked
	// whole-pool rule, so onboarding renders no per-vendor ACL and offboarding
	// removes none either (symmetry with onboardSlotSystem). The offboard work
	// is freeing the SLOT: clear assigned_to so the alias drops from
	// /api/aliases and the index returns to the pool. RevokeAssignedSlot also
	// reconciles a slot orphaned by a prior delete (vendor row already gone but
	// slot still assigned) — so a DELETE on an absent vendor still 204s when it
	// frees a slot, and 404s only when there was nothing to delete or free.
	if s.cfg.IdentityModel == config.IdentityModelPerUUID {
		slotFreed, slotErr := s.reg.RevokeAssignedSlot(r.Context(), prefix)
		if slotErr != nil {
			slog.Error("slot revoke failed on vendor delete", "vendor", prefix, "err", slotErr)
		}
		if delErr == registry.ErrNotFound && !slotFreed {
			writeError(w, http.StatusNotFound, "notfound", "vendor not found")
			return
		}
		_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-delete", prefix,
			fmt.Sprintf(`{"slot_freed":%t}`, slotFreed))
		w.WriteHeader(http.StatusNoContent)
		return
	}

	// Per-vendor mode: a missing row is a 404.
	if delErr == registry.ErrNotFound {
		writeError(w, http.StatusNotFound, "notfound", "vendor not found")
		return
	}
	// 83F revoke-path symmetry: tear down the four wire-layer marker
	// stanzas + per-vendor rocksdb dir + SIGHUP. If this fails the
	// registry-row delete is still effective but the wire layer leaks
	// the vendor's ACL until reconciled — log loudly so the operator
	// catches it. Returning 200 with a body-warning beats 500ing here:
	// the caller needs to know the registry side succeeded so a retry
	// targets the wire-layer not the registry.
	if err := s.aclRender.RemoveVendor(r.Context(), prefix); err != nil {
		slog.Error("acl wire-layer removal failed after registry delete",
			"vendor", prefix,
			"err", err)
		_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-delete-acl-failed", prefix, err.Error())
		writeJSON(w, http.StatusOK, map[string]string{
			"warning": fmt.Sprintf("registry-row deleted but wire-layer revocation failed: %s; re-run DELETE to reconcile", err.Error()),
		})
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "vendor-delete", prefix, "")
	w.WriteHeader(http.StatusNoContent)
}

// deleteOrRetireTopic dispatches DELETE /api/vendors/{vendor}/topics/{id}.
//
// Default (no query param) is **retire** — flips retired_at to now and
// keeps the row as an audit tombstone. The
// UNIQUE(vendor_prefix, key_pattern) slot remains occupied, so
// re-registering the same key_pattern is refused. This is the
// pre-existing behavior; vendors and the topics-page UI rely on it.
//
// Pass **?purge=true** to hard-delete the row: the registry row goes,
// FK CASCADE wipes topic_schemas + topic_code_examples rows, and the
// per-topic on-disk schema directory is best-effort removed. The
// (vendor_prefix, key_pattern) slot is freed for reuse. Audit-trail
// preservation is the caller's responsibility — LogAuditEvent fires
// before the deletion so audit_events carries the record.
func (s *Server) deleteOrRetireTopic(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil {
		writeError(w, http.StatusBadRequest, "decode", "id must be integer")
		return
	}
	t, err := s.reg.GetTopic(r.Context(), id)
	if err != nil || t.VendorPrefix != vendor {
		writeError(w, http.StatusNotFound, "notfound", "topic not found under this vendor")
		return
	}
	purge := r.URL.Query().Get("purge") == "true"
	if !purge {
		if err := s.reg.RetireTopic(r.Context(), id); err != nil {
			writeError(w, http.StatusInternalServerError, "db", err.Error())
			return
		}
		_ = s.reg.LogAuditEvent(r.Context(), vendor, "topic-retire", t.KeyPattern, "")
		w.WriteHeader(http.StatusNoContent)
		return
	}
	// Hard delete. Audit-log first so the row is reconstructible even
	// after the DB row is gone.
	_ = s.reg.LogAuditEvent(r.Context(), vendor, "topic-delete", t.KeyPattern, "")
	if err := s.reg.DeleteTopic(r.Context(), id); err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if s.schemas != nil {
		// Best effort — orphan files are harmless (next topic gets a
		// fresh autoincrement id), but cleaning up keeps the disk tidy.
		_ = s.schemas.RemoveTopicDir(id)
	}
	w.WriteHeader(http.StatusNoContent)
}

// --- Inspector -------------------------------------------------------

func (s *Server) handleLatest(w http.ResponseWriter, r *http.Request) {
	key := r.URL.Query().Get("key")
	if key == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing key= query param")
		return
	}
	samples, err := s.zenoh.Latest(r.Context(), key)
	if err != nil {
		writeError(w, http.StatusBadGateway, "zenoh", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"samples": samples, "key": key})
}

func (s *Server) handleRecent(w http.ResponseWriter, r *http.Request) {
	key := r.URL.Query().Get("key")
	if key == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing key= query param")
		return
	}
	n := 20
	if v := r.URL.Query().Get("n"); v != "" {
		if parsed, err := strconv.Atoi(v); err == nil && parsed > 0 && parsed <= 1000 {
			n = parsed
		}
	}
	samples, err := s.zenoh.Recent(r.Context(), key, n)
	if err != nil {
		writeError(w, http.StatusBadGateway, "zenoh", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"samples": samples, "key": key, "n": n})
}

func (s *Server) handleStream(w http.ResponseWriter, r *http.Request) {
	key := r.URL.Query().Get("key")
	if key == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing key= query param")
		return
	}
	// Recurring-read bound: reject cross-owner wildcards BEFORE SSE headers
	// go out so the client gets a real 400 with the why (a `**`-wide stream
	// = a whole-tree walk per poll — the wedge factor the crawler removes).
	if err := zenohproxy.ValidateStreamKey(key); err != nil {
		writeError(w, http.StatusBadRequest, "stream", err.Error())
		return
	}
	release, ok := s.streams.acquire(PrincipalFromContext(r.Context()), s.cfg.MaxStreamsPerPrincipal)
	if !ok {
		// Not 429: the proxy's errors middleware replaces every 429 with its
		// own page (TestPortal_NeverProducesA429). 409 says the request
		// conflicts with what this caller already holds open.
		writeError(w, http.StatusConflict, "stream_limit", fmt.Sprintf(
			"this key already has %d live streams open; close one (an inspector tab) and retry",
			s.cfg.MaxStreamsPerPrincipal))
		return
	}
	defer release()
	if err := s.zenoh.StreamSSE(r.Context(), w, key, time.Second); err != nil {
		// Once SSE headers are written we can't switch to JSON error; log
		// and let the client retry.
		return
	}
}

// --- Vendor bundle download ------------------------------------------

func (s *Server) handleVendorBundle(w http.ResponseWriter, r *http.Request) {
	vendor := r.PathValue("vendor")
	token := r.PathValue("token")
	// Token comes through with the .tar.gz extension; strip it.
	token = strings.TrimSuffix(token, ".tar.gz")
	if vendor == "" || token == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing vendor or token")
		return
	}
	path, err := s.reg.ConsumeBundleToken(r.Context(), vendor, token)
	if err != nil {
		writeError(w, http.StatusNotFound, "notfound", "bundle token not found or already consumed")
		return
	}
	w.Header().Set("Content-Type", "application/gzip")
	w.Header().Set("Content-Disposition", "attachment; filename="+filepath.Base(path))
	http.ServeFile(w, r, path)

	// Record the fetch AFTER serving. A bundle that is minted but never
	// collected is the state that stalls an onboarding silently, and nothing
	// recorded it before this: the token is deliberately not consumed on
	// download, so token state cannot be the signal. Both writes are
	// best-effort — the person already has their bundle by here, and failing
	// their download because we could not write an observability row would
	// trade a working onboarding for a status column.
	if err := s.reg.RecordBundleDownload(r.Context(), vendor); err != nil {
		slog.Warn("record bundle download failed", "vendor", vendor, "err", err)
	}
	if err := s.reg.LogAuditEvent(r.Context(), vendor, "bundle-download", vendor,
		fmt.Sprintf(`{"bundle":%q}`, filepath.Base(path))); err != nil {
		slog.Warn("audit bundle download failed", "vendor", vendor, "err", err)
	}
}

// --- helpers ----------------------------------------------------------

func generateAPIKey(prefix string) (string, error) {
	// 32 bytes of entropy → hex (64 chars). Token prefix improves
	// operator-readability when bearer tokens leak into logs.
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return "vp_" + prefix + "_" + hex.EncodeToString(b), nil
}

func bundleURL(vendor, token string) string {
	// ADR 1052 Phase 3 default-flip: mint the org path. The legacy
	// /api/vendor-bundle route still serves (back-compat) for any already-issued
	// URL, but new mints use /api/org-bundle so they don't feed the Phase-4
	// legacy-surface counter. Both routes share the {vendor} path param.
	return "/api/org-bundle/" + vendor + "/" + token + ".tar.gz"
}

// renderACL pulls the approved-vendor list out of the registry and asks
// the renderer to write + reload zenohd's config.
func (s *Server) renderACL(ctx context.Context) error {
	prefixes, err := s.reg.ApprovedVendorPrefixes(ctx)
	if err != nil {
		return err
	}
	return s.aclRender.Render(ctx, prefixes)
}

// handleAuditEvents serves the durable audit rows (operator-gated). The
// id-console's audit page reads the onboarding trail from here — its own ring
// buffer is per-container and dies on redeploy (ADR 0860 interim).
func (s *Server) handleAuditEvents(w http.ResponseWriter, r *http.Request) {
	limit := 200
	if v := r.URL.Query().Get("limit"); v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			limit = n
		}
	}
	events, err := s.reg.AuditEvents(r.Context(), r.URL.Query().Get("action_prefix"), limit)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "audit_read", "could not read audit events")
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"events": events})
}

// handleCatalog serves the whole declared surface of the goatnet in one
// response: every org, its topics, and each topic's schema versions.
//
// This is the discovery entry point. Any authenticated principal may read it,
// slot keys included — topic visibility is advisory, not access (ADR 1028), so
// a newly-onboarded publisher can see what else is on the fabric without
// asking an operator for a second credential.
//
// `?include=empty` keeps orgs that have declared no topics. The default omits
// them and reports how many were omitted in totals.orgs_without_topics.
func (s *Server) handleCatalog(w http.ResponseWriter, r *http.Request) {
	cat, err := s.reg.ListCatalog(r.Context(), r.URL.Query().Get("include") == "empty")
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, cat)
}

// seedOnboardMachineGrant seeds the machine-enrollment grant (ADR 1096) so a
// person onboarded with a machine_quota can enroll machines without a separate
// operator grant. The custodian is the slot anchor — the same id goat machine
// ls/enroll resolve. It runs on BOTH onboard paths: the immediate mint and the
// deferred (mint-on-collect) one, because the grant is an authorization approved
// at intake, independent of when the credential is minted. Best-effort: the slot
// already exists, so a grant hiccup must not fail the onboard; the operator can
// still `goat machine grant` after the fact.
func (s *Server) seedOnboardMachineGrant(ctx context.Context, anchor string, req createVendorRequest, actor string) {
	if req.MachineQuota <= 0 {
		return
	}
	aff := strings.TrimSpace(req.Affiliation)
	// Request-derived strings are stripped of line breaks before they reach a
	// log line, so a crafted value cannot forge a second entry.
	logAnchor := logLineSafe(anchor)
	if aff == "" {
		slog.Warn("machine_quota set but no affiliation on onboard — grant not seeded (a machine inherits its custodian's unit); grant later with `goat machine grant`",
			"anchor", logAnchor)
		return
	}
	if err := s.reg.SetMachineGrant(ctx, anchor, req.MachineQuota, aff, strings.TrimSpace(req.Site), actor); err != nil {
		slog.Warn("machine grant seed failed at onboard (non-fatal; grant later with `goat machine grant`)",
			"anchor", logAnchor, "err", logLineSafe(err.Error()))
		return
	}
	slog.Info("seeded machine enrollment grant at onboard", "anchor", logAnchor, "affiliation", logLineSafe(aff))
}

// logLineSafe removes carriage returns and line feeds from a value bound for a
// log line.
func logLineSafe(v string) string {
	return strings.ReplaceAll(strings.ReplaceAll(v, "\n", ""), "\r", "")
}

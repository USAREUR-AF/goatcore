package api

import (
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"path/filepath"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Mint-on-collect: a person's fabric credential is minted when they come and
// collect it, not when an operator onboards them.
//
// The finding that forces this shape: a portal API key CANNOT be shown after it
// is minted. The registry keeps a bcrypt hash and the plaintext is returned
// exactly once (see rotateVendor, which says outright that the old plaintext is
// unrecoverable). So a credential minted at onboard can only be delivered by
// carrying it out to the person in an email, or by persisting the plaintext so
// it can be re-read later. The second puts a live bearer token at rest for no
// functional gain. The third option is this one: don't mint until they ask.
//
// SCOPE — PEOPLE ONLY. A machine cannot complete an Ory login, so it can never
// collect. Machine enrollment keeps minting at enrollment and delivering to a
// human custodian (ADR 1096). Nothing here may be reached from that path.
//
// TRUST BOUNDARY. The collector authenticates to goat-id-console (Ory, through
// oauth2-proxy); the console calls this with its operator bearer and names the
// authenticated person by Kratos identity UUID. Holding an operator key IS the
// trust boundary (the same delegation X-Goat-Operator-Actor already relies on,
// F-207). The portal then resolves identity → slot ITSELF: the caller says who
// the person is, never which anchor to mint. A caller cannot ask for someone
// else's slot because it never gets to name a slot at all.

// collectRequest names the person collecting, and nothing else. There is
// deliberately no anchor/prefix/alias field — see the trust boundary above.
type collectRequest struct {
	IdentityID string `json:"identity_id"`
}

// collectResponse is the one-time hand-over. Every secret in it is shown once
// and is unrecoverable afterwards; the console page must say so before the
// reader can navigate away from it.
type collectResponse struct {
	SlotID       string `json:"slot_id"`       // the anchor: cert CN AND write namespace
	SystemHandle string `json:"system_handle"` // the live alias, display-only (ADR 1039)
	APIKey       string `json:"api_key"`       // one-time; bcrypt hash kept, plaintext unrecoverable
	BundleURL    string `json:"bundle_url"`    // tarball download URL, valid for BundleTTL after mint
	BundleToken  string `json:"bundle_token"`
	URISAN       string `json:"uri_san"`
	IdentityID   string `json:"identity_id"` // echoed so the console confirms who it collected for
	CollectedAt  int64  `json:"collected_at"`
	NoteToShare  string `json:"note_to_share"`
	// LeafCN is the collected leaf's Subject CN, "<slot_id>.g<generation>"
	// (GHSA-rr3p). slot_id stays the write namespace.
	LeafCN string `json:"leaf_cn"`
	// LeafStatus is live when the router host's reload record shows the
	// router admits leaf_cn, unknown when this host has no such record.
	LeafStatus string `json:"leaf_status"`
}

// collectFabricCredential handles POST /api/orgs/collect.
//
// Mints the whole fabric identity — leaf cert AND portal api_key — for the slot
// belonging to the named person. The two are coupled deliberately (rotateVendor
// documents why: the old plaintext is unrecoverable, so one cannot be re-issued
// without the other), and for a FIRST collection that coupling is ideal: nobody
// is using the cert yet, so the identity is created when its owner asks for it.
//
// Collection is the same mint onboarding does today, moved later and
// authenticated differently. No new decoupled portal operation is required.
func (s *Server) collectFabricCredential(w http.ResponseWriter, r *http.Request) {
	var req collectRequest
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

	// Slot-pool goatnets only. In per-vendor mode there is no slot table and no
	// person→slot linkage to resolve, so there is nothing to collect against;
	// refuse plainly rather than half-work.
	if s.cfg.IdentityModel != config.IdentityModelPerUUID {
		writeError(w, http.StatusNotImplemented, "not_slot_mode",
			"credential collection requires the per-uuid slot-pool identity model (ADR 1025); this goatnet runs per-vendor identities")
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
			"more than one live slot is recorded for this identity; refusing to guess which one to mint. An operator must fix the duplicate linkage.")
		return
	case err != nil:
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	anchor := slot.SlotID
	handle := ""
	if slot.AssignedTo != nil {
		handle = *slot.AssignedTo
	}

	// Already collected: refuse, and say what to do instead. Re-minting here
	// would silently invalidate the credential the person is already holding —
	// the rotation trap this whole design exists to close. A second attempt is
	// far more likely to be a lost key than a replay, and the honest answer to a
	// lost key is a deliberate operator re-issue, not a quiet one.
	if slot.CollectedAt != nil {
		writeError(w, http.StatusConflict, "already_collected",
			"this identity's fabric credential was already collected and shown once; the plaintext is unrecoverable. "+
				"If it was lost, an operator re-issues in place with POST /api/orgs/{anchor}/rotate — which invalidates the previous copy deliberately.")
		return
	}

	// Minted-but-not-collected: the pre-mint-on-collect state, where onboarding
	// minted the fabric leg FOR the person and delivered it out-of-band. Minting
	// again would invalidate that copy without their asking. Refuse for the same
	// reason as above. Once onboarding stops minting the people leg, freshly
	// onboarded slots reach collection with cert_minted_at NULL and pass here.
	if slot.CertMintedAt != nil {
		writeError(w, http.StatusConflict, "already_minted",
			"a fabric credential was already minted for this identity at onboarding and delivered out-of-band; collecting now would invalidate that copy. "+
				"If it is lost or was never received, an operator re-issues in place with POST /api/orgs/{anchor}/rotate.")
		return
	}

	// The org row is created at onboard (alias, sponsorship, labels, quota all
	// live on it); collection only fills in the credential. A missing row means
	// the slot is assigned but never registered — broken state we must not paper
	// over by creating one with defaults nobody decided.
	if _, err := s.reg.GetVendor(r.Context(), anchor); err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			writeError(w, http.StatusConflict, "no_org_row",
				"the slot is assigned but has no org row; onboarding did not complete for this identity. An operator must re-run the onboard rather than collect against a half-built slot.")
			return
		}
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}

	// Collection is the onboard mint, moved later. The collected leaf must
	// carry a CN no earlier leaf for this slot carried (GHSA-rr3p): if a leaf
	// was already issued at the slot's current generation, or the slot is
	// still at generation 0 (the bare anchor every pre-generation leaf has),
	// advance it first. A generation with no leaf yet (the one a deferred
	// onboard's assignment made, or one an earlier pending collect advanced to)
	// is used as is.
	gen := slot.Generation
	fresh, err := s.reg.HasLeafAt(r.Context(), anchor, gen)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if gen == 0 || fresh {
		if gen, err = s.reg.BumpSlotGeneration(r.Context(), anchor); err != nil {
			writeError(w, http.StatusInternalServerError, "bump_generation", err.Error())
			return
		}
	}
	defer s.syncSlotFragment(r.Context())
	// Hold the credential until the router admits it: a leaf the router
	// refuses would be shown once and then not work, and the person could not
	// collect again. Nothing is minted on this path, so a retry is safe.
	if s.leafStatus(anchor, gen) == leafPending {
		w.Header().Set("Retry-After", "60")
		writeError(w, http.StatusServiceUnavailable, "fabric_pending",
			"your fabric slot is being prepared on the router and nothing has been minted yet; "+
				"try again in a few minutes (the router loads slot changes within its reload interval, 5 minutes by default)")
		return
	}

	apiKey, err := generateAPIKey(anchor)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "gen_key", err.Error())
		return
	}
	hash, digest, err := hashAPIKey(apiKey)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "hash", err.Error())
		return
	}
	// The key hash must land before the mint: apiKey is baked INTO the tarball's
	// goat-cli .cbor, so a tarball minted against a key the registry never
	// accepted is a bundle that authenticates to nothing. If the mint fails
	// after this, the row carries a hash for a key nobody holds — harmless on an
	// uncollected slot (nobody held one before either), and collected_at is
	// still NULL, so the person can simply collect again.
	if err := s.reg.UpdateVendorAPIKey(r.Context(), anchor, hash, digest); err != nil {
		writeError(w, http.StatusInternalServerError, "update_key", err.Error())
		return
	}

	res, err := s.mintSlotLeaf(r.Context(), anchor, apiKey, gen, false, identityID)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "mint", err.Error())
		return
	}
	if err := s.reg.MarkBundle(r.Context(), anchor, res.BundleToken, res.TarballPath); err != nil {
		writeError(w, http.StatusInternalServerError, "mark_bundle", err.Error())
		return
	}
	if err := s.reg.StampSlotCertAt(r.Context(), anchor, res.BundleToken, res.TarballPath, gen, identityID); err != nil {
		writeError(w, http.StatusConflict, "slot_changed_hands", err.Error()+"; nothing was handed out, collect again")
		return
	}

	// Lazy slot-pool storage (ADR 1101, F-186): the <anchor>/** rocksdb store is
	// added when the slot's cert exists, which on this path is now rather than at
	// onboard. Best-effort + non-fatal, exactly as at onboard: live pub/sub
	// already works off the pre-baked ACL, and the next router config render
	// declares stores for the active anchors anyway.
	if s.cfg.LazySlotStorage && s.aclRender != nil && s.aclRender.AdminREST != nil {
		ttl := s.aclRender.StorageTTLMs
		if ttl == 0 {
			ttl = 604800000 // 7d default, matches the role
		}
		dir := filepath.Join(s.aclRender.StorageDirPrefix, anchor)
		if err := s.aclRender.AdminREST.AddStorage(r.Context(), anchor, dir, ttl); err != nil {
			slog.Warn("lazy slot storage: live store-add failed at collection (non-fatal; next render persists it)",
				"anchor", anchor, "err", err)
		} else {
			slog.Info("lazy slot storage: added <anchor>/** store at collection", "anchor", anchor)
		}
	}

	// Stamp collected LAST, after every artefact exists — the prior-art ordering
	// (write the artefact before reporting success). A failure anywhere above
	// leaves collected_at NULL and the person can collect again; stamping first
	// would burn their one collection on a mint that never produced a bundle.
	if err := s.reg.StampSlotCollectedAt(r.Context(), anchor, gen, identityID); err != nil {
		writeError(w, http.StatusConflict, "slot_changed_hands", err.Error()+"; nothing was handed out, collect again")
		return
	}
	collected, err := s.reg.SlotForIdentity(r.Context(), identityID)
	collectedAt := int64(0)
	if err == nil && collected != nil && collected.CollectedAt != nil {
		collectedAt = *collected.CollectedAt
	}

	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "fabric-collect", anchor,
		fmt.Sprintf(`{"identity_id":%q,"alias":%q,"uri_san":%q}`, identityID, handle, res.URISAN))

	// The bundle URL lands on the CONSOLE's collection page, a different host
	// from the portal — a bare "/api/org-bundle/…" path there either reads as
	// a fragment or resolves against the console and 404s. Absolutize against
	// the mesh-facing portal base (the same PortalPublicURL the probe and the
	// tarball NOTES already use). Scoped to collection: the other bundleURL
	// callers answer the goat CLI, which joins paths onto its own portal base
	// and would double-prefix an absolute URL.
	bundle := bundleURL(anchor, res.BundleToken)
	if base := strings.TrimRight(strings.TrimSpace(s.cfg.PortalPublicURL), "/"); base != "" {
		bundle = base + bundle
	}

	writeJSON(w, http.StatusCreated, &collectResponse{
		LeafCN:       registry.LeafCN(anchor, gen),
		LeafStatus:   s.leafStatus(anchor, gen),
		SlotID:       anchor,
		SystemHandle: handle,
		APIKey:       apiKey,
		BundleURL:    bundle,
		BundleToken:  res.BundleToken,
		URISAN:       res.URISAN,
		IdentityID:   identityID,
		CollectedAt:  collectedAt,
		NoteToShare: "Shown once. The api_key is stored only as a hash and the tarball URL expires 24 h after it is issued — neither can be re-read, " +
			"and nobody, including an operator, can show them to you again. Do not ask for a rotation to get a working copy: " +
			"rotation re-issues in place and invalidates whatever you already hold.",
	})
}

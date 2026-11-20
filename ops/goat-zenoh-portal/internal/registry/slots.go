package registry

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"time"
)

// Slot matches the slots table — the per-uuid slot pool (ADR 1025). Present
// only in per-uuid-slot-pool goatnets; empty in per-vendor goatnets.
type Slot struct {
	SlotID         string  `db:"slot_id"          json:"slot_id"`
	Idx            int64   `db:"idx"              json:"idx"`
	AssignedTo     *string `db:"assigned_to"      json:"assigned_to,omitempty"`
	AssignedAt     *int64  `db:"assigned_at"      json:"assigned_at,omitempty"`
	CertMintedAt   *int64  `db:"cert_minted_at"   json:"cert_minted_at,omitempty"`
	BundleToken    *string `db:"bundle_token"     json:"-"`
	BundlePath     *string `db:"bundle_path"      json:"-"`
	BundleMintedAt *int64  `db:"bundle_minted_at" json:"bundle_minted_at,omitempty"`
	RevokedAt      *int64  `db:"revoked_at"       json:"revoked_at,omitempty"`
	// ADR 1096 delegated machine enrollment — the machine-principal stamp. NULL
	// on a free slot or a developer self-slot; set by RecordMachineEnrollment.
	EnrolledBy  *string `db:"enrolled_by"  json:"enrolled_by,omitempty"` // custodian person handle (custody edge)
	NodeClass   *string `db:"node_class"   json:"node_class,omitempty"`
	Autonomy    *string `db:"autonomy"     json:"autonomy,omitempty"`
	Affiliation *string `db:"affiliation"  json:"affiliation,omitempty"`
	Workloads   *string `db:"workloads"    json:"workloads,omitempty"` // ADR 1098: PLATFORM composition (comma-joined function classes)
	EnrolledAt  *int64  `db:"enrolled_at"  json:"enrolled_at,omitempty"`
	// IdentityID is the Kratos identity UUID this slot was onboarded FOR, stamped
	// at onboard while the flow still holds it (identity fragility
	// audit §1.2). NULL = no recorded linkage (a free slot, a machine slot, or a
	// slot onboarded before the column existed) — read it as "derive ownership by
	// name and say so", never as "unowned".
	IdentityID *string `db:"identity_id" json:"identity_id,omitempty"`
	// CollectedAt is when the PERSON who owns this slot collected their own
	// fabric credential (mint-on-collect). NULL = uncollected. Distinct from
	// CertMintedAt, which only says a cert exists — it does not say who asked
	// for it. A slot minted FOR someone at onboard has CertMintedAt set and
	// CollectedAt NULL, and that difference is the whole point: it separates
	// "onboarded" from "holds a fabric credential they asked for".
	CollectedAt *int64 `db:"collected_at" json:"collected_at,omitempty"`
	// ADR 1104 tenant lifecycle class: test | notional | real; '' =
	// unclassified, which every consumer treats as real (fail-safe).
	LifecycleClass string `db:"lifecycle_class" json:"lifecycle_class,omitempty"`
	// Generation is the slot's leaf generation (GHSA-rr3p): the CN of the one
	// leaf the router admits for this slot is LeafCN(SlotID, Generation).
	// Advanced on every assign, rotate, collect and revoke.
	Generation int64 `db:"generation" json:"generation"`
}

// ErrSlotPoolExhausted means every slot is currently assigned. The fix is a
// planned restart with a larger pool (ADR 1025) — not a runtime ACL edit.
var ErrSlotPoolExhausted = errors.New("slot pool exhausted (no free slot)")

// SlotAnchor is the deterministic anchor for slot index k under salt: the
// first 32 hex chars of sha256("<salt>:<k>"). It is the slot's cert CN AND
// its write-anchor prefix.
//
// This MUST stay byte-for-byte identical to the router ACL template's formula
// (ops/ansible/roles/goat-zenoh/templates/config.json5.j2, per-uuid mode:
// `(salt ~ ':' ~ k) | hash('sha256')` then `[:32]`). A mismatch means a
// portal-minted cert (CN=anchor) matches no router rule and the system can
// write nothing. The parity is locked by TestSlotAnchorParity against the
// exact anchors the live isolated-router proof enforced.
func SlotAnchor(salt string, k int) string {
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s:%d", salt, k)))
	return hex.EncodeToString(sum[:])[:32]
}

// InitializeSlotPool bakes `size` deterministic slots into the table the first
// time (idempotent — a no-op once populated). Returns the number created
// (0 if already initialized). Call at portal boot when the goatnet's identity
// model is per-uuid-slot-pool; the salt must be non-empty and distinct per
// goatnet (so anchor spaces are disjoint across a federation).
func (r *Registry) InitializeSlotPool(ctx context.Context, salt string, size int) (int, error) {
	if salt == "" {
		return 0, errors.New("slot pool salt required (per-goatnet, must be distinct)")
	}
	if size <= 0 {
		return 0, fmt.Errorf("slot pool size must be positive, got %d", size)
	}
	var existing int
	if err := r.db.GetContext(ctx, &existing, `SELECT COUNT(*) FROM slots`); err != nil {
		return 0, fmt.Errorf("count slots: %w", err)
	}
	if existing > 0 {
		return 0, nil // already initialized; resizing the pool is a separate, deliberate op
	}
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return 0, fmt.Errorf("begin slot-pool tx: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	for k := 0; k < size; k++ {
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO slots (slot_id, idx) VALUES ($1, $2)`, SlotAnchor(salt, k), k); err != nil {
			return 0, fmt.Errorf("insert slot %d: %w", k, err)
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, fmt.Errorf("commit slot pool: %w", err)
	}
	return size, nil
}

// ErrPartnerBlockExhausted is returned when a partner mint asks for a slot from
// a pre-cut block (ADR 1138) and every slot in that block is taken. It is
// DELIBERATELY NOT ErrSlotPoolExhausted and MUST NOT fall back to an ordinary
// slot: a partner outside its block matches no partner-<anchor> subject in the
// router, so it would read the WHOLE FABRIC, silently. Refusing the
// mint is the safe failure; over-sharing is not.
var ErrPartnerBlockExhausted = errors.New("partner slot block exhausted")

// GetFreeSlot returns the lowest-idx slot that is unassigned and out of
// quarantine (no recorded leaf for it is still valid, slot_leaves), or
// ErrSlotPoolExhausted. Read-only; AllocateSlot is the atomic claim.
func (r *Registry) GetFreeSlot(ctx context.Context) (*Slot, error) {
	var s Slot
	err := r.reader().GetContext(ctx, &s,
		`SELECT * FROM slots WHERE assigned_to IS NULL AND `+slotReusableSQL(1)+` ORDER BY idx ASC LIMIT 1`, SlotLeafClock().Unix(), "")
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrSlotPoolExhausted
	}
	if err != nil {
		return nil, fmt.Errorf("get free slot: %w", err)
	}
	return &s, nil
}

// GetFreeSlotIn returns the lowest-idx unassigned slot within [from,to], or
// ErrPartnerBlockExhausted. Read-only; AllocateSlotIn is the atomic claim.
func (r *Registry) GetFreeSlotIn(ctx context.Context, from, to int64) (*Slot, error) {
	var s Slot
	err := r.reader().GetContext(ctx, &s,
		`SELECT * FROM slots WHERE assigned_to IS NULL AND idx >= $1 AND idx <= $2 AND `+slotReusableSQL(3)+` ORDER BY idx ASC LIMIT 1`,
		from, to, SlotLeafClock().Unix(), "")
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrPartnerBlockExhausted
	}
	if err != nil {
		return nil, fmt.Errorf("get free slot in [%d,%d]: %w", from, to, err)
	}
	return &s, nil
}

// AllocateSlot atomically claims the lowest-idx free slot for systemID and
// returns it. ErrSlotPoolExhausted if none free. The caller then mints a cert
// with CN = the returned SlotID.
//
// This is the ORDINARY path. A PARTNER must not come through here — see
// AllocateSlotIn.
func (r *Registry) AllocateSlot(ctx context.Context, systemID string) (*Slot, error) {
	return r.allocateSlot(ctx, systemID, "", nil, nil)
}

// AllocateSlotAs is AllocateSlot for a claim made on behalf of a known person
// (Kratos identity UUID). The reuse quarantine does not hold a slot back from
// the person its live leaves were issued for, so a person who is re-onboarded
// can get their own slot back; every other claimant waits for those leaves to
// expire. An empty identityID is AllocateSlot.
func (r *Registry) AllocateSlotAs(ctx context.Context, systemID, identityID string) (*Slot, error) {
	return r.allocateSlot(ctx, systemID, identityID, nil, nil)
}

// AllocateSlotIn atomically claims the lowest-idx free slot within [from,to].
//
// This is the portal half of ADR 1138. The router pre-renders the partner
// subject and rules for every slot in a declared block BEFORE anyone holds the
// cert, so a partner minted INTO its block matches a subject that already
// exists — no ACL edit, no fabric restart. Handing out the lowest free slot
// fleet-wide would put partners OUTSIDE their block and make the pre-cut ranges
// inert: every partner would then cost a hand-edit of zenoh_partner_read_lanes
// plus a router restart, and until that happened they would read the whole
// fabric silently. See partner-read-partition.md.
//
// Fails with ErrPartnerBlockExhausted rather than falling back to an ordinary
// slot. The fallback would be the over-sharing bug, so it is refused.
func (r *Registry) AllocateSlotIn(ctx context.Context, systemID string, from, to int64) (*Slot, error) {
	if from < 0 || to < from {
		return nil, fmt.Errorf("invalid slot block [%d,%d]", from, to)
	}
	return r.allocateSlot(ctx, systemID, "", &from, &to)
}

// AllocateSlotInAs is AllocateSlotIn on behalf of a known person; see
// AllocateSlotAs.
func (r *Registry) AllocateSlotInAs(ctx context.Context, systemID, identityID string, from, to int64) (*Slot, error) {
	if from < 0 || to < from {
		return nil, fmt.Errorf("invalid slot block [%d,%d]", from, to)
	}
	return r.allocateSlot(ctx, systemID, identityID, &from, &to)
}

// allocateSlot is the shared claim. from/to nil = anywhere in the pool.
// identityID is the claimant for the reuse quarantine (empty = unidentified).
func (r *Registry) allocateSlot(ctx context.Context, systemID, identityID string, from, to *int64) (*Slot, error) {
	if systemID == "" {
		return nil, errors.New("system id required")
	}
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, fmt.Errorf("begin allocate tx: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// One alias ⇄ one live slot (ADR 1039). Reject inside the tx before claiming
	// so a duplicate handle can't take a second anchor. Application-level for now
	// (a DB partial-unique index is the hardening, applied at deploy once
	// existing data is verified dup-free — it would fail boot on a pre-existing
	// duplicate, which must never wedge the cert-minting path).
	var dup int
	if err := tx.GetContext(ctx, &dup,
		`SELECT COUNT(*) FROM slots WHERE assigned_to = $1 AND revoked_at IS NULL`, systemID); err != nil {
		return nil, fmt.Errorf("alias uniqueness check: %w", err)
	}
	if dup > 0 {
		return nil, ErrAliasTaken
	}
	var s Slot
	if from != nil && to != nil {
		err = tx.GetContext(ctx, &s,
			`SELECT * FROM slots WHERE assigned_to IS NULL AND idx >= $1 AND idx <= $2 AND `+slotReusableSQL(3)+` ORDER BY idx ASC LIMIT 1`,
			*from, *to, SlotLeafClock().Unix(), identityID)
		if errors.Is(err, sql.ErrNoRows) {
			// NOT ErrSlotPoolExhausted, and NOT a fallback to an ordinary slot.
			return nil, ErrPartnerBlockExhausted
		}
	} else {
		err = tx.GetContext(ctx, &s,
			`SELECT * FROM slots WHERE assigned_to IS NULL AND `+slotReusableSQL(1)+` ORDER BY idx ASC LIMIT 1`, SlotLeafClock().Unix(), identityID)
		if errors.Is(err, sql.ErrNoRows) {
			return nil, ErrSlotPoolExhausted
		}
	}
	if err != nil {
		return nil, fmt.Errorf("select free slot: %w", err)
	}
	now := time.Now().Unix()
	// Clear any stale ADR 1096 machine stamp from a prior tenant so a re-claimed
	// slot never counts against the wrong custodian's quota (MachineQuotaFor
	// counts live enrolled_by rows). A machine re-enrollment re-stamps via
	// RecordMachineEnrollment right after; a non-machine self-slot stays unstamped.
	// identity_id is cleared with the machine stamp for the same reason: a
	// re-claimed slot must never carry the PRIOR tenant's person linkage, which
	// would attribute the new tenant's writes — and any revocation aimed at them
	// — to someone who no longer holds the slot. The onboard re-stamps it.
	//
	// The assignment advances the generation (GHSA-rr3p): the new holder's leaf
	// carries a CN no earlier leaf for this slot had, so none of those match the
	// subject the fragment renders for it.
	var gen int64
	if err := tx.GetContext(ctx, &gen,
		`UPDATE slots SET assigned_to = $1, assigned_at = $2, revoked_at = NULL,
		    enrolled_by = NULL, node_class = NULL, autonomy = NULL, affiliation = NULL, workloads = NULL, enrolled_at = NULL,
		    identity_id = NULL,
		    cert_minted_at = NULL, bundle_token = NULL, bundle_path = NULL, bundle_minted_at = NULL,
		    collected_at = NULL,
		    generation = generation + 1
		 WHERE slot_id = $3
		 RETURNING generation`,
		systemID, now, s.SlotID); err != nil {
		return nil, fmt.Errorf("assign slot: %w", err)
	}
	if err := supersedeLeaves(ctx, tx, s.SlotID, now); err != nil {
		return nil, err
	}
	// A slot freed before revoke ended these (or by RevokeSlot, the rollback
	// path) can still carry the previous holder's allowances: end them here
	// too, so a new holder starts with none (GHSA-fghv).
	if err := endAnchorAllowances(ctx, tx, s.SlotID); err != nil {
		return nil, fmt.Errorf("assign slot: %w", err)
	}
	if err := endAnchorGrants(ctx, tx, s.SlotID, now, "slot-reassign", 0); err != nil {
		return nil, fmt.Errorf("assign slot: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("commit allocate: %w", err)
	}
	s.AssignedTo, s.AssignedAt = &systemID, &now
	// Mirror the DB stamp-clear (above) onto the returned struct — the SELECT
	// that populated s ran before the UPDATE, so it may carry a prior tenant's
	// ADR 1096 machine stamp that the claim just wiped.
	s.RevokedAt = nil
	s.EnrolledBy, s.NodeClass, s.Autonomy, s.Affiliation, s.Workloads, s.EnrolledAt = nil, nil, nil, nil, nil, nil
	s.IdentityID = nil
	s.CertMintedAt, s.BundleToken, s.BundlePath, s.BundleMintedAt = nil, nil, nil, nil
	s.CollectedAt = nil
	s.Generation = gen
	return &s, nil
}

// StampSlotIdentity records the Kratos identity UUID the slot was onboarded for
// (identity fragility audit §1.2 — record the linkage at the moment
// the flow holds it, instead of re-deriving ownership from names forever after).
// Called right after AllocateSlot on the person path; machine enrollment does NOT
// call it (a machine is a WireGuard identity, its person link is enrolled_by).
// An empty identityID clears the stamp rather than writing an empty string, so a
// caller that has no UUID leaves an honest NULL. Errors if the slot is unknown.
func (r *Registry) StampSlotIdentity(ctx context.Context, slotID, identityID string) error {
	if slotID == "" {
		return errors.New("slot id required")
	}
	_, err := r.RelinkSlotIdentity(ctx, slotID, identityID)
	return err
}

// Relink says what RelinkSlotIdentity did.
type Relink struct {
	// HandedOver is true when the slot held a live leaf issued for another
	// identity: that holder's access as the anchor was ended (see
	// RelinkSlotIdentity) before the new identity was linked.
	HandedOver       bool
	PreviousIdentity string // the identity on file before, "" when none
	PreviousHolder   string // issued_for_identity of the live leaf that was cut, "" when none
	// LateAttributed is true when the cut leaf's issuer was written by a late
	// linkage rather than at mint: if this relink corrects a wrong stamp, the
	// leaf was the real holder's, and the anchor's allowances (keyed by the
	// anchor, not the identity) were ended for them too.
	LateAttributed bool
	// MachinesStillLive are the live machines the anchor had enrolled when it
	// was handed over. Their custody ended, but their own leaves did not: a
	// person who keeps such a device keeps its fabric access until an
	// operator revokes the machine (DELETE /api/machines/{anchor}).
	MachinesStillLive []string
	GenerationFrom    int64
	GenerationTo      int64
}

// RelinkSlotIdentity sets the identity a slot is linked to, and decides from
// the slot's LIVE LEAVES (unsuperseded, at its current generation) whether
// that is a change of hands (GHSA-rr3p reviews r4 F1, r5 F1):
//
//   - a live leaf whose issued_for_identity is non-empty and differs from the
//     new identity (a withdrawal included) is a hand-over;
//   - live leaves issued for nobody (minted before the linkage was recorded)
//     are attributed to the new identity: the late linkage of the native
//     onboard, which mints before the login leg returns;
//   - anything else (no live leaf, or a re-stamp of the leaves' identity)
//     only writes the identity.
//
// A hand-over ends, in the same transaction and before the new identity is
// linked, everything the previous holder had as this anchor, as
// RevokeAssignedSlot does on offboard (GHSA-rr3p r5 F2, GHSA-fghv): the
// generation advances and the leaves are superseded; the portal api key on the
// anchor's org row is erased; the machine grant, custody of the machines it
// enrolled, the topic budget and the producer data grants end; and the cert
// and collection stamps are cleared, so the new identity collects its own
// credential. Correcting a wrong late linkage is therefore also a hand-over:
// the real holder collects again.
func (r *Registry) RelinkSlotIdentity(ctx context.Context, slotID, identityID string) (Relink, error) {
	var out Relink
	identityID = NormalizeIdentityID(identityID)
	if slotID == "" {
		return out, errors.New("slot id required")
	}
	var val any
	if identityID != "" {
		val = identityID
	}
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return out, fmt.Errorf("stamp slot identity: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var cur struct {
		IdentityID *string `db:"identity_id"`
		Generation int64   `db:"generation"`
	}
	if err := tx.GetContext(ctx, &cur, `SELECT identity_id, generation FROM slots WHERE slot_id = $1`, slotID); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return out, fmt.Errorf("slot %q not found", slotID)
		}
		return out, fmt.Errorf("stamp slot identity: %w", err)
	}
	if cur.IdentityID != nil {
		out.PreviousIdentity = NormalizeIdentityID(*cur.IdentityID)
	}
	out.GenerationFrom, out.GenerationTo = cur.Generation, cur.Generation
	var holders []struct {
		Issuer string `db:"issued_for_identity"`
		Late   int64  `db:"late_attributed"`
	}
	if err := tx.SelectContext(ctx, &holders,
		`SELECT DISTINCT issued_for_identity, late_attributed FROM slot_leaves
		  WHERE slot_id = $1 AND generation = $2 AND superseded_at IS NULL`, slotID, cur.Generation); err != nil {
		return out, fmt.Errorf("stamp slot identity: live leaves: %w", err)
	}
	// Any move of the link away from a recorded identity (the identity on
	// file, or the issuer of a live leaf) is a hand-over and ends what that
	// person had as the anchor (GHSA-rr3p reviews r5 F1/F2, r6 F1/F2). This
	// covers slots linked before generations existed (their leaves carry no
	// issuer), a relink while a rotate or collect is between its bump and
	// its mint (no live leaf), and a deferred onboard with seeded grants.
	if out.PreviousIdentity != "" && out.PreviousIdentity != identityID {
		out.HandedOver = true
	}
	for _, h := range holders {
		if h.Issuer != "" && NormalizeIdentityID(h.Issuer) != identityID {
			out.HandedOver, out.PreviousHolder = true, h.Issuer
			if h.Late != 0 {
				out.LateAttributed = true
			}
		}
	}
	now := time.Now().Unix()
	if out.HandedOver {
		var gen int64
		if err := tx.GetContext(ctx, &gen,
			`UPDATE slots SET generation = generation + 1,
			    cert_minted_at = NULL, bundle_token = NULL, bundle_path = NULL, bundle_minted_at = NULL, collected_at = NULL
			  WHERE slot_id = $1 RETURNING generation`, slotID); err != nil {
			return out, fmt.Errorf("stamp slot identity: hand-over: %w", err)
		}
		out.GenerationTo = gen
		if err := supersedeLeaves(ctx, tx, slotID, now); err != nil {
			return out, err
		}
		if _, err := tx.ExecContext(ctx,
			`UPDATE vendors SET api_key_hash = '', api_key_sha256 = '' WHERE prefix = $1`, slotID); err != nil {
			return out, fmt.Errorf("stamp slot identity: hand-over: api key: %w", err)
		}
		if err := tx.SelectContext(ctx, &out.MachinesStillLive,
			`SELECT slot_id FROM slots WHERE enrolled_by = $1 AND revoked_at IS NULL AND assigned_to IS NOT NULL ORDER BY idx`, slotID); err != nil {
			return out, fmt.Errorf("stamp slot identity: hand-over: machines: %w", err)
		}
		if err := endAnchorAllowances(ctx, tx, slotID); err != nil {
			return out, fmt.Errorf("stamp slot identity: hand-over: %w", err)
		}
		if err := endAnchorGrants(ctx, tx, slotID, now, "slot-hand-over", 0); err != nil {
			return out, fmt.Errorf("stamp slot identity: hand-over: %w", err)
		}
	}
	if !out.HandedOver && identityID != "" {
		// Late linkage: leaves minted before any identity was recorded are this
		// identity's, so a later relink away from it is seen as a hand-over.
		if _, err := tx.ExecContext(ctx,
			`UPDATE slot_leaves SET issued_for_identity = $1, late_attributed = 1
			  WHERE slot_id = $2 AND generation = $3 AND superseded_at IS NULL AND issued_for_identity = ''`,
			identityID, slotID, out.GenerationTo); err != nil {
			return out, fmt.Errorf("stamp slot identity: attribute leaves: %w", err)
		}
	}
	if _, err := tx.ExecContext(ctx, `UPDATE slots SET identity_id = $1 WHERE slot_id = $2`, val, slotID); err != nil {
		return out, fmt.Errorf("stamp slot identity: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return out, fmt.Errorf("stamp slot identity: commit: %w", err)
	}
	return out, nil
}

// SlotForIdentity resolves a Kratos identity UUID to the LIVE slot recorded for
// that person (slots.identity_id, stamped at onboard). This is the mint-on-collect
// lookup: the collector proves who they are to the console, and the portal finds
// what is theirs — the person never names their own slot, so a caller cannot
// collect against someone else's anchor by asking for it.
//
// Returns ErrNotFound when the identity has no live slot (never onboarded, slot
// revoked, or onboarded before the linkage column existed — all of which mean
// "nothing to collect here", and none of which is safe to guess a slot for).
// Returns ErrAmbiguous when more than one live slot carries the same identity —
// fail closed rather than mint against an arbitrary one; an operator fixes the
// duplicate linkage.
func (r *Registry) SlotForIdentity(ctx context.Context, identityID string) (*Slot, error) {
	if identityID == "" {
		return nil, errors.New("identity id required")
	}
	var slots []Slot
	if err := r.reader().SelectContext(ctx, &slots,
		`SELECT * FROM slots WHERE lower(identity_id) = lower($1) AND assigned_to IS NOT NULL AND revoked_at IS NULL ORDER BY idx ASC`,
		NormalizeIdentityID(identityID)); err != nil {
		return nil, fmt.Errorf("slot for identity: %w", err)
	}
	switch len(slots) {
	case 0:
		return nil, ErrNotFound
	case 1:
		return &slots[0], nil
	default:
		return nil, ErrAmbiguous
	}
}

// LiveSlotForAlias returns the live (assigned, not revoked) slot holding alias —
// the slot AllocateSlot refused with ErrAliasTaken. ErrNotFound when no live slot
// holds it; ErrAmbiguous when more than one does, which the allocation guard is
// meant to make impossible and which no caller may resolve by picking one.
func (r *Registry) LiveSlotForAlias(ctx context.Context, alias string) (*Slot, error) {
	if alias == "" {
		return nil, errors.New("alias required")
	}
	var slots []Slot
	if err := r.reader().SelectContext(ctx, &slots,
		`SELECT * FROM slots WHERE assigned_to = $1 AND revoked_at IS NULL ORDER BY idx ASC`, alias); err != nil {
		return nil, fmt.Errorf("live slot for alias: %w", err)
	}
	switch len(slots) {
	case 0:
		return nil, ErrNotFound
	case 1:
		return &slots[0], nil
	default:
		return nil, ErrAmbiguous
	}
}

// StampSlotCollected records that the slot's owner collected their own fabric
// credential. Set once, at the moment the credential is handed over; the caller
// checks CollectedAt first and refuses a second collection, so this is not an
// idempotent "touch" — a second call overwriting the timestamp would erase when
// the credential actually left the building. Errors if the slot id is unknown.
func (r *Registry) StampSlotCollected(ctx context.Context, slotID string) error {
	now := time.Now().Unix()
	res, err := r.db.ExecContext(ctx,
		`UPDATE slots SET collected_at = $1 WHERE slot_id = $2`, now, slotID)
	if err != nil {
		return fmt.Errorf("stamp slot collected: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return fmt.Errorf("slot %q not found", slotID)
	}
	return nil
}

// SlotPoolCounts returns (total slots baked, live assigned slots). Both are 0 on
// a per-vendor goatnet, where the pool is never initialized. The budget
// projection uses it as its sanity floor: a populated pool that suddenly reports
// zero assigned anchors is a restored/emptied registry, not a fabric with no
// tenants (audit §1.3 violation 1).
func (r *Registry) SlotPoolCounts(ctx context.Context) (total, assigned int64, err error) {
	if err = r.reader().GetContext(ctx, &total, `SELECT COUNT(*) FROM slots`); err != nil {
		return 0, 0, fmt.Errorf("count slots: %w", err)
	}
	if err = r.reader().GetContext(ctx, &assigned,
		`SELECT COUNT(*) FROM slots WHERE assigned_to IS NOT NULL AND revoked_at IS NULL`); err != nil {
		return 0, 0, fmt.Errorf("count assigned slots: %w", err)
	}
	return total, assigned, nil
}

// StampSlotCert records that the CN=<slot> cert + onboarding tarball were
// minted for an already-allocated slot. A re-mint overwrites the bundle
// pointer + minted timestamps; the assignment is left untouched. Errors if the
// slot id is unknown.
func (r *Registry) StampSlotCert(ctx context.Context, slotID, bundleToken, bundlePath string) error {
	now := time.Now().Unix()
	// The self-join returns the slot's bundle_path as it was before this UPDATE.
	var old []sql.NullString
	err := r.db.SelectContext(ctx, &old,
		`UPDATE slots AS s SET cert_minted_at = $1, bundle_token = $2, bundle_path = $3, bundle_minted_at = $4
		   FROM slots AS o WHERE s.slot_id = $5 AND o.slot_id = s.slot_id
		 RETURNING o.bundle_path`,
		now, bundleToken, bundlePath, now, slotID)
	if err != nil {
		return fmt.Errorf("stamp slot cert: %w", err)
	}
	if len(old) == 0 {
		return fmt.Errorf("slot %q not found", slotID)
	}
	r.dropBundleIfUnnamed(ctx, old[0].String, bundlePath)
	return nil
}

// ClearSlotCert erases a slot's cert + bundle stamps, returning it to the
// "no credential exists" state the collection guard keys on.
//
// Why it exists: the pre-cut pool (ADR 1025) can be batch pre-minted, so slots
// carry cert_minted_at from a single provisioning pass, stamped before anyone
// was assigned to them. A
// deferred onboard that allocates such a slot inherits that stamp, and the
// collection endpoint then refuses its owner with `already_minted` — reading
// pool history as "minted for this person at onboarding". The deferred create
// calls this so the stamp state says what the deferral means: no credential
// has been issued to this person.
func (r *Registry) ClearSlotCert(ctx context.Context, slotID string) error {
	var old []sql.NullString
	err := r.db.SelectContext(ctx, &old,
		`UPDATE slots AS s SET cert_minted_at = NULL, bundle_token = NULL, bundle_path = NULL, bundle_minted_at = NULL
		   FROM slots AS o WHERE s.slot_id = $1 AND o.slot_id = s.slot_id
		 RETURNING o.bundle_path`,
		slotID)
	if err != nil {
		return fmt.Errorf("clear slot cert: %w", err)
	}
	if len(old) == 0 {
		return fmt.Errorf("slot %q not found", slotID)
	}
	r.dropBundleIfUnnamed(ctx, old[0].String, "")
	return nil
}

// RevokeSlot frees a slot back to the pool (clears the assignee, stamps
// revoked_at). The slot id stays valid for re-allocation later. Errors if the
// slot id is unknown.
func (r *Registry) RevokeSlot(ctx context.Context, slotID string) error {
	now := time.Now().Unix()
	// The free advances the generation and supersedes the slot's leaves in
	// the same transaction (GHSA-rr3p), so the fragment renders no CN for the
	// slot and no leaf issued for it matches again, even after a reassignment.
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return fmt.Errorf("revoke slot: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	res, err := tx.ExecContext(ctx,
		`UPDATE slots SET assigned_to = NULL, assigned_at = NULL, revoked_at = $1, generation = generation + 1 WHERE slot_id = $2`,
		now, slotID)
	if err != nil {
		return fmt.Errorf("revoke slot: %w", err)
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return fmt.Errorf("slot %q not found", slotID)
	}
	if err := supersedeLeaves(ctx, tx, slotID, now); err != nil {
		return err
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("revoke slot: commit: %w", err)
	}
	return nil
}

// RevokeAssignedSlot frees a slot ONLY if it is currently assigned, returning
// whether a slot was actually freed. Unlike RevokeSlot it does not error when
// the slot id is unknown or already free — it is the offboard primitive the
// vendor-delete path calls unconditionally: a no-op (false, nil) in per-vendor
// goatnets (no slots table rows) or on an already-free slot, and a clean free
// (true, nil) in per-uuid-slot-pool mode. Freeing clears assigned_to so the
// alias drops out of ListAliases and the slot index returns to the free pool
// .
func (r *Registry) RevokeAssignedSlot(ctx context.Context, slotID string) (bool, error) {
	freed, _, err := r.RevokeAssignedSlotListing(ctx, slotID)
	return freed, err
}

// RevokeAssignedSlotListing is RevokeAssignedSlot that also returns the live
// machines the anchor had enrolled, read inside the same transaction before
// their custody ends. Their own leaves are not ended (they are device
// identities); the offboard response names them so an operator can revoke them.
func (r *Registry) RevokeAssignedSlotListing(ctx context.Context, slotID string) (bool, []string, error) {
	now := time.Now().Unix()
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	// The free advances the generation (GHSA-rr3p): the rendered fragment then
	// names no CN for the slot, and the holder's leaves never match it again.
	res, err := tx.ExecContext(ctx,
		`UPDATE slots SET assigned_to = NULL, assigned_at = NULL, revoked_at = $1, generation = generation + 1
		 WHERE slot_id = $2 AND assigned_to IS NOT NULL`,
		now, slotID)
	if err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: %w", err)
	}
	n, _ := res.RowsAffected()
	if n == 0 {
		return false, nil, nil
	}
	if err := supersedeLeaves(ctx, tx, slotID, now); err != nil {
		return false, nil, err
	}
	var machines []string
	if err := tx.SelectContext(ctx, &machines,
		`SELECT slot_id FROM slots WHERE enrolled_by = $1 AND revoked_at IS NULL AND assigned_to IS NOT NULL ORDER BY idx`, slotID); err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: machines: %w", err)
	}
	// What the holder was given as this anchor ends with the slot, so nobody
	// who later holds the index inherits it (GHSA-fghv): the machine-enrolment
	// grant, custody of the machines they enrolled (those machines become
	// operator-only until an operator re-assigns them), the topic-budget
	// allowance (endAnchorAllowances), and the data grants that let others
	// read their namespace.
	if err := endAnchorGrants(ctx, tx, slotID, now, "slot-revoke", 0); err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: %w", err)
	}
	if err := endAnchorAllowances(ctx, tx, slotID); err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return false, nil, fmt.Errorf("revoke assigned slot: commit: %w", err)
	}
	return true, machines, nil
}

// AnchorForHandle resolves a live system handle (the alias an operator
// onboarded — e.g. "land-unit-1") to its slot anchor (slot_id = cert CN =
// write-anchor). Returns ErrNotFound when no live slot is assigned to the
// handle. This is the inverse of the alias map and exists so the vendor DELETE
// path can accept the SAME identifier the onboard POST issued (the human
// handle), not just the uuid anchor: the vendor row is keyed by the anchor, so
// DELETE /api/vendors/<handle> would otherwise 404 and leave an orphaned org row
// that 409s the next onboard.
func (r *Registry) AnchorForHandle(ctx context.Context, handle string) (string, error) {
	var slotID string
	err := r.reader().GetContext(ctx, &slotID,
		`SELECT slot_id FROM slots WHERE assigned_to = $1 AND revoked_at IS NULL LIMIT 1`, handle)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", fmt.Errorf("anchor for handle %q: %w", handle, err)
	}
	return slotID, nil
}

// LiveHandleAnchors returns handle → anchor for every live (assigned,
// unrevoked) slot. It exists for joins that must key on the HANDLE rather than
// the anchor — chiefly the NetBird peer-name join, because the peer-name
// grammar is `peer-<handle>[-<org_short_tag>]-<site>-<date>[-<hostname>]` and
// carries no anchor at all. Offering only anchors as candidate keys on a
// slot-pool goatnet means nothing can ever match: a 32-hex anchor never
// appears in a peer name, so the join reports an honest-looking zero for
// everyone.
//
// Empty on per-vendor goatnets, which have no slots table content; callers
// must treat an empty map as "no handle-keyed candidates", never as "no peers".
func (r *Registry) LiveHandleAnchors(ctx context.Context) (map[string]string, error) {
	type row struct {
		Handle string `db:"assigned_to"`
		Anchor string `db:"slot_id"`
	}
	var rows []row
	if err := r.reader().SelectContext(ctx, &rows,
		`SELECT assigned_to, slot_id FROM slots
		  WHERE revoked_at IS NULL AND assigned_to IS NOT NULL AND assigned_to <> ''`); err != nil {
		return nil, fmt.Errorf("live handle anchors: %w", err)
	}
	out := make(map[string]string, len(rows))
	for _, r := range rows {
		out[r.Handle] = r.Anchor
	}
	return out, nil
}

// HandleForAnchor is the inverse of AnchorForHandle: the live human handle
// (slot alias) assigned to a slot anchor, or ErrNotFound when the anchor has
// no live assignment. Sponsor-scoped list filtering matches invite/proposal
// sponsor values against BOTH forms (a CLI mint records the prefix, displays
// use the alias), so the scoping key set needs the alias alongside the anchor.
func (r *Registry) HandleForAnchor(ctx context.Context, anchor string) (string, error) {
	var handle string
	err := r.reader().GetContext(ctx, &handle,
		`SELECT assigned_to FROM slots WHERE slot_id = $1 AND revoked_at IS NULL AND assigned_to IS NOT NULL LIMIT 1`, anchor)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", fmt.Errorf("handle for anchor %q: %w", anchor, err)
	}
	return handle, nil
}

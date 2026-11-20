package registry

import (
	"context"
	"encoding/json"
	"fmt"
	"time"
)

// GHSA-rr3p migration step 3: the slots that were handed to someone else
// before slot generations existed. Their current holder's leaf and every
// earlier holder's leaf share one CN (the bare anchor, generation 0), so the
// earlier holders keep writing and reading as the current one until the slot's
// generation moves. Reissuing advances it; the current holder then needs a new
// leaf.

// ReissuePath says how the current holder of a reissued slot gets a new leaf.
type ReissuePath string

const (
	// ReissueRecollect: a person slot linked to its identity. The cert and
	// collection stamps are cleared, so the person collects a new credential
	// from the console's fabric-credential page.
	ReissueRecollect ReissuePath = "recollect"
	// ReissueRotate: a machine slot, or a person slot with no identity
	// linkage, which cannot collect. An operator rotates it
	// (POST /api/vendors/{anchor}/rotate) and delivers the new bundle.
	ReissueRotate ReissuePath = "rotate"
)

// ReissueCandidate is one assigned slot whose index had an earlier holder.
type ReissueCandidate struct {
	SlotID     string  `db:"slot_id"`
	Generation int64   `db:"generation"`
	IdentityID *string `db:"identity_id"`
	EnrolledBy *string `db:"enrolled_by"`
	AssignedAt int64   `db:"assigned_at"`
	// Cutoff is when the slot's current holder took it over: the later of
	// assigned_at and the last change of its identity link away from another
	// person (a pre-upgrade re-stamp moved a slot without moving assigned_at,
	// GHSA-rr3p review r8 F2). Allowances older than this are the earlier
	// holder's.
	Cutoff int64 `db:"-"`
	// Restamped is true when the identity link moved between people during
	// the current assignment; Cutoff is then "now" and every allowance on the
	// anchor ends.
	Restamped bool `db:"-"`
}

// ReissueEnded counts what the reissue of one slot ends (or, in a dry run,
// would end) of the earlier holder's allowances: everything tied to the anchor
// that predates the current assignment (GHSA-rr3p review r7 F2). Before the
// fix an offboard deleted only the org row, so the slot's next holder
// inherited these.
type ReissueEnded struct {
	MachineGrants int64 // machine_grants for the anchor last set before assigned_at
	Custody       int64 // machines enrolled under the anchor before assigned_at
	TopicBudgets  int64 // topic_budgets for the anchor last set before assigned_at
	DataGrants    int64 // active grants with the anchor as producer or principal grantee, created before assigned_at
}

// Add sums two counts.
func (e ReissueEnded) Add(o ReissueEnded) ReissueEnded {
	return ReissueEnded{e.MachineGrants + o.MachineGrants, e.Custody + o.Custody, e.TopicBudgets + o.TopicBudgets, e.DataGrants + o.DataGrants}
}

const (
	// A machine-grant row CREATED before the cut-off is the earlier holder's
	// whatever was updated since: a quota or network_admin change keeps the
	// row's affiliation and site (review r8 F3). It is ended, and the
	// operator re-applies the current holder's grant.
	reissueMachineGrantsWhere = `custodian = $1 AND granted_at < $2`
	reissueCustodyWhere       = `enrolled_by = $1 AND (enrolled_at IS NULL OR enrolled_at < $2)`
	reissueBudgetsWhere       = `owner = $1 AND updated_at < $2`
	reissueGrantsWhere        = `status = 'active' AND (producer = $1 OR (grantee_kind = 'principal' AND grantee_id = $1)) AND created_at < $2`
)

// ReissueWouldEnd counts, read-only, what ApplyReissue would end for c.
func (r *Registry) ReissueWouldEnd(ctx context.Context, c ReissueCandidate) (ReissueEnded, error) {
	var e ReissueEnded
	for _, q := range []struct {
		dst *int64
		sql string
	}{
		{&e.MachineGrants, `SELECT COUNT(*) FROM machine_grants WHERE ` + reissueMachineGrantsWhere},
		{&e.Custody, `SELECT COUNT(*) FROM slots WHERE ` + reissueCustodyWhere},
		{&e.TopicBudgets, `SELECT COUNT(*) FROM topic_budgets WHERE ` + reissueBudgetsWhere},
		{&e.DataGrants, `SELECT COUNT(*) FROM grants WHERE ` + reissueGrantsWhere},
	} {
		if err := r.reader().GetContext(ctx, q.dst, q.sql, c.SlotID, c.Cutoff); err != nil {
			return e, fmt.Errorf("reissue would end: %w", err)
		}
	}
	return e, nil
}

// Path is how this slot's holder gets a new leaf.
func (c ReissueCandidate) Path() ReissuePath {
	if c.EnrolledBy == nil && c.IdentityID != nil && *c.IdentityID != "" {
		return ReissueRecollect
	}
	return ReissueRotate
}

// reissueCandidatesSQL finds every assigned slot still at generation 0 that
// has evidence of a bare-anchor leaf other than its current holder's:
//
//   - an earlier holder in the audit log: an onboard, enrolment, offboard
//     (vendor-delete) or machine revoke for its anchor before the current
//     assigned_at. The anchor appears as the event target (vendor-delete) or
//     as "anchor":"<hex>" in the details. A removal (vendor-delete,
//     machine-revoke) in the same second as the assignment counts too: it can
//     only be the previous holder's.
//   - a rotation (vendor-rotate) during the current assignment, which before
//     generations re-minted the same CN without invalidating the earlier leaf.
//   - recorded leaves (slot_leaves, including what the bundle backfill read
//     from tarballs): two or more distinct unexpired serials at generation 0,
//     or one minted before the current assignment (not_before more than two
//     minutes before assigned_at; cert-sign backdates by one). That catches
//     leaves no audit event names, such as a pre-fix machine enrol that
//     minted, failed and rolled back without an audit row.
//
// A slot already past generation 0 has had a leaf generation since the fix and
// needs nothing here.
const reissueCandidatesSQL = `
SELECT s.slot_id, s.generation, s.identity_id, s.enrolled_by, s.assigned_at
  FROM slots s
 WHERE s.assigned_to IS NOT NULL AND s.revoked_at IS NULL AND s.assigned_at IS NOT NULL
   AND s.generation = 0
   AND (EXISTS (
          SELECT 1 FROM audit_events e
           WHERE (e.occurred_at < s.assigned_at
                  OR (e.occurred_at = s.assigned_at AND e.action IN ('vendor-delete', 'machine-revoke')))
             AND e.action IN ('vendor-delete', 'machine-revoke', 'slot-onboard', 'slot-onboard-deferred', 'machine-enroll')
             AND (e.target = s.slot_id OR e.details LIKE '%"anchor":"' || s.slot_id || '"%'))
        OR EXISTS (
          SELECT 1 FROM audit_events e
           WHERE e.occurred_at >= s.assigned_at AND e.action = 'vendor-rotate' AND e.target = s.slot_id)
        OR (SELECT COUNT(DISTINCT l.serial) FROM slot_leaves l
             WHERE l.slot_id = s.slot_id AND l.generation = 0 AND l.serial <> ''
               AND l.not_after > EXTRACT(EPOCH FROM now())::BIGINT) >= 2
        OR EXISTS (
          SELECT 1 FROM slot_leaves l
           WHERE l.slot_id = s.slot_id AND l.generation = 0 AND l.serial <> ''
             AND l.not_after > EXTRACT(EPOCH FROM now())::BIGINT
             AND l.not_before < s.assigned_at - 120))
 ORDER BY s.idx, s.slot_id`

// ReissueCandidates lists the slots migration step 3 applies to. Read-only.
func (r *Registry) ReissueCandidates(ctx context.Context) ([]ReissueCandidate, error) {
	return reissueCandidates(ctx, r.reader())
}

type selecter interface {
	SelectContext(ctx context.Context, dest any, query string, args ...any) error
}

// reissueCandidates is reissueCandidatesSQL plus the slots whose identity
// link moved from one person to another during the current assignment (a
// pre-upgrade re-stamp), each with its Cutoff.
func reissueCandidates(ctx context.Context, q selecter) ([]ReissueCandidate, error) {
	var listed []ReissueCandidate
	if err := q.SelectContext(ctx, &listed, reissueCandidatesSQL); err != nil {
		return nil, fmt.Errorf("reissue candidates: %w", err)
	}
	in := map[string]bool{}
	for _, c := range listed {
		in[c.SlotID] = true
	}
	var gen0 []ReissueCandidate
	if err := q.SelectContext(ctx, &gen0,
		`SELECT slot_id, generation, identity_id, enrolled_by, assigned_at FROM slots
		  WHERE assigned_to IS NOT NULL AND revoked_at IS NULL AND assigned_at IS NOT NULL AND generation = 0
		  ORDER BY idx, slot_id`); err != nil {
		return nil, fmt.Errorf("reissue candidates: %w", err)
	}
	var out []ReissueCandidate
	for _, c := range gen0 {
		cut, restamped, err := reissueCutoff(ctx, q, c)
		if err != nil {
			return nil, err
		}
		c.Cutoff = cut
		if restamped {
			// The link moved between people during this assignment (a
			// pre-upgrade re-stamp). Nothing records which of the anchor's
			// allowances were given to whom after that, so all of them end
			// (fail closed); the dry run says so and the operator re-applies
			// the current holder's (GHSA-rr3p review r8 F2).
			c.Restamped = true
			c.Cutoff = time.Now().Unix() + 1
		}
		if in[c.SlotID] || c.Restamped {
			out = append(out, c)
		}
	}
	return out, nil
}

// handOverGrace is how long after an assignment a first identity linkage
// still counts as the onboard's own (the native onboard stamps the linkage in
// a second call, after the slot is minted and its grant seeded).
const handOverGrace = 300

// reissueCutoff returns the later of assigned_at and the last change of hands
// it can date, and whether the identity link moved between people during the
// current assignment (restamped). The evidence (GHSA-rr3p reviews r8 F2, r9 F2):
//
//   - identity events (slot.identity.set, slot-onboard, slot-onboard-deferred)
//     for the anchor, walked in row order (id), not by occurred_at, so a clock
//     stepped back cannot hide a re-stamp, from the current assignment's onboard
//     row on (all rows when that row is missing). A withdrawal (empty identity)
//     is not a holder; a different non-empty identity after another is a change;
//   - a first linkage from no identity is also a change when the anchor carries
//     anything dated between assigned_at and that linkage beyond handOverGrace:
//     a machine grant, a data grant, a machine it enrolled, or a recorded leaf.
//     The first holder then held the slot unlinked (or their onboard row is
//     lost), and the linkage handed it to someone else;
//   - generation-0 leaves issued for a different person than the leaf before
//     them, in row order.
func reissueCutoff(ctx context.Context, q selecter, c ReissueCandidate) (int64, bool, error) {
	var evs []struct {
		ID      int64  `db:"id"`
		At      int64  `db:"occurred_at"`
		Action  string `db:"action"`
		Details string `db:"details"`
	}
	if err := q.SelectContext(ctx, &evs,
		`SELECT id, occurred_at, action, details FROM audit_events
		  WHERE action IN ('slot.identity.set', 'slot-onboard', 'slot-onboard-deferred')
		    AND (target = $1 OR details LIKE '%"anchor":"' || $1 || '"%')
		  ORDER BY id`, c.SlotID); err != nil {
		return 0, false, fmt.Errorf("reissue cut-off: %w", err)
	}
	// Start from the current assignment's onboard row: the last onboard row
	// dated at or after assigned_at minus the grace, or the start if none.
	start := 0
	for i, e := range evs {
		if e.Action != "slot.identity.set" && e.At >= c.AssignedAt-handOverGrace {
			start = i
		}
	}
	cut, restamped, prev := c.AssignedAt, false, ""
	firstLinkAt := int64(-1)
	for _, e := range evs[start:] {
		var d struct {
			IdentityID *string `json:"identity_id"`
		}
		if json.Unmarshal([]byte(e.Details), &d) != nil || d.IdentityID == nil {
			continue
		}
		id := NormalizeIdentityID(*d.IdentityID)
		if id == "" {
			continue // a withdrawal names no holder
		}
		if prev == "" && firstLinkAt < 0 {
			firstLinkAt = e.At
		}
		if prev != "" && id != prev {
			restamped = true
			if e.At > cut {
				cut = e.At
			}
		}
		prev = id
	}
	if !restamped && firstLinkAt > c.AssignedAt+handOverGrace {
		var rows []int64
		if err := q.SelectContext(ctx, &rows,
			`SELECT t FROM (
			   SELECT granted_at AS t FROM machine_grants WHERE custodian = $1
			   UNION ALL SELECT created_at FROM grants WHERE producer = $1 OR (grantee_kind = 'principal' AND grantee_id = $1)
			   UNION ALL SELECT enrolled_at FROM slots WHERE enrolled_by = $1 AND enrolled_at IS NOT NULL
			   UNION ALL SELECT issued_at FROM slot_leaves WHERE slot_id = $1 AND serial <> ''
			 ) x WHERE t >= $2 AND t < $3`, c.SlotID, c.AssignedAt, firstLinkAt-handOverGrace); err != nil {
			return 0, false, fmt.Errorf("reissue cut-off: allowances before the first linkage: %w", err)
		}
		if len(rows) > 0 {
			restamped = true
			if firstLinkAt > cut {
				cut = firstLinkAt
			}
		}
	}
	var leaves []struct {
		Issuer string `db:"issued_for_identity"`
		At     int64  `db:"issued_at"`
	}
	if err := q.SelectContext(ctx, &leaves,
		`SELECT issued_for_identity, issued_at FROM slot_leaves
		  WHERE slot_id = $1 AND generation = 0 AND issued_for_identity <> ''
		  ORDER BY id`, c.SlotID); err != nil {
		return 0, false, fmt.Errorf("reissue cut-off: leaves: %w", err)
	}
	prev = ""
	for _, l := range leaves {
		id := NormalizeIdentityID(l.Issuer)
		if prev != "" && id != prev {
			restamped = true
			if l.At > cut {
				cut = l.At
			}
		}
		prev = id
	}
	return cut, restamped, nil
}

// ApplyReissue advances one candidate slot's generation, supersedes its
// leaves, sets up its holder's path to a new leaf (ReissuePath) and records an
// "rr3p-reissue" audit event, in one transaction. It re-checks the candidate
// condition inside the transaction, so a slot that changed hands or moved
// generation since the listing is skipped (false, nil).
//
// It also ends what the earlier holder left on the anchor (ReissueEnded): the
// machine grant, custody of their machines, the topic budget and the data
// grants naming the anchor, where each predates the current assignment.
// Allowances set since the assignment are the current holder's and stay.
func (r *Registry) ApplyReissue(ctx context.Context, actor string, c ReissueCandidate) (bool, error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return false, fmt.Errorf("apply reissue: begin: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	cands, err := reissueCandidates(ctx, tx)
	if err != nil {
		return false, fmt.Errorf("apply reissue: recheck: %w", err)
	}
	found := false
	for _, s := range cands {
		if s.SlotID == c.SlotID {
			found, c.Cutoff, c.Restamped = true, s.Cutoff, s.Restamped
		}
	}
	if !found {
		return false, nil
	}
	now := time.Now().Unix()
	var gen int64
	if err := tx.GetContext(ctx, &gen,
		`UPDATE slots SET generation = generation + 1 WHERE slot_id = $1 RETURNING generation`, c.SlotID); err != nil {
		return false, fmt.Errorf("apply reissue: bump: %w", err)
	}
	if err := supersedeLeaves(ctx, tx, c.SlotID, now); err != nil {
		return false, err
	}
	assignedAt := c.Cutoff
	for _, q := range []string{
		`DELETE FROM machine_grants WHERE ` + reissueMachineGrantsWhere,
		`UPDATE slots SET enrolled_by = NULL WHERE ` + reissueCustodyWhere,
		`DELETE FROM topic_budgets WHERE ` + reissueBudgetsWhere,
	} {
		if _, err := tx.ExecContext(ctx, q, c.SlotID, assignedAt); err != nil {
			return false, fmt.Errorf("apply reissue: inherited allowances: %w", err)
		}
	}
	if err := endAnchorGrants(ctx, tx, c.SlotID, now, "rr3p-reissue", assignedAt); err != nil {
		return false, fmt.Errorf("apply reissue: %w", err)
	}
	path := c.Path()
	if path == ReissueRecollect {
		if _, err := tx.ExecContext(ctx,
			`UPDATE slots SET cert_minted_at = NULL, bundle_token = NULL, bundle_path = NULL, bundle_minted_at = NULL, collected_at = NULL
			  WHERE slot_id = $1`, c.SlotID); err != nil {
			return false, fmt.Errorf("apply reissue: clear collection: %w", err)
		}
	}
	if _, err := tx.ExecContext(ctx,
		`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1, $2, 'rr3p-reissue', $3, $4)`,
		now, actor, c.SlotID, fmt.Sprintf(`{"anchor":%q,"generation":%d,"path":%q}`, c.SlotID, gen, path)); err != nil {
		return false, fmt.Errorf("apply reissue: audit: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return false, fmt.Errorf("apply reissue: commit: %w", err)
	}
	return true, nil
}

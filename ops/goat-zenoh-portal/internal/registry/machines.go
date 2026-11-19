package registry

// ADR 1096 — delegated machine enrollment (docs/design/machine-enrollment.md).
//
// Stage 1: the quota + custody-edge model. A custodian person carries an
// enrollment_grant (a quota — onboard-tier-role-model.md §5); each machine they
// enroll takes its own slot (ADR 1025) and gets stamped with the person→system
// custody edge plus the grammar born-as snapshot (node-class / autonomy /
// inherited affiliation — ADR 1043 / identity-grammar.md §1.4, §2).
//
// Remaining quota is DERIVED, never stored: quota − COUNT(live enrolled slots).
// So revoking a machine (slots.revoked_at set) returns its quota by construction.

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"sort"
	"strings"
	"time"

	"github.com/jmoiron/sqlx"
)

// normalizeMachineCustodians rewrites legacy handle-keyed custodian values to the
// canonical anchor form, in machine_grants AND in the slots.enrolled_by stamps the
// quota join counts — both or neither, since normalizing only one side is what
// splits a quota. Runs at Open(); idempotent, so every boot after the first
// rewrites nothing.
//
// A row whose anchor form ALREADY exists is left alone and logged: two grants for
// one person is an operator-resolvable conflict, and silently merging them would
// pick a quota nobody chose.
func normalizeMachineCustodians(ctx context.Context, db *sqlx.DB) error {
	type aliasRow struct {
		SlotID string `db:"slot_id"`
		Alias  string `db:"assigned_to"`
	}
	var live []aliasRow
	if err := db.SelectContext(ctx, &live,
		`SELECT slot_id, assigned_to FROM slots WHERE assigned_to IS NOT NULL AND revoked_at IS NULL`); err != nil {
		return fmt.Errorf("read live slot aliases: %w", err)
	}
	anchorFor := make(map[string]string, len(live))
	for _, a := range live {
		if _, dup := anchorFor[a.Alias]; !dup {
			anchorFor[a.Alias] = a.SlotID
		}
	}
	if len(anchorFor) == 0 {
		return nil // per-vendor goatnet, or an uninitialized pool
	}

	var granted []string
	if err := db.SelectContext(ctx, &granted, `SELECT custodian FROM machine_grants`); err != nil {
		return fmt.Errorf("read machine grants: %w", err)
	}
	haveAnchorRow := map[string]bool{}
	for _, c := range granted {
		haveAnchorRow[c] = true
	}
	for _, c := range granted {
		anchor, isHandle := anchorFor[c]
		if !isHandle || anchor == c {
			continue
		}
		if haveAnchorRow[anchor] {
			slog.Warn("machine grant carries both a handle-keyed and an anchor-keyed row; leaving both for operator resolution (quota is read from the anchor row)",
				"handle", c, "anchor", anchor)
			continue
		}
		if _, err := db.ExecContext(ctx, `UPDATE machine_grants SET custodian = $1 WHERE custodian = $2`, anchor, c); err != nil {
			return fmt.Errorf("normalize grant custodian %q: %w", c, err)
		}
		haveAnchorRow[anchor] = true
		slog.Info("normalized machine grant custodian to its slot anchor", "handle", c, "anchor", anchor)
	}

	var stamped []string
	if err := db.SelectContext(ctx, &stamped,
		`SELECT DISTINCT enrolled_by FROM slots WHERE enrolled_by IS NOT NULL`); err != nil {
		return fmt.Errorf("read custody stamps: %w", err)
	}
	for _, c := range stamped {
		anchor, isHandle := anchorFor[c]
		if !isHandle || anchor == c {
			continue
		}
		if _, err := db.ExecContext(ctx, `UPDATE slots SET enrolled_by = $1 WHERE enrolled_by = $2`, anchor, c); err != nil {
			return fmt.Errorf("normalize custody stamp %q: %w", c, err)
		}
		slog.Info("normalized custody stamp to the custodian's slot anchor", "handle", c, "anchor", anchor)
	}

	// slot_leaves.issued_for_custodian (GHSA-rr3p) is copied from enrolled_by,
	// by migration 2's backfill before this normalization runs, so it can hold
	// the same legacy spelling; the machine-quota quarantine count keys on the
	// anchor form.
	var leafCustodians []string
	if err := db.SelectContext(ctx, &leafCustodians,
		`SELECT DISTINCT issued_for_custodian FROM slot_leaves WHERE issued_for_custodian <> ''`); err != nil {
		return fmt.Errorf("read leaf custodians: %w", err)
	}
	for _, c := range leafCustodians {
		anchor, isHandle := anchorFor[c]
		if !isHandle || anchor == c {
			continue
		}
		if _, err := db.ExecContext(ctx, `UPDATE slot_leaves SET issued_for_custodian = $1 WHERE issued_for_custodian = $2`, anchor, c); err != nil {
			return fmt.Errorf("normalize leaf custodian %q: %w", c, err)
		}
	}
	return nil
}

// Machine-enrollment domain errors. Callers errors.Is against these.
var (
	// ErrQuotaExhausted means the custodian has no remaining enrollment quota
	// (no grant row counts as quota 0 — fail-closed).
	ErrQuotaExhausted = errors.New("machine enrollment quota exhausted")
	// ErrInvalidNodeClass / ErrInvalidAutonomy reject a value outside the closed
	// grammar taxonomy.
	ErrInvalidNodeClass = errors.New("invalid node-class")
	ErrInvalidAutonomy  = errors.New("invalid autonomy level")
	// ErrSlotNotEnrollable means the target slot is unknown, unassigned, or
	// revoked — nothing to stamp a custody edge onto.
	ErrSlotNotEnrollable = errors.New("slot is not assigned (cannot record machine enrollment)")
	// ErrInvalidWorkload rejects a PLATFORM sub-workload declaration outside the
	// function-workload taxonomy, or workloads on a non-PLATFORM node-class.
	ErrInvalidWorkload = errors.New("invalid platform workload")
	// ErrNoMeshMapping means the node-class has no derived NetBird group. It is a
	// REFUSAL, not an empty grant: a machine whose class cannot say which groups
	// it belongs in must not be minted a key that says "none".
	ErrNoMeshMapping = errors.New("node-class has no mesh group derivation")
)

// functionNodeClasses are the mission-payload node-classes a custodian may
// enroll self-service. infraNodeClasses shape the goatnet itself and are
// operator-gated at the flow layer (Stage 3); IsInfraNodeClass is the gate's
// predicate. Values match identity-grammar.md §1.4 exactly.
var (
	functionNodeClasses = map[string]bool{"SENSOR": true, "EFFECTOR": true, "PROCESSOR": true, "PLATFORM": true}
	infraNodeClasses    = map[string]bool{"MGMT-HOST": true, "ZENOH": true, "SNITCH": true, "SERVICES": true, "NETWORK": true, "BREAKGLASS": true, "NETWORK-JUMP": true, "EDGE-SERVER": true}
	autonomyLevels      = map[string]bool{"manual": true, "remote": true, "supervised": true, "autonomous": true}

	// delegableInfraNodeClasses are the infrastructure classes a site network
	// admin may enroll for their OWN site, instead of the operator (ADR 1113).
	// Deliberately a separate, explicitly-enumerated set rather than a flag on
	// the class: widening it is a decision that should show up in a diff.
	//
	// EDGE-SERVER (ADR 1216) sits beside NETWORK under the same entitlement: a
	// named server that directly supports the site's edge devices, with
	// NETWORK's exact reach shape. BREAKGLASS and NETWORK-JUMP stay out.
	delegableInfraNodeClasses = map[string]bool{"NETWORK": true, "EDGE-SERVER": true}

	// gearNodeClasses — ADR 1116. A class is a FUNCTION and a box holds one per
	// function it performs, which is why these are finer-grained than the
	// whole-system `SENSOR`/`EFFECTOR` they replace: an effector navigator box is
	// EFFECTOR-CUE *and* EFFECTOR-ARBITER, one box, two functions.
	//
	// This enum is the mint-time half of ADR 1116 §9 ("the setup key is the
	// enforcement point, not this document"). The site-one cohort of
	// put an effector ground station, a sensor gateway and a human laptop into
	// one group in a single sitting — not carelessness, but because the
	// taxonomy offered nothing that fit and the mint accepted the catch-all.
	// Keep this list in step with ops/ansible/netbird_acl_taxonomy.yml; the
	// gear class set is shared vocabulary, not a portal-local idea.
	gearNodeClasses = map[string]bool{
		"SENSOR-FEED": true, "SENSOR-TASKING": true, "SENSOR-TELEMETRY": true, "SENSOR-ARBITER": true,
		"EFFECTOR-CUE": true, "EFFECTOR-TRACK": true, "EFFECTOR-TELEMETRY": true,
		"EFFECTOR-COMMAND": true, "EFFECTOR-ARBITER": true,
		"POD": true, "MOON": true, "KID": true, "EXTERNAL-KID": true,
		"EXTERNAL-INTERMEDIARY": true, "FUSION": true,
		"CONTROL": true,
	}

	// reservedNodeClasses are NAMED but never enrollable (ADR 1116 §3): the
	// sensing element itself and the interceptor itself. They stay in
	// ValidNodeClass so historical rows and reads still resolve — several
	// machines were enrolled as whole-system `SENSOR`/`EFFECTOR` before the
	// gear classes existed — but IsEnrollableNodeClass refuses them at the
	// mint. Naming a ground station `EFFECTOR` is the OPSYSTEM trap in
	// miniature: the name stops describing the members.
	reservedNodeClasses = map[string]bool{"SENSOR": true, "EFFECTOR": true}

	// intakeOnlyNodeClasses accept direction but grant no outbound reach of
	// their own (ADR 1116): they are a DESTINATION in the ACL and a source in
	// nothing. A peer whose only class is one of these is uncovered — it could
	// not reach the router to receive the very tasking the class exists for.
	// They compose onto a class that carries the outbound half, so the mint
	// refuses them as a primary class and says which one to use instead.
	intakeOnlyNodeClasses = map[string]bool{"SENSOR-TASKING": true, "EFFECTOR-COMMAND": true}

	// standaloneOnlyNodeClasses are enrollable gear appliances whose class is
	// also a trust boundary (ADR 1092), not a function that may be composed into
	// a PLATFORM. Keeping this separate from gearNodeClasses preserves direct,
	// self-service enrollment without widening a composite peer's memberships.
	standaloneOnlyNodeClasses = map[string]bool{"POD": true, "MOON": true}

	// nodeClassMeshGroups is the class → NetBird group-suffix derivation, and
	// the reason the mint can stop asking a human which groups to pick. Absent
	// = no mapping, and the mint says so rather than guessing: an invented
	// group is how the catch-all formed in the first place.
	//
	// Suffix-only, exactly like the ACL matrix — the site prefix is applied at
	// mint time, so the same table serves every goatnet.
	nodeClassMeshGroups = map[string][]string{
		// Gear: the class IS the group. Each is a rule-A source in the matrix,
		// so a peer holding only its class is covered.
		"SENSOR-FEED": {"SENSOR-FEED"}, "SENSOR-TELEMETRY": {"SENSOR-TELEMETRY"},
		"SENSOR-ARBITER": {"SENSOR-ARBITER"},
		"EFFECTOR-CUE":   {"EFFECTOR-CUE"}, "EFFECTOR-TRACK": {"EFFECTOR-TRACK"},
		"EFFECTOR-TELEMETRY": {"EFFECTOR-TELEMETRY"}, "EFFECTOR-ARBITER": {"EFFECTOR-ARBITER"},
		"POD": {"POD"}, "MOON": {"MOON"}, "KID": {"KID"},
		"EXTERNAL-KID": {"EXTERNAL-KID"}, "CONTROL": {"CONTROL"},
		"EXTERNAL-INTERMEDIARY": {"EXTERNAL-INTERMEDIARY"}, "FUSION": {"FUSION"},

		// Infrastructure classes that DO have a group of their own.
		"NETWORK": {"NETWORK"},

		// ADR 1114. Deliberately NOT in delegableInfraNodeClasses: the
		// entitlement that lets a site admin enroll a switch must not let them
		// enroll their own way into the rescue path.
		"BREAKGLASS": {"BREAKGLASS"},

		// ADR 1131 §2: class and group carry the same name (the F-213
		// name-match rule). Its reach — tcp 22 to NETWORK and the management
		// plane to INFRA, nothing inbound — is in the matrix, not here.
		"NETWORK-JUMP": {"NETWORK-JUMP"},

		// ADR 1216: class and group carry the same name. NETWORK's
		// shape — reached by PLATFORM-ADMIN and INFRA, initiates only to the
		// INFRA management plane — is in the matrix, not here.
		"EDGE-SERVER": {"EDGE-SERVER"},

		// Infrastructure classes that are substrate: the taxonomy's INFRA is
		// "mgmt hosts, mesh-dns, relay-health, site-hosted service peers",
		// which is exactly these three.
		"MGMT-HOST": {"INFRA"}, "ZENOH": {"INFRA"}, "SERVICES": {"INFRA"},

		// The taxonomy defines OBSERVER as "browse-only participants + read-only
		// observers (snitch probes, monitoring)".
		"SNITCH": {"OBSERVER"},

		// DELIBERATELY UNMAPPED, and each for its own reason:
		//   PLATFORM  — a COMPOSITION, not a class with reach of its own. Its
		//               groups come from the functions it performs; see
		//               MeshGroupsForMachine, which unions its workloads.
		//   PROCESSOR — a legacy whole-system class with no NetBird group. A
		//               peer enrolled as one holds nothing and is uncovered;
		//               inventing a group to hide that is the OPSYSTEM error.
		//               FUSION is the gear class for derived-data workloads.
		//   SENSOR / EFFECTOR — reserved and unenrollable (see above).
	}
)

// ValidNodeClass reports whether nc is a known node-class — function, infra,
// ADR 1116 gear, or reserved. Reserved classes are valid to READ but not to
// enroll; use IsEnrollableNodeClass for the mint-time question.
func ValidNodeClass(nc string) bool {
	return functionNodeClasses[nc] || infraNodeClasses[nc] || gearNodeClasses[nc] || reservedNodeClasses[nc]
}

// IsGearNodeClass reports whether nc is one of the ADR 1116 gear classes.
func IsGearNodeClass(nc string) bool { return gearNodeClasses[nc] }

// IsReservedNodeClass reports whether nc is named-but-never-enrollable
// (ADR 1116 §3 — the sensing element, the interceptor).
func IsReservedNodeClass(nc string) bool { return reservedNodeClasses[nc] }

// IsIntakeOnlyNodeClass reports whether nc grants inbound permission only, and
// so must never be a machine's primary class.
func IsIntakeOnlyNodeClass(nc string) bool { return intakeOnlyNodeClasses[nc] }

// IsEnrollableNodeClass reports whether nc may be minted as a machine's primary
// node-class. Valid-but-not-enrollable covers two cases, each with its own
// refusal message at the flow layer: reserved classes and intake-only classes.
func IsEnrollableNodeClass(nc string) bool {
	return ValidNodeClass(nc) && !reservedNodeClasses[nc] && !intakeOnlyNodeClasses[nc]
}

// MeshGroupsForNodeClass returns the NetBird group SUFFIXES a machine of this
// class belongs in — the derivation that replaces a human choosing auto_groups
// at key-mint time (ADR 1116 §9). Suffix-only; the caller applies the site
// prefix. Returns ErrNoMeshMapping when the class has no group, which is a
// refusal, never an empty grant.
func MeshGroupsForNodeClass(nc string) ([]string, error) {
	groups, ok := nodeClassMeshGroups[nc]
	if !ok {
		return nil, fmt.Errorf("%w: node-class %q has no mesh group derivation", ErrNoMeshMapping, nc)
	}
	out := make([]string, len(groups))
	copy(out, groups)
	return out, nil
}

// MeshGroupsForMachine is the derivation the mint actually uses: it handles the
// PLATFORM composite, which has no reach of its own.
//
// A PLATFORM is modeled as a composition of function-principals (ADR 1098), so
// its mesh reach is the UNION of the classes of the functions it performs —
// which is exactly what NetBird membership does, since holding two groups unions
// their reach. An effector navigator box is the worked example: one box, EFFECTOR-CUE
// and EFFECTOR-ARBITER, both groups.
//
// A PLATFORM with no declared workloads is REFUSED rather than defaulted. "A
// composite of nothing" has no derivable reach, and picking one for it would be
// the catch-all forming again at a different altitude.
func MeshGroupsForMachine(nodeClass string, workloads []string) ([]string, error) {
	// Enforce composition validity here as well as at the API boundary. Direct
	// and re-mint callers must not derive groups for a shape the mint refuses.
	if err := ValidWorkloads(nodeClass, workloads); err != nil {
		return nil, err
	}
	if nodeClass != "PLATFORM" {
		return MeshGroupsForNodeClass(nodeClass)
	}
	if len(workloads) == 0 {
		return nil, fmt.Errorf("%w: a PLATFORM is a composition and has no reach of its own — declare the functions it performs (--workloads) so its groups can be derived from them", ErrNoMeshMapping)
	}
	seen := map[string]bool{}
	var out []string
	for _, w := range workloads {
		gs, err := MeshGroupsForNodeClass(w)
		if err != nil {
			return nil, fmt.Errorf("platform workload %q: %w", w, err)
		}
		for _, g := range gs {
			if !seen[g] {
				seen[g] = true
				out = append(out, g)
			}
		}
	}
	sort.Strings(out)
	return out, nil
}

// IsInfraNodeClass reports whether nc is an infrastructure node-class — the
// predicate for the Stage-3 operator gate (ADR 1096 decision #1). A false here
// for an otherwise-valid node-class means "function class, custodian self-service".
func IsInfraNodeClass(nc string) bool { return infraNodeClasses[nc] }

// IsDelegableInfraNodeClass reports whether nc is an infrastructure class that a
// site network admin may enroll for their own site without the goatnet operator
// (ADR 1113). Callers MUST still verify the caller's entitlement and that the
// enrollment is site-scoped — this predicate answers only "is this class
// delegable at all", never "is this caller allowed".
func IsDelegableInfraNodeClass(nc string) bool { return delegableInfraNodeClasses[nc] }

// ValidAutonomy reports whether a is a known autonomy level.
func ValidAutonomy(a string) bool { return autonomyLevels[a] }

// IsFunctionWorkloadClass reports whether nc is a function-workload class a
// PLATFORM may decompose into (identity-grammar.md §1.4, ADR 0412 / ADR 1098).
//
// Most ADR 1116 gear classes belong here, and are in fact the ADR's own worked
// example: an effector navigator box is one box performing EFFECTOR-CUE and
// EFFECTOR-ARBITER, which is a PLATFORM decomposing into two gear functions.
// POD and MOON are the exception: they are standalone ADR 1092 appliance/trust
// boundaries, not composable function workloads.
// The legacy whole-system SENSOR/EFFECTOR/PROCESSOR stay accepted so existing
// recorded declarations keep validating; new ones should use a gear class,
// which says what the workload DOES rather than what the box is.
func IsFunctionWorkloadClass(nc string) bool {
	if gearNodeClasses[nc] {
		return !standaloneOnlyNodeClasses[nc]
	}
	return functionNodeClasses[nc] && nc != "PLATFORM"
}

// ValidWorkloads checks a PLATFORM's declared sub-workload composition: workloads
// are valid ONLY on a PLATFORM node-class, and each must be a function-workload
// class. Empty is valid (a platform may declare its composition later; a
// non-platform simply carries none).
func ValidWorkloads(nodeClass string, workloads []string) error {
	if len(workloads) == 0 {
		return nil
	}
	if nodeClass != "PLATFORM" {
		return fmt.Errorf("%w: workloads are only valid on a PLATFORM node-class, not %q", ErrInvalidWorkload, nodeClass)
	}
	for _, w := range workloads {
		if !IsFunctionWorkloadClass(w) {
			if standaloneOnlyNodeClasses[w] {
				return fmt.Errorf("%w: %q is a standalone appliance/trust-boundary class, not a composable PLATFORM workload", ErrInvalidWorkload, w)
			}
			return fmt.Errorf("%w: %q (want an ADR 1116 gear class such as EFFECTOR-CUE or SENSOR-FEED)", ErrInvalidWorkload, w)
		}
	}
	return nil
}

// MachineGrant is one custodian's enrollment_grant row: the quota plus the
// INHERITED attributes (affiliation/site) every enrolled machine is stamped with
// (onboard-tier-role-model.md §5).
type MachineGrant struct {
	Custodian   string `db:"custodian"   json:"custodian"`
	Quota       int64  `db:"quota"       json:"quota"`
	Affiliation string `db:"affiliation" json:"affiliation"`
	Site        string `db:"site"        json:"site"`
	// NetworkAdmin is the ADR 1113 delegated entitlement: may this custodian
	// enroll the delegable infrastructure classes (NETWORK) without the goatnet
	// operator? Bounded to Site — a network admin administers THEIR site's
	// bearer gear, never another's. Fail-closed (0) for every legacy row.
	NetworkAdmin bool   `db:"network_admin" json:"network_admin"`
	GrantedBy    string `db:"granted_by"  json:"granted_by"`
	GrantedAt    int64  `db:"granted_at"  json:"granted_at"`
	UpdatedAt    int64  `db:"updated_at"  json:"updated_at"`
}

// MachineQuota is the derived quota view for a custodian: the granted ceiling,
// how many live machines they have enrolled, and what remains.
type MachineQuota struct {
	Custodian string `json:"custodian"`
	Quota     int64  `json:"quota"`
	Used      int64  `json:"used"`
	// Quarantined is the number of free pool slots this custodian's revoked
	// machines keep out of circulation until their leaves expire (GHSA-rr3p
	// reuse quarantine). They count against the quota; an operator who
	// replaces machines raises the quota rather than waiting a year.
	Quarantined int64 `json:"quarantined"`
	Remaining   int64 `json:"remaining"`
}

// normalizeCustodian resolves a custodian identifier to its canonical form: the
// slot ANCHOR. The custodian key was polymorphic — an anchor, a slot alias
// (handle), or operator free text — and the quota join keys on it, so a grant
// written under one spelling and an enrollment stamped under another silently
// read quota 0 or a doubled quota (identity fragility audit §1.2). The
// anchor is the only spelling that is stable under a re-issue and unique per
// live slot, so every read and write funnels through here.
//
// A value that resolves to no live slot is returned unchanged — an operator may
// legitimately pre-grant against a handle that has no slot yet, and refusing here
// would break that. It just doesn't get the join guarantee until it does.
func (r *Registry) normalizeCustodian(ctx context.Context, custodian string) string {
	if custodian == "" {
		return custodian
	}
	anchor, err := r.AnchorForHandle(ctx, custodian)
	if err != nil || anchor == "" {
		return custodian
	}
	return anchor
}

// SetMachineQuota upserts a custodian's enrollment_grant. Seeded at person-onboard
// from the manifest machine_quota; an operator may re-set it later. quota must be
// non-negative; grantedBy is the attributed actor.
func (r *Registry) SetMachineQuota(ctx context.Context, custodian string, quota int64, grantedBy string) error {
	if custodian == "" {
		return errors.New("custodian handle required")
	}
	custodian = r.normalizeCustodian(ctx, custodian)
	if quota < 0 {
		return fmt.Errorf("quota must be non-negative, got %d", quota)
	}
	now := time.Now().Unix()
	// granted_at is preserved on update (COALESCE to the existing row); only
	// updated_at moves.
	_, err := r.db.ExecContext(ctx, `
		INSERT INTO machine_grants (custodian, quota, granted_by, granted_at, updated_at)
		VALUES ($1, $2, $3, $4, $5)
		ON CONFLICT(custodian) DO UPDATE SET
		    quota      = excluded.quota,
		    granted_by = excluded.granted_by,
		    updated_at = excluded.updated_at`,
		custodian, quota, grantedBy, now, now)
	if err != nil {
		return fmt.Errorf("set machine quota: %w", err)
	}
	return nil
}

// SetMachineGrant upserts a custodian's full enrollment_grant — quota plus the
// inherited affiliation/site every machine they enroll is stamped with. This is
// the seed the person-onboard writes (from the manifest machine_quota + the
// person's unit); affiliation is required (a grant with no unit can stamp nothing,
// so the enroll flow fail-closes on it). granted_at is preserved across updates.
func (r *Registry) SetMachineGrant(ctx context.Context, custodian string, quota int64, affiliation, site, grantedBy string) error {
	if custodian == "" {
		return errors.New("custodian handle required")
	}
	if quota < 0 {
		return fmt.Errorf("quota must be non-negative, got %d", quota)
	}
	affiliation = cleanAffiliation(affiliation)
	if affiliation == "" {
		return errors.New("grant affiliation required (a machine carries its custodian's unit)")
	}
	// Resolve the handle to its anchor BEFORE opening the transaction: the
	// registry runs on a single SQLite connection, so a read issued while the
	// transaction holds it would deadlock.
	custodian = r.normalizeCustodian(ctx, custodian)

	tx, err := r.beginLocked(ctx)
	if err != nil {
		return fmt.Errorf("begin grant tx: %w", err)
	}
	defer func() { _ = tx.Rollback() }()

	// The spelling check and the upsert share one transaction so that two
	// concurrent first grants carrying variant spellings of the same new unit
	// cannot both pass the check and both land.
	if err := checkAffiliationSpelling(ctx, tx, custodian, affiliation); err != nil {
		return err
	}

	now := time.Now().Unix()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO machine_grants (custodian, quota, affiliation, site, granted_by, granted_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
		ON CONFLICT(custodian) DO UPDATE SET
		    quota       = excluded.quota,
		    affiliation = excluded.affiliation,
		    site        = excluded.site,
		    granted_by  = excluded.granted_by,
		    updated_at  = excluded.updated_at`,
		custodian, quota, affiliation, site, grantedBy, now, now); err != nil {
		return fmt.Errorf("set machine grant: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit machine grant: %w", err)
	}
	return nil
}

// SetNetworkAdmin sets (or clears) the ADR 1113 delegated network-admin
// entitlement on an EXISTING grant. Deliberately separate from SetMachineGrant:
// quota is routine capacity management, this is an authority delegation, and the
// two should not be adjustable by the same accidental call. Refuses when the
// custodian holds no grant — the entitlement extends a grant, it does not create
// one — and refuses to grant without a site, since the whole point is that a
// network admin's reach is bounded to their own site.
func (r *Registry) SetNetworkAdmin(ctx context.Context, custodian string, enabled bool, grantedBy string) error {
	if custodian == "" {
		return errors.New("custodian handle required")
	}
	g, err := r.MachineGrantFor(ctx, custodian)
	if err != nil {
		return err
	}
	if enabled && g.Site == "" {
		return errors.New("network-admin entitlement requires the grant to carry a site (it is site-scoped by design; set it with a machine grant first)")
	}
	custodian = r.normalizeCustodian(ctx, custodian)
	_, err = r.db.ExecContext(ctx, `
		UPDATE machine_grants
		   SET network_admin = $1, granted_by = $2, updated_at = $3
		 WHERE custodian = $4`,
		boolInt(enabled), grantedBy, time.Now().Unix(), custodian)
	if err != nil {
		return fmt.Errorf("set network-admin entitlement: %w", err)
	}
	return nil
}

// MachineGrantFor returns a custodian's enrollment_grant row, or ErrNotFound when
// the custodian holds no grant (the enroll flow maps that to a fail-closed refuse).
func (r *Registry) MachineGrantFor(ctx context.Context, custodian string) (*MachineGrant, error) {
	if custodian == "" {
		return nil, errors.New("custodian handle required")
	}
	custodian = r.normalizeCustodian(ctx, custodian)
	var g MachineGrant
	err := r.reader().GetContext(ctx, &g, `SELECT * FROM machine_grants WHERE custodian = $1`, custodian)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("read machine grant: %w", err)
	}
	return &g, nil
}

// liveEnrolledCount counts a custodian's live (non-revoked) enrolled machines,
// optionally excluding one slot (so a re-stamp of an already-owned slot is not
// double-counted). Runs inside the caller's tx when q is a *sqlx.Tx-backed
// queryer; here it takes the registry db for the read-only path.
func (r *Registry) liveEnrolledCount(ctx context.Context, custodian, excludeSlotID string) (int64, error) {
	var used int64
	err := r.reader().GetContext(ctx, &used,
		`SELECT COUNT(*) FROM slots
		 WHERE enrolled_by = $1 AND revoked_at IS NULL AND slot_id != $2`,
		custodian, excludeSlotID)
	if err != nil {
		return 0, fmt.Errorf("count enrolled machines: %w", err)
	}
	return used, nil
}

// MachineQuotaFor returns the derived quota view for a custodian. A custodian
// with no grant row reads quota 0 (fail-closed) — not an error.
func (r *Registry) MachineQuotaFor(ctx context.Context, custodian string) (*MachineQuota, error) {
	if custodian == "" {
		return nil, errors.New("custodian handle required")
	}
	custodian = r.normalizeCustodian(ctx, custodian)
	var quota int64
	err := r.reader().GetContext(ctx, &quota, `SELECT quota FROM machine_grants WHERE custodian = $1`, custodian)
	if errors.Is(err, sql.ErrNoRows) {
		quota = 0
	} else if err != nil {
		return nil, fmt.Errorf("read machine grant: %w", err)
	}
	used, err := r.liveEnrolledCount(ctx, custodian, "")
	if err != nil {
		return nil, err
	}
	var quarantined int64
	if err := r.reader().GetContext(ctx, &quarantined, machineQuarantineSQL, custodian, SlotLeafClock().Unix()); err != nil {
		return nil, fmt.Errorf("count quarantined machine slots: %w", err)
	}
	remaining := quota - used - quarantined
	if remaining < 0 {
		remaining = 0 // a lowered quota can leave used > quota; never report negative
	}
	return &MachineQuota{Custodian: custodian, Quota: quota, Used: used, Quarantined: quarantined, Remaining: remaining}, nil
}

// RecordMachineEnrollment stamps the custody edge + grammar born-as snapshot onto
// an already-allocated slot (the caller allocated it via AllocateSlot). It is the
// atomic check-and-consume point: the quota ceiling is enforced inside the tx, so
// concurrent enrollments cannot together exceed the grant. Idempotent for the
// same (slot, custodian) — a re-stamp updates the grammar snapshot without
// double-counting.
//
// affiliation is the custodian's inherited unit anchor — never a mint parameter;
// the caller resolves it from the grant, this layer only persists the snapshot.
// The Stage-3 operator gate (IsInfraNodeClass) lives at the flow layer, not here:
// this primitive records whatever valid node-class it is handed.
//
// workloads is a PLATFORM's declared sub-workload composition (function classes);
// nil/empty for a simple system. Stored comma-joined as the born-as declaration
// the SPIRE role registers SVIDs from (ADR 1098, Option B) — the portal records
// the composition, SPIRE owns the SVIDs.
func (r *Registry) RecordMachineEnrollment(ctx context.Context, slotID, custodian, nodeClass, autonomy, affiliation string, workloads []string) error {
	if slotID == "" || custodian == "" {
		return errors.New("slot id and custodian handle required")
	}
	if !ValidNodeClass(nodeClass) {
		return fmt.Errorf("%w: %q", ErrInvalidNodeClass, nodeClass)
	}
	if !ValidAutonomy(autonomy) {
		return fmt.Errorf("%w: %q", ErrInvalidAutonomy, autonomy)
	}
	if err := ValidWorkloads(nodeClass, workloads); err != nil {
		return err
	}
	affiliation = cleanAffiliation(affiliation)
	if affiliation == "" {
		return errors.New("inherited affiliation required (a machine carries its custodian's unit)")
	}
	// Stamp the SAME spelling the grant + quota reads use, or the quota join
	// silently splits in two (audit §1.2).
	custodian = r.normalizeCustodian(ctx, custodian)

	tx, err := r.beginLocked(ctx)
	if err != nil {
		return fmt.Errorf("begin enroll tx: %w", err)
	}
	defer func() { _ = tx.Rollback() }()

	// The target slot must exist, be assigned, and not be revoked.
	var (
		assignedTo *string
		revokedAt  *int64
	)
	err = tx.QueryRowxContext(ctx,
		`SELECT assigned_to, revoked_at FROM slots WHERE slot_id = $1`, slotID).Scan(&assignedTo, &revokedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrSlotNotEnrollable
	}
	if err != nil {
		return fmt.Errorf("load slot: %w", err)
	}
	if assignedTo == nil || revokedAt != nil {
		return ErrSlotNotEnrollable
	}

	// The spelling guard runs here as well as on the grant, in the same
	// transaction as the write. The custody retro-stamp
	// (PUT /api/slots/{anchor}/custody) hands this layer a caller-stated
	// affiliation, and a fresh enrollment's inherited spelling can meet a
	// different spelling already stamped on a slot from before the guard.
	// Either way the column is not written with a second spelling of a unit
	// on file; an exact match (the common enroll case) passes untouched.
	if err := checkAffiliationSpelling(ctx, tx, custodian, affiliation); err != nil {
		return err
	}

	// Enforce the quota ceiling atomically. Excluding this slot makes a re-stamp
	// of an already-owned slot a no-cost update rather than a quota hit.
	var quota int64
	err = tx.GetContext(ctx, &quota, `SELECT quota FROM machine_grants WHERE custodian = $1`, custodian)
	if errors.Is(err, sql.ErrNoRows) {
		quota = 0
	} else if err != nil {
		return fmt.Errorf("read grant: %w", err)
	}
	var used int64
	if err := tx.GetContext(ctx, &used,
		`SELECT COUNT(*) FROM slots WHERE enrolled_by = $1 AND revoked_at IS NULL AND slot_id != $2`,
		custodian, slotID); err != nil {
		return fmt.Errorf("count enrolled: %w", err)
	}
	var quarantined int64
	if err := tx.GetContext(ctx, &quarantined, machineQuarantineSQL, custodian, SlotLeafClock().Unix()); err != nil {
		return fmt.Errorf("count quarantined: %w", err)
	}
	if used+quarantined >= quota {
		return ErrQuotaExhausted
	}

	if _, err := tx.ExecContext(ctx,
		`UPDATE slots SET enrolled_by = $1, node_class = $2, autonomy = $3, affiliation = $4, workloads = $5, enrolled_at = $6
		 WHERE slot_id = $7`,
		custodian, nodeClass, autonomy, affiliation, strings.Join(workloads, ","), time.Now().Unix(), slotID); err != nil {
		return fmt.Errorf("stamp machine enrollment: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit enroll: %w", err)
	}
	return nil
}

// MachinesForCustodian lists a custodian's live (non-revoked) enrolled machines,
// newest enrollment first — the read model behind the custody surface (Stage 5)
// and the operator's "show me this person's fleet" view.
func (r *Registry) MachinesForCustodian(ctx context.Context, custodian string) ([]Slot, error) {
	if custodian == "" {
		return nil, errors.New("custodian handle required")
	}
	custodian = r.normalizeCustodian(ctx, custodian)
	var out []Slot
	err := r.reader().SelectContext(ctx, &out,
		`SELECT * FROM slots
		 WHERE enrolled_by = $1 AND revoked_at IS NULL
		 ORDER BY enrolled_at DESC, idx ASC`, custodian)
	if err != nil {
		return nil, fmt.Errorf("list machines for custodian: %w", err)
	}
	return out, nil
}

// MachineByAnchor reads one live custody-enrolled machine by its slot anchor.
// ErrNotFound covers both "no such slot" and "not a machine" (no custody
// stamp, or revoked) — the caller's question is "is this an enrolled machine",
// and either way the answer is no.
func (r *Registry) MachineByAnchor(ctx context.Context, anchor string) (*Slot, error) {
	var s Slot
	err := r.reader().GetContext(ctx, &s,
		`SELECT * FROM slots
		 WHERE slot_id = $1 AND enrolled_by IS NOT NULL AND revoked_at IS NULL`, anchor)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("read machine by anchor: %w", err)
	}
	return &s, nil
}

// AllEnrolledMachines lists every live (non-revoked) custody-enrolled machine
// across all custodians — the operator fleet view (stage 5). Only slots carrying
// an ADR 1096 custody stamp are returned; a developer self-slot (enrolled_by
// NULL) is not a machine and is excluded. Newest enrollment first.
func (r *Registry) AllEnrolledMachines(ctx context.Context) ([]Slot, error) {
	var out []Slot
	err := r.reader().SelectContext(ctx, &out,
		`SELECT * FROM slots
		 WHERE enrolled_by IS NOT NULL AND revoked_at IS NULL
		 ORDER BY enrolled_at DESC, idx ASC`)
	if err != nil {
		return nil, fmt.Errorf("list all enrolled machines: %w", err)
	}
	return out, nil
}

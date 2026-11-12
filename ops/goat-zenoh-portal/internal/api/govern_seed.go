package api

// govern_seed.go implements the ADR 1102 governance cutover from the
// bootstrap-open kick-start posture to the canonical fail-closed model. Two
// mechanisms, both keyed on identity UUIDs resolved from the roster's per-org
// email-hashes via a single Kratos scan (the email-hash bridge, the F-234 key;
// nomination stores a UUID, the roster carries only the hash):
//
//  1. One-time existing-orgs migration (seedExistingOrgsAllMembers). At the
//     first boot after the goatnet flips to fail-closed, every current person
//     member of every not-yet-narrowed org is nominated as an explicit admin.
//     Behavior-preserving: under bootstrap-open every member already governed,
//     so nominating them all keeps exactly that authority as the flag flips —
//     it is NOT retroactive-first-member; existing orgs keep all members. A
//     completion marker in govern_meta makes it run exactly once.
//
//  2. Going-forward first-member reconcile (reconcileFirstMember). On every
//     boot (and periodically) while fail-closed, any org with person members
//     but no active admins gets its FIRST resolvable roster member seeded as
//     admin — a new org is born governed by its first onboardee, who then
//     delegates. Existing orgs, seeded in (1), already have admins, so this
//     never touches them.
//
// Order within a boot: migration first (marker-guarded), then reconcile. On the
// cutover boot the migration seeds every existing org, leaving the reconcile a
// no-op; on later boots the migration is skipped and the reconcile handles new
// orgs only. Both are idempotent — a duplicate nomination is skipped — and both
// are gated on the fail-closed flag: while the goatnet is bootstrap-open,
// seeding would NARROW an org (one nomination flips it from everyone-governs to
// nominees-only), so the cutover runs only once the operator has chosen the
// canonical model. Governance admins are persons; a roster member with no
// matching identity is logged and left unseeded (operator proxy stewardship is
// the recovery path, ADR 1102).

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// governCutoverMarkerKey is the govern_meta key the one-time all-members
// migration writes on success so it runs exactly once.
const governCutoverMarkerKey = "cutover_all_members_seeded"

// governSeedSummary is the structured outcome of a seed pass — logged, not
// served. Self-describing counts so the cutover is auditable from the portal log.
type governSeedSummary struct {
	Mode            string   // "all-members" | "first-member"
	OrgsConsidered  int      // orgs with at least one person member
	OrgsSeeded      int      // orgs that gained at least one new admin this pass
	AdminsNominated int      // new nomination rows written
	AlreadyAdmins   int      // duplicate nominations skipped (idempotent re-run)
	SkippedNarrowed int      // orgs left alone because they already have admins
	Unresolved      []string // "org/handle" members with no matching identity
}

// emailHashToUUID scans the identity layer once and returns email-hash →
// identity UUID for every identity with a login email. This is the bridge from
// the roster (email-hash keyed) to the nomination store (UUID keyed).
func (s *Server) emailHashToUUID(ctx context.Context) (map[string]string, error) {
	ids, err := s.humans.listAll(ctx)
	if err != nil {
		return nil, err
	}
	out := make(map[string]string, len(ids))
	for _, id := range ids {
		if id.Email == "" {
			continue
		}
		out[access.EmailHash(id.Email)] = id.ID
	}
	return out, nil
}

// seedGovernAdmins is the shared seed core. firstOnly=false nominates every
// person member of each not-yet-narrowed org (the migration); firstOnly=true
// nominates only each such org's first resolvable member (the new-org
// reconcile). An org that already has active admins is never touched.
func (s *Server) seedGovernAdmins(ctx context.Context, firstOnly bool, nominatedBy string) (governSeedSummary, error) {
	sum := governSeedSummary{Mode: "all-members"}
	if firstOnly {
		sum.Mode = "first-member"
	}
	if s.humans == nil {
		return sum, errors.New("govern seed: identity layer (kratos) not configured")
	}
	if s.cfg.RosterPath == "" {
		return sum, errors.New("govern seed: declared registry (roster path) not configured")
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		return sum, err
	}
	narrowed, err := s.reg.GovernNarrowedOrgs(ctx)
	if err != nil {
		return sum, err
	}
	h2u, err := s.emailHashToUUID(ctx)
	if err != nil {
		return sum, err
	}
	for _, org := range dr.OrgsWithPersonMembers() {
		sum.OrgsConsidered++
		if narrowed[org] {
			sum.SkippedNarrowed++
			continue
		}
		seededThisOrg := 0
		for _, m := range dr.OrgPersonMembers(org) {
			uuid := h2u[m.EmailHash]
			if uuid == "" {
				sum.Unresolved = append(sum.Unresolved, org+"/"+m.Handle)
				continue
			}
			g := &registry.GovernAdmin{
				Org:           org,
				IdentityID:    uuid,
				NominatedBy:   nominatedBy,
				Justification: "ADR 1102 governance cutover (" + sum.Mode + ")",
			}
			switch err := s.reg.NominateGovernAdmin(ctx, g); {
			case err == nil:
				sum.AdminsNominated++
				seededThisOrg++
			case errors.Is(err, registry.ErrGovernAdminExists):
				sum.AlreadyAdmins++
			default:
				return sum, fmt.Errorf("nominate %s (%s): %w", org, uuid, err)
			}
			// First-member mode: the first resolvable member governs the new
			// org; stop after it is seeded (or found already seeded).
			if firstOnly {
				break
			}
		}
		if seededThisOrg > 0 {
			sum.OrgsSeeded++
		}
	}
	return sum, nil
}

// seedExistingOrgsAllMembers runs the one-time all-members migration, guarded by
// the govern_meta marker so it fires exactly once (idempotent even if a prior
// run crashed before writing the marker — duplicate nominations are skipped).
func (s *Server) seedExistingOrgsAllMembers(ctx context.Context) error {
	if _, done, err := s.reg.GovernMetaGet(ctx, governCutoverMarkerKey); err != nil {
		return err
	} else if done {
		return nil
	}
	sum, err := s.seedGovernAdmins(ctx, false, "system:govern-cutover")
	if err != nil {
		return err
	}
	slog.Info("govern cutover: existing-orgs all-members migration complete",
		"orgs_considered", sum.OrgsConsidered,
		"orgs_seeded", sum.OrgsSeeded,
		"admins_nominated", sum.AdminsNominated,
		"already_admins", sum.AlreadyAdmins,
		"orgs_narrowed_skipped", sum.SkippedNarrowed,
		"unresolved_members", len(sum.Unresolved))
	if len(sum.Unresolved) > 0 {
		slog.Warn("govern cutover: roster members with no matching identity were NOT seeded as admins (operator proxy stewardship is the recovery path, ADR 1102)",
			"members", sum.Unresolved)
	}
	return s.reg.GovernMetaSet(ctx, governCutoverMarkerKey, time.Now().UTC().Format(time.RFC3339))
}

// reconcileFirstMember seeds the first resolvable member of any fail-closed
// zero-admin org — the going-forward new-org path. A no-op once every org has
// an admin.
func (s *Server) reconcileFirstMember(ctx context.Context) error {
	sum, err := s.seedGovernAdmins(ctx, true, "system:govern-first-member")
	if err != nil {
		return err
	}
	if sum.AdminsNominated > 0 {
		slog.Info("govern reconcile: seeded first member for new zero-admin org(s)",
			"orgs_seeded", sum.OrgsSeeded,
			"admins_nominated", sum.AdminsNominated,
			"unresolved_members", len(sum.Unresolved))
	}
	return nil
}

// RunGovernReconcile drives the ADR 1102 cutover. While the goatnet is
// bootstrap-open it does nothing (the kick-start posture stands). Once
// fail-closed it runs the one-time all-members migration then the first-member
// reconcile at startup, and re-runs the reconcile every interval so a newly
// onboarded org is seeded without a restart. interval<=0 runs the startup pass
// only. Non-fatal: seeding errors are logged, never crash the portal.
func (s *Server) RunGovernReconcile(ctx context.Context, interval time.Duration) {
	if s.cfg.GovernBootstrapOpen {
		slog.Info("govern cutover inactive: goatnet is bootstrap-open (ADR 1102 kick-start); membership confers governance until an org narrows itself")
		return
	}
	if s.humans == nil || s.cfg.RosterPath == "" {
		slog.Warn("govern cutover: goatnet is fail-closed but kratos/roster is unconfigured; governance seeding disabled (orgs without nominated admins are locked out until seeded)")
		return
	}
	run := func() {
		if err := s.seedExistingOrgsAllMembers(ctx); err != nil {
			slog.Error("govern cutover: all-members migration failed; skipping reconcile this pass", "err", err)
			return
		}
		if err := s.reconcileFirstMember(ctx); err != nil {
			slog.Error("govern cutover: first-member reconcile failed", "err", err)
		}
	}
	run()
	if interval <= 0 {
		return
	}
	go func() {
		t := time.NewTicker(interval)
		defer t.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-t.C:
				run()
			}
		}
	}()
}

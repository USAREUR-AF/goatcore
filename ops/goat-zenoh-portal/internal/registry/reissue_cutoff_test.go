package registry_test

// GHSA-rr3p review r9 F2: the reissue cut-off cases (the reviewer's probes,
// asserting the fixed behaviour). A change of hands during the assignment,
// however it shows, ends every allowance; the rightful holder re-stamped as
// themselves keeps theirs.

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

type reissueCase struct {
	r       *registry.Registry
	slot    string
	tAssign int64
	grant   string
}

// legacyReissueSlot builds a pre-upgrade gen-0 slot assigned at tAssign, with A's
// allowances (machine quota 3, a producer data grant) set after the
// assignment, and makes it a reissue candidate through a vendor-rotate event.
func legacyReissueSlot(t *testing.T, salt string, onboardIdentity string) reissueCase {
	t.Helper()
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, salt, 4); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "person-a")
	tAssign := time.Now().Unix() - 3600
	var id any
	if onboardIdentity != "" {
		id = onboardIdentity
	}
	x := func(q string, a ...any) {
		t.Helper()
		if _, err := r.DB().ExecContext(ctx, q, a...); err != nil {
			t.Fatal(err)
		}
	}
	x(`UPDATE slots SET generation = 0, assigned_at = $1, identity_id = $2 WHERE slot_id = $3`, tAssign, id, p.SlotID)
	x(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot-onboard','person-a',$2)`,
		tAssign, fmt.Sprintf(`{"slot_idx":0,"anchor":%q,"identity_id":%q}`, p.SlotID, onboardIdentity))
	if err := r.SetMachineQuota(ctx, p.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	g := &registry.Grant{GranteeKind: "org", GranteeID: "a-reader", Producer: p.SlotID, GrantedBy: "person-a", Justification: "t"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}
	x(`UPDATE machine_grants SET granted_at = $1, updated_at = $1 WHERE custodian = $2`, tAssign+100, p.SlotID)
	x(`UPDATE grants SET created_at = $1 WHERE id = $2`, tAssign+100, g.ID)
	// A rotate during the assignment makes it a candidate (two bare leaves).
	x(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-rotate',$2,'{}')`, tAssign+200, p.SlotID)
	return reissueCase{r: r, slot: p.SlotID, tAssign: tAssign, grant: g.ID}
}

func (s reissueCase) exec(t *testing.T, q string, a ...any) {
	t.Helper()
	if _, err := s.r.DB().ExecContext(context.Background(), q, a...); err != nil {
		t.Fatal(err)
	}
}

func (s reissueCase) restamp(t *testing.T, at int64, id string) {
	t.Helper()
	s.exec(t, `INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot.identity.set',$2,$3)`,
		at, s.slot, fmt.Sprintf(`{"identity_id":%q}`, id))
	var v any
	if id != "" {
		v = id
	}
	s.exec(t, `UPDATE slots SET identity_id = $1 WHERE slot_id = $2`, v, s.slot)
}

// reissue runs the dry run and the apply and reports what B is left with.
func (s reissueCase) reissue(t *testing.T, label string) (restamped bool, kept bool) {
	t.Helper()
	ctx := context.Background()
	cands, err := s.r.ReissueCandidates(ctx)
	if err != nil {
		t.Logf("%s: ReissueCandidates error: %v", label, err)
		return false, false
	}
	for _, c := range cands {
		if c.SlotID != s.slot {
			continue
		}
		e, _ := s.r.ReissueWouldEnd(ctx, c)
		ok, err := s.r.ApplyReissue(ctx, "rr3p-reissue", c)
		q, _ := s.r.MachineQuotaFor(ctx, s.slot)
		g, _ := s.r.GetGrant(ctx, s.grant)
		kept = q.Quota != 0 || g.Status == registry.GrantActive
		t.Logf("%s: candidate cutoff=assigned_at%+d restamped=%v dry-run would-end=%+v applied=%v err=%v -> current holder keeps quota=%d, A's producer grant=%s",
			label, c.Cutoff-s.tAssign, c.Restamped, e, ok, err, q.Quota, g.Status)
		return c.Restamped, kept
	}
	t.Logf("%s: not a candidate", label)
	return false, true
}

func mustHandOver(t *testing.T, s reissueCase, label string) {
	t.Helper()
	if r, kept := s.reissue(t, label); !r || kept {
		t.Fatalf("%s: restamped=%v kept=%v; want the change of hands detected and the earlier holder's allowances ended", label, r, kept)
	}
}

func mustKeep(t *testing.T, s reissueCase, label string) {
	t.Helper()
	if r, kept := s.reissue(t, label); r || !kept {
		t.Fatalf("%s: restamped=%v kept=%v; want the rightful holder to keep theirs", label, r, kept)
	}
}

func TestReissueCutoff_cases(t *testing.T) {
	t.Run("R1 unlinked first holder, then stamped B", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r1", "")
		s.restamp(t, s.tAssign+600, idB)
		mustHandOver(t, s, "R1")
	})
	t.Run("R2 re-stamp row dated before assigned_at (clock step)", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r2", idA)
		s.restamp(t, s.tAssign-5, idB)
		mustHandOver(t, s, "R2")
	})
	t.Run("R3 re-stamp in the assignment's second", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r3", idA)
		s.restamp(t, s.tAssign, idB)
		mustHandOver(t, s, "R3")
	})
	t.Run("R4 control", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r4", idA)
		s.restamp(t, s.tAssign+600, idB)
		mustHandOver(t, s, "R4")
	})
	t.Run("R5 same-second rows A, B, A", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r5", idA)
		s.restamp(t, s.tAssign+600, idB)
		s.restamp(t, s.tAssign+600, idA)
		mustHandOver(t, s, "R5")
	})
	t.Run("R6 withdrawn and restored to the same person", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r6", idA)
		s.restamp(t, s.tAssign+600, "")
		s.restamp(t, s.tAssign+700, idA)
		mustKeep(t, s, "R6")
	})
	t.Run("R6b same person, case and space", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r6b", idA)
		s.restamp(t, s.tAssign+600, " "+strings.ToUpper(idA))
		s.restamp(t, s.tAssign+700, idA)
		mustKeep(t, s, "R6b")
	})
	t.Run("R8 onboard row lost", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-r8", idA)
		s.restamp(t, s.tAssign+600, idB)
		s.exec(t, `DELETE FROM audit_events WHERE action = 'slot-onboard' AND details LIKE '%' || $1 || '%'`, s.slot)
		mustHandOver(t, s, "R8")
	})
	t.Run("native onboard: linkage stamped within the grace is not a hand-over", func(t *testing.T) {
		s := legacyReissueSlot(t, "rc-native", "")
		s.restamp(t, s.tAssign+60, idA)
		s.exec(t, `UPDATE machine_grants SET granted_at = $1 WHERE custodian = $2`, s.tAssign+10, s.slot)
		s.exec(t, `UPDATE grants SET created_at = $1 WHERE id = $2`, s.tAssign+10, s.grant)
		mustKeep(t, s, "native")
	})
}

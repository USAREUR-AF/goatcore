package registry_test

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// GHSA-rr3p review r7 F1: grants naming the anchor as GRANTEE (a co-publish
// write grant, a principal view grant) end with the holder on every path:
// hand-over, offboard, and the assignment of a slot freed without cleanup.
func TestAnchorGranteeGrants_endOnHandOverRevokeAndReassignment(t *testing.T) {
	ctx := context.Background()
	for _, path := range []string{"hand-over", "offboard", "reassignment"} {
		t.Run(path, func(t *testing.T) {
			r := open(t)
			if _, err := r.InitializeSlotPool(ctx, "grantee-"+path, 2); err != nil {
				t.Fatal(err)
			}
			p, _ := r.AllocateSlot(ctx, "person-a")
			if err := r.StampSlotIdentity(ctx, p.SlotID, idA); err != nil {
				t.Fatal(err)
			}
			recordLeaf(t, r, p.SlotID)
			w := &registry.Grant{GranteeKind: "principal", GranteeID: p.SlotID, Producer: "multipub/poolx/topicy", Action: "write", GrantedBy: "admin", Justification: "co-publish"}
			v := &registry.Grant{GranteeKind: "principal", GranteeID: p.SlotID, Producer: strings.Repeat("c", 32), GrantedBy: "c", Justification: "view"}
			for _, g := range []*registry.Grant{w, v} {
				if err := r.CreateGrant(ctx, g); err != nil {
					t.Fatal(err)
				}
			}
			switch path {
			case "hand-over":
				if rl, err := r.RelinkSlotIdentity(ctx, p.SlotID, idB); err != nil || !rl.HandedOver {
					t.Fatalf("relink: %+v %v", rl, err)
				}
			case "offboard":
				if ok, err := r.RevokeAssignedSlot(ctx, p.SlotID); err != nil || !ok {
					t.Fatalf("revoke: %v %v", ok, err)
				}
			case "reassignment":
				if err := r.RevokeSlot(ctx, p.SlotID); err != nil { // the cleanup-free path
					t.Fatal(err)
				}
				saved := registry.SlotLeafClock
				registry.SlotLeafClock = func() time.Time { return time.Now().Add(400 * 24 * time.Hour) }
				defer func() { registry.SlotLeafClock = saved }()
				if s, err := r.AllocateSlot(ctx, "person-c"); err != nil || s.SlotID != p.SlotID {
					t.Fatalf("reallocate: %+v %v", s, err)
				}
			}
			for _, g := range []*registry.Grant{w, v} {
				if got, _ := r.GetGrant(ctx, g.ID); got == nil || got.Status != registry.GrantRevoked {
					t.Fatalf("%s: grant to the anchor (%s) still %+v", path, g.Producer, got)
				}
			}
		})
	}
}

// GHSA-rr3p review r7 F2: reissuing a slot reused before the fix ends what the
// earlier holder left on the anchor, and keeps what was set since the current
// assignment. The dry-run count says what it ends.
func TestReissue_endsAllowancesInheritedBeforeTheAssignment(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "reissue-inherit", 4); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "person-a")
	if err := r.SetMachineQuota(ctx, p.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	m := enrollSlot(t, r, ctx, "a-m1", p.SlotID, "SENSOR", "manual", "unit-a")
	old := &registry.Grant{GranteeKind: "org", GranteeID: "a-reader", Producer: p.SlotID, GrantedBy: "a", Justification: "t"}
	if err := r.CreateGrant(ctx, old); err != nil {
		t.Fatal(err)
	}
	db := r.DB()
	exec := func(q string, a ...any) {
		t.Helper()
		if _, err := db.ExecContext(ctx, q, a...); err != nil {
			t.Fatal(err)
		}
	}
	// Pre-fix: A onboarded, offboarded with the old revoke (nothing ended),
	// B assigned at generation 0 a minute later.
	t0 := time.Now().Add(-time.Hour).Unix()
	tB := t0 + 60
	exec(`UPDATE grants SET created_at = $1 WHERE id = $2`, t0, old.ID)
	exec(`UPDATE machine_grants SET granted_at = $1, updated_at = $1 WHERE custodian = $2`, t0, p.SlotID)
	exec(`UPDATE slots SET enrolled_at = $1 WHERE slot_id = $2`, t0, m)
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot-onboard','person-a',$2)`,
		t0, fmt.Sprintf(`{"anchor":%q}`, p.SlotID))
	exec(`UPDATE slots SET generation = 0 WHERE slot_id IN ($1, $2)`, p.SlotID, m)
	exec(`DELETE FROM slot_leaves WHERE slot_id = $1`, p.SlotID)
	exec(`UPDATE slots SET assigned_to = 'person-b', assigned_at = $1, identity_id = $2, revoked_at = NULL WHERE slot_id = $3`, tB, idB, p.SlotID)
	// B's own grant, made after B's assignment, must survive.
	mine := &registry.Grant{GranteeKind: "org", GranteeID: "b-reader", Producer: p.SlotID, GrantedBy: "b", Justification: "t"}
	if err := r.CreateGrant(ctx, mine); err != nil {
		t.Fatal(err)
	}

	cands, err := r.ReissueCandidates(ctx)
	if err != nil || len(cands) != 1 {
		t.Fatalf("candidates = %+v, %v", cands, err)
	}
	e, err := r.ReissueWouldEnd(ctx, cands[0])
	if err != nil || e.MachineGrants != 1 || e.Custody != 1 || e.DataGrants != 1 {
		t.Fatalf("dry-run count = %+v, %v; want 1 machine grant, 1 custody edge, 1 data grant", e, err)
	}
	if ok, err := r.ApplyReissue(ctx, "test", cands[0]); err != nil || !ok {
		t.Fatalf("ApplyReissue = %v, %v", ok, err)
	}
	if q, _ := r.MachineQuotaFor(ctx, p.SlotID); q.Quota != 0 {
		t.Fatalf("inherited machine grant survived: %+v", q)
	}
	var custody *string
	_ = db.GetContext(ctx, &custody, `SELECT enrolled_by FROM slots WHERE slot_id = $1`, m)
	if custody != nil {
		t.Fatal("custody of the earlier holder's machine survived")
	}
	if g, _ := r.GetGrant(ctx, old.ID); g.Status != registry.GrantRevoked {
		t.Fatalf("earlier holder's grant still %s", g.Status)
	}
	if g, _ := r.GetGrant(ctx, mine.ID); g.Status != registry.GrantActive {
		t.Fatalf("the current holder's own grant was ended: %s", g.Status)
	}
}

// GHSA-rr3p review r7 F3: the same person re-stamped in another case is not
// a hand-over, and the stored spelling is lower-case.
func TestRelink_sameIdentityOtherCase_isNotAHandOver(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "relink-case", 1); err != nil {
		t.Fatal(err)
	}
	s, _ := r.AllocateSlot(ctx, "holder")
	upper := strings.ToUpper(idA)
	if err := r.StampSlotIdentity(ctx, s.SlotID, upper); err != nil {
		t.Fatal(err)
	}
	recordLeaf(t, r, s.SlotID)
	if rl, err := r.RelinkSlotIdentity(ctx, s.SlotID, idA); err != nil || rl.HandedOver {
		t.Fatalf("re-stamp in lower case = (%+v, %v), want no hand-over", rl, err)
	}
	sl, err := r.SlotForIdentity(ctx, upper)
	if err != nil || sl.IdentityID == nil || *sl.IdentityID != idA {
		t.Fatalf("lookup by the upper-case spelling = %+v, %v; want the slot, stored lower-case", sl, err)
	}
}

// GHSA-rr3p review r8 F1: the co-publish subjects are per (anchor, topic):
// each is live only while a grant for exactly that topic is active and in
// effect, every topic ever granted stays declared, and a hand-over ends them.
func TestSlotBindings_copubIsPerTopic(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "copub-bind", 2); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "writer")
	if err := r.StampSlotIdentity(ctx, p.SlotID, idA); err != nil {
		t.Fatal(err)
	}
	const t1, t2 = "multipub/poolx/topic1", "multipub/poolx/topic2"
	live := func() map[string]bool {
		t.Helper()
		bs, err := r.SlotBindings(ctx)
		if err != nil {
			t.Fatal(err)
		}
		out := map[string]bool{}
		for _, b := range bs {
			if b.Anchor == p.SlotID {
				for _, c := range b.Copub {
					if c.Key != registry.CopubKey(c.Base) {
						t.Fatalf("key %s is not CopubKey(%s)", c.Key, c.Base)
					}
					out[c.Base] = c.Live
				}
			}
		}
		return out
	}
	if l := live(); len(l) != 0 {
		t.Fatalf("copub subjects without any grant: %v", l)
	}
	g1 := &registry.Grant{GranteeKind: "principal", GranteeID: p.SlotID, Producer: t1, Action: "write", GrantedBy: "admin", Justification: "t"}
	g2 := &registry.Grant{GranteeKind: "principal", GranteeID: p.SlotID, Producer: t2, Action: "write", GrantedBy: "admin", Justification: "t"}
	for _, g := range []*registry.Grant{g1, g2} {
		if err := r.CreateGrant(ctx, g); err != nil {
			t.Fatal(err)
		}
	}
	if l := live(); !l[t1] || !l[t2] {
		t.Fatalf("two active grants: %v", l)
	}
	// Two grants, one revoked: only that topic closes.
	if err := r.RevokeGrant(ctx, g2.ID, "admin"); err != nil {
		t.Fatal(err)
	}
	if l := live(); !l[t1] || l[t2] || len(l) != 2 {
		t.Fatalf("after revoking T2: %v, want T1 live, T2 declared but not live", l)
	}
	// Expiry: a grant past effective_to closes with status still active.
	saved := registry.SlotLeafClock
	registry.SlotLeafClock = func() time.Time { return time.Now().Add(48 * time.Hour) }
	if _, err := r.DB().ExecContext(ctx, `UPDATE grants SET effective_to = $1 WHERE id = $2`, time.Now().Add(24*time.Hour).Unix(), g1.ID); err != nil {
		t.Fatal(err)
	}
	if l := live(); l[t1] {
		t.Fatalf("an expired T1 grant keeps its subject live: %v", l)
	}
	registry.SlotLeafClock = saved
	// The reviewer's T1/T2 scenario: A held T1; a hand-over to B ends it;
	// B is then granted T2 only. T1 stays declared and closed.
	if _, err := r.DB().ExecContext(ctx, `UPDATE grants SET effective_to = NULL WHERE id = $1`, g1.ID); err != nil {
		t.Fatal(err)
	}
	if rl, err := r.RelinkSlotIdentity(ctx, p.SlotID, idB); err != nil || !rl.HandedOver {
		t.Fatalf("relink: %+v %v", rl, err)
	}
	g3 := &registry.Grant{GranteeKind: "principal", GranteeID: p.SlotID, Producer: t2, Action: "write", GrantedBy: "admin", Justification: "B"}
	if err := r.CreateGrant(ctx, g3); err != nil {
		t.Fatal(err)
	}
	if l := live(); l[t1] || !l[t2] {
		t.Fatalf("after hand-over and B's T2 grant: %v, want T1 closed, T2 live", l)
	}
}

// GHSA-rr3p review r8 F2: a slot moved from A to B by a pre-upgrade identity
// re-stamp (assigned_at never moved) is a candidate, and since nothing records
// whose allowances are whose after the re-stamp, all of them end.
func TestReissue_preUpgradeRestampEndsEverything(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "reissue-restamp", 4); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "unit-x")
	db := r.DB()
	exec := func(q string, a ...any) {
		t.Helper()
		if _, err := db.ExecContext(ctx, q, a...); err != nil {
			t.Fatal(err)
		}
	}
	tAssign := time.Now().Add(-time.Hour).Unix()
	exec(`UPDATE slots SET generation = 0, assigned_at = $1, identity_id = $2 WHERE slot_id = $3`, tAssign, idA, p.SlotID)
	if err := r.SetMachineQuota(ctx, p.SlotID, 3, "op"); err != nil {
		t.Fatal(err)
	}
	g := &registry.Grant{GranteeKind: "org", GranteeID: "a-reader", Producer: p.SlotID, GrantedBy: "a", Justification: "t"}
	if err := r.CreateGrant(ctx, g); err != nil {
		t.Fatal(err)
	}
	// The pre-patch identity PUT: only identity_id moved, and its audit row.
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot-onboard','unit-x',$2)`,
		tAssign, fmt.Sprintf(`{"anchor":%q,"identity_id":%q}`, p.SlotID, idA))
	exec(`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','slot.identity.set',$2,$3)`,
		tAssign+600, p.SlotID, fmt.Sprintf(`{"identity_id":%q}`, idB))
	exec(`UPDATE slots SET identity_id = $1 WHERE slot_id = $2`, idB, p.SlotID)
	cands, err := r.ReissueCandidates(ctx)
	if err != nil || len(cands) != 1 || !cands[0].Restamped {
		t.Fatalf("candidates = %+v, %v; want the re-stamped slot", cands, err)
	}
	if e, _ := r.ReissueWouldEnd(ctx, cands[0]); e.MachineGrants != 1 || e.DataGrants != 1 {
		t.Fatalf("dry-run count = %+v", e)
	}
	if ok, err := r.ApplyReissue(ctx, "t", cands[0]); err != nil || !ok {
		t.Fatalf("apply = %v %v", ok, err)
	}
	if q, _ := r.MachineQuotaFor(ctx, p.SlotID); q.Quota != 0 {
		t.Fatalf("machine grant survived: %+v", q)
	}
	if gg, _ := r.GetGrant(ctx, g.ID); gg.Status != registry.GrantRevoked {
		t.Fatalf("grant survived: %s", gg.Status)
	}
}

// GHSA-rr3p review r8 F3: a machine-grant row created before the cut-off is
// ended even if a later partial update (quota, network_admin) moved
// updated_at past it, because its affiliation and site are the earlier
// holder's.
func TestReissue_machineGrantCreatedBeforeCutoffEndsDespiteUpdate(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "reissue-partial", 4); err != nil {
		t.Fatal(err)
	}
	p, _ := r.AllocateSlot(ctx, "person-a")
	if err := r.SetMachineGrant(ctx, p.SlotID, 2, "unit-a", "site-a", "op"); err != nil {
		t.Fatal(err)
	}
	db := r.DB()
	t0 := time.Now().Add(-time.Hour).Unix()
	tB := t0 + 60
	for _, q := range []struct {
		sql  string
		args []any
	}{
		{`UPDATE machine_grants SET granted_at = $1, updated_at = $1 WHERE custodian = $2`, []any{t0, p.SlotID}},
		{`INSERT INTO audit_events (occurred_at, actor, action, target, details) VALUES ($1,'op','vendor-delete',$2,'{}')`, []any{t0 + 1, p.SlotID}},
		{`UPDATE slots SET generation = 0, assigned_to = 'person-b', assigned_at = $1, identity_id = $2 WHERE slot_id = $3`, []any{tB, idB, p.SlotID}},
	} {
		if _, err := db.ExecContext(ctx, q.sql, q.args...); err != nil {
			t.Fatal(err)
		}
	}
	if err := r.SetMachineQuota(ctx, p.SlotID, 5, "op"); err != nil { // B's quota raised after B's assignment
		t.Fatal(err)
	}
	cands, err := r.ReissueCandidates(ctx)
	if err != nil || len(cands) != 1 {
		t.Fatalf("candidates = %+v, %v", cands, err)
	}
	if ok, err := r.ApplyReissue(ctx, "t", cands[0]); err != nil || !ok {
		t.Fatalf("apply = %v %v", ok, err)
	}
	if q, _ := r.MachineQuotaFor(ctx, p.SlotID); q.Quota != 0 {
		t.Fatalf("the earlier holder's machine-grant row survived a partial update: %+v", q)
	}
}

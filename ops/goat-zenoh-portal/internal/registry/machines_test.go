package registry_test

import (
	"context"
	"errors"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// enrollHelper allocates a fresh slot and records a machine enrollment on it,
// failing the test on any error. Returns the slot id.
func enrollSlot(t *testing.T, r *registry.Registry, ctx context.Context, systemID, custodian, nodeClass, autonomy, aff string) string {
	t.Helper()
	s, err := r.AllocateSlot(ctx, systemID)
	if err != nil {
		t.Fatalf("AllocateSlot(%s): %v", systemID, err)
	}
	if err := r.RecordMachineEnrollment(ctx, s.SlotID, custodian, nodeClass, autonomy, aff, nil); err != nil {
		t.Fatalf("RecordMachineEnrollment(%s): %v", systemID, err)
	}
	return s.SlotID
}

func TestGrammarValidators(t *testing.T) {
	for _, nc := range []string{"SENSOR", "EFFECTOR", "PROCESSOR", "PLATFORM", "MGMT-HOST", "ZENOH", "SNITCH", "SERVICES", "NETWORK"} {
		if !registry.ValidNodeClass(nc) {
			t.Errorf("ValidNodeClass(%q) = false, want true", nc)
		}
	}
	for _, nc := range []string{"sensor", "ROUTER", "", "person"} {
		if registry.ValidNodeClass(nc) {
			t.Errorf("ValidNodeClass(%q) = true, want false", nc)
		}
	}
	// The Stage-3 operator-gate predicate: infra classes true, function false.
	for _, nc := range []string{"MGMT-HOST", "ZENOH", "SNITCH", "SERVICES", "NETWORK"} {
		if !registry.IsInfraNodeClass(nc) {
			t.Errorf("IsInfraNodeClass(%q) = false, want true (operator-gated)", nc)
		}
	}
	for _, nc := range []string{"SENSOR", "EFFECTOR", "PROCESSOR", "PLATFORM"} {
		if registry.IsInfraNodeClass(nc) {
			t.Errorf("IsInfraNodeClass(%q) = true, want false (self-service)", nc)
		}
	}
	for _, a := range []string{"manual", "remote", "supervised", "autonomous"} {
		if !registry.ValidAutonomy(a) {
			t.Errorf("ValidAutonomy(%q) = false, want true", a)
		}
	}
	for _, a := range []string{"Manual", "auto", ""} {
		if registry.ValidAutonomy(a) {
			t.Errorf("ValidAutonomy(%q) = true, want false", a)
		}
	}
}

func TestMachineQuota_NoGrantIsZeroFailClosed(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	q, err := r.MachineQuotaFor(ctx, "nobody")
	if err != nil {
		t.Fatalf("MachineQuotaFor: %v", err)
	}
	if q.Quota != 0 || q.Used != 0 || q.Remaining != 0 {
		t.Fatalf("no-grant custodian = %+v, want all zero (fail-closed)", q)
	}
}

func TestSetMachineQuota_UpsertPreservesGrantedAt(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if err := r.SetMachineQuota(ctx, "alice", 3, "operator:root"); err != nil {
		t.Fatalf("SetMachineQuota: %v", err)
	}
	q, _ := r.MachineQuotaFor(ctx, "alice")
	if q.Quota != 3 || q.Remaining != 3 {
		t.Fatalf("after set = %+v, want quota 3 remaining 3", q)
	}
	// re-set to a higher ceiling
	if err := r.SetMachineQuota(ctx, "alice", 5, "operator:root"); err != nil {
		t.Fatalf("SetMachineQuota re-set: %v", err)
	}
	q, _ = r.MachineQuotaFor(ctx, "alice")
	if q.Quota != 5 {
		t.Fatalf("after re-set = %+v, want quota 5", q)
	}
	if err := r.SetMachineQuota(ctx, "alice", -1, "x"); err == nil {
		t.Fatal("SetMachineQuota(-1) = nil, want non-negative error")
	}
}

func TestRecordMachineEnrollment_HappyPathAndQuotaConsumption(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "galaxy-test", 8); err != nil {
		t.Fatalf("InitializeSlotPool: %v", err)
	}
	if err := r.SetMachineQuota(ctx, "alice", 2, "operator:root"); err != nil {
		t.Fatalf("SetMachineQuota: %v", err)
	}

	enrollSlot(t, r, ctx, "alice-sensor-1", "alice", "SENSOR", "supervised", "usareur-af")
	q, _ := r.MachineQuotaFor(ctx, "alice")
	if q.Used != 1 || q.Remaining != 1 {
		t.Fatalf("after 1 enroll = %+v, want used 1 remaining 1", q)
	}

	enrollSlot(t, r, ctx, "alice-effector-1", "alice", "EFFECTOR", "remote", "usareur-af")
	q, _ = r.MachineQuotaFor(ctx, "alice")
	if q.Used != 2 || q.Remaining != 0 {
		t.Fatalf("after 2 enroll = %+v, want used 2 remaining 0", q)
	}

	// The custody surface lists both, newest first, stamped correctly.
	fleet, err := r.MachinesForCustodian(ctx, "alice")
	if err != nil {
		t.Fatalf("MachinesForCustodian: %v", err)
	}
	if len(fleet) != 2 {
		t.Fatalf("fleet = %d, want 2", len(fleet))
	}
	m := fleet[0]
	if m.EnrolledBy == nil || *m.EnrolledBy != "alice" || m.NodeClass == nil || m.Affiliation == nil || *m.Affiliation != "usareur-af" {
		t.Fatalf("stamp not persisted: %+v", m)
	}
}

func TestRecordMachineEnrollment_QuotaCeilingFailClosed(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 8)
	if err := r.SetMachineQuota(ctx, "bob", 1, "op"); err != nil {
		t.Fatalf("SetMachineQuota: %v", err)
	}
	enrollSlot(t, r, ctx, "bob-1", "bob", "SENSOR", "manual", "unit-x")

	// second slot allocates fine, but the enroll must refuse — over ceiling.
	s2, err := r.AllocateSlot(ctx, "bob-2")
	if err != nil {
		t.Fatalf("AllocateSlot: %v", err)
	}
	err = r.RecordMachineEnrollment(ctx, s2.SlotID, "bob", "SENSOR", "manual", "unit-x", nil)
	if !errors.Is(err, registry.ErrQuotaExhausted) {
		t.Fatalf("over-ceiling enroll = %v, want ErrQuotaExhausted", err)
	}

	// a custodian with NO grant is quota 0 → immediate refuse (fail-closed).
	s3, _ := r.AllocateSlot(ctx, "carol-1")
	if err := r.RecordMachineEnrollment(ctx, s3.SlotID, "carol", "SENSOR", "manual", "unit-y", nil); !errors.Is(err, registry.ErrQuotaExhausted) {
		t.Fatalf("no-grant enroll = %v, want ErrQuotaExhausted", err)
	}
}

func TestRecordMachineEnrollment_RevocationReturnsQuota(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 8)
	_ = r.SetMachineQuota(ctx, "dana", 1, "op")
	slotID := enrollSlot(t, r, ctx, "dana-1", "dana", "PROCESSOR", "autonomous", "unit-z")

	q, _ := r.MachineQuotaFor(ctx, "dana")
	if q.Remaining != 0 {
		t.Fatalf("pre-revoke remaining = %d, want 0", q.Remaining)
	}
	if err := r.RevokeSlot(ctx, slotID); err != nil {
		t.Fatalf("RevokeSlot: %v", err)
	}
	// revoked slot no longer counts → quota returns by construction.
	q, _ = r.MachineQuotaFor(ctx, "dana")
	if q.Used != 0 || q.Remaining != 1 {
		t.Fatalf("post-revoke = %+v, want used 0 remaining 1 (quota returned)", q)
	}
}

func TestRecordMachineEnrollment_RestampIsIdempotent(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 8)
	_ = r.SetMachineQuota(ctx, "erin", 1, "op")
	slotID := enrollSlot(t, r, ctx, "erin-1", "erin", "SENSOR", "manual", "unit-e")

	// re-stamp the SAME slot for the SAME custodian (e.g. autonomy correction);
	// must not consume a second quota unit.
	if err := r.RecordMachineEnrollment(ctx, slotID, "erin", "SENSOR", "supervised", "unit-e", nil); err != nil {
		t.Fatalf("re-stamp: %v", err)
	}
	q, _ := r.MachineQuotaFor(ctx, "erin")
	if q.Used != 1 || q.Remaining != 0 {
		t.Fatalf("after re-stamp = %+v, want used 1 (no double-count)", q)
	}
}

func TestAllocateSlot_ClearsStaleStampSoQuotaIsNotMisattributed(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 2) // tiny pool forces slot reuse
	_ = r.SetMachineQuota(ctx, "frank", 2, "op")
	slotID := enrollSlot(t, r, ctx, "frank-1", "frank", "SENSOR", "manual", "unit-f")

	// revoke → slot returns to the free pool; then a DIFFERENT tenant re-claims it
	// for a non-machine purpose. Frank's stale stamp must not follow the slot.
	if err := r.RevokeSlot(ctx, slotID); err != nil {
		t.Fatalf("RevokeSlot: %v", err)
	}
	reclaimed, err := r.AllocateSlot(ctx, "someones-dev-slot")
	if err != nil {
		t.Fatalf("AllocateSlot re-claim: %v", err)
	}
	if reclaimed.SlotID != slotID {
		t.Fatalf("expected slot reuse of %s, got %s", slotID, reclaimed.SlotID)
	}
	if reclaimed.EnrolledBy != nil {
		t.Fatalf("re-claimed slot still stamped enrolled_by=%v — stale custody leaked", *reclaimed.EnrolledBy)
	}
	// Frank's live count reflects the revoke, unaffected by the re-claim.
	q, _ := r.MachineQuotaFor(ctx, "frank")
	if q.Used != 0 {
		t.Fatalf("frank used = %d after revoke+reclaim, want 0", q.Used)
	}
}

func TestRecordMachineEnrollment_Validation(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 4)
	_ = r.SetMachineQuota(ctx, "gina", 4, "op")
	s, _ := r.AllocateSlot(ctx, "gina-1")

	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "gina", "ROUTER", "manual", "u", nil); !errors.Is(err, registry.ErrInvalidNodeClass) {
		t.Fatalf("bad node-class = %v, want ErrInvalidNodeClass", err)
	}
	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "gina", "SENSOR", "auto", "u", nil); !errors.Is(err, registry.ErrInvalidAutonomy) {
		t.Fatalf("bad autonomy = %v, want ErrInvalidAutonomy", err)
	}
	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "gina", "SENSOR", "manual", "", nil); err == nil {
		t.Fatal("empty affiliation = nil, want error (affiliation inherited, required)")
	}
	// unknown / unassigned slot is not enrollable
	if err := r.RecordMachineEnrollment(ctx, "deadbeef", "gina", "SENSOR", "manual", "u", nil); !errors.Is(err, registry.ErrSlotNotEnrollable) {
		t.Fatalf("unknown slot = %v, want ErrSlotNotEnrollable", err)
	}
}

func TestAllEnrolledMachines_FleetView(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 8)
	_ = r.SetMachineGrant(ctx, "alice", 3, "usareur-af", "", "op")
	_ = r.SetMachineGrant(ctx, "bob", 3, "unit-b", "", "op")

	enrollSlot(t, r, ctx, "alice-1", "alice", "SENSOR", "manual", "usareur-af")
	enrollSlot(t, r, ctx, "bob-1", "bob", "EFFECTOR", "remote", "unit-b")
	revokeMe := enrollSlot(t, r, ctx, "bob-2", "bob", "PROCESSOR", "autonomous", "unit-b")
	// a non-machine self-slot (no custody stamp) must NOT appear.
	if _, err := r.AllocateSlot(ctx, "someones-dev-slot"); err != nil {
		t.Fatalf("AllocateSlot: %v", err)
	}
	// a revoked machine must NOT appear.
	if err := r.RevokeSlot(ctx, revokeMe); err != nil {
		t.Fatalf("RevokeSlot: %v", err)
	}

	all, err := r.AllEnrolledMachines(ctx)
	if err != nil {
		t.Fatalf("AllEnrolledMachines: %v", err)
	}
	if len(all) != 2 {
		t.Fatalf("fleet = %d, want 2 (alice-1 + bob-1; dev-slot + revoked excluded)", len(all))
	}
	for _, m := range all {
		if m.EnrolledBy == nil || m.NodeClass == nil {
			t.Fatalf("fleet row missing custody stamp: %+v", m)
		}
	}
}

func TestRecordMachineEnrollment_PlatformWorkloads(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	_, _ = r.InitializeSlotPool(ctx, "galaxy-test", 8)
	_ = r.SetMachineGrant(ctx, "uas-op", 4, "usareur-af", "", "op")

	// A PLATFORM may declare a function-workload composition.
	s, _ := r.AllocateSlot(ctx, "uas-1")
	if err := r.RecordMachineEnrollment(ctx, s.SlotID, "uas-op", "PLATFORM", "supervised", "usareur-af",
		[]string{"SENSOR", "EFFECTOR", "PROCESSOR"}); err != nil {
		t.Fatalf("platform enroll: %v", err)
	}
	fleet, _ := r.MachinesForCustodian(ctx, "uas-op")
	if len(fleet) != 1 || fleet[0].Workloads == nil || *fleet[0].Workloads != "SENSOR,EFFECTOR,PROCESSOR" {
		t.Fatalf("workloads not stored: %+v", fleet)
	}

	// Workloads on a non-PLATFORM node-class are rejected.
	s2, _ := r.AllocateSlot(ctx, "sensor-1")
	if err := r.RecordMachineEnrollment(ctx, s2.SlotID, "uas-op", "SENSOR", "manual", "usareur-af",
		[]string{"PROCESSOR"}); !errors.Is(err, registry.ErrInvalidWorkload) {
		t.Fatalf("workloads on non-platform = %v, want ErrInvalidWorkload", err)
	}
	// A non-function-class workload is rejected.
	s3, _ := r.AllocateSlot(ctx, "uas-2")
	if err := r.RecordMachineEnrollment(ctx, s3.SlotID, "uas-op", "PLATFORM", "manual", "usareur-af",
		[]string{"MGMT-HOST"}); !errors.Is(err, registry.ErrInvalidWorkload) {
		t.Fatalf("infra workload = %v, want ErrInvalidWorkload", err)
	}
	// PLATFORM itself is 1 quota unit regardless of workload count.
	q, _ := r.MachineQuotaFor(ctx, "uas-op")
	if q.Used != 1 {
		t.Fatalf("platform used = %d, want 1 (workloads are internal composition, not quota draws)", q.Used)
	}
}

func TestValidWorkloads_And_IsFunctionWorkloadClass(t *testing.T) {
	for _, w := range []string{"SENSOR", "EFFECTOR", "PROCESSOR", "EFFECTOR-CUE", "KID"} {
		if !registry.IsFunctionWorkloadClass(w) {
			t.Errorf("IsFunctionWorkloadClass(%q) = false, want true", w)
		}
	}
	for _, w := range []string{"PLATFORM", "MGMT-HOST", "POD", "MOON", "sensor", ""} {
		if registry.IsFunctionWorkloadClass(w) {
			t.Errorf("IsFunctionWorkloadClass(%q) = true, want false", w)
		}
	}
	if registry.ValidWorkloads("PLATFORM", nil) != nil {
		t.Error("empty workloads must be valid")
	}
	if registry.ValidWorkloads("PLATFORM", []string{"SENSOR"}) != nil {
		t.Error("valid platform workload rejected")
	}
	if registry.ValidWorkloads("PLATFORM", []string{"KID"}) != nil {
		t.Error("PLATFORM/KID compatibility rejected")
	}
	for _, workloads := range [][]string{{"POD"}, {"MOON"}, {"MOON", "POD"}} {
		if err := registry.ValidWorkloads("PLATFORM", workloads); !errors.Is(err, registry.ErrInvalidWorkload) {
			t.Errorf("PLATFORM/%v = %v, want ErrInvalidWorkload", workloads, err)
		}
	}
	if registry.ValidWorkloads("SENSOR", []string{"PROCESSOR"}) == nil {
		t.Error("workloads on non-platform must be rejected")
	}
}

// TestGearClassPredicates covers the ADR 1116 mint-time vocabulary: which
// classes exist, which may be minted, and which are named-but-refused.
func TestGearClassPredicates(t *testing.T) {
	gear := []string{
		"SENSOR-FEED", "SENSOR-TASKING", "SENSOR-TELEMETRY", "SENSOR-ARBITER",
		"EFFECTOR-CUE", "EFFECTOR-TRACK", "EFFECTOR-TELEMETRY", "EFFECTOR-COMMAND",
		"EFFECTOR-ARBITER", "KID", "EXTERNAL-KID", "EXTERNAL-INTERMEDIARY", "FUSION",
	}
	for _, nc := range gear {
		if !registry.ValidNodeClass(nc) || !registry.IsGearNodeClass(nc) {
			t.Errorf("%q should be a valid gear class", nc)
		}
		// Gear is custodian self-service, never operator-gated infrastructure.
		if registry.IsInfraNodeClass(nc) {
			t.Errorf("IsInfraNodeClass(%q) = true, want false", nc)
		}
	}

	// Reserved: valid to READ (historical rows carry them) but never mintable.
	for _, nc := range []string{"SENSOR", "EFFECTOR"} {
		if !registry.ValidNodeClass(nc) {
			t.Errorf("%q must stay readable — machines were enrolled as one before the gear classes existed", nc)
		}
		if !registry.IsReservedNodeClass(nc) || registry.IsEnrollableNodeClass(nc) {
			t.Errorf("%q must be reserved and unenrollable (ADR 1116 §3)", nc)
		}
	}

	// Intake-only: a destination in the ACL and a source in nothing, so never a
	// machine's primary class.
	for _, nc := range []string{"SENSOR-TASKING", "EFFECTOR-COMMAND"} {
		if !registry.IsIntakeOnlyNodeClass(nc) || registry.IsEnrollableNodeClass(nc) {
			t.Errorf("%q must be intake-only and unenrollable as a primary class", nc)
		}
	}

	for _, nc := range []string{"SENSOR-FEED", "KID", "FUSION", "NETWORK", "MGMT-HOST"} {
		if !registry.IsEnrollableNodeClass(nc) {
			t.Errorf("IsEnrollableNodeClass(%q) = false, want true", nc)
		}
	}
}

func TestMeshGroupsForMachine(t *testing.T) {
	// A gear class IS its group.
	got, err := registry.MeshGroupsForMachine("KID", nil)
	if err != nil || len(got) != 1 || got[0] != "KID" {
		t.Fatalf("KID → %v, %v; want [KID]", got, err)
	}
	// Substrate classes fold into INFRA, which is what the taxonomy defines
	// INFRA as: mgmt hosts, mesh-dns, relay-health, site-hosted services.
	for _, nc := range []string{"MGMT-HOST", "ZENOH", "SERVICES"} {
		got, err := registry.MeshGroupsForMachine(nc, nil)
		if err != nil || len(got) != 1 || got[0] != "INFRA" {
			t.Errorf("%s → %v, %v; want [INFRA]", nc, got, err)
		}
	}
	// A PLATFORM has no reach of its own: it is the UNION of the functions it
	// performs. ADR 1116's worked example is an effector navigator box.
	got, err = registry.MeshGroupsForMachine("PLATFORM", []string{"EFFECTOR-CUE", "EFFECTOR-ARBITER", "EFFECTOR-CUE"})
	if err != nil {
		t.Fatalf("PLATFORM union: %v", err)
	}
	if len(got) != 2 || got[0] != "EFFECTOR-ARBITER" || got[1] != "EFFECTOR-CUE" {
		t.Fatalf("PLATFORM union = %v, want deduped [EFFECTOR-ARBITER EFFECTOR-CUE]", got)
	}
	// KID remains a legitimate composable function workload.
	got, err = registry.MeshGroupsForMachine("PLATFORM", []string{"KID"})
	if err != nil || len(got) != 1 || got[0] != "KID" {
		t.Fatalf("PLATFORM/KID = %v, %v; want [KID]", got, err)
	}
	// POD and MOON remain valid standalone classes but are trust-boundary
	// appliances, not PLATFORM workloads. The derivation layer rejects them so
	// direct and re-mint callers cannot bypass API validation.
	for _, workloads := range [][]string{{"POD"}, {"MOON"}, {"MOON", "POD"}} {
		if _, err := registry.MeshGroupsForMachine("PLATFORM", workloads); !errors.Is(err, registry.ErrInvalidWorkload) {
			t.Errorf("PLATFORM/%v = %v, want ErrInvalidWorkload", workloads, err)
		}
	}
	// A composition of nothing has no derivable reach — refuse, never default.
	if _, err := registry.MeshGroupsForMachine("PLATFORM", nil); !errors.Is(err, registry.ErrNoMeshMapping) {
		t.Fatalf("empty PLATFORM = %v, want ErrNoMeshMapping", err)
	}
	// Legacy whole-system class with no group: refuse rather than invent one.
	if _, err := registry.MeshGroupsForMachine("PROCESSOR", nil); !errors.Is(err, registry.ErrNoMeshMapping) {
		t.Fatalf("PROCESSOR = %v, want ErrNoMeshMapping", err)
	}
	// The mapping never hands back the catch-all the ADR retires.
	for _, nc := range []string{"SENSOR-FEED", "KID", "FUSION", "NETWORK", "SNITCH", "MGMT-HOST"} {
		gs, err := registry.MeshGroupsForMachine(nc, nil)
		if err != nil {
			t.Fatalf("%s: %v", nc, err)
		}
		for _, g := range gs {
			if g == "OPSYSTEM" {
				t.Fatalf("%s derives OPSYSTEM — the class the ADR retires", nc)
			}
		}
	}
}

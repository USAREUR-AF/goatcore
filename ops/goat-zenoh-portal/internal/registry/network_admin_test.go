package registry

// ADR 1113 — the NETWORK node-class and the delegated network-admin entitlement.
//
// The property under test is the fail-closed one: an entitlement that is
// site-scoped, never granted by a migration, never granted without a site, and
// never granted implicitly by a routine quota change.

import (
	"context"
	"testing"
)

func TestNetworkNodeClassIsInfraAndDelegable(t *testing.T) {
	if !ValidNodeClass("NETWORK") {
		t.Fatal("ValidNodeClass(NETWORK) = false, want true")
	}
	if !IsInfraNodeClass("NETWORK") {
		t.Error("IsInfraNodeClass(NETWORK) = false — NETWORK shapes the substrate and must stay infra-classified")
	}
	if !IsDelegableInfraNodeClass("NETWORK") {
		t.Error("IsDelegableInfraNodeClass(NETWORK) = false, want true")
	}
	// The other infra classes must NOT become delegable by accident. This is the
	// assertion that fails loudly if someone widens the set without an ADR.
	for _, nc := range []string{"MGMT-HOST", "ZENOH", "SNITCH", "SERVICES"} {
		if IsDelegableInfraNodeClass(nc) {
			t.Errorf("IsDelegableInfraNodeClass(%q) = true — only NETWORK and EDGE-SERVER are delegable (ADR 1113, ADR 1216)", nc)
		}
	}
	// Function classes are not infra and not delegable-infra.
	for _, nc := range []string{"SENSOR", "EFFECTOR", "PROCESSOR", "PLATFORM"} {
		if IsDelegableInfraNodeClass(nc) {
			t.Errorf("IsDelegableInfraNodeClass(%q) = true, want false (function classes use the quota path)", nc)
		}
	}
}

func TestSetNetworkAdminFailsClosed(t *testing.T) {
	reg := testRegistry(t)
	ctx := context.Background()

	// No grant at all → refuse. The entitlement extends a grant; it never mints one.
	if err := reg.SetNetworkAdmin(ctx, "riley", true, "operator"); err == nil {
		t.Error("SetNetworkAdmin on a custodian with no grant succeeded; want refusal")
	}

	// A grant with no site → refuse, because the entitlement is site-scoped and an
	// unscoped network admin is exactly the unbounded authority this avoids.
	if err := reg.SetMachineGrant(ctx, "riley", 5, "usareur-af", "", "operator"); err != nil {
		t.Fatalf("seed grant without site: %v", err)
	}
	if err := reg.SetNetworkAdmin(ctx, "riley", true, "operator"); err == nil {
		t.Error("SetNetworkAdmin on a site-less grant succeeded; want refusal")
	}
	g, err := reg.MachineGrantFor(ctx, "riley")
	if err != nil {
		t.Fatalf("read grant: %v", err)
	}
	if g.NetworkAdmin {
		t.Fatal("network_admin set despite the refusal — the write must not land")
	}

	// With a site, the grant takes.
	if err := reg.SetMachineGrant(ctx, "riley", 5, "usareur-af", "harbor", "operator"); err != nil {
		t.Fatalf("seed grant with site: %v", err)
	}
	if err := reg.SetNetworkAdmin(ctx, "riley", true, "operator"); err != nil {
		t.Fatalf("SetNetworkAdmin on a sited grant: %v", err)
	}
	g, err = reg.MachineGrantFor(ctx, "riley")
	if err != nil {
		t.Fatalf("read grant: %v", err)
	}
	if !g.NetworkAdmin {
		t.Error("network_admin not persisted")
	}
	if g.Site != "harbor" {
		t.Errorf("site = %q, want harbor (the entitlement's bound)", g.Site)
	}

	// Revocable.
	if err := reg.SetNetworkAdmin(ctx, "riley", false, "operator"); err != nil {
		t.Fatalf("clear entitlement: %v", err)
	}
	if g, _ = reg.MachineGrantFor(ctx, "riley"); g.NetworkAdmin {
		t.Error("entitlement survived an explicit clear")
	}
}

// A routine quota adjustment must not silently grant or revoke the entitlement —
// they are separate authorities on purpose (SetMachineGrant vs SetNetworkAdmin).
func TestQuotaChangeDoesNotDisturbNetworkAdmin(t *testing.T) {
	reg := testRegistry(t)
	ctx := context.Background()
	if err := reg.SetMachineGrant(ctx, "riley", 5, "usareur-af", "harbor", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}
	if err := reg.SetNetworkAdmin(ctx, "riley", true, "operator"); err != nil {
		t.Fatalf("grant entitlement: %v", err)
	}
	if err := reg.SetMachineGrant(ctx, "riley", 12, "usareur-af", "harbor", "operator"); err != nil {
		t.Fatalf("adjust quota: %v", err)
	}
	g, err := reg.MachineGrantFor(ctx, "riley")
	if err != nil {
		t.Fatalf("read grant: %v", err)
	}
	if !g.NetworkAdmin {
		t.Error("a quota adjustment cleared the network-admin entitlement")
	}
	if g.Quota != 12 {
		t.Errorf("quota = %d, want 12", g.Quota)
	}
}

// A pre-existing grant (the migration case) must read as NOT entitled.
func TestLegacyGrantIsNotNetworkAdmin(t *testing.T) {
	reg := testRegistry(t)
	ctx := context.Background()
	if err := reg.SetMachineGrant(ctx, "legacy", 3, "usareur-af", "galaxy-prod", "operator"); err != nil {
		t.Fatalf("seed grant: %v", err)
	}
	g, err := reg.MachineGrantFor(ctx, "legacy")
	if err != nil {
		t.Fatalf("read grant: %v", err)
	}
	if g.NetworkAdmin {
		t.Error("a grant created without the entitlement reads as entitled — migration default is not fail-closed")
	}
}

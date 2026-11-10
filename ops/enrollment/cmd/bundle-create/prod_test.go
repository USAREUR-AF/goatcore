//go:build !nopiv

package main

import (
	"os"
	"testing"

	"github.com/go-piv/piv-go/v2/piv"
)

func TestParsePIVSlot(t *testing.T) {
	for _, tc := range []struct {
		in      string
		want    piv.Slot
		wantErr bool
	}{
		{"", piv.SlotSignature, false},
		{"9c", piv.SlotSignature, false},
		{"9C", piv.SlotSignature, false},
		{"0x9c", piv.SlotSignature, false},
		{"9a", piv.SlotAuthentication, false},
		{"9d", piv.SlotKeyManagement, false},
		{"9e", piv.SlotCardAuthentication, false},
		{"9f", piv.Slot{}, true},
		{"bogus", piv.Slot{}, true},
	} {
		t.Run(tc.in, func(t *testing.T) {
			got, err := parsePIVSlot(tc.in)
			if tc.wantErr {
				if err == nil {
					t.Errorf("expected error for %q", tc.in)
				}
				return
			}
			if err != nil {
				t.Errorf("unexpected error: %v", err)
			}
			if got != tc.want {
				t.Errorf("got %+v; want %+v", got, tc.want)
			}
		})
	}
}

func TestResolvePIN_FlagWins(t *testing.T) {
	t.Setenv("GOAT_CA_PIN", "from-env")
	pin, err := resolvePIN("from-flag")
	if err != nil {
		t.Fatalf("resolvePIN: %v", err)
	}
	if pin != "from-flag" {
		t.Errorf("got %q; want from-flag", pin)
	}
}

func TestResolvePIN_EnvFallback(t *testing.T) {
	t.Setenv("GOAT_CA_PIN", "from-env")
	pin, err := resolvePIN("")
	if err != nil {
		t.Fatalf("resolvePIN: %v", err)
	}
	if pin != "from-env" {
		t.Errorf("got %q; want from-env", pin)
	}
}

// TestResolvePIN_NoTTYNoEnvNoFlag is the failure mode under CI / scripts:
// no flag, no env, no terminal. resolvePIN must surface a clear error
// rather than blocking on stdin.
func TestResolvePIN_NoTTYNoEnvNoFlag(t *testing.T) {
	// Unset GOAT_CA_PIN if a parallel test set it.
	_ = os.Unsetenv("GOAT_CA_PIN")
	// In `go test` with no -tty redirection, stdin is not a terminal,
	// so this exercises the non-interactive failure path.
	_, err := resolvePIN("")
	if err == nil {
		t.Fatal("expected error when no PIN source available")
	}
}

func TestRun_RejectsUnknownMode(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		Mode:    "bogus",
		OutPath: "/tmp/bundle.cbor",
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	if err := run(args); err == nil {
		t.Fatal("expected error for unknown mode")
	}
}

func TestRun_DefaultsToDevMode(t *testing.T) {
	// Mode unset → run() should default to dev (the existing happy
	// path covers the rest; this just locks the default in).
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		Mode:    "", // explicit
		OutPath: t.TempDir() + "/bundle.cbor",
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("dev mode (defaulted) should succeed: %v", err)
	}
}

func TestRun_ProdModeRequiresNoCardWhenNoneAvailable(t *testing.T) {
	// In CI / on dev laptops without a YubiKey inserted, --mode prod
	// must fail with a recognizable error rather than hanging or
	// silently producing a broken bundle.
	relaysPath := fixtureRelays(t)
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "prod-test",
		Mode: "prod", PIVSlotName: "9c", PINFlag: "123456",
		OutPath: t.TempDir() + "/bundle.cbor",
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error in prod mode without a YubiKey present")
	}
	// Don't strictly assert the error string — piv.Cards() returns
	// platform-specific errors. We just need a non-nil error so
	// scripts can branch.
}

func TestValidateArgs_DevModeRequiresAgeFlags(t *testing.T) {
	a := &runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: "/tmp/r", ACLGroupsRaw: "G",
		CAID:    "dev-test",
		Mode:    "dev",
		OutPath: "/tmp/o",
		TTLDays: 90, ActivationDays: 14,
	}
	if err := validateArgs(a); err == nil {
		t.Fatal("expected error for missing --ca-key in dev mode")
	}
}

func TestValidateArgs_ProdModeDoesNotRequireAgeFlags(t *testing.T) {
	a := &runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: "/tmp/r", ACLGroupsRaw: "G",
		CAID: "prod-test", Mode: "prod",
		OutPath: "/tmp/o",
		TTLDays: 90, ActivationDays: 14,
	}
	if err := validateArgs(a); err != nil {
		t.Errorf("prod mode should validate without --ca-key/--age-identity: %v", err)
	}
}
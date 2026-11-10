// ADR 1148 §3 removed the member release-lane derivation, and with it four
// tests that pinned it. The rules those tests also exercised are still covered
// here directly: membership dating by TestIsMember_MembershipRangeEndsAtMemberUntil,
// member markers by TestIsMember_MemberMarkersAreMembersByDefinition, and the
// registry parity check by TestIsMember_ParityWithCanonicalRegistry.
package access

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

const testFrameworks = `
default_framework: nato
frameworks:
  nato:
    members:
      - { nation: FRA, member_since: 1949-08-24 }
      - { nation: USA, member_since: 1949-08-24 }
      - { nation: FIN, member_since: 2023-04-04 }
`

func writeFile(t *testing.T, name, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
		t.Fatalf("write %s: %v", name, err)
	}
	return p
}

func TestIsMember_NationAndDate(t *testing.T) {
	cf, err := LoadCoalitionFrameworks(writeFile(t, "cf.yaml", testFrameworks))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	now := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	if !cf.isMember("nato", "FRA", now) {
		t.Error("FRA should be a NATO member")
	}
	if !cf.isMember("nato", "fra", now) { // case-insensitive
		t.Error("nation match must be case-insensitive")
	}
	if cf.isMember("nato", "CHN", now) {
		t.Error("CHN is not a NATO member (partner)")
	}
	// FIN joined 2023 — member now, not in 2020.
	if !cf.isMember("nato", "FIN", now) {
		t.Error("FIN is a member as of 2026")
	}
	if cf.isMember("nato", "FIN", time.Date(2020, 1, 1, 0, 0, 0, 0, time.UTC)) {
		t.Error("FIN was not yet a member in 2020 (member_since respected)")
	}
	if cf.isMember("unknown-fw", "FRA", now) {
		t.Error("unknown framework ⇒ not a member")
	}
}

func TestLoadCoalitionFrameworks_MissingFailsClosed(t *testing.T) {
	if _, err := LoadCoalitionFrameworks(filepath.Join(t.TempDir(), "nope.yaml")); err == nil {
		t.Fatal("missing frameworks file must fail closed")
	}
}

// --- member_markers + the membership range (added with the rule fix) ---------
//
// Both of these were missing from the first version of isMember, and both decide
// who lands in the derived member release lane — a silent over- or under-share
// rather than an error, which is why they are pinned here.

const testFrameworksFull = `
default_framework: nato
frameworks:
  nato:
    member_markers: [NATO]
    members:
      - { nation: USA, member_since: 1949-08-24 }
      - { nation: FIN, member_since: 2023-04-04 }
      - { nation: XXA, member_since: 2000-01-01, member_until: 2026-01-01 }
`

func TestIsMember_MemberMarkersAreMembersByDefinition(t *testing.T) {
	cf, err := LoadCoalitionFrameworks(writeFile(t, "cf.yaml", testFrameworksFull))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	now := time.Date(2026, 8, 17, 0, 0, 0, 0, time.UTC)

	// An alliance body carries no nation row; it is a member via member_markers.
	// Before the fix this returned false, so a NATO-affiliated org was excluded
	// from the very lane it defines.
	if !cf.isMember("nato", "NATO", now) {
		t.Error("a member_markers value must resolve as a member")
	}
	if !cf.isMember("nato", "nato", now) {
		t.Error("marker match must fold case like nation match does")
	}
	// A marker is per-framework, not global.
	if cf.isMember("nato", "EU", now) {
		t.Error("EU is not a declared nato marker and must not resolve as a member")
	}
}

func TestIsMember_MembershipRangeEndsAtMemberUntil(t *testing.T) {
	cf, err := LoadCoalitionFrameworks(writeFile(t, "cf.yaml", testFrameworksFull))
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	day := func(s string) time.Time {
		d, err := time.Parse(dateFormat, s)
		if err != nil {
			t.Fatalf("bad date %q: %v", s, err)
		}
		return d
	}

	if !cf.isMember("nato", "XXA", day("2025-12-31")) {
		t.Error("still a member the day before member_until")
	}
	// member_until is EXCLUSIVE — a partner on that date, not the day after.
	if cf.isMember("nato", "XXA", day("2026-01-01")) {
		t.Error("member_until is exclusive: must read as partner ON that date")
	}
	if cf.isMember("nato", "XXA", day("2026-08-17")) {
		t.Error("a nation that left must not keep reading as a member")
	}
	// The range does not leak onto other nations.
	if !cf.isMember("nato", "USA", day("2026-08-17")) {
		t.Error("USA membership is unaffected by another nation's withdrawal")
	}
}

// Parity against the REAL registry, not a fixture: this rule and the console's
// copy both claim to re-express generate.py:resolve_coalition, so both are
// pinned to the shipped file. A membership edit that nobody reviewed fails here.
func TestIsMember_ParityWithCanonicalRegistry(t *testing.T) {
	path := filepath.Join("..", "..", "..", "identity", "coalition-frameworks.yaml")
	if _, err := os.Stat(path); err != nil {
		t.Skipf("canonical registry not reachable: %v", err)
	}
	cf, err := LoadCoalitionFrameworks(path)
	if err != nil {
		t.Fatalf("load canonical registry: %v", err)
	}
	now := time.Date(2026, 8, 17, 0, 0, 0, 0, time.UTC)

	cases := []struct {
		nation string
		want   bool
	}{
		{"USA", true},
		{"DEU", true},
		{"FIN", true},
		{"SWE", true},
		{"NATO", true}, // the marker, from the shipped registry
		{"UKR", false},
		{"ZZZ", false},
		{"", false},
	}
	for _, c := range cases {
		if got := cf.isMember(cf.DefaultFramework, c.nation, now); got != c.want {
			t.Errorf("isMember(%s, %q) = %v, want %v", cf.DefaultFramework, c.nation, got, c.want)
		}
	}
	// Accession is effective-dated in the real file too.
	if cf.isMember(cf.DefaultFramework, "FIN", time.Date(2023, 4, 3, 0, 0, 0, 0, time.UTC)) {
		t.Error("FIN must read as partner the day before its accession")
	}
}

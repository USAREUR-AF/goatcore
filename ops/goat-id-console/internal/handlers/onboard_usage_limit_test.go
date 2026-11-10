package handlers

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The console collects "how many machines?" as one number in some places, but it
// feeds two fields that are not the same kind of thing:
//
//   - `netbird.usage_limit` is a CAPACITY — how many of their OWN devices a
//     person may enroll with the key they were issued.
//   - `machine_quota` is an AUTHORIZATION (ADR 1096) — a custodian holding it may
//     enroll that many OTHER machines.
//
// Defaulting the first is a convenience; defaulting the second would be a silent
// privilege grant, so absence there must stay absence.
//
// Regression: assembleManifest gated both on
// `MachineQuota > 0`, so an intake with no number produced a manifest carrying
// neither field, and operator-onboard.yml's `| default(1)` turned that silence
// into a single-use key. The first enrolment consumed it and every later attempt
// — a second machine, a reinstall, native Windows after the client had been
// installed under WSL — was refused with "couldn't add peer: setup key is
// invalid". Four people on the zagros goatnet hit it in one morning; nine more were
// holding unfired single-use keys. Nothing displayed the limit, because
// identity-card.js renders the machine row only when the value is truthy.
func TestAssembleManifestSetupKeyUsageLimit(t *testing.T) {
	hints, err := loadIntakeHints()
	if err != nil {
		t.Fatalf("loadIntakeHints: %v", err)
	}

	base := func(quota, usage int) effectiveInputs {
		return effectiveInputs{
			Tier:          "browse",
			Affiliation:   "partner",
			AffiliationID: "usage-org",
			Handle:        "test-handle",
			Email:         "test@example.test",
			MachineQuota:  quota,
			UsageLimit:    usage,
		}
	}

	t.Run("no numbers collected: reusable key, no custodian grant", func(t *testing.T) {
		m, err := assembleManifest(hints, base(0, 0))
		if err != nil {
			t.Fatalf("assembleManifest: %v", err)
		}

		v := getPath(m, "netbird.usage_limit")
		got, ok := toInt(v)
		if !ok {
			t.Fatalf("netbird.usage_limit: want a number, got %T (%v) — an omitted "+
				"number must leave the intake-form default standing, not fall "+
				"through to the play's last-resort 1", v, v)
		}
		if got < 2 {
			t.Fatalf("netbird.usage_limit = %d; want >= 2. A limit of 1 is a "+
				"single-use key: the first enrolment consumes it and every "+
				"later attempt fails with \"setup key is invalid\". That is a "+
				"thing to choose deliberately, never a thing to get by default.", got)
		}

		// The authorization half must stay absent.
		if q := getPath(m, "machine_quota"); q != nil {
			t.Fatalf("machine_quota = %v, want unset. It authorizes a custodian to "+
				"enroll OTHER machines (ADR 1096); no number collected means no "+
				"grant, never a default grant.", q)
		}
	})

	t.Run("an explicit usage limit overrides the default", func(t *testing.T) {
		m, err := assembleManifest(hints, base(0, 3))
		if err != nil {
			t.Fatalf("assembleManifest: %v", err)
		}
		if got, _ := toInt(getPath(m, "netbird.usage_limit")); got != 3 {
			t.Fatalf("netbird.usage_limit = %d, want 3 — an explicit number must "+
				"override the default", got)
		}
	})

	t.Run("the two fields are written from separate inputs", func(t *testing.T) {
		// A capacity on its own must never imply an authorization. These are two
		// struct fields precisely so no caller can grant custody by supplying a
		// device count.
		m, err := assembleManifest(hints, base(0, 9))
		if err != nil {
			t.Fatalf("assembleManifest: %v", err)
		}
		if q := getPath(m, "machine_quota"); q != nil {
			t.Fatalf("machine_quota = %v, want unset — a device allowance is not a "+
				"custodian grant (ADR 1096)", q)
		}

		m, err = assembleManifest(hints, base(4, 4))
		if err != nil {
			t.Fatalf("assembleManifest: %v", err)
		}
		if got, _ := toInt(getPath(m, "machine_quota")); got != 4 {
			t.Fatalf("machine_quota = %d, want 4 — a sponsor's number does grant "+
				"custodian authority", got)
		}
	})

	t.Run("a deliberate single-use key is still expressible", func(t *testing.T) {
		m, err := assembleManifest(hints, base(0, 1))
		if err != nil {
			t.Fatalf("assembleManifest: %v", err)
		}
		if got, _ := toInt(getPath(m, "netbird.usage_limit")); got != 1 {
			t.Fatalf("netbird.usage_limit = %d, want 1 — asking for a one-shot key "+
				"explicitly must still work; only the SILENT path changed", got)
		}
	})
}

// A candidate answering "how many devices will you bring?" on the public form
// must set their own key's capacity and NOTHING else. The invite schema states
// the rule — "capability rides the server-held invite, never the public form"
// (ADR 1062 §2) — and previously the submit path broke it: when a sponsor
// left machine quota blank, the candidate's number fell through to BOTH fields,
// so typing a number self-granted custody over that many machines, uncapped.
// Three proposals in the zagros store carry a 20 no sponsor typed.
func TestCandidateNumberSetsCapacityNotAuthorization(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()

	manifestOf := func(pid string) map[string]any {
		t.Helper()
		p, err := s.Onboard.GetProposal(context.Background(), pid)
		if err != nil {
			t.Fatalf("GetProposal: %v", err)
		}
		var m map[string]any
		if err := json.Unmarshal([]byte(p.ManifestJSON), &m); err != nil {
			t.Fatalf("decode manifest: %v", err)
		}
		return m
	}

	t.Run("sponsor left it blank: candidate sets capacity only", func(t *testing.T) {
		tok := mintTokenWithQuota(t, s, 0)
		pid := submit(t, h, tok, map[string]any{
			"name": "Pat Liaison", "email": "person-76@example.org", "organization": "Usage Org", "machine_quota": 7,
		})
		m := manifestOf(pid)
		if got, _ := toInt(getPath(m, "netbird.usage_limit")); got != 7 {
			t.Fatalf("netbird.usage_limit = %d, want 7 — the candidate's own device "+
				"count is theirs to state", got)
		}
		if q := getPath(m, "machine_quota"); q != nil {
			t.Fatalf("machine_quota = %v, want unset — a candidate must not be able "+
				"to grant themselves custody of other machines from the public form "+
				"(ADR 1062 §2)", q)
		}
	})

	t.Run("candidate blank: the intake-form default stands", func(t *testing.T) {
		tok := mintTokenWithQuota(t, s, 0)
		pid := submit(t, h, tok, map[string]any{"name": "Pat Two", "email": "person-78@example.org", "organization": "Usage Org"})
		m := manifestOf(pid)
		got, _ := toInt(getPath(m, "netbird.usage_limit"))
		if got < 2 {
			t.Fatalf("netbird.usage_limit = %d, want the intake-form default (>= 2) — "+
				"blank on both sides must not mint a single-use key", got)
		}
	})

	t.Run("an absurd candidate number is clamped, not refused", func(t *testing.T) {
		tok := mintTokenWithQuota(t, s, 0)
		pid := submit(t, h, tok, map[string]any{
			"name": "Pat Three", "email": "person-85@example.org", "organization": "Usage Org", "machine_quota": 100000,
		})
		m := manifestOf(pid)
		if got, _ := toInt(getPath(m, "netbird.usage_limit")); got != maxCandidateUsageLimit {
			t.Fatalf("netbird.usage_limit = %d, want %d — the intake form documents "+
				"1-100, and a bound that lives only in help text is not a bound",
				got, maxCandidateUsageLimit)
		}
	})

	t.Run("sponsor set it: the sponsor's number wins both fields", func(t *testing.T) {
		tok := mintTokenWithQuota(t, s, 4)
		pid := submit(t, h, tok, map[string]any{
			"name": "Pat Four", "email": "person-92@example.org", "organization": "Usage Org", "machine_quota": 9,
		})
		m := manifestOf(pid)
		if got, _ := toInt(getPath(m, "machine_quota")); got != 4 {
			t.Fatalf("machine_quota = %d, want 4 — a sponsor-locked number stands "+
				"and the candidate's counter-offer is a petition, not a value", got)
		}
		if got, _ := toInt(getPath(m, "netbird.usage_limit")); got != 4 {
			t.Fatalf("netbird.usage_limit = %d, want 4", got)
		}
	})
}

// mintTokenWithQuota mints a store-level invite carrying a prebound machine
// quota (0 = the sponsor left the field blank) and returns its one-time token.
func mintTokenWithQuota(t *testing.T, s *Server, quota int) string {
	t.Helper()
	pb := onboardstore.Prebound{Tier: "developer", Affiliation: "partner", MachineQuota: quota}
	pbJSON, _ := json.Marshal(pb)
	inv := &onboardstore.Invite{Sponsor: "dev@bypass.local", Env: "zagros", PreboundJSON: string(pbJSON), ExpiresAt: 0}
	tok, err := s.Onboard.MintInvite(context.Background(), inv)
	if err != nil {
		t.Fatalf("mint via store: %v", err)
	}
	return tok
}

// toInt reads a JSON-ish number out of an assembled manifest. The defaults block
// arrives via encoding/json as float64; an explicit input arrives as int.
func toInt(v any) (int, bool) {
	switch n := v.(type) {
	case int:
		return n, true
	case int64:
		return int(n), true
	case float64:
		return int(n), true
	default:
		return 0, false
	}
}

package api

import (
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
)

// recordSources is a goatnet whose people enrolled through the console and the
// portal, so the declared roster knows none of them: the shape that left every
// machine `unmatched` while the registry held its custodian and affiliation.
// All identities are synthetic.
func recordSources() access.Sources {
	return access.Sources{
		Vendors: []access.VendorIn{
			// A person's slot, its anchor and the Kratos identity stamped at onboard.
			{Prefix: "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a", Alias: "rivera-j", IdentityID: "ory-rivera"},
			// A person whose slot predates the identity_id column.
			{Prefix: "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b", Alias: "okafor-n"},
		},
		Identities: []access.IdentityIn{{
			ID: "ory-rivera", Email: "rivera.j@example.org", Name: "J Rivera",
			Vendor: "anvil", AffID: "org-anvil", AffKind: "vendor", AffNation: "XAA",
		}},
		Machines: []access.MachineIn{
			// Custodian recorded as the slot anchor (most live rows) …
			{SystemHandle: "anvil-pod-7", Custodian: "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a", NodeClass: "FUSION", SlotID: "s-7", Affiliation: "1st Anvil Bn"},
			// … or as the handle (some rows).
			{SystemHandle: "anvil-relay", Custodian: "rivera-j", NodeClass: "NETWORK", SlotID: "s-8", Affiliation: "1st Anvil Bn"},
			// A custodian that holds no live slot.
			{SystemHandle: "orphan-box", Custodian: "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c", SlotID: "s-9"},
		},
	}
}

func owner(t *testing.T, resp map[string]any) map[string]any {
	t.Helper()
	o, ok := resp["owner"].(map[string]any)
	if !ok {
		t.Fatalf("no owner block: %v", resp)
	}
	return o
}

func TestAMachineResolvesFromItsEnrolledCustodianWithNoRoster(t *testing.T) {
	for _, peer := range []string{"machine-anvil-pod-7-rack2", "machine-anvil-relay"} {
		resp := resolvePeerName(recordSources(), peer)
		o := owner(t, resp)
		if o["source"] != "record" || o["via"] != "machine-custody" {
			t.Fatalf("%s: owner = %v, want source record via machine-custody", peer, o)
		}
		if o["matched_by"] != "peer-name" {
			t.Fatalf("%s: matched_by = %v, want peer-name: the first hop is the name, and the answer must say so", peer, o["matched_by"])
		}
		if resp["handle"] != "rivera-j" || resp["resolved"] != true {
			t.Fatalf("%s: handle/resolved = %v/%v, want the custodian rivera-j, resolved", peer, resp["handle"], resp["resolved"])
		}
		if resp["org"] != "org-anvil" || resp["nation"] != "XAA" {
			t.Fatalf("%s: org/nation = %v/%v, want the custodian's recorded org-anvil / XAA", peer, resp["org"], resp["nation"])
		}
		if o["machine_unit"] != "1st Anvil Bn" {
			t.Fatalf("%s: machine_unit = %v, want the slot's recorded affiliation", peer, o["machine_unit"])
		}
		if _, ok := resp["machine"]; !ok {
			t.Fatalf("%s: no machine block, so a consumer cannot label the handle a custodian", peer)
		}
	}
}

func TestAPersonsDeviceResolvesFromTheSlotIdentityWithNoRoster(t *testing.T) {
	resp := resolvePeerName(recordSources(), "peer-rivera-j-zagros-2026-02-09-laptop")
	o := owner(t, resp)
	if o["source"] != "record" || o["via"] != "slot-identity" {
		t.Fatalf("owner = %v, want source record via slot-identity", o)
	}
	p := o["person"].(map[string]any)
	if p["handle"] != "rivera-j" || p["identity_id"] != "ory-rivera" {
		t.Fatalf("person = %v", p)
	}
	org := o["org"].(map[string]any)
	if org["state"] != "recorded" || org["id"] != "org-anvil" || org["nation"] != "XAA" {
		t.Fatalf("org = %v", org)
	}
}

func TestNationalityIsTheIdentitysOwnAndNeverTheOrgNation(t *testing.T) {
	src := recordSources()
	resp := resolvePeerName(src, "machine-anvil-pod-7")
	nat := owner(t, resp)["nationality"].(map[string]any)
	if nat["state"] != "not_stamped" || nat["value"] != "" || resp["nationality"] != "" {
		t.Fatalf("nationality = %v / %v, want not_stamped and empty: the org's nation XAA must not fill it", nat, resp["nationality"])
	}
	if !strings.Contains(nat["reason"].(string), "not yet stamped") {
		t.Fatalf("reason = %v, want it to say where the fact is held", nat["reason"])
	}
	src.Identities[0].Nationality = "XCC"
	resp = resolvePeerName(src, "machine-anvil-pod-7")
	nat = owner(t, resp)["nationality"].(map[string]any)
	if nat["state"] != "recorded" || resp["nationality"] != "XCC" || resp["nation"] != "XAA" {
		t.Fatalf("nationality %v, nation %v: want XCC recorded beside the org's XAA", resp["nationality"], resp["nation"])
	}
}

func TestAMissingLinkIsNamedNotLeftBlank(t *testing.T) {
	cases := []struct{ peer, want string }{
		{"machine-orphan-box", "holds no live slot"},
		{"peer-okafor-n-zagros-2026-01-05-desk", "carries no recorded identity"},
		{"edge-gw-unlisted", "no enrolled machine and no slot handle"},
	}
	for _, c := range cases {
		resp := resolvePeerName(recordSources(), c.peer)
		o := owner(t, resp)
		if o["source"] != "none" {
			t.Fatalf("%s: owner = %v, want none", c.peer, o)
		}
		if !strings.Contains(o["reason"].(string), c.want) || !strings.Contains(resp["reason"].(string), c.want) {
			t.Fatalf("%s: reason = %v, want it to say %q", c.peer, o["reason"], c.want)
		}
	}
}

func TestTheRosterAnswersOnlyWhenTheRecordCannotAndSaysSo(t *testing.T) {
	src := recordSources()
	src.Identities = append(src.Identities, access.IdentityIn{ID: "ory-okafor", Email: "okafor.n@example.org"})
	src.Roster = []access.RosterEntry{{
		Handle: "okafor-n", Type: "person", Org: "org-birch", Status: "active",
		EmailHash: access.EmailHash("okafor.n@example.org"),
	}}
	resp := resolvePeerName(src, "peer-okafor-n-zagros-2026-01-05-desk")
	o := owner(t, resp)
	if o["source"] != "roster" || resp["org"] != "org-birch" {
		t.Fatalf("owner = %v org = %v, want the roster's org-birch labeled roster", o, resp["org"])
	}
	if !strings.Contains(o["note"].(string), "carries no recorded identity") {
		t.Fatalf("note = %v, want it to say why the record did not answer", o["note"])
	}
	// Where the record answers, the roster is not consulted even if it disagrees.
	src.Roster = append(src.Roster, access.RosterEntry{Handle: "rivera-j", Org: "org-stale"})
	if got := resolvePeerName(src, "machine-anvil-pod-7")["org"]; got != "org-anvil" {
		t.Fatalf("org = %v, want the record's org-anvil, not the roster's", got)
	}
}

func TestADisabledRecordedOwnerIsNotResolved(t *testing.T) {
	src := recordSources()
	src.Identities[0].Disabled = true
	resp := resolvePeerName(src, "machine-anvil-pod-7")
	if resp["resolved"] != false || owner(t, resp)["source"] != "record" {
		t.Fatalf("resolved = %v, want false with the record still named", resp["resolved"])
	}
	if !strings.Contains(resp["reason"].(string), "disabled") {
		t.Fatalf("reason = %v", resp["reason"])
	}
}

func TestAnIdentityWithNoOrganizationSaysSo(t *testing.T) {
	src := recordSources()
	src.Identities[0].AffID, src.Identities[0].Vendor, src.Identities[0].AffNation = "", "", ""
	org := owner(t, resolvePeerName(src, "machine-anvil-pod-7"))["org"].(map[string]any)
	if org["state"] != "missing" || !strings.Contains(org["reason"].(string), "no organization") {
		t.Fatalf("org = %v, want state missing with a reason", org)
	}
}

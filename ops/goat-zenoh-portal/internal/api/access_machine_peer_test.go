package api

import (
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
)

func TestMatchMachineHandle(t *testing.T) {
	machines := []access.MachineIn{
		{SystemHandle: "sam-baxter"},
		{SystemHandle: "sam-baxter-1"}, // longer handle sharing a prefix
		{SystemHandle: ""},             // an empty handle must never match
	}
	cases := []struct {
		name string
		peer string
		want string
	}{
		{"minted key name + NetBird hostname", "machine-sam-baxter-1-sam-baxter-1", "sam-baxter-1"},
		{"minted key name, no hostname", "machine-sam-baxter-1", "sam-baxter-1"},
		{"shorter handle keeps its own peer", "machine-sam-baxter-orin", "sam-baxter"},
		{"handle must end at a hyphen", "machine-sam-baxter-10", "sam-baxter"},
		{"a person's peer is not a machine", "peer-sam-baxter-1-galaxy-2026-08-25-box", ""},
		{"a bare handle is not claimed", "sam-baxter-1", ""},
		{"unknown machine", "machine-nobody-box", ""},
	}
	for _, c := range cases {
		if got := access.MatchMachineHandle(c.peer, machines); got != c.want {
			t.Errorf("%s: MatchMachineHandle(%q) = %q, want %q", c.name, c.peer, got, c.want)
		}
	}
}

// machineSources is one custodian person, live in Ory and declared in the
// roster, who enrolled one machine through the portal. The custodian column
// holds the custodian's slot anchor, as GALAXY's live rows do.
func machineSources() access.Sources {
	return access.Sources{
		Vendors: []access.VendorIn{
			{Prefix: "anchor-custodian", Alias: "a-person", IdentityID: "ory-1"},
		},
		Identities: []access.IdentityIn{{ID: "ory-1", Email: "a.person@example.org"}},
		Roster: []access.RosterEntry{{
			Handle: "a-person", Type: "person", Org: "example-org", Status: "active",
			EmailHash: access.EmailHash("a.person@example.org"),
		}},
		Machines: []access.MachineIn{
			{SystemHandle: "box-1", Custodian: "anchor-custodian", NodeClass: "FUSION", SlotID: "slot-1"},
		},
	}
}

func TestResolvePeerNameMachineNamesItsCustodian(t *testing.T) {
	resp := resolvePeerName(machineSources(), "machine-box-1-box-1")
	if resp["handle"] != "a-person" {
		t.Fatalf("handle = %v, want the custodian a-person", resp["handle"])
	}
	if resp["resolved"] != true {
		t.Fatalf("resolved = %v, want true: the custodian is a live identity", resp["resolved"])
	}
	m, ok := resp["machine"].(map[string]any)
	if !ok || m["system_handle"] != "box-1" || m["node_class"] != "FUSION" {
		t.Fatalf("machine = %v, want box-1 / FUSION so a consumer knows the handle is a custodian", resp["machine"])
	}
}

func TestResolvePeerNameMachineFallsBackToTheSlotAlias(t *testing.T) {
	// No recorded slot→identity linkage: the alias (the handle the operator
	// onboarded, ADR 1039) still names the roster member, but nothing confirms a
	// live person, so the answer must not claim one.
	src := machineSources()
	src.Vendors[0].IdentityID = ""
	src.Identities = nil
	resp := resolvePeerName(src, "machine-box-1-box-1")
	if resp["handle"] != "a-person" {
		t.Fatalf("handle = %v, want a-person via the slot alias", resp["handle"])
	}
	if resp["resolved"] != false {
		t.Fatalf("resolved = %v, want false: no live identity matched", resp["resolved"])
	}
}

func TestResolvePeerNameMachineAcceptsAHandleInTheCustodianColumn(t *testing.T) {
	src := machineSources()
	src.Vendors = nil
	src.Machines[0].Custodian = "a-person"
	if resp := resolvePeerName(src, "machine-box-1"); resp["handle"] != "a-person" {
		t.Fatalf("handle = %v, want a-person from a handle-valued custodian", resp["handle"])
	}
}

func TestResolvePeerNameMachineWithUnknownCustodianNamesNobody(t *testing.T) {
	src := machineSources()
	src.Machines[0].Custodian = "anchor-nobody-knows"
	resp := resolvePeerName(src, "machine-box-1-box-1")
	if h, present := resp["handle"]; present {
		t.Fatalf("handle = %v, want absent: an owner must never be manufactured", h)
	}
	if _, ok := resp["machine"]; !ok {
		t.Fatal("machine missing: the row must still say the peer is an enrolled machine")
	}
	if resp["reason"] == nil || resp["reason"] == "" {
		t.Fatal("reason missing: an unresolved machine must say why")
	}
}

func TestResolvePeerNamePersonPathUnchanged(t *testing.T) {
	resp := resolvePeerName(machineSources(), "peer-a-person-galaxy-2026-06-10-mac")
	if resp["handle"] != "a-person" || resp["resolved"] != true {
		t.Fatalf("person path changed: %v", resp)
	}
	if _, ok := resp["machine"]; ok {
		t.Fatal("a person's peer must not carry a machine block")
	}
}

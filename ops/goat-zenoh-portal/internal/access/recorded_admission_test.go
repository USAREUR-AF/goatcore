package access

import "testing"

// A login the portal onboarded is known the moment it is onboarded. The live
// slot recorded to that identity at onboard (slots.identity_id, arriving here as
// VendorIn.IdentityID) is the admission record; the roster is a hand-rebuilt
// copy of it that lags every onboarding, so people onboarded since the last
// rebuild must not reconcile as strays with on_mesh=false while connected.

// recordedFixture: robin has a live slot recorded to his identity and is absent
// from the roster; the roster knows one unrelated person so it counts as
// supplied.
func recordedFixture() Sources {
	return Sources{
		Vendors: []VendorIn{
			{Prefix: "c6eb6dab92bd221200d2fc8f3cf7efd6", DisplayName: "robin", WriteAnchor: "c6eb6dab92bd221200d2fc8f3cf7efd6",
				Alias: "robin", IdentityID: "kid-robin"},
		},
		Identities: []IdentityIn{
			{ID: "kid-robin", Email: "robin@examplelab.example", Vendor: "examplelab"},
		},
		Roster: []RosterEntry{
			{Handle: "someone-else", Type: "person", Org: "otherorg", Status: "active",
				EmailHash: EmailHash("someone@other.example")},
		},
	}
}

func TestRecordedSlotAdmitsIdentityTheRosterDoesNotKnow(t *testing.T) {
	g := Assemble(recordedFixture())
	n := g.Nodes[nodeID(KindPerson, "kid-robin")]
	if n.State != StateActive {
		t.Errorf("state = %q, want active — a live slot recorded to this login at onboard is the admission record", n.State)
	}
	if n.ReconReason != "" {
		t.Errorf("recon_reason = %q, want empty (not a stray)", n.ReconReason)
	}
	for _, row := range g.Roster().Rows {
		if row.ID == "kid-robin" && row.State == StateOutOfBand {
			t.Errorf("roster row for kid-robin is out_of_band; the stray list would still show him")
		}
	}
}

func TestRecordedSlotWithoutAnyRosterIsStillAdmitted(t *testing.T) {
	src := recordedFixture()
	src.Roster = nil
	if st := Assemble(src).Nodes[nodeID(KindPerson, "kid-robin")].State; st != StateActive {
		t.Errorf("state = %q, want active — the recorded slot does not depend on a roster being configured", st)
	}
}

func TestRecordedSlotOnDisabledIdentityIsCutOff(t *testing.T) {
	src := recordedFixture()
	src.Identities[0].Disabled = true
	if st := Assemble(src).Nodes[nodeID(KindPerson, "kid-robin")].State; st != StateCutOff {
		t.Errorf("state = %q, want cut_off — Ory says the login is inactive", st)
	}
}

// A roster "revoked" is outranked by a live slot recorded to this identity:
// revocation frees the slot, so a live one means the access is live. Reporting
// cut_off would claim a person is cut off while their credential works.
func TestLiveRecordedSlotOutranksStaleRosterRevoked(t *testing.T) {
	src := recordedFixture()
	src.Roster = append(src.Roster, RosterEntry{
		Handle: "robin-old", Type: "person", Org: "examplelab", Status: "revoked",
		EmailHash: EmailHash("robin@examplelab.example"),
	})
	if st := Assemble(src).Nodes[nodeID(KindPerson, "kid-robin")].State; st != StateActive {
		t.Errorf("state = %q, want active", st)
	}
}

// The stray check still does its job: a login with no recorded slot and no
// roster entry is a stray.
func TestUnrecordedUndeclaredLoginIsStillAStray(t *testing.T) {
	src := recordedFixture()
	src.Vendors[0].IdentityID = ""
	n := Assemble(src).Nodes[nodeID(KindPerson, "kid-robin")]
	if n.State != StateOutOfBand {
		t.Fatalf("state = %q, want out_of_band", n.State)
	}
	if n.ReconReason != ReasonUnregisteredKey {
		t.Errorf("recon_reason = %q, want %q", n.ReconReason, ReasonUnregisteredKey)
	}
}

// A re-onboarded person has a revoked entry and an active one under the same
// email. Whichever order the roster lists them in, the active one decides.
func TestReonboardedEmailHashPrefersTheLiveEntry(t *testing.T) {
	email := "lee@example.test"
	revoked := RosterEntry{Handle: "lee-hollis", Type: "person", Org: "harvard", Status: "revoked", EmailHash: EmailHash(email)}
	active := RosterEntry{Handle: "lee-hollis-2", Type: "person", Org: "harvard", Status: "active", EmailHash: EmailHash(email)}
	for name, roster := range map[string][]RosterEntry{
		"revoked first": {revoked, active},
		"active first":  {active, revoked},
	} {
		src := Sources{
			Identities: []IdentityIn{{ID: "kid-lee", Email: email}},
			Roster:     roster,
		}
		if st := Assemble(src).Nodes[nodeID(KindPerson, "kid-lee")].State; st != StateActive {
			t.Errorf("%s: state = %q, want active", name, st)
		}
	}
}

// The recorded slot's handle is the handle the person's NetBird peers are named
// with, so a person the roster does not know still shows their own devices.
func TestRecordedPersonPicksUpOwnPeers(t *testing.T) {
	src := recordedFixture()
	src.Peers = []PeerIn{{
		ID: "nb-robin", Name: "peer-robin-galaxy-2026-09-12-Rorys-MacBook-Pro-2.local",
		Connected: true, LastSeenUnix: 1789400000,
	}}
	g := Assemble(src)
	w := g.Who("robin")
	if w.Resolved != nodeID(KindPerson, "kid-robin") {
		t.Fatalf("Who(robin) resolved %q, want the person via the recorded slot handle", w.Resolved)
	}
	if !w.OnMesh || !w.Connected || w.MeshAttribution != MeshAttrPeer {
		t.Errorf("on_mesh=%v connected=%v attribution=%q, want true/true/peer", w.OnMesh, w.Connected, w.MeshAttribution)
	}
	if w.LastSeenUnix != 1789400000 {
		t.Errorf("last_seen_unix = %d, want the peer's", w.LastSeenUnix)
	}
}

// Longest-wins needs the full candidate set: a roster handle `jane` must not
// claim `peer-jane-smith-…` when `jane-smith` is a recorded slot handle.
func TestRecordedHandleJoinsTheLongestWinsCandidateSet(t *testing.T) {
	src := Sources{
		Vendors: []VendorIn{
			{Prefix: "anchor-ds", WriteAnchor: "anchor-ds", Alias: "jane-smith", IdentityID: "kid-ds"},
		},
		Identities: []IdentityIn{
			{ID: "kid-jane", Email: "jane@example.test"},
			{ID: "kid-ds", Email: "dsmith@example.test"},
		},
		Roster: []RosterEntry{
			{Handle: "jane", Type: "person", Org: "(internal / no vendor)", Status: "active", EmailHash: EmailHash("jane@example.test")}, // oss-allow: generic bucket label
		},
		Peers: []PeerIn{{ID: "nb-ds", Name: "peer-jane-smith-galaxy-2026-09-14-laptop", Connected: true}},
	}
	g := Assemble(src)
	if g.Who("jane").OnMesh {
		t.Error("jane picked up jane-smith's peer — longest-wins was not given the recorded handle")
	}
	if !g.Who("jane-smith").OnMesh {
		t.Error("jane-smith did not pick up their own peer")
	}
}

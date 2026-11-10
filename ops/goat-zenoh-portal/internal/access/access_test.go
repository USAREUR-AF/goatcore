package access

import (
	"strings"
	"testing"
)

// fixture: two vendors (example-vendor with a rendered write anchor + a topic +
// a person + a mesh peer; ghost declared-but-revoked), a community read
// tier, and a release-set.
func fixture() Sources {
	return Sources{
		Vendors: []VendorIn{
			{Prefix: "example-vendor", DisplayName: "Example Vendor", WriteAnchor: "example-vendor"},
			{Prefix: "ghost", DisplayName: "Ghost Robotics", WriteAnchor: ""}, // no rule rendered
		},
		Topics: []TopicIn{
			{VendorPrefix: "example-vendor", KeyPattern: "example-vendor/observations/v1"},
			{VendorPrefix: "ghost", KeyPattern: "ghost/telemetry/v1"},
		},
		Slots: []SlotIn{
			{SlotID: "slot-7", Anchor: "abc123", AssignedTo: "example-vendor/lattice", Vendor: "example-vendor"},
			{SlotID: "slot-8", Anchor: "def456", AssignedTo: "ghost/spot", Vendor: "ghost", Revoked: true},
		},
		Identities: []IdentityIn{
			{ID: "kid-1", Email: "ops@example-vendor.example", Vendor: "example-vendor", Groups: []string{"community-read"}},
			{ID: "kid-2", Email: "ops@ghost.example", Vendor: "ghost", Disabled: true},
		},
		Peers: []PeerIn{
			{ID: "peer-aa", Name: "example-vendor-lattice", Vendor: "example-vendor", Connected: true},
			{ID: "peer-zz", Name: "stray-laptop", Vendor: ""}, // unmanaged
		},
		Roster: []RosterEntry{
			{Handle: "example-vendor-ops", Type: "person", Org: "example-vendor", Status: "active"},
			{Handle: "ghost-ops", Type: "person", Org: "ghost", Status: "revoked"},
		},
		ReadTiers: []ReadTier{{Group: "community-read", Prefix: "**"}},
		Releases:  []ReleaseIn{{Set: "fvey", Partners: []string{"nld", "aus"}, Prefix: "example-vendor/observations"}},
	}
}

func TestAssembleBuildsVendorAndTopicNodes(t *testing.T) {
	g := Assemble(fixture())
	if _, ok := g.Nodes[nodeID(KindVendor, "example-vendor")]; !ok {
		t.Fatal("missing example-vendor vendor node")
	}
	if _, ok := g.Nodes[nodeID(KindTopic, "example-vendor/observations/v1")]; !ok {
		t.Fatal("missing topic node")
	}
}

func TestVendorWithAnchorMayWriteItsTopics(t *testing.T) {
	g := Assemble(fixture())
	r := g.Who("example-vendor")
	// Two writes: the canonical `<anchor>/**` namespace grant (zenoh-acl) +
	// the one specifically-declared topic (zenoh). The namespace grant sorts
	// first ('*' < 'o' at the diverging byte).
	if len(r.Writes) != 2 {
		t.Fatalf("example-vendor should write its namespace + its one topic, got %+v", r.Writes)
	}
	if r.Writes[0].Topic != "example-vendor/**" || r.Writes[0].Layer != "zenoh-acl" {
		t.Errorf("first write should be the anchor namespace grant, got %+v", r.Writes[0])
	}
	if r.Writes[1].Topic != "example-vendor/observations/v1" || r.Writes[1].Layer != "zenoh" {
		t.Errorf("second write should be the declared topic, got %+v", r.Writes[1])
	}
}

// TestSlotPoolAnchorWritesItsNamespace is the W/R 0/0 regression guard: a
// slot-pool member (a vendor whose prefix is its slot anchor) declares NO
// topics yet must still report one write — its `<anchor>/**` namespace grant.
// Without it no per-topic edge exists and the write column is empty for the
// whole roster.
func TestSlotPoolAnchorWritesItsNamespace(t *testing.T) {
	g := Assemble(Sources{
		Vendors: []VendorIn{{Prefix: "e7a9c75b", DisplayName: "jane-operator", WriteAnchor: "e7a9c75b"}},
		// no Topics declared for this anchor
		FlatCommunityRead: true,
	})
	r := g.Who("e7a9c75b")
	if len(r.Writes) != 1 || r.Writes[0].Topic != "e7a9c75b/**" {
		t.Fatalf("slot-pool anchor should write its namespace, got %+v", r.Writes)
	}
	if len(r.Reads) != 1 || r.Reads[0].Topic != "**" {
		t.Fatalf("flat community-read should grant a read, got %+v", r.Reads)
	}
}

// TestCommunityReadReflectedInWho guards that the flat community-read FLAG
// (carried on the graph, not as N×M edges) surfaces as a read grant in the
// forward query — it previously did not, so reads showed 0 for everyone.
func TestCommunityReadReflectedInWho(t *testing.T) {
	s := fixture()
	s.FlatCommunityRead = true
	g := Assemble(s)
	r := g.Who("example-vendor")
	if len(r.Reads) == 0 || r.Reads[0].Topic != "**" {
		t.Fatalf("community-read flag should add a `**` read grant, got %+v", r.Reads)
	}
}

// TestLastSeenPropagatesToVendor checks the mesh last-activity dimension rolls
// up from a peer to its vendor spine (the freshest peer wins).
func TestLastSeenPropagatesToVendor(t *testing.T) {
	s := fixture()
	s.Peers[0].LastSeenUnix = 1_700_000_000
	g := Assemble(s)
	r := g.Who("example-vendor")
	if r.LastSeenUnix != 1_700_000_000 {
		t.Fatalf("vendor last_seen should be its peer's, got %d", r.LastSeenUnix)
	}
	if !r.Connected {
		t.Error("example-vendor peer is connected → vendor connected")
	}
}

// TestRosterSummarizesEveryIdentity checks the bulk query returns one row per
// vendor + person with non-zero W/R where granted.
func TestRosterSummarizesEveryIdentity(t *testing.T) {
	s := fixture()
	s.FlatCommunityRead = true
	g := Assemble(s)
	rs := g.Roster()
	byID := map[string]RosterRow{}
	for _, row := range rs.Rows {
		byID[row.ID] = row
	}
	a, ok := byID["example-vendor"]
	if !ok {
		t.Fatal("example-vendor missing from roster")
	}
	if a.Writes < 1 || a.Reads < 1 {
		t.Errorf("example-vendor should have ≥1 write (namespace) + ≥1 read (community), got W%d/R%d", a.Writes, a.Reads)
	}
	if a.WriteAnchor != "example-vendor" {
		t.Errorf("example-vendor roster row should carry its write anchor, got %q", a.WriteAnchor)
	}
	// vendors + persons are rows; topics/peers/slots are not standalone rows.
	if _, ok := byID["ghost"]; !ok {
		t.Error("ghost vendor should be a roster row even with no write rule")
	}
}

func TestVendorWithoutAnchorWritesNothing(t *testing.T) {
	g := Assemble(fixture())
	r := g.Who("ghost")
	if len(r.Writes) != 0 {
		t.Fatalf("ghost has no rendered anchor → no writes, got %+v", r.Writes)
	}
	if len(r.Notes) == 0 {
		t.Error("expected a note explaining the missing write rule")
	}
}

func TestPersonReadViaCommunityTier(t *testing.T) {
	g := Assemble(fixture())
	r := g.Who("ops@example-vendor.example")
	if r.Kind != KindPerson {
		t.Fatalf("resolved kind = %q, want person", r.Kind)
	}
	// community-read (**) covers both topics
	if len(r.Reads) != 2 {
		t.Fatalf("community-read should cover both topics, got %+v", r.Reads)
	}
}

func TestReverseTopicWritersAndReceivers(t *testing.T) {
	g := Assemble(fixture())
	r := g.Topic("example-vendor/observations/v1")
	if !r.Found {
		t.Fatal("topic should be found")
	}
	if len(r.Writers) != 1 || r.Writers[0].Identity != "Example Vendor" {
		t.Fatalf("expected Example Vendor as sole writer, got %+v", r.Writers)
	}
	// release-set fvey prefix example-vendor/observations covers this key
	if len(r.Receivers) != 2 {
		t.Fatalf("expected 2 release receivers (nld,aus), got %v", r.Receivers)
	}
}

func TestReverseTopicNoWriterFlagsSilentDenial(t *testing.T) {
	g := Assemble(fixture())
	r := g.Topic("ghost/telemetry/v1") // ghost has no rendered anchor
	if len(r.Writers) != 0 {
		t.Fatalf("ghost topic should have no writers, got %+v", r.Writers)
	}
	found := false
	for _, n := range r.Notes {
		if containsF177(n) {
			found = true
		}
	}
	if !found {
		t.Error("a writer-less topic must flag the F-177 silent-denial note")
	}
}

func TestRevokedSlotReconcilesCutOff(t *testing.T) {
	g := Assemble(fixture())
	n := g.Nodes[nodeID(KindSlot, "slot-8")]
	if n.State != StateCutOff {
		t.Fatalf("revoked slot should be cut_off, got %q", n.State)
	}
}

func TestDisabledIdentityWithRevokedRosterIsCutOff(t *testing.T) {
	g := Assemble(fixture())
	n := g.Nodes[nodeID(KindPerson, "kid-2")]
	if n.State != StateCutOff {
		t.Fatalf("ghost person (disabled + roster revoked) should be cut_off, got %q", n.State)
	}
}

func TestEmailHashReconcilesTaglessPerson(t *testing.T) {
	// The F-234 fix: an organic person with NO vendor tag on the live Ory
	// identity still reconciles — and picks up affiliation — because a declared
	// member shares its email hash. Previously this was an "untracked" stray.
	email := "pat.example@mil.example"
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "pat-example", Type: "person", Org: "estonia", Status: "active",
			Affiliation: "organic", Nation: "EST", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-organic", Email: email, Vendor: ""}},
	}
	g := Assemble(s)
	n := g.Nodes[nodeID(KindPerson, "kid-organic")]
	if n.State != StateActive {
		t.Errorf("state = %s, want active (email-hash reconciled, not a stray)", n.State)
	}
	if n.ReconReason != "" {
		t.Errorf("recon_reason = %q, want empty (reconciled, not a stray)", n.ReconReason)
	}
	if n.Affiliation != "organic" || n.Nation != "EST" {
		t.Errorf("affiliation = %q/%q, want organic/EST (attached via email match)", n.Affiliation, n.Nation)
	}

	// Case-insensitive: an upper-cased Ory email still matches.
	s.Identities[0].Email = "PAT.EXAMPLE@MIL.EXAMPLE"
	if Assemble(s).Nodes[nodeID(KindPerson, "kid-organic")].State != StateActive {
		t.Error("upper-cased email must still reconcile (hash is case-insensitive)")
	}

	// A revoked declared member reconciles to cut-off, not a spurious active.
	s.Roster[0].Status = "revoked"
	s.Identities[0].Email = email
	if Assemble(s).Nodes[nodeID(KindPerson, "kid-organic")].State != StateCutOff {
		t.Error("revoked declared member must reconcile to cut_off")
	}
}

func TestSlotAnchorAffiliationViaAliasHop(t *testing.T) {
	// Slot-pool shape: the vendor-spine row's Prefix is an opaque UUID
	// anchor with no declared-org key, so `declaredByOrg[Prefix]` is empty and
	// the row used to render "·". The declared member is keyed by handle, and
	// the slot's alias (assigned_to) IS that handle — so hopping
	// Prefix→Alias→declared member attaches the member's affiliation.
	anchor := "e7a9c75b1f2a3c4d5e6f7a8b9c0d1e2f"
	s := Sources{
		Vendors: []VendorIn{{Prefix: anchor, DisplayName: "Kratcorp", Alias: "example-system"}},
		Roster: []RosterEntry{{
			Handle: "example-system", Type: "system", Org: "kratcorp", Status: "active",
			Affiliation: "vendor", Nation: "USA",
		}},
	}
	n := Assemble(s).Nodes[nodeID(KindVendor, anchor)]
	if n.Affiliation != "vendor" || n.Nation != "USA" {
		t.Errorf("affiliation = %q/%q, want vendor/USA (attached via alias hop)", n.Affiliation, n.Nation)
	}

	// No alias, no org-key match ⇒ honest "·" (empty), never guessed.
	s.Vendors[0].Alias = ""
	if a := Assemble(s).Nodes[nodeID(KindVendor, anchor)].Affiliation; a != "" {
		t.Errorf("affiliation = %q, want empty when neither prefix-key nor alias resolves", a)
	}

	// A direct declared-org key still wins without needing the alias (per-vendor
	// goatnet path is unaffected).
	s.Vendors[0] = VendorIn{Prefix: "kratcorp", DisplayName: "Kratcorp"}
	if a := Assemble(s).Nodes[nodeID(KindVendor, "kratcorp")].Affiliation; a != "vendor" {
		t.Errorf("affiliation = %q, want vendor (direct org-key match)", a)
	}
}

func TestStrayReconReasonClassified(t *testing.T) {
	// Each stray class, both for Ory identities and mesh peers, gets the reason
	// a safe sweep needs to tell a true cut candidate from a reconcilable mismatch.
	s := Sources{
		Roster: []RosterEntry{
			// An org with only a system member — an identity claiming it has no
			// declared *person* home (offboarding residue).
			{Handle: "sys-1", Type: "system", Org: "orphanorg", Status: "active"},
		},
		Identities: []IdentityIn{
			{ID: "kid-untracked", Vendor: ""},        // no tag at all
			{ID: "kid-unreg", Vendor: "ghosttag"},    // tag matches no declared org
			{ID: "kid-residue", Vendor: "orphanorg"}, // org declared, no person member
		},
		Peers: []PeerIn{
			{ID: "p-untracked", Name: "random-laptop", Vendor: ""}, // off-convention name
			{ID: "p-unreg", Name: "peer-acme-1", Vendor: ""},       // convention, unmatched vendor
		},
	}
	g := Assemble(s)
	cases := map[string]ReconReason{
		nodeID(KindPerson, "kid-untracked"): ReasonUntracked,
		nodeID(KindPerson, "kid-unreg"):     ReasonUnregisteredKey,
		nodeID(KindPerson, "kid-residue"):   ReasonNoDeclaredMember,
		nodeID(KindMeshPeer, "p-untracked"): ReasonUntracked,
		nodeID(KindMeshPeer, "p-unreg"):     ReasonUnregisteredKey,
	}
	for id, want := range cases {
		n, ok := g.Nodes[id]
		if !ok {
			t.Fatalf("missing node %s", id)
		}
		if n.State != StateOutOfBand {
			t.Errorf("%s: state=%s, want out_of_band", id, n.State)
		}
		if n.ReconReason != want {
			t.Errorf("%s: recon_reason=%q, want %q", id, n.ReconReason, want)
		}
	}
}

func TestAffClaimReconcilesTagFreeIdentity(t *testing.T) {
	// Phase D (ADR 1043): a live identity carrying traits.aff but NO legacy
	// vendor tag reconciles against the roster's true org id (OrgID), even
	// though every declared member keys the legacy index under its per-person
	// vendor_tag. Before the aff claim this identity was an untracked stray.
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "example-org-sam", Type: "person", Org: "example-org-sam", OrgID: "example-org",
			Status: "active", Affiliation: "vendor", Nation: "USA",
		}},
		Identities: []IdentityIn{{ID: "kid-aff", AffID: "example-org", AffKind: "vendor", AffNation: "USA"}},
	}
	n := Assemble(s).Nodes[nodeID(KindPerson, "kid-aff")]
	if n.State != StateActive {
		t.Errorf("state = %s, want active (aff-claim reconciled, not a stray)", n.State)
	}
	if n.ReconReason != "" {
		t.Errorf("recon_reason = %q, want empty (reconciled)", n.ReconReason)
	}
	if n.Affiliation != "vendor" || n.Nation != "USA" {
		t.Errorf("affiliation = %q/%q, want vendor/USA (declared join via OrgID)", n.Affiliation, n.Nation)
	}
}

func TestPerPersonVendorTagBeatsOrgLevelAff(t *testing.T) {
	// The finer per-person key must win over the org-level aff key: a revoked
	// member's own vendor_tag entry keeps the identity cut_off even though an
	// org sibling under the same aff org is still active.
	s := Sources{
		Roster: []RosterEntry{
			{Handle: "example-org-old", Type: "person", Org: "example-org-old", OrgID: "example-org", Status: "revoked"},
			{Handle: "example-org-new", Type: "person", Org: "example-org-new", OrgID: "example-org", Status: "active"},
		},
		Identities: []IdentityIn{{ID: "kid-old", Vendor: "example-org-old", AffID: "example-org"}},
	}
	if st := Assemble(s).Nodes[nodeID(KindPerson, "kid-old")].State; st != StateCutOff {
		t.Errorf("state = %s, want cut_off (per-person vendor-tag key wins over org-level aff)", st)
	}
}

func TestAffStrayClassification(t *testing.T) {
	// aff-carrying strays: an aff.id matching no declared org = unregistered
	// key; matching an org with no person member = offboarding residue. The
	// untracked class now requires BOTH keys absent (covered in
	// TestStrayReconReasonClassified).
	s := Sources{
		Roster: []RosterEntry{{Handle: "sys-1", Type: "system", Org: "sys-1", OrgID: "orphanorg", Status: "active"}},
		Identities: []IdentityIn{
			{ID: "kid-aff-unreg", AffID: "ghostorg"},
			{ID: "kid-aff-residue", AffID: "orphanorg"},
		},
	}
	g := Assemble(s)
	if r := g.Nodes[nodeID(KindPerson, "kid-aff-unreg")].ReconReason; r != ReasonUnregisteredKey {
		t.Errorf("unmatched aff.id: recon_reason = %q, want %q", r, ReasonUnregisteredKey)
	}
	if r := g.Nodes[nodeID(KindPerson, "kid-aff-residue")].ReconReason; r != ReasonNoDeclaredMember {
		t.Errorf("aff org with no person member: recon_reason = %q, want %q", r, ReasonNoDeclaredMember)
	}
}

func TestAffClaimFillsAffiliationWhenNoDeclaredJoin(t *testing.T) {
	// An aff-carrying identity whose org isn't in the roster still renders its
	// own declared-at-mint kind/nation (the claim was written from the manifest
	// at onboard — declared data, not a guess), while its STATE stays an honest
	// out_of_band: display fills in, state is never guessed.
	s := Sources{
		Roster:     []RosterEntry{{Handle: "x", Type: "person", Org: "x", OrgID: "otherorg", Status: "active"}},
		Identities: []IdentityIn{{ID: "kid-new", AffID: "neworg", AffKind: "partner", AffNation: "NO"}},
	}
	n := Assemble(s).Nodes[nodeID(KindPerson, "kid-new")]
	if n.State != StateOutOfBand {
		t.Errorf("state = %s, want out_of_band (org not declared)", n.State)
	}
	if n.Affiliation != "partner" || n.Nation != "NO" {
		t.Errorf("affiliation = %q/%q, want partner/NO (claim fallback)", n.Affiliation, n.Nation)
	}
}

func TestAffOrgEdgeWhenVendorNodeIsOrgID(t *testing.T) {
	// Per-vendor goatnet where the vendor spine node's prefix IS the org id: an
	// aff-carrying, tag-free identity gets its belongs-to-org edge via the
	// claim, with the deciding record naming traits.aff.id.
	s := Sources{
		Vendors:    []VendorIn{{Prefix: "example-org", DisplayName: "Example-Org"}},
		Roster:     []RosterEntry{{Handle: "example-org-sam", Type: "person", Org: "example-org-sam", OrgID: "example-org", Status: "active"}},
		Identities: []IdentityIn{{ID: "kid-aff", AffID: "example-org"}},
	}
	g := Assemble(s)
	found := false
	for _, e := range g.Edges {
		if e.From == nodeID(KindPerson, "kid-aff") && e.To == nodeID(KindVendor, "example-org") && e.Kind == EdgeBelongsToOrg {
			found = true
			if !strings.Contains(e.Reason, "traits.aff.id=example-org") {
				t.Errorf("edge reason = %q, want it to name traits.aff.id", e.Reason)
			}
		}
	}
	if !found {
		t.Fatal("missing belongs_to_org edge via the aff claim")
	}
}

func TestVendorSpineAffiliationViaOrgIDIndex(t *testing.T) {
	// Vendor prefix == the org id, but every member keys the legacy index under
	// its per-person vendor tag — the OrgID index attaches the org's
	// affiliation to the spine node anyway.
	s := Sources{
		Vendors: []VendorIn{{Prefix: "example-org"}},
		Roster: []RosterEntry{{
			Handle: "example-org-sam", Type: "person", Org: "example-org-sam", OrgID: "example-org",
			Status: "active", Affiliation: "vendor", Nation: "USA",
		}},
	}
	n := Assemble(s).Nodes[nodeID(KindVendor, "example-org")]
	if n.Affiliation != "vendor" || n.Nation != "USA" {
		t.Errorf("affiliation = %q/%q, want vendor/USA (via the OrgID index)", n.Affiliation, n.Nation)
	}
}

func TestUnmanagedPeerIsOutOfBand(t *testing.T) {
	g := Assemble(fixture())
	n := g.Nodes[nodeID(KindMeshPeer, "peer-zz")]
	if n.State != StateOutOfBand {
		t.Fatalf("vendor-less peer should be out_of_band, got %q", n.State)
	}
}

func TestNoRosterYieldsUnknownNotGuess(t *testing.T) {
	s := fixture()
	s.Roster = nil
	g := Assemble(s)
	// active (non-disabled) identity with no roster → unknown, not active
	n := g.Nodes[nodeID(KindPerson, "kid-1")]
	if n.State != StateUnknown {
		t.Fatalf("no roster → unknown, got %q", n.State)
	}
	// but a disabled identity is unambiguously cut off even without a roster
	n2 := g.Nodes[nodeID(KindPerson, "kid-2")]
	if n2.State != StateCutOff {
		t.Fatalf("disabled identity is cut_off even with no roster, got %q", n2.State)
	}
}

func TestUnresolvedWhoIsGraceful(t *testing.T) {
	g := Assemble(fixture())
	r := g.Who("nobody")
	if r.Resolved != "" || len(r.Notes) == 0 {
		t.Fatalf("unresolved who should return empty resolution + a hint, got %+v", r)
	}
}

func TestOnMeshReflectsPeerEdge(t *testing.T) {
	g := Assemble(fixture())
	if !g.Who("example-vendor").OnMesh {
		t.Error("example-vendor has a matched peer → on_mesh should be true")
	}
	if g.Who("ghost").OnMesh {
		t.Error("ghost has no peer → on_mesh should be false")
	}
}

func TestPrefixCovers(t *testing.T) {
	cases := []struct {
		tier, key string
		want      bool
	}{
		{"**", "anything/here", true},
		{"example-vendor/**", "example-vendor/obs/v1", true},
		{"example-vendor", "example-vendor/obs/v1", true},
		{"example-vendor", "example-vendor", true},
		{"example-vendor", "ghost/obs", false},
		{"example-vendor/obs", "example-vendor/observations", false}, // not a path-segment boundary
	}
	for _, c := range cases {
		if got := prefixCovers(c.tier, c.key); got != c.want {
			t.Errorf("prefixCovers(%q,%q)=%v want %v", c.tier, c.key, got, c.want)
		}
	}
}

func TestWhyAllowsWriteWithAnchorAndMesh(t *testing.T) {
	s := fixture()
	s.FlatCommunityRead = true
	g := Assemble(s)
	r := g.Why("example-vendor", "example-vendor/observations/v1")
	if r.WriteVerdict != "allow" {
		t.Fatalf("example-vendor (anchor + mesh) should write-allow, got %q; steps=%+v", r.WriteVerdict, r.Steps)
	}
	if r.ReadVerdict != "allow" {
		t.Errorf("flat community-read should read-allow, got %q", r.ReadVerdict)
	}
}

func TestWhyDeniesWriteWithoutAnchorAndNamesF177(t *testing.T) {
	g := Assemble(fixture())
	r := g.Why("ghost", "ghost/telemetry/v1") // ghost has no rendered anchor
	if r.WriteVerdict != "deny" {
		t.Fatalf("ghost (no anchor) should write-deny, got %q", r.WriteVerdict)
	}
	// the zenoh step should name the F-177 silent-denial cause
	found := false
	for _, st := range r.Steps {
		if st.Layer == "zenoh" && st.Verdict == "deny" && containsF177(st.Reason) {
			found = true
		}
	}
	if !found {
		t.Errorf("zenoh deny step must name the F-177 cause; steps=%+v", r.Steps)
	}
}

func TestWhyDeniesWriteWhenOffMesh(t *testing.T) {
	s := fixture()
	// give ghost an anchor but it still has no mesh peer
	s.Vendors[1].WriteAnchor = "ghost"
	g := Assemble(s)
	r := g.Why("ghost", "ghost/telemetry/v1")
	if r.WriteVerdict != "deny" {
		t.Fatalf("off-mesh should write-deny even with an anchor, got %q", r.WriteVerdict)
	}
	meshDenied := false
	for _, st := range r.Steps {
		if st.Layer == "mesh" && st.Verdict == "deny" {
			meshDenied = true
		}
	}
	if !meshDenied {
		t.Errorf("expected a mesh deny step; steps=%+v", r.Steps)
	}
}

func TestWhyReadDeniedWhenNotCommunityAndNoTier(t *testing.T) {
	s := fixture()
	s.FlatCommunityRead = false
	s.ReadTiers = nil
	g := Assemble(s)
	r := g.Why("example-vendor", "example-vendor/observations/v1")
	if r.ReadVerdict != "deny" {
		t.Fatalf("no community-read + no tier → read-deny, got %q", r.ReadVerdict)
	}
}

func TestWhyUnregisteredTopic(t *testing.T) {
	g := Assemble(fixture())
	r := g.Why("example-vendor", "example-vendor/nope/v9")
	if r.WriteVerdict != "deny" {
		t.Errorf("unregistered topic → write-deny, got %q", r.WriteVerdict)
	}
}

func containsF177(s string) bool {
	return len(s) > 0 && (indexOf(s, "F-177") >= 0 || indexOf(s, "silently denied") >= 0)
}

func indexOf(haystack, needle string) int {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if haystack[i:i+len(needle)] == needle {
			return i
		}
	}
	return -1
}

func TestReonboardedPersonPrefersActiveDeclaredEntry(t *testing.T) {
	// The bsmith1 regression: a re-onboarded person carries BOTH
	// their old revoked declared entry and a fresh active one (same email —
	// the registry deprecates, never deletes). First-match let the revoked
	// ghost win and reported a live, freshly minted admin as cut_off.
	email := "person-901@example.org"
	s := Sources{
		Roster: []RosterEntry{
			{Handle: "ben-smith-2", Type: "person", Org: "demo-org-data", Status: "revoked", EmailHash: emailHash(email)},
			{Handle: "bsmith1", Type: "person", Org: "usareur-af", Status: "active", EmailHash: emailHash(email)},
		},
		Identities: []IdentityIn{{ID: "kid-ben", Email: email}},
	}
	g := Assemble(s)
	n := g.Nodes[nodeID(KindPerson, "kid-ben")]
	if n.State != StateActive {
		t.Errorf("state = %s, want active (active declared entry must beat the revoked ghost)", n.State)
	}
	// And with ONLY the revoked entry, cut_off must still hold (the fix must
	// not weaken the real offboarding signal).
	s2 := Sources{
		Roster:     []RosterEntry{{Handle: "ben-smith-2", Type: "person", Org: "demo-org-data", Status: "revoked", EmailHash: emailHash(email)}},
		Identities: []IdentityIn{{ID: "kid-ben2", Email: email}},
	}
	g2 := Assemble(s2)
	n2 := g2.Nodes[nodeID(KindPerson, "kid-ben2")]
	if n2.State != StateCutOff {
		t.Errorf("revoked-only state = %s, want cut_off", n2.State)
	}
}

func TestWhoResolvesDeclaredRosterHandle(t *testing.T) {
	// The selector a human actually types — and what the CLI's `me` resolves
	// an operator key to — is the declared roster HANDLE. It must reach the
	// same person node the email selector reaches (join = email hash, F-234).
	email := "person-900@example.org"
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "jane", Type: "person",
			Org:    "(internal / no vendor)", // oss-allow: generic bucket label
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-jane", Email: email, Vendor: ""}},
	}
	g := Assemble(s)

	byHandle := g.Who("jane")
	if byHandle.Resolved == "" {
		t.Fatalf("Who(handle) missed: notes=%v", byHandle.Notes)
	}
	byEmail := g.Who(email)
	if byHandle.Resolved != byEmail.Resolved {
		t.Errorf("handle and email resolve differently: %q vs %q", byHandle.Resolved, byEmail.Resolved)
	}

	// A declared handle with NO live identity resolves to nothing — an
	// honest miss, never a fabricated node.
	miss := g.Who("ghost-handle")
	if miss.Resolved != "" {
		t.Errorf("undeclared handle resolved to %q, want miss", miss.Resolved)
	}
}

func TestWhoAttributesPersonOwnMeshPeer(t *testing.T) {
	// The on-mesh honesty gap: an internal person with no org never matched
	// any peer (the join rode the org spine only), so `who` reported
	// "on mesh: no" for someone demonstrably connected. The peer-name custody
	// link (peer-<handle>-…, Option B) attributes their own device — org
	// membership not required.
	email := "person-900@example.org"
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "jane", Type: "person",
			Org:    "(internal / no vendor)", // oss-allow: generic bucket label
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-jane", Email: email}},
		Peers: []PeerIn{{
			ID: "nb-1", Name: "peer-jane-galaxy-20260601-mac",
			Connected: true, LastSeenUnix: 1750000000,
		}},
	}
	g := Assemble(s)
	w := g.Who("jane")
	if !w.OnMesh || !w.Connected {
		t.Errorf("on_mesh=%v connected=%v, want true/true from the person's own peer", w.OnMesh, w.Connected)
	}
	if w.LastSeenUnix != 1750000000 {
		t.Errorf("last_seen_unix = %d, want the attributed peer's", w.LastSeenUnix)
	}
	if w.MeshAttribution != MeshAttrPeer {
		t.Errorf("mesh_attribution = %q, want %q", w.MeshAttribution, MeshAttrPeer)
	}
	// The why-trace's mesh gate must pass on the same custody edge.
	why := g.Why("jane", "some/topic")
	for _, st := range why.Steps {
		if st.Layer == "mesh" && st.Verdict != "allow" {
			t.Errorf("why mesh verdict = %s (%s), want allow via the person's own peer", st.Verdict, st.Reason)
		}
	}
}

func TestWhoUnattributedPersonIsHonest(t *testing.T) {
	// A declared person with no matching peer and no org peers: the graph has
	// nothing to attribute — that is "can't know", NOT "known absent". The
	// wire must say so (mesh_attribution=none) instead of a bare on_mesh=false
	// the CLI would render as a misleading "no".
	email := "ghost@example.com"
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "ghost", Type: "person", Org: "(internal / no vendor)", // oss-allow: generic bucket label
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-ghost", Email: email}},
		Peers:      []PeerIn{{ID: "nb-2", Name: "laptop-unconventional-name", Connected: true}},
	}
	w := Assemble(s).Who("ghost")
	if w.OnMesh {
		t.Errorf("on_mesh = true, want false (no attributable peer)")
	}
	if w.MeshAttribution != MeshAttrNone {
		t.Errorf("mesh_attribution = %q, want %q", w.MeshAttribution, MeshAttrNone)
	}
}

func TestWhoPersonOrgFallbackStillRollsUp(t *testing.T) {
	// A person with no own-named device but whose org has peers keeps the
	// legacy org roll-up — labeled honestly as org-sourced, not person-sourced.
	email := "member@vendorco.example"
	s := Sources{
		Vendors: []VendorIn{{Prefix: "vendorco"}},
		Roster: []RosterEntry{{
			Handle: "vc-member", Type: "person", Org: "vendorco",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-vc", Email: email, Vendor: "vendorco"}},
		Peers:      []PeerIn{{ID: "nb-3", Name: "peer-vendorco-gw-1", Vendor: "vendorco", Connected: true}},
	}
	w := Assemble(s).Who(email)
	if !w.OnMesh {
		t.Errorf("on_mesh = false, want true via the org spine")
	}
	if w.MeshAttribution != MeshAttrOrg {
		t.Errorf("mesh_attribution = %q, want %q", w.MeshAttribution, MeshAttrOrg)
	}
}

func TestMatchPeerHandleLongestPrefixWins(t *testing.T) {
	roster := []RosterEntry{
		{Handle: "jane", Type: "person"},
		{Handle: "jane-2", Type: "person"},
	}
	if h := MatchPeerHandle("peer-jane-2-galaxy-20260601", roster); h != "jane-2" {
		t.Errorf("matched %q, want jane-2 (longest prefix)", h)
	}
	if h := MatchPeerHandle("peer-jane-galaxy-20260601", roster); h != "jane" {
		t.Errorf("matched %q, want jane", h)
	}
	if h := MatchPeerHandle("unconventional", roster); h != "" {
		t.Errorf("matched %q, want no match", h)
	}
}

func TestWhoPersonWritesResolveViaSlotAlias(t *testing.T) {
	// On a slot-pool goatnet the vendor spine is keyed by opaque UUID anchors
	// while a live identity carries only the legacy short-tag, so the direct
	// person→org hop never resolves and every person would report 0 writes while
	// every org row has its grant. The slot ALIAS is the hop that closes it — keyed
	// on the org short-tag, which is what an alias actually spells (the onboard sets
	// it from `zenoh.trial.vendor` and drift-checks it against `org.short_tag`).
	email := "alex@example.com"
	s := Sources{
		Vendors: []VendorIn{{
			Prefix: "3f2a0b1c-uuid-anchor", WriteAnchor: "3f2a0b1c-uuid-anchor",
			Alias: "example-org-alex",
		}},
		Roster: []RosterEntry{{
			Handle: "alex", Type: "person", Org: "example-org-alex",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-alex", Email: email, Vendor: "example-org-alex"}},
	}
	g := Assemble(s)
	w := g.Who("alex")
	if len(w.Writes) == 0 {
		t.Fatalf("writes = 0, want the anchor grant via the alias hop; notes=%v", w.Notes)
	}
	if w.Writes[0].Topic != "3f2a0b1c-uuid-anchor/**" {
		t.Errorf("write topic = %q, want the slot anchor namespace", w.Writes[0].Topic)
	}
	// The email selector must land on the same answer.
	if we := g.Who(email); len(we.Writes) != len(w.Writes) {
		t.Errorf("email selector writes = %d, handle selector = %d — selectors diverge", len(we.Writes), len(w.Writes))
	}
	// The why-trace's zenoh step hops the same anchor for declared topics.
	s.Topics = []TopicIn{{VendorPrefix: "3f2a0b1c-uuid-anchor", KeyPattern: "3f2a0b1c-uuid-anchor/obs/v1"}}
	g2 := Assemble(s)
	why := g2.Why("alex", "3f2a0b1c-uuid-anchor/obs/v1")
	for _, st := range why.Steps {
		if st.Layer == "zenoh" && st.Verdict != "allow" {
			t.Errorf("why zenoh verdict = %s (%s), want allow via the person's anchor", st.Verdict, st.Reason)
		}
	}
}

func TestWhoPersonTagKeyedOrgStillResolves(t *testing.T) {
	// Seed-model regression guard: on a per-vendor goatnet the person's tag IS
	// the vendor prefix — that hop must keep working with the alias hop added.
	email := "member@vendorco.example"
	s := Sources{
		Vendors: []VendorIn{{Prefix: "vendorco", WriteAnchor: "vendorco"}},
		Roster: []RosterEntry{{
			Handle: "vc-member", Type: "person", Org: "vendorco",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-vc", Email: email, Vendor: "vendorco"}},
	}
	w := Assemble(s).Who(email)
	if len(w.Writes) == 0 || w.Writes[0].Topic != "vendorco/**" {
		t.Errorf("writes = %v, want the tag-keyed vendorco/** grant preserved", w.Writes)
	}
}

func TestWhoPersonZeroWritesGetsUnattributedNote(t *testing.T) {
	// A person with no resolvable anchor must say "unattributed", not imply a
	// known zero — writes are cert-gated and grants outside the registry
	// (operator ops/** rules) are invisible to the graph.
	email := "person-900@example.org"
	s := Sources{
		Roster: []RosterEntry{{
			Handle: "jane", Type: "person",
			Org:    "(internal / no vendor)", // oss-allow: generic bucket label
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-jane", Email: email}},
	}
	w := Assemble(s).Who("jane")
	if len(w.Writes) != 0 {
		t.Fatalf("writes = %v, want none (no anchor exists)", w.Writes)
	}
	found := false
	for _, note := range w.Notes {
		if strings.Contains(note, "writes unattributed") {
			found = true
		}
	}
	if !found {
		t.Errorf("missing the writes-unattributed honesty note; notes=%v", w.Notes)
	}
}

func TestWhoPersonWritesResolveViaTagAlias(t *testing.T) {
	// A goatnet can mint slot aliases as the member's VENDOR TAG, not their personal
	// handle. A person whose tag names an alias anchors there.
	email := "alex@example.com"
	s := Sources{
		Vendors: []VendorIn{{
			Prefix: "3ea54776-uuid-anchor", WriteAnchor: "3ea54776-uuid-anchor",
			Alias: "example-org-alex",
		}},
		Roster: []RosterEntry{{
			Handle: "alex-smith", Type: "person", Org: "example-org-alex",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-alex", Email: email, Vendor: "example-org-alex"}},
	}
	w := Assemble(s).Who(email)
	if len(w.Writes) == 0 || w.Writes[0].Topic != "3ea54776-uuid-anchor/**" {
		t.Errorf("writes = %v, want the anchor grant via the tag-alias join", w.Writes)
	}
}

func TestWhoTagAliasWinsAndHandleAliasIsRefused(t *testing.T) {
	// The join-policy reconcile (fragility audit §1.2, fix-train item 8). A slot
	// whose alias equals someone's HANDLE is not evidence it is their slot — an
	// alias spells the org short-tag, so a handle can only match by colliding
	// with another member's tag. goat-cli's rotate/revoke cascade has refused
	// that join as a mis-rotation ; the portal now refuses it too,
	// so the two components can no longer disagree about who owns a slot.
	email := "dual@example.com"
	s := Sources{
		Vendors: []VendorIn{
			{Prefix: "own-uuid", WriteAnchor: "own-uuid", Alias: "dual"},
			{Prefix: "org-uuid", WriteAnchor: "org-uuid", Alias: "org-tag"},
		},
		Roster: []RosterEntry{{
			Handle: "dual", Type: "person", Org: "org-tag",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-dual", Email: email, Vendor: "org-tag"}},
	}
	w := Assemble(s).Who(email)
	if len(w.Writes) == 0 || w.Writes[0].Topic != "org-uuid/**" {
		t.Errorf("writes = %v, want the org-tag alias — the handle-aliased slot must not be joined", w.Writes)
	}

	// With ONLY the handle-aliased slot present the person is unattributed and
	// says so. An honest miss beats a confident wrong owner; this is the row the
	// recorded linkage (slots.identity_id) fills.
	s.Vendors = s.Vendors[:1]
	w2 := Assemble(s).Who(email)
	if len(w2.Writes) != 0 {
		t.Fatalf("writes = %v, want none — the only candidate matched on a handle", w2.Writes)
	}
	var noted bool
	for _, note := range w2.Notes {
		if strings.Contains(note, "writes unattributed") {
			noted = true
		}
	}
	if !noted {
		t.Errorf("missing the writes-unattributed honesty note; notes=%v", w2.Notes)
	}
}

func TestWhoPersonWritesResolveViaSuffixedAlias(t *testing.T) {
	// A re-issued slot's alias carries a -<n> suffix (kim-park-2 — the
	// ≤2-certs convention); exact matching left these people "unattributed"
	// despite a live slot (2 of the 21 zero-write rows).
	email := "sang@example.com"
	s := Sources{
		Vendors: []VendorIn{{
			Prefix: "9ab1-uuid", WriteAnchor: "9ab1-uuid", Alias: "kim-park-2",
		}},
		Roster: []RosterEntry{{
			Handle: "kim-park", Type: "person", Org: "kim-park",
			Status: "active", EmailHash: emailHash(email),
		}},
		Identities: []IdentityIn{{ID: "kid-kim", Email: email, Vendor: "kim-park"}},
	}
	w := Assemble(s).Who(email)
	if len(w.Writes) == 0 || w.Writes[0].Topic != "9ab1-uuid/**" {
		t.Errorf("writes = %v, want the suffixed-alias slot attributed", w.Writes)
	}
	// An exact alias must always beat another slot's stripped suffix.
	s.Vendors = append([]VendorIn{{Prefix: "exact-uuid", WriteAnchor: "exact-uuid", Alias: "kim-park"}}, s.Vendors...)
	w2 := Assemble(s).Who(email)
	if len(w2.Writes) == 0 || w2.Writes[0].Topic != "exact-uuid/**" {
		t.Errorf("writes = %v, want the exact-alias slot to win", w2.Writes)
	}
}

func TestAliasBase(t *testing.T) {
	for in, want := range map[string]string{
		"kim-park-2": "kim-park", "demo-org-data-adam-2": "demo-org-data-adam",
		"team-x1": "team-x1", "example-org-alex": "example-org-alex", "solo": "solo", "a-10": "a",
	} {
		if got := aliasBase(in); got != want {
			t.Errorf("aliasBase(%q) = %q, want %q", in, got, want)
		}
	}
}

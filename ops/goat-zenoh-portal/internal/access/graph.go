// Package access assembles the Data-Access Common Operating Picture
// (ADR 1026, docs/design/data-access-cop.md). It reads the four
// enforcement layers (Netbird mesh, Ory identity, the Zenoh write-slot /
// ACL registry, and partner release-sets) plus the declared issuance
// roster, and answers one question in both directions: who can access
// which data, and why.
//
// This file holds the PURE core: the graph model + assembly + the
// declared-vs-actual reconciliation. It has no I/O and no portal-type
// dependencies — callers adapt their live readers into the Sources
// struct, so the assembly logic is unit-testable against fixtures. The
// live wiring (reg + kratos + netbird readers → Sources) lives in the
// API handler; the why-trace is a separate file (G.4 PR-B).
//
// Join model: the vendor prefix is the spine. A vendor org has a
// mesh peer (peer name → vendor), Ory identities (traits.vendor.short_tag
// → vendor), one or more Zenoh write slots / a vendor-prefix write anchor,
// and topics (topics.vendor_prefix). The declared roster keys org →
// person/system siblings. Reconciling declared against the live layers is
// what distinguishes "cut off" from "never had it".
package access

import (
	"crypto/sha256"
	"encoding/hex"
	"sort"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/peername"
)

// emailHash is the tag-independent reconcile key: SHA-256 of the
// lowercased, trimmed email, matching the hash gen-roster.py emits per member.
// Empty email → empty (never a spurious match).
func emailHash(email string) string {
	e := strings.TrimSpace(strings.ToLower(email))
	if e == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(e))
	return hex.EncodeToString(sum[:])
}

// EmailHash exposes the tag-independent reconcile key (SHA-256 of the lowercased
// email) so callers outside this package (the peer→person resolver) can match a
// live Ory identity to a roster member by the same key the graph uses.
func EmailHash(email string) string { return emailHash(email) }

// NodeKind is the category of an access-graph node.
type NodeKind string

const (
	KindVendor       NodeKind = "vendor"        // an org / tenant (the spine)
	KindPerson       NodeKind = "person"        // an Ory identity (person or service)
	KindSlot         NodeKind = "slot"          // a Zenoh publisher slot (cert CN + write anchor)
	KindMeshPeer     NodeKind = "mesh_peer"     // a Netbird peer
	KindTopic        NodeKind = "topic"         // a topic / namespace region
	KindPartner      NodeKind = "partner"       // a release-set member org
	KindReleaseTopic NodeKind = "release_topic" // release/<set>/… region
)

// EdgeKind is the category of a permission edge. Each edge names the
// layer that grants it and the specific record that is the source of
// truth (Edge.Reason).
type EdgeKind string

const (
	EdgeOnMesh         EdgeKind = "is_on_mesh"          // mesh peer ── transport ──> vendor/slot
	EdgeMayWrite       EdgeKind = "may_write"           // slot/vendor ── Zenoh rule ──> topic
	EdgeMayRead        EdgeKind = "may_read"            // person ── read tier ──> topic
	EdgeBelongsToOrg   EdgeKind = "belongs_to_org"      // person ── traits.vendor ──> vendor
	EdgeMayReceiveRel  EdgeKind = "may_receive_release" // partner ── release-set ──> release topic
	EdgeMarkedReleasab EdgeKind = "marked_releasable"   // topic ── releasability ──> release topic
)

// ReconState is a node's declared-vs-actual reconciliation verdict — the
// "cut off vs never had it" distinction the COP exists to resolve.
type ReconState string

const (
	// StateActive: present in both the declared roster and the live layer.
	StateActive ReconState = "active"
	// StateCutOff: in the declared roster but absent from the live layer —
	// negative enforcement was applied (revoked / disabled / cert expired).
	StateCutOff ReconState = "cut_off"
	// StateOutOfBand: in the live layer but not the declared roster — an
	// offboarding residual or an unmanaged peer no enrollment tracked.
	StateOutOfBand ReconState = "out_of_band"
	// StateUnknown: insufficient declared data to reconcile (the roster
	// was not supplied). Never reported as active/cut-off on a guess.
	StateUnknown ReconState = "unknown"
)

// ReconReason explains WHY a node is StateOutOfBand (a "stray") — the
// classification a safe stray-sweep needs so it never blind-cuts a false
// positive. A stray is live-on-a-layer but unmatched in the declared roster;
// the reason names which failure produced that, and therefore whether the
// stray is a genuine cut candidate or just a reconcilable key mismatch.
type ReconReason string

const (
	// ReasonUntracked: the live entity carries no org/tenant key at all (a mesh
	// peer whose name doesn't follow the peer-name grammar, or an Ory
	// identity with no vendor short-tag). Never enrolled through the tracked
	// path → a genuine cut candidate (after corroboration).
	ReasonUntracked ReconReason = "untracked"
	// ReasonUnregisteredKey: the live entity DOES carry an org/tenant key, but
	// it matches no declared org/registry. Ambiguous — naming drift / a typo'd
	// tag (reconcilable: fix the key, do NOT cut) OR an org that was offboarded
	// (a real cut). Must not be blind-cut; route to human review. This is the
	// "false stray" highlight class.
	ReasonUnregisteredKey ReconReason = "unregistered-key"
	// ReasonNoDeclaredMember: the org IS declared, but this specific member is
	// absent from its roster — classic offboarding residue (the member record
	// was removed, the live artifact lingers). A cut candidate.
	ReasonNoDeclaredMember ReconReason = "no-declared-member"
)

// Node is one identity or data region in the graph.
type Node struct {
	ID    string     `json:"id"`    // stable key, e.g. "vendor:acme" or "topic:acme/obs/v1"
	Kind  NodeKind   `json:"kind"`  //
	Label string     `json:"label"` // human display name
	State ReconState `json:"state"` // declared-vs-actual verdict (identities only; topics StateActive)
	// Vendor is the org-spine key this node belongs to (empty for release topics).
	Vendor string `json:"vendor,omitempty"`
	// WriteAnchor is the Zenoh write-anchor prefix a vendor spine node may
	// publish under (<anchor>/**). Empty means no write rule is in the ACL —
	// the silent-denial gap the COP surfaces. For slot-pool members the anchor
	// IS the vendor prefix (the pre-baked per-uuid grant, ADR 1025); for the
	// seed model it is the vendor prefix once ACLRenderedAt is stamped.
	WriteAnchor string `json:"write_anchor,omitempty"`
	// LastSeenUnix is the most recent mesh last_seen across this entity's peers
	// (0 = unknown — never reported as a confident "offline"). The last-activity
	// dimension of the cross-layer COP; mesh is the first layer to carry it.
	LastSeenUnix int64 `json:"last_seen_unix,omitempty"`
	// Connected is true when at least one of this entity's mesh peers is
	// currently connected (best-effort live reachability).
	Connected bool `json:"connected,omitempty"`
	// Affiliation/Nation are the identity-grammar attributes (ADR 1043) from the
	// declared roster, attached by matching this node's org key to the declared
	// org. Empty when the declared org carried none or this node has no declared
	// match (rendered "·" downstream — never guessed). Identity-only display
	// attribution, never an access input (the cert URI-SAN carries born-as
	// affiliation; this is the registry's current view).
	Affiliation string `json:"affiliation,omitempty"`
	Nation      string `json:"nation,omitempty"`
	// ReconReason classifies WHY this node is a stray (StateOutOfBand): untracked
	// / unregistered-key / no-declared-member. Empty for non-stray nodes. The
	// safe-sweep signal — unregistered-key is the "don't blind-cut" highlight.
	ReconReason ReconReason `json:"recon_reason,omitempty"`
}

// Edge is one permission, labeled with its granting layer + source record.
type Edge struct {
	From   string   `json:"from"`   // Node.ID
	To     string   `json:"to"`     // Node.ID
	Kind   EdgeKind `json:"kind"`   //
	Layer  string   `json:"layer"`  // "mesh" | "identity" | "zenoh" | "release"
	Reason string   `json:"reason"` // the deciding record, e.g. "topics.vendor_prefix=acme"
}

// Graph is the assembled, queryable access picture. Nodes are keyed by ID.
type Graph struct {
	Nodes map[string]Node `json:"nodes"`
	Edges []Edge          `json:"edges"`
	// CommunityRead is the flat read model (ADR 1025): every on-mesh
	// identity may read every topic. Carried as a flag rather than N×M
	// per-topic read edges; the why-trace consults it for the read verdict.
	// When false, reads come only from explicit MayRead edges (read tiers).
	CommunityRead bool `json:"community_read"`
	// OnboardingStage carries Sources.OnboardingStage through to Roster() —
	// anchor → furthest onboarding step with positive evidence. It participates
	// in no edge and decides no verdict; it is display state the COP renders
	// alongside access, never an input to one.
	OnboardingStage map[string]string `json:"onboarding_stage,omitempty"`
	// HandleIndex maps a declared roster member handle to the graph node it
	// reconciled to (person via email-hash match). Lets the who/why resolver
	// answer the handle a human actually types (`goat access who <handle>`, and
	// the CLI's `me` — whoami resolves an operator key to its person handle).
	// Only reconciled handles appear; a declared member with no live identity
	// resolves to nothing (honest miss, not a fabricated node).
	HandleIndex map[string]string `json:"handle_index,omitempty"`
	// PersonAnchor maps a person node ID to the vendor/anchor node that holds
	// their write slot. On a slot-pool goatnet the vendor spine is keyed by
	// opaque UUID anchors, so the direct person→org hop resolves nothing and
	// every person reported 0 writes; the recorded linkage (slots.identity_id)
	// closes it, with the slot alias↔org-tag join as the labeled fallback.
	PersonAnchor map[string]string `json:"person_anchor,omitempty"`
	// PersonAnchorSource records HOW each PersonAnchor entry was resolved —
	// AnchorRecorded (slots.identity_id, stamped at onboard) or the org-tag name
	// join. A consumer that shows ownership must be able to distinguish a
	// recorded owner from a name that happened to match, because presenting the
	// second as the first mis-attributes ownership.
	PersonAnchorSource map[string]string `json:"person_anchor_source,omitempty"`
	// AnchorJoin counts the resolutions by source. The fallback count is the
	// backfill's progress meter — it trends to zero as recorded linkages land
	// (identity fragility audit §1.2).
	AnchorJoin AnchorJoinStats `json:"anchor_join"`
}

// Person→anchor resolution sources, strongest first.
const (
	// AnchorRecorded: slots.identity_id — the linkage recorded at onboard.
	AnchorRecorded = "recorded"
	// AnchorTagJoin: the slot alias equals the identity's org short-tag. A name
	// join, and the ONLY one this graph will make: a slot is onboarded with
	// `prefix = zenoh.trial.vendor` and the portal sets the slot alias to that
	// prefix, so the org short-tag is what a slot alias actually spells. It is
	// still coarse — org-mates sharing a tag share the anchor, right for a
	// shared system and wrong for a person — so it is always labeled.
	AnchorTagJoin = "tag-join"
)

// AnchorJoinStats counts person→anchor resolutions by source for one assembly.
type AnchorJoinStats struct {
	Recorded int `json:"recorded"`
	TagJoin  int `json:"tag_join"`
}

// Fallbacks is the number of anchors resolved by parsing names rather than by a
// recorded linkage — the number the backfill drives to zero.
func (a AnchorJoinStats) Fallbacks() int { return a.TagJoin }

// setPersonAnchor records a person→anchor resolution, keeping the FIRST (i.e.
// strongest, since callers apply sources in strength order) and counting which
// source answered. A later, weaker join never overwrites a recorded one.
func (g *Graph) setPersonAnchor(personID, vendorNodeID, source string) {
	if g.PersonAnchor == nil {
		g.PersonAnchor = map[string]string{}
		g.PersonAnchorSource = map[string]string{}
	}
	if _, have := g.PersonAnchor[personID]; have {
		return
	}
	g.PersonAnchor[personID] = vendorNodeID
	g.PersonAnchorSource[personID] = source
	switch source {
	case AnchorRecorded:
		g.AnchorJoin.Recorded++
	case AnchorTagJoin:
		g.AnchorJoin.TagJoin++
	}
}

// ── input adapters (populated by the live handler from reg/kratos/netbird) ──

// VendorIn is a registered vendor (the org spine). Prefix is its identity.
type VendorIn struct {
	Prefix      string
	DisplayName string
	// WriteAnchor is the Zenoh write-anchor prefix this vendor's rule grants
	// (the seed model: the vendor prefix itself; the slot-pool model: the
	// slot's SlotAnchor). Empty means no write rule rendered yet.
	WriteAnchor string
	// Alias is the slot handle assigned to this vendor's anchor (ADR 1039's
	// assigned_to), empty on per-vendor goatnets. On a slot-pool goatnet the
	// Prefix is an opaque UUID with no declared-roster key, so affiliation is
	// resolved by hopping Prefix→Alias→declared member handle instead.
	Alias string
	// IdentityID is the Kratos identity UUID recorded on this anchor's slot at
	// onboard (slots.identity_id). When present it is the person→anchor ground
	// truth and beats both name joins below; empty means no linkage was recorded
	// and ownership falls back to matching names.
	IdentityID string
}

// TopicIn is a declared topic. VendorPrefix + KeyPattern; KeyPattern is
// "<vendor>/<system>/…".
type TopicIn struct {
	VendorPrefix string
	KeyPattern   string
}

// SlotIn is a Zenoh write slot. Assigned (AssignedTo non-empty) slots are
// allocated to a system; Revoked marks a cut-off slot.
type SlotIn struct {
	SlotID     string
	Anchor     string // cert CN + write-anchor prefix (registry.SlotAnchor)
	AssignedTo string // system id (vendor/system), empty if free
	Vendor     string // resolved org-spine for the assignment
	Revoked    bool
}

// MachineIn is a live machine the portal enrolled (ADR 1096), read from the
// slots registry. The declared roster is built from operator manifests and
// never carries these, so without this layer a `machine-<handle>` peer
// resolves to nobody while the registry holds its custodian.
type MachineIn struct {
	SystemHandle string // slots.assigned_to — the handle the mesh key was minted for
	// Custodian is slots.enrolled_by as the enroll path stamped it. registry.Slot
	// documents it as a person handle; rows can also hold the custodian's slot
	// anchor. Both are accepted by the resolver.
	Custodian string
	NodeClass string
	SlotID    string
}

// IdentityIn is an Ory person/service. Vendor is the legacy per-identity
// traits.vendor.short_tag; AffID/AffKind/AffNation are the identity-grammar
// Phase D traits.aff claim (ADR 1043) — the true org id (shared unit, e.g.
// `example-org`), its kind, and nation. Identities minted before Phase D carry no
// aff (all three empty) and reconcile via the legacy vendor-tag key. Groups
// are the group claims that map to read tiers; Disabled marks an
// Ory-inactive identity.
type IdentityIn struct {
	ID        string
	Email     string
	Name      string
	Vendor    string
	AffID     string
	AffKind   string
	AffNation string
	Groups    []string
	Disabled  bool
}

// PeerIn is a Netbird mesh peer. Vendor is the matched org (peer name →
// vendor); Connected is live reachability (best-effort); LastSeenUnix is the
// mgmt API's last_seen as a unix timestamp (0 = never reported / unknown).
type PeerIn struct {
	ID           string
	Name         string
	Vendor       string
	Connected    bool
	LastSeenUnix int64
}

// RosterEntry is one declared member from the issuance registry (the
// `ops/operators/*.yml` manifests / generated roster index). Type is
// "person" | "system"; Status is "active" | "inactive" | "revoked".
// Affiliation/Nation are the member's org-level identity-grammar attributes
// (ADR 1043), joined from the roster's orgs block in LoadRoster — empty when
// the declared org carried none (rendered as an honest "·", never guessed).
type RosterEntry struct {
	Handle string
	Type   string
	Org    string // legacy reconcile key: the member's vendor_tag, else its org id
	// OrgID is the member's true affiliation.id (the shared unit, e.g. `example-org`) —
	// the key an aff-carrying (Phase D) identity reconciles on, independent of
	// the per-identity vendor-tag substitution in Org.
	OrgID       string
	Status      string
	Affiliation string // org kind: vendor | organic | partner
	Nation      string // org nation (ISO code) — a fact about the ORGANISATION
	// Nationality is the PERSON's own citizenship (ISO alpha-3), from the
	// manifest's identity.nationality. It is NOT Nation and is not derived from
	// it: a Canadian officer at a US command is CAN while the org
	// is USA, and both are true (ADR 1171). Empty means UNDECLARED — the roster
	// says nothing — and must never be read as "same as the org", which is the
	// inference this field exists to delete. Older manifests leave it empty
	// because identity.nationality post-dates them.
	Nationality string
	// EmailHash is SHA-256(lowercased email) — the tag-independent reconcile key
	// that matches a live Ory identity to this member even when it carries no
	// vendor short-tag (organic/partner people), closing F-234. Empty for
	// members the generator had no email for (e.g. systems).
	EmailHash string
}

// ReadTier maps an Ory group to the topic-key prefix its members may read.
// A flat community-read tier is represented by Prefix == "**".
type ReadTier struct {
	Group  string
	Prefix string
}

// ReleaseIn is a release-set: which partner orgs may receive which release
// topic prefixes (ADR 1027).
type ReleaseIn struct {
	Set      string
	Partners []string
	// Prefix is the release/<set>/… region members may receive.
	Prefix string
}

// Sources is the full raw input to Assemble — what the live handler reads
// from the four layers + the declared roster. Roster may be nil (then
// identity nodes reconcile to StateUnknown rather than guessing).
type Sources struct {
	Vendors    []VendorIn
	Topics     []TopicIn
	Slots      []SlotIn
	Machines   []MachineIn // portal-enrolled machines: the custody edge the roster lacks
	Identities []IdentityIn
	Peers      []PeerIn
	Roster     []RosterEntry
	ReadTiers  []ReadTier
	Releases   []ReleaseIn
	// OnboardingStage maps a write anchor to the furthest onboarding step with
	// positive evidence behind it (the /api/progress `stage` vocabulary). The
	// COP carries it so an admin looking at "who is here" can see that someone
	// never finished arriving, instead of having to notice a quiet row and go
	// read a different surface. Absent key = no progress row; the COP renders that as
	// unknown, never as a stage.
	OnboardingStage map[string]string
	// FlatCommunityRead sets the graph's CommunityRead model. When
	// true, every on-mesh identity reads every topic; restricted read tiers
	// (ReadTiers) still add their own explicit edges on top.
	FlatCommunityRead bool
}

func nodeID(kind NodeKind, key string) string { return string(kind) + ":" + key }

// Assemble builds the access graph from the four live layers + the
// declared roster. Pure: deterministic given Sources, no I/O. Edges carry
// the layer + deciding record so a downstream answer is auditable.
func Assemble(s Sources) *Graph {
	g := &Graph{Nodes: map[string]Node{}, Edges: nil, CommunityRead: s.FlatCommunityRead, OnboardingStage: s.OnboardingStage}

	// Declared roster index: org → members, for reconciliation. Plus an email-
	// hash index so a live Ory identity matches its declared member even
	// with no vendor short-tag (organic/partner people) — the tag-independent key.
	declaredByOrg := map[string][]RosterEntry{}
	// declaredByAffOrg keys members on the true affiliation.id — the key a
	// Phase D aff-carrying identity presents (traits.aff.id, ADR 1043). The
	// vendor-tag index above stays for identities minted before Phase D.
	declaredByAffOrg := map[string][]RosterEntry{}
	declaredByEmailHash := map[string]RosterEntry{}
	declaredByHandle := map[string]RosterEntry{}
	rosterSupplied := len(s.Roster) > 0
	for _, r := range s.Roster {
		declaredByOrg[r.Org] = append(declaredByOrg[r.Org], r)
		if r.OrgID != "" {
			declaredByAffOrg[r.OrgID] = append(declaredByAffOrg[r.OrgID], r)
		}
		if r.EmailHash != "" {
			// A re-onboarded person carries BOTH their old revoked entry and a
			// fresh one under the same email. The live entry is their current
			// identity, so a revoked entry never displaces a non-revoked one;
			// last-write-wins made the verdict depend on the roster's ordering.
			if prev, dup := declaredByEmailHash[r.EmailHash]; !dup || prev.Status == "revoked" || r.Status != "revoked" {
				declaredByEmailHash[r.EmailHash] = r
			}
		}
		if r.Handle != "" {
			declaredByHandle[r.Handle] = r
		}
	}

	// Vendor write anchors, resolved before node creation so the spine node
	// carries its own write-anchor (the Who/Roster namespace-grant source).
	anchorByVendor := map[string]string{}
	// Slot alias → vendor node ID, for the person→anchor join (see
	// Graph.PersonAnchor). Aliases are unique per slot; first wins on a dup.
	vendorByAlias := map[string]string{}
	// Recorded identity UUID → vendor node ID (slots.identity_id). This is the
	// join the org-tag alias join approximates; when it answers, that one doesn't run.
	vendorByIdentity := map[string]string{}
	// Recorded identity UUID → the handle its slot was onboarded under (the
	// slot alias, ADR 1039). The onboarding handle is also the handle the
	// person's mesh peers are named with, so this is what lets a person the
	// roster does not know yet still pick up their own devices.
	aliasByIdentity := map[string]string{}
	for _, v := range s.Vendors {
		if v.WriteAnchor != "" {
			anchorByVendor[v.Prefix] = v.WriteAnchor
		}
		if v.Alias != "" {
			if _, dup := vendorByAlias[v.Alias]; !dup {
				vendorByAlias[v.Alias] = nodeID(KindVendor, v.Prefix)
			}
		}
		if v.IdentityID != "" {
			if _, dup := vendorByIdentity[v.IdentityID]; !dup {
				vendorByIdentity[v.IdentityID] = nodeID(KindVendor, v.Prefix)
				if v.Alias != "" {
					aliasByIdentity[v.IdentityID] = v.Alias
				}
			}
		}
	}
	// Second pass: a re-issued slot's alias carries a `-<n>` suffix
	// (jane-doe-2, demo-org-data-adam-2 — the ≤2-certs-per-person
	// convention), which an exact tag match misses. Register the stripped base
	// too, but only when no exact alias claims it — an alias that IS someone's
	// exact tag always wins over another slot's suffix.
	for _, v := range s.Vendors {
		if v.Alias == "" {
			continue
		}
		base := aliasBase(v.Alias)
		if base == v.Alias {
			continue
		}
		if _, taken := vendorByAlias[base]; !taken {
			vendorByAlias[base] = nodeID(KindVendor, v.Prefix)
		}
	}

	// 1. Vendor spine nodes. Affiliation rides on the declared org, matched by
	// the vendor's prefix key (clean on per-vendor goatnets where prefix == the
	// declared org tag). On a slot-pool goatnet the prefix is an opaque UUID with
	// no declared key, so hop Prefix→Alias→declared member handle to pick up the
	// member's affiliation — still declared-data-only, never guessed. A "·"
	// remains only when neither key nor alias resolves to a declared member.
	for _, v := range s.Vendors {
		label := v.DisplayName
		if label == "" {
			label = v.Prefix
		}
		aff, nation := affFromDeclared(declaredByOrg[v.Prefix])
		if aff == "" && nation == "" {
			// Per-vendor goatnets where the prefix is the org id but every
			// member keys the legacy index under its per-person vendor tag.
			aff, nation = affFromDeclared(declaredByAffOrg[v.Prefix])
		}
		if aff == "" && nation == "" && v.Alias != "" {
			if m, ok := declaredByHandle[v.Alias]; ok {
				aff, nation = m.Affiliation, m.Nation
			}
		}
		g.Nodes[nodeID(KindVendor, v.Prefix)] = Node{
			ID: nodeID(KindVendor, v.Prefix), Kind: KindVendor, Label: label,
			State: StateActive, Vendor: v.Prefix, WriteAnchor: anchorByVendor[v.Prefix],
			Affiliation: aff, Nation: nation,
		}
	}

	// 2. Topic nodes + the vendor write edge (Zenoh rule).
	for _, t := range s.Topics {
		tid := nodeID(KindTopic, t.KeyPattern)
		g.Nodes[tid] = Node{ID: tid, Kind: KindTopic, Label: t.KeyPattern, State: StateActive, Vendor: t.VendorPrefix}
	}
	for _, t := range s.Topics {
		from := nodeID(KindVendor, t.VendorPrefix)
		if _, ok := g.Nodes[from]; !ok {
			continue // topic for an unregistered vendor — skip the dangling edge
		}
		anchor, has := anchorByVendor[t.VendorPrefix]
		if !has {
			continue // no write rule rendered for this vendor yet
		}
		g.Edges = append(g.Edges, Edge{
			From: from, To: nodeID(KindTopic, t.KeyPattern), Kind: EdgeMayWrite, Layer: "zenoh",
			Reason: "zenoh write rule anchor=" + anchor + " covers prefix " + t.VendorPrefix + "/**",
		})
	}

	// 3. Slot nodes + their assignment + write edge.
	for _, sl := range s.Slots {
		if sl.AssignedTo == "" {
			continue // free slots are pool capacity, not access-graph identities
		}
		sid := nodeID(KindSlot, sl.SlotID)
		st := StateActive
		if sl.Revoked {
			st = StateCutOff
		}
		g.Nodes[sid] = Node{ID: sid, Kind: KindSlot, Label: sl.AssignedTo, State: st, Vendor: sl.Vendor}
		if v := nodeID(KindVendor, sl.Vendor); sl.Vendor != "" {
			if _, ok := g.Nodes[v]; ok {
				g.Edges = append(g.Edges, Edge{
					From: sid, To: v, Kind: EdgeBelongsToOrg, Layer: "zenoh",
					Reason: "slot " + sl.SlotID + " assigned_to=" + sl.AssignedTo,
				})
			}
		}
	}

	// 4. Identity nodes + org membership + read edges (read tier via group).
	for _, id := range s.Identities {
		pid := nodeID(KindPerson, id.ID)
		label := id.Email
		if label == "" {
			label = id.Name
		}
		// Reconcile by the tag-independent email hash FIRST: a declared
		// member sharing this identity's email is a match even with no vendor
		// short-tag (organic/partner people), so they aren't mis-flagged as
		// strays and pick up their org's affiliation. Then the legacy per-person
		// vendor-tag key; then the Phase D aff claim (traits.aff.id, ADR 1043) —
		// the true org id, so an aff-carrying identity reconciles to its declared
		// org even when it carries no vendor tag at all. The finer per-person
		// keys win over the org-level aff key: org membership can't decide one
		// person's cut-off state when a sibling is still active.
		var st ReconState
		var aff, nation string
		var reason ReconReason
		if entry, ok := declaredByEmailHash[emailHash(id.Email)]; ok && id.Email != "" {
			if entry.Status == "revoked" || id.Disabled {
				st = StateCutOff
			} else {
				st = StateActive
			}
			aff, nation = entry.Affiliation, entry.Nation
		} else {
			declared := declaredByOrg[id.Vendor]
			if len(declared) == 0 && id.AffID != "" {
				declared = declaredByAffOrg[id.AffID]
			}
			st = reconcileIdentity(id, declared, rosterSupplied)
			aff, nation = affFromDeclared(declared)
			if st == StateOutOfBand {
				reason = classifyStrayIdentity(id, declaredByOrg, declaredByAffOrg)
			}
		}
		// The portal's own onboarding record decides the state when it exists. A
		// live (assigned, unrevoked) slot recorded to THIS identity at onboard
		// (slots.identity_id) is written by the operator-authenticated onboard
		// path and cleared by revocation (RevokeSlot frees the slot), so it is the
		// current fact about whether this login was admitted. The roster is a
		// hand-regenerated copy of the same decision and lags every onboarding
		// until someone rebuilds it; without this, everyone onboarded since the
		// last rebuild reads as a stray. It also
		// outranks a roster "revoked": the slot is live, so the access is live,
		// and cut_off would claim otherwise.
		if _, recorded := vendorByIdentity[id.ID]; recorded {
			if id.Disabled {
				st = StateCutOff
			} else {
				st = StateActive
			}
			reason = ""
		}
		// The identity's own aff claim fills what no declared join produced —
		// declared-at-mint data (the onboard writes traits.aff from the
		// manifest), not a guess; the regenerated roster stays primary.
		if aff == "" {
			aff = id.AffKind
		}
		if nation == "" {
			nation = id.AffNation
		}
		g.Nodes[pid] = Node{ID: pid, Kind: KindPerson, Label: label, State: st, Vendor: id.Vendor, Affiliation: aff, Nation: nation, ReconReason: reason}
		// Person → anchor, in strength order. The RECORDED linkage
		// (slots.identity_id, stamped at onboard) is ground truth and runs first;
		// the org-tag join below is the one name-parsing fallback it exists to
		// retire, and setPersonAnchor records which one answered so a consumer can
		// tell an owner from a guess (identity fragility audit §1.2).
		if vid, ok := vendorByIdentity[id.ID]; ok {
			g.setPersonAnchor(pid, vid, AnchorRecorded)
		}
		// Index the declared handle -> this person (the email-hash join is
		// the tag-independent key, F-234), so handle selectors resolve.
		if entry, ok := declaredByEmailHash[emailHash(id.Email)]; ok && id.Email != "" && entry.Handle != "" {
			if g.HandleIndex == nil {
				g.HandleIndex = map[string]string{}
			}
			g.HandleIndex[entry.Handle] = pid
		}
		// And the handle the recorded slot was onboarded under, for a person the
		// roster has not caught up with. The declared handle above wins a clash.
		if alias, ok := aliasByIdentity[id.ID]; ok {
			if g.HandleIndex == nil {
				g.HandleIndex = map[string]string{}
			}
			if _, taken := g.HandleIndex[alias]; !taken {
				g.HandleIndex[alias] = pid
			}
		}
		// Fallback, tag-keyed: a slot alias spells the member's ORG SHORT-TAG
		// (e.g. `example-org-jane`) because the onboard sets the alias from
		// `zenoh.trial.vendor` and drift-checks it against `org.short_tag`. A
		// person whose tag names a slot alias anchors there.
		//
		// There is deliberately NO alias↔handle fallback. A handle is not a slot
		// alias, so a handle lookup can only "succeed" by colliding with some
		// OTHER member's tag alias — which goat-cli's rotate/revoke cascade has
		// refused as a mis-rotation (resolve_member_anchors in
		// crates/goat-cli/src/cmd/admin.rs, docs/design/credential-rotation.md).
		// The portal made that join anyway, so the two components disagreed about
		// ownership and the surface people READ was the looser one (
		// identity fragility audit §1.2). A person the tag join cannot place now
		// reports no anchor — an honest miss, which the recorded linkage fills.
		if id.Vendor != "" {
			if vid, ok := vendorByAlias[id.Vendor]; ok {
				g.setPersonAnchor(pid, vid, AnchorTagJoin)
			}
		}
		if v := nodeID(KindVendor, id.Vendor); id.Vendor != "" && g.has(v) {
			g.Edges = append(g.Edges, Edge{
				From: pid, To: v, Kind: EdgeBelongsToOrg, Layer: "identity",
				Reason: "traits.org.short_tag=" + id.Vendor,
			})
		} else if v := nodeID(KindVendor, id.AffID); id.AffID != "" && g.has(v) {
			// Phase D join: the aff claim's org id names a vendor spine node
			// directly (per-vendor goatnets where prefix == the org id).
			g.Edges = append(g.Edges, Edge{
				From: pid, To: v, Kind: EdgeBelongsToOrg, Layer: "identity",
				Reason: "traits.aff.id=" + id.AffID,
			})
		}
		// Read edges: each group the identity holds maps (maybe) to a read tier.
		for _, grp := range id.Groups {
			for _, rt := range s.ReadTiers {
				if rt.Group != grp {
					continue
				}
				for _, t := range s.Topics {
					if prefixCovers(rt.Prefix, t.KeyPattern) {
						g.Edges = append(g.Edges, Edge{
							From: pid, To: nodeID(KindTopic, t.KeyPattern), Kind: EdgeMayRead, Layer: "identity",
							Reason: "group " + grp + " → read tier " + rt.Prefix,
						})
					}
				}
			}
		}
	}

	// 5. Mesh peer nodes + on-mesh edge to the vendor spine. A peer's last_seen
	// + connected state is the mesh layer's last-activity input; it propagates
	// up to the vendor spine (an org's last-activity = its freshest peer).
	//
	// Additionally: the person↔peer custody edge. The onboarding orchestrator
	// names every peer deterministically (peer-<handle>[-<short_tag>]-<site>-…,
	// docs/design/peer-identity-resolution.md Option B), so a peer attributes to
	// its declared member by the same longest-handle-prefix match the /api/
	// access/peer resolver uses. Without this edge a person's on-mesh state
	// rode the org spine only, and internal people with no org never showed
	// their own enrolled devices (the "on mesh: no while demonstrably
	// connected" gap). A person with attributed peers rolls up their own
	// connected/last_seen, org membership not required.
	// Candidate handles = every declared roster handle plus every recorded-slot
	// handle indexed above. Passing the full set matters: longest-wins is what
	// stops `ada` claiming `peer-ada-smith-…`, and it only works when
	// `ada-smith` is among the candidates.
	peerHandles := make([]string, 0, len(s.Roster)+len(aliasByIdentity))
	for _, m := range s.Roster {
		peerHandles = append(peerHandles, m.Handle)
	}
	for _, alias := range aliasByIdentity {
		peerHandles = append(peerHandles, alias)
	}
	for _, p := range s.Peers {
		mid := nodeID(KindMeshPeer, p.ID)
		st := StateOutOfBand // a peer with no vendor match is unmanaged
		if p.Vendor != "" {
			st = StateActive
		}
		reason := ReconReason("")
		if st == StateOutOfBand {
			reason = classifyStrayPeer(p.Name)
		}
		g.Nodes[mid] = Node{
			ID: mid, Kind: KindMeshPeer, Label: p.Name, State: st, Vendor: p.Vendor,
			LastSeenUnix: p.LastSeenUnix, Connected: p.Connected, ReconReason: reason,
		}
		if p.Vendor != "" {
			if vid := nodeID(KindVendor, p.Vendor); g.has(vid) {
				g.Edges = append(g.Edges, Edge{
					From: mid, To: vid, Kind: EdgeOnMesh, Layer: "mesh",
					Reason: "netbird peer name " + p.Name + " → vendor " + p.Vendor,
				})
				vn := g.Nodes[vid]
				if p.LastSeenUnix > vn.LastSeenUnix {
					vn.LastSeenUnix = p.LastSeenUnix
				}
				vn.Connected = vn.Connected || p.Connected
				g.Nodes[vid] = vn
			}
		}
		if h := peername.MatchLongestKey(p.Name, peerHandles); h != "" {
			if pid, ok := g.HandleIndex[h]; ok {
				g.Edges = append(g.Edges, Edge{
					From: mid, To: pid, Kind: EdgeOnMesh, Layer: "mesh",
					Reason: "netbird peer name " + p.Name + " → member handle " + h + " (peer-name custody)",
				})
				pn := g.Nodes[pid]
				if p.LastSeenUnix > pn.LastSeenUnix {
					pn.LastSeenUnix = p.LastSeenUnix
				}
				pn.Connected = pn.Connected || p.Connected
				g.Nodes[pid] = pn
			}
		}
	}

	// 6. Release sets: partner → release topic, and topic → release marking.
	for _, rel := range s.Releases {
		rtid := nodeID(KindReleaseTopic, rel.Set)
		g.Nodes[rtid] = Node{ID: rtid, Kind: KindReleaseTopic, Label: rel.Prefix, State: StateActive}
		for _, pn := range rel.Partners {
			ptid := nodeID(KindPartner, pn)
			g.Nodes[ptid] = Node{ID: ptid, Kind: KindPartner, Label: pn, State: StateActive}
			g.Edges = append(g.Edges, Edge{
				From: ptid, To: rtid, Kind: EdgeMayReceiveRel, Layer: "release",
				Reason: "release-set " + rel.Set + " member",
			})
		}
	}

	sort.SliceStable(g.Edges, func(i, j int) bool {
		if g.Edges[i].From != g.Edges[j].From {
			return g.Edges[i].From < g.Edges[j].From
		}
		if g.Edges[i].To != g.Edges[j].To {
			return g.Edges[i].To < g.Edges[j].To
		}
		return g.Edges[i].Kind < g.Edges[j].Kind
	})
	return g
}

func (g *Graph) has(id string) bool { _, ok := g.Nodes[id]; return ok }

// reconcileIdentity decides an identity's declared-vs-actual state. With no
// roster supplied we can't tell cut-off from never-onboarded, so we report
// Unknown rather than guess. Disabled (Ory-inactive) but declared-active =
// cut off. Declared-revoked = cut off. Active in both = active.
func reconcileIdentity(id IdentityIn, declared []RosterEntry, rosterSupplied bool) ReconState {
	if !rosterSupplied {
		if id.Disabled {
			return StateCutOff // Ory itself says inactive — unambiguous
		}
		return StateUnknown
	}
	var dec *RosterEntry
	for i := range declared {
		// match a person roster entry to this identity by email-handle or name
		if declared[i].Type != "person" {
			continue
		}
		if dec == nil {
			dec = &declared[i]
		}
		// A re-onboarded person carries BOTH their old revoked entry and a
		// fresh active one (same email; the registry deprecates, never
		// deletes). The active entry is their current identity — first-match
		// here let the revoked ghost win and reported a live, freshly minted
		// admin as cut_off.
		if declared[i].Status != "revoked" {
			dec = &declared[i]
			break
		}
	}
	switch {
	case dec == nil:
		return StateOutOfBand // live identity with no declared roster entry
	case dec.Status == "revoked", id.Disabled:
		return StateCutOff
	default:
		return StateActive
	}
}

// classifyStrayIdentity explains why a live Ory identity reconciled to
// StateOutOfBand: no org key at all (neither the legacy vendor tag nor the
// Phase D aff claim) = untracked; a key that matches a declared org (but no
// person member of it) = offboarding residue; keys that match no declared org
// = an unregistered key (naming drift or offboarded org — the "don't
// blind-cut" class).
func classifyStrayIdentity(id IdentityIn, declaredByOrg, declaredByAffOrg map[string][]RosterEntry) ReconReason {
	if id.Vendor == "" && id.AffID == "" {
		return ReasonUntracked
	}
	if _, ok := declaredByOrg[id.Vendor]; ok && id.Vendor != "" {
		return ReasonNoDeclaredMember
	}
	if _, ok := declaredByAffOrg[id.AffID]; ok && id.AffID != "" {
		return ReasonNoDeclaredMember
	}
	return ReasonUnregisteredKey
}

// aliasBase strips one trailing re-issue suffix (`-<digits>`) from a slot
// alias: "jane-doe-2" → "jane-doe". Anything else returns unchanged — the
// suffix convention is a single numeric leg, so "team-x1" and bare tags
// pass through.
func aliasBase(alias string) string {
	i := strings.LastIndexByte(alias, '-')
	if i <= 0 || i == len(alias)-1 {
		return alias
	}
	for _, c := range alias[i+1:] {
		if c < '0' || c > '9' {
			return alias
		}
	}
	return alias[:i]
}

// MatchPeerHandle resolves a peer name to the onboarding handle by longest-key
// match against the declared roster — the same matcher the peer→vendor join
// uses, keyed on the person handle instead of the org. The grammar lives in
// internal/peername (generated from ops/identity/peer-name-grammar.yaml). This
// is the deterministic person↔peer custody link
// (docs/design/peer-identity-resolution.md, Option B): peer.user_id is empty by
// NetBird design under setup-key enrollment, so the name IS the link. Shared by
// the /api/access/peer resolver and the graph's person↔peer attribution.
func MatchPeerHandle(peerName string, roster []RosterEntry) string {
	handles := make([]string, 0, len(roster))
	for _, m := range roster {
		handles = append(handles, m.Handle)
	}
	return peername.MatchLongestKey(peerName, handles)
}

// MatchMachineHandle resolves a `machine-<handle>` peer name to the enrolled
// machine it was minted for. The portal mints the mesh setup key as
// "machine-"+handle (api/machine_enroll.go) and NetBird appends "-<hostname>",
// so `machine-<handle>` and `machine-<handle>-<anything>` both match, and the
// longest handle wins so `sam-baxter-1` is never claimed by `sam-baxter`.
// ops/ci/check-node-class-membership.sh grades alignment on the same forms. The
// `machine-` form is not in ops/identity/peer-name-grammar.yaml, which covers
// only operator-onboarded `peer-` names.
func MatchMachineHandle(peerName string, machines []MachineIn) string {
	rest, ok := strings.CutPrefix(peerName, "machine-")
	if !ok {
		return ""
	}
	best := ""
	for _, m := range machines {
		h := m.SystemHandle
		if len(h) <= len(best) {
			continue
		}
		if rest == h || strings.HasPrefix(rest, h+"-") {
			best = h
		}
	}
	return best
}

// classifyStrayPeer explains why a live mesh peer reconciled to StateOutOfBand:
// a name that follows the peer-name grammar but matched no registered vendor
// carries an org key that resolves to nothing (unregistered-key — reconcilable
// or offboarded, don't blind-cut); any other name carries no org key at all
// (untracked — a genuine unmanaged peer).
func classifyStrayPeer(name string) ReconReason {
	if peername.IsPeerName(name) {
		return ReasonUnregisteredKey
	}
	return ReasonUntracked
}

// affFromDeclared returns the org-level affiliation (kind, nation) for a set of
// declared members. Affiliation is an org property, so every member of an org
// shares it; the first member carrying a non-empty value wins. Empty when no
// declared member carried affiliation (an honest absence, not a guess).
func affFromDeclared(declared []RosterEntry) (kind, nation string) {
	for _, d := range declared {
		if kind == "" {
			kind = d.Affiliation
		}
		if nation == "" {
			nation = d.Nation
		}
		if kind != "" && nation != "" {
			break
		}
	}
	return kind, nation
}

// prefixCovers reports whether a read-tier prefix covers a topic key. "**"
// is the flat community-read tier (covers everything); otherwise the topic
// key must sit under the prefix.
func prefixCovers(tierPrefix, topicKey string) bool {
	if tierPrefix == "**" || tierPrefix == "" {
		return true
	}
	tp := tierPrefix
	// strip a trailing "/**" or "/*" glob for prefix comparison
	for _, suf := range []string{"/**", "/*", "**", "*"} {
		if len(tp) >= len(suf) && tp[len(tp)-len(suf):] == suf {
			tp = tp[:len(tp)-len(suf)]
			break
		}
	}
	tp = trimTrailingSlash(tp)
	if tp == "" {
		return true
	}
	return topicKey == tp || (len(topicKey) > len(tp) && topicKey[:len(tp)+1] == tp+"/")
}

func trimTrailingSlash(s string) string {
	for len(s) > 0 && s[len(s)-1] == '/' {
		s = s[:len(s)-1]
	}
	return s
}

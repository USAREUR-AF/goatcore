package access

// dataroles.go is the registry data-roles resolver (ADR 1074 build item 1,
// docs/design/data-access-roles.md §5–§6). It answers, for one principal,
// the DERIVED data-access role set: the sentences in the ADR 1074 grammar
// (data.view.org:<O>, data.view.producer:<K>, data.write.producer:<K>) that
// the registry confers by construction — org membership, producer custody,
// and the vertical org-hierarchy roll-up — PLUS the expanded producer-key set
// those org grants cover on a seed-model goatnet (org → member producers).
//
// It serves ONLY registry-derived roles. Explicit claim-carried grants ride
// the Ory `groups` claim and the notional default is a per-goatnet flag;
// neither is served here (design doc §5: "claims carry explicit extras, the
// automatic majority derives"). Grant rows (ADR 1075) have no registry store
// yet — when one lands, active rows within their effective window are the
// named extension point (see resolveGrantRows below and the ADR 1074 addendum).
//
// PRINCIPAL-KEY DISCIPLINE (ADR 1051 spine + ADR 1069): a human principal is
// keyed on the identity UUID; the roster join uses the tag-independent
// email-hash (the F-234 key, the same one the COP reconciles on). Email is a
// lookup alias resolved to the UUID/hash by the caller — never stored or
// joined here. A machine principal is keyed on its producer (write anchor).
//
// This file is PURE: the loader parses the committed roster.json projection
// and the resolvers are deterministic functions over it, no I/O beyond the
// one file read. The portal handler (internal/api/data_roles.go) does the
// live resolution (Kratos id ↔ email, vendors-registry existence) and calls
// these.

import (
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"sort"
	"strings"
	"time"
)

// DataRoleResult is the derived, registry-only role set for one principal.
// Roles are ADR 1074 grammar sentences; Producers is the expanded producer-key
// set the org/producer grants cover (org → member producers, transitively
// through vertical parent containment). Notes carry honesty caveats (an
// identity with no registry membership, a producer with no declared owning
// org) so an empty result is never mistaken for a denial.
type DataRoleResult struct {
	Roles     []string `json:"roles"`
	Producers []string `json:"producers"`
	// CoPublish lists the MULTI-PUBLISHER TOPICS this principal may publish to
	// (ADR 1105), as `multipub/<pool>/<topic>` key-bases. It is a distinct
	// membership, NOT a producer write role — a multi-publisher topic is not a
	// producer, and single-owner roots are never here.
	CoPublish []string `json:"co_publish,omitempty"`
	Notes     []string `json:"notes,omitempty"`
}

// DataRoster is the resolver's registry projection: the org hierarchy (vertical
// parent containment, ADR 1043), each org's declared member producers, the
// person→membership index keyed on email-hash, and the producer→owning-orgs
// index. Built once by LoadDataRoster; the resolvers are pure reads over it.
type DataRoster struct {
	// orgParent maps an org id to its vertical parent org id ("" = root). Only
	// this containment edge rolls up data view; lateral/time-bounded edges
	// (comrel, supports, data-authority) are deliberately not represented here.
	orgParent map[string]string
	// orgChildren is the inverse: parent org id → its direct child org ids.
	orgChildren map[string][]string
	// orgProducers maps an org id to the distinct producer keys (member
	// vendor_tags) declared directly under it — its OWN producers, before roll-up.
	orgProducers map[string][]string
	// membershipsByEmailHash maps a person's email-hash to the (org, producer)
	// rows they appear in. Multi-org membership yields multiple rows (union).
	membershipsByEmailHash map[string][]dataMembership
	// orgPersonMembers maps an org id to its active PERSON members in roster
	// order (email-hash + handle). Person = a member carrying an email-hash;
	// machines carry none. Roster order is preserved so "first member" (the
	// new-org auto-seed anchor) is deterministic, and the full slice is the
	// all-members set the one-time cutover seed nominates. Governance admins are
	// person-keyed (nomination stores a UUID), so machines never appear here.
	orgPersonMembers map[string][]OrgPersonMember
	// producerOrgs maps a producer key to the org ids that declare it (its
	// owning orgs — usually one, but the seed model can list a producer under
	// both its unit org and its own vendor org).
	producerOrgs map[string][]string
	// orgNation maps an org id to the nation that OWNS it, and orgComposition to
	// `national` or `multinational` (ADR 1171). Absent = undeclared, and an
	// undeclared org cannot answer the nationality intersection, so it wraps to
	// nobody rather than to everybody.
	orgNation      map[string]string
	orgComposition map[string]string
	// producerNationality maps a producer key to the OWN nationality of the
	// person holding it — from their manifest, never inherited from their org.
	// Absent = undeclared.
	producerNationality map[string]string
	// knownProducers is the set of producer keys the roster declares.
	knownProducers map[string]bool
	// Freshness carries the roster artifact's provenance stamp for the response.
	Freshness *RosterFreshness
}

// dataMembership is one person↔org membership row: the org the person belongs
// to and the producer key (vendor_tag) they carry there ("" when none).
type dataMembership struct {
	Org      string
	Producer string
}

// OrgPersonMember is one active person member of an org, in roster order. The
// governance cutover reads these: the first entry is the new-org auto-seed
// anchor; the full slice is the one-time all-members migration set.
type OrgPersonMember struct {
	Handle    string
	EmailHash string
}

// dataRosterFile mirrors the orgs block of docs/operators/roster.json (the
// ADR 1043 affiliation graph with vertical parent edges + per-org members).
// The resolver reads the orgs block, not the flat top-level members list, so it
// has the org grouping the org→producer roll-up needs.
type dataRosterFile struct {
	GeneratedAt  string `json:"generated_at"`
	SourceDigest string `json:"source_digest"`
	Orgs         []struct {
		ID     string  `json:"id"`
		Parent *string `json:"parent"`
		// Nation is the nation that OWNS the organisation, and Composition says
		// whether that means one sovereign or a body of them (ADR 1171). Both
		// may be absent — an undeclared org yields no wrap slot rather than a
		// guess.
		Nation      string `json:"nation"`
		Composition string `json:"composition"`
		Members     []struct {
			Handle    string  `json:"handle"`
			Type      string  `json:"type"`
			State     string  `json:"state"`
			VendorTag *string `json:"vendor_tag"`
			EmailHash string  `json:"email_hash"`
			// Nationality is the PERSON's own citizenship, never the org's.
			// Empty means UNDECLARED, which is not the same as "matches".
			Nationality string `json:"nationality"`
		} `json:"members"`
	} `json:"orgs"`
}

// LoadDataRoster reads the committed roster.json projection at path and indexes
// it for resolution. A missing/unreadable/malformed file is an error to the
// caller (fail-closed: a silently-empty roster must not read as "nobody has any
// role" — that is a denial, not an absence).
func LoadDataRoster(path string) (*DataRoster, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read data-roster %s: %w", path, err)
	}
	return parseDataRoster(path, raw)
}

// parseDataRoster indexes the bytes of a roster.json read from path (path is
// only for error messages). Split from LoadDataRoster so DataRosterCache can
// skip the parse when the bytes are unchanged.
func parseDataRoster(path string, raw []byte) (*DataRoster, error) {
	var rf dataRosterFile
	if err := json.Unmarshal(raw, &rf); err != nil {
		return nil, fmt.Errorf("parse data-roster %s: %w", path, err)
	}
	dr := &DataRoster{
		orgParent:              map[string]string{},
		orgChildren:            map[string][]string{},
		orgProducers:           map[string][]string{},
		membershipsByEmailHash: map[string][]dataMembership{},
		orgPersonMembers:       map[string][]OrgPersonMember{},
		producerOrgs:           map[string][]string{},
		knownProducers:         map[string]bool{},
		orgNation:              map[string]string{},
		orgComposition:         map[string]string{},
		producerNationality:    map[string]string{},
	}
	for _, o := range rf.Orgs {
		if o.ID == "" {
			continue
		}
		parent := ""
		if o.Parent != nil {
			parent = *o.Parent
		}
		// A parent that points at itself is a mislabeled root, not a cycle —
		// drop the self-edge so the roll-up BFS never spins on it.
		if parent == o.ID {
			parent = ""
		}
		dr.orgParent[o.ID] = parent
		if parent != "" {
			dr.orgChildren[parent] = appendUnique(dr.orgChildren[parent], o.ID)
		}
		if n := strings.TrimSpace(o.Nation); n != "" {
			dr.orgNation[o.ID] = normalizeNation(n)
		}
		if c := strings.TrimSpace(o.Composition); c != "" {
			dr.orgComposition[o.ID] = c
		}
		for _, m := range o.Members {
			// Offboarding freshness (WS4): a member whose roster state is
			// inactive (revoked / suspended / offboarded / departed / disabled)
			// is NOT a wrap target — skip it entirely so it contributes no
			// producer anchor, no org roll-up, and no person membership. Because
			// the roster is loaded per request, marking a member inactive in the
			// roster cuts its wrap access on the next audience derivation. Empty
			// or unrecognized state is treated as active (fail-open on the label
			// only, never on an explicit inactive state).
			if !isActiveMemberState(m.State) {
				continue
			}
			producer := ""
			if m.VendorTag != nil {
				producer = strings.TrimSpace(*m.VendorTag)
			}
			if producer != "" {
				dr.orgProducers[o.ID] = appendUnique(dr.orgProducers[o.ID], producer)
				dr.producerOrgs[producer] = appendUnique(dr.producerOrgs[producer], o.ID)
				dr.knownProducers[producer] = true
				// The holder's OWN nationality, from their manifest. Never the
				// org's — that inheritance is the thing ADR 1171 deleted.
				if nat := strings.TrimSpace(m.Nationality); nat != "" {
					dr.producerNationality[producer] = normalizeNation(nat)
				}
			}
			if m.EmailHash != "" {
				dr.membershipsByEmailHash[m.EmailHash] = append(
					dr.membershipsByEmailHash[m.EmailHash],
					dataMembership{Org: o.ID, Producer: producer},
				)
				// Person member (carries an email-hash), captured in roster
				// order for the governance cutover: first = new-org anchor,
				// full slice = all-members migration set.
				dr.orgPersonMembers[o.ID] = append(
					dr.orgPersonMembers[o.ID],
					OrgPersonMember{Handle: strings.TrimSpace(m.Handle), EmailHash: m.EmailHash},
				)
			}
		}
	}
	dr.Freshness = LoadRosterFreshness(path)
	return dr, nil
}

// isActiveMemberState reports whether a roster member's state makes it a live
// participant. Empty/unknown = active (the roster's common case omits state for
// active rows); the explicit inactive labels are excluded. Offboarding marks a
// member with one of these to cut its wrap access (WS4).
func isActiveMemberState(state string) bool {
	switch strings.ToLower(strings.TrimSpace(state)) {
	case "revoked", "suspended", "offboarded", "departed", "disabled", "inactive":
		return false
	default:
		return true
	}
}

// KnownProducer reports whether the roster declares this producer key. The
// handler consults it (plus the live vendors registry) to fail closed on an
// anchor that names no producer.
func (dr *DataRoster) KnownProducer(key string) bool { return dr.knownProducers[key] }

// MachineAnchorsForPerson returns the machine anchors (producer keys) a person
// carries across their org memberships — the wrap targets a PRINCIPAL grant to
// that person must reach (WS4). A person with no producer identity anywhere
// returns empty (a browser-only reader — noted, not wrapped). The caller
// resolves the identity UUID → email → email-hash; the hash is the join key.
func (dr *DataRoster) MachineAnchorsForPerson(emailHash string) []string {
	set := map[string]bool{}
	for _, m := range dr.membershipsByEmailHash[emailHash] {
		if m.Producer != "" {
			set[m.Producer] = true
		}
	}
	return sortedKeys(set)
}

// RolesForPerson derives the registry role set for a human principal identified
// by its email-hash (the caller resolves the identity UUID → email → hash; the
// hash is the join key, never the email itself). Membership → data.view.org:<O>;
// the person's own producer(s) → data.view.producer:<K> (custody); the expanded
// producer set is the union of every membership org's producers rolled up
// vertically (org → member producers, transitively through parent containment).
// Multi-org membership is a union. A person with no membership yields an empty
// role set with an honest note — not an error.
func (dr *DataRoster) RolesForPerson(emailHash string) DataRoleResult {
	res := DataRoleResult{Roles: []string{}, Producers: []string{}}
	rows := dr.membershipsByEmailHash[emailHash]
	if len(rows) == 0 {
		res.Notes = append(res.Notes,
			"no registry membership found for this identity — registry-derived roles are empty (explicit claim grants and the notional default are not served by this endpoint)")
		return res
	}
	roleSet := map[string]bool{}
	prodSet := map[string]bool{}
	for _, m := range rows {
		roleSet["data.view.org:"+m.Org] = true
		// NOTE (ADR 1102): governance authority (data.govern.org:<O>) is NOT
		// emitted here. It is computed in ONE place at the handler layer
		// (Server.governedOrgs) from the nomination store + the per-goatnet
		// bootstrap-open default, because it depends on state this pure roster
		// projection does not see (who is nominated). The org MEMBERSHIP this
		// loop records (data.view.org:<O>) is the input that computation reads.
		for _, p := range dr.expandOrg(m.Org) {
			prodSet[p] = true
		}
		if m.Producer != "" {
			roleSet["data.view.producer:"+m.Producer] = true
			prodSet[m.Producer] = true
		}
	}
	res.Roles = sortedKeys(roleSet)
	res.Producers = sortedKeys(prodSet)
	return res
}

// RolesForAnchor derives the registry role set for a machine principal
// identified by its producer key (write anchor). The write and self-view roles
// are held by construction (ADR 1074 G3: the cert anchored at K holds
// data.write.producer:K; custody of K holds data.view.producer:K). The org
// roll-up derives from the roster: the anchor's owning org(s) confer
// data.view.org:<O> and expand to that org's producers (transitively down).
// An anchor the roster does not place under any org still gets its by-construction
// roles, with a note that no org roll-up applies.
func (dr *DataRoster) RolesForAnchor(anchor string) DataRoleResult {
	res := DataRoleResult{Roles: []string{}, Producers: []string{}}
	roleSet := map[string]bool{
		"data.write.producer:" + anchor: true,
		"data.view.producer:" + anchor:  true,
	}
	prodSet := map[string]bool{anchor: true}
	owningOrgs := dr.producerOrgs[anchor]
	for _, o := range owningOrgs {
		roleSet["data.view.org:"+o] = true
		// NOTE (ADR 1102): governance authority is NOT emitted here — see
		// RolesForPerson. The machine's org MEMBERSHIP (data.view.org:<O>) is
		// the input; Server.governedOrgs decides whether that confers govern.
		for _, p := range dr.expandOrg(o) {
			prodSet[p] = true
		}
	}
	if len(owningOrgs) == 0 {
		res.Notes = append(res.Notes,
			"producer has no declared owning org in the roster projection — no org roll-up applied (write + self-view are held by construction)")
	}
	res.Roles = sortedKeys(roleSet)
	res.Producers = sortedKeys(prodSet)
	return res
}

// expandOrg returns the producer keys visible under an org grant: the org's own
// producers plus every descendant org's producers, walking vertical parent
// containment DOWNWARD (holding data.view.org:<parent> covers child orgs — the
// rollup rule). Cycle-guarded so a mis-declared parent loop can't spin.
func (dr *DataRoster) expandOrg(org string) []string {
	seen := map[string]bool{}
	out := map[string]bool{}
	var walk func(string)
	walk = func(o string) {
		if seen[o] {
			return
		}
		seen[o] = true
		for _, p := range dr.orgProducers[o] {
			out[p] = true
		}
		for _, c := range dr.orgChildren[o] {
			walk(c)
		}
	}
	walk(org)
	return sortedKeys(out)
}

// KnownOrg reports whether the roster declares org: as an org row, as the org of
// a producer, or as the org of a person member. An org grant naming anything
// else wraps to nobody, so grant creation refuses it.
func (dr *DataRoster) KnownOrg(org string) bool {
	if _, ok := dr.orgParent[org]; ok {
		return true
	}
	return len(dr.orgProducers[org]) > 0 || len(dr.orgPersonMembers[org]) > 0
}

// ProducersUnderOrg is the exported org→member-anchor projection the audience
// resolver (grant-keyed encryption §3a) needs: the producer keys an org grant
// opens — the org's own producers plus every descendant's, the same downward
// rollup expandOrg computes. The machine-plane audience of an org grantee is
// exactly this set (each producer key is a member anchor the DEK is wrapped to);
// members with no producer key are browser-only readers, not machine-plane wrap
// targets, so they are correctly absent here.
func (dr *DataRoster) ProducersUnderOrg(org string) []string { return dr.expandOrg(org) }

// OrgNationality reports the nation that OWNS org, whether that ownership is
// `national` or `multinational`, and whether the org declared either at all.
// An org that declared neither cannot answer the intersection below, and the
// caller must fail closed rather than guess.
func (dr *DataRoster) OrgNationality(org string) (nation, composition string, declared bool) {
	nation = dr.orgNation[org]
	composition = dr.orgComposition[org]
	return nation, composition, nation != ""
}

// ProducerNationality reports the OWN nationality of the person holding the
// producer key. The second result is false when the holder has not declared
// one — which is not the same as "it matches their org", and is the distinction
// the whole of ADR 1171 rests on.
func (dr *DataRoster) ProducerNationality(producer string) (string, bool) {
	n, ok := dr.producerNationality[producer]
	return n, ok
}

// MayReceiveOrgMaterial reports whether the holder of producer earns a wrap slot
// for material addressed to org, and why not when it does not.
//
// THE RULE (operator-ratified backlog `grant-audience-nationality-
// intersection`): membership in the org AND the holder's own nationality being
// one of the nations that own it. For a `national` org that is the owning nation
// itself; for a `multinational` one it is membership of the body the org is
// constituted from — a NATO-owned organisation's material reaches the citizens
// of NATO nations within it, which is what "owned by NATO" means.
//
// `affiliation.relationship` is deliberately NOT consulted. A Canadian at the
// US-owned usareur-af is excluded because CAN is not USA, and an American at the
// same command is included for the same reason. Testing the relationship field
// instead would exclude the American too — ADR 1171 makes them both `assigned` —
// and empty the organisation.
//
// FAIL-CLOSED ON ABSENCE, both directions. An org that declares no nation, and a
// holder who declares no nationality, each yield no wrap slot. Absence is not a
// match; a guess here is a silent disclosure.
func (dr *DataRoster) MayReceiveOrgMaterial(producer, org string, fw *CoalitionFrameworks, asOf time.Time) (bool, string) {
	ok, _, why := dr.ClassifyOrgMaterial(producer, org, fw, asOf)
	return ok, why
}

// WrapRefusal classifies WHY a member anchor earns no wrap slot. The sentence
// MayReceiveOrgMaterial returns is for a human reading one anchor; this is the
// same verdict as a value, so a caller can COUNT refusals by kind.
//
// The distinction the whole preview rests on is Excluded vs the two Undeclared
// kinds. "Nobody qualifies" and "nobody has declared" produce an identical zero
// and mean opposite things: the first is the policy working, the second is a
// registry that has not been filled in. A producer who reads an undeclared zero
// as an excluded zero concludes the restriction is tight when in fact it is
// unevaluable, which is the wrong call in the unsafe direction.
type WrapRefusal string

const (
	// WrapAdmitted — the holder earns a wrap slot.
	WrapAdmitted WrapRefusal = "admitted"
	// WrapOrgNationUndeclared — the ORG never declared an owning nation, so
	// there is nothing to match against. Fixed by the org registry, not by the
	// person.
	WrapOrgNationUndeclared WrapRefusal = "org_nation_undeclared"
	// WrapHolderNationalityUndeclared — the HOLDER never declared a nationality.
	// Fixed by their manifest (and by regenerating the roster projection), not
	// by the org.
	WrapHolderNationalityUndeclared WrapRefusal = "holder_nationality_undeclared"
	// WrapNationalityExcluded — both facts are declared and they do not
	// intersect. This is the rule doing its job, and the only refusal kind that
	// means the restriction is actually holding.
	WrapNationalityExcluded WrapRefusal = "nationality_excluded"
	// WrapFrameworkUnavailable — a multinational org whose body cannot be
	// expanded because no coalition framework is loaded. A deployment fault,
	// not a fact about anyone.
	WrapFrameworkUnavailable WrapRefusal = "framework_unavailable"
)

// ClassifyOrgMaterial is MayReceiveOrgMaterial with the verdict as a value.
// ONE implementation of the rule: the audience resolver that decides real wrap
// slots and the pre-publish preview both call this, so the preview cannot drift
// from the thing it previews. A preview computed by a second copy of the rule
// would be worse than no preview at all — it would be confidently wrong exactly
// when the two copies disagreed.
func (dr *DataRoster) ClassifyOrgMaterial(producer, org string, fw *CoalitionFrameworks, asOf time.Time) (bool, WrapRefusal, string) {
	nation, composition, declared := dr.OrgNationality(org)
	if !declared {
		return false, WrapOrgNationUndeclared,
			"the organisation declares no owning nation, so no holder can be matched against it"
	}
	held, ok := dr.ProducerNationality(producer)
	if !ok {
		return false, WrapHolderNationalityUndeclared,
			"the holder declares no nationality — undeclared is not a match"
	}
	if composition == "multinational" {
		if fw == nil {
			return false, WrapFrameworkUnavailable,
				"the organisation is multinational and no coalition framework is loaded to expand " + nation
		}
		if fw.isMember(nation, held, asOf) {
			return true, WrapAdmitted, ""
		}
		return false, WrapNationalityExcluded,
			held + " is not a member of " + nation + " as of this date"
	}
	// `national`, or an undeclared composition, which is treated as national:
	// the narrower reading is the safe one when the axis was never declared.
	if held == nation {
		return true, WrapAdmitted, ""
	}
	return false, WrapNationalityExcluded, held + " is not " + nation
}

// OrgsOfProducer returns the org ids that declare a producer key (its owning
// orgs — usually one). The org data-governance gate (ADR 1102) uses it to decide
// whether a caller may grant for a producer: a caller governs a producer when the
// producer's owning org is one the caller's own producer is also declared under
// (same-org authority — the bootstrapped `data.govern.org:<O>`).
func (dr *DataRoster) OrgsOfProducer(key string) []string {
	// A copy: a DataRoster may be shared across requests (DataRosterCache).
	return slices.Clone(dr.producerOrgs[key])
}

// OrgPersonMembers returns an org's active person members in roster order. The
// governance cutover (ADR 1102) reads it two ways: the first entry is the
// new-org auto-seed anchor (the org's first onboardee, who governs by default
// until the org delegates), and the full slice is the one-time all-members
// migration set. Returns nil for an unknown org or one with only machine
// members. The slice is a copy: a DataRoster may be shared across requests
// (DataRosterCache), so no caller may hold its backing array.
func (dr *DataRoster) OrgPersonMembers(org string) []OrgPersonMember {
	return slices.Clone(dr.orgPersonMembers[org])
}

// OrgsWithPersonMembers returns every org id that has at least one active person
// member, sorted for deterministic iteration. The cutover reconcile walks it to
// find zero-admin orgs needing a first-member seed.
func (dr *DataRoster) OrgsWithPersonMembers() []string {
	out := make([]string, 0, len(dr.orgPersonMembers))
	for org := range dr.orgPersonMembers {
		out = append(out, org)
	}
	sort.Strings(out)
	return out
}

// GrantRow is the minimal grant shape the resolver needs to fold a grant into a
// principal's derived roles: who it is for and which producer it opens. The
// portal handler supplies only IN-EFFECT rows (the registry grant store filtered
// by its own effective-window + revocation logic) converted to this shape, so
// the access package stays free of the registry/SQL dependency and the
// time/lifecycle logic lives with the store. GranteeKind is "org" | "principal".
type GrantRow struct {
	GranteeKind string
	GranteeID   string
	Producer    string
	// Action is "view" (ADR 1075 read grant — the default when empty) or
	// "write" (ADR 1105 co-publish grant). For a write grant Producer holds a
	// multipub-topic key-base (`multipub/<pool>/<topic>`) and it folds into the
	// principal's CoPublish membership list, not a producer view/write role.
	Action string
}

// OrgsFromRoles pulls the org ids out of a derived role set — the
// data.view.org:<O> sentences the roster resolver produced. It is how the grant
// merge learns which orgs a principal belongs to without a second roster query
// (the org membership is already encoded in the roles).
func OrgsFromRoles(roles []string) []string {
	const p = "data.view.org:"
	out := []string{}
	for _, r := range roles {
		if strings.HasPrefix(r, p) {
			out = append(out, strings.TrimPrefix(r, p))
		}
	}
	return out
}

// MergeGrants folds in-effect grant rows into a principal's derived role set —
// the ADR 1075 resolver join, and the named extension point the #1515 resolver
// left open. A principal grant (grantee principal:<id>) whose id equals the
// principal, or an org grant (grantee org:<O>) for an org the principal belongs
// to, yields data.view.producer:<producer> and adds that producer to the visible
// set. Grant rows are enumerable relations evaluated by reading them — no rule
// engine, no central PDP. principalID is the identity UUID (person) or the write
// anchor (machine); the org set derives from base.Roles. A new result is
// returned; base is not mutated. Notes about applied grants are appended so an
// expanded result is never mistaken for an ambient one.
func MergeGrants(base DataRoleResult, principalID string, grants []GrantRow) DataRoleResult {
	orgSet := map[string]bool{}
	for _, o := range OrgsFromRoles(base.Roles) {
		orgSet[o] = true
	}
	roleSet := map[string]bool{}
	prodSet := map[string]bool{}
	for _, r := range base.Roles {
		roleSet[r] = true
	}
	for _, p := range base.Producers {
		prodSet[p] = true
	}
	coPubSet := map[string]bool{}
	for _, c := range base.CoPublish {
		coPubSet[c] = true
	}
	applied := 0
	coPublish := 0
	for _, g := range grants {
		match := false
		switch g.GranteeKind {
		case "principal":
			match = principalID != "" && g.GranteeID == principalID
		case "org":
			// Org grants are view-only (write grants are principal-only, refused
			// at the store for an org grantee) — a write grant here would be
			// malformed; ignore it rather than widen an org to co-publish.
			match = g.Action != "write" && orgSet[g.GranteeID]
		}
		if !match || g.Producer == "" {
			continue
		}
		if g.Action == "write" {
			// ADR 1105 co-publish: the grantee gains publish access to a
			// registered MULTI-PUBLISHER TOPIC (Producer = its multipub/<pool>/<topic>
			// key-base). This is a distinct membership, NOT a producer role —
			// single-owner roots are never opened. The ACL render carries the
			// enforced key-expr; the COP surfaces the membership.
			coPubSet[g.Producer] = true
			coPublish++
			applied++
			continue
		}
		roleSet["data.view.producer:"+g.Producer] = true
		prodSet[g.Producer] = true
		applied++
	}
	res := DataRoleResult{
		Roles:     sortedKeys(roleSet),
		Producers: sortedKeys(prodSet),
		CoPublish: sortedKeys(coPubSet),
		Notes:     append([]string(nil), base.Notes...),
	}
	if applied > 0 {
		res.Notes = append(res.Notes,
			fmt.Sprintf("%d in-effect grant row(s) applied — the producer view(s) above include ADR 1075 grants, not only registry-derived roles", applied))
	}
	if coPublish > 0 {
		res.Notes = append(res.Notes,
			fmt.Sprintf("%d of those are ADR 1105 co-publish grant(s) — this principal may also publish to the named MULTI-PUBLISHER topic(s) (co_publish), a separate, marked class; single-owner roots are untouched", coPublish))
	}
	return res
}

func appendUnique(s []string, v string) []string {
	for _, x := range s {
		if x == v {
			return s
		}
	}
	return append(s, v)
}

func sortedKeys(m map[string]bool) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

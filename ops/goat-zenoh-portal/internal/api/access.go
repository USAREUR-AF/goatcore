package api

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/kratosadmin"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/peername"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// access.go wires the Data-Access COP read API (ADR 1026, COP Phase 1 /
// goat-cli Phase G.4). It adapts the portal's live readers — the topic /
// vendor registry, the Kratos identity admin, and the Netbird peer poller
// — into an access.Sources snapshot, assembles the cross-layer graph, and
// answers the forward (who) and reverse (topic) queries.
//
// Admin-only: the COP reads across every identity layer, so both routes
// are operator-gated (requireOperator), per data-access-cop.md §"Security
// of the access picture itself".
//
// Freshness: each request collects a fresh snapshot (no cache) — simplest
// correct model for Phase 1; a short per-layer cache is a documented
// open question (data-access-cop.md §"Data freshness").

// collectAccessSources reads the live layers into an access.Sources
// snapshot. It returns best-effort warnings for any layer that is
// unconfigured or empty so the caller can surface the gap rather than
// silently under-report. Writes are modeled precisely (the F-177 half);
// flat community-read and the not-yet-built slot-pool / release-set
// layers are represented as notes, not fabricated edges.
func (s *Server) collectAccessSources(ctx context.Context) (access.Sources, []access.Note) {
	var src access.Sources
	var warn []access.Note
	// Each note is tagged where it is written, by the code that knows whether it
	// is a failure or a standing statement — the caller then never has to infer
	// severity from the prose (audit follow-on: the roster's notes[] was being
	// string-matched for "unavailable" / "unreachable" / "error:").
	fail := func(kind, text string) access.Note {
		return access.Note{Text: text, Kind: kind, Severity: access.SeverityError}
	}
	degraded := func(kind, text string) access.Note {
		return access.Note{Text: text, Kind: kind, Severity: access.SeverityWarn}
	}
	doctrine := func(kind, text string) access.Note {
		return access.Note{Text: text, Kind: kind, Severity: access.SeverityInfo}
	}

	vendors, err := s.reg.ListVendors(ctx)
	if err != nil {
		return src, []access.Note{fail("registry_unavailable", "registry unavailable: "+err.Error())}
	}
	// Assigned (live, non-revoked) slot anchors. On a per-uuid-slot-pool goatnet
	// the router ACL pre-bakes a `<anchor>/**` grant for every slot, so a
	// slot-pool vendor (prefix == its slot anchor) CAN write even though its
	// vendors row carries no ACLRenderedAt stamp (there is no per-onboard render
	// — onboardSlotSystem creates the row approved, not rendered). Without this
	// the whole roster reports 0 writes. Empty on per-vendor goatnets.
	assignedAnchor := map[string]bool{}
	aliasByAnchor := map[string]string{}
	// The recorded person→slot linkage (slots.identity_id). Where it exists the
	// graph attributes ownership from it instead of matching names.
	identityByAnchor := map[string]string{}
	if aliases, aerr := s.reg.ListAliases(ctx); aerr != nil {
		warn = append(warn, fail("slot_aliases_unavailable", "slot aliases unavailable: "+aerr.Error()))
	} else {
		for _, a := range aliases {
			assignedAnchor[a.SlotID] = true
			aliasByAnchor[a.SlotID] = a.Alias
			if a.IdentityID != "" {
				identityByAnchor[a.SlotID] = a.IdentityID
			}
		}
	}
	for _, v := range vendors {
		anchor := ""
		// A vendor can write once EITHER its per-prefix rule is rendered into
		// the router ACL (seed model — ACLRenderedAt stamped; the write anchor
		// IS the vendor prefix) OR it holds a live slot whose pre-baked
		// `<anchor>/**` grant already admits it (slot-pool model). A vendor with
		// neither is approved-but-can't-write — the silent-denial gap the COP
		// surfaces.
		if v.ACLRenderedAt != nil || assignedAnchor[v.Prefix] {
			anchor = v.Prefix
		}
		src.Vendors = append(src.Vendors, access.VendorIn{
			Prefix: v.Prefix, DisplayName: v.DisplayName, WriteAnchor: anchor,
			Alias:      aliasByAnchor[v.Prefix],
			IdentityID: identityByAnchor[v.Prefix],
		})
		topics, terr := s.reg.ListVendorTopics(ctx, v.Prefix)
		if terr != nil {
			warn = append(warn, fail("topics_unavailable", "topics for "+v.Prefix+" unavailable: "+terr.Error()))
			continue
		}
		for _, t := range topics {
			if t.RetiredAt != nil {
				continue // retired topics aren't live access surface
			}
			src.Topics = append(src.Topics, access.TopicIn{
				VendorPrefix: t.VendorPrefix, KeyPattern: t.KeyPattern,
			})
		}
	}

	// Identity layer (Ory / Kratos). nil client ⇒ portal built without
	// admin access; report the gap.
	if s.kratos == nil {
		warn = append(warn, fail("identity_layer_unavailable", "identity layer unavailable (kratos admin not configured) — persons + cut-off reconciliation omitted"))
	} else {
		ids, ierr := s.kratos.ListIdentities(ctx)
		if ierr != nil {
			warn = append(warn, fail("identity_layer_error", "kratos admin error: "+ierr.Error()))
		}
		for _, id := range ids {
			affID, affKind, affNation := id.Aff()
			src.Identities = append(src.Identities, access.IdentityIn{
				ID:          id.ID,
				Email:       id.Email(),
				Name:        id.Name(),
				Vendor:      id.VendorShortTag(),
				AffID:       affID,
				AffKind:     affKind,
				AffNation:   affNation,
				Nationality: identityNationality(id),
				Disabled:    id.State != "active",
				// Groups intentionally empty in PR-A: the model is flat
				// community-read (ADR 1025), surfaced as a note rather than
				// N×M per-topic read edges. Restricted read tiers plug in
				// here when they exist.
			})
		}
	}

	// Declared half — the issuance registry (roster.json). Empty path ⇒ the
	// COP runs without declared data and identities reconcile to StateUnknown
	// (can't distinguish cut-off from never-onboarded); report the gap rather
	// than silently under-reconcile. A configured-but-unreadable file is a
	// warning, not a fabricated empty roster. Identity-grammar Phase B.
	if s.cfg.RosterPath == "" {
		warn = append(warn, fail("declared_registry_unavailable", "declared registry unavailable (roster path not configured) — identity reconciliation degrades to unknown (cut-off vs never-onboarded indistinguishable)"))
	} else if roster, rerr := access.LoadRoster(s.cfg.RosterPath); rerr != nil {
		warn = append(warn, fail("declared_registry_error", "declared registry load error: "+rerr.Error()))
	} else {
		src.Roster = roster
	}

	// Machines the portal enrolled (ADR 1096). The roster above is built from
	// operator manifests and never carries them, so this is the only layer that
	// can say who answers for a `machine-<handle>` peer. Degraded, not fatal:
	// people still resolve without it.
	if machines, merr := s.reg.AllEnrolledMachines(ctx); merr != nil {
		warn = append(warn, degraded("machines_unavailable",
			"enrolled machines unavailable: "+merr.Error()+" — machine peers resolve to no custodian"))
	} else {
		for _, m := range machines {
			src.Machines = append(src.Machines, access.MachineIn{
				SystemHandle: derefStr(m.AssignedTo),
				Custodian:    derefStr(m.EnrolledBy),
				NodeClass:    derefStr(m.NodeClass),
				SlotID:       m.SlotID,
				Affiliation:  derefStr(m.Affiliation),
			})
		}
	}

	// Mesh layer (Netbird). Served through the shared cache-backed client so a
	// slow/unreachable netbird-mgmt degrades the COP to a stale-with-warning
	// snapshot instead of serializing its full latency onto every access call.
	// `enabled=false` = no mgmt configured (the honest "unavailable" case).
	peers, nbWarn, nbEnabled := s.nb.PeersForAccess(ctx)
	if !nbEnabled {
		warn = append(warn, fail("mesh_layer_unavailable", "mesh layer unavailable (netbird mgmt URL/token not configured) — peers + on-mesh edges omitted"))
	} else {
		if nbWarn != "" {
			warn = append(warn, degraded("mesh_stale", nbWarn))
		}
		for _, p := range peers {
			src.Peers = append(src.Peers, access.PeerIn{
				ID: p.ID, Name: p.Name, Vendor: matchPeerVendor(p.Name, vendors),
				Connected: p.Connected, LastSeenUnix: p.LastSeenUnix(),
			})
		}
	}

	// Onboarding stage, anchor-keyed. The COP is "who is here and what
	// are they doing"; without this it cannot distinguish someone quiet because
	// they finished arriving and stopped, from someone quiet because they never
	// finished arriving. Carried as display state only — it grants nothing,
	// decides no verdict, and no edge is built from it. Degraded read is
	// non-fatal: the COP is still correct without the column, so a progress
	// failure must not take the whole roster down.
	if prog, perr := s.reg.ListProgress(ctx, true); perr != nil {
		warn = append(warn, degraded("progress_unavailable",
			"onboarding stage unavailable: "+perr.Error()+" — the roster is complete, the stage column is not"))
	} else {
		stages := make(map[string]string, len(prog))
		for _, p := range prog {
			stages[p.Prefix] = stageOf(p)
		}
		src.OnboardingStage = stages
	}

	// Read model. The trial fabric is flat community-read (ADR 1025): every
	// on-mesh identity may read all topics. That stays true until the B2.1
	// partition enforces — so FlatCommunityRead is reported regardless of
	// whether release sets are declared.
	src.FlatCommunityRead = true
	warn = append(warn, doctrine("write_model", "writes: a peer may publish ONLY its own anchor — the router enforces `<anchor>/**` (and its `volatile/` mirror) per-UUID, deny-by-default (ADR 1025 slot-write rules), and that single-owner root is INVIOLABLE. The sole multi-writer case is an explicitly-declared, org-scoped `multipub/<org-pool>/<topic>` MULTI-PUBLISHER topic (ADR 1105): a peer holding an in-effect co-publish grant additionally publishes THAT multi-publisher topic (at-most-own-org; cross-org refused) — never another peer's root. This view shows the anchor-level grant, not the per-topic slot detail."))

	// Release-set governance reveal (ADR 1027, data-access program B2): show
	// which partner orgs are DECLARED to receive which release lanes. This is
	// the picture, not the enforcement — reads remain flat until B2.1 cuts the
	// static read carve. Fail-closed: a load error is surfaced, never a silent
	// empty release picture.
	if s.cfg.ReleaseSetsPath == "" {
		warn = append(warn, doctrine("read_model", "reads: flat community-read — every on-mesh identity may read all topics (ADR 1025); release-set membership not declared (release-sets path unset)"))
	} else if rel, rerr := access.LoadReleaseSets(s.cfg.ReleaseSetsPath); rerr != nil {
		warn = append(warn, fail("release_sets_error", "release-set registry load error: "+rerr.Error()))
	} else {
		src.Releases = rel
		warn = append(warn, doctrine("read_model", "reads: flat community-read is STILL in effect (ADR 1025) — the declared release-sets are the governance picture, NOT yet an enforced read partition (B2.1)"))
	}

	// The derived member release-lane was removed by ADR 1148 §3 -- it revealed a
	// lane that grants nothing, since alliance members already read everything.
	// Declared release-sets are unaffected and still populate src.Releases.
	return src, warn
}

// identityNationality returns metadata_public.nationality: the PERSON's own
// citizenship (ADR 1171), ISO 3166-1 alpha-3. Read from admin-managed metadata
// only, never from self-editable traits, and never derived from the
// organisation's nation in aff. Empty means the identity record carries none:
// the approved manifest holds it, and stamping it onto the identity is
// designed, not built (docs/design/peer-identity-resolution.md).
func identityNationality(id kratosadmin.Identity) string {
	s, _ := id.MetadataPublic["nationality"].(string)
	return strings.TrimSpace(s)
}

// assembleAccessGraph builds the COP graph and reports how ownership was
// attributed. Every person whose anchor came from the org-tag name join instead
// of the recorded linkage (slots.identity_id) is a guess the surface is
// presenting as ownership — and ownership is what revocation targets — so the
// counts are logged on every assembly and returned as a note for the
// operator-facing surfaces. The fallback count goes to zero as onboards stamp
// the linkage and the backfill lands (identity fragility audit §1.2).
func assembleAccessGraph(src access.Sources) (*access.Graph, []access.Note) {
	g := access.Assemble(src)
	if g.AnchorJoin.Fallbacks() == 0 {
		return g, nil
	}
	slog.Warn("access COP attributed slot ownership by parsing names — no recorded identity linkage on those slots",
		"by_recorded_linkage", g.AnchorJoin.Recorded,
		"by_tag_join", g.AnchorJoin.TagJoin)
	return g, []access.Note{{Kind: "anchor_linkage_fallback", Severity: access.SeverityWarn, Text: fmt.Sprintf(
		"ownership attribution: %d person(s) resolved from the recorded linkage (slots.identity_id) and %d by the org-tag name join. "+
			"A name-joined row is an inference, not a recorded owner — check anchor_linkage before acting on it.",
		g.AnchorJoin.Recorded, g.AnchorJoin.TagJoin)}}
}

// enforceSelfOrOperator implements the three-tier COP privilege model that
// resolves the catch-22 (a denied vendor must be able to ask why without
// admin, data-access-cop.md §"Security"): an operator gets full access; a
// vendor principal may query only its own prefix; anything else is a
// graceful 403 that names the self-query it *can* run. Returns false (and
// writes the response) when the request is not permitted.
func (s *Server) enforceSelfOrOperator(w http.ResponseWriter, r *http.Request, id string) bool {
	p := PrincipalFromContext(r.Context())
	if p == nil {
		writeError(w, http.StatusUnauthorized, "auth", "missing principal")
		return false
	}
	if p.Operator {
		return true
	}
	if p.Vendor != "" && id == p.Vendor {
		return true // self-scoped: a vendor diagnosing its own access
	}
	writeError(w, http.StatusForbidden, "auth",
		"you may query your own access only (id="+p.Vendor+"); cross-identity queries need an operator")
	return false
}

// matchPeerVendor resolves a peer name to the vendor prefix that owns it. It
// used to be a hand-copied twin of netbirdpoll's matcher ("kept local because
// the poller's matcher is unexported"); both now call the one generated
// implementation in internal/peername.
func matchPeerVendor(peerName string, vendors []registry.Vendor) string {
	prefixes := make([]string, 0, len(vendors))
	for _, v := range vendors {
		prefixes = append(prefixes, v.Prefix)
	}
	return peername.MatchLongestKey(peerName, prefixes)
}

// handleAccessPeer serves GET /api/access/peer?netbird_name=<name> — resolves a
// NetBird peer to the person who owns it (docs/design/peer-identity-resolution.md,
// Option B). peer.user_id is empty by NetBird design (setup-key enrollment), so the
// link is the deterministic name: peer-<handle>-… → the declared roster (handle →
// email-hash) → the live Ory identity (email → same hash). Returns the Ory identity
// id (for an id-admin-console person deep-link) + org (for the access view). The
// person is best-effort — org always resolves; the Ory id resolves when the roster
// carries an email-hash that matches a live identity. Operator-only (exposes PII).
func (s *Server) handleAccessPeer(w http.ResponseWriter, r *http.Request) {
	name := strings.TrimSpace(r.URL.Query().Get("netbird_name"))
	if name == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing ?netbird_name= (the peer's NetBird name, e.g. peer-<handle>-<site>-<date>)")
		return
	}
	src, warn := s.collectAccessSources(r.Context())
	resp := resolvePeerName(src, name)
	resp["notes"] = access.NoteTexts(warn)
	writeJSON(w, http.StatusOK, resp)
}

// handleAccessPeersBatch serves POST /api/access/peers {"netbird_names":[...]} —
// the bulk form of the resolver: one graph assembly, every name resolved against
// it, so a consumer (a peer-name resolver baking the relay-health card)
// makes ONE call instead of N graph reassemblies. Operator-only (exposes PII).
func (s *Server) handleAccessPeersBatch(w http.ResponseWriter, r *http.Request) {
	var body struct {
		NetbirdNames []string `json:"netbird_names"`
	}
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxJSONBody)).Decode(&body); err != nil {
		writeDecodeError(w, err)
		return
	}
	src, warn := s.collectAccessSources(r.Context())
	out := make(map[string]map[string]any, len(body.NetbirdNames))
	for _, name := range body.NetbirdNames {
		name = strings.TrimSpace(name)
		if name == "" {
			continue
		}
		out[name] = resolvePeerName(src, name)
	}
	writeJSON(w, http.StatusOK, map[string]any{"peers": out, "notes": access.NoteTexts(warn)})
}

// resolvePeerName does one peer→person resolution against an assembled source
// set. The record answers first (recordOwner): a machine through its enrolled
// custodian, a person's device through the slot that carries the person's
// identity. The declared roster answers only when the record does not, and the
// row's `owner.source` says which one did (docs/design/peer-identity-resolution.md
// §"The owner comes from the record, not the roster").
func resolvePeerName(src access.Sources, name string) map[string]any {
	owner := recordOwner(src, name)
	if owner.source == "record" {
		return owner.row(src, name)
	}
	resp := resolvePeerNameFromRoster(src, name)
	if _, ok := resp["org"]; ok {
		resp["owner"] = map[string]any{
			"source": "roster",
			"note":   "answered from the declared roster: a lead, not the recorded link; the record says: " + owner.reason,
		}
	} else {
		resp["owner"] = map[string]any{"source": "none", "reason": owner.reason}
		if r, ok := resp["reason"].(string); ok && r != "" {
			resp["reason"] = owner.reason + "; roster: " + r
		} else {
			resp["reason"] = owner.reason
		}
	}
	return resp
}

// resolvePeerNameFromRoster is the roster resolution: name → roster handle
// (longest prefix) → org + the live Ory identity (email-hash reconcile). It is
// the fallback behind the record.
func resolvePeerNameFromRoster(src access.Sources, name string) map[string]any {
	resp := map[string]any{"netbird_name": name, "resolved": false}
	handle := access.MatchPeerHandle(name, src.Roster)
	if handle == "" {
		if system := access.MatchMachineHandle(name, src.Machines); system != "" {
			return resolveMachinePeer(src, name, system)
		}
		resp["reason"] = "no roster handle matched this peer name"
		return resp
	}
	resp["handle"] = handle
	var member *access.RosterEntry
	for i := range src.Roster {
		if src.Roster[i].Handle == handle {
			member = &src.Roster[i]
			break
		}
	}
	if member == nil {
		return resp
	}
	resolveMember(resp, src, member)
	return resp
}

// ownerAnswer is what the record says about one peer.
type ownerAnswer struct {
	source  string // "record" or "" (the record did not answer)
	via     string // "machine-custody" | "slot-identity"
	reason  string // why the record did not answer
	handle  string // the responsible person's slot handle
	ident   access.IdentityIn
	machine *access.MachineIn
}

// recordOwner answers from the recorded links only: a `machine-<handle>` peer
// from its enrolled machine's custodian (slots.enrolled_by → the custodian's
// slot, by anchor or alias → slots.identity_id → the Kratos identity); a
// `peer-<handle>-…` device from the slot whose alias is the longest handle the
// name carries and that has a recorded identity. Nothing here reads the roster.
func recordOwner(src access.Sources, name string) ownerAnswer {
	if system := access.MatchMachineHandle(name, src.Machines); system != "" {
		var m access.MachineIn
		for _, x := range src.Machines {
			if x.SystemHandle == system {
				m = x
				break
			}
		}
		a := ownerAnswer{via: "machine-custody", machine: &m}
		if m.Custodian == "" {
			a.reason = "machine " + system + " is enrolled with no custodian on its slot"
			return a
		}
		v := vendorFor(src, m.Custodian)
		if v == nil {
			a.reason = "machine " + system + " is enrolled, but its custodian " + m.Custodian + " holds no live slot"
			return a
		}
		return identityAnswer(src, a, v, "machine "+system+"'s custodian")
	}
	aliases := make([]string, 0, len(src.Vendors))
	for _, v := range src.Vendors {
		if v.Alias != "" {
			aliases = append(aliases, v.Alias)
		}
	}
	handle := peername.MatchLongestKey(name, aliases)
	if handle == "" {
		return ownerAnswer{reason: "no enrolled machine and no slot handle matches this peer name"}
	}
	return identityAnswer(src, ownerAnswer{via: "slot-identity"}, vendorFor(src, handle), "the slot for "+handle)
}

// vendorFor finds the slot a custodian column or a handle names: by anchor
// (the vendor prefix) first, then by alias.
func vendorFor(src access.Sources, key string) *access.VendorIn {
	for i := range src.Vendors {
		if src.Vendors[i].Prefix == key {
			return &src.Vendors[i]
		}
	}
	for i := range src.Vendors {
		if src.Vendors[i].Alias == key {
			return &src.Vendors[i]
		}
	}
	return nil
}

func identityAnswer(src access.Sources, a ownerAnswer, v *access.VendorIn, what string) ownerAnswer {
	a.handle = v.Alias
	if v.IdentityID == "" {
		a.reason = what + " carries no recorded identity (slots.identity_id is empty)"
		return a
	}
	for _, id := range src.Identities {
		if id.ID == v.IdentityID {
			a.ident = id
			a.source = "record"
			return a
		}
	}
	a.reason = what + " records identity " + v.IdentityID + ", which the identity layer does not hold"
	return a
}

// row renders a record answer. The top-level fields keep their meaning, so a
// consumer that reads only them gets the recorded owner; `owner` says where
// each fact came from, and states a fact the record lacks instead of leaving
// it blank.
func (a ownerAnswer) row(src access.Sources, name string) map[string]any {
	id := a.ident
	orgID := id.AffID
	if orgID == "" {
		orgID = id.Vendor
	}
	org := map[string]any{"id": id.AffID, "kind": id.AffKind, "nation": id.AffNation, "short_tag": id.Vendor}
	if orgID == "" {
		org["state"] = "missing"
		org["reason"] = "the identity record carries no organisation (metadata_public.aff and .org are absent)"
	} else {
		org["state"] = "recorded"
	}
	nat := map[string]any{"value": id.Nationality}
	if id.Nationality != "" {
		nat["state"] = "recorded"
	} else {
		nat["state"] = "not_stamped"
		nat["reason"] = "the identity record carries no nationality; it is held on the approved manifest and not yet stamped onto the identity"
	}
	owner := map[string]any{
		"source": "record",
		"via":    a.via,
		// The first hop of both record tiers is still the peer NAME: the
		// machine or slot handle is found in the name the key minted. Every
		// hop after it is a recorded link. The link is recorded end to end
		// only when the slot carries the setup key id or the NetBird peer id
		// that admitted this peer (ADR 1249 decision 3, designed). Said here
		// so a consumer does not read this as stronger than it is.
		"matched_by":  "peer-name",
		"link_note":   "the slot was found by the handle in the peer name; the slot's owner, identity and organisation are recorded links",
		"person":      map[string]any{"handle": a.handle, "identity_id": id.ID, "disabled": id.Disabled},
		"org":         org,
		"nationality": nat,
	}
	resp := map[string]any{
		"netbird_name": name,
		"resolved":     !id.Disabled,
		"handle":       a.handle,
		"org":          orgID,
		"affiliation":  id.AffKind,
		// Two facts, kept apart (ADR 1171): the organisation's nation and the
		// person's own nationality. Neither is filled from the other.
		"nation":      id.AffNation,
		"nationality": id.Nationality,
		"person": map[string]any{
			"ory_identity_id": id.ID,
			"email":           id.Email,
			"name":            id.Name,
			"disabled":        id.Disabled,
		},
		"deep_links": map[string]any{
			"id_console_user": "/users/" + id.ID,
			"org_access":      "/api/access/who?id=" + orgID,
		},
		"owner": owner,
	}
	if id.Disabled {
		resp["reason"] = "the recorded owner's identity is disabled"
	}
	if a.machine != nil {
		resp["machine"] = map[string]any{
			"system_handle": a.machine.SystemHandle,
			"slot_id":       a.machine.SlotID,
			"node_class":    a.machine.NodeClass,
		}
		owner["machine_unit"] = a.machine.Affiliation
	}
	return resp
}

// resolveMachinePeer answers for a peer the portal enrolled as a machine
// (ADR 1096). A machine cannot answer for itself (peer-identity-resolution.md,
// gap 1), so `handle` names the CUSTODIAN who enrolled it and `machine` says
// so. A consumer that reads only `handle` still gets the person answerable for
// the peer; one that reads `machine` can label that person a custodian.
func resolveMachinePeer(src access.Sources, name, system string) map[string]any {
	var m access.MachineIn
	for _, x := range src.Machines {
		if x.SystemHandle == system {
			m = x
			break
		}
	}
	resp := map[string]any{
		"netbird_name": name,
		"resolved":     false,
		"machine": map[string]any{
			"system_handle": m.SystemHandle,
			"slot_id":       m.SlotID,
			"node_class":    m.NodeClass,
		},
	}
	member := custodianMember(src, m.Custodian)
	if member == nil {
		resp["reason"] = "machine " + system + " is enrolled, but its custodian matches no declared roster member"
		return resp
	}
	resp["handle"] = member.Handle
	resolveMember(resp, src, member)
	return resp
}

// custodianMember finds the roster member a machine's custody edge points at.
// The recorded linkage first: the custodian's slot carries the Ory identity it
// was onboarded for (slots.identity_id), whose email-hash names the member.
// Then the slot alias, which is the handle the operator onboarded (ADR 1039),
// and last a custodian column that already holds a roster handle.
func custodianMember(src access.Sources, custodian string) *access.RosterEntry {
	if custodian == "" {
		return nil
	}
	var identityID, alias string
	for _, v := range src.Vendors {
		if v.Prefix == custodian {
			identityID, alias = v.IdentityID, v.Alias
			break
		}
	}
	if identityID != "" {
		for _, id := range src.Identities {
			if id.ID != identityID {
				continue
			}
			if h := access.EmailHash(id.Email); h != "" {
				for i := range src.Roster {
					if src.Roster[i].EmailHash == h {
						return &src.Roster[i]
					}
				}
			}
			break
		}
	}
	for _, key := range []string{alias, custodian} {
		if key == "" {
			continue
		}
		for i := range src.Roster {
			if src.Roster[i].Handle == key {
				return &src.Roster[i]
			}
		}
	}
	return nil
}

// resolveMember fills one matched roster member into a resolver row: its org
// facts always, and the person and deep-links when a live Ory identity shares
// the member's email-hash.
func resolveMember(resp map[string]any, src access.Sources, member *access.RosterEntry) {
	resp["org"] = member.Org
	resp["affiliation"] = member.Affiliation
	// Two separate facts, deliberately adjacent so a reader cannot mistake one
	// for the other (ADR 1171): `nation` is the nation that owns the member's
	// organisation, `nationality` is the member's own citizenship. Empty
	// nationality means the roster declares none — never "same as the org".
	resp["nation"] = member.Nation
	resp["nationality"] = member.Nationality
	resp["status"] = member.Status
	if member.EmailHash != "" {
		for _, id := range src.Identities {
			if access.EmailHash(id.Email) == member.EmailHash {
				resp["resolved"] = true
				resp["person"] = map[string]any{
					"ory_identity_id": id.ID,
					"email":           id.Email,
					"name":            id.Name,
					"disabled":        id.Disabled,
				}
				resp["deep_links"] = map[string]any{
					"id_console_user": "/users/" + id.ID,
					"org_access":      "/api/access/who?id=" + member.Org,
				}
				break
			}
		}
	}
	if resp["resolved"] == false {
		resp["reason"] = "handle resolved to org " + member.Org + " but no live Ory identity matched the roster email-hash"
	}
}

// handlePersonPresence serves GET /api/lookup/person-presence?id=<email|handle>
// — one person's WhoResult for the id-console's offboard-verification view
// (goat#319). The console holds the lookup service secret, not an operator
// key, so the operator-gated roster is out of its reach by design; this is
// the narrow door: same admission rule as the sponsorship lookup
// (lookupAuthorized), one identity per call. An unmatched id returns the
// unresolved WhoResult, not an error — "nothing in the graph" is the finding
// the offboard verifier is looking for.
func (s *Server) handlePersonPresence(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or console service token required")
		return
	}
	id := strings.TrimSpace(r.URL.Query().Get("id"))
	if id == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing ?id= (a person email or roster handle)")
		return
	}
	src, warn := s.collectAccessSources(r.Context())
	g, linkWarn := assembleAccessGraph(src)
	res := g.Who(id)
	res.Notes = append(res.Notes, access.NoteTexts(warn)...)
	res.Notes = append(res.Notes, access.NoteTexts(linkWarn)...)
	writeJSON(w, http.StatusOK, res)
}

// handleAccessWho serves GET /api/access/who?id=<identity> — the forward
// query (what this identity may write/read, with the reason at each layer).
// Self-or-operator scoped: a vendor may ask about its own prefix.
func (s *Server) handleAccessWho(w http.ResponseWriter, r *http.Request) {
	id := strings.TrimSpace(r.URL.Query().Get("id"))
	if id == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing ?id= (an org prefix, person email, peer name, or slot id)")
		return
	}
	if !s.enforceSelfOrOperator(w, r, id) {
		return
	}
	src, warn := s.collectAccessSources(r.Context())
	g, linkWarn := assembleAccessGraph(src)
	res := g.Who(id)
	res.Notes = append(res.Notes, access.NoteTexts(warn)...)
	res.Notes = append(res.Notes, access.NoteTexts(linkWarn)...)
	writeJSON(w, http.StatusOK, res)
}

// handleAccessWhy serves GET /api/access/why?id=<identity>&key=<topic> —
// the single allow/deny trace across mesh -> identity -> Zenoh ACL ->
// release (the troubleshooting workhorse + the basis for the CLI's
// doctor --topic write-precheck). Self-or-operator scoped on id.
func (s *Server) handleAccessWhy(w http.ResponseWriter, r *http.Request) {
	id := strings.TrimSpace(r.URL.Query().Get("id"))
	key := strings.TrimSpace(r.URL.Query().Get("key"))
	if id == "" || key == "" {
		writeError(w, http.StatusBadRequest, "decode", "why needs both ?id= and ?key=")
		return
	}
	if !s.enforceSelfOrOperator(w, r, id) {
		return
	}
	src, warn := s.collectAccessSources(r.Context())
	g, _ := assembleAccessGraph(src)
	res := g.Why(id, key)
	res.Notes = append(res.Notes, access.NoteTexts(warn)...)
	writeJSON(w, http.StatusOK, res)
}

// handleAccessRoster serves GET /api/access/roster — the bulk forward query:
// a slim cross-layer summary (W/R counts + reconcile state + on-mesh /
// last-activity) for every identity, assembled once. It is the responsiveness
// answer for `goat who`: the roster renders W/R + state for the whole fleet
// from this one call instead of an N-deep per-row `who` fetch over the lossy
// mesh. Operator-only — it reads across every identity layer.
func (s *Server) handleAccessRoster(w http.ResponseWriter, r *http.Request) {
	src, warn := s.collectAccessSources(r.Context())
	g, linkWarn := assembleAccessGraph(src)
	res := g.Roster()
	// Roster() emits no notes of its own, so these two are the whole list —
	// which is what lets NoteDetails stay exactly index-aligned with Notes.
	notes := append(append([]access.Note{}, warn...), linkWarn...)
	res.Notes = append(res.Notes, access.NoteTexts(notes)...)
	res.NoteDetails = notes
	// Stamp the declared-registry provenance so the client can report roster age
	// and detect a portal serving a stale copy (the "strays are suspect" caveat,
	// made concrete). Advisory — nil when the roster path is unset/unstamped.
	if s.cfg.RosterPath != "" {
		res.Freshness = access.LoadRosterFreshness(s.cfg.RosterPath)
	}
	writeJSON(w, http.StatusOK, res)
}

// handleAccessTopic serves GET /api/access/topic?key=<key> — the reverse
// query (who may write/read this topic, and who may receive it if released).
func (s *Server) handleAccessTopic(w http.ResponseWriter, r *http.Request) {
	key := strings.TrimSpace(r.URL.Query().Get("key"))
	if key == "" {
		writeError(w, http.StatusBadRequest, "decode", "missing ?key= (a topic key, e.g. acme/observations/v1)")
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "access.topic.read", key, "")
	src, warn := s.collectAccessSources(r.Context())
	g, _ := assembleAccessGraph(src)
	res := g.Topic(key)
	res.Notes = append(res.Notes, access.NoteTexts(warn)...)
	writeJSON(w, http.StatusOK, res)
}

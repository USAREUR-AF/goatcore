# Peer ↔ person identity resolution

**Status:** Option B **deployed + live-verified on ZAGROS 2026-07-08** — the portal name→person resolver (`GET /api/access/peer` + batch `POST /api/access/peers`) resolves a real peer to its owning Ory identity, and the Thor relay-health dashboard bakes the id into each card and renders an `identity & access →` deep-link to the id-admin-console person page. Peer cards whose names are not `peer-<handle>` (infra/synth names) stay unresolved, correctly. The Thor resolver reaches the portal app by docker-inspecting the container IP on the `mgmt` network (port `8090`, operator Bearer key) — the portal mesh hostname doesn't resolve from the Thor itself and the host-published `:8088` is the oauth2/ADMINS gate, not the app. **Option A (NetBird `user_id` stamp) — attempted, then found a dead-end by live-fire (F-237).** The fork patch is built + validated + deployed on ZAGROS, and it correctly stamps `peer.UserID` — but NetBird **validates `peer.UserID` against its own `users` table**, and ZAGROS has zero person-scoped NetBird users (all `issued=api` service accounts), so a peer bound to a Kratos identity id fails to sync (`user … not found`). Binding an external identity id therefore *breaks* enrollment. The onboarding wiring was reverted before merge; the merged patch (#1437) stays as a harmless, default-off capability usable only with a real NetBird user id. **Option B is the resolution mechanism** — it needs no NetBird user and covers all peers. Companion to [data-access-cop.md](data-access-cop.md) §"Entry from the relay-health baseball card" and relay-observability.md B.4.

**Second consumer, 2026-08-14: netvis.** The reachability surface now carries an
ownership lens on its peer and pair views — see netvis.md §"Who is
responsible for this machine". It calls the same batch endpoint through
`panoscope_core::peerowner`, the **Rust** client for this resolver (the Thor
side is Go). Two things that module adds, and that any future consumer should
take rather than re-derive:

- **The four-state attribution is a type, not a convention.** `registry` (the
  recorded linkage) / `name` (handle parsed, no live identity) / `unmatched`
  (the name carries no principal) / `unread` (we could not ask). The resolver's
  wire format distinguishes the middle two only in prose — `resolved: false`
  covers both, separated by whether `handle` is present — so every consumer
  that reads this API raw has to rediscover that. It also has no `unread` at
  all, because a transport failure is the caller's to name; collapsing it into
  "not resolved" reports a failure to measure as a measurement.
- **A resolver failure may never take the page down.** `resolve()` never returns
  `Err`: it returns a row for every name asked about, with the transport error
  carried as that row's reason. Ownership is an enrichment on a surface whose
  actual job is reachability.

**Third consumer, 2026-08-17: the peerview surfaces**, and with them the
**Python client**, [`ops/identity/peerowner.py`](../../ops/identity/peerowner.py).
The endpoint was already shared; the *reading* of it was not — the Thor
resolver carried its own inline POST and its own partial reading, panoscope_core
carried the typed one, and peerview was about to be a fourth. There is now one
client per language (Go on the card, `panoscope_core::peerowner` in Rust, this in
Python) and the Thor resolver has been moved onto it, so the four-state
attribution is derived once per language rather than once per consumer. The
Python client additionally **drops the person's email and display name by
default**: the endpoint is operator-scoped and returns both, and a surface that
renders handles must not leak a roster of email addresses into a served json by
forgetting to strip a field.

### Two gaps this resolver has, found by the third consumer

Both are cases where the resolver's answer is correct and cannot be acted on.
peerview supplies them locally today and says which source it used; they belong
here, so that every consumer gets them.

1. **A machine resolves to the machine.** A peer minted for a system resolves to
   that system's handle, which can never answer for anything. The person who can
   is its **custodian** — the person→system custody edge ([ADR 1096](../adr/1096-delegated-machine-enrollment.md)).

   **For machines the portal enrolled, the resolver answers this itself** (code
   written 2026-09-11; live once the portal is redeployed). A
   `machine-<handle>[-<hostname>]` name is matched against the registry's
   enrolled machines (`access.MatchMachineHandle`, longest handle wins), `handle`
   names the custodian, and a `machine` block (`system_handle`, `slot_id`,
   `node_class`) marks it as a custodian rather than an owner. Custodian → roster
   member uses the recorded linkage first (the custodian slot's `identity_id` →
   Ory identity → email-hash), then the slot alias, then a handle-valued
   custodian column. With no member found the row carries `machine` and a
   `reason` but no `handle`.

   Two parts stay open. Systems declared in operator manifests rather than
   enrolled through the portal still resolve to the machine; peerview re-points
   those locally. And `panoscope_core::peerowner` and the Go relay-health card
   read `handle` without `machine`, so they show a machine's custodian with no
   custodian label until they read the block.
2. **Substrate has no answer at all, forever.** The mgmt host, the relays, a
   kiosk box: no onboarding name, so `unmatched` is correct and permanent. The
   activity log does record the enrolment, but for a setup-key enrolment **the
   initiator is the key** (`opEvent.InitiatorID = sk.Id`, netbird
   `management/server/peer.go`), so a shared key names nobody — a lead, not an
   owner. peerview takes a declared `<peer-glob>: <handle>` map from config as a
   fallback that never overrides a resolved answer. A declared-owner tier
   belongs in the registry rather than in one surface's config.

## Machine names and their NetBird peer names

A machine's name follows one grammar, recorded in ADR 1267 (machine-name grammar, `ops/identity`,
generated and CI-checked): one string is the machine handle, its OS hostname, its fabric key segment and
its record. Its NetBird peer name is `machine-<handle>`.

Resolution already accepts that form: `owned_peer_name` (goat-cli `machine_ssh.rs`) and
`MatchMachineHandle` (portal `access/graph.go`) match `machine-<handle>` exactly, as well as
`machine-<handle>-<host>` and the bare handle.

Enrollment does not produce it yet. The portal names the setup key `machine-<handle>`, and NetBird appends
`-<hostname>`, so a machine whose hostname equals its handle joins as `machine-<handle>-<handle>`.
**Proposed:** when the hostname equals the handle, the peer is named `machine-<handle>`, which needs a change
where the fork composes the peer name from the setup key and the hostname. Until then the documented hand
rename to `machine-<handle>` (through the NetBird management API, recorded with the machine) keeps
resolution working.

## The owner comes from the record, not the roster

**Status:** the resolver's record tiers for machines and for slot-linked
devices are built (§"The resolver"), and so is the host carrier (§"The
carrier"). The identity handle and nationality stamps, declared owners for
infrastructure, the ownership audit and the surface states are designed, not
built; each names where it will be built.

**The rule.** No machine, and above all no router, holds mesh access without a
recorded responsible person and organization. A surface that cannot show the
owner says the lookup failed. "Not recorded" is a finding, never a label.

**Why the roster cannot answer.** Until this change the resolver answered every
peer through the declared roster (`roster.json`), which `gen-roster.py`
generates from operator manifests. Onboarding through the identity console and
the portal writes no manifest, so the roster stopped growing when onboarding
moved there.

### What the records hold, peer by peer

An audit of every mesh peer of a goatnet against the portal registry and the
Kratos identities sorts each peer into one of five classes:

| Class | What the record holds |
|---|---|
| `machine-<system>` | an enrollment row with custodian and affiliation |
| person device the roster resolver answered | a live identity, reached through the roster |
| person device (`peer-<handle>-…`) the roster resolver did not answer | an identity that matches the handle only by name or e-mail local part, not by a recorded link |
| person device | no identity at all |
| a name that carries no owner | nothing, including some of the goatnet's own infrastructure |

The identities carry an organization; none carries a nationality. Machine
custodians onboarded through the console are not in the roster.

### Where each fact is recorded

| Fact | Recorded in | Read by the portal |
|---|---|---|
| A machine's custodian | `slots.enrolled_by` on the machine's slot (ADR 1096): the custodian's slot anchor or handle | yes |
| A machine's unit | `slots.affiliation`, inherited from the custodian's grant. Free text until it becomes an org id (ADR 1112) | yes |
| A person's slot and identity | `slots.assigned_to` (the handle, ADR 1039) and `slots.identity_id` (the Kratos identity, stamped at onboard) | yes |
| A person's organization and its nation | Kratos `metadata_public.aff {id, kind, nation}` and `.org {short_tag, display_name}`, written at mint from the approved manifest (`goat-cli` `admin.rs`, ADR 1188). The console's `orgs` table is the org registry | yes |
| A person's handle | the approved manifest in the console's `onboard_proposals`; the mint writes it nowhere on the identity | **no** |
| A person's own nationality | `identity.nationality` on the same approved manifest | **no** |
| Who answers for an infrastructure peer | nowhere a resolver reads; peerview keeps a local glob map | **no** |

### The resolver (built: machine custody and slot identity)

`POST /api/access/peers` and `GET /api/access/peer` resolve from the record
first. Every answer carries an `owner` block that says where each fact came
from. The tiers, in order:

1. **`machine-custody`** for a `machine-<handle>[-<host>]` peer. Built. The
   enrolled machine's `enrolled_by` is matched to the custodian's slot by
   anchor or alias, then to that slot's `identity_id`, then to the Kratos
   identity. The responsible person is the custodian. The organization and its
   nation are the custodian's recorded `aff`. The machine's own unit
   (`slots.affiliation`) is carried beside them.
2. **`slot-identity`** for a `peer-<handle>-…` device. Built. The handle is
   matched, longest first, against the aliases of slots, then to the slot's
   `identity_id`.
3. **`identity-handle`** for a person device with no slot link. Designed: the
   mint stamps `metadata_public.handle` from the approved manifest, and `goat
   admin identity resync` backfills it from the console's approved proposals.
   The resolver then matches the peer's handle to an identity exactly, so the
   approximate matches become recorded ones.
4. **`declared`** for an infrastructure peer (management hosts, relays, DNS,
   collaboration and help-desk hosts). Designed: a per-goatnet declaration in
   the overlay, beside `ops/netbird/service-identities.yaml`. Each entry
   matches peer names exactly or by an anchored pattern and names an owner
   handle that must itself resolve to a recorded identity, plus the reason the
   peer exists. The portal reads it through a configured path. A declaration
   whose owner does not resolve is itself a finding.
5. **`roster`**, a labeled fallback (`owner.source: "roster"`) while tiers 3
   and 4 are not built.
6. **`none`**, with a `reason` naming the missing link. Examples: no enrolled
   machine and no slot handle matches the name; the custodian holds no live
   slot; the slot carries no recorded identity; the identity is not in Kratos.

| `owner` field | Meaning |
|---|---|
| `source` | `record`, `roster` or `none` (`declared` when tier 4 is built) |
| `via` | `machine-custody` or `slot-identity`; `identity-handle` when built |
| `matched_by` | `peer-name` for both built tiers: see below |
| `person` | the responsible person: `handle`, `identity_id`, `disabled`. E-mail and display name stay in the existing `person` block |
| `org` | `id`, `kind`, `nation` (the organization's nation), `short_tag`, and `state`: `recorded`, or `missing` with a reason |
| `nationality` | `value` and `state`: `recorded`, or `not_stamped` with a reason. Never filled from the organization's nation (ADR 1171) |
| `machine_unit` | a machine's `slots.affiliation`, as recorded |

**How strong a record answer is.** Both built tiers find the slot by name:
`machine-custody` reads the machine handle in `machine-<handle>-…` and
`slot-identity` reads the longest slot alias in `peer-<handle>-…`. Every hop
after that is a recorded link: the slot's custodian, its `identity_id`, and the
identity's organization. So `matched_by: "peer-name"` says the first hop is
the name, which the mint set from the same record but which a hand-made key
could imitate.

The link becomes recorded end to end when the slot (or the approved proposal,
for a person) carries the setup key id that admitted the peer, and the
resolver joins the peer to it through NetBird's peer-add event. That is
ADR 1249 decision 3, designed and not built. `matched_by` then becomes
`setup-key`.

**Which slots carry an identity.**

| Slot | Expected to carry `identity_id` |
|---|---|
| A person's slot | Yes, stamped at onboard. One per person identity |
| An organization's own slot (vendor prefix, no person) | No |
| A machine's slot | No. Its owner is `enrolled_by`, the custodian |

A person slot without an identity is a person device that falls back to the
roster. The ownership audit (below) reports how many there are.

The existing top-level fields (`handle`, `org`, `nation`, `nationality`,
`resolved`, `person`) are filled from the record when the record answers, so a
consumer that reads only those fields improves without a change. `resolved`
stays true only for a live, enabled identity.

### The nationality stamp (designed, not built)

The mint writes the authorization fields to `metadata_public` from the approved
manifest. It also writes `metadata_public.nationality` from
`identity.nationality` and `metadata_public.handle` from `identity.handle`.
`goat admin identity resync` backfills both for existing identities from the
console's approved proposals. The resolver already reads `nationality`, so it
fills in with no portal change. Until then every answer carries
`nationality.state: not_stamped`.

### The ownership audit (designed, not built)

The check that keeps the rule true runs in the portal, because the portal
already holds the mesh peer list (`PeersForAccess`) beside the records:

- `GET /api/access/ownership` (operator) resolves every mesh peer through the
  tiers above. It returns counts by tier and class, and the peers whose
  `owner.source` is not `record` or `declared`, each with its reason.
- The same counts are exported as `goat_peer_owner_unrecorded{class}`. The
  expected value is 0, and a rule on the existing Alertmanager path fires
  while it is not.
- A peer that is connected and unrecorded is critical. An offline one is a
  warning until its access is removed or its owner recorded.

The built tiers answer every enrolled machine from custody and every person
device that has a slot link from it, which also moves roster answers onto the
record. Tiers 3 and 4, plus owner declarations for the goatnet's own
infrastructure, leave the person devices with no identity and the peers whose
name carries no owner. Each of those needs an owner recorded or its access
removed. That is an operator action, not code.

### The carrier to the host surfaces (built)

The relay-health `relay-peer-names` writer runs every 5 minutes on the
management host and writes `peer-names.json`. A failed owner read never
becomes "no owner".

- **It asks about every peer,** machines and infrastructure included. Names go
  in chunks of 500, under the portal's 1 MiB request cap.
- **It writes each card's `owner` block.** The block carries no e-mail or
  display name; `ory_id` is kept as before. The shared client
  `ops/identity/peerowner.py` carries the block on every row and never raises:
  a body that is not a peers map is `unread` for every name.
- **The portal says when it could not read its own sources.** The batch
  answer carries `owner_sources: {ok, failed}`. When a layer the owner tiers
  read did not answer (the registry, the slot aliases, the enrolled machines
  or the identity layer), `ok` is false and every row's owner is `unread`,
  naming the layer.
- **Other answers that are unread, not "none":**
  - the portal cannot be asked (no key on the host, refused, unreachable);
  - its sources failed;
  - it is older than the `owner` block;
  - it sends an owner block without a `source`.

  When every card is unread, the file says so once:
  `owner_read: {state: "unread", reason, at}`.
- **The cloud relay-health box gets the owner state only.** It receives the
  names it always had plus each card's owner state (`source`, and whether an
  owner exception covers it), and the file-level `owner_read` and
  `generated_at`. These ride in a reserved key that an older ingest stores
  harmlessly and a current one lifts to the top of its file. No person,
  identity id, organization or nationality leaves the management host.
- **An over-cap or refused cloud body is a failure you can see.** A body over
  the ingest's cap (262144 bytes) is not sent, and the writer exits non-zero
  after writing the on-host file. A cloud ingest that refuses the body does
  the same. A cloud box that cannot be reached is logged only.
- **The deploy proves the key on the route the writer uses.** The relay-health
  role sends `POST /api/access/peers` with an empty list and the writer's key.
  - A refused key (401 or 403) fails the deploy.
  - A portal that is down or not found is a warning, and the health dashboard
    still deploys.
  - The writer enabled with no key fails the deploy, unless
    `relay_peer_names_owner_enabled: false` switches owners off on purpose.

  A host writer's key can drift from the portal's root key, and the portal's
  handle-map guard never compares the root key, so this check is the only
  thing that catches it.

No new credential: the writer already holds a key.

### The topology surface (designed, not built)

`goat-topology serve` reads the `owner` block from `peer-names.json` first. The
roster file remains only as a fallback, and the page labels it as one.

| State | Shown as |
|---|---|
| recorded or declared | the organization, its nation and the person, marked "from the record" |
| lead | from the roster or the name only, marked "a lead, not the record" |
| lookup failed | "owner not read", with the reason |
| no record | a **critical finding**: a router in the fabric with no recorded responsible person or organization |

### The roster after this

`roster.json` stops being the source of ownership. It remains a fallback only
until every peer answers from the record or a declaration; then it is retired
from the resolver. Designed, not built: `gen-roster.py` refuses to write a
roster with fewer members than the committed one unless it is told to, so
regenerating against an empty manifest folder fails loudly instead of emptying
the file.

## The problem

Surfaces that hold a **NetBird peer** (the relay-health baseball card, `goat who`, panoscope) want to resolve it to the **person** who owns it — their Ory identity, credentialing, roles, affiliations, permissions. There was no obvious link, and the natural key is empty.

## The finding — `user_id` is empty *by NetBird design*, and the link lives in the name

- **`peer.user_id` is blank for every setup-key-enrolled peer.** This is not a bug: NetBird only stamps `peer.UserID` when a peer is added by an authenticated *user* over JWT (the `addedByUser` gate, `netbird management/server/peer.go:701` + `:562`). Setup-key (non-interactive) enrollment never sets it. A setup-key-enrolled peer therefore has `user_id=""`.
- **The real link is the deterministic peer NAME.** Onboarding names peers `peer-<handle>[-<short_tag>]-<site>-<date>` from `manifest.identity.handle` at mint time (`operator-onboard.yml`, `auto_peer_name`); the handle is schema-validated kebab-case (`operator-manifest.schema.json`). The manifest (`ops/operators/<env>/<handle>.yml`) carries `handle → email → name → org`. So **peer → person = parse the handle from the name → registry lookup → email → Ory identity**. Deterministic, not fuzzy.
- **peer → org is already resolvable in the portal** (`matchPeerVendor`, `access.go`; `who?id=<peer-name>` already accepts a peer name). What is *missing* is **peer → the person's Ory identity id** (needed to deep-link the id-admin-console person page) and a **recorded, queryable** index so this doesn't depend on re-deriving the join each time.

## Option B (now) — formalize the name → registry resolution

Make the person↔peer link **explicit, recorded, and queryable** without a NetBird fork. Works for existing peers (their names already encode the handle) and all forward ones.

1. **Record the link at onboard.** Manifest schema gains `netbird.setup_key_ids: {<site>: "<key-id>"}` and `netbird.auto_peer_names: {<site>: "peer-…"}`; the orchestrator writes the minted key id + computed peer name back into the manifest after Phase 1. This turns the fire-and-forget key mint into a recorded edge (audit + resolution).
2. **A resolver.** `GET /api/access/peer?netbird_name=<name>` (or `?netbird_id=`) → `{handle, org, person: {email, name, ory_identity_id}, deep_links: {id_console, org_access}}`. It parses the handle from the name, looks the person up in the registry/roster (by handle/email), resolves the Ory id via the existing `kratosadmin` reader (email→id), and returns the deep-link targets. Operator-scoped, reuses the readers the access graph already assembles.
3. **The deep-link consumes it.** The relay-health baseball card's "identity & access →" link (mesh-only, admin-gated) targets the id-admin-console person page (`/users/<ory_identity_id>`) when the person resolves, else the org access view (always resolvable). Resolution is baked server-side (the Thor resolver), so the browser never needs a cross-service authed call.

**Invariants:** the name→handle convention is the contract (validated at onboard); a peer name that doesn't parse fails loud (no silent mis-attribution); the resolver is read-only + operator-scoped; nothing here touches enforcement.

**The access graph consumes the same link (2026-07).** `access.MatchPeerHandle` (the resolver's longest-handle-prefix match, moved into the pure graph package) now also runs inside `Assemble`: every mesh peer whose name matches a declared member handle gets an `is_on_mesh` edge to that person's reconciled node, and the person's `on_mesh`/`connected`/`last_seen` roll up from their own devices — org membership no longer required, which closes the "internal person with no org shows `on mesh: no` while demonstrably connected" gap. `who` responses carry `mesh_attribution` (`peer`/`org`/`none`) so renderers can be honest about can't-know vs known-absent (see [data-access-cop.md](data-access-cop.md) §"The person↔peer custody edge"). Note the manifest write-back fields from step 1 (`netbird.auto_peer_names`) are still absent from the live manifest set — the graph join deliberately derives the prefix from the handle (the convention) rather than requiring the recorded field.

**The convention is single-sourced + fenced (2026-08).** Because the name IS the link, its grammar became load-bearing — and by the 2026-08-01 identity fragility audit §1.2 it had five independent implementations across Go, Rust and the onboard playbook with no fence between them. There is now one machine source, [`ops/identity/peer-name-grammar.yaml`](../../ops/identity/peer-name-grammar.yaml), generated by `ops/identity/generate-peer-name.py` into `internal/peername` (Go) and `panoscope_core::peername` (Rust). `ops/ci/check-peer-name-grammar-sync.sh` fails the build when a generated consumer drifts, and asserts the Ansible playbook still templates the fields in the SoT's order; the SoT's conformance vectors are generated into a table-driven test in **both** languages, so a behavioural divergence between the minter (goat-cli) and the parsers (the portal) fails a test, not only a byte compare.

Two rules with different strictness come out of that SoT, and which one a caller uses is a safety decision:

- `match_longest_key` — the loose join behind the person and org attributions. Its stated limit is that the field separator also occurs *inside* handles, so `alex` does match `peer-alex-smith-…` when `alex-smith` is not itself among the candidates; pass the full candidate set and let longest-wins disambiguate.
- `belongs_to` — anchored on handle + optional org tag + site. This is the one a **cut** may use: goat-cli's native revoke selects peers with it, which is what stopped the 2026-07-14 org-mate mis-attribution.

## Option A (attempted — dead-end, see F-237) — NetBird `user_id` stamp for the DB-layer link

The idea was to close the DB-layer gap for **forward** peers so `SELECT … WHERE user_id = ?` works without name parsing, by minting **user-bound** setup keys that stamp `peer.UserID` = the owner's Ory identity id.

**What was built + deployed:**

1. **NetBird fork — built + deployed (`ops/netbird/patches/setupkey-user-id-stamp.patch`, #1437).** Adds optional `UserID` on the `SetupKey` type; a user-bound key stamps its owner on `peer.UserID` at `AddPeer`. Mirrors the AutoPeerName wiring exactly (struct field + `Copy` + `GenerateSetupKey`/`CreateSetupKey` param + `api.CreateSetupKeyRequest.user_id` + `handleSetupKeyAddedPeer`→`peerAddConfig.SetupKeyUserID`→`AddPeer`); does NOT set `addedByUser`. Empty `user_id` = upstream behavior (additive/default-off). Applied 4th in `ops/netbird/Dockerfile`; deployed on ZAGROS (`:latest`, `setup_keys.user_id` column added by AutoMigrate). Verified live: a `POST /api/setup-keys` with `user_id` persists it, and enrollment stamps `peer.UserID`. **The patch is correct.**

**Why it's a dead-end (live-fire, F-237):** NetBird **validates `peer.UserID` against its own `users` table** for the network-map/sync path. ZAGROS has zero person-scoped NetBird users (all `issued=api` service accounts); setup-key participants have no NetBird user at all. So a peer bound to a Kratos identity id fails to sync (`rpc … user: <id> not found`) — binding an external id *breaks* enrollment, worse than a no-op. The onboarding wiring that would have set it (resolve-by-email at mint) was **reverted before merge**. The merged patch stays as a harmless capability usable **only** with a real NetBird user id — it must never be fed an external identity id.

**Resolution: Option B is the mechanism.** It needs no NetBird user, covers all peers by name, and is live. Making `peer.UserID` viable would require *creating* a NetBird user per person purely to populate an attribution column — not worth it when B resolves cleanly. Existing peers (and forward ones) stay name+registry-linked via B.

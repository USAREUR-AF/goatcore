# ADR 1249 — No mesh peer joins without a recorded owner; a peer without one is shown, counted, and carries an owner exception or pages

| Field | Value |
|---|---|
| Status | Proposed. Decisions 5, 6 and 7 are built for machines and slot-linked person devices; decisions 2, 3 and 9 are designed, not built. |
| Date | 2026-11-16 |
| Repo scope | goat core (`ops/goat-zenoh-portal/`, `crates/goat-cli/`, `ops/ansible/playbooks/operator-onboard.yml`, `ops/mgmt-host-bootstrap/`, `ops/ansible/scripts/create-setup-key.py`, `ops/relay/health/relay-peer-names.py`, `docs/design/peer-identity-resolution.md`); each goatnet's overlay (the declared-owner registry, the owner exceptions file) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1096](1096-delegated-machine-enrollment.md) (delegated machine enrollment) · [ADR 1039](1039-slot-alias-display-layer.md) (slot aliases) · [ADR 1171](1171-person-nationality-and-org-ownership-are-two-facts.md) (two nationality facts) · [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock NetBird) · [`peer-identity-resolution.md`](../design/peer-identity-resolution.md) |

## TL;DR (for a layperson)

Every computer that joins a goatnet's private network must have a named person and an organization responsible for it, written down before it is let in. Today some ways of joining record that and some do not, so a goatnet can hold machines nobody can trace to a person. From now on, only the ways of joining that record an owner are used. A check that runs every few minutes reports any machine that arrived without one. Machines already on the network are not touched. Each one without a recorded owner can carry an owner exception: a short note, with who granted it and why, that every screen shows beside the machine. The operator decides about each of them later.

## Context

A mesh peer is admitted by a NetBird setup key. Whoever holds a key can bring a machine onto the mesh with the key's groups. Nothing in stock NetBird requires the key, or the peer it admits, to name a responsible person.

An audit of a goatnet's mesh peers against the portal registry and the Kratos identities finds every enrolled machine recorded with a custodian and organization, person devices that reach an identity only through the roster or only by name, person devices with no identity, and peers whose name carries no owner, including some of the goatnet's own infrastructure.

The owner records that exist were not read by the surfaces; that defect is fixed separately ([`peer-identity-resolution.md`](../design/peer-identity-resolution.md)). This decision is about the doors: the ways a peer can join, and which of them record an owner.

**Nine doors, read from the code on 2026-11-16:**

| # | Door | Key | Owner recorded before the key exists | Key linked to the record |
|---|---|---|---|---|
| 1 | Person onboarding: console, approval, mint (`goat admin onboard`, `operator-onboard.yml`) | reusable, usage 5 by default, `peer-<handle>-…` name | yes: person, sponsor and organization in the console's approved proposal | by name prefix only; the manifest field `netbird.setup_key_ids` exists and nothing writes it |
| 2 | Portal `POST /api/machines/enroll` | one-off, 24 h, `machine-<handle>` name | yes: custodian and affiliation on the slot | by name prefix only; the key id is not stored |
| 2b | The same door with key minting off: an operator makes the key by hand | any | the slot, yes; the key, no | nothing |
| 3 | Management-host bootstrap (`mgmt-host-bootstrap create-setup-key`) | reusable, usage 200 by default, written straight into the database | no | nothing |
| 4 | Join-node playbook (`mesh-enroll-host.yml`) | one-off, 1 h, `phase8-<host>` | no | key name only |
| 5 | Terraform-built hosts (DNS, relay health, host nodes) | from an environment secret | no | hostname convention |
| 6 | Bench and test keys, `create-setup-key.py` | reusable, usage 10–20 | `created_by` text in an audit table | nothing |
| 7 | Re-mints for existing people (reissue, Rosenpass re-invite) | reusable, short-lived | carried from the person's record | name prefix |
| 8 | NetBird dashboard or API, by any NetBird admin or token holder | any shape, including unlimited | no | NetBird's own activity event only |
| 9 | SSO peer login, where a goatnet enables it | none | the NetBird user, created at first sign-in; no organization | `peer.user_id` |

Doors 1 and 2 record an owner before the key exists. Doors 3 to 6 and 8 record none. Door 9 records a user and no organization. No door writes the key id back to the owner record, so nothing can say, for a given peer, "this key, minted for this person".

## Decision

1. **The rule.** A peer may join a goatnet only through a door that records a responsible person and an organization before the key exists, and that records the key's id against that owner. A peer that joins any other way is a finding, raised within one reconcile cycle and paged.

2. **Owner records for infrastructure.** Each goatnet's overlay carries a declared-owner registry for its own infrastructure peers (management hosts, relays, DNS, observability, collaboration and help-desk hosts), beside `ops/netbird/service-identities.yaml`. Each entry carries:
   - an exact name, or an anchored name pattern;
   - an owner handle that must resolve to a recorded identity;
   - an organization;
   - the reason the peer exists.

   An entry whose owner handle does not resolve is itself a finding.

3. **Every key-minting path records an owner and the key id.** Each path keeps its present key shape, plus the following:
   - **Door 1:** writes the minted key id to the approved proposal (`netbird.setup_key_ids`).
   - **Door 2:** stores the key id on the machine's slot.
   - **Doors 3, 4, 6, and 2b's hand-made key:** the tool refuses to mint without `--owner <declared entry>` and writes the key id into the declared-owner registry's record of mints.
   - **Door 5:** takes its key from the same owner-recording tool, never from a bare environment secret.
   - **Door 7:** carries the person's record over, as today, and writes the new key id beside it.
   - **The bootstrap default changes from 200 uses to 1.** The re-mint keys stay reusable, short-lived and bounded.

4. **Keys made outside these paths are not part of the process.** Door 8 is not closed in NetBird (stock NetBird, ADR 1248). It is covered by decision 5: a peer admitted by a key with no recorded owner is reported within one cycle.

5. **The reconcile.** Every cycle (5 minutes), the host writer that already reads every mesh peer and asks the portal for its owner (relay-health `relay-peer-names`) grades each peer:
   - **Recorded:** by the record tiers of [`peer-identity-resolution.md`](../design/peer-identity-resolution.md), or, once decision 3 is built, by the key id that admitted it (NetBird's peer-add activity event joined to the recorded key ids).
   - **Declared:** by the declared-owner registry.
   - **Excepted:** under an owner exception (decision 7). Counted in `goat_peer_owner_excepted`; it does not page.
   - **Exception stale:** under an exception, but the record now names an owner, or the peer has left the mesh. Reported for removal (`PeerOwnerExceptionStale`, warning).
   - **Lead:** answered only by the declared roster. A lead is not a record.
   - **Unrecorded:** anything else.

   Leads and unrecorded peers are counted in `goat_peer_owner_unrecorded`, whose expected value is 0. The rule `PeerOwnerUnrecorded` pages (critical) while it is above 0. Two warnings say when the count is blind: `PeerOwnerNotRead` (the lookup failed) and `PeerOwnerReconcileStale` (the writer stopped). The names are in the writer's journal, never in the metric. Nothing is removed automatically: removing access is an operator action.

6. **Surfaces show the record.** The portal resolver answers from the record first, and every answer says its source. This is **built**: machine custody and slot identity. The host writer carries the answer, says when it could not ask, and fails its deploy when the portal refuses its key. Surfaces show an unrecorded peer, and above all an unrecorded router, as a finding, never as a gray label.

7. **A peer without a recorded owner may carry an owner exception: shown, counted, never hidden.** An owner exception is a decision the operator made about one peer, kept in each goatnet's private overlay.
   - **One entry per peer:** the stable NetBird peer id and its name, a `reason`, `granted_by`, `granted_on`, and an optional `review_by` that only the operator sets. No tool invents a date.
   - **The public tree** carries only the schema (`ops/identity/peer-owner-exceptions.schema.json`), a synthetic example and tests.
   - **How it is shown.** An excepted peer is not counted in `goat_peer_owner_unrecorded` and does not page. It is counted in `goat_peer_owner_excepted`. Every surface that shows owners shows it as "owner exception" with its reason, never as recorded and never hidden: the peer-names card, the topology page, and any later ownership endpoint.
   - **Staleness.** An exception whose peer later gets a recorded owner, or leaves the mesh, is reported as stale so it can be removed.
   - **The initial file.** It is written once by `relay-peer-names.py --write-exceptions` for the peers present that day. The tool refuses when owners were not read, never overwrites, and leaves `review_by` empty.
   - **New peers.** A peer that joins later is excepted only by an entry the operator adds.
8. **No change reaches a running peer.** This decision removes, re-keys, renames and regroups nothing. Behavior changes only at mint time and in what the surfaces report. Anything that would change a running peer goes on the operator's list instead.

9. **SSO login (door 9) records an owner through its identity.** A peer admitted by SSO login carries `peer.user_id`. The reconcile treats that peer as recorded only when the NetBird user maps to a Kratos identity whose `metadata_public.aff` names an organization. Until that mapping is verified on a goatnet that enables SSO login, its SSO peers are graded unrecorded.

## Alternatives considered

- **Refuse an ownerless peer inside NetBird at peer-add, with a fork patch.**
  - This would close every door at once, including door 8 and the keys written straight into the database.
  - ADR 1248 commits goat to stock NetBird and to retiring its departures, and this would be a new one.
  - The existing `user_id` stamp patch cannot carry an owner, because NetBird checks that field against its own users table (F-237).
  - Kept as the option to propose upstream if upstream offers an owner or approval hook.
- **NetBird's peer approval (`extra_peer_approval_enabled`).** It would hold a new peer until an admin approves it. It records who approved, not who is responsible. It depends on the edition in use. Every goatnet sets it to false at account creation. Rejected as the owner record. It may complement decision 5 later.
- **Remove or quarantine the existing ownerless peers now.** Rejected by the operator on 2026-11-16: nothing running is disrupted. Decision 7 lists them instead.
- **Rely on the peer-name grammar alone.**
  - The name is evidence, not a record.
  - Doors 3 to 6 and 8 set no grammar name.
  - A hand-made key can carry any name.

## Consequences

**Positive.**
- Every peer that joins after the list's date traces to a person and an organization, or is reported within one cycle.
- The key id ties a peer to the act that admitted it, not just to a name.
- Infrastructure peers have owners on record.
- Owner exceptions make every gap countable and visible, each with a reason and the name of whoever granted it. A stale exception is reported, so the list does not outlive its reason.

**Negative.**
- Five tools change at mint time and each needs an owner argument, which makes bootstrap and bench work one step longer.
- Door 8 stays physically open. A NetBird admin can still mint a key, and the result is detected, not prevented.
- The reconcile depends on NetBird's peer-add activity events, which a deployment that prunes its activity log could lose. Retention must cover at least one cycle.
- Until the console stamps a handle on each identity (designed in `peer-identity-resolution.md`), person devices without a slot link resolve only through the roster fallback.

## Assessor view (SCA-R)

| Family | Control | How this decision bears on it |
|---|---|---|
| IA | IA-2, IA-3 (identification and authentication of users and devices), IA-4 (identifier management) | A device identifier (the peer and its key) is bound to a responsible person and organization at issuance (decisions 1 and 3) |
| CM | CM-8 (system component inventory) | Every mesh peer is graded each cycle as recorded, declared, excepted or unrecorded (decision 5); the declared registry is the inventory of infrastructure peers (decision 2) |
| AC | AC-2 (account management) | Key minting is limited to owner-recording paths (decision 3); an admin-minted key is detected (decision 4) |
| AU, SI | AU-6, SI-4 (monitoring) | An unrecorded peer is a paged critical finding with a metric whose expected value is 0 (decision 5) |

**Residual risk, stated plainly:**
- Excepted peers keep their access until the operator records an owner or decides otherwise.
- A NetBird admin can admit a peer that is detected within one cycle but not refused.

## Status of each decision

| Decision | State |
|---|---|
| 1 rule, 4 door 8, 8 no change to running peers | written here |
| 2 declared-owner registry | designed, not built |
| 3 key ids recorded at each mint; bootstrap default of 1 use | designed, not built |
| 5 reconcile and metric | built (`relay-peer-names` grading, `goat_peer_owner_*`, rules `peer-owner.yml` with promtool tests); the key-id tier waits on decision 3 |
| 6 resolver from the record, host carrier | built for machines and slot-linked devices (portal resolver; relay-peer-names); the surface finding is designed |
| 7 owner exceptions | built: schema, synthetic example, `--write-exceptions`, `goat_peer_owner_excepted` and `_exception_stale`, the card's `owner_exception`, and `PeerOwnerExceptionStale`; each goatnet's file is written on its first reconcile day. The topology page shows it (goatstuff) |
| 9 SSO login mapping | designed, not built |

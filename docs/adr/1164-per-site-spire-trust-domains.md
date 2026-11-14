# ADR 1164 — A SPIRE trust domain per site, named before the first federating party exists

- **Status:** **Proposed.** Nothing in a live goatnet changes on merge. The decision this records is the *naming*; the build it enables is separate work.
- **Date:** 2026-09-06
- **Deciders:** operator
- **Evidence:** `ops/ansible/group_vars/zagros_hosts/spire.yml:7` · `ops/ansible/roles/spire/defaults/main.yml:17,42-45` · `docs/project/zagros-spire-implementation-plan.md:9` · F-424
- **Amends:** ADR 0412 — its Phase A shape is one issuer per goatnet, co-located on a single host. This scopes the trust domain to the site instead, which is the boundary a partition actually falls on.
- **Related:** [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) · [ADR 1084](1084-block-64r-mgmt-repset-and-two-node-federation.md) · `docs/design/every-site-authoritative` program · `docs/design/federated-trust-model.md`

## TL;DR (for a layperson)

Every running program in the system carries a short-lived certificate naming who it is.
The first half of that name says which authority vouches for it. Today one authority
covers a whole environment, and it lives on one machine.

If a site loses its link to that machine, its programs keep working for one hour and then
stop being able to prove who they are. The fix is to give each site its own authority, so
a cut-off site can keep vouching for its own programs, with the sites recognising each
other's authorities when they can talk again.

The part that has a deadline is not the building — it is the **naming**. The name is
baked into every certificate issued and into every agreement with an outside party. While
no outside party holds our name, changing it is our own work on our own schedule. Once
partners hold it, changing it needs their cooperation too. So the name gets decided now
and built later.

## Context

Workload identity on ZAGROS runs on SPIRE. `spire_enabled: true` appears on one
mgmt host per goatnet, and that host is the
sole issuer for its goatnet.

Three configured lifetimes set the partition behaviour
(`ops/ansible/roles/spire/defaults/main.yml:42-45`):

| Setting | Value | Meaning under partition |
|---|---|---|
| `spire_default_svid_ttl` | `1h` | A workload cut off from its server loses identity one hour in |
| `spire_agent_svid_ttl` | `360h` | The agent survives power-off or relocation for 15 days |
| `spire_ca_ttl` | `2160h` | CA rotation clock; `spire-trust-sync` keeps the router trusting the rotating root |

One hour is the binding number, and on ZAGROS it is load-bearing:
`onboard_runner_mtls_enabled` and `id_admin_console_mtls_onboard_enabled` are both `true`
on `zagros_hosts` and `zagros_cloud_hosts`, so the onboarding path itself depends on a single
issuer. `container-orchestration.md:63` states the standard this fails —
*"Every site must continue to enroll peers, change policies, and run hosted services
during WAN partitions"* — the same test used to reject consensus-dependent orchestrators.

The trust domain is currently goatnet-wide. `group_vars/zagros_hosts/spire.yml:7` hard-sets
`zagros.zagros.example`, overriding a role default that would otherwise derive it as
`{{ site_id }}.{{ site_dns_zone }}`. The role's own comment marks the name **locked before
the first org onboard**, and `zagros-spire-implementation-plan.md:9` gives the reason:
*"Trust-domain renames cascade through every SVID + federation relationship + the
OIDC-discovery bridge."*

## Decision

### D1 — The trust domain is scoped to the site, not the goatnet

A site that must survive a partition runs its own SPIRE server under its own trust domain,
derived as the role default already computes it. The goatnet-wide override in
`group_vars/zagros_hosts/spire.yml` is retired in favour of that derivation.

### D2 — Sites recognise each other by SPIRE federation, not by trust-bundle concatenation

Cross-site workload authentication uses the `federation` block already present in
`roles/spire/templates/server.conf.j2` — a bundle endpoint on the `https_spiffe` profile
plus a `federates_with` entry per peer — enabled by `spire_federation_enabled`. Only public
bundles cross; no private key leaves a domain.

Concatenating a second root into a router's trust file is **not** the mechanism, for the
reason F-424
gives: the Zenoh ACL matches CN strings and has no issuer matcher, so a second root can
mint any CN and satisfy any rule. That constraint governs how recognition is wired; it does
**not** make intra-goatnet federation improper, because both roots are ours and the router
already carries the offline CA alongside the SPIRE CA today.

### D3 — Recognition is not authorization, and the mapping is written separately

A federated site's SVIDs are recognised by construction. What they are permitted to do is a
separate artifact, per [`federated-trust-model.md`](../design/federated-trust-model.md).
This ADR does not grant a peer site anything.

### D4 — The naming decision lands now; the build does not

Merging this ADR settles the name and changes no running goatnet. Standing up a second
server, enabling federation, and re-issuing entries are separate changes with their own
verification.

The asymmetry that justifies deciding early: every consequence of a rename is our own work
**except one**. A partner's `federates_with` block names *our* trust domain, so
re-establishing it requires them. That count is zero today and rises with every org
onboarded and every moon-pod that federates — including the partner-pod shape in
`zagros-spire-implementation-plan.md` §92-133, where a partner runs its own domain and
federates with ours.

### D5 — A second server at one site is HA, not this

Adding a second SPIRE server at the same site for availability is standard SPIRE HA against
a shared datastore. It needs no federation, no new trust domain, and it is not a partition
answer. Keeping the two apart prevents an availability change being mistaken for an island-mode one.

A shared datastore here means one database instance that every server reaches and sees
identically. The `spire` database as deployed is one instance on one node, so two servers
sharing it would still depend on that node. A copy replicated by Spock is not a shared
datastore: replication is asynchronous with last-write-wins, which SPIRE does not expect
(see Alternatives). The other shape, two independent servers with separate datastores in
one trust domain behind
[spire-ha-agent](https://spiffe.io/blog/2026-07-19-bottom-turtle-ha-architecture/), needs no
shared datastore. [ADR 1236](1236-a-sites-spire-server-is-made-redundant-by-two-independent-servers-after-a-gated-trial.md)
proposes it, gated on a trial; it is not built.

## Alternatives

- **Keep one trust domain, replicate the `spire` datastore multi-master.** Rejected: SPIRE
  expects a *shared* datastore, not a last-write-wins replicated one, and a shared datastore
  is exactly what a partition removes. It also trips the reconciler gap below.
- **Keep one issuer, rely on promotion.** `spire-ca-standby.yml` already stages the CA key
  to passive members and `spire-failover-promotion.md` promotes. That covers node *loss*
  and does nothing for a partition, where the far site has no issuer and no operator to
  promote one.
- **Leave the name and add per-site domains later.** This is the rejected option that
  costs the most, for the D4 reason.

## Consequences

- **Every issued SVID, registration entry, CA and bundle is re-minted** when the name
  changes. Both ends of the runner↔console allow-list move with it, along with four
  hardcoded `default('zagros.zagros.example')` fallbacks in the console and runner
  templates — a rename that misses those leaves the old name as a silent fallback.
- **N sites means N CA keys to custody.** The `spire-ca-standby.yml` staging problem
  multiplies, and its snapshot-not-stream caveat applies per site.
- **A latent reconciler defect must be fixed before any server shares a datastore.**
  `roles/spire/tasks/main.yml:348-352` checks for an existing entry with
  `entry show -spiffeID` and nothing else, creating only when the output reads
  `Found 0 entries`. Entries are parented per host — `spire_agent_node_path` is
  `agent/{{ inventory_hostname }}` — so a second site sharing that view would see the
  first site's entry, skip creating its own, and leave its workloads with no SVID while
  the deploy reports clean. Adding `-parentID` to the filter is the fix.
- **The OIDC-discovery bridge issuer changes with the name**, per the implementation plan.

## What this ADR does not decide

Envelope-bounded local admits, the portal registry's replication, the mesh-plane
admit/revoke classification, and the effect-deduplication design. Those are ADRs 1165,
1166 and 1167 in the same program.

---

*See `GLOSSARY.md` for terms and acronyms.*

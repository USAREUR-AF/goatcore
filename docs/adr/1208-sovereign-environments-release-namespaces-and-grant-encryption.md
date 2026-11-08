# ADR 1208 — Each sovereign runs its own environment, shares only by deliberate release, and seals restricted data to its intended receivers

- **Status:** **Accepted.** Nothing in a live goatnet changes on merge. The migration and the deployment below are separate, operator-approved changes.
- **Date:** 2026-10-30
- **Deciders:** operator
- **Amends:** [ADR 1075](1075-producer-data-labels-and-access-request-loop.md) (the `access` label) · [ADR 1094](1094-grant-keyed-payload-encryption.md) (the go-live condition) · [ADR 1027](1027-partner-release-set-governance.md) (the condition restated in `wire_read_posture.hard_floor`)
- **Related:** [ADR 1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md) (static ACL, slot pool) · [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (boundary nodes) · [`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md) · [`data-access-roles.md`](../design/data-access-roles.md) · `grant-keyed-encryption.md`

## TL;DR (for a layperson)

Each country runs its own copy of this system and keeps its own data there. Data crosses to another
country only when its owner deliberately releases it, through a named release area and a boundary
node built for that one partner. Inside an environment, data that must reach only certain readers
is encrypted to those readers, so a message forwarded by mistake can be opened only by the people
it was meant for.

## Decision

Operator's words, 2026-10-30: "each sovereign would only run their own data in their own network and
share only through deliberate relase"; "encrypt
for grants so that if a message were inadvertently forwarded, only the [original intended] receiver
should decrypt".

1. **One sovereign, one environment.** Each sovereign's data runs in that sovereign's own
   environment, with its own CA, management plane and identity domain.
2. **A sovereign's production environment hosts only that sovereign.** A site belonging to another
   nation runs in that nation's own environment, and allied data arrives only by release from the
   allies' own environments.
3. **Cross-sovereign sharing has one path:** `release/<set>/**` namespaces, carried over federated
   boundary nodes (ADR 1157). Release is a deliberate act by the data's owner.
4. **Encryption to grants is the per-reader control.** The router ACL is static and changes only
   with a restart, so read grants never go into it. A sealed message can be opened only by the
   receivers granted when it was published, however it is forwarded or stored. A grant names its
   intended receivers; to reach one receiver, it names one.
5. **Encryption is a standard production control.** It does not wait for an environment to host
   organizations that cannot share with each other. This replaces the
   "two non-releasable organizations" go-live condition.
6. **The `access` label is not an access control.** `community`, `org` and `controlled` mean nothing
   to Zenoh or goat, and nothing reads them to allow or refuse access. The label may become relevant
   to a consumer data persistence system; it is not defined for that use. The grant rows of ADR 1075
   stay: they name the audience the key service seals to.

7. **A multinational common environment is a separate, supported case.** Each sovereign has total
   control of all data in its own environment. If a multinational organization such as an alliance
   runs a common environment, the data in it is visible to that environment's members by default, as the
   draft NATO policy proposes, and release lanes, the partner read partition and encryption to
   grants narrow it. No such environment is planned yet.

## Alternatives considered

- **Per-reader read rules in the router.** Rejected: every grant change would need a router restart (ADR 1025, F-181).
- **Separate sovereigns sharing one environment, relying on labels or read partitions.** Rejected: the environment boundary is the trust boundary, and a label enforces nothing. A common environment run by a multinational organization is decision 7.
- **Hold encryption until an environment hosts two non-releasable organizations.** Rejected: forwarding exposes a message whoever else shares the environment.

## Consequences

- A production environment never hosts another nation's site. A node located in another nation's territory stays in its own nation's environment.
- `grant-keyd` runs on production environments. Its release path is goatbuilder, and `ops/goatbuilder/endstate/baseline.txt` stops carrying it as undeclared.
- Grant encryption across a federated boundary is open under F-481.
- The portal's `access` label code and its catalog badge remain until a separate change removes them.

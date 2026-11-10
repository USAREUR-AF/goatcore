# ADR 1092 — MOON and POD mesh peer classes for attached goatnets and appliances

> **Scope: a moon pod is a learning tool.** Partners use it in an exercise environment to learn how goat works. It is not the operational path for sharing data between nations. Operationally, each sovereign runs its own environment and manages its own pods, and data passes between sovereigns only across boundary nodes that federate their environments ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)). Where this document describes a pod joining another nation's environment, read it as the learning setup.

| Field | Value |
|---|---|
| Status | Accepted (2026-07-17) |
| Date | 2026-07-17 |
| Related | mesh-and-ory-roles.md, [goat-moon-pod.md](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md), ADR 1043 (node-class dimension), [ADR 1087](1087-advanced-grant-catalog.md) (router-peer grant), [ADR 1020](1020-zenoh-federated-peering-trust-model.md) (federated peering trust) |

## Context

Two attachment shapes need mesh vocabulary; without it their peers get
mis-classed into person roles:

- A **moon**: another host (or site) running its own complete goat-net stack —
  its own mgmt plane, identity stack, and CA — that securely passes data with
  us but is **not** a federated node inside our mesh. Its entire mesh presence
  here is one uplink peer enrolled purely to move data.
- A **pod**: a node **inside** our mesh following the moon-pod guidance
  ([goat-moon-pod.md](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md)) — it uses **our** NetBird,
  our Ory, our X.509 cert CA, and may run its own local Zenoh router for the
  systems behind it.

`OPSYSTEM` (our deployed mission machines) and `EXTERNAL-INFRA` (services
everyone reaches one-way) both misdescribe these: a moon/pod is not ours to the
same degree as an OPSYSTEM, and it is a *client/publisher* peers never need to
reach, not a service.

## Decision

**Two new canonical node-class suffixes, declared in the taxonomy and stamped
per prefix like every canonical group:**

- **`MOON`** — the uplink peer of a foreign-stack goatnet. Reach: initiates to
  `INFRA` on the peer-facing service ports only (tcp `443/53/5353/7447`, udp
  `53/5353/7448`, ICMP). **Nothing initiates toward it** — not even `INFRA`:
  the box is sovereign (typically partner-operated), and its health is
  reveal-only (it publishes; we don't probe), consistent with the
  environment-health doctrine. No external-infra reach, no lateral reach,
  never P2P-eligible.
- **`POD`** — a moon-pod-guidance appliance inside our identity planes.
  Reach: OPSYSTEM-grade substrate access (same rule-A source ports as MOON)
  **plus** it is a normal rule-A destination (`INFRA → POD` any port — it's our
  mesh member; we probe and administer it) and a P2P-rule destination (admins
  can reach it laterally). **Excluded from the external-infra rule** —
  appliances have no standing need for collab servers; grant later if a real
  need appears (fail-closed principle). Peering our fabric into the pod's own
  local Zenoh router is **not** base-group reach — it is the `router-peer`
  graduated capability (ADR 1087 catalog), per-pod, audited.

Identity-grammar placement (ADR 1043): both are **node-class** values, joining
the SENSOR/EFFECTOR/PROCESSOR/PLATFORM family. Affiliation of the operating
partner stays registry data — never in the group name. The moon's *own*
operators never touch our planes at all (their identities live in their stack);
a pod's operators are onboarded normally into our Ory.

Replication: canonical suffixes, so site bootstrap and the exercise stamp
template carry `MOON`/`POD` to every prefixed environment (locations,
exercises, activities). Probation (ADR 1089) applies: a dormant moon/pod uplink
sheds into `PROBATION-MOON`/`PROBATION-POD` like any other principal.

## Alternatives

- **Keep uplinks in `DEVELOPER`/`OPSYSTEM`:** the status quo; conflates person
  roles / our-machines with foreign-stack and partner-operated principals, and
  grants reach they don't need (external-infra, probe-ability of sovereign
  boxes). Rejected.
- **One combined `ATTACHED` class:** loses the load-bearing distinction — POD
  is administrable and inside our planes; MOON is sovereign and reveal-only.
  Rejected.
- **Model the moon as `EXTERNAL-INFRA`:** wrong direction of service — moons
  initiate, they aren't reached. Rejected.

## Consequences

Two groups + their matrix rows per goatnet; an existing uplink moves
`DEVELOPER → MOON` by the standard expand→verify→contract discipline
(mirror-first: add MOON, verify the uplink publishes across a window, then drop
DEVELOPER). Future moon/pod enrollments get setup keys minting the correct
class from day one. The classes carry trust semantics the ACL can now express;
what they deliberately do **not** carry is data-plane trust — fabric admission
for moon-published data remains the ADR 1020 gateway posture, and the pod's
router peering remains grant-gated (ADR 1087).

# ADR 1216 — A site network engineer reaches that site's devices one way, on declared ports, through `NET-ENGINEER`; `EDGE-SERVER` is a delegable class with `NETWORK`'s shape

| Field | Value |
|---|---|
| Status | Accepted. No live effect on merge: the groups exist only once a scope is re-applied, and no person or machine is declared into them |
| Date | 2026-11-02 |
| Deciders | operator |
| Repo scope | goat core (mesh ACL declaration, identity grammar, portal mint, lifecycle sweep) |
| Amends | [ADR 1113](1113-network-node-class-and-delegated-network-admin.md) §3 (the delegable set) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (warrant), ADR 1124 (cross-scope), [ADR 1126](1126-acl-assertions-rechecked-when-nothing-is-written.md) (assertions), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (`NETWORK-JUMP`), [ADR 1136](1136-dormancy-lifecycle-quarantine-and-revoke.md) (the sweep), [ADR 1189](1189-netbird-read-only-dashboard-and-warrant-owned-membership.md); design mesh-and-ory-roles.md, [mesh-change-control.md](../design/mesh-change-control.md) |

## TL;DR (for a layperson)

A site's network engineer needs to log in to the switches, C2 consoles and support servers at their site. Without a declared path, that gets done with policies typed into the NetBird dashboard that let every machine on the goatnet reach one device on every port. This decision gives the engineer a named group instead: their laptop can open SSH, NetBird SSH and remote desktop to their own site's devices, and ping them, and nothing else. The devices cannot reach back to the laptop. A new device class, `EDGE-SERVER`, names the servers that support edge devices, so they stop borrowing a class that does not describe them.

## Context

A policy created by hand in the NetBird dashboard with source group `All`, protocol `all`, and one peer as the destination makes one device reachable by every enrolled machine on the goatnet, on every port. Such policies appear because an engineer needed to manage a device and the declared ACL had no path for a person whose job is network administration: the only any-port path to `NETWORK` gear is `PLATFORM-ADMIN`, which is full-mesh authority.

Three gaps sit behind that:

1. **No person group for network engineering.** Role groups (`DEVELOPER`, `MONITOR`, `OBSERVER`) give substrate reach; `PLATFORM-ADMIN` gives everything. Nothing grants "administer this site's devices".
2. **No class for servers that support edge devices.** Such a server is not substrate (`INFRA` reaches everyone any-port in return), not bearer gear, and not a gear function. With no class, it lands in whichever group made it reachable.
3. **No port vocabulary for device administration.** `shell` (tcp 22) is the break-glass hop; there is no set that carries NetBird SSH (22022) or RDP (3389).

## Decision

1. **`NET-ENGINEER` — a PERSON group.** Network engineers' laptops. Site-scoped: every rule is written per scope and resolves to `<SCOPE>-NET-ENGINEER`, so an `alpha` engineer reaches only `alpha`'s devices. Never held by gear or any machine (the live-membership guard fails gear wearing it). Additive to the holder's participant role, which carries their substrate and DNS reach. Mesh-only: no Ory group, no cert role. No person is declared into it by this change.
2. **`EDGE-SERVER` — a NODE CLASS** for named servers that directly support edge devices. Exactly `NETWORK`'s shape: `PLATFORM-ADMIN` and `INFRA` reach it any port, one way; it initiates only toward `INFRA` on port-set `netadmin` plus icmp; no fabric ports, no lateral reach, never P2P. In a scope with a host site it mirrors `NETWORK`'s ADR 1124 rules too (management plane toward `HOST:INFRA`, and `HOST:PLATFORM-ADMIN` downward). Class name and group name are the same. Added to every closed enum `NETWORK-JUMP` was: portal registry (`infraNodeClasses`, `nodeClassMeshGroups`), `ops/identity/grammar.yaml`, `goat machine enroll --help`, the node-class membership guard, the membership reconciler, and the gear-acting-group scope guard. **Delegable beside `NETWORK`** under the ADR 1113 network-admin entitlement; `BREAKGLASS` and `NETWORK-JUMP` stay operator-gated.
3. **Port-set `device-admin`** in `ops/warrant/port-sets.yaml`: tcp 22, 22022 (NetBird SSH), 3389 (RDP; GNOME Remote Desktop uses the same port); udp 3389. Owner `platform-admin`. VNC (5900) is not in it. Ping is a separate icmp rule.
4. **Rules, one way, `bidirectional: false`**, generated through warrant into `ops/ansible/netbird_acl_matrix.yml`:
   - `NET-ENGINEER → [NETWORK, CONTROL, EDGE-SERVER]` on `device-admin` tcp, `device-admin` udp, and icmp — in the persistent goatnet set (each persistent goatnet by alias). In site scopes (every exercise-template scope) `NETWORK` and `EDGE-SERVER` come from the exercise template and `CONTROL` from the gear template, because `CONTROL` exists only where `gear: true`.
   - `EDGE-SERVER`: `platform-admin → edge-server` any; `infra → edge-server` any; `edge-server → infra` on `netadmin` tcp and udp; `edge-server → infra` icmp.
5. **Assertions** (ADR 1126) per scope — each persistent goatnet (by alias) and each site scope: the engineer reaches all three classes on 22, 22022 and 3389; opens nothing else on them (5900, 443, 7447 denied); reaches no mission class, person, P2P member or substrate shell; nothing in the three classes reaches the engineer; `EDGE-SERVER` is reachable by `PLATFORM-ADMIN` and `INFRA`, names its management plane, and has no fabric port and no lateral reach. At a site scope the engineer reaches nothing at the host goatnet.
6. **The lifecycle sweep never acts on these devices.** `ops/scripts/lifecycle-sweep.py` reports dormant peers in a `NETWORK`, `CONTROL` or `EDGE-SERVER` group in their own `exempt` bucket, with the lane they would have entered; no `--apply` lane quarantines or revokes them. A device already quarantined holds only its hold group, so the class is also read from its hold record's prior groups. A device in a hold group and a class group at once is reported as inconsistent, not exempt. `NET-ENGINEER` is a person group to the sweep: a laptop holding only it is on the person clock. Revoking a device is an explicit operator act on that device.
7. **A read-only guard** `ops/ci/check-no-all-any-port-policies.py` reads a NetBird policy list (file or stdin) and fails on any enabled rule with source `All`, protocol `all`, and a destination that is one device (a `peer` or `host` resource) or the group `All`, in either direction a bidirectional rule permits. A `subnet` or `domain` resource is a route, outside this guard. Its fixtures run in `repo-guards`; it is not on a schedule.

## Alternatives considered

- **Give engineers `PLATFORM-ADMIN` at their site.** Rejected: any-port to every peer in the scope, including people and mission gear, for a job that needs three ports on three classes.
- **Keep the hand-made policies and narrow their source.** Rejected: they would stay outside the declaration, unasserted and invisible to `goat acl test` and netvis.
- **Reuse `shell` instead of a new port set.** Rejected: `shell` is tcp 22 only and is the break-glass vocabulary; widening it would widen every `BREAKGLASS` and `NETWORK-JUMP` rule with it.
- **Class edge servers as `INFRA` or `SERVICES`.** Rejected for the reason ADR 1113 rejected it for switches: `INFRA` reaches every peer any port, which is the wrong authority on a site-exposed box.
- **Add `NET-ENGINEER` to the Ory role family.** Rejected: the grant is mesh reach, and a role-family value rides the Ory groups and the cert URI-SAN, which nothing here needs.

## Consequences

**Positive.** Device administration has a declared, port-scoped, asserted path, so the hand-made policies can be disabled once engineers hold the group. A site engineer's reach is bounded to their site by construction. Edge servers get a class that describes them and a site admin can enroll them without the operator. Dormant switches, consoles and edge servers are no longer one `--apply` from losing their membership.

**Negative.** `check-warrant-narrowing.py` grades this change a widening: every new permit is sourced from `NET-ENGINEER` or `EDGE-SERVER`, or targets `EDGE-SERVER`, and nothing else gains anything. Where hand-made policies exist, those devices remain reachable from `All` until they are disabled; the new guard names them. A dormant device in one of the three classes now has no automatic end; it relies on an operator reading the sweep's `exempt` bucket.

## Assessor view (SCA-R)

This replaces unrecorded any-port grants with least-privilege, role-based, one-way access (AC-6, AC-17 remote access over named services, CM-7 least functionality). Every grant is declared in version control, generated, and asserted on each change and against live state (CM-3, CA-7). The new guard detects the over-broad policy shape on demand (AC-6(9)-style review). Residual risk: any hand-made policies stay enabled until disabled; the compensating control is the guard's report naming them.

## Follow-ups

| Follow-up | Status |
|---|---|
| An alert when a `NETWORK`, `CONTROL` or `EDGE-SERVER` peer becomes able to expire (login or inactivity expiration turned on) | **Built.** `ops/compose/goat-obs/dormancy-report/dormancy_report.py` writes `goat_device_peer_expiry_enabled{env,class,kind}` (peer names to journald as `EXPIRABLE-DEVICE`); alert `DevicePeerCanExpire` in `prometheus/rules/dormancy.yml`; fixture tests `test_dormancy_report.py` + `tests/dormancy.test.yml`. Live where the dormancy-report cron runs |
| A `goat machine ssh enable` command that turns on NetBird SSH on an enrolled device | **Built.** `crates/goat-cli/src/cmd/machine_ssh.rs`; one `PUT /api/peers/{id}` with every other setting echoed (`NetbirdClient::set_peer_ssh`); audited as `machine.ssh.enable`; errors `E_MACHINE_UNRESOLVED`, `E_MESH_PEER_UNRESOLVED`, `E_NETBIRD_FAILED`, `E_NOT_ADMIN` |
| Membership: declare the site engineers into `<SCOPE>-NET-ENGINEER` through warrant membership | **Not built**: each deployment declares its own engineers per scope, then re-applies the scope so the group exists |

## Trigger conditions for revisit

- An engineer needs a port outside `device-admin` on one of the three classes (the set grows by review, not by a hand-made policy).
- A device class other than `NETWORK`, `CONTROL` and `EDGE-SERVER` needs engineer administration.
- NetBird gains nested groups or per-rule peer posture that would let one rule replace the per-scope copies.

# ADR 1114 — `BREAKGLASS` node class, and the two-path reach model that makes it worth having

| Field | Value |
|---|---|
| Status | **Accepted** — declared in the ACL taxonomy, the permission matrix and the enrolment mint |
| Related | [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md) (a site declares whether it has one; standup phase 14 converges it), [ADR 1113](1113-network-node-class-and-delegated-network-admin.md) (`NETWORK` tier — the template this follows), [ADR 1131](1131-network-jump-class-and-herd-gear-scope.md) (`NETWORK-JUMP`, the narrower class for reaching network gear), [ADR 1092](1092-moon-pod-peer-classes.md) (MOON/POD peer classes), [ADR 0843](0843-public-mgmt-signal-relay-mobile-crutch.md) (the named successor for the bad-day path) |

## Context

A host site needs a node whose only job is to be reachable when the things around it are not: a local jump host onto the core and observability nodes when the mesh or a host wedges, and onto the site's network gear when that misbehaves.

The obvious class for it is `INFRA`, and it is the trap. `INFRA` reaches everyone on any port, which is the jump-host requirement, and that is exactly why it is wrong. This node carries a **second, independent WAN transport** whose purpose is to stay up when the site's primary path is down. A node that is both WAN-attached and holds any-port reach to the whole mesh is the worst peer in the fleet to lose. [ADR 1113](1113-network-node-class-and-delegated-network-admin.md) made the same argument for network bearer devices and resolved it with a narrower tier rather than over-granting `INFRA`; this ADR follows that decision.

There is a second problem. Every NetBird rule is enforced **on the goatnet's own mesh**, and the jump host exists for the case where that mesh is the broken thing. A reach model expressed only in mesh policy describes the day you do not need it.

## Decision

### 1. Add `BREAKGLASS` to the node-class enum, as an infrastructure class

> **`BREAKGLASS`** — a node whose function is operator reach-in of last resort: dual-homed, minimal, running no mission or fabric workload, positioned so it can be reached and can reach the local substrate when the normal control plane is unavailable.

Infrastructure, not function: like `NETWORK`, a compromised break-glass node is a substrate-wide event, because its entire value is privileged reach during an incident. It is **not** delegable; unlike `NETWORK` under ADR 1113 §3, this class stays operator-only. The entitlement that lets a site admin enrol a switch must not let them enrol their own way into the rescue path.

The class describes what a node does **on the goatnet the class is scoped to**, not everything its hardware does elsewhere.

### 2. A `BREAKGLASS` group tier — `NETWORK`'s shape, plus exactly one hop

A new suffix in `netbird_acl_taxonomy.yml`, in both the canonical site template and the activity/site-set template:

- **Reachable** by `PLATFORM-ADMIN` on any port.
- **Reachable** by `INFRA` on any port.
- **Initiates** to `INFRA` and to `NETWORK` on `tcp 22`, plus **udp 5353 to `INFRA`** for mesh name resolution. A jump host that can reach the core nodes but cannot name them is not usable in an incident. Resolving a name is not publishing on the fabric.
- **No fabric ports** (7447/7448), in either direction. A jump host is not a publisher and not a subscriber.
- **No lateral reach** to `OPSYSTEM`, `OBSERVER`, `DEVELOPER`, `POD` or another `BREAKGLASS` peer.
- **Never in `P2P`.** P2P graduation grants `p2p → all`, which would restore the over-grant this tier exists to avoid.
- **The NetBird client's local resolver binds to loopback** (`--dns-resolver-address 127.0.0.1:53`). "No lateral reach to another `BREAKGLASS` peer" cannot tell another peer from the node itself: the node's own mesh address is in its own group, so the ban would swallow its own DNS listener and no mesh name would resolve. Loopback is in no group, so no ACL applies and no reach is widened. The setting survives a reboot but not a re-enrolment, which is why it is stated here.

Inbound is deliberately identical to `NETWORK`: reachable by the substrate and by platform admins, invisible to everyone else.

**The one deliberate divergence is `BREAKGLASS → NETWORK :22`.** ADR 1113 §2 forbids lateral reach *from* a `NETWORK` peer. This grants the mirror-image hop *toward* one, because reaching the switch when it misbehaves is half of why this node exists. It is one port, one direction, to one class.

### 3. Reach is defined as two paths, and both are named

| Path | When | Carried by | Governed by |
|---|---|---|---|
| **Normal-day** | mesh healthy; routine admin, verification, patching | the goatnet's NetBird mesh | §2 above |
| **Bad-day** | mesh, management host or ACL plane unavailable | an out-of-band transport on the second NIC, sharing **no control plane** with the goatnet | §4 below |

A node may not be declared `BREAKGLASS` with only the normal-day path configured. Without the second, it is an ordinary node with an unusual name.

### 4. The bad-day path today is a separate overlay VPN, declared interim

The bad-day path may be any overlay with its own control plane, independent of the goatnet's; a separate WireGuard-based overlay such as Tailscale qualifies. It earns the slot on one merit: a genuinely separate control plane, the only property that matters here.

It cannot be the end state, on two grounds:

1. **Crypto.** WireGuard's transport primitives (ChaCha20-Poly1305, Curve25519) are not FIPS-approvable. WireGuard is accepted for the intra-site, inside-the-trust-boundary role; a rescue path over a public WAN is not that role.
2. **Sovereignty.** A hosted overlay depends on a third-party coordination service. For a path whose purpose is working when our own control plane has failed, that is the harder of the two problems.

**Named successor: the public mTLS crutch tier** ([ADR 0843](0843-public-mgmt-signal-relay-mobile-crutch.md)). It authenticates with per-device X.509 certificates issued by the goatnet's offline CA under `RequireAndVerifyClientCert`, is operator-toggleable, and its primitives can run on a validated FIPS module.

**Sunset condition:** when the crutch tier, or an equivalent sovereign, FIPS-capable reach-in path, is active for a `BREAKGLASS` node and proven in a drill, the interim overlay is removed from that node, not merely disabled. Until then every `BREAKGLASS` node carries it as a declared deviation.

### 5. The dual-homing contract

- **Route-metric asymmetry.** The out-of-band interface takes a strictly higher route metric than the primary. It is a reach-in path, never an egress path; without this it silently becomes the default route.
- **Out-of-band inbound is closed by default.** Nothing on the node binds a service to that interface.
- **Health checks on the out-of-band path are TCP-level, never ICMP.** A cellular or satellite bearer can keep carrier, lease and ping green while TCP forwarding fails; a ping-based monitor reports healthy in exactly the case it exists to catch.

## Alternatives

- **Use `INFRA`.** Simplest, and grants any-port reach to everything from a WAN-attached box. Rejected for the reasons ADR 1113 gives.
- **Leave it as `POD`.** Describes a role the box does not have, and grants substrate reach it does not need.
- **`BACKEND-INFRA`.** Its inbound posture is close to right, but it is a class for passive hidden backends with no outbound leg. A jump host that cannot initiate is not a jump host.
- **Skip the class; grant per-host rules.** Works for one node; per-host rules are how a taxonomy rots.
- **Wait for the FIPS-capable path before declaring the class.** Leaves the node with no bad-day path defined, indefinitely.

## Consequences

- Each `BREAKGLASS` node carries a declared dependency on a separate control plane until the successor path is proven. That is a compliance exception and belongs in the waiver surface.
- `BREAKGLASS → NETWORK :22` only means something once the site's network devices sit in the `NETWORK` tier.
- The class asserts a property only a drill can verify: reaching the node over its bad-day path, and from it the core nodes by name, while the mesh is actually down.

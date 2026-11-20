# ADR 1227 — A platform admin may read a goatnet's fabric through its door, as an admin workaround only

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-10 |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | ADR-1117 (per-env doors), ADR-1019 (goat CLI), ADR-1085 (relay ingress dispatch), ADR-0843 (public mTLS crutch tier) |
| Implementation | Pull request #333, held as a draft until this ADR is decided |

## TL;DR (for a layperson)

A goatnet's data fabric can only be reached by machines on that goatnet's private network. Platform admins already have a back door into each goatnet for deploying software. This proposal lets an admin also use that back door to read the fabric, for example to check health after a deploy, from a laptop that is on a different network. It is a workaround for admins, not a way in for anyone else, and it adds a second, less protected route to each goatnet's router. The reviewers decide whether that is worth it.

## Context

A laptop is a peer of at most one goatnet's mesh at a time (ADR 1117). A goatnet's zenoh router listens only on that goatnet's mesh, so `goat health`, `goat sub` and the other fabric commands fail from a laptop meshed to a different goatnet. The supported ways to read a goatnet's fabric are to be a peer of its mesh with a `goat` profile, or to use the goatnet's own panoscope or stethoscope.

Deploys do not have this problem, because they ride a per-env door (ADR 1117, `docs/design/env-doors.md`): an SSM port forward to the goatnet's management host, then SSH. Only platform admins hold what a door needs, which is the project's cloud account and an SSH key on the management host.

After a release reaches a goatnet the admin's laptop is not meshed to, the admin has no way to read that goatnet's health from the laptop. The question this ADR settles is whether the door should also carry fabric reads, and if so, in what form.

## Decision (proposed)

1. **`env-door.sh <goatnet> --forward <ip>:<port>`.** With the door open, this forwards a local port (`<port> + 10000`, or `GOAT_FORWARD_PORT`) to one service on that goatnet's mesh, through the management host. The hop to the door port skips its host key check because that key churns; the outer SSH checks the management host's own key.
2. **`goat --router <endpoint>`.** This replaces the context's `router_endpoint` for one command and prints a line on stderr saying so. The certificate, key and CA roots still come from the context. Every reader of the router endpoint resolves it through one function, `profile::resolve_router_endpoint`.
3. **Admin-only, and documented as such.** The feature is described only in `docs/design/env-doors.md`, as a platform-admin workaround. It does not appear in tutorials, onboarding, the goat gym or any person-facing guide, and it is never the answer to "how do I read a goatnet's health".
4. **Open question for review:** whether `GOAT_ROUTER_ENDPOINT` (the environment variable form) is kept. It lets a shell silently redirect every command; the flag alone does not.

## Alternatives considered

| Alternative | Why not chosen, or what it costs |
|---|---|
| **Do nothing.** The admin re-meshes the laptop to the target goatnet, or reads health from that goatnet's panoscope | Re-meshing is what ADR 1117 exists to avoid: it interrupts concurrent work on other goatnets. Panoscope works and needs no change; it costs a browser session on that goatnet's mesh. This is the strongest alternative. |
| **Runbook only, no code.** A hand-written `ssh -L` plus a temporary profile directory with an edited `router_endpoint` | It works today with no product change, and was used once. It leaves an edited copy of profile files on disk and is easy to get wrong, but it adds nothing to the public CLI. |
| **The ADR 1117 Tier 2 zenoh-door lane.** An mTLS TCP lane through the relay ingress to the router, built for the zagros aggregator | It is designed for exactly this traffic, needs no cloud API at connect time, and fails closed to anyone without a CA-signed client certificate. It needs per-goatnet DNS, relay and Traefik configuration, and it is a permanently open public surface, gated only by the certificate. |
| **Run `goat` on the management host** | It needs an operator certificate placed on a shared host, which is the key placement the credential rules refuse. |

## Consequences

### Additional complexity

- A new global flag that changes where every fabric command connects. A reader of a command's output can no longer assume it came over the mesh; the stderr line is the only marker, and `--json` output does not carry it.
- A second transport stacked on an existing one: the SSH forward rides the SSM door, so the forward's lifetime is the door's lifetime.
- One more code path to keep consistent: any future reader of `router_endpoint` that bypasses `resolve_router_endpoint` silently ignores the override.

### Fragility

- **Idle timeout.** An SSM session with no traffic closes after about 20 minutes. The forward then dies, and both commands must be run again.
- **Network drops.** A laptop network interruption kills the door and the forward without notice; the next `goat` command reports the router as unreachable, which reads like a goatnet outage.
- **Single door peer.** The door depends on one cloud instance per goatnet and on the cloud provider's API being reachable.
- **Local port reuse.** Every goatnet's router forwards to the same default local port (17447). A forward left open for one goatnet, used with another goatnet's context, fails mTLS when the CAs differ. It would succeed silently if two goatnets ever shared a CA.

### Security concerns

- **The mesh stops being the network boundary for the router.** Today, reaching a router requires mesh membership, which the warrant-generated mesh ACL controls. With this, anyone holding the cloud account and an SSH key on the management host can reach the router from anywhere. The router's own certificate check (the client certificate's CN) still decides what may be read. The network-layer check is skipped.
- **Attribution moves.** Router and mesh logs record the connection as coming from the management host, not from the operator's own peer. Per-peer traffic attribution and peer-demand views no longer show who is reading.
- **Hostname verification stays off.** The endpoint is an IP address (`127.0.0.1`), so `goat` skips hostname verification of the router's certificate. Contexts whose `router_endpoint` is an IP address already skip it, and this makes that the normal shape for door reads. The certificate chain is still verified against the goatnet's CA in both directions.
- **Redirection by environment variable.** A `GOAT_ROUTER_ENDPOINT` left set in a shell, or planted, redirects every command. The router at the other end still has to present a certificate from the context's CA, so a redirect cannot impersonate a router without that CA's key. It can make a command fail, or connect to a different goatnet that shares the CA.
- **Normalizing the door.** Doors were justified as a deploy transport (ADR 1117). Carrying routine reads over them makes the door a standing way into every goatnet's data plane for whoever holds the cloud account. That raises the value of that account and of management-host SSH keys.

### Positive

- An admin can check a release on any goatnet from one laptop, without re-meshing.
- The operator's own certificate is used, so nothing is placed on a host.

## Assessor view (SCA-R)

The feature adds an access path to the data plane (AC-17 remote access, SC-7 boundary protection) that bypasses the mesh ACL and relies on two existing controls: the cloud account's access policy for the door, and the router's mTLS client-certificate check. Audit attribution (AU-3, AU-10) is weaker on this path, because the network source is the management host rather than the operator's peer. Compensating controls: the path is limited to platform admins by the door's own credentials, the mTLS identity still names the operator, and the path closes on idle. An assessor would ask for door use to be logged per person (the cloud provider's session logs) and for door reads to be distinguishable in the router's logs.

## Trigger conditions for revisit

- A non-admin asks for fabric access from off-mesh: that is a different decision (onboarding, or the Tier 2 lane), not this one.
- Two goatnets come to share a CA.
- The Tier 2 zenoh-door lane is built for a second goatnet.
- Door session logs show reads outnumbering deploys.

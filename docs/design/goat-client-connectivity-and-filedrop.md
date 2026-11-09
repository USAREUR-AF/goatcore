# goat-client connectivity check + peer file drop — Tailscale-parity features for the inner mesh

**Status:** Designed-not-built. Authoritative ADR: [`1063-goat-client-connectivity-check-and-file-drop.md`](../adr/1063-goat-client-connectivity-check-and-file-drop.md) (proposed).
**Repo:** **v1 (both features) lands in [`USAREUR-AF/goat-client`](https://github.com/USAREUR-AF/goat-client)** (daemon + GUI); goat-core carries only this doc + the ADR. The one core-repo code change is **phase 2** (offline store-and-forward), which touches the Rust relay crates `goat-relay` / `goat-relay-protocol` — v1 basic transfer needs none of it. Rough proportion: ~95% goat-client, ~5% core, and that 5% is deferred.
**Parent design:** [`goat-client.md`](goat-client.md) (the daemon + GUI this extends). **Substrate levers:** [ADR 0709](../adr/0709-block-33-relay-rewrite-architecture.md) (the Rust relay), `identity-grammar.md` (identity-grade authz), [`operator-action-audit.md`](operator-action-audit.md) (audit trail).

---

## 1. What this is, and why now

The Tailscale desktop app shows a per-device detail pane with two affordances we want and NetBird does not provide: a **live connectivity check** ("Direct connection — 4 ms" with a rolling latency graph and a direct-vs-relayed badge) and **Taildrop** (drag a file onto a device to send it peer-to-peer). Both are everyday-useful, both set the bar for "feels like a finished mesh client," and both are missing from stock NetBird.

This document scopes what it takes to reach **functional parity** on those two features inside `goat-client`, sets honest expectations about where we land relative to Tailscale (including one capability we cannot match and several we exceed), and records the inner mechanisms so a future implementer starts from understanding rather than re-derivation.

The conclusion up front: **both features are small, well-understood builds**, because we already own every hard primitive each one needs — a stable per-peer tunnel IP, source-IP-as-identity authentication from the mesh, NetBird linked as a library inside `goat-clientd`, and our own Rust relay. On the relay path we do not just match Tailscale, we beat it.

## 2. Scope boundary — these are inner-mesh features

Both features operate on **inner-mesh peers** (the NetBird overlay), not on the wg-cp0 outer tunnel. The outer tunnel is single-peer (you ↔ the silent control plane); there is no peer roster to ping or drop files to. So both features are live only in `goat-client`'s `netbird-only` and `combined` modes (see [`goat-client.md`](goat-client.md) §"Operating modes"). In `wg-cp0-only` mode the connectivity/drop UI is dormant (the device list is empty), exactly as the tunnel-status pane already degrades.

This is a deliberate nudge to [`goat-client.md`](goat-client.md) §2, which lists "mesh-service-level UX" as out of scope for v1. A peer ping and a peer file-drop are *mesh-peer* UX, not *mesh-service* UX (they target peers, not hosted services like `mgmt.<site>`), and they are scoped post-v1 here, not into v1. The boundary stands; this names the seam.

## 3. The state of the art we are measuring against

Tailscale's two features, mechanism-first. Each claim below is labeled **[evidence]** (read from Tailscale's open source / docs) or **[inference]** (reasoning not directly confirmed).

### 3.1 The Ping panel = two independent data sources

**[evidence]** The panel stitches together two things that come from different places:

1. **The latency number + live graph** comes from **repeated disco pings**. "disco" is a small authenticated/encrypted control protocol Tailscale layers *inside* magicsock (the UDP socket multiplexer beneath WireGuard). A disco `Ping` carries a random transaction ID; the `Pong` echoes that ID and adds the source address the receiver saw (a STUN-like field). RTT = monotonic-clock delta between ping-sent and matching-pong-received, kept in a per-endpoint ring buffer. `tailscale ping` surfaces this; the GUI graph is the same ping issued on a timer (**[inference]** on the ~1 s cadence; the mechanism is evidence).

2. **The "Direct" vs "Relayed" badge** comes from **transport-level path knowledge**, not from the ping latency. Tailscale races disco pings across all candidate paths (direct UDP from STUN / local interfaces / port-mapping, peer-relay, and DERP) simultaneously; whichever returns a pong, and how, is the path signal, ranked direct > peer-relay > DERP then by latency. The cheaper `tailscale status` already carries the current path (`CurAddr` set = direct; `Relay` set = DERP region) **without** issuing a ping.

**[evidence]** The load-bearing insight: an app-layer echo over the tunnel IP **cannot** see direct-vs-relay, because WireGuard hides the underlay — both look identical at the overlay IP. Path detection requires asking the transport for the peer's current underlay endpoint. The latency number does *not* need transport introspection; the path badge does.

### 3.2 Taildrop = an HTTP file server on the tunnel IP

**[evidence]** Stripped to essentials, Taildrop is:
- An HTTP server (the "PeerAPI") every `tailscaled` runs, **bound to its own tunnel IP**, reachable only over the tailnet.
- A `PUT /v0/put/<filename>` handler that streams the request body straight to an incoming directory.
- A send path that PUTs to `<peer-tunnel-ip>:<peerapi-port>/put/<name>`.
- **Authentication = `WhoIs(source-tunnel-IP)`**: the daemon maps the HTTP connection's source overlay IP back to a known node/user in the netmap. Because you cannot put packets on the tunnel with a peer's source IP without its key, **the connection's source address *is* the authenticated identity** — there is no separate token.
- **Authorization policy** = same-user only (you can send between your own devices, not to another user's, not to/from tagged nodes).

Incidental polish, not load-bearing: the CRC32-derived PeerAPI port (a fixed well-known port works for an MVP), partial-file-hash resume, and the push-vs-prompt accept UX (**[inference]** that the accept workflow is platform-dependent; the push transport is evidence).

## 4. What goat-client / NetBird already give us

**[evidence, from the goat-client repo + the NetBird fork]**

| Primitive both features need | What we have today |
|---|---|
| Stable per-peer overlay IP | NetBird `wt0` / 100.x address per peer — the foundation of both features. |
| Source-IP-as-identity auth | NetBird's mesh provides it; `netbird status --json` maps overlay IP → peer/pubkey/fqdn. This is the `WhoIs` analog, free. |
| In-process peer status | `goat-clientd` already links NetBird as a library (`internal/innermesh/`) in `netbird-only`/`combined` modes — we read connection-type/ICE/handshake in-process, no CLI shelling. |
| Daemon↔GUI IPC | `internal/ipc/` JSON-RPC over unix socket / named pipe; trivial to extend with new methods. |
| A probing pattern | `internal/reachability/prober.go` exists — but it targets **wg-cp0 relay endpoints for dial-selection** and deliberately discards RTT jitter. It is a reference pattern, **not reusable** for a live peer graph. |
| GUI shell | Fyne desktop app (`internal/ui/`) — today renders one or two status cards, **not** a device roster with a per-peer detail pane. |

**[evidence]** What NetBird does *not* give us, and we must build:
- **A usable live latency.** NetBird's per-peer `latency` is a one-shot, written only at ICE candidate-pair selection (`worker_ice.go`), never re-sampled, and **absent on the relay path** (no ICE agent → never written). There is no `netbird ping` and no Ping RPC. This is the gap the peer-echo prober fills.
- **Any file transfer.** None exists; greenfield. (NetBird's guidance is "scp/sftp over the mesh," which is not a one-click drop.)

**[evidence]** What NetBird *does* give us as a read (so we don't rebuild it): `netbird status --json` already carries `connectionType` (`P2P`/`Relayed`), `iceCandidateType` {local,remote}, `relayAddress`, live `lastWireguardHandshake`, and cumulative `transferReceived`/`transferSent`. The path badge is a read, not a build.

## 5. Build plan

### 5.1 Feature 1 — Connectivity / Ping panel

| Piece | Where | Note |
|---|---|---|
| **Peer-echo prober** | new `internal/peerping/` in `goat-clientd` | UDP echo responder bound to the inner-mesh tunnel IP + a sender that stamps a monotonic clock and keeps a per-peer RTT ring buffer. This is *ours* — it fills NetBird's missing live latency. Both ends run goat-client, so both have the responder. |
| **Path/ICE read** | extend `internal/innermesh` status snapshot | Pull `connectionType` / `iceCandidateType` / `relayAddress` from the in-process NetBird status. The badge is read, not measured. |
| **IPC method** | `internal/ipc` | `getPeerConnectivity(peer) → {rtt_samples[], path: direct\|relayed, ice_type, relay_addr, last_handshake, bytes}`. |
| **Fyne detail pane** | `internal/ui` | A per-peer detail pane with a sparkline/line-graph widget (Fyne Canvas) + a path badge. The device-roster-with-detail layout is the bulk of the UI work and is shared with Feature 2. |

Result: faithfully reproduces "Direct connection — 4 ms + live graph." Latency line is ours; path badge is read from NetBird.

### 5.2 Feature 2 — File drop ("goatdrop")

| Piece | Where | Note |
|---|---|---|
| **Receive server** | new `internal/filedrop/` | HTTPS `PUT /put/<name>` bound to the inner-mesh tunnel IP (mesh-only, never public), streaming to a configured inbox dir. Mirrors the PeerAPI shape. |
| **Identity gate** | `internal/filedrop` + `internal/innermesh` | Resolve the connection's source overlay IP → peer via the NetBird peer list (the `WhoIs` analog); reject sources not in the allowed set. |
| **Send path** | `cmd/goat-client` + `internal/ui` | `goat send <peer> <file>` CLI + a drag-and-drop zone in the detail pane; resolve peer name → tunnel IP, HTTP PUT. |
| **Transfer status** | IPC + UI | An `IncomingFiles`-style in-progress list + progress bars. |

A fixed well-known port is fine for v1 (skip the CRC32 derivation); add resume + per-peer accept toggle as polish.

## 6. Expectations — where we land vs Tailscale

Two honest scorecards. The point of stating these explicitly is to set expectations before the build, not after.

### 6.1 What we CANNOT match at parity

1. **NAT-traversal hit-rate + global relay footprint — the real ceiling, and it lands on Feature 1.** **[evidence + inference]** Tailscale runs a global DERP mesh (dozens of POPs) and disco hole-punching hardened over years; magicsock continuously races candidate paths and upgrades relay→direct live. NetBird's ICE engine is coarser, and we run a handful of relays. Consequence for the Ping panel specifically: in adversarial / symmetric-NAT conditions our badge will read **"Relayed"** in cases where Tailscale would have achieved **"Direct."** We can match the *display* exactly; we cannot match the *hole-punch success rate* without reworking NetBird's ICE, and we cannot conjure a global anycast relay mesh from a handful of relays. **Mitigating fact:** our scope is a small, bounded number of sites with known network posture (ADR 0215), not a global consumer fleet — so "cannot match" is largely "do not need to."

2. **A ping that needs nothing on the far end.** **[evidence]** Tailscale's disco ping lives in the transport, so it can measure RTT to a peer running nothing but stock `tailscaled`. Our latency graph is an **app-layer echo** that needs our responder on both ends. Inside the closed goat-client fleet both ends are ours, so this is a non-issue in practice — but it is a genuine architectural difference: we cannot measure RTT to an arbitrary mesh peer that is not running our agent.

3. **Consumer-grade product breadth + app-store polish.** **[evidence]** The broader Tailscale product (MagicDNS, exit nodes, subnet routers, Serve/Funnel, share-sheet OS integration, mature app-store presence) is not in scope for these two features, but if "UX parity" is read as "feels as finished as the Tailscale app," we are not there. Our mobile signing is plumbed-but-dormant (no Developer-ID cert yet; see F-208 area). Closing that is sustained polish, not a feature.

Nothing in the two target features is *architecturally* impossible for us. The only hard ceiling is #1 (hole-punch hit-rate / relay POP density), bounded by our deliberate small-site scope.

### 6.2 Where we EXCEED Tailscale

1. **Relay-path throughput.** **[evidence, ADR 0709]** Our Rust relay targets ≥110 Mbps single-flow vs the old Go relay's 25–30, and DERP is not built for bulk. Any relayed transfer or relayed traffic is materially faster on our substrate — and basic transfer gets this for free, because the relay carries the encrypted tunnel transparently (no relay change needed). *Caveat: 110 Mbps is the ADR target measured on a t3.micro; the real ceiling is load-test-dependent.*

2. **Store-and-forward to OFFLINE peers.** **[inference, enabled by relay ownership]** Taildrop is strictly online-to-online. Because we own the relay, we can add a relay-side inbox so a file dropped to a currently-offline peer is delivered on reconnect. This is the "customize the relays" lever and a clean capability Tailscale does not have. It is the *one* place relay customization is load-bearing (basic transfer is not) — scope it as **phase 2**, after at-parity direct/relay-fallback drop ships.

3. **Identity-grade authz + attribution.** **[evidence]** Tailscale gates Taildrop on "same user" — source-IP identity, nothing more. We have the identity grammar (role / affiliation / echelon, cert URI-SAN slot anchors per `identity-grammar.md`); a drop or connectivity action can be authorized and *attributed* against real identity, not just mesh membership.

4. **Evidence-grade audit.** **[evidence]** Every drop / connectivity action can land on the audit fabric (Object-Lock, age-encrypted operator-action audit per [`operator-action-audit.md`](operator-action-audit.md)). Tailscale logs; it has no equivalent tamper-evident trail.

5. **Sovereignty / denied-environment operation — the categorical one.** **[evidence]** Tailscale's coordination plane is its SaaS and DERP is theirs; new-connection setup degrades without their control server. Our control plane is self-hosted, offline-CA-rooted, silent-by-design, and built to keep working through a management-plane outage and in island / disconnected mode. For our threat model this is not a feature edge — it is a category Tailscale structurally cannot enter. A drop and a peer ping between two peers on the same LAN keep working with the internet cut; Taildrop's path setup does not.

6. **PQC / FIPS depth.** **[evidence]** Rosenpass post-quantum is already a NetBird flag we carry, atop a FIPS-140-3 + PQC posture deeper than Tailscale's for our compliance target.

The exceed items (#2–#6) are exactly what justifies building these natively into goat-client rather than telling users "scp over the mesh."

## 7. Inner-mechanism reference (for the implementer)

The two things worth internalizing before writing code, because they shape everything:

- **The Ping panel is two sources, deliberately kept separate.** Latency = repeated app-layer echo (ours, because NetBird's number is a stale one-shot). Path = a status read (NetBird's `connectionType`, free). Do not try to infer the path from the echo — you cannot; the underlay is hidden under WireGuard. Do not try to get live latency from NetBird's status — it is frozen at ICE-pair selection and zero on the relay path.

- **File drop's auth is the tunnel, not a token.** Trust the source overlay IP. The mesh guarantees a peer cannot forge another peer's source IP without its key, so `WhoIs(source-IP)` is sufficient identity. Resist adding a separate auth token; it is redundant with what the mesh already enforces, and it would be the thing that rots.

## 8. Sequencing

1. **Fast validation first.** A throwaway peer-echo prober + a one-file HTTP `PUT` between two inner-mesh peers, proving the latency-graph mechanics and the drop transport end-to-end, before committing the UI build. (Matches the project's fast-validation-then-codify discipline.)
2. **Feature 1** (peer-echo prober + path read + IPC + detail-pane graph). Builds the shared device-roster-with-detail UI both features need.
3. **Feature 2 v1** (receive server + identity gate + send + status), direct + automatic relay-fallback.
4. **Feature 2 phase 2** (relay-side store-and-forward to offline peers) — the one piece that touches the Rust relay; the differentiator beyond Taildrop.
5. Identity-grade authz + audit emission folded in as a policy layer once the mechanism is live.

## 9. Relationship to existing components

- **`internal/reachability/` (goat-client):** unchanged. It selects wg-cp0 relay endpoints; the new `internal/peerping/` is a different prober for inner-mesh peers. Do not overload one for the other.
- **`volatile/blob` chunked transfer (Zenoh fabric, [ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md)):** conceptually adjacent (both move bytes between participants) but a **different layer** — `volatile/blob` is fabric-mediated bulk transfer for services on the Zenoh bus; goatdrop is direct HTTP peer-to-peer over the WG tunnel for laptop/phone users. Deliberately separate; do not merge.
- **The Rust relay ([ADR 0709](../adr/0709-block-33-relay-rewrite-architecture.md)):** carries the encrypted data plane transparently, so basic transfer needs no relay change. Relay customization is load-bearing only for the phase-2 offline store-and-forward.
- **goat-client outer tunnel:** untouched. These features are additive on the inner-mesh side.

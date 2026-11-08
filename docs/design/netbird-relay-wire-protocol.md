# Netbird relay wire protocol — reverse-engineered spec

**Status:** SKELETON (Block 33A — wire-protocol conformance work in progress).
**Source-of-truth pin:** upstream `netbirdio/netbird` commit `53b04e512` (2026-04-16) — matches `NETBIRD_REF` pin in [`ops/netbird/Dockerfile`](../../ops/netbird/Dockerfile), which is the version production runs.
**Authored:** 2026-04-29 by the Block 33 work; updated as each section is filled in from upstream source reading.
**Companion artifacts:**
- Block 33 plan: `../project/implementation-plan.md` §"Block 33"
- Working state: `../project/block-33-progress.md`
- Implementation crate (Apache 2.0): [`goat-relay-protocol/`](../../goat-relay-protocol/)
- Conformance soak driver: `ops/wave-33/relay-conformance/`

## Why this doc exists

Block 33 is rewriting the netbird relay data plane in Rust (F-017 RCA — Go runtime overhead dominates). The new binary must be a **byte-for-byte drop-in** for production peers; existing peers cannot change. To build that, we need an executable spec of the wire protocol independent of the upstream Go source — both for clean-room hygiene (avoid AGPL §13 entanglement on the AGPL `relay/` subtree per [ADR 0110](../adr/0110-license-posture-agplv3-forked-components.md)) and for the conformance test surface (the spec is also the test surface).

This doc IS the spec. The Rust crate `goat-relay-protocol/` implements it; the conformance soak driver verifies the Rust crate produces byte-for-byte equivalent forwarding decisions to the upstream Go relay on the same input stream.

## License posture for spec extraction

Two-tier upstream:

| Upstream subtree | License | Our extraction approach |
|---|---|---|
| `netbird/relay/` (server.go, listener/, handshake, relay.go, peer.go) | **AGPLv3** | **Clean-room.** Read for protocol semantics. Document semantics here in our own words. Do not transliterate code structure into Rust. Per Oracle v Google / Sega v Accolade, protocols/APIs are not copyrightable subject matter; semantic spec extraction is fine. |
| `netbird/shared/relay/messages/` (message.go, auth/, address/, id.go, peer_state.go) | **BSD-3-Clause** | **Permissive.** We could in principle adapt the BSD-3-Clause code with attribution, but we still write our own Rust types — clean-room is the consistent posture project-wide. The BSD-3-Clause status is noted here because it materially de-risks the byte-format extraction (no AGPL on the message bytes themselves). |

Anchor: every upstream-source section reference in this doc names the file path + the license tier. The Rust crate has zero AGPL provenance.

## Protocol stack — overview

Relay traffic on TCP/UDP `:33073` rides through these layers, outermost first:

```
┌──────────────────────────────────────────────────────────────┐
│  TCP path (`rels://...` — WebSocket variant)                  │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  TLS 1.3 (rustls / Go crypto/tls)                       │  │
│  │  ┌──────────────────────────────────────────────────┐  │  │
│  │  │  HTTP/1.1 Upgrade → WebSocket frames (RFC 6455)   │  │  │
│  │  │  ┌────────────────────────────────────────────┐  │  │  │
│  │  │  │  Relay-message framing (this spec §3)       │  │  │  │
│  │  │  │  ├── auth handshake messages (§3.1)          │  │  │  │
│  │  │  │  ├── peer-state messages (§3.2)              │  │  │  │
│  │  │  │  ├── relay-payload messages (§3.3)           │  │  │  │
│  │  │  │  └── close messages (§3.4)                   │  │  │  │
│  │  │  └────────────────────────────────────────────┘  │  │  │
│  │  └──────────────────────────────────────────────────┘  │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  UDP path (`rels://...` — QUIC variant on same port :33073)   │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  QUIC (RFC 9000) with TLS 1.3 in handshake              │  │
│  │  ┌──────────────────────────────────────────────────┐  │  │
│  │  │  QUIC bidirectional stream                        │  │  │
│  │  │  ┌────────────────────────────────────────────┐  │  │  │
│  │  │  │  Relay-message framing (same shape as WS)   │  │  │  │
│  │  │  └────────────────────────────────────────────┘  │  │  │
│  │  └──────────────────────────────────────────────────┘  │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

The **same port** (`:33073`) hosts both TCP-WebSocket and UDP-QUIC listeners simultaneously — confirmed in M-002 (binary started both listeners on startup). Per F-016, today's daemon picks WebSocket exclusively despite QUIC support being compiled in. The Rust impl supports both transports; the existing daemon's protocol-selection behavior is out of scope for Block 33.

## §1 — TCP listener (WebSocket variant)

**Spec source files (AGPLv3, clean-room read):**
- `relay/server/listener/ws/listener.go` (110 lines)
- `relay/server/listener/ws/conn.go` (93 lines)
- `shared/relay/constants.go` (BSD-3-Clause — defines URL path constant)
- `shared/relay/client/dialer/ws/ws.go` (BSD-3-Clause — client-side dial; confirms the `rels://` → `wss://` rewrite)

Read 2026-04-29.

**Wire-protocol semantics:**

| Aspect | Value | Source |
|---|---|---|
| HTTP path | `/relay` (constant `WebSocketURLPath`) | `shared/relay/constants.go:5` |
| URL scheme rewrite | client rewrites `rels://...` → `wss://...` and `rel://...` → `ws://...` | `shared/relay/client/dialer/ws/ws.go:60-65` |
| TLS | TLS 1.3 via Go `crypto/tls` (rustls equivalent in Rust impl) | `listener.go:43-50` |
| ALPN | **NOT set** — the upstream `tls.Config` does not populate `NextProtos`. ALPN is silent for the WS path. | `relay/server/server.go:25` (TLSConfig flows through unchanged) |
| Subprotocol (`Sec-WebSocket-Protocol`) | None — no subprotocol header set on the upgrade | inferred from `coder/websocket` `AcceptOptions` lacking `Subprotocols` |
| Origin policy | `OriginPatterns: ["*"]` — any origin accepted | `listener.go:81` |
| Reverse-proxy support | If `X-Real-Ip` + `X-Real-Port` headers set, those override `r.RemoteAddr` for the logged peer address | `listener.go:105-110` |
| Frame opcode (inbound) | **Only `MessageBinary` accepted** — text or control frames at the message-data layer cause `unexpected message type` error and connection close | `conn.go:40-43` |
| Frame opcode (outbound) | Only `MessageBinary` written | `conn.go:59` |
| Message-to-frame mapping | **1 relay message = 1 WebSocket binary frame** — confirmed by the `Read` impl which calls `Reader()` (returns next-message reader) then drains it into the caller's buffer in one go | `conn.go:34-50` |
| Per-message size cap | No explicit WS-layer cap — relies on the relay-message layer's `MaxMessageSize = 8820` (spec §3.0). The `coder/websocket` library's defaults govern fragmentation | inferred |
| ReadHeaderTimeout | 5s — caps how long the HTTP upgrade phase may take | `listener.go:44` |
| Write timeout | 10s per call — `Conn.Write` wraps each write in a 10s context. If the WS internal buffer is full and the timeout fires, the write fails | `conn.go:16, 56-57` |
| Read backpressure | Synchronous — `Read(ctx, b)` blocks until the next binary frame arrives or `ctx` cancels | `conn.go:34-50` |
| WebSocket library (Go) | `github.com/coder/websocket` (the coder fork; minimal-API, lower memory footprint than gorilla/websocket) | `listener.go:12` |
| WebSocket library (Rust impl) | TBD at 33B — candidates `tokio-tungstenite` (tokio-native), `fastwebsockets` (cloudflare's high-perf, no_std-friendly). Glommio support at TBD-time | — |
| Close-code semantics | On listener shutdown: graceful HTTP server shutdown (`http.Server.Shutdown(ctx)`). On client-initiated: WS `StatusNormalClosure` mapped to `net.ErrClosed` (no error to caller). On error: WS `CloseError` propagates | `conn.go:80-93`, `listener.go:64-75` |
| Ping/pong keepalive | NOT at the WS layer — relies on the relay-message-layer `MsgTypeHealthCheck` (spec §3.5). The `coder/websocket` library does send WS-protocol-level pings transparently, but those are independent of the relay's healthcheck | inferred + spec §3.5 |

**Resolves open questions from §7:**
- ✅ ALPN is NOT set on the WS path. (kTLS handoff path in 33B does not need to coordinate with ALPN.)
- ✅ 1 relay message = 1 WS binary frame. (No need for a length-prefix framer inside the WS frame.)
- ✅ WS upgrade lives at `/relay`. (`Sec-WebSocket-Protocol` is empty; no subprotocol.)

**Implementation notes for the Rust crate:**

- `goat_relay_protocol::transport::ws::URL_PATH = "/relay"` (constant).
- `goat_relay_protocol::transport::ws::READ_HEADER_TIMEOUT = Duration::from_secs(5)`.
- `goat_relay_protocol::transport::ws::WRITE_TIMEOUT = Duration::from_secs(10)`.
- `goat_relay_protocol::transport::ws::rewrite_dial_url(rels_url) -> wss_url` helper to mirror the client-side rewrite.
- The actual WS implementation (accept loop, frame I/O) lives at the binary crate (33B). The protocol crate exposes constants + the URL rewrite helper + a stub `Conn` trait that the binary crate's WS adapter implements.

## §2 — UDP listener (QUIC variant)

**Spec source files (AGPLv3, clean-room read 2026-04-29):**
- `relay/server/listener/quic/listener.go` (73 lines)
- `relay/server/listener/quic/conn.go` (76 lines)

**Spec source files (BSD-3-Clause):**
- `shared/relay/constants.go` (defines `QUICInitialPacketSize`)
- `shared/relay/tls/{server_prod,alpn}.go` (defines `NBalpn`)
- `shared/relay/client/dialer/quic/quic.go` (client-side dial; confirms keepalive + idle-timeout)

**Headline finding (significant):** **QUIC path uses RFC 9221 unreliable datagrams, NOT QUIC streams.** This materially changes the F-015 TCP-over-TCP head-of-line-blocking picture for the QUIC path — datagrams are independent, lossy, and have no per-message retransmit. Loss-tolerant by design; well-suited for inner WireGuard traffic which has its own retry machinery.

**Wire-protocol semantics:**

| Aspect | Value | Source |
|---|---|---|
| QUIC version | v1 / RFC 9000 (whatever quic-go's default produces) | `quic-go.ListenAddr` |
| Datagrams | **RFC 9221 ENABLED** — `quic.Config.EnableDatagrams = true`. **All relay messages ride as QUIC datagrams.** Unreliable, no retransmit, no in-order guarantees between datagrams. | `listener.go:30, conn.go:25, conn.go:34-39` |
| Streams | **NOT USED** — the relay does NOT open QUIC streams. The QUIC connection is established only to host datagrams within. | `conn.go:24-39` (no `OpenStream` / `AcceptStream` calls) |
| ALPN | `"nb-quic"` (constant `NBalpn` at `shared/relay/tls/alpn.go:3`). Set on the server's TLSConfig by `ServerQUICTLSConfig` (`server_prod.go:15`). Client dials with the same ALPN. | `shared/relay/tls/alpn.go:3` |
| Initial packet size | `1232` bytes — conservative IPv6-safe MTU floor (`QUICInitialPacketSize`) | `shared/relay/constants.go:10` |
| Server idle timeout | not explicitly set — uses quic-go default | inferred |
| Client KeepAlivePeriod | 30 seconds | `client/dialer/quic/quic.go:43` |
| Client MaxIdleTimeout | 4 minutes | `client/dialer/quic/quic.go:44` |
| 0-RTT | not enabled (no `Allow0RTT: true`) | inferred |
| Close error code | `0x0` ("normal closure"). Server `CloseWithError(0, "normal closure")`; remote peer's `appErr.ErrorCode == 0x0` is mapped to `net.ErrClosed` for symmetry with WS path | `conn.go:54, conn.go:71-73` |
| Conn interface | implements same `Read/Write/Close/RemoteAddr` shape as the WS Conn — relay logic above this layer is transport-agnostic | `conn.go:24-43` |

**Resolves open questions from §7:**
- ✅ ALPN string is `"nb-quic"`. (Was unanswered before step 4.)
- ✅ Stream model: NO streams used; datagrams only.
- ✅ Per-message framing within QUIC: 1 relay message = 1 QUIC datagram.

### §2.1 — The size asymmetry between transports

"1 relay message = 1 QUIC datagram" carries a consequence the transport-agnostic
`Conn` interface hides: **the two transports do not accept the same message sizes.**

| | max message a peer can send | max message a peer can receive |
|---|---|---|
| WebSocket (TCP-framed) | `MaxMessageSize` = 8820 B | 8820 B |
| QUIC (RFC 9221 datagrams) | path-MTU-bound (~1200 B pre-PMTUD, ~1450 B after) — the **client enforces this itself** and refuses to send over budget | same path-MTU bound |

A relay forwards between transports without re-framing, so a **WS sender can hand the
relay a frame no QUIC receiver can accept**. A QUIC sender structurally cannot produce
this (its own client refuses first: measured `datagram frame too large: 1438 byte
datagram over path budget 1232`), so the condition is **exclusively cross-transport**.

Implementations must therefore treat `SendDatagramError::TooLarge` as a **per-message**
condition — drop the frame and continue, as upstream Go does (`relay/server/peer.go`).
Treating it as fatal to the writer permanently severs the destination's receive path
while its connection stays up: F-275, which cost months of investigation because the
resulting zombie looks healthy from every surface. Transport selection is a
first-to-finish race with no preference (`dialer/race_dialer.go`), so any mixed fleet
meets this case routinely. Because that race outcome can flip on every reconnect and is
**not observable from outside the relay** (an unconnected QUIC UDP socket is invisible in
`ss`; auth-line counts are cumulative history), the ADR 1055 stats endpoint exposes a
per-peer `"transport"` field (`"quic"`/`"ws"`/`"unknown"`), set by the listener at accept
time — the authoritative, empirical answer to "which transport is peer X on right now",
the question F-275's investigation kept having to guess at.
- ✅ 0-RTT NOT enabled.

**Asymmetry between WS and QUIC paths (open question, possible upstream issue):**

QUIC datagrams carry a payload size limit determined by the path's MTU and the QUIC packet overhead. With `InitialPacketSize=1232`, the per-datagram payload is roughly **~1200 bytes**. But the relay-message-layer's `MaxMessageSize=8820` (spec §3.0) and `MaxHandshakeRespSize=8192` allow much larger messages. Concretely:

| Message | Typical size | Fits in QUIC datagram? |
|---|---|---|
| `Auth` | ≤212 bytes | ✅ yes |
| `AuthResponse` (URL string) | typically <100 bytes | ✅ yes (in practice; would fail if URL string > ~1100 bytes) |
| `Transport` (WireGuard MTU-bound payload) | ~1500 bytes typical | **⚠️ may exceed datagram size depending on path MTU** |
| Long peer-state list (244 peers × 36 + 2) | up to 8786 bytes | ❌ **would fail to send via QUIC** |

`quic-go.SendDatagram` returns `ErrMessageTooLarge` for oversize datagrams, and the upstream relay code does NOT specially handle this error (`conn.go:34-39` returns it as-is from `Write`). A hypothetical long peer-state list would fail to send via QUIC.

**This asymmetry has not surfaced in production** because (a) F-016 reports 100% of production peers use WS, not QUIC; (b) typical workloads don't generate large peer-state lists; (c) typical WireGuard MTU is 1380 (mesh) which fits within 1232 minus protocol overhead in some cases or just barely overflows. **For Block 33, we preserve the upstream behavior** — Rust impl rides datagrams, accepts the same size limits. Documenting this as a known wire-protocol asymmetry rather than fixing it (fix would require peer-side support, breaks "byte-for-byte conformance" non-goal).

**Production implication: F-015 (TCP-over-TCP HOL) does NOT apply to the QUIC path.** Datagrams are independent — there is no outer transport layer that retransmits and stalls. If the daemon could be flipped to QUIC, the multi-flow degradation we measured in M-001 might largely disappear. **This raises the priority of M-003 (force QUIC and re-measure) — but the F-017 Go-runtime overhead remains the dominant cost, so QUIC is at best a partial fix to the throughput ceiling. It would, however, eliminate the catastrophic multi-flow degradation pattern.**

**Implementation notes for the Rust crate:**

- `goat_relay_protocol::transport::quic::ALPN: &str = "nb-quic"`.
- `goat_relay_protocol::transport::quic::INITIAL_PACKET_SIZE: u16 = 1232`.
- `goat_relay_protocol::transport::quic::CLIENT_KEEP_ALIVE: Duration = Duration::from_secs(30)`.
- `goat_relay_protocol::transport::quic::CLIENT_MAX_IDLE: Duration = Duration::from_secs(4 * 60)`.
- `goat_relay_protocol::transport::quic::CLOSE_NORMAL_ERROR_CODE: u64 = 0x0`.
- `rewrite_dial_url` / port-default helper analogous to WS — `rels://...` → 443, `rel://...` → 80; the Rust impl can mirror upstream's `prepareURL`.
- The actual QUIC implementation (datagram I/O, accept loop) lives at the binary crate (33B). Candidate Rust QUIC libraries: `quinn` (most mature, tokio-native), `s2n-quic` (AWS, supports kTLS via s2n-tls), `quiche` (Cloudflare, low-level).

## §3 — Relay-message framing

**Spec source files (BSD-3-Clause, permissive):** `shared/relay/messages/message.go`, `id.go`, `peer_state.go`, `peer_state_test.go`, `auth/auth.go`, `address/address.go`. Read 2026-04-29.

The relay multiplexes all peer traffic over a single connection per peer (one peer = one TCP-WS / QUIC connection to relay). Each message is a discrete unit; on the WS path, one WebSocket binary frame = one relay message (boundaries preserved by the WS framer). Routing decisions happen on-relay based on the destination peer ID embedded in each Transport message.

### §3.0 — Common header

Every message starts with a 2-byte header:

```
offset  size  field
  0      1    version  (currently 0x01; ValidateVersion rejects others)
  1      1    msg_type (see table below)
  2      ?    body bytes (may be empty for header-only messages)
```

Constants:

| Constant | Value | Source |
|---|---|---|
| `CurrentProtocolVersion` | 1 | `message.go:14` |
| `sizeOfProtoHeader` | 2 | `message.go:36` |
| Max-byte-length-for-Auth-message | 212 (`MaxHandshakeSize`) | `message.go:10` |
| Max-byte-length-for-AuthResponse | 8192 (`MaxHandshakeRespSize`) | `message.go:11` |
| Max-byte-length-for-Transport+payload | 8820 (`MaxMessageSize`) | `message.go:12` |

### §3.0.1 — PeerID (36 bytes, fixed)

PeerIDs on the wire are 36 bytes: 4-byte ASCII prefix `"sha-"` (`0x73 0x68 0x61 0x2D`) followed by a 32-byte SHA-256 digest of the peer's plaintext identifier (peer enrollment ID, hashed at the daemon side).

```
offset  size  field
  0      4    "sha-" (0x73 0x68 0x61 0x2D)
  4     32    sha256(plaintext_peer_id)
```

The relay never sees plaintext peer identifiers — only the 36-byte hashed form. This is the routing key for the per-peer-pair index in spec §4.

### §3.0.2 — Magic bytes (4 bytes, used by Hello + Auth)

```
0x21 0x12 0xA4 0x42
```

This is the **STUN magic cookie** value (RFC 5389 §6). Re-used here as a sentinel in the auth handshake to disambiguate the Auth/Hello message body from a malformed wire stream. Validated on receive; reject if mismatch.

### §3.0.3 — Message-type table

| Tag | Name | Direction | Status | Body shape |
|---|---|---|---|---|
| 0x00 | `Unknown` | — | reserved (sentinel) | — |
| 0x01 | `Hello` | client → server | **DEPRECATED** — replaced by `Auth` | magic(4) + peerID(36) + additions(varlen) |
| 0x02 | `HelloResponse` | server → client | **DEPRECATED** — replaced by `AuthResponse` | additionalData(varlen) |
| 0x03 | `Transport` | both | **active — data-plane hot path** | peerID(36) + payload(varlen) |
| 0x04 | `Close` | both | active | (header-only — body empty) |
| 0x05 | `HealthCheck` | server ping → client respond | active | (header-only) |
| 0x06 | `Auth` | client → server | active | magic(4) + peerID(36) + authPayload(varlen, total ≤212) |
| 0x07 | `AuthResponse` | server → client | active | address(varlen string, total ≤8192) |
| 0x08 | `SubscribePeerState` | client → server | active | concatenated peerIDs (N × 36 bytes) |
| 0x09 | `UnsubscribePeerState` | client → server | active | concatenated peerIDs |
| 0x0A | `PeersOnline` | server → client | active | concatenated peerIDs |
| 0x0B | `PeersWentOffline` | server → client | active | concatenated peerIDs |

`DetermineClientMessageType` (server-side, parsing inbound from client) accepts `Hello | Auth | Transport | Close | HealthCheck | SubscribePeerState | UnsubscribePeerState`. `DetermineServerMessageType` (client-side, parsing inbound from server) accepts `HelloResponse | AuthResponse | Transport | Close | HealthCheck | PeersOnline | PeersWentOffline`. Source: `message.go:108-149`.

### §3.1 — Auth FSM (the handshake)

**Spec source files (AGPLv3, clean-room read 2026-04-29):**
`relay/server/handshake.go` (162 lines).

**Spec source files (BSD-3-Clause):** `shared/relay/messages/auth/auth.go`, `shared/relay/messages/address/address.go` — both marked DEPRECATED in upstream.

The handshake is the auth-flow state machine that runs on every newly-accepted WS/QUIC connection before the connection enters the steady-state forwarding loop. Behavior in upstream:

```
                                      ┌─────────────────────────────────┐
   ┌─────────────────────────────┐    │ 10s timeout (`handshakeTimeout` │
   │ accepted conn (post-TLS)     │    │ at handshake.go:21)             │
   └──────────────┬──────────────┘    └─────────────────────────────────┘
                  │ read up to 212 bytes (MaxHandshakeSize)
                  ▼
   ┌─────────────────────────────┐
   │ ValidateVersion(buf)         │ ── reject if buf[0] ≠ 0x01
   └──────────────┬──────────────┘
                  │
                  ▼
   ┌─────────────────────────────┐
   │ DetermineClientMessageType   │ ── reject if msgType ∉ {Hello, Auth}
   └──────┬─────────────────┬────┘
          │                 │
       Hello (0x01)      Auth (0x06)        ── server records which path was used
          │                 │
          ▼                 ▼
   ┌─────────────────┐ ┌────────────────┐
   │ UnmarshalHello  │ │ UnmarshalAuth  │
   └────────┬────────┘ └────────┬───────┘
            │                   │
            ▼                   ▼
   ┌─────────────────┐ ┌────────────────────┐
   │ gob decode      │ │ validator.Validate │
   │ auth.Msg from   │ │ (auth_payload)     │
   │ `additions`     │ └────────┬───────────┘
   └────────┬────────┘          │
            │                   │
            ▼                   │
   ┌──────────────────────┐     │
   │ validator.ValidateHelloMsgType │
   │ (auth.Msg.AdditionalData)     │
   └────────┬─────────────┘     │
            │                   │
            └─────────┬─────────┘
                      │
                      ▼
   ┌─────────────────────────────┐
   │ on success: send pre-marshaled response   │
   │   Hello path → HelloResponse              │
   │              (gob `address.Address{URL}`) │
   │   Auth  path → AuthResponse               │
   │              (raw URL bytes)              │
   │                                           │
   │ on failure: close connection silently     │
   │             (no response written)         │
   └─────────────────────────────┘
                      │
                      ▼
            entry to steady-state forwarding loop (spec §4)
```

#### §3.1.1 — Auth (`MsgTypeAuth = 0x06`) — client → server

The first message a client sends after the WS / QUIC handshake completes. Rejected → server closes the connection silently (per the comment in `MarshalAuthMsg` doc-string).

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x06)
  2      4    magic (0x21 0x12 0xA4 0x42)
  6     36    peerID
 42      ?    authPayload (varlen, total message ≤212 bytes → authPayload ≤170 bytes)
```

The authPayload is opaque at this protocol level (see `MarshalAuthMsg` doc-string: "The authentication is done using an HMAC method. The protocol does not limit to use HMAC, it can be any other method"). Validation is implementation-defined; the relay protocol invokes a `Validator` interface — see §3.1.5.

#### §3.1.2 — AuthResponse (`MsgTypeAuthResponse = 0x07`) — server → client

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x07)
  2      ?    address (UTF-8 string — server's instance URL, total ≤8192 bytes)
```

The `address` is the server's instance URL (e.g., `rels://relay-b.example:33073`) used by the client to choose a common relay if the two peers are on different relay servers. (Source: `MarshalAuthResponse` doc-string.) Currently same-relay only — federation between relays is not implemented in upstream. **Address is sent as raw UTF-8 bytes, no wrapping.**

#### §3.1.3 — Hello / HelloResponse (DEPRECATED) — gob-wrapped variant

`MsgTypeHello = 0x01` and `MsgTypeHelloResponse = 0x02`. Marked deprecated in upstream comments (`message.go:17, 19`).

**Wire-frame body shape is the same as Auth / AuthResponse** (header + magic + peerID + body), BUT the body bytes are interpreted differently:

- **Hello body's `additions` field** is a Go `gob`-encoded `auth.Msg{AuthAlgorithm Algorithm, AdditionalData []byte}` struct (`shared/relay/messages/auth/auth.go`, BSD-3-Clause, marked DEPRECATED). The relay decodes the gob, extracts `AdditionalData`, passes it to `validator.ValidateHelloMsgType()`. The `Algorithm` field is informational — values: `Unknown=0`, `HMACSHA256=1`, `HMACSHA512=2`.
- **HelloResponse body** is a Go `gob`-encoded `address.Address{URL string}` struct (`shared/relay/messages/address/address.go`, BSD-3-Clause, marked DEPRECATED). Client decodes the gob to recover the URL. Modern AuthResponse skips the gob wrapper.

Implementation note for the Rust crate: gob is a Go-specific binary format. **Implementing a gob decoder/encoder for these two types is queued as a follow-up sub-task** (the deprecated path is a backwards-compat concern, not a hot-path concern). New daemons emit Auth and accept AuthResponse. Block 33A's wire-format spec covers Hello and HelloResponse at the envelope level; the Rust crate parses the envelope but exposes the gob-wrapped body bytes as opaque `Bytes` — consumers needing the deprecated path can ship gob support in their consuming binary.

**Production-traffic question (open):** of the netbird daemon binaries actively connecting to our production relays, what fraction send Hello vs Auth? F-016 reports 100% WebSocket (vs QUIC); the Hello-vs-Auth split is unmeasured. Tracked in §7.

#### §3.1.4 — `Validator` interface contract

The relay protocol invokes a host-supplied `Validator` to decide whether to accept a connection. Upstream interface (`relay/server/handshake.go:24-28`):

```go
type Validator interface {
    Validate(any) error
    // Deprecated: Use Validate instead.
    ValidateHelloMsgType(any) error
}
```

In practice both methods receive `[]byte`:

| Method | Input | When called |
|---|---|---|
| `Validate(payload []byte)` | the raw `auth_payload` from an Auth message | Auth path (modern) |
| `ValidateHelloMsgType(additional_data []byte)` | the `AdditionalData` field of the gob-decoded `auth.Msg` from a Hello message's `additions` | Hello path (deprecated) |

Return `nil` to accept; return an error to reject. **On rejection, the server closes the underlying connection silently** — no rejection message is sent on the wire (per `MarshalAuthMsg` doc-string: "If the authentication failed the server will close the network connection without any response").

What credential format the Validator expects is implementation-defined — upstream's production deployment uses an HMAC-token scheme issued by the management server, but the relay protocol is agnostic. The Rust crate exposes an equivalent trait; binding to a specific HMAC scheme lives in the consuming binary's auth backend (33B+).

#### §3.1.5 — `preparedMsg` pattern (zero-allocation response)

Upstream pre-marshals the success response **once at server start** and reuses the byte slice across all handshakes. From `handshake.go:30-51`:

```
type preparedMsg struct {
    responseHelloMsg []byte   // gob-encoded address.Address{URL}
    responseAuthMsg  []byte   // raw URL bytes
}
```

Per-handshake the server picks the right pre-marshaled response based on which path the client used. This avoids one allocation + one marshal per handshake on the hot path — small saving in absolute terms, but it's the kind of zero-alloc discipline the Rust impl should preserve. The `goat-relay-protocol` crate exposes `Bytes::from_static`-friendly helpers; the consuming binary can construct the pre-marshaled responses once at startup and clone the `Bytes` (refcount bump, no allocation) per handshake.

#### §3.1.6 — 10-second handshake timeout

`handshake.go:21` defines `handshakeTimeout = 10 * time.Second`. This bounds how long a connection may sit pre-auth before being closed. **In upstream this is not enforced inside `handshakeReceive` itself** — the calling code passes a `context.Context` whose deadline is set by the listener layer. The Rust impl must enforce equivalent semantics: a connection that has not completed `handshakeReceive` + `handshakeResponse` within 10 seconds of accept must be closed. Implementation detail of 33B's listener loop, not the protocol crate.

### §3.2 — Peer-state messages (`MsgTypeSubscribePeerState = 0x08`, etc.)

Four message types share a uniform shape — concatenated 36-byte PeerIDs after the 2-byte header:

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x08 / 0x09 / 0x0A / 0x0B)
  2     36    peerID[0]
 38     36    peerID[1]
  …      …    …
```

Total message size ≤ `MaxMessageSize = 8820`, so max peers per message = `(8820 - 2) / 36 = 244`. Larger lists are chunked across multiple messages (`marshalPeerIDs` in `peer_state.go:48`).

| Tag | Name | Semantics |
|---|---|---|
| 0x08 | `SubscribePeerState` | Client tells server: "notify me when these peers come online or go offline" |
| 0x09 | `UnsubscribePeerState` | Client tells server: "stop notifying me about these peers" |
| 0x0A | `PeersOnline` | Server tells client: "these peers in your subscription set are now connected" |
| 0x0B | `PeersWentOffline` | Server tells client: "these peers in your subscription set just disconnected" |

The peer-state subsystem is a publish-subscribe overlay on top of the connection-tracking maintained for routing in §4. The relay knows which peers are connected; subscribers learn through these messages.

### §3.3 — Transport (`MsgTypeTransport = 0x03`) — both directions, data-plane hot path

**This is the message that dominates relay throughput.** Every WireGuard packet ridden over the relay rides as one of these.

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x03)
  2     36    peerID
 38      ?    payload (varlen, opaque to relay — inner WireGuard packet)
```

Total message size ≤ `MaxMessageSize = 8820`, so max payload = 8820 - 38 = 8782 bytes. Real WireGuard packets are MTU-bounded (typically ≤1500 bytes), well under this.

**The peerID field has dual meaning depending on direction:**

| Direction | peerID semantics |
|---|---|
| Client → Server | **Destination peer ID** — "deliver this packet to peer with this hashed ID" |
| Server → Client | **Source peer ID** — "this packet came from the peer with this hashed ID" |

The server rewrites the peerID **in-place** between receive-from-source and forward-to-destination, via `UpdateTransportMsg` (`message.go:324-330`). This is the upstream's zero-copy trick: the server reuses the same byte slice it received, only overwriting the 36-byte peerID field, and forwards the resulting bytes to the destination peer's connection. Critical to performance — and a constraint the Rust impl must preserve to match throughput characteristics. Per F-017's strace evidence the per-byte work is small relative to runtime overhead, but conformance demands matching the protocol semantics regardless.

The Rust impl will produce a `Transport { peer_id: PeerId, payload: Bytes }` struct on parse, but the steady-state forwarding path can opt into a zero-copy variant that operates on the raw byte buffer with an offset table (parallel to the upstream zero-copy design). Implementation detail of 33B; the protocol crate exposes both APIs.

### §3.4 — Close (`MsgTypeClose = 0x04`) — both directions

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x04)
```

Header-only. Either side sends to gracefully close the connection. After receiving Close, the peer closes the underlying transport (WS / QUIC). No body, no acknowledgement.

### §3.5 — HealthCheck (`MsgTypeHealthCheck = 0x05`) — server pings, client responds

```
offset  size  field
  0      1    version (0x01)
  1      1    msg_type (0x05)
```

Header-only. Server periodically sends; client must respond with the same message. Frequency and timeout live in `relay/healthcheck/` (AGPLv3 — clean-room read for §5). If the client does not respond within the timeout, server closes the connection (per `MarshalHealthcheck` doc-string).

The constant `healthCheckMsg = []byte{0x01, 0x05}` is pre-allocated in `message.go:61` so that hot-path emission needs no allocation.

### §3.6 — Implementation notes for the Rust crate

Direct mappings:

- `goat-relay-protocol::messages::PeerId` — `[u8; 36]` newtype with `AsRef<[u8]>` + `Display` (matches Go's `String()` format `"sha-" + base64(hash)`).
- `goat-relay-protocol::messages::MsgType` — `#[repr(u8)]` enum with explicit discriminants per the table above + `TryFrom<u8>` that maps both client-accepted and server-accepted variants (uniform parsing; direction validated at the consumer's choice).
- `goat-relay-protocol::framing::Frame` — typed `enum` with one variant per active message type; payload variants borrow `Bytes` for zero-copy where possible.
- `goat-relay-protocol::messages::MAGIC: [u8; 4] = [0x21, 0x12, 0xA4, 0x42]`.
- `goat-relay-protocol::messages::PROTOCOL_VERSION: u8 = 0x01`.
- Constants `MAX_HANDSHAKE_SIZE = 212`, `MAX_HANDSHAKE_RESP_SIZE = 8192`, `MAX_MESSAGE_SIZE = 8820`, `PEER_ID_SIZE = 36`, `HEADER_SIZE = 2`.
- Property tests: round-trip parse/serialize identity for each message variant; reject-malformed: short messages, wrong version, wrong magic, oversize.

The deprecated Hello/HelloResponse types are implemented for parity with the production daemon binary; they are accepted on parse and emitted only when explicitly constructed (no auto-fallback).

## §4 — Routing-decision logic

**Spec source files (AGPLv3, clean-room read 2026-04-29):**
- `relay/server/relay.go` (199 lines — top-level forwarding logic)
- `relay/server/peer.go` (298 lines — per-peer state machine)
- `relay/server/store/store.go` (97 lines — peer index)
- `relay/server/store/notifier.go` (61 lines — peer-state broadcast)

### §4.1 — Peer index

**Data structure:** `map[PeerID]*Peer` protected by a single `sync.RWMutex` (`store.go:16-19`). Lookup is O(1) on the 36-byte hash key. Insertion and deletion take the write lock; lookup takes the read lock.

**Reconnection semantics (`store.AddPeer`, `store.go:31-41`):** if a peer ID is already in the map when `AddPeer` is called, the **existing peer's connection is forcibly closed** and the new connection takes its place. `AddPeer` returns `true` when this happens — the relay records this as a "reconnection" metric. This means a peer that reconnects after a transient network blip seamlessly takes over its old slot; the old goroutine will see `net.ErrClosed` on its next read and exit.

**Deletion safety check (`store.DeletePeer`, `store.go:44-58`):** delete is conditional — it only removes if the stored peer *is* the same peer object passed in. This prevents the case where peer A reconnects (replacing A's old entry), and then A's old goroutine exits and tries to delete the new A's entry — the equality check catches this.

### §4.2 — Forwarding loop (the data-plane hot path)

Per-peer goroutine model (`peer.go:Work()`):

1. **One goroutine per connected peer.** Reads from the peer's transport conn into an 8820-byte buffer (`bufferSize = MaxMessageSize`). Buffer is **allocated once per goroutine and reused across reads** — same buffer mutated in place for the in-place rewrite trick.
2. **Validation per message:** `ValidateVersion` then `DetermineClientMessageType`. Reject (close conn) on invalid.
3. **Dispatch by type** (`peer.go:handleMsgType`):
   - `Transport (0x03)` → see §4.3 below.
   - `HealthCheck (0x05)` → wake the keepalive subsystem (§5).
   - `Close (0x04)` → close conn, exit goroutine.
   - `SubscribePeerState (0x08)` → register interest in named peers, respond with currently-online subset.
   - `UnsubscribePeerState (0x09)` → drop interest in named peers.
   - Other types (Auth/Hello, AuthResponse/HelloResponse, peers-online/offline notifications) on inbound from a client → log warning, ignore (these are server-emitted or already-handled-in-handshake).
4. **On read error or n=0** → exit goroutine. Deferred cleanup: cancel ctx, remove listener from notifier, close conn.
5. **Goroutine count per peer:** 1 main + 1 healthcheck-sender + 1 healthcheck-event-handler = **3 per peer**. Plus 1 listener-event goroutine per peer. So 4 goroutines per peer, scaling linearly with active peers — directly explains F-017's futex pressure (Go runtime synchronization between O(N) goroutines under load).

### §4.3 — Transport forwarding (the actual byte-shuffle)

`peer.go:handleTransportMsg`:

```
1. UnmarshalTransportID(msg)            // extract destination peer_id (36 bytes)
2. dst, ok := store.Peer(destID)         // O(1) map lookup, RLock
3. if !ok { log debug; drop }            // silent drop — no bounce, no buffer
4. UpdateTransportMsg(msg, sender_id)    // overwrite peer_id field IN PLACE
                                         // (4 bytes header + 36 bytes peer_id)
5. dst.Write(dst.ctx, msg)               // forward — same buffer, no allocation
```

**Key properties:**

- **Silent drop on unknown destination.** No NACK is sent back to the source; no buffer holds the message for the dest to come online later. This is intentional — relay is stateless w.r.t. peer-to-peer ordering. WireGuard's inner retry takes care of "destination temporarily unreachable" within bounds.
- **Zero-copy in the steady state.** The same `msg` slice that `peer.Read` filled is passed to `dst.Write` after the in-place peer_id rewrite. No alloc, no copy.
- **One peer-pair = two-way stream multiplexed over TWO independent peer-to-relay connections** (one per direction's source peer). The relay does NOT have a "pair" abstraction — each peer's connection is independent; routing is per-message based on the destination ID embedded.
- **Cross-relay routing not supported.** If A is on relay-a and B is on relay-b, A's Transport addressed to B does an O(1) lookup on relay-a's local store, finds B not in the map, drops. **The daemon side is responsible for ensuring the peer pair shares a relay** (handled client-side via DNS round-robin + the AuthResponse-supplied instance URL letting peers re-dial the same relay).

### §4.4 — Backpressure semantics

**Per-peer-source goroutine blocks on dest's `Write`.** When `dst.Write(dst.ctx, msg)` blocks (e.g., dst's WS internal buffer is full and the 10s write timeout hasn't fired yet), the source peer's read goroutine cannot proceed to its next read. This is end-to-end TCP backpressure through the relay.

**Concrete consequence for F-015 / M-001's catastrophic multi-flow degradation:** under multi-flow, multiple source peers contend on the same destination peer's single Write path. If destination's WS write is slow, ALL source readers stall. Combined with TCP-over-TCP HOL on the WS path, this produces the explosive memory growth + retransmit pattern we measured: the upstream readers keep accumulating frames in the kernel TCP buffer because nothing is reading them; eventually the relay process memory growth happens because Go's TCP buffers are tied to its goroutine state.

**The QUIC path (datagrams) breaks this pattern** because datagrams are fire-and-forget — `SendDatagram` either succeeds immediately or returns `ErrMessageTooLarge`/`ErrConnClosed`. No write-side backpressure → no read-side stall. Confirms the Block 33B Rust impl should preserve the datagram model.

**For the Rust impl (33B):** target architecture is **shard-per-core** + per-shard map + bounded MPSC for cross-shard routing. When a destination is on a different shard, the routing message goes through an SPSC ring; on overflow we apply a configurable policy (drop oldest, drop newest, or backpressure to source). **Conformance-test-relevant behavior must match upstream byte-for-byte at the wire layer**, but internal threading + buffering is implementation-detail and can deviate. Specifically: the conformance test asserts forwarding decisions, not internal goroutine timing.

### §4.5 — Peer-state broadcast (NotifierBroadcast pattern)

`store.PeerNotifier` (`store/notifier.go`) maintains a `map[*Listener]CancelFunc`. When ANY peer joins or leaves, ALL listeners are notified (`PeerCameOnline` / `PeerWentOffline` walk the entire listener map).

Each peer has its own Listener with a per-peer interest-set. The Listener filters incoming notifications and only acts on peers that this peer has subscribed to.

**Scale concern:** notification is O(N) per peer state change (where N = total connected peers). At 600 peers in our scope envelope, a single peer disconnect produces 600 listener notifications. Mostly cheap (each does an interest-set membership check) but worth measuring once we get to 600+ peers in soak. For Block 33's Rust impl, **a more efficient design pre-builds inverse interest indexes** — `map[PeerID]Vec<&Listener>` so a peer-state change touches only listeners with explicit interest. Defer to 33B as an optimization; conformance does not require this internal change.

**Rust impl status — built.** `goat-relay/src/routing.rs` keeps the subscription registry indexed both ways under one lock: subject → watchers (read by the online/offline fan-out) and watcher → subjects (so a departing peer's subscriptions are stripped in time proportional to what it watched, not a walk over every subject in the registry). A peer may watch at most `MAX_SUBSCRIPTIONS_PER_PEER` (16,384) subjects; new subjects past that are refused without disconnecting the peer, and counted as `subscriptions_refused` on the stats endpoint. Not observable at the wire for any client within the cap.

### §4.6 — Shutdown

`Relay.Shutdown(ctx)` (`relay.go:177-194`):

1. Take write lock on the close mutex (also blocks new `Accept` calls — they exit immediately when `r.closed == true`).
2. For each peer in the store, spawn a goroutine that calls `peer.CloseGracefully(ctx)` — sends a Close message (with a 3-second write timeout) and then closes the underlying conn.
3. Wait for all peer-close goroutines.
4. Cancel the metrics context.
5. Mark the relay as closed.

**Observable wire behavior:** every peer receives one Close message before its conn is torn down. The 3-second write timeout means a misbehaving peer doesn't block shutdown indefinitely. In the Rust impl, mirror this: send Close + wait up to 3s + force-close.

### §4.7 — Implementation notes for the Rust crate

- **Routing-table type:** in 33B's binary crate, use `dashmap::DashMap<PeerId, Arc<PeerHandle>>` for the simplest concurrent-map; for shard-per-core target, switch to per-shard `RwLock<HashMap<...>>` with peer-ID-hash routing + cross-shard MPSC for off-shard sends.
- **Per-peer task model:** in tokio, spawn one task per peer for the Read loop + one detached task per peer for the healthcheck-sender. (In glommio, both run on the same shard's executor.)
- **In-place rewrite:** `goat_relay_protocol::messages::payload::rewrite_peer_id` already exists; the binary crate calls it on the Read buffer before forwarding.
- **Reconnection:** on `AddPeer` collision, must trigger close on the old peer's task (signal via watch channel or AbortHandle).
- **Conformance test surface:** the Rust impl produces byte-for-byte equivalent forwarding decisions on a captured packet trace. Internal threading differences (tokio task vs Go goroutine, dashmap vs sync.RWMutex) are not observable to the conformance test — it sees only the bytes on the destination peer's stream.

## §5 — Healthcheck / keepalive

**Two distinct healthcheck subsystems** (resolves a long-standing ambiguity):

1. **Per-peer in-conn keepalive** (the main subject of this section) — relay-message-layer ping/pong using `MsgTypeHealthCheck = 0x05`. Source: `shared/relay/healthcheck/{sender,receiver,env}.go` (BSD-3-Clause).
2. **HTTP healthcheck endpoint** (operator/orchestrator-facing) — separate `:health-port/health` HTTP listener. Source: `relay/healthcheck/healthcheck.go` (AGPLv3). NOT the keepalive — this is for kube-probes / load-balancer health checks. Out of scope for the wire protocol; documented in §8 as out-of-scope.

### §5.1 — In-conn keepalive — server side (Sender)

**Spec source files (BSD-3-Clause):** `shared/relay/healthcheck/sender.go` (118 lines), `env.go` (24 lines). Read 2026-04-29.

The relay's per-peer goroutine spawns a `Sender` (`peer.go:77-78` calls `healthcheck.NewSender(p.log)` then `go hc.StartHealthCheck(ctx)`). The Sender ticker fires every `HealthCheckInterval = 25s` and wakes a separate goroutine in `peer.go:handleHealthcheckEvents` which writes a `HealthCheck (0x05)` relay-message to the conn (header-only — 2 bytes total).

**Sender state machine (`sender.go:StartHealthCheck`):**

```
ticker every 25s ──┐
                   ├─→ HealthCheck channel ──→ peer.go writes 0x01 0x05 to conn
                   │
timeout-ticker every 45s (= 25s interval + 20s timeout)
                   │
                   ├─→ if alive: clear alive flag, continue
                   ├─→ else: failureCounter++, if >= attemptThreshold: signal Timeout, exit
                   ↓
ack channel (signaled by OnHCResponse) ──→ reset failureCounter, set alive = true
```

| Constant | Value | Source |
|---|---|---|
| `defaultHealthCheckInterval` | 25 seconds | `sender.go:13` |
| `defaultHealthCheckTimeout` | 20 seconds | `sender.go:14` |
| Effective timeout window | 45 seconds (= interval + timeout) | `sender.go:57` |
| `defaultAttemptThreshold` | 1 | `sender.go:11` |
| Override env var | `NB_RELAY_HC_ATTEMPT_THRESHOLD` | `env.go:11` |

**Behavior:** every 25 s the server sends a `HealthCheck` message. The client must echo a `HealthCheck` back. If 45 s pass with no echo, `failureCounter` increments; once it reaches `attemptThreshold` (default 1), the Sender signals `Timeout`. `peer.go:handleHealthcheckEvents` reacts to `Timeout` by closing the underlying conn (`peer.go:195-201`).

The `OnHCResponse()` (`sender.go:75-81`) signals the `ack` channel. `peer.go:handleMsgType` calls this when it sees an inbound `MsgTypeHealthCheck` from the client (`peer.go:120-121`).

### §5.2 — In-conn keepalive — client side (Receiver)

**Spec source files (BSD-3-Clause):** `shared/relay/healthcheck/receiver.go` (116 lines).

The client side is symmetric but asymmetric in timing. Constants:

| Constant | Value | Source |
|---|---|---|
| `defaultHeartbeatTimeout` | 25 + 10 = 35 seconds | `receiver.go:11` |
| `defaultAttemptThreshold` | 1 | (shared with sender via `getAttemptThresholdFromEnv`) |

The receiver's `waitForHealthcheck` ticker fires every 35 s (slightly longer than the sender's 25 s interval — gives some slack for in-flight latency). On each tick:

- If the alive flag is set → clear it, continue.
- Else → increment failure counter; once it reaches threshold, send `OnTimeout`.

The client calls `Heartbeat()` (`receiver.go:67-73`) on every received `HealthCheck` message, which signals an `alive` flag and resets the failure counter. On `OnTimeout`, the client typically tears down the conn and reconnects to a different relay (DNS round-robin).

### §5.3 — Wire-protocol implication: keepalive is NOT WebSocket control frames

The keepalive ping/pong rides as **regular `MsgTypeHealthCheck` (0x05) relay-message-layer binary frames**, not WebSocket protocol-level control frames (RFC 6455 §5.5). This means:

- The Rust crate's existing `messages::close::marshal_health_check()` + `parse_health_check()` (already shipped in step 1) IS the keepalive wire format.
- WebSocket-library-level pings (which `coder/websocket` may send transparently) are a separate, lower-layer concern that the relay protocol does not depend on.
- For QUIC: QUIC has its own connection-level idle handling (`MaxIdleTimeout` on the client side) which is separate from this relay-message-layer keepalive. The relay-message-layer keepalive runs over both transports identically.

### §5.4 — Implementation notes for the Rust crate

The protocol library already has the wire-format pieces (step 1). Step 6 adds a state-machine helper that mirrors the upstream Sender / Receiver pattern.

- New module `goat_relay_protocol::keepalive` with a sync `Sender` and `Receiver` shape that the binary crate's async runtime drives.
- Constants:
  - `HEALTH_CHECK_INTERVAL: Duration = 25s`
  - `HEALTH_CHECK_TIMEOUT: Duration = 20s`  (sender side; effective timeout window = interval + timeout = 45s)
  - `HEARTBEAT_TIMEOUT: Duration = 35s`     (receiver side)
  - `DEFAULT_ATTEMPT_THRESHOLD: usize = 1`
- `attempt_threshold` overridable; consuming binary reads `NB_RELAY_HC_ATTEMPT_THRESHOLD` env var (or its own equivalent).
- The state machine is sync — `tick()` advances state on each timer fire; `on_response()` records a successful echo. The binary crate's async runtime drives the timers.

This keeps the protocol crate runtime-agnostic; binary crate decides whether to use tokio's `interval` or glommio's `timer_periodic`.

## §6 — Conformance test surface

This section is the spec for the conformance soak driver at `ops/wave-33/relay-conformance/`.

**Test design:**

1. **Replay test (1M packets).** Capture a representative trace from a bench-env iperf3 run through the upstream Go relay (tcpdump on the relay's :33073 listener with TLS keys logged via `SSLKEYLOGFILE` so we can decrypt offline). Replay the captured packet stream into both the upstream Go relay and the Rust impl. Assert byte-for-byte equivalence on the outbound peer-side stream.

2. **Property tests (in-Rust unit-level).** For each message type, generate random valid messages, serialize, parse — assert round-trip identity. For the AGPLv3-side routing logic, replicate the test suite from upstream by re-implementing equivalent test cases against the spec, not by porting the upstream test code.

3. **Cross-impl integration test (in-Rust).** Stand up the Rust relay; connect a real upstream netbird daemon to it; verify the daemon enrolls, reaches another peer, exchanges payload. Drives the same daemon binaries production runs.

**Soak driver shape per [ADR 1008](../adr/1008-soak-testing-discipline.md):**

- bash + docker-compose orchestration.
- JSONL evidence captured to `docs/reports/<date>-relay-conformance-soak.jsonl`.
- Smoke (5 min, CI-runnable) / validation (2 hr, default for verdict) / canonical (72 hr, pre-prod) tiers.
- Pin-regression hook: re-runs smoke tier on third-party-image bumps (rustls, tokio, etc.).

## §7 — Open questions / rabbit-holes to resolve

- ~~**Does `rels://` advertise an ALPN string?**~~ ✅ **RESOLVED 2026-04-29 steps 3 + 4:** WS path does NOT set ALPN. QUIC path sets ALPN `"nb-quic"`.
- ~~**WS / QUIC simultaneous listeners on same port — how is this implemented in upstream?**~~ ✅ **RESOLVED 2026-04-29 steps 3 + 4:** WS uses TCP (HTTP server `ListenAndServeTLS` on `:33073`); QUIC uses UDP (`quic.ListenAddr` on `:33073`). They share port number but are on different L4 protocols, so no contention at the OS level. Rust impl spawns one tokio task per listener, each owning its own socket; routing table is shared via Arc.
- ~~**Per-peer connection multiplexing inside one TLS session?**~~ ✅ **RESOLVED 2026-04-29 steps 3 + 4:** One TLS session per peer-to-relay on the WS path; one QUIC connection per peer-to-relay on the QUIC path. The relay multiplexes peer-to-peer logically — each peer's TLS/QUIC connection carries that peer's whole bidirectional stream of relay messages. WS uses 1-message-per-binary-frame; QUIC uses 1-message-per-datagram.
- **What does `instance URL` mean to a peer?** Verify the peer's understanding so the Rust impl reports it correctly.
- **Production-traffic Hello-vs-Auth split** (added 2026-04-29 from step 2 reading). All production peers measured in F-016 use WebSocket; the Hello-vs-Auth split (orthogonal to WS-vs-QUIC, since both message types ride either transport) is unmeasured. If 0% of production daemons send Hello, we can defer gob support indefinitely. If non-zero, gob decode + encode for `auth.Msg` and `address.Address` becomes a Block 33A pre-prod gate. **Cheap to measure:** add a Prometheus counter on the upstream Go relay binary (or strings-grep the daemon binaries already deployed) for `MsgTypeHello` vs `MsgTypeAuth` first-message frequency. ~30 min.
- **Hot-path validator latency budget** (added 2026-04-29 from step 2 reading). The validator can take arbitrary time (HMAC verify + management-server token introspection if applicable). The 10-second handshake timeout caps it, but real budgets are tighter — every second a peer spends in the validator delays the start of forwarding. Worth measuring once we have a real validator backend wired (33B+).

## §8 — Out of scope for this spec doc

- Relay's metrics endpoint (`relay/metrics/` — that's an OpenTelemetry surface, not the data-plane wire protocol).
- Relay's healthcheck HTTP endpoint (separate from peer-side keepalive).
- The netbird daemon's protocol-selection logic.
- The mgmt-server's auth-token issuance (we receive tokens from peers; we don't issue them).

---

## Spec-completion checklist (drives 33A close-out)

When all of these are filled in from upstream-source reading, 33A's spec deliverable is complete:

- [x] §1 TCP/WS listener — all TBDs resolved (✅ shipped 2026-04-29 step 3)
- [x] §2 UDP/QUIC listener — all TBDs resolved (✅ shipped 2026-04-29 step 4); headline finding: datagrams not streams; F-015 HOL pathology does NOT apply to QUIC path
- [x] §3.1 auth handshake — message types + byte layouts + FSM + Validator + preparedMsg + timeout documented (✅ shipped 2026-04-29 step 2)
- [x] §3.2 peer-state — message types + byte layouts documented (✅ shipped 2026-04-29 step 1)
- [x] §3.3 relay-payload (hot path) — message types + byte layouts + in-place rewrite documented (✅ shipped 2026-04-29 step 1)
- [x] §3.4 close / disconnect — message types + byte layouts documented (✅ shipped 2026-04-29 step 1)
- [x] §4 routing logic — semantics documented (✅ shipped 2026-04-29 step 5); 7 sub-sections covering peer index, forwarding loop, transport hot path, backpressure, peer-state broadcast, shutdown, Rust impl notes
- [x] §5 healthcheck/keepalive — frequency + timeouts documented (✅ shipped 2026-04-29 step 6); 25s interval / 20s timeout / 35s heartbeat-receive timeout / 1 attempt threshold; rides as MsgTypeHealthCheck (0x05) relay-message-layer frames, NOT WS control frames; sync state-machine helpers in `goat_relay_protocol::keepalive`
- [x] §6 conformance test surface — `goat-relay-conformance` binary + 17 hand-crafted fixtures + JSONL soak driver (✅ shipped 2026-04-29 step 7); smoke (1 iter) and validation (100 iter) tiers exercised — all 17/17 fixtures round-trip byte-for-byte under both tiers (1700/1700 ok); canonical tier reserved for real captured production traffic at step 8
- [ ] §7 open questions — each one resolved (yes/no/by-design) — open-questions list extended in step 2; resolution per-step

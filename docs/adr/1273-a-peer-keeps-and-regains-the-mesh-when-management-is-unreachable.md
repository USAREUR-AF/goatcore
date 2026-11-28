# ADR 1273 — A peer keeps and regains the mesh when management is unreachable

| Field | Value |
|---|---|
| Status | **Proposed** (2026-11-27). Built on the NetBird fork `USAREUR-AF/netbird`: phase 1, start from the cached map (#2, merged). Built, in review: phase 2, signed maps (#4); phase 3, the goatnet root document (#5); phase 4, flooded revocations (#8); phase 5, peer certificates with relative windows (#9); the two-process network-namespace test harness they are tested on (#7). Designed, not built: the root document in the enrollment bundle, revocations issued from management's ledger, certificates kept on disk, and the warrant rule for the overlay port (#1476). The status column of §1 is updated as each phase merges |
| Date | 2026-11-27 |
| Deciders | operator |
| Repo scope | NetBird fork (client, management); goat core (`goatbuilder` declarations, the offline enrollment bundle, the goatnet root document); `goat-client` takes each fork change through its `go.mod` replace |
| Supersedes | — |
| Amends | — |
| Related | [ADR 0206](0206-island-mode-by-construction.md) (island mode by construction), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (every site writes), [ADR 1193](1193-revocations-are-absorbing-on-replicated-mesh-and-identity-tables.md) (revocations are absorbing), [ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) (the highest-echelon site can sever a site), [ADR 1037](1037-fabric-internal-time-synchronization-first.md) (internal synchronization first), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) (expiring authorizations), [ADR 0313](0313-silent-control-plane-outer-wg-wrap.md) (silent control plane), [ADR 1083](1083-goat-client-single-client-and-distribution.md) (goat-client embeds the fork); [`offline-enrollment.md`](../design/offline-enrollment.md), [`name-fallback-resolver.md`](../design/name-fallback-resolver.md), [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) |

## TL;DR (for a layperson)

A goat device finds the other devices of its network by asking a management service, which hands it a list of who it may talk to and how. If that service cannot be reached, a device that is already running keeps working, but a device that restarts cannot rejoin, because it asks management first and gives up when there is no answer. This decision makes a device keep the last list it received, signed by its network, and start from that list when management is down, so a site that loses its outside connection keeps its devices connected to each other even across restarts. The list is only trusted for a limited time, measured on the network's own clock, so a device cut off for too long stops trusting it, and revocations reach every device that can still reach another one.

## Context

A NetBird client starts in this order (`client/internal/connect.go`, `ConnectClient.run`): open a gRPC connection to management, log in, connect to signal, create the engine, start it. Every step before the engine is a precondition. When management is unreachable:

| Situation | Today |
|---|---|
| Client running, management goes away | Tunnels, routes and firewall rules stay. The `Sync` stream retries for up to three months (`shared/management/client/grpc.go:defaultBackoff`), then the engine cancels the client |
| Client restarts while management is away | The mesh does not come back. `loginToManagement` fails, the engine is never created, and the run loop retries the login |
| What the client keeps on disk | Its WireGuard private key and settings (`config.json`). The last network map is kept in memory only, for debug bundles (`Engine.latestSyncResponse`) |
| What proves a map came from management | The gRPC transport and the per-message encryption to the management server's WireGuard key. Nothing on the map itself |

A site that keeps power and its local network but loses its path to the management service therefore loses every device that restarts. ZeroTier's controller, read at tag 1.14.2, solves the restart case by saving every signed network config and loading it at start, and goes further: peers check each other's controller-signed membership certificates, the certificates expire relative to each other instead of against the wall clock, and revocations are signed and flooded among members. Those ideas (K1 to K8 below) fit goat's island-mode model. Some of ZeroTier's choices (A1 to A6) do not, and this ADR designs them out.

## Decision

The client keeps, verifies and starts from the last signed network map; management signs every map; the goatnet's signed root document names who may sign; revocations are signed, stored and flooded; peers check each other's membership certificates against a relative time window. The A3 bound caps how long any of it is trusted without a fresh map.

### 1. What goat builds, where, and when

| # | Idea | What goat builds | Where | Phase | Status |
|---|---|---|---|---|---|
| K3a | Start from the last map | The client writes the last network map it received after an authenticated login or sync beside the client's state file (`state.json` → `state.goat-map.json`, root only, mode 0600), with its receive time. When the login fails for any reason other than management refusing the peer, the engine starts from that map: WireGuard keys and allowed IPs of known peers, each peer's last direct endpoint, routes, firewall rules and DNS. Management and signal are retried in the background; the first fresh map replaces the cached one in the engine and on disk | Client | 1 | Built: `USAREUR-AF/netbird` #2, merged (99c51fbb6) |
| K7 | Signed, versioned maps | Management signs each map it sends. The signed bytes are one envelope: the serialized map, the peer key it is issued to, the goatnet ID, the serial, the signing key ID and the issue time on the site's clock. The signature covers the envelope bytes behind the domain string `goat-network-map-v1`; the client parses only bytes it has verified, so no canonical encoding is needed. When signing is on the map travels only inside the envelope, in sync-response field 1001, which the upstream schema does not declare: protobuf keeps undeclared fields, so the fork changes no generated code and an upstream client ignores the field. A fork test parses upstream's `management.proto` and fails if `SyncResponse` ever declares field 1001 or 1002, so an upstream sync cannot silently reuse either number. A map whose serial is below the one held is ignored. Management: `GoatMapSigning` in its config (goatnet ID, key file, refresh interval); client: `NB_MAP_TRUST_FILE` (goatnet ID and signer keys) until phase 3. A client without a trust file opens a signed map without verifying it, so a goatnet can turn signing on before every client holds a trust file | Management, client | 2 | Built, in review: `USAREUR-AF/netbird` #4 |
| K3b | Refuse an unverified cached map | The cache holds the envelope and its signature as received. At start the client verifies it again against its trusted signers and refuses an unsigned or wrongly signed map. A client holding a trust file refuses every unsigned map, cached or fresh, and deletes a cached map that does not verify | Client | 2 | Built, in review: `USAREUR-AF/netbird` #4 |
| K8 | Pull on an interval plus push on change | Management re-signs and sends a fresh map to every connected peer at least every `map_refresh_interval` (default 1 h) even when nothing changed, so a connected peer's map age stays below the interval and its issue time is usable for K2. The client relies on the stream's keepalive to notice a dead stream; a running engine whose last verified map passes the A3 bound removes its peers (§3) | Management | 2 | Built, in review: `USAREUR-AF/netbird` #4 |
| K6 | Signed, chained root document | One document per goatnet lists its map signers (one key per site management, ADR 1165), its management, signal and relay endpoints, and the A3 bound. It is signed by a goatnet root key (Ed25519, a key file today; built). The offline enrollment CA, which signs enrollment bundles with ECDSA P-256 (`ops/enrollment/bundle`, `cmd/bundle-create`; built), does not sign root documents; holding the root key on the same hardware as that CA is designed, not built. A newer document replaces the held one only when it has the same goatnet ID, a higher version, and a signature by a key the held document names as its successor signer. Clients accept it from management, from any peer, or from the enrollment bundle, since the signature, not the carrier, carries the trust. Same shape as the signed name snapshots of `name-fallback-resolver.md`. Built: the format, the chain rule (a root key hands over one version at a time), delivery from management in sync-response field 1002, the held file on the client (`NB_GOATNET_ROOT_FILE`, which takes the place of the trust file), and the issuer tool `tools/goatroot` in the fork. Designed: delivery from a peer (phase 4 channel), the first version in the enrollment bundle, and signing with the hardware-held root key (the tool reads a key file) | Fork (format, client, management, issuer tool); goat core (bundle) | 3 | Built, in review: `USAREUR-AF/netbird` #5; bundle and hardware key designed |
| K4 | Signed revocation, absorbing, flooded once | A revocation names a peer's WireGuard key and is signed by a map signer of the goatnet. It is absorbing, as revocations are on goat's replicated mesh and identity tables ([ADR 1193](1193-revocations-are-absorbing-on-replicated-mesh-and-identity-tables.md)): the key stays revoked for good, with no threshold, no expiry and no reinstatement. A later network map that still lists the key is refused for that peer: the peer is left out and the client counts the refusal and logs it. A device that comes back does so with a new WireGuard key and a new enrollment. The client writes each verified revocation to disk (beside the cached map, mode 0600) before acting on it, removes the peer, and forwards the revocation once over the overlay (UDP 33073 on each peer's overlay address) to every connected peer except the sender; a repeat is dropped, which ends the flood. Management sends its revocation list with every sync response (sync-response field 1003, undeclared upstream, guarded by the same test as 1001 and 1002), so a peer that was offline receives it on its next map (A4). Built: all of this, with the list held in a file on management (`GoatMapSigning.RevocationsFile`) and signed by the issuer tool (`goatroot revoke`). Designed: issuing revocations from management's own peer deletions and from the ADR 1193 revocation ledger | Management (issue), client (store, enforce, forward) | 4 | Built, in review: `USAREUR-AF/netbird` #8; ledger issuance designed |
| K1 | Peer-checked membership certificate | Management issues each peer a short certificate (goatnet, peer WireGuard key, issue time, window), signed by the map signer, with every sync response (field 1004), and sends the latest certificate it issued to each of the peer's remote peers (field 1005). Peers also send their own certificate to each other over the overlay every window/3; a certificate from an overlay address counts only for the peer whose tunnel carries that address. A peer whose certificate disagrees with ours, or that has none after twice the window, is removed; it is accepted again as soon as a certificate that agrees arrives. Built. Designed: keeping the certificates on disk, so a restart from the cached map checks them before management returns | Management (issue), client (exchange, check) | 5 | Built, in review: `USAREUR-AF/netbird` #9; on-disk certificates designed |
| K2 | Relative time windows | A certificate is valid against another when their issue times are within `membership_window` (management config `MembershipWindow`; this ADR's default 30 min) of each other, compared on issue times, not on the receiver's clock. Peers cut off together stay valid together; a peer whose last certificate is older than the others' by more than the window is refused until it gets a fresh one | Client | 5 | Built, in review: `USAREUR-AF/netbird` #9 |
| K5 | Policy credentials checked by the receiver | Not built in this program. If built, a sender presents signed grants compiled from warrant output (ADR 1129) and only the receiver enforces them, failing closed on a missing grant (A5) | Management, client | Later | Designed only as a direction |

### 2. How each pattern to avoid is avoided

| # | ZeroTier pattern | What goat does instead | Phase |
|---|---|---|---|
| A1 | One controller identity per network; high availability by copying that identity to a standby | Every site's management has its own map-signing key. The root document (K6) lists all of them, so a map or revocation from any site verifies at every peer and no key is copied between hosts. The highest-echelon site severs a site by publishing a root document without that site's key (ADR 1205) | 3 (built, in review #5) |
| A2 | 40-bit addresses with proof of work and controller-side collision checks | Identity stays the WireGuard public key for machines and the SPIFFE ID for workloads. Every signed object names the full WireGuard key | 2 |
| A3 | No upper bound on an island's validity | A map is trusted for at most `map_max_age` after it was received (rule in §3). Past that, the client does not start from it and removes its peers. Certificates older than the bound are dropped with it; revocations never expire (K4) | 1 (at start, built, #2 merged), 2 (running engine, built in review #4) |
| A4 | Revocation sent only to members seen in the last 120 s, kept in memory | Revocations are written to disk before they take effect, never expire, are carried in every sync response from management, and are flooded once among connected peers. The flood and the membership certificates of K1 use UDP 33073 between peers, which a goatnet's mesh ACL must allow: a warrant rule from every peer group of a goatnet to every peer group of the same goatnet, UDP 33073 only, nothing across goatnets and nothing from outside the mesh. The messages are signed and verified on receipt, so the port carries no authority. The rule belongs to the warrant generator (`ops/warrant/`), designed, not built: USAREUR-AF/goatcore#1476. Without it, a goatnet whose policy is not all-to-all gets revocations from management only, and a peer without certificates is refused after twice the window | 4 (built, in review #8; ACL rule designed, #1476) |
| A5 | Sender-side tag checks treated as matched when the remote tag is unknown | Enforcement stays at the receiver, as NetBird's firewall already does. A missing credential is a refusal, never a match | 5, and K5 if built |
| A6 | One shared admin token for the whole controller API | Unchanged goat model: per-person roles and the operator-action audit. Nothing in this ADR adds an admin surface | — |

TEE, WATCH and REDIRECT rule actions, the SSO split between an open controller and a hosted service, and layer-2 emulation are not adopted.

### 3. The A3 rule: an island's authorization has a maximum age

| Property | Rule |
|---|---|
| Bound | `map_max_age`, declared per goatnet. Default **72 h**, the same point at which an enrollment validator fails closed on a CRL it cannot refresh (`offline-enrollment.md` §6.2). Allowed range 1 h to 30 days; a value outside it is refused, not clamped |
| Age of a map | Time since the client received it from management, not since it was issued. Phase 2 adds a second test for a verified map: time since the issue time in the signed envelope, on the system clock, must also be within the bound |
| Clock, same boot | The boot clock (Linux `CLOCK_BOOTTIME`, which counts suspend and is never stepped). The cache records the boot ID with the receive time; when the boot ID matches, age is the boot-clock difference |
| Clock, after a reboot | The system clock, which goat disciplines by peer-to-peer internal synchronization (ADR 1037). A system clock that reads earlier than the receive time is refused: a cold boot without a real-time clock must not make an old map look new |
| Platforms without a boot ID | The system clock only, with the same refusal of a negative age |
| At the bound, at start | The client does not start from the map, logs why, and keeps retrying management as it does today |
| At the bound, running engine | From phase 2, for a client holding a trust file: when the last verified map passes the bound with no fresh one, the engine removes every remote peer and keeps retrying management; the next verified map restores them. Without a trust file a running engine behaves as upstream NetBird does |
| Configuration | Fork client: `NB_MAP_CACHE_MAX_AGE` (Go duration; an invalid or out-of-range value disables starting from the cache) and `NB_MAP_CACHE_DISABLE=true` (nothing written or read) until phase 3; from phase 3 the root document carries the bound and the environment value may only shorten it |

### 4. What the cache holds and what it refuses

| Item | Rule |
|---|---|
| Written | After every network map applied from management (login, sync), and only from those. Atomic write (temporary file, then rename) in the daemon's state directory, owner root, mode 0600 |
| Contents | Management URL; this peer's WireGuard public key; this peer's address, FQDN and DNS settings from the login; signal and relay URLs; the last network map; per remote peer, the last direct (not relayed, not proxied) WireGuard endpoint seen; receive time on both clocks; boot ID |
| Not written | The relay token, the flow token and TURN credentials: the relay is used again when management issues a fresh token. The WireGuard private key stays in `config.json` only |
| Binding | One cache per client profile, beside that profile's state file. It is refused, and deleted, when its management URL or peer key differs from the profile's current ones, so a peer moved to another management or re-keyed starts clean. From phase 2 the signed envelope also binds it to the goatnet ID |
| Deleted | When management refuses the peer (`PermissionDenied` at login, including the login that follows a refused sync: deleted, expired, blocked), at logout of the active profile, and when it is refused for age, binding or format. A clock that reads earlier than the receive time refuses the cache without deleting it, since the clock may be corrected |
| Replaced | By the first fresh map. The engine resets its serial before that map, so a fresh map from a site whose serial is lower than the cached one still applies (island-mode serial resets, ADR 1165 D3 class 5) |
| Peers from the cache | Configured with their last direct endpoint so WireGuard can complete a handshake without signal. A peer with no recorded direct endpoint is added and waits for signal or relay. Peers taken from the cache are kept out of lazy connection until a fresh map arrives |

### 5. Threat model change

A cached map on disk is a credential: with it, a host can rebuild its tunnels, routes and firewall rules without asking anyone.

| Threat | Before | After | Control |
|---|---|---|---|
| Read the cache (stolen disk, other local user) | The WireGuard private key in `config.json` already gives this peer's identity | Also reveals peer keys, overlay addresses, last real endpoints, routes and SSH authorized-user hashes | Root only, mode 0600, beside `config.json`; disk encryption at rest; no relay token in the file |
| Edit the cache to add a peer or widen a rule (local root) | Not possible across a restart | Phase 1: possible, at the same trust level as editing `config.json` | Phase 2: the envelope signature is checked at every load; an edited map is refused |
| Replay an older signed map | — | A valid old map could be offered | Serial and issue time in the envelope; the A3 bound on both |
| Keep a revoked peer alive on an island | The running engine keeps the peer until management returns | Same, until phase 4 | Phase 4 flooded revocations; phase 5 certificates; A3 bound in every phase |
| A compromised site management signs maps | One management is trusted for everything | Same | Phase 3: the highest-echelon site removes that site's key from the root document; peers refuse its maps once they hold the newer document |
| Clock set back to make a map look fresh | — | — | Boot clock within a boot; a negative age is refused; issue time in the envelope (phase 2) |

### 6. Algorithms

| Use | Algorithm now | Later |
|---|---|---|
| Map envelope, revocation, certificate, root document signatures | Ed25519 (FIPS 186-5). The offline enrollment bundle is signed with ECDSA P-256 (built, `ops/enrollment/bundle`); the two keys are separate | Hybrid: Ed25519 and ML-DSA-65 (FIPS 204), both signatures required, when the fork's Go toolchain carries ML-DSA in a validated module. The key ID names the algorithm set so both kinds can coexist during a change |
| Hash for key IDs and dedupe | SHA-256 | — |

ZeroTier's choices (Curve25519/Ed25519 identities with a Salsa20 proof-of-work hash) are not used.

### 7. How it composes

| With | Relationship |
|---|---|
| Offline enrollment (`offline-enrollment.md`) | The bundle is the first join without the cloud; the cached map is the rejoin. The bundle names its CA by `ca_id` and carries no root document (built, `ops/enrollment/bundle`). Carrying the first root document in the bundle, so a device trusts the right map signers from its first boot, is designed, not built; until then phase 3 depends on the first root document being placed at `NB_GOATNET_ROOT_FILE` by whatever provisions the device. The bundle validator on management (`offline-enrollment.md` §4) is designed, not built |
| Island mode (ADR 0206, ADR 1165) | Inside an island the site's own management is reachable, so peers get fresh maps from it and the cache is only the last resort when no management at all is reachable. Each site signs with its own key (A1). Revocations are set-union and absorbing, matching ADR 1193 and class 3 of ADR 1165 D3 |
| Highest-echelon site (ADR 1205) | Owns the root document's signer list and the bound, so it can sever a site and shorten island lifetimes |
| Internal time (ADR 1037) | Ages and windows compare clock readings taken on goat's own synchronized time, never an outside UTC source. K2 compares issue times with each other, so it needs only that the sites' clocks agree |
| Warrant expiry (ADR 1129) | A warrant that expires during a cut-off is applied by management in the next fresh map; on an island the A3 bound is the longest a removed warrant can outlive its expiry |
| Silent control plane (ADR 0313) | Unchanged: the cache removes the need to reach management at start, not the path to it |

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep upstream behavior | Every device that restarts during a management outage stays off the mesh until management returns |
| Cache the map without a time bound (ZeroTier's model) | An island keeps a peer's authorization forever; a revocation made elsewhere never applies (A3) |
| Start from the cache without the last endpoints, waiting for signal | Signal is usually lost with management, so peers would never handshake; the last direct endpoint is what lets WireGuard reconnect on a local network |
| Run a management replica on every device | Every device would hold the whole account database and the policy engine; far larger than a signed per-peer map |
| Trust the cache file by file permissions only, forever | A local root user could add peers and rules that survive a restart. Acceptable only as phase 1 |
| Copy one goatnet signing key to every site (ZeroTier's high-availability model) | One key compromise is the whole goatnet's; no site can be severed (A1) |

## Consequences

**Positive.** A site that loses its uplink keeps its devices meshed across restarts, up to the bound. Every map becomes a signed artifact that can be verified at rest. Revocations made at any site reach islands through any connected peer.

**Negative.** The fork carries a client and management change set that upstream does not have, to be rebased on each upstream sync. A cached map is a new secret-bearing file. A peer whose address changed during the outage cannot be reached from the cache. Phase 1 alone trusts the cache by file permission until phase 2 adds signatures.

## Assessor view (SCA-R)

The decision bounds the lifetime of a disconnected authorization (AC-12-like session bound, measured on the internal clock), authenticates configuration at rest and in transit (SC-8, SI-7 integrity of the map), stores revocations durably and distributes them peer to peer (AC-2(3), IA-5(2) revocation), and keeps identity on WireGuard keys and SPIFFE IDs (IA-3). Residual risk: between phases 1 and 2 the cache's integrity rests on root-only file permissions and disk encryption; compensating control is the 72 h default bound and the binding to the management URL and peer key. Algorithms are FIPS 186-5 now with a FIPS 204 hybrid path.

## Trigger conditions for revisit

- Upstream NetBird ships its own offline start or signed maps.
- A goatnet needs islands longer than 30 days without any reachable management.
- The fork's Go toolchain carries a validated ML-DSA implementation (switch signatures to the hybrid set).
- A measured outage shows peers that cannot reconnect from cache because their endpoints changed (consider a local peer-discovery broadcast).

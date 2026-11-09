# goat-client — Cross-platform daemon + GUI for wg-cp0 onboarding

**Status:** Canon through **v0.3.7** (2026-07-31, latest release). The v0.2 three-mode triad (Blocks 76N–Q, [ADR 0840 Amendment 2026-05-13](../adr/0840-goat-client-cross-platform-daemon-gui.md)) shipped in v0.2.0; v0.3.x added the inner-mesh per-peer features (ADR 1063), the device-wide name-fallback resolver (ADR 1082), OS installers, and the wedge-class fixes validated 6/6 against a real management plane by the 2026-07-31 assay (cloud-env-lifecycle.md `goat-client-assay` row).

**Readiness, plainly — desktop-headless yes, general distribution no.** What ships is six-target desktop binaries plus `.deb`/`.rpm`/`.dmg`/`.msi`, cosign-signed, publicly downloadable at `get.goat.example` ([goat-client-distribution.md](goat-client-distribution.md)). Four things stand between that and putting it in a non-engineer's hands, none of them code-complete:

| Gap | State |
|---|---|
| **OS code signing** | Absent. Builds are cosign-signed (supply-chain provenance) but not Apple Developer ID-notarized or Authenticode-signed, so macOS needs `xattr -d com.apple.quarantine` and Windows shows SmartScreen. Procurement item, open since v0.1.0 (F-208 — notarization plumbed, dormant until secrets exist) |
| **Update path** | None. No auto- or self-update code exists; every fix needs a manual re-install, so the installed base cannot be moved |
| **Mobile** | Source-complete, never distributed. iOS + Android shells and the gomobile SDK are in-tree, `mobile.yml` is an advisory bind smoke, and **no release carries a mobile artifact** — no store, no MDM path. This is also the one platform where the embedded CA is load-bearing (§10.1) |
| **Windows** | Unvalidated live. The MSI passed flags the daemon doesn't register through 0.3.6 — the same defect class as the deb/rpm/dmg units — fixed in 0.3.7 by source review, never by a live install |

**End-user install + run:** [`docs/quickstart.md` in the goat-client repo](https://github.com/USAREUR-AF/goat-client/blob/main/docs/quickstart.md) — per-OS download + cosign verify + install + bundle import + connect verify, ~10 min on a laptop. The release archive carries both binaries (daemon + GUI); the GUI-from-source build the quickstart once described was a v0.1.0-only necessity. Maturity caveat: operator-class first-contact dogfood, not non-engineer end-user distribution yet.
**Authoritative ADR:** [`0840-goat-client-cross-platform-daemon-gui.md`](../adr/0840-goat-client-cross-platform-daemon-gui.md).
**Repo:** [`USAREUR-AF/goat-client`](https://github.com/USAREUR-AF/goat-client) — public, Apache 2.0.
**Companion:** [`snitch-app.md`](snitch-app.md) (Block 77, separate app — different problem, same client-tooling pattern).

---

## 1. Problem

Today, getting a device onto the Block 22 silent control plane (`wg-cp0`) requires:

1. Operator runs [`bundle-create`](../../ops/enrollment/cmd/bundle-create/) against the offline CA on a workstation.
2. Bundle is delivered out-of-band to the device.
3. Device runs [`wg-cp0-bundle-apply`](../../ops/enrollment/cmd/wg-cp0-bundle-apply/) — a Go CLI — which writes `/etc/wireguard/wg-cp0.conf` and brings up the tunnel via `wg-quick`.

This is hostile to:

- **Off-LAN end users** on phones and laptops who need to reach mesh services from outside any operator-controlled site.
- **Non-engineer roles** (program staff, partner liaisons, sponsors, viewers) who should not be expected to drive a CLI.
- **Mobile platforms** (iOS / iPadOS / Android) where there is no `wg-quick`, no `/etc/wireguard/`, no shell, and no offline-enrollment binary.
- **Engineers onboarding their own workstations** — empirically validated 2026-05-09: see §1.1 below.

[netbirdio/netbird](https://github.com/netbirdio/netbird) solved exactly this problem class for the *inner* mesh: a Go daemon paired with a Fyne desktop GUI on Linux/macOS/Windows and gomobile-wrapped daemons with native Swift / Kotlin shells on iOS / Android. The pattern works, the codebase is MIT, the install experience is "download app, paste setup key, click Connect."

`goat-client` replicates that ease-of-use for the **outer** wg-cp0 tunnel — using the existing [Block 22K offline-enrollment bundle](offline-enrollment.md) as the onboarding artifact instead of a setup key.

## 1.1 Why now — pulled-forward urgency (2026-05-09 reframe)

Block 76 was originally written as a "user-experience polish for off-LAN / non-engineer / mobile" item — a desirable future build, not load-bearing for any current deployment. The 2026-05-09 first-operator-laptop onboarding to goat-prod from macOS demonstrated empirically that the engineer path is also painful enough to warrant goat-client as **operationally needed near-term**, not "future polish."

**What happened:** the trunk owner attempted the simplest possible engineer onboarding flow — "operator laptop, macOS, mint a bundle, apply it, run `netbird up` against the silent control plane, reach the mgmt-API." Three layered findings stacked end-to-end, blocking enrollment entirely:

1. F-090 — macOS Security framework refuses Ed25519 root certs from `security add-trusted-cert`. Operator could not import the offline-CA root via the standard OS dialog.
2. F-103 — Traefik default cert config served leaf-only on the wire, no intermediate. Even with the root trusted, chain validation failed because Traefik did not present the intermediate.
3. F-104 — Go gRPC dial passed `host:port` as `tls.Config.ServerName`, breaking IP-SAN match for IP-literal mgmt-API URLs (the natural shape over wg-cp0 since there is no public DNS by ADR 0313).

**Bridging fixes** (all landed 2026-05-09, all surface-area we own): netbird-fork patch at `github.com/USAREUR-AF/netbird` commit `c5841359c` (`client/grpc/dialer.go` — embed offline-CA root via `go:embed` + strip port from ServerName via `net.SplitHostPort`); Traefik role renders leaf+intermediate fullchain; operator-laptop-onboarding playbook gained Phase 3.5 trust-anchor + Phase 3.6 patched-binary install. These worked for *us* but were operator-fired-CLI-driven, not viable for non-engineer users at any deployment scale beyond the trunk team. (The playbook's Phase 3.5/3.6 were retired 2026-07-20 — ADR 1083 Track B6 — when operator-laptop onboarding moved to installing goat-client from `get.goat.example` and importing a v0.2 combined-mode bundle; the fork-side fixes live on through `client/embed`.) The fork now embeds no certificate: its dialer adds the roots in `grpc.ExtraRootCAsPEM`, and goat-client sets it from every `*.pem` file compiled in from its `internal/ipc/grpc/trust/` directory. The public goat-client tree ships no certificate and pins no enrollment-bundle signing key (`internal/trustanchor/anchors.yaml` is empty), so a public build trusts the system roots only; a deployment adds its CA root and bundle-signing keys before building.

**Why this elevates Block 76 sequencing:**

- **Today's CLI-and-fork path is operator-fired engineering work for every new laptop.** Each operator and future partner-side engineer becomes a one-off ceremony of: (a) clone repo, (b) `aws-vault exec` to push allowlist, (c) operator-laptop-onboarding playbook with sudo, (d) accept the trust-anchor, (e) install the goat-patched netbird binary, (f) `netbird up` with the carefully-formed hostname URL. Each of those is its own small failure mode; today's onboarding burned ~6 hours operator-time chasing the three findings above before the substrate worked.
- **Engineers are the canary.** If the trunk owner — who *built* the substrate — burned a day on first-laptop, the second / third / fourth engineer hits the same wall. Partners and end users hit it harder.
- **goat-client collapses the multi-step ceremony into a single binary** with the bundle as input + click Connect as output. Same trust-anchor pinning, same port-strip patch, same fullchain handling — but compiled in once, not re-discovered per-laptop.
- **The two architectural-fix paths compose, not compete.** Block 79 Ed25519 → ECDSA P-256 root rotation makes stock OS trust paths work natively (every macOS / iOS / Android / Windows / Linux client accepts ECDSA P-256 roots in their native dialogs); Block 79 reduces Block 76's load-bearing surface but does not eliminate it (the Block 22K offline-enrollment bundle import + wg-cp0 tunnel-keep-alive + reachability prober + native UI surface remain). Block 76 ships even if Block 79 lands first — embed-CA becomes defense-in-depth rather than load-bearing.

**Sequencing implication:** Block 76 was sized as 6-10 weeks single engineer + parallel mobile tracks. The "future polish" framing implied "ship when convenient." The 2026-05-09 evidence flips this: every additional engineer onboarding to goat-prod through 2026-Q3 pays a recurring ~6-hour cost without goat-client. The break-even point against engineer-time-saved alone is plausibly under three operator onboardings — which the project will exceed within a month given the active operator cohort and inbound partner-staff onboarding.

**Open question — does the 2026-05-09 incident change v1 scope?** No. The bridging fixes (embed-CA via `go:embed`, ServerName port-strip) translate cleanly into goat-client v1 per §10.1 + §10.2 below. The pulled-forward urgency changes *when*, not *what*.

## 2. Scope

### In scope (v1)

- Cross-platform daemon (Go) that drives `wireguard-go` (or kernel WireGuard where available) to maintain the wg-cp0 tunnel.
- Desktop GUI shell (Go + Fyne) for Linux + macOS + Windows.
- Mobile clients on iOS (Swift + NetworkExtension) and Android (Kotlin + VpnService), each wrapping the same Go daemon via gomobile.
- Bundle import flow: file picker (desktop), QR scan + share-sheet + file picker (mobile), drag-and-drop (desktop).
- Bundle verification against pinned offline-CA trust anchor — refuses unsigned / expired / wrong-issuer bundles.
- Tunnel up / down, auto-reconnect on network change, status surface (interface state, last handshake, bytes in/out, peer pubkey, configured endpoint).
- Diagnostic surface: WireGuard log tail, "test connection" button, per-relay reachability indicators.

### Scope note — inner-mesh netbird is embedded (as of v0.2)

- **Running the inner-mesh netbird is now IN scope (superseding the original v1 boundary).** The `v1` design deliberately shipped the outer wg-cp0 tunnel only and deferred the inner mesh. That deferral is over: goat-client v0.2 **embeds the netbird client as a compiled-in library** — `github.com/netbirdio/netbird/client/embed` in `internal/innermesh/netbird.go`, built against **our fork** (`go.mod`: `replace github.com/netbirdio/netbird => github.com/USAREUR-AF/netbird`). See the mode triad below: `netbird-only` (inner mesh only — "goat-client effectively replaces stock netbird"), `combined` (both tunnels), and `wg-cp0-only` (the original v1 behavior, now one mode of three). Because the embed tracks our fork, **fork-level client changes ship through goat-client** — e.g. the ADR 1081 storm-resilience client patches reach every goat-client user when its netbird-fork dependency is bumped (currently pinned at branch `goat/extra-root-ca`, commit `816c31c65`, which pre-dates those patches). The historical out-of-scope wording ("maintains only the wg-cp0 outer tunnel") is retired.
- **Bundle issuance.** goat-client is a *consumer* of bundles minted operator-side via the existing offline-CA ceremony. Issuing bundles from the client would require shipping the CA private key — never.
- **Mesh-service-level UX.** Once the tunnel is up, mesh services are reached the same way they always have been (browser to `mgmt.<site>.netbird.legacy.example`, ssh to mesh IPs, etc.). goat-client does not embed a service browser, dashboard, or chat client. **Seam note:** per-*peer* affordances — a live connectivity check (latency graph + direct/relayed badge) and a peer file drop — are *mesh-peer* UX, not *mesh-service* UX, and are scoped **post-v1** in [`goat-client-connectivity-and-filedrop.md`](goat-client-connectivity-and-filedrop.md) ([ADR 1063](../adr/1063-goat-client-connectivity-check-and-file-drop.md)). They live on inner-mesh peers (`netbird-only`/`combined` modes), so this out-of-scope line stands for v1; the new design names where the boundary moves.
- **Spock / management-plane / signal-server logic.** Those are server-side concerns; goat-client is a peer client only.
- **Bundle-revocation propagation.** v1 uses the existing CRL pipeline ([`docs/design/offline-enrollment.md`](offline-enrollment.md) §6). Real-time revocation push is a Phase 3+ extension.

### Future (post-v1)

- **v0.2 path-A: embed netbird inner-mesh as a library inside one PacketTunnelProvider on mobile.** Architectural commitment per [ADR 0840 amendment 2026-05-10b](../adr/0840-goat-client-cross-platform-daemon-gui.md). Closes the mobile tunnel-slot-mutex gap (F-109) — one app, one VPN slot, both wg-cp0 outer + netbird inner active. Desktop coexistence stays unchanged (no platform-level mutex on desktops). See §"Mobile coexistence" below.
- BLE-based zero-network onboarding for air-gap pairing.
- Push-based renewal notifications when bundle TTL approaches expiry.
- **Multi-network UI switching (queued for v0.2, alongside netbird-mesh embed).** One goat-client install holding multiple wg-cp0 bundles (multi-site) AND multiple netbird-mesh profiles (multi-mesh; e.g. dev/test + prod), with a UI affordance to switch between them in one click. Motivation: stock NetBird CLI already supports `netbird profile add` / `netbird up --profile <name>` for multi-mesh enrollments and works cleanly, but stock NetBird's macOS tray GUI is **broken** for self-hosted profiles — tray-submenu profile-select triggers a SaaS Auth0 OIDC redirect (`login.netbird.io`), and the Settings → Connection → Management URL editor wipes the daemon's cached enrollment creds on Save (lands the daemon in `LoginFailed`). See `docs/operations/troubleshooting.md` for the full symptom matrix. goat-client v0.2 is the natural place to do this right: the netbird-mesh embed (ADR 0840 amendment 2026-05-10b) gives us full control of the profile-state-and-URL plumbing instead of layering UI on top of stock netbird's broken self-hosted handling. Scope includes per-network status indicators in the tray, durable profile-state persistence that doesn't get clobbered by GUI actions, and a "switch to" action that re-uses cached creds without re-prompting for setup keys. Subsumes the earlier "multi-bundle support (one device → multiple sites)" bullet — the two axes (multi-site bundles + multi-mesh profiles) compose into one switching surface.

### Operating modes across platform classes (added 2026-05-13)

goat-client supports **any subset of {wg-cp0 outer, netbird inner}**, selected at install or runtime. Three useful modes apply uniformly across mobile / desktop / headless:

| Mode | wg-cp0 outer | netbird inner | When to use |
|---|---|---|---|
| `wg-cp0-only` | ✅ active | ❌ | Stock netbird is composed separately (Ansible role, packaged installer, multi-profile CLI). Today's v0.1.x default on every platform; useful on fleet hosts where stock netbird already works cleanly, and for testing / degenerate cases (e.g. a phone that only needs wg-cp0 reach). |
| `netbird-only` | ❌ | ✅ active | goat-client reaches the mgmt-API + signal + relay through the Block 80 public-mTLS crutch tier (mTLS client cert from the bundle's `mobile_cert` field per ADR 0843 Block 80F). No wg-cp0 outer tunnel claimed; the inner mesh is the only tunnel. **goat-client effectively replaces stock netbird** for the surface a user sees — same offline-CA trust anchor, same UX shell, same bundle import. Mobile-friendly (no system-VPN slot wasted on a wg-cp0 outer when the crutch is up); desktop-useful on locked-down hosts where the crutch is operator-policy-mandated. |
| `combined` | ✅ active | ✅ active | Both tunnels in one Go runtime. Reaches the silent-control-plane (wg-cp0) AND participates in the inner mesh natively. Mobile-mandatory shape when the operator wants both layers on the device (single PacketTunnelProvider per F-109). Desktop / headless can opt in for single-binary "one install brings up everything" deployments. |

**Mobile constraint.** A mobile device gets at most one system-VPN slot (one PacketTunnelProvider on iOS, one VpnService on Android). goat-client mobile fills that one slot with whichever single mode the operator-issued bundle + the deployment posture call for. Modes can switch between sessions; two PacketTunnelProvider extensions cannot run side-by-side regardless. Coexistence with stock netbird mobile remains incompatible per F-109 — the platform mutex blocks two providers, not two tunnels-in-one-provider. The escape from that bind is to use goat-client mobile (one slot, any of the three modes) and not stock netbird mobile — `netbird-only` is precisely the "replace stock netbird mobile" path.

**Desktop / headless.** No platform mutex; all three modes are valid runtime states. The same `goat-clientd` binary picks the mode from install-time flag (`--mode wg-cp0-only|netbird-only|combined`), runtime IPC call (`setMode`), or detection from bundle contents (presence of `inner_mesh_setup` field + presence of `mobile_cert` field).

**Why three modes, not two.** Block 80 crutch tier exposes mgmt + signal + relay over public mTLS as a parallel reachability surface. A goat-client install on a device that reaches goat through the crutch (and not through wg-cp0) is still a useful client — it just doesn't claim a tunnel-slot for wg-cp0. On mobile this is meaningful: a user on the Block 80 crutch can run *only* the netbird inner mesh in their single PacketTunnelProvider and reach the rest of goat without burning the slot on a wg-cp0 outer that the crutch already makes redundant. On desktop this is meaningful: a locked-down partner laptop that lives behind the crutch can run goat-client as a stock-netbird replacement without ever needing the wg-cp0 outer. Without `netbird-only`, the only two options would be "no goat-client at all (use stock netbird)" or "full combined-mode goat-client (wasted wg-cp0 surface where the crutch handles reach)." Three modes preserves the operator's freedom to match the substrate posture per-device.

**Why both `wg-cp0-only` and `combined`, not just one or the other.** Operating reality is two-shaped: some hosts already run stock netbird under Ansible / packaged installers / per-mesh-profile workflows we don't want to replace; others (partner laptops, sponsor demos, non-engineer end-users, single-purpose Orin sites) benefit from one binary that brings up everything. Forcing either shape across the whole fleet creates churn we don't have to take.

**Behavior-parity goal.** Mobile and desktop/headless share the same daemon, same IPC surface, same status model, same diagnostics, same bundle handling, same trust-anchor pinning. The UI surface differs (Fyne tray vs Swift/Kotlin native) but the *capabilities* are the same — a user moving between desktop and mobile in the same mode sees the same per-tunnel status, the same setup flows, the same failure surfaces. Inactive tunnels in any mode are dormant (IPC returns `disabled`), not missing.

**Headless shape.** `goat-clientd --headless --mode <mode>` runs as a Linux systemd service (or equivalent) with no Fyne UI dependency. Bundle import via the same IPC surface (`goat-client` CLI client, or a one-shot `goat-clientd --import-bundle <path>` mode), or via the existing `wg-cp0-bundle-apply` CLI which stays alive as the operator-fired ceremony path for the modes that include wg-cp0. Single-Orin sites, partner-prep stations, and Block 60 moon-pods get a single-binary "join goat" shape; locked-down server-class hosts behind the crutch get a single-binary "join the inner mesh" shape via `netbird-only`.

**Composes with Block 80 without depending on it.** Block 80's public-mTLS crutch tier is the substrate that makes `netbird-only` mode reachable; without it, `netbird-only` mode is conceptually well-defined but operationally degenerate (no path to mgmt). `wg-cp0-only` and `combined` modes do not depend on Block 80. Block 80 work proceeding in parallel does not block 76N / 76O / 76P / 76Q; it gates the `netbird-only` mode's verdict (meaningful `netbird-only` end-to-end testing requires the crutch tier to be live somewhere — bench-env or production).

### Mobile coexistence

iOS `NEPacketTunnelProvider` and Android `VpnService` are single-claimant system-VPN slots: only one app at a time can hold one. This is documented in F-109. The implication for the §"Operating modes" framing above is that mobile fills its single slot with whichever one of the three modes the deployment calls for — it cannot run two side-by-side. Two consequences:

- **v0.1.x mobile shells** (Block 76 sub-tasks 76I + 76J) hold only the wg-cp0 outer tunnel — that's the `wg-cp0-only` mode of the v0.2 triad, shipped early. A v0.1.x mobile install + stock netbird mobile install still collide on the system-VPN slot.
- **v0.2 mobile** (76Q) extends to all three modes: `wg-cp0-only` (today's v0.1.x), `netbird-only` (replaces stock netbird mobile; reaches mgmt over Block 80 crutch), and `combined` (path A per ADR 0840 amendment 2026-05-10b — both tunnels in one PacketTunnelProvider). The user / operator picks the mode that matches the deployment posture at bundle-issue time or at install time.

The [Block 80 / ADR 0843](../adr/0843-public-mgmt-signal-relay-mobile-crutch.md) crutch tier is no longer just a "bridging window before v0.2" — it is the permanent substrate that makes `netbird-only` mobile reachable. After v0.2 ships, the crutch stays in the toolbox for the union of rationales: `netbird-only` mobile reachability + the ADR 0843 D5 list (partner devices, emergency on-ramp, locked-down operator devices, external-perspective validation).

## 3. Architecture

```
┌──────────────────────────── User device ──────────────────────────────┐
│                                                                       │
│  ┌────────────────────┐         ┌──────────────────────────────────┐  │
│  │   goat-client GUI  │  IPC    │       goat-client daemon         │  │
│  │  (Fyne / Swift /   │◄───────►│       (Go, single binary)        │  │
│  │   Kotlin)          │         │                                  │  │
│  │                    │         │  ┌────────────────────────────┐  │  │
│  │  - Import bundle   │         │  │ Bundle parser + verifier   │  │  │
│  │  - Connect/Discon. │         │  │  (CBOR, Ed25519 sig check) │  │  │
│  │  - Status display  │         │  └────────────────────────────┘  │  │
│  │  - Diagnostics     │         │  ┌────────────────────────────┐  │  │
│  └────────────────────┘         │  │ Tunnel manager             │  │  │
│           │                     │  │  - desktop: wg-quick / wg  │  │  │
│           │                     │  │  - iOS: NetworkExtension   │  │  │
│           ▼                     │  │  - Android: VpnService     │  │  │
│  ┌────────────────────┐         │  └────────────────────────────┘  │  │
│  │   Local config     │         │  ┌────────────────────────────┐  │  │
│  │   ~/.goat-client/  │         │  │ Reachability prober        │  │  │
│  │  (bundle, state)   │         │  │ (per-endpoint TCP/UDP test)│  │  │
│  └────────────────────┘         │  └────────────────────────────┘  │  │
│                                 └──────────────┬───────────────────┘  │
└────────────────────────────────────────────────┼──────────────────────┘
                                                 │
                                                 ▼ UDP 51821 (wg-cp0)
                                       ┌─────────────────────┐
                                       │  wg-cp0 relay tier  │
                                       │  (silent listener)  │
                                       └─────────────────────┘
```

### Components

| Component | Role | Platform |
|---|---|---|
| `goat-clientd` | Daemon: bundle parser + tunnel manager + reachability prober | All five (Linux/macOS/Windows desktop + iOS/Android mobile) |
| `goat-client` (GUI) | Fyne desktop GUI shell | Linux / macOS / Windows |
| `GoatClient.app` | Swift UI shell wrapping daemon framework | iOS / iPadOS |
| `goat-client.apk` | Kotlin UI shell wrapping daemon AAR | Android |

### Daemon ↔ GUI IPC

- **Desktop:** Unix domain socket (Linux/macOS) at `~/.goat-client/sock`, Windows named pipe at `\\.\pipe\goat-client` — same wire protocol (length-prefixed JSON-RPC over the bytestream).
- **iOS:** in-process — daemon is a static framework linked into the host app per Apple NetworkExtension constraints.
- **Android:** AIDL or local bound-Service IPC; daemon runs in a foreground service surface for VpnService lifetime.

### Bundle handling

The bundle format is the existing [Block 22K CBOR offline-enrollment bundle](offline-enrollment.md). On import:

1. Read bundle from input source (file / QR / share-sheet).
2. Parse CBOR, extract Ed25519 signature.
3. Verify signature against the **pinned offline-CA trust anchor** compiled into the goat-client binary (per-build constant; rotates with new offline-CA roots; multiple anchors active during transition windows). The trust anchor is *not* user-configurable — that would defeat the offline-CA chain.
4. Verify expiry (`not_after` in bundle header).
5. Verify pairing-invariant fields (per-device pubkey matches the bundle's `cp_device_pubkey` half).
6. Persist the relevant pieces (wg-cp0 conf, peer pubkey, allowed-IPs, endpoint set) to encrypted local storage.
7. Discard the rest of the bundle (the bundle carries fields like `tls_trust_anchor`, `snitch_probe_key` that are out-of-scope for goat-client v1).

### Tunnel platform abstraction

| Platform | Tunnel mechanism | Privilege model |
|---|---|---|
| Linux | kernel WireGuard via `wg-quick` (root) or `wireguard-go` (CAP_NET_ADMIN) | Daemon runs as systemd service with capabilities |
| macOS | `wireguard-go` (kernel WG not available) | Daemon runs as launchd service; GUI runs as user |
| Windows | `wireguard.dll` from upstream WireGuard for Windows | Daemon runs as Windows Service |
| iOS | NetworkExtension Packet Tunnel Provider (Apple-mandated) | Per-app entitlement; runs in extension process |
| Android | VpnService API (Android-mandated) | App-scoped; foreground service lifetime |

The Go daemon presents a single `Tunnel` interface; per-platform shim packages implement it. This mirrors netbird's existing `client/internal/dns/` and `client/iface/` per-platform structure — we fork that pattern, not the netbird-mesh-specific assumptions on top.

## 4. Onboarding flow

The user's experience on each platform.

### Desktop (Linux / macOS / Windows)

1. Operator hands user a `goat-client-install.{deb,rpm,dmg,msi}` from a release.
2. User installs (one click / one apt command). Daemon installs as a system service; GUI installs as a per-user app.
3. Operator delivers `bundle.cbor` out-of-band (encrypted email, signed Matrix message, USB stick, etc.).
4. User opens goat-client GUI → drag bundle file onto the window OR `File → Import Bundle...`.
5. GUI shows bundle metadata (issued-to name, site, expires, peer pubkey, endpoints). User clicks **Apply**.
6. GUI shows the **Connect** button. User clicks it.
7. Tunnel up. Status shows green. User can now reach mesh services via the wg-cp0 path.

### Mobile (iOS / Android)

1. User installs `goat-client` from App Store / Play Store / sideload.
2. Operator generates a QR-encoded bundle (or short-lived file delivery via Matrix / iMessage / Signal share-sheet).
3. User opens goat-client → **Scan Bundle QR** OR **Open Bundle File**.
4. iOS / Android prompts for VPN-permission (one-time, Apple/Google-mandated).
5. Bundle imported, **Connect** button shown.
6. User taps **Connect**. Tunnel up. Status shows green.

### Why bundles, not setup keys

netbird uses a setup key — a string the user pastes into the app, which then talks to the management server to retrieve the actual peer config. wg-cp0 is *silent by design* per [ADR 0313](../adr/0313-silent-control-plane-outer-wg-wrap.md): there is no public discovery endpoint to resolve a setup key against. The bundle carries the full tunnel config + peer pubkey + endpoint set, signed by the offline CA, with no online discovery step. This is structurally stronger than netbird's setup-key model for the wg-cp0 layer.

## 5. Repo layout (planned)

```
USAREUR-AF/goat-client/
├── cmd/
│   ├── goat-clientd/        # daemon binary (desktop + entry for gomobile)
│   └── goat-client/          # Fyne GUI binary (desktop only)
├── internal/
│   ├── bundle/               # CBOR parse + sig verify (shared with bundle-extract)
│   ├── tunnel/               # platform-abstracted tunnel manager
│   │   ├── tunnel_linux.go
│   │   ├── tunnel_darwin.go
│   │   ├── tunnel_windows.go
│   │   ├── tunnel_ios.go     # gomobile entry — implements via Apple NE
│   │   └── tunnel_android.go # gomobile entry — implements via Android VpnService
│   ├── ipc/                  # JSON-RPC over unix socket / named pipe / in-process
│   ├── reachability/         # per-endpoint probe loop
│   └── ui/                   # Fyne GUI components
├── mobile/
│   ├── ios/                  # Xcode project, Swift shell, NE extension target
│   └── android/              # Gradle project, Kotlin shell, VpnService
├── docs/
│   └── README.md             # build + install
├── packaging/
│   ├── deb/
│   ├── rpm/
│   ├── dmg/
│   ├── msi/
│   └── github-actions/       # CI: 5-platform build matrix + signed releases
└── go.mod
```

## 6. Build + release

- Five-platform CI matrix mirroring [Block 61H multi-platform snitch builds](snitch.md): `linux/{amd64,arm64}`, `darwin/{amd64,arm64}`, `windows/{amd64,arm64}` for desktop; gomobile-bound iOS .xcframework and Android .aar for mobile.
- Reproducible builds via `-trimpath -buildvcs=false`; cosign-signed binaries + per-asset `.sha256` (per Block 50B.5 reproducible-binaries pattern).
- Code signing: macOS Developer ID + notarization, Windows Authenticode cert, iOS App Store distribution profile, Android Play signing key. Code-signing is the single highest-friction operational requirement and is the lateral block for getting v1 to mobile users.
- Release cadence: tagged releases at `goat-client-v<semver>`; CI fans out binaries + manifests to GitHub Releases.

## 7. Trust anchor model

The pinned offline-CA trust anchor in the goat-client binary is what makes bundle verification meaningful. Two failure modes:

1. **Trust anchor compromise** — if the offline-CA root key is compromised, every issued bundle is potentially attacker-controlled. Mitigation: ADR 0406 YubiKey-PIV custody (private key never leaves the YubiKey); root rotation per the same ADR.
2. **Stale binary with retired trust anchor** — old goat-client builds carry old trust anchors that may no longer correspond to live bundle issuance. Mitigation: multiple-anchor support during rotation windows; in-app expiry warning when a bundle is signed by an anchor older than the local-build cutoff; periodic forced-update prompts.

Trust-anchor rotation runbook lives at `docs/operations/goat-client-trust-anchor-rotation.md` (drafted as part of v1 implementation).

## 8. Relationship to existing components

- **[`bundle-create`](../../ops/enrollment/cmd/bundle-create/) (operator side):** unchanged. Continues to mint bundles via the offline CA. May gain an optional `--qr` output flag for mobile-friendly delivery.
- **[`bundle-extract`](../../ops/enrollment/cmd/bundle-extract/) (device side):** unchanged. Stays as the operator-fired CLI for diagnostic / scripted extraction. goat-client uses the same bundle parser library internally but does not call the CLI.
- **[`wg-cp0-bundle-apply`](../../ops/enrollment/cmd/wg-cp0-bundle-apply/) (device side):** unchanged. Stays as the operator-fired CLI for headless / Ansible-driven Orin sites. goat-client is the GUI alternative for end-user devices, not a replacement for the CLI on operator-managed nodes.
- **[Block 22K](offline-enrollment.md) bundle format:** unchanged at v1; an optional `bundle_v2` extension for QR-encodable compact form may follow as a separate sub-block if the v1 CBOR is too large to fit a reasonable QR (CBOR bundles are typically ~1.5 kB; QR-25 codes hold ~2.9 kB at error-correction level L; v1 should fit comfortably without compaction).
- **[`snitch-app`](snitch-app.md) (Block 77):** sibling, separate app, separate repo. Both follow the same Go-+-Fyne / gomobile-+-native pattern but solve different problems.
- **netbird upstream:** goat-client forks netbird's *client* tree (MIT) for the desktop GUI scaffolding, gomobile setup, NetworkExtension/VpnService wiring, and tunnel-manager pattern. We strip everything netbird-mesh-specific (signal, management-API, peer-discovery) and replace it with bundle-based wg-cp0 onboarding. Forking is preferred over clean-room because the platform-specific wiring is the bulk of the work and it already works in netbird's tree.

## 9. Open design questions

| # | Question | Decision deadline | Notes |
|---|---|---|---|
| Q1 | Single bundle slot vs multi-bundle (multi-site)? | v1 | Lean: single slot. Multi-bundle is a v2 ask. |
| Q2 | Does the desktop GUI run as user or admin? | v1 | Lean: GUI runs as user; daemon runs as service with privilege; GUI invokes daemon via IPC for tunnel ops. |
| Q3 | Auto-update on / off / opt-in? | v1 | Lean: opt-in for v1; operator-controllable in a future enterprise build. |
| Q4 | Does the binary embed netbird-client as well, or stay strictly wg-cp0-only? | v1 | Lean: strictly wg-cp0-only for v1. Co-residency adds shipping/signing/UX complexity that gates the first release. |
| Q5 | Mobile app store distribution: public store vs enterprise/MDM? | post-v1 | Public store is the easier path for the broad-user case; MDM is the operator path. Both eventually. |
| Q6 | Trust-anchor pinning per build vs in-app refresh? | v1 | Lean: pinned per build (security). Refresh requires a separate signed-channel for anchor delivery; deferred. |
| Q7 | Logging: what gets emitted, where does it go, what's the redaction policy on mobile? | v1 | Mobile especially — no `journalctl` analog; logs are app-scoped. Decide before first release. |

## 10. Implementation tracking

Block 76 sub-tasks live at `docs/project/implementation-plan.md`. Backlog entry at `docs/project/backlog.md`.

### 10.1 Embed-CA-via-go:embed pattern — validated 2026-05-09

The first operator-laptop onboarding to goat-prod NetBird from macOS (2026-05-09) hit F-090 (macOS Security framework refuses Ed25519 root certs) + F-103 (Traefik leaf-only on wire) + F-104 (Go gRPC ServerName-with-port fails IP-SAN match). The bridging fix landed in `github.com/USAREUR-AF/netbird` commit `c5841359c`: embed the offline-CA root via `go:embed` in `client/grpc/dialer.go` + append to RootCAs pool unconditionally + strip port from ServerName via `net.SplitHostPort`.

**Implication for goat-client v1:** the embed-CA pattern is validated as a path that works on macOS pure-Go binaries. Same pattern translates to the Fyne desktop (direct `go:embed`), iOS (gomobile resource bundle), and Android (assets) builds.

**Sequencing relative to Block 79 Ed25519 → ECDSA root rotation (revised 2026-05-11 post-closure):** Block 79 closed 2026-05-10 with stock NetBird 0.70.5 onboarding goat-prod end-to-end via the macOS System keychain (verified path B test 2026-05-11). The embed-CA pattern's load-bearing status now splits by platform:

- **Desktop (macOS, Linux, Windows):** OS trust stores ALL accept ECDSA P-256 roots via standard per-OS dialog (`security add-trusted-cert` / `update-ca-certificates` / `Certmgr.msc`). Stock NetBird's Go crypto/tls path validates cleanly. Embed-CA on desktop is **defense-in-depth only** — protects against trust-store-compromise threat model, but the standard path doesn't need it.
- **Mobile (iOS, iPadOS, Android):** sandbox restrictions PREVENT app-level modification of the OS trust store. There is no `security add-trusted-cert` on iOS; Android user-mode apps cannot install system CAs. Embed-CA remains **load-bearing** on mobile and is the only viable pattern for a netbird-derived mobile app to trust the goat offline-CA chain. Block 76 mobile builds MUST carry the embed.
- **Locked-down hosts (DoD STIG-hardened, kiosk-mode, embedded):** policy-managed trust stores may forbid operator-side root insertion. Embed-CA is **load-bearing** in those contexts too — same operational reasoning as mobile.

Q6 above (trust-anchor pinning vs in-app refresh) lands the same way: pinned-per-build is the right v1 choice for mobile (root rotates ~yearly; mobile-app cadence handles that). Desktop builds can use the OS trust store path; embed becomes belt-and-suspenders.

The fork apparatus (`github.com/USAREUR-AF/netbird` build pipeline, install paths, operator-laptop daemon plist) survives Block 79 closure for two distinct reasons: (a) mobile/locked-down platforms need embed-CA load-bearing; (b) future netbird patches for non-TLS reasons (performance, custom mesh-routing, upstream-unfixed security issues) reuse the same fork infrastructure.

### 10.2 ServerName port-strip — required regardless of root rotation

F-104 is a Go gRPC bug that surfaces when dialing IP-literal URLs. Block 76's mgmt-API connection IS subject. Pull the `net.SplitHostPort` patch into Block 76's gRPC dialer OR document operator path discipline ("always use hostname URLs"). Codified default in v1: hostname URL with /etc/hosts mapping for off-DNS sites.

### 10.3 Distinct fork-apparatus from `github.com/USAREUR-AF/netbird` — clarified 2026-05-11

There are **two separate fork apparatuses** in the goat ecosystem and they should not be conflated:

| Apparatus | Repo | Role | Trust path |
|---|---|---|---|
| **netbird-fork** (existing) | `github.com/USAREUR-AF/netbird` — installed at `/usr/local/bin/netbird → /opt/goat/bin/netbird-goat` on operator laptops today | Desktop daemon. Originally built 2026-05-09 as the bridging-fix carrier for F-090/F-103/F-104 against the Ed25519 root. Today's role post-Block-79: **dev-account daily-driver** for operators. NOT load-bearing for goat-prod (stock NetBird is the supported goat-prod desktop client). | `SSL_CERT_FILE=/etc/ssl/goat-ca-bundle.pem` (CGO_ENABLED=0 pure-Go pool) + `go:embed`ed root in `client/grpc/dialer.go`. Whether the embed-pool *appends* or *replaces* system pool is undetermined as of 2026-05-11; a bundle-file update alone did NOT make it validate the new ECDSA chain in field testing. Retired from the goat-prod path. |
| **goat-client** (Block 76, this design) | `USAREUR-AF/goat-client` (new repo, separate codebase) | Cross-platform mobile + desktop client. Mobile sandboxes force embed-CA load-bearing — this is the apparatus that carries that requirement. | Embed-only on mobile (pinned-per-build); embed + OS trust store on desktop (defense-in-depth). |

**Block 76 v0.1.x scope** is the goat-client repo, NOT the netbird-fork. The work shape:

- **Repo:** new at `USAREUR-AF/goat-client`. Forks netbird's MIT `client/` tree, strips netbird-mesh-specific logic, adds wg-cp0 bundle import.
- **Embed:** `//go:embed`s the canonical offline-CA root at `ops/enrollment/ca/dev/root-cert.pem` (X.509 CERTIFICATE PEM, ECDSA P-256 post-Block-79). NOT the retired `dev-goat-ca-2026-04-20.pem` (Ed25519, now in `ca/dev-ed25519-retired-2026-05-10/`).
- **Pubkey-load pattern:** copy from [`ops/enrollment/cmd/wg-cp0-bundle-apply/main.go`](../../ops/enrollment/cmd/wg-cp0-bundle-apply/main.go) `loadCAPubkey()` (commit `dc3944fe`, 2026-05-11) — switches on PEM block type (PUBLIC KEY or CERTIFICATE), extracts SPKI, type-asserts to `*ecdsa.PublicKey`, validates `Curve == elliptic.P256()`. 12-line pattern; copy verbatim.
- **Test fixtures:** mint ECDSA P-256 keys in setup (not Ed25519) — reference test pattern in [`wg-cp0-bundle-apply/main_test.go`](../../ops/enrollment/cmd/wg-cp0-bundle-apply/main_test.go) `TestLoadCAPubkey` cases. Test-vs-prod algorithm parity prevents F-090-class "macOS rejects Ed25519, but only-in-prod" surprises.
- **Production CA pubkey:** v0.1.x ships against `ca/dev/`. `ca/prod/` (YubiKey-PIV-rooted) is gated on Block 22M production-mode hardware ceremony; not in v0.1.x scope. Future v0.2.x or later swaps the embed source path when prod CA materializes.
- **No dual-algorithm:** Block 79 was a hard cutover. Don't carry Ed25519 verification code paths in goat-client — the Ed25519 root is retired across the substrate.
- **Block 80 interaction:** Block 80 (mobile VPN-slot exclusivity, ADR 0843, F-109) is a parallel constraint at a different layer (tunnel-slot, not TLS). It may inform whether v0.1.x ships as wg-cp0-only or wg-cp0+netbird-combined on iOS/Android — but it does NOT change the cert-chain work shape.

## 11. Glossary

See `docs/adr/GLOSSARY.md` — wg-cp0, offline-CA, bundle, gomobile, NetworkExtension, VpnService, Fyne all defined there.

# Name fallback resolver — signed snapshots for mesh names when mesh DNS is down

| Field | Value |
|---|---|
| Status | Phase 1 built (snapshot generation + signing + CI drift/sig gates + `get.<site>.*` distribution + goat-cli fallback resolver with staleness grading in `goat status` / `goat doctor`). **Also built (2026-07-15):** noncanonical observed-records tier in goat-cli (§4.2, ZAGROS live-fired). **Unified (2026-07-16):** goat-clientd fronts OS mesh-zone DNS with its forwarder (§4.3, goat-client v0.3.4) and goat-cli reads the daemon's shared store (§4.1, goat-cli 0.4.24) — one store, one refresher per device. **Committed follow-on (operator decision):** signed self-claims (§7). Phase 2 (courier propagation) remains adoption-gated (§6). Tracking: `../project/backlog.md`. |
| ADR | [ADR 1082](../adr/1082-name-fallback-resolver-signed-snapshots.md) |
| Related | `dns-service-discovery.md`, [`bootstrap-bundle.md`](bootstrap-bundle.md), `goat-cli.md`, [`goat-cli-mesh-download.md`](goat-cli-mesh-download.md), [`federated-trust-model.md`](federated-trust-model.md), [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md), F-039, F-197 (declared-but-unresolvable deploys dark) |

## 1. Problem — the failure class

**Node network up, mesh name service down or unreachable.** The mesh data
plane (WireGuard tunnels, direct IP connectivity) survives outages that the
name plane does not:

- **F-039** — CoreDNS hosts plugin silently loaded *zero* records when the
  zone file was mode 0600. Every name dark, every IP fine.
- **F-197** — a declared service with no DNS record deploys dark: containers
  up, route rendered, unreachable by name.

Mesh DNS is a single CoreDNS instance per goatnet, on the mgmt host. When it
is down, wedged, or unreachable across a partitioned link, every client that
only knows names loses every service — even though the WireGuard paths to
those services still work. goat names are the one piece of state a client
cannot re-derive locally.

## 2. The design in one paragraph

**Single-signer versioned snapshots + untrusted couriers + signed
single-owner self-claims + graded staleness.** Every registry change
regenerates the per-goatnet name set with a monotonic serial (DNS-SOA-style),
signed by the one zone authority (the existing per-goatnet offline-CA — the
same key that signs enrollment bundles; no new key infrastructure). Clients
cache the newest validly-signed snapshot and fall back to it *only when live
mesh DNS fails to answer*, labeling every fallback answer with source +
serial + age. Propagation trust lives entirely in the signature, so *any*
transport — the `get.<site>.*` download tier today, any peer or USB stick in
phase 2 — is an untrusted-but-useful courier. Phase 3 adds the one safe
CRDT-adjacent element: each peer may assert *its own* name→IP under its own
leaf cert, never overriding snapshot service names.

### Rejected alternative — distributed record-level CRDT name store

Considered and rejected. goat names are **slowly-changing single-writer
data**: the per-site registry (`isitup_services` in group_vars, rendered by
`ops/dns/generate-mesh-hosts.py`) has exactly one author — the repo — so
there is no multi-writer concurrency for a CRDT to converge. A gossiped
*writable* name table is a poisoning/interception surface: any peer could
inject mappings, and defending that means rebuilding DNSSEC-grade origin
authentication inside a gossip layer. Full-state signed snapshots make the
whole problem disappear — replace, don't merge; verify the signer, don't
trust the transport. This is [`federated-trust-model.md`](federated-trust-model.md)'s
recognition-≠-authorization split applied to names: couriers carry a
*verifiable artifact*, never *authority over records*.

## 3. Element 1 — versioned signed snapshots (phase 1, built)

### 3.1 Artifact

`ops/dns/<site>/name-snapshot.json` — committed, generated, drift-gated,
exactly like the sibling `mesh-hosts` zone file and from the same source of
truth (the group_vars service registry; one registry, two renderings):

```json
{
  "format": "goat-name-snapshot-v1",
  "site_id": "zagros",
  "zone": "netbird.zagros.example",
  "serial": 1,
  "generated_at": "2026-07-15T00:00:00Z",
  "ttl_seconds": 2592000,
  "ca_id": "dev-nova-ca-ecdsa-2026-05-14",
  "records": [ {"name": "portal.zagros.netbird.zagros.example", "ip": "100.127.0.175"}, … ]
}
```

- **`serial`** — monotonic integer, bumped by the generator **only when the
  record set changes** (regeneration with no registry change is a no-op, so
  the committed file is stable and reviewable). Clients prefer the highest
  validly-signed serial and never replace a cached snapshot with a lower one.
- **`ttl_seconds`** — the zone authority's declared staleness bound (default
  30 days; per-site override `name_snapshot_ttl_seconds` in group_vars). Past
  it, clients refuse to serve the snapshot (§5).
- **Full-state replacement, no tombstones.** A snapshot is the complete name
  set; removal is just absence in the next serial.

`ops/dns/<site>/name-snapshot.json.sig` — detached signature: base64 of an
ASN.1 DER ECDSA-Sig-Value (P-256) over SHA-256 of the exact snapshot file
bytes. Same signature shape as the enrollment bundle
(`ops/enrollment/bundle/bundle.go`), same signer machinery
(`ops/enrollment/ca/age_ca.go` `AgeSigner` — age-encrypted CA key on the
operator workstation), same client-side verification path goat-cli
uses for bundle verification (`crates/goat-cli/src/bundle.rs`). **No new key
infrastructure**: the signer is the per-goatnet offline-CA named by the site's
`nova_ca_id` group_var.

**Consequence of F-406.** goat-cli compiles in no CA root — roots are
operator-supplied through `goat ca add`, which is what lets one binary serve
several goatnets. Snapshot verification reads the *same* trust store, so a
goat-cli install with no roots added resolves nothing from a snapshot. That is
the correct fail-closed answer and it is indistinguishable, by design, from an
unverifiable snapshot; the fix is the same one command. Adding a goatnet's root
enables bundle install and name fallback together, because both are signed by
that goatnet's offline CA. The separate Go `goat-client` / `goat-clientd`
binary was not changed by that work and keeps whatever trust model its own
release carries.

### 3.2 Generation + signing workflow

`ops/dns/generate-mesh-hosts.py` (the existing zone generator) also emits
the snapshot for every site that declares `name_snapshot_enabled: true` in
group_vars (opt-in per goatnet, wired at standup — the
`instance-variability-model.md` fail-closed
pattern; ZAGROS is enabled). Registry change → rerun the generator → records +
serial + `generated_at` update → the old signature no longer verifies →
the operator re-signs:

```bash
go run ./ops/enrollment/cmd/name-snapshot-sign \
    --snapshot ops/dns/zagros/name-snapshot.json \
    --root-key ~/.goat/state/nova-ca/ca.key.age \
    --age-identity ~/.config/age/keys.txt
```

`name-snapshot-sign` reuses `AgeSigner` verbatim and appends to the same
`~/.goat/` CA audit log pattern as `cert-sign`. Commit all three
(registry, snapshot, sig) together.

**CI gates (drift + signature, both fail-closed):** the existing
`dns-registry-sync` job runs `generate-mesh-hosts.py --check`, which now also
refuses snapshot drift (records not matching the registry, or a serial that
did not bump on change); a sibling step runs
`ops/ci/check-name-snapshot-sig.sh`, which verifies every committed
`name-snapshot.json.sig` against the site's committed CA root cert via
openssl. An unsigned or stale-signed snapshot cannot merge — the F-197
lesson (silent drift deploys dark) applied to the fallback layer itself.

### 3.3 Distribution (phase 1 channels — both existing)

- **`get.<site>.<zone>`** — the goat-cli mesh-download tier
  ([`goat-cli-mesh-download.md`](goat-cli-mesh-download.md)) additionally
  serves `name-snapshot.json` + `.sig` from `/opt/goat-get/www/` (the
  `goat-get` role copies them from the repo checkout at deploy). goat-cli
  refreshes opportunistically: during `goat status` / `goat doctor` (the
  natural connectivity moments) it best-effort fetches both files (bounded
  timeout, silent on failure), verifies the signature against the embedded
  CA root, and caches them in the active context dir
  (`~/.goat/contexts/<name>/name-snapshot.json{,.sig}`) — only if the serial
  is newer than the cached one. The cache is re-verified at every *use*, not
  just at fetch, so a tampered cache file is inert.
- **Bootstrap / onboarding** — the artifact is self-verifying, so any
  transport works: an operator can drop the two files into a fresh context
  dir by hand (USB, bundle sidecar, chat attachment) and the resolver treats
  them identically. This is the phase-2 courier property already at work,
  operator-mediated. The [`bootstrap-bundle.md`](bootstrap-bundle.md) repo
  snapshot carries the committed files by construction.

The chicken-and-egg is deliberate: the snapshot is a **cache refreshed while
names work, consumed when they don't**. A client that has never reached the
goatnet has no cache — and also nothing to resolve; first contact rides
enrollment, which uses public DNS.

## 4. Client fallback resolver (phase 1, built in goat-cli)

An in-process resolver module (`crates/goat-cli/src/names.rs`), **not** a
hosts-shim and not a local DNS daemon (rejected in
[ADR 1082](../adr/1082-name-fallback-resolver-signed-snapshots.md): a
hosts-file writer needs root, mutates global state for every process on the
box, collides with NetBird's pushed-nameserver management, and cannot carry
an honesty label).

**Resolution order — live wins, always:**

1. **Live DNS** — the OS resolver (which includes the NetBird-pushed mesh
   nameserver), bounded timeout. If it answers, that answer is used and the
   snapshot is never consulted. The fallback **never shadows live DNS**.
2. *(phase 3 slot — valid self-claim for the peer's own name; committed
   follow-on, see §7)*
3. **Snapshot** — exact-name lookup in the newest validly-signed cached
   snapshot, subject to staleness grading (§5). Signature verified at load;
   bad signature or expired TTL → refuse (fall through to the original DNS
   error, with a line naming why the snapshot was refused).
4. **Observed records** — the noncanonical tier (§4.2): gap-fill for
   names absent from the snapshot, fresher-wins on conflict; every answer
   labeled noncanonical (ad hoc) + logged as codification debt.

### 4.1 Local store — flat signed files, one owner, shared readers

There is deliberately **no database and no store-guarding service**. The
canonical tier is two flat files, written by whichever component refreshes
and **signature-verified on every read** — the signature makes the store
tamper-evident, so flat files carry the full trust story:

```
name-snapshot.json        # the artifact, byte-for-byte as signed
name-snapshot.json.sig    # detached ECDSA P-256 signature
```

**Single-store contract (anti-double-implementation rule).** The same
artifact must never be maintained twice on one device:

- **Daemon present** (goat-clientd, the device-wide consumer — see §4.3):
  the daemon is the **sole refresher** and owns the store at the system
  state dir (`/var/lib/goat-client/names/` on Linux; per-OS equivalent).
- **goat-cli** resolves live → daemon store (when present) → its own
  per-context cache `~/.goat/contexts/<name>/name-snapshot.json{,.sig}`.
  The per-context cache exists ONLY for daemon-less operator laptops;
  goat-cli never writes the daemon store.
- Both readers verify against the same per-goatnet CA root, apply the same
  precedence + grading rules, and prefer the highest validly-signed serial
  across whichever stores exist. Same format, same semantics, one
  implementation per language — no divergent copies.

**Implemented (goat-cli 0.4.24, `crates/goat-cli/src/names.rs`).** Store
discovery is files-only (no IPC): goat-cli probes, in order, the system
install path (macOS `/Library/Application Support/goat-client/names/`,
Linux `/var/lib/goat-client/names/`) then the user-run-daemon path
(`os.UserConfigDir()/goat-client/names` as the Go daemon derives it —
macOS `~/Library/Application Support/goat-client/names/`, Linux
`~/.config/goat-client/names/`); `GOAT_DAEMON_NAMES_DIR` overrides
(empty = opt out). Every candidate is signature-verified at read; the
highest validly-signed serial wins, daemon store on ties. The observed
tier merges the same way (freshest record wins; goat-cli still writes
only its own context store). When the daemon store holds a fresh valid
snapshot, goat-cli skips its own `get.<site>` refresh — one refresher
per device. `goat status` labels a daemon-sourced verdict "daemon
store". Store readability on system installs comes from the daemon side
(names dir 0755; verify-at-read is the integrity story, not
permissions).

### 4.2 Noncanonical observed-records tier (built in goat-cli; daemon side with §4.3)

The signed snapshot carries only **registry-declared** names. Live DNS
legitimately holds more: peer records auto-synced from the NetBird API
(`sync-peers.sh`), and — the edge-case that must not become a loop — an
operator's out-of-band hotfix during an incident that live DNS served but
the repo never captured before DNS died. Those must not vanish in fallback.

Design: a second persisted file, `observed-names.json`, in the same store —
every **successful live resolution** is recorded as
`{name, ip, observed_at}` (bounded size, LRU by `observed_at`). Resolution
consults it **after** the signed snapshot, under two rules:

- **Gap-fill:** a name absent from the snapshot (peer records, never-codified
  ad-hoc records) resolves from the observed cache.
- **Conflict:** a name present in both resolves from the observed cache only
  when `observed_at > snapshot.generated_at` — the operator-hotfix case,
  where the observation is the newer truth.

**Honesty rules are stricter than the snapshot's:** every answer from this
tier is labeled **noncanonical (ad hoc)** with its observed-at age, logged
as a gap (the record exists live but not in the registry — a codification
debt signal), and rendered visibly flagged in every interface (goat-cli
output, TUI lenses, goat-client GUI) — never blended with canonical
records. Observed records are unsigned by construction; they were learned
over the WireGuard-trusted live channel at capture time, and the label is
what keeps that trust downgrade visible.

### 4.3 Device-wide consumer — goat-clientd (built: goat-maintainer/goat-client#83)

goat-cli covers operator tooling only; browsers and other apps on a device
still lose names when mesh DNS dies. The device-wide consumer is
**goat-clientd** (repo `goat-maintainer/goat-client`, PR #83, `internal/names` +
`docs/names-fallback.md` there): it consumes the same artifact (hourly
`get.<site>` refresh, monotonic-serial acceptance, verify-at-read against
the same trust roots as enrollment bundles), owns the shared store (§4.1)
at `<bundle-dir>/names/`, maintains the observed tier (§4.2), and answers
at the DNS layer through a loopback UDP forwarder (`GOAT_NAMES_LISTEN`,
default `127.0.0.1:53530`) — live-upstream-first, snapshot, observed,
SERVFAIL. Fallback state (grade/serial/age/observed count/served
counters) surfaces in `getStatus` → `goat-client status` (bold FALLBACK
IN USE warning) and a flagged GUI banner. Validated e2e against the
production ZAGROS artifact (dig answered from the signed snapshot,
NOERROR).

**OS wiring (built, goat-client v0.3.4): forwarder-in-front.** When the
daemon applies the wg-cp0 host-DNS config, it points the OS's mesh-zone
resolution at the forwarder (`127.0.0.1:53530`) instead of the mesh
nameservers directly — chosen over "secondary resolver" because per-OS
secondary semantics are inconsistent (macOS races servers, resolved
round-robins). The forwarder stays live-first (never shadowed), so
browsers get the outage protection with zero manual steps. Per-OS: macOS
scutil `ServerPort`; Linux systemd-resolved `SetDNSEx` (≥ 246); Windows
NRPT cannot carry a port → stays on direct nameservers. Fail open to
live: the forwarder is supervised (crash → rebind with backoff);
unhealthy → the daemon re-applies direct mesh nameservers, recovery →
re-fronts. Knob `--names-fronting` (default on for the wg-cp0 adapter
path, i.e. wg-cp0-only + combined; netbird-only's embedded netbird DNS
path is untouched). Details: `docs/names-fallback.md` in that repo.

Wired at every place goat-cli turns a mesh hostname into a connection: the
`goat status` / `goat doctor` TCP probes, the Zenoh router endpoint
(`connect/endpoints` host substituted with the resolved IP — safe because
the fabric's TLS config sets `verify_name_on_connect: false` and trust is
the mTLS chain, not the name), and the portal HTTP client (a reqwest
resolve-override applied only when fallback resolution was needed).

**goat-clientd note:** the daemon (separate repo, Go) reaches its endpoints
from bundle-carried config and today embeds no name-plane dependency at
steady state; adopting the same resolver library shape there is a named
follow-on, not phase 1.

## 5. Graded staleness, fail-closed honesty

Every fallback answer is labeled with **source + serial + age** — "names as
of serial N, age M". Grading:

| Grade | Condition | Behavior |
|---|---|---|
| `live` | live DNS answered | normal; no snapshot involvement |
| `snapshot-fresh` | fallback, age < 7 d | resolve; label |
| `snapshot-aging` | fallback, 7 d ≤ age < TTL | resolve; label + explicit staleness warning |
| `expired` | age ≥ declared TTL | **refuse** — surface the DNS failure + "snapshot expired (serial N, age M) — refresh via get.<site> or operator re-sign" |
| `unverified` | signature bad / missing | **refuse** — never serve unverifiable names |

Surfaced in `goat status` (a `names` verdict line alongside ztol/portal/cert)
and `goat doctor` (per-probe annotation when a probe resolved via snapshot,
plus remediation hints). Silence is never an option: an operator on fallback
names always sees it.

Age is measured against the local clock, so grading inherits the node's UTC
confidence ([`universal-ntp-hygiene.md`](universal-ntp-hygiene.md), ADR 1037)
— same caveat as cert validity, accepted deliberately: a node whose clock is
wrong enough to mis-grade a 30-day TTL has louder problems first.

**Replay bound:** a compromised courier or download tier can serve only an
*old-valid* snapshot (it cannot forge a mapping — signature — and cannot
roll a client back — monotonic serial acceptance). Exposure is bounded by
the declared TTL and shrinks with regeneration cadence.

## 6. Element 2 — courier propagation (phase 2, designed-not-built)

Any peer may hand any peer a newer snapshot; the signature makes couriers
untrusted-but-useful. Gossip the **signed artifact, never records**. Shape:
`goat names pull <peer>` / `goat names offer` over the mesh, plus the
sneakernet path (§3.3) that already works. Peers serve their cached
snapshot+sig verbatim; recipients apply the same verify + monotonic-serial +
TTL rules as any other source. A compromised peer can replay old-valid
snapshots (TTL-bounded) but can never forge a mapping — recognition of the
zone authority's signature, never authority granted to the courier
([`federated-trust-model.md`](federated-trust-model.md)).

**Adoption gate:** evidence that the phase-1 refresh channel leaves real
clients stale in real outages — e.g. staleness grades observed at
`snapshot-aging` or worse during an actual name-plane incident, or a
denied-edge deployment where clients routinely cannot reach `get.<site>.*`
between outages. Until then, phase 2 is complexity without a demonstrated
consumer.

## 7. Element 3 — signed single-owner self-claims (committed follow-on)

Each peer may assert **only its own** name→IP binding, signed under its own
leaf cert (the URI-SAN identity it already holds). The safe CRDT-adjacent
layer: single-owner registers with no cross-writer conflict by construction.
Handles runtime IP changes (re-enrollment, DHCP churn on bearer networks)
that snapshots are too slow for. Verification chains the claim's leaf to the
same per-goatnet CA root; the claimed name must equal the cert identity's own
peer name — a peer can never claim a *service* name, and self-claims **never
override snapshot service records**; they slot between live DNS and the
snapshot in the resolution order (§4), for peer names only.

**Status (2026-07-15): pulled forward from adoption-gated backlog to
committed follow-on** by operator decision — peer names failing during
fallback (they are live-only via `sync-peers.sh`, never in the registry
snapshot) is a real coverage hole, not a hypothetical. The observed-records
tier (§4.2) is the interim answer (peer names gap-fill from observations,
labeled noncanonical); self-claims are the signed replacement for that
tier's peer-name slice. Sequencing: with the goat-clientd device-wide
consumer (§4.3).

**Design settled 2026-07-17 (ADR 1082 Amendment 2); build follows.** The
three decisions:

### 7.1 Offline verification — the snapshot carries the name→anchor bindings

The Zenoh leaf cert identifies its holder (`org=<anchor>` URI-SAN / slot CN
= anchor), but nothing in the cert says which *DNS peer name* that anchor
owns — and during an outage the verifier can consult no registry. Solving
this with claim-side attestation would rebuild registry trust inside the
claim. Instead the **already-signed, already-distributed snapshot** gains an
additive field:

```json
"peer_bindings": [ {"name": "<peer name>", "anchor": "<uuid>"} , … ]
```

Registry-derived (the same authority that declares service records declares
which anchor owns which peer name), slow-changing, and riding the existing
serial + signature + TTL + distribution machinery — no new trust root, no
new channel, v1-format-compatible (readers that don't know the field ignore
it). A claim for `name` verifies only when the *cached verified snapshot*
binds `name` to the claim's anchor. Consequences, by construction:

- a peer can claim **only its own** name (binding mismatch → refused);
- a claim can **never override a service record** (service names carry no
  binding, so no claim for them can ever verify);
- verification is **fully offline** — leaf chain → the trusted CA root,
  binding → cached snapshot, freshness → claim TTL. Fail-closed like
  everything else in this design.

### 7.2 Claim artifact

```json
{
  "format": "goat-name-claim-v1",
  "name": "<the peer name being claimed>",
  "ip": "<current mesh IP>",
  "anchor": "<uuid>",
  "issued_at_unix": 1784200000,
  "ttl_seconds": 3600
}
```

Envelope = the canonical artifact bytes + ECDSA signature by the **leaf**
key + the leaf cert (chain to the embedded root). Short TTL (default 1 h)
because claims exist precisely for fast-moving IPs; an expired claim is
refused, and the observed tier (§4.2) remains the labeled last resort.
Re-issue = publish a fresher `issued_at_unix`; freshest valid claim per
name wins (single-owner register — no cross-writer conflict by
construction).

### 7.3 Publication + distribution — fabric while healthy, store while broken

Decision: **(a) publish on the Zenoh fabric, collected into the existing
`get.<site>` tier** — the same consume-while-healthy / serve-while-broken
philosophy as the snapshot itself.

1. **Publish:** the claim holder publishes its envelope to
   `ops/names/claim/<anchor>` at connect, on IP change, and at TTL/2.
   goat-cli publishes for operator laptops (it owns the zenoh session);
   a device whose daemon has no fabric identity is covered by whichever
   fabric-connected component holds its leaf.
2. **Collect:** a passive collector on the mgmt host (audit-subscriber
   pattern — reveal-only, brokerless-safe, in no enforcement path)
   subscribes `ops/names/claim/**` and maintains `claims.json` served
   beside the snapshot at `https://get.<site>.<zone>/claims.json`. The
   server is untrusted transport: every claim carries its own leaf
   signature, and clients verify at read — a compromised collector can
   drop claims (denial, bounded by the observed tier) but never forge one.
3. **Cache + serve:** the existing hourly client refresh fetches
   `claims.json` opportunistically into the shared store (§4.1, third
   file); the forwarder and goat-cli serve valid claims between live DNS
   and the snapshot, labeled `self-claim, age M`. The observed tier is
   demoted to gap-fill-only for names that carry no valid claim.

Rejected channels: **(b) direct peer claim-query** — needs the claimant
reachable at exactly the moment names are broken, and leaves no cached
artifact for the outage window; **(c) piggyback the mesh-management peer
list** — NetBird's peer list is alive precisely when the name plane is
alive (mgmt up ⇒ netbird DNS up); its failure couples to the failure being
survived, and it bypasses the leaf-signature trust story.

### 7.4 Build order (all built 2026-07-17)

1. **Generator + registry export (built).** Optional
   `ops/dns/<site>/peer-bindings.json` (registry-produced: name→anchor)
   folds a sorted, duplicate-refused `peer_bindings` table into the
   signed snapshot; sites without one regenerate byte-identically.
   ZAGROS serial 2 carries the first binding.
2. **goat-cli + goat-clientd store/verify/serve (built).** `claims.json`
   third store file; `verify_claim` (Rust `crates/goat-cli/src/names.rs`,
   Go `internal/names/claims.go`) — leaf chains to the embedded root +
   validity window + artifact sig under the leaf key (Ed25519 for the
   fleet's zenoh leafs; P-256 accepted) + leaf identity == claimed anchor
   + snapshot binding authorizes the name + TTL. The claim tier slots
   between live DNS and the snapshot; observed demoted by precedence.
   Labeled in `goat status`/`doctor`/pubsub + the forwarder log.
3. **Publisher + collector (built + deployed).** `goat names
   publish-claim` signs `{name, ip, anchor, issued_at, ttl}` with the
   context leaf and publishes to `ops/names/claim/<anchor>`; `goat names
   collect` subscribes `ops/names/claim/**` and maintains `claims.json`
   for the get tier (freshest-per-name, atomic). Router ACL gains
   `peer-w-name-claim` (broad ingress-allow on the claim subtree — safe
   because forgery is inert under verify-at-read; the collector is
   untrusted transport). The collector runs as a persistent mgmt-host
   service (`ops/ansible/roles/names-collector`, systemd unit
   `goat-names-collector.service`) under a dedicated infra leaf
   (`CN=names-collector-trial-peer`, clientAuth — covered by the router's
   existing `trial-peer-r-all` read rule, so no extra ACL), writing
   `claims.json` into the goat-get www dir. ZAGROS-live 2026-07-17.
4. **Gate (live-fired on ZAGROS 2026-07-17).** With mesh DNS down, the
   operator's own peer name resolves from a **signed** claim labeled
   `self-claim, age M`; the observed tier no longer serves it.

**Program complete.** All four steps built, deployed, and live-fired; no
name-resilience engineering work remains. Courier propagation (§6) stays
adoption-gated.

## 8. Operational summary

| Piece | Where |
|---|---|
| Snapshot + sig (committed) | `ops/dns/<site>/name-snapshot.json{,.sig}` |
| Generator | `ops/dns/generate-mesh-hosts.py` (same registry, same `--check`) |
| Signer | `ops/enrollment/cmd/name-snapshot-sign/` (reuses `AgeSigner`) |
| CI gates | `dns-registry-sync` job (drift) + `ops/ci/check-name-snapshot-sig.sh` (signature) |
| Serving | `goat-get` role → `https://get.<site>.<zone>/name-snapshot.json{,.sig}` |
| Daemon store (device SoT) | `<bundle-dir>/names/` (`/var/lib/goat-client/names/` on packaged Linux, `/Library/Application Support/goat-client/names/` on packaged macOS, `os.UserConfigDir()/goat-client/names` user-run); dir 0755, verify-at-read |
| Forwarder + OS wiring | goat-clientd ≥ v0.3.4: loopback forwarder (`GOAT_NAMES_LISTEN`, default `127.0.0.1:53530`) fronting OS mesh-zone DNS; `--names-fronting` knob; `mesh_dns_*` config keys |
| Client cache (daemon-less) | `~/.goat/contexts/<name>/name-snapshot.json{,.sig}` |
| Resolver | `crates/goat-cli/src/names.rs` (≥ 0.4.24: daemon store first) + wiring in `status`/`doctor`/zenoh/portal paths |
| Self-claims (§7) | `peer_bindings` in the snapshot (from `ops/dns/<site>/peer-bindings.json`); `claims.json` third store file; `goat names publish-claim` / `goat names collect` / `goat names resolve`; router ACL `peer-w-name-claim`; persistent collector service `ops/ansible/roles/names-collector` (infra leaf `CN=names-collector-trial-peer`) |
| Enablement | `name_snapshot_enabled: true` in site group_vars (ZAGROS on) |

## 9. Adoption — who does what (as of goat-client v0.3.4 / goat-cli 0.4.24)

Three tiers, from zero-effort to install-time. The end state — "install the
client, get device-wide protection" — is gated on three named items at the
bottom of this section.

### 9.1 Operator with goat-cli: nothing to do

`goat` self-nudges updates from `get.<site>` (`version.json`); 0.4.24 reads
the daemon's shared store automatically when one exists (highest
validly-signed serial wins; `goat status` labels the source "daemon store")
and falls back to its own per-context cache when not. Protection covers goat
commands only — status/doctor probes, Zenoh endpoints, the portal client.

Since 0.4.25, `goat status` also renders the whole posture directly: the
`names` line (live vs snapshot vs observed), a `device` line (daemon store →
forwarder answering → OS fronted; the three tiers of §9.2, with the broken
tier as the only red), and a `pqc` line (WireGuard Rosenpass state read from
the local netbird daemon; OFF renders neutral `[–]`, never a failed exit).

### 9.2 Device-wide (browsers + every app): goat-clientd + two config lines

Requirements on the device:

1. **goat-clientd ≥ v0.3.4** running as a system service (root — utun
   creation + scutil/resolved writes need it) with a bundle for the site.
2. **Mesh DNS config** — enrollment bundles do not carry nameservers yet, so
   the daemon's config file must say where live mesh DNS is:

   ```toml
   mesh_dns_servers = "<mesh-dns-ip:port>[,<mesh-dns-ip:port>]"
   mesh_dns_match_domains = "<mesh zone, e.g. netbird.zagros.example>"
   ```

   List every live upstream the site runs (the same set the netbird
   nameserver group carries), not just one.

With those, the daemon points the OS's mesh-zone resolution at its forwarder
(live-first, snapshot fallback) — proven live on ZAGROS 2026-07-16: a browser
loaded the portal during a mesh-dns stop with zero manual steps
(transcript: PR #1664). Fail-open: forwarder trouble reverts the OS to
direct mesh DNS; `--names-fronting=false` turns the layer off.

Current caveats, honestly stated:

| Caveat | Status |
|---|---|
| Windows | NRPT cannot express a resolver port — device stays on direct mesh DNS (goat-cli tier still works) |
| netbird-only mode | the embedded netbird DNS path is untouched — fronting applies to the wg-cp0 host-DNS path (wg-cp0-only + combined). Decision for netbird-only deferred. |
| Packaged installers | deb/rpm/pkg service files pass flags the daemon does not define (`--bundle-dir`/`--ipc-socket`/`--log-file`) — packaged daemons fail to start; fix owned by client-distribution (ADR 1083). Until then: operator-assisted install. |

### 9.3 Per goatnet: one group_var at standup

`name_snapshot_enabled: true` in the site's group_vars → run the generator →
operator signs with that site's CA → deploy `goat-get`.
Fold into the turnkey-standup sequence for new sites (`goatnet-turnkey-standup.md`).
Note the name plane itself may be multi-instance (a single instance stop is
then not a client-visible outage); the snapshot tier is the answer for *site-wide* name-plane loss.

### 9.4 What closes the gap to "just install the client"

1. **Bundle-schema rev carries DNS config** (nameservers + zones minted into
   the enrollment bundle) — removes the config-lines step. Noted in
   `tunnel.Config.DNSServers` as the successor to `mesh_dns_*`.
2. **client-distribution (ADR 1083)** ships working packaged installers and
   fixes the service-file flag drift.
3. **netbird-only fronting decision** — whether/how to front the embedded
   netbird DNS path on devices without a wg-cp0 leg.

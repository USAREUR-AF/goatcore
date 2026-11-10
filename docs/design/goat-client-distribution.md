# goat-client distribution + L1 delivery — canonical roadmap

Canonical detail for [ADR 1083](../adr/1083-goat-client-single-client-and-distribution.md).
This is the **authoritative, stable plan** — edit here rather than re-deriving it.

**One-line thesis:** goat-client is the single netbird client; storm-resilience
L1 and every future fork-level client fix ship as a goat-client update; the
chicken-and-egg of getting goat-client to a not-yet-joined user is solved by a
signed release, publicly hosted from the cloud on Cloudflare R2 at `get.goat.example`
(`terraform-prod/`, reusing the live image-mirror pattern), independent of any
goatnet's mgmt host or mesh.

## The three things that run netbird, named exactly

| name | what it is | role in the target state |
|---|---|---|
| stock netbird | downloaded from `pkgs.netbird.io` | **migrate away from** — no L1, not our build |
| hand-built patched `netbird` | onboarding Phase 3.6 `go build ./client` → `/usr/local/bin/netbird` | **retire** — legacy pre-embed cruft |
| **goat-client / goat-clientd** | Go client that **embeds** netbird (`client/embed`) from our fork | **the one client** |

`goat-cli` (Rust TUI) is the control surface over goat-clientd (IPC) — it never
runs the VPN itself.

## Already live (the server-side floor — no work)

- **L2 negative cache** — sheds server load, works on any client (ADR 1081 L2, ZAGROS-live).
- **L3 rathole hardening** — mgmt-host tunnel reconnect, heartbeat 20 s / retry 5 s (ZAGROS-live).

## Track A — L1 reaches every goat-client user

1. **[linchpin] Rebase the L1 commits onto the goat fork lineage.** L1
   (`backoff` + `breaker`) currently sits on `USAREUR-AF/netbird` branch
   `scratch/l1-retry`, **diverged** from `c5841359c` (the goat CA-embed lineage
   goat-client pins). Rebase the two commits onto that lineage → a new fork
   commit. Nothing consumes L1 until this exists on a consumable ref.
2. **goat-client `go.mod` bump + multi-OS rebuild.** Point goat-client's
   `replace => USAREUR-AF/netbird` at the new commit; rebuild the release binaries.
   L1 now ships as a normal goat-client update.
3. **L1 decay bench (measured gate).** `tc-netem` induced loss on nova-twin with the
   L1-carrying goat-client: aggregate signal traffic must **decay** to <5% of
   storm baseline within 2 min of streams dying; reconvergence ≤3 min after loss
   clears. Closes the open loop from M-086
   (L2 alone raised link load against unpatched clients — L1 is what makes it decay).

## Track B — goat-client is the one client, publicly distributed

4. **Mature goat-client** from "operator-class first-contact dogfood" (its own
   maturity caveat) toward non-engineer distribution: signed release artifacts
   per OS, a plain install path, and the connect-verify UX already designed in
   [goat-client.md](goat-client.md).
5. **Public distribution — a fully cloud-hosted, signed release page on
   Cloudflare R2.** The download is decoupled from onboarding entirely: a plain,
   public, cloud-served page for the goat-client release, reachable pre-join and
   independent of any goatnet's mgmt host or mesh. Integrity comes from the artifacts
   being **signed**, not from gating who can reach the page.
   - **Home = `terraform-prod/` (the `goat.example` repo), on Cloudflare R2.** R2 is
     already live on the prod Cloudflare account — the goatnet **container-image
     mirror** runs on it (`terraform-prod/r2-mirror.tf`,
     Block 68C). The download bucket reuses that exact pattern in the same file's
     neighbourhood (`terraform-prod/goat-download.tf`):
     a `cloudflare_r2_bucket` applied with
     `aws-vault exec usareur-af --no-session -- terraform -chdir=terraform-prod apply`.
     (This supersedes an earlier plan to host it on AWS S3 in
     `collab-services` — once R2 was chosen, the natural home is where
     R2 already lives.)
   - **Topology = R2 object store + Cloudflare edge (no mgmt host, no rathole, no
     mesh, no AWS, no CloudFront).** R2 is the right fit for download-heavy
     binaries: **free egress**, HTTPS end-to-end via the Cloudflare edge cert,
     private origin, WAF/rate-limiting at the edge — no S3-website HTTP-origin
     wrinkle and no CloudFront/ACM. Public name **`get.goat.example`**, bound to the
     bucket as a Cloudflare **custom domain** (connected in the R2 dashboard —
     Cloudflare auto-provisions the cert + proxied record; kept a dashboard step
     to match the image mirror, whose author found the provider's
     `cloudflare_r2_custom_domain` incomplete). The bucket holds the signed
     binaries + `SHA256SUMS` + `.cosign-bundle` + a static `index.html`, mirrored
     from the GitHub release (GitHub stays source of record; the page is ours).
     Same framing the image mirror already relies on: the artifacts are public by
     design and integrity is the **detached signature**, not origin auth.
   - **No onboarding coupling.** There is no one-time token, no presigned URL, no
     portal mint in the byte path — the page is openly reachable because every
     artifact is cosign-signed and checksummed, so exposure is not the threat that
     the earlier one-time-link design guarded against. The SHA256SUMS + cosign
     bundles sit next to the binaries; the verify key is published out-of-band
     (docs / onboarding text), never fetched from the same page.
   - **Invite links unchanged for now.** The onboarding invite keeps its current
     mgmt-host-origin `onboard-intake-collector` path (public DNS → relay apex →
     rathole → mgmt-host Traefik → collector). Hosting invite links off the mgmt host is a
     *separate, later* track; this change does not touch it.
   - **Decisions — settled.**
     - **Public name: `get.goat.example`** (one-level under `goat.example`, same shape as
       `collab.goat.example`), bound as an R2 custom domain.
     - **Storage/edge: Cloudflare R2 + Cloudflare edge** (free egress, HTTPS
       end-to-end, private origin). Chosen over AWS S3+CloudFront (heavier, egress
       cost) and over Cloudflare-proxied-S3-website (HTTP-origin TLS wrinkle);
       CloudFront+OAC stays the fallback only if we later need AWS-side per-object
       access controls.
     - **Signing key: the existing goat-client release-pipeline cosign key.** The
       artifacts are already signed with it (v0.3.4 ships `.cosign-bundle`s); the
       public page just serves those already-signed files, so a separate
       distribution key would add key management for no added provenance.
     - **Mirror: manual sync from the GitHub release on each cut** (`rclone copy`
       or `aws s3 sync … --endpoint-url <account>.r2.cloudflarestorage.com`) for
       v1; a release webhook is a later nicety, not a launch gate.
6. **Retire the netbird-build path.** Remove onboarding Phase 3.6 (`go build
   ./client` + repoint `/usr/local/bin/netbird`); migration guidance points
   stock-netbird users at goat-client.

## Track C — shrink the peer set (bounds the storm for the stock-client majority)

7. **Lazy connections bench + adoption** (ADR 1081 item 5). Defer P2P setup until
   a pair has traffic → fewer signaling-active pairs. Bench on nova-twin:
   pair-count + signal-rate before/after; first-packet latency on fabric paths
   within budget; rosenpass-permissive + ACL-group interplay clean over a soak.
8. **ACL scoping** (separate track/session) — shrinks the attempted peer set at
   the ACL layer; compounds with lazy connections. Coordinate, don't collide.

## Sequencing + drivability

| # | work | repo | drivable now | needs |
|---|---|---|---|---|
| A1 | L1 → goat fork lineage | `USAREUR-AF/netbird` | yes (rebase) | push access to `USAREUR-AF/netbird` |
| A2 | goat-client go.mod bump + rebuild | `goat-client` | yes (PR) | goat-client release pipeline |
| A3 | ~~L1 decay bench~~ | — | **dropped** | confirmation-only of an already-shipped, code-proven fix; not worth the nova-mesh disruption |
| B4 | mature goat-client | `goat-client` | partial | product decisions |
| B5 | public distribution (R2 release page, `get.goat.example`) | `terraform-prod/` | **LIVE + complete** (serving v0.3.7; re-probed 2026-08-13) | — (root `/` handled by the `root-index` Redirect Rule) |
| B6 | retire Phase 3.6 (→ goat-client bundle onboarding) | goat | **built** — real-Mac validation pending | operator-laptop-onboarding.yml rewritten to the bundle model (combined mode; setup-key minted at issuance via `bundle-create --inner-mesh-*`); needs one live Mac run to close |
| B7 | **no update path** — the installed base cannot be moved | `goat-client` | **not built** | goat-client carries no auto-update or self-update code. Every fix — L1 included, which is this ADR's whole delivery thesis — needs a human to re-download and re-install. Names the gap; does not schedule it |
| C7 | lazy connections bench | — | partial | nova-twin |
| C8 | ACL scoping | — | other session | coordinate |

## Status

Per-track; each measured/live gate logged to `docs/measurements/`. B5 (public
distribution) is **LIVE**: a fully cloud-hosted, signed release page on
**Cloudflare R2** at `get.goat.example`, bucket defined in
`terraform-prod/goat-download.tf` (applied)
reusing the live image-mirror pattern, independent of any mgmt host or mesh and decoupled
from onboarding. The `get.goat.example` custom domain is bound (Cloudflare-managed cert)
and the v0.3.4 artifacts (44 signed files + `index.html`) are synced; a download
from it hash-matches `SHA256SUMS` end-to-end. Signing = the existing goat-client
release-pipeline cosign key. **Root path:** R2 is object storage, not a website
host — it serves objects by exact key with no index-document behaviour, so bare
`/` doesn't auto-serve `index.html`. Resolved with a Cloudflare **Redirect Rule**
(`root-index`: URI Path `/` → 301 → `/index.html`), which is the proportionate fix
for a static file surface. A Worker fronting R2 would serve `/` natively with no
hop, but that puts a compute layer in the path of every binary download for no
functional gain here — reserve the Worker pattern for buckets that need routing
logic (e.g. the image-mirror's tier fallback). Per-release refresh = re-sync the
new assets to the bucket. Invite-link hosting is untouched (a separate, later
track). Implementation tracking in in-flight.md.

B6 (retire Phase 3.6) is **built, validation pending**:
the operator-laptop onboarding playbook (`operator-laptop-onboarding.yml`,
kept with the deployment that runs it) is rewritten from the
three-piece legacy flow (wg-cp0-bundle-apply + wg-quick + hand-built
patched netbird + onboarding-time setup-key) to the goat-client model —
install the signed release from `get.goat.example` (SHA256SUMS-verified
.dmg/.pkg), import one v0.2 bundle carrying `inner_mesh_setup`, run
`combined` mode. Setup-keys are now minted at bundle-issuance time
(operator-bundle-issuance.md §"Inner-mesh setup");
`bundle-extract` grew `has_inner_mesh` / `inner_mesh_mgmt_url` fields so
the playbook fail-closes on pre-v0.2 bundles. Two v0.3.4/v0.3.5
packaging defects surfaced during the port (source-verified in the
goat-client repo): the macOS LaunchDaemon plist and the desktop
deb/rpm systemd units pass flag names the daemon doesn't register
(`--bundle-dir`/`--ipc-socket`/`--log-file` vs the real
`-bundle`/`-socket`) → the system daemon crash-loops as shipped; and no
trust-roots PEM is shipped or seeded, which is fatal at daemon start.
The playbook repairs both on macOS (self-disarming once a fixed release
ships). **Both durable fixes have since landed in goat-client packaging**
— the unit/plist flags in #90 (deb/rpm/dmg) and the *same defect missed in
the MSI* in #98/#99, and every package now seeds
`/etc/goat-client/trust-roots.pem` (`packaging/*/nfpm.yaml`,
`packaging/dmg/build-dmg.sh`, `packaging/msi/goat-client.wxs`) — so the
playbook's repairs are disarmed against ≥0.3.6 artifacts. That the same
defect shipped twice, and was caught by a source sweep rather than by a
test, is the argument for a package-install gate in goat-client CI. Be
precise about what was missing: `package-deb-headless-smoke` **did**
install its package — but only the *headless* deb, whose unit was the
one that wasn't broken, and its check tolerated a dead unit
(`systemctl status || true`) on a no-trust-roots-in-CI rationale that
#90 itself retired. The desktop `.deb`/`.rpm`/`.dmg`/`.msi` were built,
inspected and shipped without ever being installed. goat-client PR #100
closes it in two parts: a static flag-contract check over all six
packaging surfaces (verified to fail against both historical defects,
and running on PRs), plus a real install-and-assert-active on the
desktop deb. The `.rpm`, `.dmg` and `.msi` still have no install gate.
B6 closes when a fresh Mac run reaches the mesh end-to-end.

# goat-cli mesh download surface (`get.<site>.*`)

| Field | Value |
|---|---|
| Status | Built 2026-06-04 (role + deploy playbook authored; deploy operator-fired). |
| Scope | A mesh-internal HTTP surface that re-serves the signed goat-cli release binaries so on-mesh vendors can install `goat` without reaching the private GitHub release page. |
| Related | `goat-cli.md`, [ADR 1019](../adr/1019-goat-cli-unified-operator-binary.md), `operator-onboard-utilities.md`, `nova-public-posture.md`, [`docs/operations/goat-cli-install.md`](../operations/goat-cli-install.md) |

## Problem

The operator-onboard bundle hands a vendor a signed goat-cli **profile**
(`<handle>-goat-cli.cbor`) plus `goat profile init` / `goat doctor`
instructions — but no way to get the **binary**. The only published
binaries live in GitHub Releases on the **private** `usareur-af/goatcore`
repo, which external vendors cannot download from (and which we do not
link in vendor-facing material). So the CLI path dead-ends at "install
goat" for anyone outside the repo.

## Decision

Re-serve the release binaries from a **mesh-internal** download surface at
`get.<site>.<zone>`, gated by the same `mesh-only-allowlist@file` Traefik
middleware as the other mesh surfaces. A vendor enrolls NetBird from
their bundle first (they're on the mesh), then `curl`s the binary for
their platform. No public exposure; no private-repo access required by the
vendor; the binaries keep their release provenance.

**Per-goatnet, not nova-specific.** The hostname is
`get.{{ site_id }}.{{ site_dns_zone }}`, so every goatnet that deploys the
role gets its own surface — `get.nova.netbird.nova.example` on nova-a,
`get.zagros.netbird.zagros.example` on ZAGROS, and so on. `get.nova.*` is one
instance + the role default, not the surface's identity.

### Shape

`goat-get` Ansible role ([`ops/ansible/roles/goat-get/`](../../ops/ansible/roles/goat-get/)),
a direct mirror of the `brief` role:

- **nginx:alpine** serving `/opt/goat-get/www` (read-only bind) — the
  `goat-{linux,darwin}-{amd64,arm64}` + `goat-windows-amd64.exe` binaries,
  each with its sibling `<asset>.sha256` (load-bearing: `goat update`
  fetches `<asset>.sha256` from this surface to verify the binary before
  replacing it — omit them and `goat update` 404s even though manual
  install still works), plus the aggregate `SHA256SUMS` + a landing
  `index.html`. nginx's default `application/octet-stream` type makes the
  extensionless binaries download cleanly.
- **CA roots** at `/ca-roots/` (`goat_get_ca_roots_enabled`, **default on**
  — unlike the opt-in `sdk`/`kmod`/`octo-pod` payloads below, this is the
  other half of the binary that already ships here). goat-cli compiles in no
  goatnet CA (F-406),
  so a download with no obtainable root is a tool that cannot install a
  profile. Every `ops/enrollment/public-keys/*.pem` is published — not just
  this goatnet's, because the point of the change is that one laptop works
  against several at once. Copied from the repo checkout rather than a GitHub
  release: they are public keys already in the tree, and a trust root should
  not depend on a release fetch that can 404. The index renders each root's
  CA-ID and fingerprint, derived by
  [`ops/enrollment/ca-root-fingerprint.py`](../../ops/enrollment/ca-root-fingerprint.py)
  so the page and `goat ca add` cannot disagree — a wrong fingerprint on a
  trust page is worse than none. The deploy fail-closes if the source
  directory holds no PEM, because an empty `ca-roots/` is indistinguishable
  from a working one until an operator is already stuck. Serving a root is not
  asserting it: `goat ca add` prints the fingerprint and says to confirm it out
  of band.
- **Platform chooser** (`index.html.j2`): rather than dumping every
  binary, the page is a tabbed picker that **auto-detects** the visitor's
  OS/arch (vanilla JS off `navigator`) and shows only that platform's
  one-time install snippet + a single direct-download link + the matching
  verify command (`shasum`/`sha256sum`/`Get-FileHash`). One click switches
  platforms; the full artifact list is tucked behind an "all artifacts"
  disclosure for the air-gapped/manual case. The tab set is generated from
  `goat_get_assets`, so it tracks exactly what's served — no per-release
  template edits. Windows carries the `goat client` not-on-Windows caveat
  inline.
- **Traefik file-provider** route (`get.<site>.<zone>`, `le` cert resolver,
  mesh-only middleware) — same pattern as brief/panoscope.
- **Binary provenance:** the role pulls the assets from the goat-cli
  GitHub release (`goat_get_release_tag`, default `goat-cli-v0.4.4` — adds
  the Phase G send/receive lifecycle commands (`goat schema`/`access`/`blob`/
  `replay`), slot-alias resolve/render in `doctor`/`pub`/`sub`, `pub
  --encoding`, and the `goat admin lan` triage kit; `goat-cli-v0.2.0` was the
  first release with `goat-windows-amd64.exe`) **on
  the host** at deploy time (host has internet egress; nothing multi-MB
  crosses the mesh via scp). The private-repo read uses a transient
  `GOAT_CLI_RELEASE_TOKEN` (no_log, never persisted on the host).
- **DNS:** `get.<site>.<zone>` → that goatnet's mesh IP in its
  `ops/dns/<goatnet>/mesh-hosts` (e.g.
  `ops/dns/nova/mesh-hosts` on
  nova-a, `ops/dns/zagros/mesh-hosts` on ZAGROS), generated from the
  `isitup_services` registry like the other per-site service records.

### macOS code-signing (launch-hang fix)

The served macOS binaries carry the Rust linker's `adhoc,linker-signed`
signature, which intermittently **hangs `goat` at launch** in the kernel's
Gatekeeper trust evaluation (pre-`main()`, un-killable) —
F-208. The
landing page's macOS install snippet therefore appends
`codesign --force --sign -` (a proper ad-hoc re-sign that clears the stall).
The durable end-state is a Developer-ID-signed + notarized release built by the
pipeline, after which `spctl -a` accepts and the re-sign becomes a no-op —
provisioning + the pipeline step are in
[`../operations/goat-cli-macos-signing.md`](../operations/goat-cli-macos-signing.md).

### Bump path

To serve a newer goat-cli: cut the `goat-cli-v<semver>` release (existing
`.github/workflows/goat-cli-release.yml`), bump `goat_get_release_tag` in
the role defaults (or `-e`), re-run [`goat-get-deploy.yml`](../../ops/ansible/goat-get-deploy.yml).
The role re-fetches the pinned assets; easy-rollback ledger per ADR 0859.

### Version-staleness check

goat-cli ships often, but participants install once and forget — so they end
up confused on an old binary whose flags/output have since moved. The download
surface doubles as the **"what version are we handing out"** authority so a
running `goat` can tell it's behind:

- The role renders `version.json` next to the binaries
  (`{ "version", "tag", "install_url" }`) from the **same**
  `goat_get_release_tag` that pins the served assets — so there is no second
  bump point to keep in sync. Bumping the tag and re-running the deploy updates
  the marker automatically.
- When `goat` **connects** (any command that opens a Zenoh session, plus a
  fresh check in `goat doctor`), it GETs `<get-base>/version.json`, compares to
  its own `CARGO_PKG_VERSION`, and prints a one-line nudge to stderr if behind
  (`⚠ goat 0.4.7 is out of date — 0.4.8 is available. Update: <install_url>`).
  `goat doctor` always renders the line — `[✓] goat <v> (latest)` or the
  out-of-date form.
- **Per-goatnet by construction.** The get-base is resolved from the active
  profile's own portal host (`portal.<site>.…` → `get.<site>.…`, matching the
  role's `get.<site>.<zone>` hostname), so each context checks against ITS
  goatnet's surface. Overridable with `GOAT_GET_ENDPOINT` /
  `<CONTEXT>_GET_ENDPOINT`.
- **Best-effort + quiet.** Implemented in `crates/goat-cli/src/version_check.rs`.
  Fail-silent (off-mesh / no surface / timeout → no output, never fails the
  command), TTY-only (never pollutes pipes or the `--json` contract), at most
  one nag per process, and cache-gated (`~/.goat/version-check.json`, 3 h TTL)
  so the warm path adds no latency — only a stale/cold cache does a single
  bounded (mesh-local, sub-2s) fetch, once per TTL. Silence entirely with
  `GOAT_NO_VERSION_CHECK=1` (or `GOAT_NO_FUN=1`).

The staleness check is a **nudge**; the operator acts on it with `goat
update` (below). The nudge itself never downloads or replaces anything — it
just reports.

### `goat update` — one-command self-update

`goat update` (goat-cli 0.4.26+, `crates/goat-cli/src/cmd/update.rs`) closes
the loop the nudge only reports: it updates the running binary from the SAME
per-goatnet surface `version_check` resolves.

- **Per-goatnet + no external host.** Resolves `get.<site>` from the active
  context's portal host (reusing `version_check::get_base_url`), reads
  `<get-base>/version.json`, and downloads `<get-base>/goat-<os>-<arch>` — so
  `goat update` on an ZAGROS context pulls from get.zagros, on nova from get.nova.
  Overridable with `GOAT_GET_ENDPOINT` / `<CONTEXT>_GET_ENDPOINT`.
- **Verify-before-replace.** Checks the download against the surface's own
  `<asset>.sha256` *before* touching the installed binary; a mismatch aborts
  with the old binary intact.
- **Atomic replace.** Downloads to a temp file next to `current_exe()`,
  verifies, then renames over it on the same filesystem — the running process
  keeps its inode; the new binary is in place for the next invocation. No
  partial-write window.
- **`--dry-run`** resolves + reports the plan without touching anything;
  **`--force`** re-installs the same version (repairs a corrupt binary).
  Windows: not a rename-over-self target — the command errors with the
  manual-install pointer (Unix operators — Linux + macOS — are the fleet).
- **Fail-closed + honest.** Any resolve/fetch/verify/replace error is a
  non-zero exit with a specific message; the installed binary is never left
  half-written.

### `goatnet.toml`: the goatnet's own facts

goat-cli holds no goatnet. Each goatnet publishes the facts the CLI needs at
`get.<site>/goatnet.toml`, and `goat context refresh` writes them into the
context's `profile.toml`, where `Profile::goatnet_endpoints()`
(`crates/goat-cli/src/profile.rs`) reads them.

| Key | Derived from (group_vars) |
|---|---|
| `goatnet` | `site_id` |
| `router_endpoint` | `portal_router_endpoint`, else `tls/zenoh.<site_id>.<site_dns_zone>:7447` |
| `netbird_mgmt_url` | `https://<public_mgmt_fqdn>` |
| `netbird_token_env` | `<SITE_ID>_NB_API_TOKEN` |
| `netbird_site_prefix` | `site_id` upper-cased |
| `netbird_rosenpass` | `rosenpass_enabled` |
| `ory_auth_url` | `https://auth.<site_id>.<site_dns_zone>` |
| `ory_ssh_host` | `mesh_ip_a` |
| `ory_ssh_user`, `mgmt_ssh_user` | `ansible_user` |
| `demo_service` | `goat_get_demo_service` (declared per goatnet; the service `goat demo` opens with no name given) |
| `i18n_feedback_email` | `goat_get_i18n_feedback_email` (declared per goatnet; where operators report translation errors) |

The goat-get role renders it (`roles/goat-get/defaults/main.yml`,
`goat_get_goatnet_facts`) with the same derivations as the onboard-runner role,
so a goatnet needs no new declaration for those; `demo_service` and
`i18n_feedback_email` are the two a goatnet declares, and an empty one is left
out of the file. The surface is mesh-only, like the rest
of `get.<site>`.

goat-cli carries no table of goatnets. Every command that acts on a goatnet
reads these keys from the context whose profile names that goatnet
(`Profile::goatnet_endpoints_for`), so `--env <goatnet>` never borrows another
goatnet's facts. Before any `goat admin` command, a context that names no
goatnet yet fetches its `goatnet.toml` once (4 s timeout); when the facts are
still missing, the command says to run `goat context refresh`.

## SDK assets (grant-crypto)

The same role optionally serves the **grant-crypto SDK** ([ADR 1106](../adr/1106-grant-crypto-sdk-and-endpoint-forward-secrecy.md))
at `get.<site>.<zone>/sdk/` — a parallel payload for on-mesh *developers*
(the goat-cli surface above is for *operators* installing the binary). Operator
runbook: [`docs/operations/grant-crypto-sdk-install.md`](../operations/grant-crypto-sdk-install.md).

- **Same everything but the subdir.** It re-uses the existing host, LE cert, DNS
  record, and mesh-only middleware — it's a subdir of the surface that already
  exists, so there is **no new SAN and no new DNS record**. Enabling it on a host
  already running goat-get is one flag (`goat_get_sdk_enabled`, default off) plus
  a redeploy. nginx serves the whole www tree, so `www/sdk/` just works with no
  nginx config change.
- **A separate asset set + version marker.** The goat-cli path is hard-coupled to
  the `goat-<os>-<arch>` binary-name parser and a goat-cli-specific `version.json`,
  so the SDK gets its **own** `goat_get_sdk_*` var set, its own `sdk/index.html`
  developer landing page, and its own `sdk/version.json` marker — none of the
  goat-cli machinery is touched. The served assets are the
  `grant-crypto-sdk-v*` release: one source tarball
  (`grant-crypto-sdk-<ver>-src.tar.gz`, the SDK workspace + shared conformance
  vectors + wire spec, re-rooted so it builds standalone), four prebuilt C ABI
  library tarballs (`grant-crypto-capi-<os>-<arch>.tar.gz`, linux/darwin ×
  amd64/arm64), each with a sibling `.sha256`, plus the aggregate `SHA256SUMS`.
- **Same fail-closed fetch discipline.** A token-authz probe on the SDK tag
  then a set-equality assert on the requested assets — a listed
  asset absent from the release fails the deploy. The source-tarball name carries
  the version, derived from the one pinned tag (`goat_get_sdk_version`), so a bump
  is a single line.
- **The SDK ships its own documentation site**, served at `get.<site>.<zone>/sdk/docs/`
  and linked prominently from the `/sdk/` landing page. It is a hand-authored
  static HTML+CSS site committed at `ops/grant-crypto/sdk/docs/site/` — so the
  source tarball (which archives all of `ops/grant-crypto/sdk`) carries it
  automatically, and a standalone `grant-crypto-sdk-docs-<ver>.tar.gz` release
  asset (the site re-rooted to `docs/`) is unpacked by the role into
  `www/sdk/docs/`. The web-served docs are the same bytes as the SDK's in-tarball
  docs, pinned to the served release. No new asset-name math beyond the
  version-derived tarball name; no separate bump point (it rides
  `goat_get_sdk_release_tag`).
- **Release pipeline.** `.github/workflows/grant-crypto-sdk-release.yml` (modeled
  on `goat-cli-release.yml`): tag `grant-crypto-sdk-v*` → builds the source
  tarball and **proves it self-sufficient by unpacking it and running `cargo test`
  from inside `grant-crypto/sdk`** (the conformance vectors resolve at their
  relative `../../envelope/testdata/vectors` path from the tarball), builds the
  four C ABI tarballs (macOS libraries signed like the goat-cli binaries),
  aggregates checksums, and attaches everything to the GitHub Release.

**Bump path (SDK):** cut the `grant-crypto-sdk-v<semver>` release → bump
`goat_get_sdk_release_tag` in the role defaults (or `-e`) → re-run
[`goat-get-deploy.yml`](../../ops/ansible/goat-get-deploy.yml) with
`goat_get_sdk_enabled: true` for the site (`goatbuilder deploy --component
goat-get --env <goatnet>`) → `goatbuilder verify goat-get --env <goatnet>`: each
host's ledger row records what it serves, read back from `/sdk/version.json`
(served payloads, universal-deploy-rollback.md).

## Deploy

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  GOAT_CLI_RELEASE_TOKEN=$GOAT_CLI_RELEASE_TOKEN \
  ansible-playbook -i inventory/hosts.ini -i inventory/mgmt-hosts-mesh.ini \
  -l <mgmt-host> goat-get-deploy.yml
```

Pre-flight: the `goat-cli-v*` release exists with its assets; the LE cert
for `get.<site>.<zone>` mints on first request via the `le` resolver (same as
brief — no static SAN edit needed); mesh DNS entry is in `mesh-hosts`.

## Non-goals

- **Windows served as of `goat-cli-v0.2.0`.** goat-cli gained a
  Windows/amd64 build in the ADR 1019 2026-06-07 revisit, and the role
  default now serves `goat-windows-amd64.exe` (the index page renders a
  Windows install block automatically). This surface serves exactly what
  the deployed release tag attaches; the served-asset assertion
  fail-closes if an asset is listed but absent, so a future tag bump must
  keep the asset list in sync. `goat client` (tunnel-daemon IPC) is the
  one subcommand unavailable on Windows (the Windows goat-clientd +
  named-pipe IPC are designed per ADR 0840 but not built yet).
- **Not public.** Mesh-only by the shared middleware — the whole auth
  surface, like the other static nova surfaces. Do not disable on nova
  without a substitute gate.
- **Not a package manager.** Flat file download; `goat profile init` does
  the real work. Not *auto*-update either — the binary is never replaced
  behind the operator's back. `goat update` (above) is operator-invoked,
  verify-before-replace self-update from the same surface; the connect-time
  staleness check only *nudges*, it never downloads or replaces anything.

## Jetson kernel-module assets (`/kmod/`, added 2026-08-09)

Third asset family on the same surface, mirroring the SDK family exactly:
`goat_get_kmod_enabled` (default off; on for `zagros_hosts` + `zagros_prod_hosts`)
serves the `jetson-wireguard-kmod` release (pinned by
`goat_get_kmod_release_tag`) at `get.<site>.<zone>/kmod/` — per-kernel
tarballs (`jetson-wireguard-kmod-<uname -r>.tar.gz`: .ko set + SHA256SUMS +
PROVENANCE + install.sh + README + GPL-2.0 LICENSE) plus aggregate
SHA256SUMS and a landing page with the one-liner install. Host-side fetch
with `goat_get_github_token`, token-probe + asset set-equality asserts,
fail-closed. Why on this surface: an on-mesh Jetson can adopt kernelspace
WireGuard (ADR 1118) with zero GitHub reach — same argument as the CLI
binaries; and the artifacts keep working when the WAN doesn't, which is
exactly when a Jetson site is installing things. Bump the release tag here +
re-run `goat-get-deploy.yml` when jetson-wireguard-kmod cuts a new release
(kernel bumps).

## octo-pod appliance assets (`/octo-pod/`, added 2026-08-15)

Fourth asset family, same shape as the kmod family: `goat_get_octopod_enabled`
(default off) serves the `octo-pod` release (pinned by
`goat_get_octopod_release_tag`) at `get.<site>.<zone>/octo-pod/` — the
`Architecture: all` .deb, the tarball for a box with no apt, per-artifact
`.sha256`, the aggregate `SHA256SUMS`, its cosign bundle, and `LATEST`.
Host-side fetch with `goat_get_github_token`, token-probe + asset
set-equality asserts, fail-closed. Artifact names carry the version, so the
asset list is templated off the tag the way the SDK family does — one
variable to bump.

**Why a subdirectory is mandatory here.** octo-pod publishes a file named
`SHA256SUMS`, as does goat-cli. Mirrored flat into the www root, whichever
landed second would silently replace the other's checksums, and every verify
on the surface would then fail against the wrong manifest.

**Why it earns a place on this surface.** The argument is the CLI's, one step
further: an octo-pod is an edge appliance, frequently a Jetson at a site whose
egress to GitHub is the slow, flaky leg — the same leg that forced the
`timeout: 300` bump on the goat-cli download task. Pointing a pod's
`octo_repo_url` at this directory means `octo-pod upgrade` reads `LATEST`,
the artifact and the signature over the mesh, and takes new software when the
WAN is down. `LATEST` is load-bearing, not decoration: it is how `upgrade`
discovers a newer version exists, and a mirror without it serves software
nobody upgrades to.

**The mirror is not in the trust path.** It copies a signed release; it does
not sign one. `octo-pod upgrade` verifies the cosign signature over
`SHA256SUMS` against the release workflow's identity after downloading, and
refuses a release it cannot verify — so a compromised surface serves software
no pod will install. This is why the set-equality assert includes
`SHA256SUMS.cosign-bundle`: a release mirrored without it would look healthy
here and be rejected at every pod, turning a publishing mistake into an
edge-side failure.

Bump the release tag here + re-run `goat-get-deploy.yml` when octo-pod cuts a
new release.

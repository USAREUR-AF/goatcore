# goatbuilder — distribution, hosting and documentation

| Field | Value |
|---|---|
| Status | **Built, released** (2026-10-29). `goatbuilder/v0.1.0` (2026-10-28) and `goatbuilder/v0.2.0` (2026-10-29) are cut and signed; zagros-dev serves the release the goat-get role pins at `get.<site>/goatbuilder/` and its book at `docs.<site>/goatbuilder/`; zagros and zagros-prod follow on the rung order. Changes per release: `crates/goatbuilder/CHANGELOG.md`. Per piece: §"Status by piece" |
| Design | [`goatbuilder.md`](goatbuilder.md) · [ADR 1198](../adr/1198-goatbuilder-config-driven-deploy-release-cli.md) |
| Model it applies | goat-cli's: [ADR 1083](../adr/1083-goat-client-single-client-and-distribution.md) (public pre-trust distribution), [`goat-cli-mesh-download.md`](goat-cli-mesh-download.md) (`get.<site>.<zone>`), [`goat-cli-hosted-docs.md`](goat-cli-hosted-docs.md) (`docs.<site>` mdBook) |

## What this is

`goatbuilder` is hosted exactly the way `goat` is — one signed build, served from two homes — plus one rule the builder needs that the runtime client does not: the **first** copy of the tool that builds an environment cannot come from that environment. Every copy after the first can.

| Home | Who it serves | What it is |
|---|---|---|
| **Public** — GitHub Releases on the open-source repository, and the public docs site, each linking to the other | anyone bootstrapping a goatnet; the external teams who run their own repos (ADR 1197) | the release artifacts and their signatures, the docs rendered from this repository |
| **Private** — `get.<site>.<zone>` inside each ZAGROS goatnet (`zagros-dev`, `zagros`, `zagros-prod`), with `docs.<site>.<zone>` beside it | operators already inside a goatnet; air-gapped and islanded sites | the **same** signed artifacts, served mesh-only, never a fork |

The two homes carry one build with one signature. The private copy exists for reach (an operator on the mesh, a site cut off from the internet), never for a different version.

## The catch-22, and how it resolves

The tool that stands a goatnet up is hosted inside goatnets. That is only circular for the first copy:

1. **Bootstrap** — the first goatnet of an organization, or a founder site with nothing to reach, gets `goatbuilder` from the public release or from an operator's laptop that already has it. This is the same seat that holds the bootstrap credentials ([goatnet-standup.md](../operations/goatnet-standup.md) phase 1) and it is the only time the binary arrives from outside.
2. **Every later copy comes from inside.** Once a goatnet serves `get.<site>`, operators download from it; a joiner site's standup pulls the binary from its founder's surface; and under the pull model the binary is one more **signed artifact on the registry chain** — an islanded site gets it from its nearest store, exactly as it gets a release (contract C2: trust is in the signed content, not in where it was fetched).
3. **The signature is the same in both homes**, so a copy from `get.zagros` and a copy from GitHub verify identically against the organization's key. A partner or an air-gapped site can host the surface without being able to alter a byte.

So the rule is: *a goatnet never needs the internet to hand its operators the builder, and the builder never needs a goatnet to exist before the first goatnet does.*

## The release

Applies goat-cli's release path to a second binary, with no new machinery:

| Step | Mechanism |
|---|---|
| **Tag** | `goatbuilder/vX.Y.Z` on `main`, nominated by a repository admin — the rule `rc-validate.yml` already enforces for every candidate ([ADR 1173](../adr/1173-release-candidate-tags-trigger-affected-flow-validation.md)) |
| **Build** | `goatbuilder-release.yml`, the shape of `goat-cli-release.yml`: Linux `x86_64` + `aarch64` and macOS, static where the target allows, `cargo test` and `clippy -D warnings` gating the build |
| **Sign** | `SHA256SUMS` over every artifact, signed with cosign under the organization's key — the same identity and the same verify command an operator runs for `goat` |
| **macOS** | the code-signing step goat-cli needed to stop the launch hang, or the binary does not run for a Mac operator |
| **Publish** | a GitHub Release on the source repository; the private `get.<site>` surfaces pull the same artifacts on their bump path |
| **Promote** | `goatbuilder` is a component like any other: a release candidate travels dev → preprod → prod on the ladder — through its own `release` verb once that verb is ported, through `rc.sh` until then |

The binary ships the schema it validates against, so a release of `goatbuilder` is a release of the schema version it enforces; the config's `schema:` field names which one a repository expects.

## Inside the goatnet — `get.<site>` and `docs.<site>`

`goatbuilder` becomes a second artifact on the surface [`goat-cli-mesh-download.md`](goat-cli-mesh-download.md) already defines, not a second surface:

- `https://get.<site>.<zone>/goatbuilder/<version>/<platform>/goatbuilder`, with the aggregate `SHA256SUMS` and the landing page's install one-liner extended with the builder; mesh-only by the shared Traefik allowlist; LE certificate on first request.
- **Staleness and self-update**: `goatbuilder update` replaces the binary with the release `get.<site>` serves, the way `goat update` does, on the same core (`crates/goat-selfupdate`). The surface is `GOATBUILDER_UPDATE_URL`, else `--site <site>` named from the site's group_vars (`site_id`, `site_dns_zone`, `goat_get_goatbuilder_enabled`, as the goat-get role renders the hostname), else the surface the last update used (`~/.goatbuilder/update-check.json`). Before the installed binary is touched it checks the asset against its `.sha256`, that digest against `SHA256SUMS`, and the keyless signature over `SHA256SUMS` (`SHA256SUMS.cosign-bundle`), verified in the binary with no cosign (`crates/goatbuilder/src/update/signature.rs`). That check is offline from the bundle. The certificate must chain to a Fulcio CA of the Sigstore trusted root valid at the entry's time, and Rekor's signed entry timestamp must verify over the canonical payload. The entry must name this file, signature and certificate. The identity must be the release signer's (the repository `goatbuilder.yaml` names, else the binary's build provenance): the GitHub OIDC issuer, that source repository, the served version's `goatbuilder/v<version>` tag as the source ref, and `goatbuilder-release.yml` at that tag as the Build Config URI. The certificate's SAN names the reusable `_release-binary.yml`, which goat's release runs too, so it is not what identifies a goatbuilder release. The trusted root is the public Sigstore instance's, embedded; `GOATBUILDER_UPDATE_TRUSTED_ROOT` names a trusted_root.json whose keys are trusted beside it, so an islanded site takes a Sigstore key rotation without a new binary. A signature that does not verify, or is not served, refuses the update (GB-254); there is no unchecked path. Status: built, unit and integration tests only; WIP — gate: an update from a published release served by a real get.<site> surface. The binary download is bounded by a 30-second stall, not a total (`GOATBUILDER_UPDATE_STALL_SECS`). `--check` reads only and exits 3 when behind. Every other verb prints one behind-version line on a terminal at most once a day (off with `GOATBUILDER_NO_VERSION_CHECK`), and `goatbuilder doctor` adds a `goatbuilder version` row, failing while the binary is behind — the builder is the one tool whose staleness can silently reintroduce a fixed defect. Errors GB-072 (no surface), GB-073 (surface unreadable or stalled), GB-074 (checksum disagrees), GB-075 (cannot replace the binary), GB-078 (no release signer is known), GB-254 (the signature does not verify, or is not served).
- **Docs**: chapters on `docs.<site>.<zone>` composed from the canonical files by include, never forked — `goatbuilder.md` (overview and contract), the one-page deploy guide, `goatbuilder-conformance.md`, this document, the error and exit-code reference generated from the binary (`-o json` is the stable machine contract), and a *Getting started* chapter that is `goatbuilder tutor` on `--mock` written down.
- **Air-gapped sites**: the artifact is carried by the same chain as the images (the pull paper's release bundle; octo-pod's air-gap bundle is the appliance-side precedent), so a site with no path out still serves `get.<site>` from what it last pulled.

The DNS record and the Traefik route are the per-site service records the mesh-download role already renders; adding the builder is one more artifact in that role, not a new role.

## After open-source

When the core goes public (ADR 1197), the public home becomes the open-source repository's GitHub Releases and its docs site, and the two homes are **mutually linked**: the public docs name `get.<site>` as the in-goatnet path and the goatnet landing page names the public release as the bootstrap path. The private goatnets keep serving the same signed artifacts for reach; they never become a different build. An external team's goatnet runs the same binary with its own config.

## Documentation standard

The builder's documentation is held to goat-cli's: a design of record ([`goatbuilder.md`](goatbuilder.md)); a one-page operator guide (the deploy guide, which replaces its script-era predecessor as each verb lands); the contract as tests ([`goatbuilder-conformance.md`](goatbuilder-conformance.md)); `--help` that is complete enough to work from, `-o json` as the reference a program reads; `tutor` as the hands-on path on the mock fixture; and the hosted book above, always matching the binary an operator just downloaded. A verb is not done until its runbook step is deleted and its chapter is written.

## Status by piece

| Piece | Status (2026-10-29) |
|---|---|
| `.github/workflows/goatbuilder-release.yml` | **built**: on `goatbuilder/v*` — admin-on-main nomination, `cargo test` + `clippy -D warnings` gate, Linux amd64/arm64 (glibc 2.31 floor via cargo-zigbuild) + macOS amd64/arm64, the version-equals-tag check, the macOS signing step verbatim from goat-cli's, per-asset `.sha256`, `SHA256SUMS` signed keyless with cosign under the workflow's identity (`SHA256SUMS.cosign-bundle`), the docs book tarball, a GitHub Release with `.github/release-notes/goatbuilder.md` as its body |
| the releases | **cut**: `goatbuilder/v0.1.0` (2026-10-28) and `goatbuilder/v0.2.0` (2026-10-29), each a signed tag on `main` by a repository admin (`git tag -s goatbuilder/v<semver> <sha on main> && git push origin goatbuilder/v<semver>`), built and signed by the workflow end to end; the version in `crates/goatbuilder/Cargo.toml` is bumped in the pull request before the tag, since the workflow refuses a binary whose `--version` differs from the tag |
| `get.<site>/goatbuilder/` | **built**: a fourth opt-in family in the goat-get role (`goat_get_goatbuilder_enabled`, `_release_tag`, `_assets`; landing page with the verify-then-install sequence; `version.json`); the release repo is named (`USAREUR-AF/goatcore`), not self-referential, per the one-release-source direction. The pinned tag is empty until the first release exists, because the release-asset parity guard refuses a pin naming a release that is in neither repo; the bump path is: set `goat_get_goatbuilder_release_tag` in the role defaults, flip the flag per goatnet in `group_vars/<site>_hosts/vars.yml`, `zagros-dev` first, re-run `goat-get-deploy.yml`, record `served_goatbuilder_release_tag` in the role's `deploy-history.yml` |
| `docs.<site>/goatbuilder/` | **built**: an opt-in second book in the goat-docs role (`goat_docs_goatbuilder_enabled`, `_release_tag`), the `goatbuilder-docs.tar.gz` asset sha256-verified and unpacked under the goat-cli book's webroot; same host, cert and mesh gate; no new DNS record |
| the book (`docs/goatbuilder-book/`) | **built**: Welcome; First run; Command reference (the binary's own `--help`, verb by verb, plus the exit codes); the design; the conformance suite; this document — every chapter but Welcome composed at build time from the canonical file by `compose.sh`, never forked. Built locally from `goatbuilder 0.1.0` (1.1 MB, 6 pages) |
| `docs/operations/goatbuilder-first-run.md` | **written**: where the binary comes from, `doctor`, secrets as references, a deploy with its ledger proof, a release in six lines, exit codes, what still runs through the scripts |
| `goatbuilder update`, the daily behind-version line, and the `goatbuilder version` row in `doctor`, against `get.<site>`'s `version.json` | **built** (2026-10-30), unreleased: lands in the next `goatbuilder/v<semver>`. `update --site zagros-dev --check` measured against get.zagros-dev (served 0.2.0); a forced install from it onto a scratch copy verified the served sha256 |
| the pull-chain carriage of the artifact | **not built**: with the pull converger's first class |
| the public home after open-source (ADR 1197) | **designed**: the release page and the hosted book link each other once the repository is public; nothing to build before then |

## Build order

1. `goatbuilder-release.yml` on `goatbuilder/v*`, signed, with the macOS step — the first public home. **Built**; released as v0.1.0 and v0.2.0.
2. The builder as a second artifact in the mesh-download role; `get.zagros-dev` first, then `zagros` and `zagros-prod` on the ladder — **built**, off by default per goatnet; `update` and the staleness check in `doctor` — **built**.
3. The docs chapters on `docs.<site>`, composed by include — **built**; the CLAUDE.md pointer landed with this step.
4. The pull-chain carriage of the artifact, when the pull converger lands for the first class.

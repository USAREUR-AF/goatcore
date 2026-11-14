# 2. Install and verify

One signed build is published in two places. Verify it the same way from
either, then run `validate` and `doctor` before anything else.

## Which machines it runs on

Linux and macOS, on amd64 and arm64. There is no Windows build, and that is a
decision rather than a gap: a run drives Ansible and aws-vault from the machine
you type on, and neither runs natively on Windows.

## Where the binary comes from

| Home | Where | Use it when |
|---|---|---|
| Public release | the `goatbuilder/v<semver>` release on the goat source repository: one binary per platform, `SHA256SUMS`, its cosign bundle, and this book | the first copy, on any machine with internet |
| Inside a goatnet | `https://get.<site>.<zone>/goatbuilder/`, with this book at `https://docs.<site>.<zone>/goatbuilder/` | every later copy, a laptop on the mesh, an islanded site |

Hosting is described in [goatbuilder-distribution.md](../../design/goatbuilder-distribution.md).

## Verify and install

```bash
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github\.com/USAREUR-AF/goatcore/\.github/workflows/_release-binary\.yml@refs/tags/goatbuilder/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS     # macOS: shasum -a 256 -c
install -m 0755 "goatbuilder-$OS-$ARCH" ~/.local/bin/goatbuilder
goatbuilder --version
```

The signature binds the checksums to the release workflow on the goat source
repository. The checksum then binds the binary to the checksums.

On macOS, if the release notes say the binary was ad-hoc signed rather than
notarized, re-sign it locally after the checksum passes. A linker-signed binary
can hang at launch.

```bash
codesign --force --sign - ~/.local/bin/goatbuilder
```

## Keep it current

`goatbuilder update` replaces the binary with the release a goatnet serves,
after checking it, and changes nothing when already current.

```bash
goatbuilder update --site <site> --check     # running against served; exit 3 when behind
goatbuilder update --site <site>             # download, verify, swap
goatbuilder update                           # later: the surface the last update used
```

| Step | What it checks |
|---|---|
| surface | `GOATBUILDER_UPDATE_URL`, else `--site` (from `ops/ansible/group_vars/<site>_hosts/`: `site_id`, `site_dns_zone`, `goat_get_goatbuilder_enabled`), else `~/.goatbuilder/update-check.json` |
| version | `version.json`; current or newer prints one row and exits 0, `--force` reinstalls |
| checksum | the asset's `.sha256` equals its line in `SHA256SUMS`, and the download hashes to it |
| signature | `SHA256SUMS.cosign-bundle` verified by goatbuilder itself, offline, with no cosign needed. It checks the Sigstore certificate chain, Rekor's timestamp, and the release workflow `goatbuilder-release.yml` of the repository `goatbuilder.yaml` names, at the served version's tag. Not verified, or not served: GB-261, nothing installed. After a Sigstore key rotation, set `GOATBUILDER_UPDATE_TRUSTED_ROOT` to a trusted_root.json that holds the new key |
| install | a temp file beside the binary, renamed over it: the old binary or the new one, never a partial |

The binary download stops after 30 seconds without a byte
(`GOATBUILDER_UPDATE_STALL_SECS` for a slower link), not after a total time.
Run it as the user that owns the install directory. Errors: GB-072 no surface,
GB-073 surface unreadable or stalled, GB-074 checksum or signature disagrees,
GB-075 cannot replace the binary.

Every other verb, on a terminal and with table output, prints one line at most
once a day when the binary is behind the remembered surface;
`GOATBUILDER_NO_VERSION_CHECK=1` silences it and the `doctor` row.
`goatbuilder doctor --site <site>` adds a `goatbuilder version` row that fails
while the binary is behind.

## Build from source

Any machine with a Rust toolchain can build it from a checkout.

```bash
cd crates && cargo build --release -p goatbuilder
# the binary: crates/target/release/goatbuilder
```

## Cutting a release of goatbuilder itself

A repository administrator tags a commit on `main`. The tag is the whole
nomination. `.github/workflows/goatbuilder-release.yml` refuses a tag pushed by
anyone else or on a commit that is not on `main`.

First, in the version pull request:

1. Bump `version` in `crates/goatbuilder/Cargo.toml` and `crates/Cargo.lock`.
2. Fold the changelog's entries into the release's section, which removes
   the entry files from `crates/goatbuilder/changelog.d/`:

   ```bash
   python3 ops/release/changelog.py release <version> <date>   # for example: release 0.7.0 2026-11-20
   ```

3. Add the release's opening summary under its new heading in
   `crates/goatbuilder/CHANGELOG.md`, and its paragraph to
   `.github/release-notes/goatbuilder.md`.

```bash
git tag -s goatbuilder/v<semver> <sha on main> && git push origin goatbuilder/v<semver>
```

Serving a release inside a goatnet is the goat-get and goat-docs roles' bump:

1. Set `goat_get_goatbuilder_release_tag` and `goat_docs_goatbuilder_release_tag`
   to the release tag in the two roles' defaults.
2. Set `goat_get_goatbuilder_enabled: true` and `goat_docs_goatbuilder_enabled: true`
   in the goatnet's `group_vars/<site>_hosts/vars.yml`.
3. Run `goat-get-deploy.yml` and `goat-docs-deploy.yml` with `-e target=<site>_hosts`.
4. Record the served tag in each role's `deploy-history.yml`.

## First commands

Run these from the checkout you will deploy from. A deploy or release runs from
a `main` checkout that is current with origin, never from a feature branch.

```bash
git checkout main && git pull --ff-only origin main
goatbuilder validate                  # the config against its schema, offline
goatbuilder doctor --env <goatnet>    # what a run on that environment needs
goatbuilder tutor                     # eighteen lessons on throwaway fixtures
```

**Success looks like** `validate: ok` and every `doctor` row `ok`. Each `FAIL`
row names its fix, and `doctor` exits 1 while any row fails.

## Global flags

Every verb accepts these.

| Flag | Meaning |
|---|---|
| `-o table\|json\|yaml` | Output for a person (table) or a program. JSON carries `"schema": "goatbuilder.output/v1"` |
| `-C, --repo <DIR>` | The repository root. Default: the checkout the binary runs in |
| `--yes` | Confirm a declarative verb without a prompt. Required when there is no terminal (GB-008) |
| `--frozen` | Refuse any run that would need to fast-forward the checkout |
| `--offline` | An islanded site: judge the checkout against the cached trunk instead of fetching it, and record the run as offline |
| `--mock <DIR>` | Run against a fixture directory instead of hosts |
| `--lang <TAG>` | Language for help and error explanations. Also `GOAT_LANG` |
| `-V, --version` | Print the version |

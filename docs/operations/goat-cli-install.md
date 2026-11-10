# goat-cli — install runbook

Operator-facing install for the `goat` binary published by the
`.github/workflows/goat-cli-release.yml` workflow on every
`goat-cli-v<semver>` tag push. Replaces the source-build path
that was the only option through Phase D.B.1.

Two install paths: the **GitHub Release** one-liner below (operators with
repo access), and the **mesh download** surface (on-mesh vendors who can't
reach the private repo). Both serve the same release binaries: macOS,
Linux, and — since the ADR 1019 2026-06-07 revisit — Windows/amd64
(`goat-windows-amd64.exe`). `goat-cli-v0.2.0` is the first release built
with the Windows leg (v0.1.0 was Unix-only). On Windows, every subcommand
works except `goat client` (the tunnel-daemon IPC), which is not built for
Windows yet — see the Windows section below.

## macOS: the launch-hang re-sign (read first on a Mac)

On macOS, the released binary ships with the Rust linker's
`adhoc,linker-signed` signature. That signature form is fragile on
macOS 14+ (Sonoma/Sequoia/Tahoe): the first time you run `goat`, the
kernel's AMFI / Gatekeeper trust evaluation can **stall the process in
the dynamic loader before `main()`** — an indefinite, un-killable launch
hang (`goat --version` never returns) — and a *copied* binary can be
`Killed: 9` outright. Root cause + evidence:
F-208
(sibling F-209).

**Fix: re-sign once, locally, right after install** — this replaces the
fragile linker-signed signature with a proper ad-hoc one (fresh cdhash,
registers cleanly in the trust cache):

```bash
codesign --force --sign - "$(which goat)"   # prefix with sudo for a /usr/local/bin install
```

Every macOS install block below already includes this line (guarded to
run only on `darwin`). Releases built after the signing pipeline landed
carry a proper signature already, so the re-sign is a harmless no-op
there — keep it for older binaries and copies. The durable end-state is
Developer-ID-signed + notarized releases (`spctl -a` accepts, no eval
stall): [`goat-cli-macos-signing.md`](goat-cli-macos-signing.md).

## Mesh download (on-mesh / vendors)

For anyone already on the NetBird mesh — including vendors who receive an
onboarding bundle — install from the mesh-internal surface instead of the
private GitHub release page:

```bash
# Your goatnet's download host (from your onboarding packet):
#   nova-a → get.nova.netbird.nova.example
#   ZAGROS → get.zagros.netbird.zagros.example
GOAT_GET=get.nova.netbird.nova.example
(
set -eu   # fail closed inside this subshell, without changing your shell options
GOAT_INSTALL_DIR=$(mktemp -d)
trap 'rm -rf "$GOAT_INSTALL_DIR"' EXIT
OS=$(uname -s | tr '[:upper:]' '[:lower:]')   # darwin | linux
ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
BIN="goat-${OS}-${ARCH}"
cd "$GOAT_INSTALL_DIR"
curl -fsSL "https://${GOAT_GET}/${BIN}" -o "$BIN"
curl -fsSL "https://${GOAT_GET}/${BIN}.sha256" -o "${BIN}.sha256"
EXPECTED_SHA=$(awk -v bin="$BIN" '$2 == bin { print $1 }' "${BIN}.sha256")
if [ "$OS" = darwin ]; then
  printf '%s  %s\n' "$EXPECTED_SHA" "$BIN" | shasum -a 256 -c - --strict
  codesign --force --sign - "$BIN"   # clear the launch hang, after verification
else
  printf '%s  %s\n' "$EXPECTED_SHA" "$BIN" | sha256sum -c - --strict
fi
sudo mkdir -p /usr/local/bin
sudo install -m 0755 "$BIN" /usr/local/bin/goat
/usr/local/bin/goat --version
)
```

`/usr/local/bin` is normally on the macOS and Linux shell `PATH`.
The block verifies the installed binary using its absolute path. If `goat`
is not found afterward, add `/usr/local/bin` to your shell's `PATH`.
`~/.local/bin` is NOT reliably on `PATH`: Arch never adds it, Ubuntu/Debian
only if the folder already existed when the shell started, and macOS
doesn't add it at all — which is why the mesh-download flow above no
longer uses it (the Quick install, Verified install, source-build and
rollback sections below still do — a separate, pre-existing concern).

**Already have a `goat` on your `PATH` from an older `~/.local/bin`
install?** Debian's default `.profile` prepends `~/.local/bin` ahead of
`/usr/local/bin` when the folder exists, so the old copy keeps shadowing
this one and `goat --version` silently won't show the change. Remove it:
`rm -f ~/.local/bin/goat`.

**On a mgmt host running `onboard-runner`?** That container bind-mounts
`/usr/local/bin/goat` and pins the inode present at container start
— swapping the host binary in place leaves the container running
the OLD `goat` until it's restarted (`docker restart onboard-runner` or
the role re-run). Not a concern for an ordinary operator laptop.

The host is mesh-only (NetBird must be up first). Surface design +
deploy: [`../design/goat-cli-mesh-download.md`](../design/goat-cli-mesh-download.md).
Vendor rollout: [`goat-cli-vendor-rollout.md`](goat-cli-vendor-rollout.md).

## Quick install (one-liner)

```bash
# Pick latest. Substitute the version + OS/arch for your laptop.
VERSION=goat-cli-v0.5.15
OS=$(uname -s | tr '[:upper:]' '[:lower:]')   # darwin | linux
ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')

# The repo is private, so an unauthenticated curl gets a 404. gh uses your login.
gh release download "$VERSION" --repo USAREUR-AF/goatcore \
  --pattern "goat-${OS}-${ARCH}" --output /tmp/goat --clobber
install -m 0755 /tmp/goat ~/.local/bin/goat
rm -f /tmp/goat
[ "$OS" = darwin ] && codesign --force --sign - ~/.local/bin/goat   # macOS: clear the launch hang

goat --version
```

Add `~/.local/bin` to `PATH` if it isn't already.

## Verified install (sha256-check first)

The release page attaches per-binary `*.sha256` sidecars plus an
aggregate `SHA256SUMS`. For air-gapped or
high-assurance installs, verify before exec:

```bash
VERSION=goat-cli-v0.5.15
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')

cd /tmp
gh release download "$VERSION" --repo USAREUR-AF/goatcore --clobber \
  --pattern "goat-${OS}-${ARCH}" --pattern "goat-${OS}-${ARCH}.sha256" --pattern SHA256SUMS

# Per-binary check.
sha256sum -c "goat-${OS}-${ARCH}.sha256"   # linux
shasum -a 256 -c "goat-${OS}-${ARCH}.sha256"   # macOS

# Or check against the aggregate manifest.
grep "goat-${OS}-${ARCH}" SHA256SUMS | sha256sum -c -

install -m 0755 "goat-${OS}-${ARCH}" ~/.local/bin/goat
rm -f "goat-${OS}-${ARCH}" "goat-${OS}-${ARCH}.sha256" SHA256SUMS
[ "$OS" = darwin ] && codesign --force --sign - ~/.local/bin/goat   # macOS: clear the launch hang
```

> Verify the sha256 **before** the `codesign` re-sign — the checksum
> matches the released bytes; re-signing rewrites the signature load
> command, so the on-disk hash changes afterward (expected).

## Supported targets

Per the release workflow's build matrix:

| OS | Architecture | Asset | Notes |
|---|---|---|---|
| linux | amd64 | `goat-linux-amd64` | ubuntu-latest runner, linked against glibc 2.31 (Ubuntu 20.04, Debian 11 or later) |
| linux | arm64 | `goat-linux-arm64` | ubuntu-24.04-arm runner, linked against glibc 2.31 (includes Jetson Ubuntu-for-Tegra 22.04) |
| darwin | amd64 | `goat-darwin-amd64` | macos-14 runner, cross-compiled to x86_64 |
| darwin | arm64 | `goat-darwin-arm64` | macos-14 runner (Apple Silicon) |
| windows | amd64 | `goat-windows-amd64.exe` | windows-latest runner (native MSVC) |

**Linux glibc floor.** goat-cli-v0.5.4 and earlier linked the glibc of the
runner that built them. That `goat-linux-amd64` needs glibc 2.39 (Ubuntu 24.04
or later) and stops at launch on Ubuntu 22.04 or Debian 12 with
``version `GLIBC_2.38' not found``. Releases after goat-cli-v0.5.4 link against
glibc 2.31, and the release build fails if a Linux binary needs more. To see
what an installed binary needs:

```bash
objdump -T ~/.local/bin/goat | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
```

If your goatnet still serves an affected release, run it in a container with a
newer glibc until the serving pin moves:

```bash
docker run --rm -it --net=host --user "$(id -u):$(id -g)" -e HOME="$HOME" \
  -v "$HOME:$HOME" -w "$PWD" -v "$(command -v goat):/usr/local/bin/goat:ro" \
  ubuntu:24.04 goat --version
```

Inside the container `goat status` cannot reach the NetBird daemon, so it
reports the PQC state as unknown; the mesh itself is unaffected.

**Windows/amd64** was added in the ADR 1019 2026-06-07 revisit (the "operator demand" trigger). The one subcommand it lacks is `goat client` (tunnel-daemon IPC) — the Windows goat-clientd + named-pipe IPC are designed (ADR 0840) but not built yet. Everything else works.

### Windows install

The repo is private, so download with the GitHub CLI (`gh auth login` first).
Command Prompt:

```bat
set "VERSION=goat-cli-v0.5.15"
gh release download %VERSION% --repo USAREUR-AF/goatcore --pattern goat-windows-amd64.exe --output "%USERPROFILE%\goat.exe" --clobber
"%USERPROFILE%\goat.exe" --version
```

PowerShell:

```powershell
$VERSION = "goat-cli-v0.5.15"
gh release download $VERSION --repo USAREUR-AF/goatcore --pattern goat-windows-amd64.exe --output "$env:USERPROFILE\goat.exe" --clobber
& "$env:USERPROFILE\goat.exe" --version
```

Verify the checksum against `SHA256SUMS` with `certutil -hashfile goat.exe SHA256`
(PowerShell: `Get-FileHash goat.exe -Algorithm SHA256`), then move `goat.exe`
somewhere on your `PATH` and open a new window. The profile lives at
`%USERPROFILE%\.goat`.

## After install

```bash
goat ca add <goatnet-ca.pem>      # trust the CA that signed your bundle (once per goatnet)
goat profile init <bundle.cbor>   # land your operator-onboard CBOR
goat doctor                       # verify 4-step reachability
goat org ls                       # browse the trial portal
```

**Why the first line.** `goat` compiles in no goatnet CA, so it starts out
trusting nothing and `profile init` refuses until you tell it which CA to
trust. That is what lets one binary work against every goatnet you have added
— `goat ca ls` shows them.

The root PEM comes with your onboarding tarball as
`<prefix>-goat-cli-ca.pem`, beside the `.cbor` it verifies. If you are already
on a goatnet's mesh, every root is also served at
`https://get.<site>.<zone>/ca-roots/` with its fingerprint.

`goat ca add` prints a SHA-256 fingerprint. **Check it against one an operator
gave you out of band** before you rely on it — a file that reached you over a
channel proves only that the channel delivered it.

Skipping this used to be impossible because one CA was baked into the binary;
that also meant a release could only install bundles from one goatnet, which is
F-406.

See `docs/design/goat-cli.md` for the full subcommand catalog.

## Stuck on a pre-0.5.0 binary (no `goat ca` at all)

If `goat ca` errors as an unrecognized subcommand, the installed binary
predates the F-406 fix (first shipped in `goat-cli-v0.5.0`) and compiles in
a closed, small list of CA keys instead — `0.4.44` and earlier recognize at
most four goatnets and never included plain `zagros` (only `zagros-prod`);
anything older than `0.4.44` recognizes exactly one, nova, and nothing
else. A completely valid, uncorrupted bundle from any goatnet outside that
binary's baked-in list fails `profile init` with:

```
error: bundle signature or expiry verification failed
  help: signature invalid — wrong CA, tampered payload, or canonical-form mismatch
  code: E_BUNDLE_INVALID
```

which reads exactly like a corrupt or tampered file. It is not — re-downloading
or re-minting the same bundle changes nothing, because the bundle was never
the problem. The binary is.

**Update the binary before touching the bundle at all — `goat update` works
for this even with no profile.** It's resolved `get.<site>` from
`GOAT_GET_ENDPOINT` before it ever reads a profile (since `0.4.26`), so
point it straight at the goatnet's mesh-download surface:

```bash
GOAT_GET_ENDPOINT=https://get.zagros.netbird.zagros.example goat update
```

```bat
:: Windows (Command Prompt)
set "GOAT_GET_ENDPOINT=https://get.zagros.netbird.zagros.example" && goat update
```

```powershell
# Windows (PowerShell)
$env:GOAT_GET_ENDPOINT = "https://get.zagros.netbird.zagros.example"
goat update
```

That path verifies the published `.sha256` before replacing anything, keeps
the exec bit set, and handles the Windows rename-aside `goat update` already
does for a locked, running `.exe` — none of which a bare download does for
you. It does **not** run the macOS `codesign` re-sign the curl fallback
below includes — `goat update` has no signing step, though the release
pipeline already ad-hoc-signs the darwin binary (the F-208 fix), so this
should be a non-issue in practice. If `goat` hangs on first launch right
after an update, that's the tell: `codesign --force --sign - "$(which goat)"`
(`sudo` in front for a `/usr/local/bin` install).

**Fallback — only for a binary too old to have `goat update` at all**
(anything before `0.4.26`, so this shouldn't come up in practice): pull the
binary directly from the same mesh-download surface by hand.

```bash
# macOS / Linux — self-contained, no dependency on the Mesh download block above
(
set -eu   # fail closed inside this subshell, without changing your shell options
GOAT_INSTALL_DIR=$(mktemp -d)
trap 'rm -rf "$GOAT_INSTALL_DIR"' EXIT
OS=$(uname -s | tr '[:upper:]' '[:lower:]')   # darwin | linux
ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
BIN="goat-${OS}-${ARCH}"
cd "$GOAT_INSTALL_DIR"
curl -fsSL "https://get.zagros.netbird.zagros.example/${BIN}" -o "$BIN"
curl -fsSL "https://get.zagros.netbird.zagros.example/${BIN}.sha256" -o "${BIN}.sha256"
EXPECTED_SHA=$(awk -v bin="$BIN" '$2 == bin { print $1 }' "${BIN}.sha256")
if [ "$OS" = darwin ]; then
  printf '%s  %s\n' "$EXPECTED_SHA" "$BIN" | shasum -a 256 -c - --strict
  codesign --force --sign - "$BIN"   # clear the launch hang, after verification
else
  printf '%s  %s\n' "$EXPECTED_SHA" "$BIN" | sha256sum -c - --strict
fi
sudo mkdir -p /usr/local/bin
sudo install -m 0755 "$BIN" /usr/local/bin/goat
)
```

```bat
:: Windows (Command Prompt)
curl -fsSL "https://get.zagros.netbird.zagros.example/goat-windows-amd64.exe" -o "%USERPROFILE%\goat.exe"
```

```powershell
# Windows (PowerShell)
iwr "https://get.zagros.netbird.zagros.example/goat-windows-amd64.exe" -OutFile "$env:USERPROFILE\goat.exe"
```

Then confirm the fix actually landed — `goat ca ls` should now be a
recognized subcommand — and retry `profile init` on the exact same `.cbor`
from before. No re-download, no re-mint.

If the machine already has a working profile from before (it was set up
before this bug mattered, or `goat ca add` already succeeded there once),
plain `goat update` with no `GOAT_GET_ENDPOINT` override works too — it
derives `get.<site>` from the profile's own `portal_endpoint` in that case.

## Updating

`goat update` replaces the installed binary in place: it resolves `get.<site>` from the profile's `portal_endpoint` (or `GOAT_GET_ENDPOINT`), verifies the published `.sha256`, and handles the Windows rename-aside for a running `.exe`. The install one-liner from the get page does the same by hand. There's no auto-update — per ADR 1019 §"Patterns explicitly NOT adopted" — a person runs `goat update` when they want the new version; `goat doctor` says when one is waiting. Releases are tagged + the GH release page shows changelog notes.

**Installed via the mesh-download page or the Fallback block above (`/usr/local/bin`)? Update with `sudo /usr/local/bin/goat update --profile-dir "$HOME/.goat"`.** That directory is root-owned, and `goat update` writes its replacement binary beside the running one with no privilege-elevation of its own — a plain `goat update` fails there with a permission error. `--profile-dir` is not optional under `sudo`: `sudo` resets `HOME` to root's and drops `GOAT_GET_ENDPOINT`, so without it `goat` reads `/root/.goat`, finds no `portal_endpoint`, and exits with `cannot resolve this context's download surface (get.<site>)`. `~/.local/bin` installs (Quick install / Verified install / source-build) need neither.

## Source-build fallback

The release workflow may temporarily fail to publish (zenoh upstream regression, GH runner outage, etc.). Source-build path is always available:

```bash
gh repo clone USAREUR-AF/goatcore
cd goat/crates
cargo build --release -p goat-cli
install -m 0755 target/release/goat ~/.local/bin/goat
[ "$(uname -s)" = Darwin ] && codesign --force --sign - ~/.local/bin/goat   # macOS: clear the launch hang
```

Source build needs the full Rust toolchain (~5GB, Rust 1.88 or later since 0.5.14) and takes ~30 min on a fresh laptop because zenoh's transitive deps are large.

## Cutting a release

Author with admin access to the repo:

```bash
# From main, with the desired commits already merged (next version shown):
git tag goat-cli-v0.4.29
git push <remote> goat-cli-v0.4.29
```

This fires `goat-cli-release.yml`, which builds the 5-target matrix (linux/darwin × amd64/arm64 + windows/amd64), computes per-binary SHA-256, aggregates into SHA256SUMS, and attaches everything to a GH Release for the tag. The Release notes are auto-generated from commits between this tag and the previous `goat-cli-v*` tag.

If the workflow's tag-attach step fails (typically because the Release already existed with different assets), re-run the failed job from the Actions UI — the upload step is idempotent on artifact names.

## Rollback

The released binaries are immutable artifacts on GH. To "roll back," operators install an earlier `goat-cli-v<X>` tag's binary using the same one-liner; the installer overwrites the existing `~/.local/bin/goat`. Per ADR 0859 (easy-rollback engineering principle): every install step is reversible without restoring from backup.

## See also

- macOS code-signing + notarization (launch-hang fix): [`goat-cli-macos-signing.md`](goat-cli-macos-signing.md), F-208
- Release workflow: `.github/workflows/goat-cli-release.yml`
- ADR 1019 §Distribution + install + the 2026-06-07 Windows-support addendum: [`docs/adr/1019-goat-cli-unified-operator-binary.md`](../adr/1019-goat-cli-unified-operator-binary.md)
- Design doc §Distribution + install: `docs/design/goat-cli.md`

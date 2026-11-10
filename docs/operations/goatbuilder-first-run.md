# goatbuilder — first run

`goatbuilder` deploys and releases a goat environment from a checkout of the
source repository on `main`. This page is the first hour: where the binary
comes from, what to run before anything is cut, a deploy with its proof, and a
release in six lines. Full reference: [the goatbuilder reference](../reference/goatbuilder/README.md);
design of record: [goatbuilder.md](../design/goatbuilder.md);
hosting: [goatbuilder-distribution.md](../design/goatbuilder-distribution.md).

## Where the binary comes from

One signed build, two homes. Verify the same way from either.

| Home | Where | When |
|---|---|---|
| Public release | the `goatbuilder/v<semver>` release on the source repository (`USAREUR-AF/goatcore`): one binary per platform, `SHA256SUMS`, its cosign bundle, the docs book | the first copy; any laptop with internet |
| Inside a goatnet | `https://get.<site>.<zone>/goatbuilder/` with the book at `https://docs.<site>.<zone>/goatbuilder/` | every later copy; a laptop on the mesh; an islanded site |

```bash
# from the release page or from get.<site>.<zone>/goatbuilder/ — same files, same signature
OS=$(uname -s | tr '[:upper:]' '[:lower:]'); ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/usareur-af/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS          # shasum -a 256 -c on macOS
install -m 0755 "goatbuilder-$OS-$ARCH" ~/.local/bin/goatbuilder
goatbuilder --version
```

macOS: if the release log says the binary was ad-hoc signed rather than
notarized, run `codesign --force --sign - ~/.local/bin/goatbuilder` after the
checksum passes; a linker-signed binary can hang at launch.

From source, on any machine with a Rust toolchain: `cd crates && cargo build
--release -p goatbuilder` puts it at `crates/target/release/goatbuilder`.

**Cutting the release** (repository admin, on `main`, after the release
workflow is merged): the tag is the whole nomination; the workflow refuses a
tag from anyone but an admin or on a commit not on `main`.

```bash
# bump crates/goatbuilder/Cargo.toml to <semver> and add its CHANGELOG entry first
git tag -s goatbuilder/v<semver> <sha on main> && git push origin goatbuilder/v<semver>
```

Serving it inside a goatnet is the goat-get and goat-docs roles' bump path,
the dev rung's goatnet first, then the rung order: once the release exists, set
`goat_get_goatbuilder_release_tag` and `goat_docs_goatbuilder_release_tag` to
it in the two roles' defaults (they are empty until then, so no pin names a
release that is not cut), set `goat_get_goatbuilder_enabled: true` and
`goat_docs_goatbuilder_enabled: true` in that goatnet's
`group_vars/<site>_hosts/vars.yml`, run `goat-get-deploy.yml` and
`goat-docs-deploy.yml` with `-e target=<site>_hosts`, and record the served tag
in each role's `deploy-history.yml`.

**New to it?** `goatbuilder tutor` walks every verb in 18 short lessons on throwaway copies of a fixture, and checks each command you run before the next lesson opens; nothing touches a goatnet.

## Before anything is cut: `doctor`

Run from the checkout you will deploy from. `doctor` names what a run needs:
the tools, the checkout rule (a release runs from a `main` checkout current
with origin, never a feature branch — F-586; an assembled tree, marked by
its `.goat-assembled` file, is on a detached HEAD by construction and passes
with what it was built from), the transport to each
environment, every playbook's target mode, and the secrets each
component's play templates.

```bash
cd <checkout of the source repository>
git checkout main && git pull --ff-only origin main
goatbuilder validate                 # the config against its schema, offline
goatbuilder doctor --env <goatnet> --credentials   # one environment; omit --env for all
```

Environment keys are the pins' goatnet keys, one per rung of the release
ladder (dev, preprod, prod). Every row is `ok` or `FAIL` with a fix; the
verb exits 1 on any `FAIL`. The live rows' browser toolchain (`tool:live-rows`)
is a note unless you pass `--browser-tests`; pass it before a release.

## Secrets: named before any host is touched

A play templates its secrets from your environment (`lookup('env', NAME)`).
`doctor` and `deploy` list every variable a component needs and which are not
set, and say how to supply each one under your team's declared store (next
section). `ops/scripts/with-secrets <command>`, or `goatbuilder secrets exec --
<command>`, runs one command with the store's values.

```bash
goatbuilder secrets missing <NAME>…    # which of these the store cannot supply
goatbuilder secrets hint <NAME>…       # how to supply them under this store
goatbuilder secrets doctor             # whether the store can serve: signed in, file mode
```

Under a reference store, one malformed reference anywhere in your shell can
refuse the whole run (GB-031). `env | grep -E 'op://|bao://'` finds it; complete
it or unset it for the command.

## Choosing your secrets posture

Your team declares where deploy secrets live, once, as `secrets: {store: …}` in
`ops/goatbuilder/goatbuilder.yaml`; `GOAT_SECRET_STORE` overrides it for one
command ([ADR 1204](../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md),
[pluggable-dependencies.md](../design/pluggable-dependencies.md)). Every driver
and `goatbuilder` read the same declaration, and `goatbuilder doctor` prints the
store in force on its first `secrets:` row.

- **`1password`** — references only, resolved per command by `op run`. No value
  ever sits on disk. goat's own team runs this (ADR 1186).
- **`openbao`** — references to a self-hosted OpenBao store (ADR 1203), resolved
  per command. The same protection without a SaaS account, at the cost of running
  the store.
- **`sops`** or **`git-crypt`** — values encrypted in a committed file. Nothing
  outside git to run; revoking a person means rotating every value they could
  decrypt, and an unlocked git-crypt file is plaintext on that laptop.
- **`env-file`** — plaintext values in `.envrc.local.d/secrets.env`, gitignored
  and mode 0600. The fastest start; a copied disk, a backup or a stray `cat`
  exposes every value. `doctor` refuses the file if it is readable by others or
  not gitignored.
- **`env`** — values already exported in your shell. Nothing is resolved.

A laptop with no SaaS accounts starts here:

```bash
goatbuilder secrets init --env <goatnet>   # strong random values, 0600, gitignored; a re-run keeps them
export GOAT_SECRET_STORE=env-file
goatbuilder doctor --env <goatnet>         # lists any value only an outside service can issue
```

Without aws-vault, `GOAT_AWS_MODE=profile` uses a plain AWS profile and
`GOAT_AWS_MODE=none` runs with no AWS at all (a deploy over the mesh needs none).
Image digests resolve with crane, skopeo or docker, whichever is installed.

## A deploy, with its proof

Transport is decided by reaching, not by name: the laptop's NetBird profile,
then the environment's mesh host on tcp/22, then the SSM door, then the base
inventory. Tailscale is never a transport.

```bash
# decide everything, touch nothing
goatbuilder deploy --env <goatnet> --component <component> --dry-run

# the live run, secrets resolved from the declared store for this one command
ops/scripts/with-secrets \
  goatbuilder deploy --env <goatnet> --component <component> --ack "<your name>" --yes

# the proof, on its own: the newest ledger row on every host names the
# declared digest with action=deploy (C7); exit 3 while any host does not
goatbuilder verify <component> --env <goatnet>     # same check as deploy --verify-only

# what differs from main right now, every component of the environment
goatbuilder diff --env <goatnet>

# the ledger as a timeline: each deploy and rollback, its prior ref and operator
goatbuilder history <component> --env <goatnet>
```

The ledger is `/var/lib/goat/<component>/deploy-history.jsonl` on each host.
`deploy` reports done only from that record; a host it cannot read is
*unknown*, a host that has not converged is *pending*, and neither is ever
counted as done.

## A release, in six lines

```bash
goatbuilder release microscope --version 0.1.0 --ack "<your name>" --dry-run   # every step, mutating nothing
goatbuilder release microscope --version 0.1.0 --ack "<your name>" --yes       # tag → CI → dev deploy → live rows → preprod
goatbuilder release microscope --tag microscope/v0.1.0-rc.2 --from dev --yes   # resume an existing candidate at a step
# … the preprod soak (the ladder's min_days) …
goatbuilder release microscope --tag microscope/v0.1.0-rc.2 --from prod --yes  # graduate, fold into fleet:, deploy prod, re-verify
goatbuilder rollback microscope --list                                          # the history, newest first; `rollback microscope 1` reverts one step
```

Nomination stays with repository admins: the candidate tag is refused for
anyone else, and the run stops there.

## Output and exit codes

Every verb takes `-o table|json|yaml`; JSON is the stable form a program
reads. Exit `0` ok, `1` refused (the report names what and the fix), `2`
usage, `3` not converged (`status`/`deploy`) or refused as unsafe for the
artifact type (`rollback`), `4` guided (`rollback` on a config or ACL
artifact prints the manual steps). A refusal keeps every step that ran as the
last rows of the report.

## The scripts beside the verbs

Every verb with a script equivalent keeps it: `rc.sh`, `deploy-component.sh`,
`rollback.sh`, `fastlane.sh`, `graduate.py`, `generate-pins.py`,
`goat-standup.sh` and `peering-standup.sh` stay on `main`, documented and
working. goatbuilder is the supported avenue. A CI parity gate per verb runs
both sides dry and fails on any difference
([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)). The
full table is the reference's
[Appendix B](../reference/goatbuilder/appendix-b-script-equivalents.md).

## The full reference

This page is the first hour. Every verb as a task, the contract, secrets,
transports, errors and testing are in the
[goatbuilder reference](../reference/goatbuilder/README.md).

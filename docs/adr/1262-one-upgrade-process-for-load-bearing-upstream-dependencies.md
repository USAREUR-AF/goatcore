# ADR 1262 — Every load-bearing upstream dependency upgrades through one process: one manifest, a private watcher, one bump command, a candidate suite, then the ladder

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-26 |
| Repo scope | goat core (`ops/upstream/`, CI), goatstuff (its own manifests), the private overlay (watcher, private manifest entries, ladder coupling) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0858](0858-zenoh-image-upstream-pivot.md), [ADR 1079](1079-zenoh-pqc-hybrid-tls.md), [ADR 1120](1120-release-graduation-ladder.md), [ADR 1177](1177-graduation-is-never-automated-the-daily-review-is.md), [ADR 1211](1211-rollout-preflight-readiness-and-reversibility.md), [`upstream-upgrade-process.md`](../design/upstream-upgrade-process.md), [`zenoh-version-pinning.md`](../design/zenoh-version-pinning.md) |

## TL;DR (for a layperson)

goat is built on a handful of outside projects it cannot run without: the Zenoh data fabric,
the NetBird mesh, the Ory sign-in services, PostgreSQL and a few more. Each publishes new
versions with fixes we need. Today moving to one of them is a project of its own: someone
notices the release, finds every file that names the old version by hand, works out which of
our own fixes are still needed, and tests in whatever way occurs to them. This decision makes
it one routine path that every such project goes through, so a new version becomes a
prepared, tested candidate waiting for a person to roll it out.

## Context

The Zenoh check of 2026-11-26 found the fleet on 1.9.0 while upstream had shipped three
releases, one of them fixing a live exposure (a single unauthenticated QUIC connection blocks
all new QUIC admissions, upstream #2719, fixed in 1.10.2). The version is named in about 175
places across goatcore alone. The check that should hold them together,
`ops/ci/check-zenoh-pinning.sh`, was called by no workflow, and `ops/grant-crypto/keyd/Cargo.toml`
used the range `"1.9"`, which the check let through. A backport we carried (upstream #2635)
had been in a release for three months before anyone noticed. NetBird (our fork of
netbirdio/netbird) and Ory have the same shape: a version spread across images, role
defaults and lock files, local changes on top, and clients or partners that must move with
the server.

Dependabot covers Docker base digests and GitHub Actions. It cannot read a version in an
Ansible role default, a Go string a partner is shown, or a patch list, and it opens public
pull requests, which during an embargo would disclose the fix.

## Decision

1. **One manifest per dependency**, `ops/upstream/deps/<name>.yaml`, holds the version of
   record, the facts that move with it (image digest, plugin checksums, toolchain), the rules
   that locate every place the version appears, the patches or fork commits carried with
   their finding, upstream pull request and first release containing the fix, who must move
   with it, and the name of its candidate suite.
2. **One tool**, `ops/upstream/upstream.py`, reads only the manifest: `check` (every place
   agrees), `bump <name> <version>` (rewrites every place, prints the lock-file commands,
   names each carried patch whose fix has shipped), `patches --src` (which patches still
   apply to an upstream checkout), `watch` (newer releases, keywords in their notes, carried
   patches whose upstream pull request has shipped).
3. **CI enforces the manifest** on every pull request (`.github/workflows/upstream-pins.yml`),
   after a self-test that proves each rule kind fails a broken fixture.
4. **The watcher is scheduled privately.** The private overlay runs `watch` weekly and opens
   or updates one private issue per dependency. It also merges `<name>.private.yaml`, which
   lists what must not be written publicly (an embargoed patch, its paths).
5. **The embargo stop is the existing public push scan.** A bump touches exactly the files an
   embargoed dependency patch touches; the private push scan already refuses such a push to a
   public repository. A bump of a dependency with an open embargo goes to the embargo owner
   before its first push.
6. **Each dependency has a candidate suite** run in CI on the candidate build: the plugin or
   service boots, the access-control proofs pass, old clients work against the new server and
   new clients against the old one, database migrations run forward on a copy, and the
   finding-specific rigs pass their recorded bars.
7. **Rollout is the release ladder unchanged.** A green candidate is recorded and climbs
   the dev, preprod and prod rungs by a platform admin's explicit instruction per rung. Graduation
   is never automated (ADR 1177). The server and its clients are linked with `coupled_with`
   so the ladder moves them in order.

Per dependency, only the manifest and the candidate tests differ.

## Consequences

- A version is changed in one file and one command; CI fails any place left behind.
- Dropping a backport is prompted by the tool when the release containing it is reached.
- A new dependency joins by adding a manifest and a candidate suite, no new code.
- Partner-facing text that names a version is a rule like any other, so it cannot lag the
  router.
- `check-zenoh-pinning.sh` stays as an entry point and calls the tool.

## Alternatives rejected

| Option | Why not |
|---|---|
| Dependabot or Renovate alone | Cannot see role defaults, Go and HTML text, patch lists or fork commits; opens public pull requests, which discloses an embargoed fix |
| A separate script per dependency | Three copies of the same logic diverge; the Zenoh check already showed a check that drifted from its doc |
| Generate every place from the manifest by template | Places are Cargo, Go, HTML, YAML and Dockerfiles owned by different tools; checking them is cheap, templating all of them is not |
| Run the watcher in public goatcore | Its issue would list the patches we carry and their upstream state, including an embargoed one |

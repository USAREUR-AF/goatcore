# OpenSSF Scorecard: the result per check, and what each check asks of a change

| Field | Value |
|---|---|
| Status | **Built: the practices in the rows marked built below. Not yet: action pins and default token permissions in four workflow files. Not built: the rows marked so, and a Scorecard run in CI.** |
| Anchor ADR | [ADR 1246](../adr/1246-workflows-pin-actions-and-start-read-only-releases-carry-signatures-and-provenance.md) |
| Related | [ci-discipline.md](ci-discipline.md) (the rules for writing a workflow) · [codeql-coverage.md](codeql-coverage.md) · [ADR 0707](../adr/0707-rust-toolchain-pinning.md) (advisories that stay open are listed with a reason) · [`CONTRIBUTING.md`](../../CONTRIBUTING.md) |
| Audience | anyone who adds a workflow, a dependency or a release, and the maintainers |

[OpenSSF Scorecard](https://github.com/ossf/scorecard) reads a public repository and scores 18 checks from 0 to 10. This page records the result for this repository per check, what is built for each, and the rule a contributor follows so that a change keeps the result.

## Result per check

Both columns were measured on 15 November 2026 with Scorecard v5.4.0. The first is `main` before the changes listed here. The second is `main` at commit `194681c4`, with every pull request marked merged below.

| Check | Before | With the merged changes | State |
|---|---|---|---|
| Aggregate | 4.0 | 5.5 | |
| Dangerous-Workflow | 0 | 10 | **Built** (#895). The three `pull_request_target` gates take no checkout `ref` from the event. Rule W4 in [ci-discipline.md](ci-discipline.md#rules-for-writing-a-workflow). |
| Vulnerabilities | 0 (35 advisories) | 3 (7 advisories) | **Built**, with dependency updates (#907, #908, #925) and recorded ignores (#909). 7 advisories are not yet cleared. See [Dependency advisories](#dependency-advisories). |
| SAST | 8 | 8 | **Built** (#900). CodeQL runs on every pull request and every push to `main`. Scorecard reads the 30 most recent commits, and 13 of them carried a result at the measurement. |
| Signed-Releases | 0 | 0 | **Built** in the `wg-cp0` and grant-crypto SDK release workflows (#904); no release has been cut with them. The `goat` and `goatbuilder` releases do not yet carry the files. Scorecard reads the five most recent releases that have assets. See [Release signatures and provenance](#release-signatures-and-provenance). |
| Token-Permissions | 0 | 0 | **Built** for 96 of 98 workflows (#915). The check scores 0 while any workflow lacks top-level `permissions`; two do not yet. Rules W2 and W3. |
| Pinned-Dependencies | 1 | 6 | **Built** for action references (#915): 392 of 446 name a full commit SHA, and the rest are in four workflow files not yet converted. Not built: hashes for `pip`, `npm` and `go install` commands, digests for Dockerfile base images, and checks for download-then-run commands. Rule W1. |
| Code-Review | 0 | 0 | **Not built.** The ruleset `main-protection` requires a pull request, signed commits and the checks `leak-gate` and `contributor-trust`, and requires no approving review. Owners merge on green checks. Requiring an approval is an owner decision, and it has not been taken. |
| Branch-Protection | 3 | 3 | **Not built.** The same ruleset. Scorecard reports four settings as absent: required approvers, code-owner review, approval of the last push, and up-to-date branches. |
| Maintained | 0 | 0 | Scorecard scores this check once the repository is 90 days old. |
| CII-Best-Practices | 0 | 0 | **Not built.** The project is not registered for the OpenSSF Best Practices badge. |
| Contributors | 3 | 3 | Scorecard counts the organizations that recent contributors belong to, and it found one. |
| Binary-Artifacts, CI-Tests, Dependency-Update-Tool, Fuzzing, License, Packaging, Security-Policy | 10 | 10 | Built. |

A weekly Scorecard workflow and the README badge are in pull request #695, a draft. Until it merges, no CI job runs Scorecard, and a result comes from a run by hand.

## Run Scorecard yourself

Install [`scorecard`](https://github.com/ossf/scorecard#installation) and the [GitHub CLI](https://cli.github.com/), then run it against the published repository:

```bash
GITHUB_AUTH_TOKEN=$(gh auth token) scorecard --repo=github.com/USAREUR-AF/goatcore --show-details
```

Add `--checks=<Name>` for one check, for example `--checks=Token-Permissions`. `--show-details` names each file and line that a check counted.

To test a change before it merges, run the checks that read only the working tree against your checkout:

```bash
scorecard --local=. --checks=Dangerous-Workflow,Pinned-Dependencies,Token-Permissions,Vulnerabilities --show-details
```

The other checks read GitHub's record of the repository (releases, rulesets, pull requests, commits) and report on `main` only.

## What each check asks of a change

| You are | Do this | Check that reads it |
|---|---|---|
| Adding or editing a workflow | Follow rules W1 to W4 in [ci-discipline.md](ci-discipline.md#rules-for-writing-a-workflow). | Pinned-Dependencies, Token-Permissions, Dangerous-Workflow |
| Adding a dependency, or a lock file | Follow [Dependency advisories](#dependency-advisories). | Vulnerabilities |
| Cutting a release | Follow [Release signatures and provenance](#release-signatures-and-provenance). | Signed-Releases |
| Opening any pull request | Expect a `codeql` run of about three minutes. It is not a required check. [codeql-coverage.md](codeql-coverage.md) has the detail. | SAST |

## Dependency advisories

A lock file or `go.mod` that enters `main` carries no published advisory, or carries a recorded reason for each one. Scorecard's Vulnerabilities check reads every lock file through [OSV](https://osv.dev/) and takes one point off for each advisory it finds.

When a dependency you add or update has a published advisory:

1. **Update it** to a version that fixes the advisory. For Rust, `cargo update -p <crate> --precise <version>` in the directory of that `Cargo.lock`. For Go, `go get <module>@<version> && go mod tidy` in the module.
2. **If no fixed version exists, or the advisory does not apply to how the code uses the dependency, record an ignore** in the file `osv-scanner.toml` in the same directory as the lock file. One entry per advisory holds the public advisory id and one factual sentence that says why it does not apply:

   ```toml
   [[IgnoredVulns]]
   id = "RUSTSEC-2025-0134"
   reason = "Unmaintained-crate notice for rustls-pemfile: the advisory reports no vulnerability and names no fixed version."
   ```

3. **Keep `.github/workflows/rust-audit.yml` consistent.** That workflow runs `cargo audit` on each Rust lock file in its matrix and fails on a vulnerability it is not told to ignore. An advisory that `cargo audit` fails on needs an `--ignore <id>` in the matrix row for that lock file, with the reason in the comment above it, as well as the `osv-scanner.toml` entry. An unmaintained-crate notice does not fail `cargo audit` and needs only the `osv-scanner.toml` entry.

Scorecard and `osv-scanner` read the `osv-scanner.toml` in each lock file's own directory. An entry in the file at the repository root covers the root `Cargo.lock` only. A new `Cargo.lock` therefore gets its own `osv-scanner.toml` when it carries an advisory that stays.

| File | Covers |
|---|---|
| `osv-scanner.toml` | `Cargo.lock` (the relay workspace) |
| `crates/osv-scanner.toml` | `crates/Cargo.lock` |
| `ops/consistency/rust/osv-scanner.toml` | `ops/consistency/rust/Cargo.lock` |
| `ops/federation/republisher/osv-scanner.toml` | `ops/federation/republisher/Cargo.lock` |
| `ops/grant-crypto/keyd/osv-scanner.toml` | `ops/grant-crypto/keyd/Cargo.lock` |
| `ops/zenoh-ingress-gateway/osv-scanner.toml` | `ops/zenoh-ingress-gateway/Cargo.lock` |
| `ops/zenoh/pqc-image/bench/osv-scanner.toml` | `ops/zenoh/pqc-image/bench/Cargo.lock` |
| `ops/zenoh/pqc-image/fips-probe/osv-scanner.toml` | `ops/zenoh/pqc-image/fips-probe/Cargo.lock` |

Check the result before you push:

```bash
scorecard --local=. --checks=Vulnerabilities --show-details
```

The gates that already run on a pull request are unchanged: `rust-audit.yml` (`cargo audit`, Rust), the `go-vuln` job in `ci.yml` (`govulncheck`, every tracked Go module, coverage checked by `ops/ci/check_go_vuln_coverage.py`) and `cargo-deny.yml` (licenses and sources).

## Release signatures and provenance

A release workflow that signs per asset attaches these files to the release page, beside the assets:

| File | What it is | Verify with |
|---|---|---|
| `SHA256SUMS` | the SHA-256 of every asset | `sha256sum -c --ignore-missing SHA256SUMS` |
| `SHA256SUMS.cosign-bundle` | a keyless cosign signature over `SHA256SUMS` | `cosign verify-blob --bundle` |
| `<asset>.sigstore.json` | a keyless cosign signature over that one asset, in the Sigstore bundle format | `cosign verify-blob --new-bundle-format --bundle` (cosign 2.4 or later) |
| `<component>.intoto.jsonl` | SLSA build provenance: every asset by digest, and the workflow run that built it | `gh attestation verify --bundle … --signer-workflow` |

Each release page prints the full commands for its own assets. The text comes from the component's file in `.github/release-notes/`.

| Component | Workflow | `SHA256SUMS` and its signature | Per-asset signature and provenance |
|---|---|---|---|
| `wg-cp0` | `wg-cp0-release.yml` | built | built; the next `wg-cp0/v<semver>` release is the first to carry them |
| `goat-pod-api` | `goat-pod-api-release.yml` | built | built; no `goat-pod-api/v<semver>` release has been cut yet |
| grant-crypto SDK | `grant-crypto-sdk-release.yml` | built | built; the next release is the first to carry them |
| `goat` | `goat-release.yml`, through `_release-binary.yml` | built | not yet |
| `goatbuilder` | `goatbuilder-release.yml`, through `_release-binary.yml` | built | not yet |

When you cut a release of a component in the "built" rows, list the assets and confirm the new files are there:

```bash
gh release view <tag> -R USAREUR-AF/goatcore --json assets --jq '.assets[].name'
```

The list holds one `.sigstore.json` per binary or archive and one `.intoto.jsonl`. Then run the two verification commands from the release page on one asset.

A new release workflow follows `wg-cp0-release.yml`: the job that writes `SHA256SUMS` also signs each asset, writes the provenance with `actions/attest-build-provenance`, verifies both, and holds `id-token: write` and `attestations: write` on that job only.

## Not built

- A check that refuses a workflow that breaks rule W1, W2, W3 or W4. The rules are read by a reviewer and by Scorecard.
- A check that refuses a lock file with an advisory that has no `osv-scanner.toml` entry. `rust-audit.yml` and the `go-vuln` job refuse vulnerabilities; neither reads `osv-scanner.toml`.
- A Scorecard run in CI and the README badge (#695, draft).
- Per-asset signatures and provenance on the `goat` and `goatbuilder` releases.
- Registration for the OpenSSF Best Practices badge.

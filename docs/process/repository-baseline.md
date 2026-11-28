# What every repository carries

Four controls belong to the repositories of an organization that holds goat, from a repository's first pull request. Two apply to every repository and two to public repositories only. Each one acts on what is about to be added; none of them reads or rewrites history that is already merged.

| Member | Applies to | What it stops | How it is delivered | Decision |
|---|---|---|---|---|
| **Commit identity** | every repository | A personal mailbox entering commit history without its owner's choice | The status `commit-identity`, required by an organization ruleset. It is set by one workflow in the organization's `.github` repository that the ruleset runs on every pull request, or, in a repository that had open pull requests when the check started, by that repository's own copy of the workflow | [ADR 1276](../adr/1276-a-personal-mailbox-in-a-pull-request-is-refused-once.md) |
| **Credential scanning** | every repository | A credential entering the repository | Public: GitHub secret scanning with push protection. Private and internal: the check `credential-scan`, gitleaks at a pinned version and checksum over the commits a pull request adds, findings redacted; delivered the same two ways as commit identity | [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) |
| **Leak gate** | public repositories | A deployment's own names, addresses, places or people entering a public tree | A `leak-gate` workflow in the repository, its check required by an organization ruleset. The scanner is taken from goatcore `main`; the list of names is one encrypted organization secret, shared with the public repositories that run the gate. A hit shows the file, the line and an id, never the name | [ADR 1215](../adr/1215-open-source-split-five-repos-usareur-af.md) |
| **License and notice** | public repositories | Code with no stated terms | `LICENSE` and `NOTICE` at the repository root. A fork keeps its upstream's license files and needs no notice of ours. A private or internal repository needs neither | [ADR 1215](../adr/1215-open-source-split-five-repos-usareur-af.md) |

Two more controls apply to every repository and are decided elsewhere: every commit is signed and verified ([ADR 1226](../adr/1226-every-commit-in-a-pull-request-is-signed-and-verified.md)), and outside contributions are reviewed by tier ([ADR 1221](../adr/1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md)).

## Check a repository, or all of them

```sh
GITHUB_TOKEN=$(gh auth token) python3 ops/ci/audit-repo-baseline.py <organization>
GITHUB_TOKEN=$(gh auth token) python3 ops/ci/audit-repo-baseline.py <organization> --visibility all --json
```

The report has one row per repository and one column per member. A cell reads `required` (present and enforced), `present` (there, not enforced), `missing`, or `n/a` where the member does not apply. A line per visibility under the table counts the repositories where each member is in place. The command reads the GitHub API and changes nothing; it exits 1 when any cell is `missing`.

## Where it stands

Measured with the command above over the public organization's 75 repositories that are not archived:

| | Commit identity required | Credential scanning in place | Leak gate required | License and notice |
|---|---|---|---|---|
| Public (39) | 39 | 39 | 39 | 39 |
| Private (27) | 26 | 27 | does not apply | does not apply |
| Internal (9) | 9 | 9 | does not apply | does not apply |

The one private repository without the commit-identity status has a required check of its own that fails on its default branch for a reason unrelated to this page; its pull request waits for that.

## A new repository

- **Commit identity** needs nothing: the organization ruleset `commit-identity` covers a repository from its creation. Add `.github/workflows/commit-identity-recheck.yml` (from `.github/commit-identity/recheck-workflow.yml` in goatcore) so that a `/email-ok` comment takes effect at once.
- **Credential scanning**: a private or internal repository needs nothing, the organization ruleset `credential-scan` covers it from its creation. For a public repository, enable secret scanning and push protection in its security settings.
- **Leak gate** (public): copy `.github/leak-gate/caller-workflow.yml` from goatcore to `.github/workflows/leak-gate.yml`, add the repository to the organization secret `GOAT_LEAK_NEEDLES` (Actions and Dependabot) and to the ruleset `leak-gate-status`.
- **License and notice** (public): add `LICENSE` and `NOTICE` before the first pull request. Which license applies is a decision for the repository's owner; the audit reports a missing file and never adds one.

## Why two deliveries

A workflow that an organization ruleset requires starts only when a pull request is opened, reopened or pushed to. A pull request that was already open has no result and stays blocked until its next push. So a check is first installed as a file in each repository that has open pull requests, every open pull request is given a result, and only then is the check required. A repository with no open pull requests, and every new one, takes the organization workflow directly.

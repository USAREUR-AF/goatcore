# What every repository carries

Four controls belong to every repository of an organization that holds goat, from its first pull request. Each one acts on what is about to be added; none of them reads or rewrites history that is already merged.

| Member | What it stops | How it is delivered | Decision |
|---|---|---|---|
| **Leak gate** | A deployment's own names, addresses, places or people entering a public tree | A `leak-gate` workflow in the repository, required on the default branch | [ADR 1215](../adr/1215-open-source-split-five-repos-usareur-af.md) |
| **Secret scanning** | A credential entering the repository | GitHub secret scanning and push protection, both enabled on the repository | [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) |
| **License and notice** | Code with no stated terms | `LICENSE` and `NOTICE` at the repository root | [ADR 1215](../adr/1215-open-source-split-five-repos-usareur-af.md) |
| **Commit identity** | A personal mailbox entering public commit history without its owner's choice | One workflow in the organization's `.github` repository, run on every pull request by an organization ruleset that also requires its status | [ADR 1276](../adr/1276-a-personal-mailbox-in-a-pull-request-is-refused-once.md) |

Two more controls apply to every repository and are decided elsewhere: every commit is signed and verified ([ADR 1226](../adr/1226-every-commit-in-a-pull-request-is-signed-and-verified.md)), and outside contributions are reviewed by tier ([ADR 1221](../adr/1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md)).

## Check a repository, or all of them

```sh
GITHUB_TOKEN=$(gh auth token) python3 ops/ci/audit-repo-baseline.py <organization>
GITHUB_TOKEN=$(gh auth token) python3 ops/ci/audit-repo-baseline.py <organization> --visibility all --json
```

The report has one row per repository and one column per member. A cell reads `required` (present and enforced), `present` (there, not enforced) or `missing`. The command reads the GitHub API and changes nothing; it exits 1 when any cell is `missing`.

## A new repository

- **Commit identity** needs nothing: the organization ruleset covers a repository from its creation. Add `.github/workflows/commit-identity-recheck.yml` (from `.github/commit-identity/recheck-workflow.yml` in goatcore) so that a `/email-ok` comment takes effect at once.
- **Secret scanning**: enable secret scanning and push protection in the repository's security settings.
- **Leak gate**: add the `leak-gate` workflow and require its check on the default branch.
- **License and notice**: add `LICENSE` and `NOTICE` before the first pull request. Which license applies is a decision for the repository's owner; the audit reports a missing file and never adds one.

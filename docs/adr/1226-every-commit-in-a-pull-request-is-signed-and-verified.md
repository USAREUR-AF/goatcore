# ADR 1226 — Every commit in a pull request is signed and verified, and one organization rule enforces it on every repository

| Field | Value |
|---|---|
| Status | Accepted (2026-11-09, operator) |
| Date | 2026-11-09 |
| Deciders | operator |
| Repo scope | Multi (every repository in the organizations that hold goat: the public organization and the private overlay's) |
| Amends | [ADR 0109](0109-git-collaboration-trunk-based.md) (signing covers every commit in a pull request, not only the commit that lands on `main`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1186](1186-credentials-at-rest-are-references-only.md) (credentials at rest are references; a signing key is a person's credential), [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) |

## TL;DR (for a layperson)

Every change to goat is recorded as a commit, and every commit must carry a digital signature that GitHub checks against a key the author registered. A pull request that contains even one commit without a valid signature is refused and cannot be merged, in every repository, until its author signs it. One rule per GitHub organization does this for all of its repositories at once, so a new repository is covered from its first pull request. The result is that every line of goat can be traced to a person who proved who they were when they wrote it.

## Context

ADR 0109 requires signed commits on `main` and relies on GitHub's "require signed commits" branch rule. That rule only inspects the commits that land on `main`. goat squash-merges pull requests, and GitHub signs the squash commit with its own key whatever the pull request contained, so a pull request made entirely of unsigned commits passes the rule and lands as a commit GitHub reports verified. The signature on `main` then proves only that GitHub merged it, not who wrote it.

The repositories also enforced the rule unevenly: the public repositories required signed commits on `main`, the private overlay did not, and other repositories had no rule. Counted on 2026-11-09, 30 of 165 open pull requests across the two organizations held commits GitHub does not verify: 27 unsigned and 3 signed with a key not registered as a signing key on the author's account.

## Decision

- **Every commit in every pull request carries a signature GitHub verifies** (`verification.verified: true`, reason `valid`), in every repository of both organizations. A commit that is unsigned, signed with a key not registered as a signing key on its author's GitHub account (`unknown_key`), or made from an email no account has verified (`no_user`) is refused.
- **One organization ruleset per organization enforces it**, named `signed-and-verified-commits`, targeting every repository's default branch, with no bypass actors. It carries two rules:
  - `required_signatures`: every commit reaching the default branch is signed.
  - `workflows`: the workflow `.github/workflows/verified-commits.yml` in the organization's `.github` repository must pass on every pull request. It reads `GET /repos/{repo}/pulls/{n}/commits` and fails when any commit is not verified, naming each commit, its reason and how to re-sign the branch. It runs on `pull_request_target`, so a pull request from a fork is checked without waiting for a maintainer to approve the run, and it checks out no code. On a refusal it tells the author on the pull request: one comment that names each unverified commit, the reason and the fix, and the label `needs-signed-commits`; both are cleared by the push that fixes it.
- **Squash-merge stays** (ADR 0109). The squash commit on `main` is signed by GitHub; the check above is what proves each author's commits were signed.
- **Automation commits through the GitHub API, never by pushing unsigned commits.** A workflow or service that writes to a repository creates its commit through the contents API or the GraphQL `createCommitOnBranch` mutation with its token; GitHub signs those commits. A `git push` of commits made inside a runner is unsigned and is refused.
- **A signing key is a person's credential** (ADR 1186). The private half stays in the person's own key store (an SSH agent, a password manager's SSH agent, or a hardware key) and never enters a repository, a shared host or an environment file. The public half is registered on the person's GitHub account as a *signing* key; an authentication key alone does not verify commits. A person who leaves is removed from the organization, and their registered keys stop verifying new work.
- **Contributors verify locally too.** `git config --global gpg.ssh.allowedSignersFile ~/.ssh/allowed_signers`, with the file listing each trusted email and public key, lets `git log --show-signature` check signatures on a laptop; GitHub's merge key (`https://github.com/web-flow.gpg`) imported into GPG checks squash commits.

## Alternatives considered

- **Branch rule "require signed commits" alone.** Already in place on the public repositories. It cannot see the commits inside a squash-merged pull request, which is the gap this ADR closes.
- **A copy of the check in every repository.** Works, but 134 repositories would each carry a copy that drifts, and a new repository would start unprotected. An organization ruleset covers every repository, including new ones, from one file.
- **GitHub's vigilant mode.** It changes how unsigned commits are displayed for the people who enable it. It refuses nothing.
- **Rebase-merge to keep authors' signatures on `main`.** It would put the authors' own signatures on `main`, but it gives up the one-commit-per-pull-request history ADR 0109 chose, and still needs a rule refusing unsigned commits.

## Consequences

**Positive.** Every commit on every branch that merges was signed by a key its author registered, and GitHub checked it. The rule is the same in every repository, and a new repository is covered without anyone remembering to configure it.

**Negative.** A first-time contributor must set up commit signing before their first pull request can merge. Open pull requests with unverified commits fail the check until their authors re-sign and push again (`git rebase --exec 'git commit --amend --no-edit -S' origin/main`, then `git push --force-with-lease`). Automation that committed with `git push` must move to the API. The check makes one API call per pull request and runs on GitHub-hosted runners.

## Assessor view (SCA-R)

This is a change-provenance control: each change is attributable to an identified person through a credential that person holds, and an unattributable change cannot reach a protected branch. It supports NIST SP 800-53 CM-5 (access restrictions for change), SA-10 (developer configuration management) and AU-10 (non-repudiation) for source changes. Enforcement sits at the organization level with no bypass actors, so a repository administrator cannot turn it off for one repository. The residual risk is a stolen signing key; the compensating controls are keys held in personal key stores or hardware, removal from the organization when a person leaves, and pull-request review before merge.

## Trigger conditions for revisit

- GitHub's branch rule starts verifying every commit of a squash-merged pull request itself, making the workflow redundant.
- The organizations move to a forge other than GitHub.
- A contributor population appears that cannot hold a signing key (for example, a partner whose policy forbids registering keys with GitHub).

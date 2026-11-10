# ADR 1221 — An outside contribution merges only from a known author, or after a recorded security review; sensitive paths always need the review

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-11-08 |
| Deciders | operator |
| Repo scope | Multi: every public repository of the organization. The check is a required status check on `main` in each one; the first six were goatcore, goatstuff, goatdocs, goatenv, hello-galaxy-goatenv and goatrodeo (2026-11-08), the remaining twenty followed on 2026-11-10 |
| Amends | [ADR 1015](1015-code-review-discipline.md) (adds a review tier for contributions from outside the maintainers) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1215](1215-open-source-split-five-repos-usareur-af.md) (the public repositories), [`SECURITY.md`](../../SECURITY.md) (advance notice to registered members), [`CONTRIBUTING.md`](../../CONTRIBUTING.md) ("Outside contributions") |

## TL;DR (for a layperson)

Anyone can propose a change to goat. Before this decision, a change from a stranger needed the same single approval as a change from a maintainer. Now a check on every pull request asks who wrote it. A maintainer's change goes through as before. A change from someone we know, because they are registered in one of our community environments or have had a change merged before, also goes through, unless it touches code that controls security: the build and CI, who may do what, cryptography, the relay. A change from someone we do not know, and any outside change to that security code, waits until an owner has reviewed it for security and recorded that by applying a label. Pushing new code after the review withdraws it.

## Context

The public repositories accept pull requests from anyone. Their protection on `main` was: one approval from a code owner (`goat-owners`), the last push approved, signed commits, squash merges, the leak gate passing, and an owner approving the first CI run of any outside contributor. None of that distinguishes a maintainer from an account created that morning. A single approving review is the only thing between an outside change and `main`, and a reviewer reading a 250-line change to relay routing or to the portal's access checks can miss a flaw put there by accident or on purpose.

Two facts tell us an author is known. goat's community environments register people, and a registered person who links a GitHub account is made a read collaborator of the private advance-notice repository (`SECURITY.md`); that collaborator list is a roster of registered GitHub accounts that already exists and is kept current when people leave. And GitHub records who has had a pull request merged in the organisation.

A prior contribution is a weaker signal than it looks: a contributor can build trust with harmless changes before sending a harmful one. So a known author skips the extra review only away from security-sensitive code.

## Decision

- **A required check, `contributor-trust`, runs on every pull request** in each public repository. It runs under `pull_request_target` from the base branch, reads the author and the changed paths from the GitHub API, and never checks out, builds or runs the pull request's code. A change to the check takes effect only after it merges.
- **Tiers.**
  - *maintainer*: GitHub reports the author as `OWNER`, `MEMBER` or `COLLABORATOR`;
  - *automation*: Dependabot;
  - *known*: a collaborator of the roster repository (registered), or the author of at least one merged pull request in the organisation;
  - *unknown*: neither.
- **When a security review is needed.** Always for *unknown*. For *known*, when the change touches a path in `.github/contributor-trust/sensitive-paths.txt`: CI and build files, pins and lockfiles, identity, access and policy code, the identity console, cryptography, the relay and the fabric. Never for *maintainer* or *automation*, whose changes take the normal review.
- **Recording the review.** An owner (admin or maintain permission) reviews the change against the checklist in `CONTRIBUTING.md` and applies the label `security-reviewed`. The check passes only if the account that applied the label holds that permission. A new push removes the label, so the review always covers the commits that merge. The approving review that merges the pull request comes from a different owner than the one who applied the label.
- **Fail closed.** If the check cannot read the API it fails, and is re-run. If the roster cannot be read (no token configured), the author counts as not registered, never as registered.
- **The roster is configured, not written into the public tree.** The repository variable `CONTRIBUTOR_ROSTER_REPO` names the roster repository, and the secret `CONTRIBUTOR_ROSTER_TOKEN` is a read-only token that can list its collaborators. No list of people appears in any public repository.
- **The check is made required** in each repository's `main` ruleset once it has run on open pull requests.
- **Required checks cannot be skipped.** The `main-protection` ruleset in every public repository has no bypass actors and requires no approving review: anyone with write permission (owners and maintainers only) merges when the required checks are green, and a required check that is failing or never started blocks everyone. The emergency path is an owner editing the ruleset, a logged settings change. Until 2026-11-10 the `goat-owners` team held an always-on bypass that owners used for their own merges and that also skipped the checks; on 2026-11-09 #237 merged through it while every run of this check was failing to start (issues #338, #339). octo-pod keeps its own two-approval rule.

## Consequences

- An outside contribution from an unknown author cannot merge until an owner records a security review with the label; only an account with write permission can perform the merge, and no account can merge past a failing or missing required check.
- Owners and maintainers merge their own pull requests as soon as the required checks are green; no approving review is required by the ruleset, and no bypass exists to skip a check.
- A registered person or a returning contributor gets a fast path for ordinary changes, and still gets the review on sensitive code.
- Approving an outside contributor's first CI run stays separate and stays safe: no repository uses a self-hosted runner, fork runs get no secrets, and the only workflows that run with more privilege (`leak-gate`, `agent-references`, this check) run the base branch's code.
- The sensitive-path list is a judgement, kept in one file the owners edit by pull request.
- Someone who knows the roster repository's name learns nothing from it: it is private, and the check never prints who is on it.

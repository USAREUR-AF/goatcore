# ADR 1276 — A pull request that would publish a personal mailbox is refused once, until its owner fixes it or accepts it

| Field | Value |
|---|---|
| Status | Proposed. The operator directed on 2026-11-28 that it be put in place on every repository of the organization; this record describes what is installed |
| Date | 2026-11-28 |
| Deciders | operator |
| Repo scope | Multi (every public repository of the organization, once rolled out) |
| Amends | — |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1226](1226-every-commit-in-a-pull-request-is-signed-and-verified.md) (signed and verified commits; the same delivery by organization ruleset), [ADR 1221](1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md) (contributor-trust; the script lives in goatcore), [`../process/commit-identity.md`](../process/commit-identity.md), [`../process/repository-baseline.md`](../process/repository-baseline.md) |

## TL;DR (for a layperson)

A commit records who made it, with an email address, and that record is public forever. Many contributors do not realize that the sign-off line they add to each commit copies their personal mailbox into that record. A new check reads each pull request and, the first time it finds a personal mailbox, stops the pull request and tells the person how to keep it out. A person who does not mind says so once with a short comment, and is never stopped or reminded again.

## Context

A commit carries an author address, a committer address and, with `git commit -s`, a `Signed-off-by:` line built from the local `user.email`. GitHub's "Keep my email addresses private" and "Block command line pushes that expose my email" cover the first two. Neither reads the commit message, so a sign-off made on a machine whose `user.email` is a personal mailbox passes both, and a squash merge copies it onto the default branch. A signature is verified against the committer address, and the no-reply address satisfies that, so nothing about signing requires a personal mailbox.

The choice belongs to the person: some contributors want their address in the history. A check that refuses every such commit forever takes that choice away; one that only warns is not read.

## Decision

- **A status named `commit-identity` on each pull request's head commit.** It is set by `.github/commit-identity/check.py` in goatcore. One workflow in the organization's `.github` repository runs it from goatcore `main` on every pull request of every repository, by an organization ruleset, the delivery shape of the verified-commits check (ADR 1226); the same ruleset requires the status. A repository may also carry a small recheck workflow that recomputes the status on a comment, which a ruleset cannot run. The pull request's code is never checked out.
- **It reads only what a pull request adds.** Commits already merged are not read and nothing asks for them to be rewritten.
- **What is read:** for every commit the pull request adds, the author address, the committer address, and each `Signed-off-by:` and `Co-authored-by:` line.
- **What passes:** a GitHub no-reply address, the addresses GitHub itself writes, an address under a domain reserved for examples (`.invalid`, `.example`, `.test`, `example.com` and the like), and the lines of `allowed-addresses.txt`. Every other address is a personal mailbox. The rule lists what is acceptable; it does not list mail providers.
- **The first time, the status fails** and one comment on the pull request shows each address masked, the commands that remove it, and the link to the guidance page.
- **A person accepts their own address once** by commenting exactly `/email-ok`. The status is recomputed on the comment and turns green with no new push. From then on no pull request of theirs, in any repository of the organization, is stopped or commented on for their own address. Deleting the comment takes it back.
- **Only the owner of an address accepts it.** An author or committer address belongs to the account GitHub maps the commit to. A trailer belongs to the commit's author when it carries the author's address, or is a sign-off under the author's name or login; likewise the committer. A trailer for anyone else has no known owner and cannot be accepted on that person's behalf: it is fixed, or an owner of the repository (admin or maintain) comments `/email-ok-maintainer`, which holds for that pull request alone.
- **The record is the comment.** A workflow on a timer in the `.github` repository (`registry.py`) keeps an index of where each person's `/email-ok` is: one comment on one issue there, holding user names and links to the comments. The check follows the link and reads the comment itself, so the index is never trusted and an entry whose comment is deleted stops counting at once. No mailbox is stored anywhere, and no credential beyond each workflow's own token is needed. The index reads public repositories; in a private repository an acceptance holds for that repository.
- **Bots and the merge queue pass.** A pull request opened by a bot account is not read. A merge group is given a passing status: its commits were decided on their pull request.
- **It fails closed on an API failure** (status `error`), except the lookup of an earlier acceptance: when that cannot be read the person is asked again on this pull request.
- **It is one member of the repository baseline** ([`../process/repository-baseline.md`](../process/repository-baseline.md)): the leak gate, secret scanning with push protection, the license and notice files, and this check. `ops/ci/audit-repo-baseline.py` reports every repository of an organization against the four.

## Alternatives considered

- **Refuse every personal mailbox, always.** Removes the contributor's choice, and blocks forks that carry upstream history.
- **Warn only.** A comment that blocks nothing is not acted on; the first-time stop is what makes a person choose.
- **A list of acknowledged logins in a file, written by each repository's workflow.** Every repository's workflow would need a credential that can write to one shared repository, held as an organization secret and exposed to runs triggered from pull requests and comments. The comment is already durable, public, tied to the account that wrote it, and removable by its author; the index of comments needs only the `.github` repository's own token.
- **Finding the comment with the search API.** Measured on the pilot: a comment was not returned by a search several minutes after it was written.
- **A copy of the workflow in every repository as the only delivery.** A new repository would start without it, and a required status that nothing sets blocks every pull request. An organization ruleset runs one file everywhere.
- **GitHub's push block alone.** It does not read commit messages, and it is a setting each person must find.
- **A list of mail-provider domains as the test.** Misses every address on a domain not listed, including an agency's or an employer's.

## Consequences

- A contributor meets the question once and is told the fix in the place they are working.
- A repository that takes upstream history by pull request (a fork that syncs) would be stopped by other people's addresses; such a repository needs an owner override per sync, or stays outside the rollout.
- A commit made with an address GitHub maps to no account cannot be accepted by anyone; the verified-commits check (ADR 1226) refuses the same commit, and the fix is the same.
- History already pushed is unchanged.

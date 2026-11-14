# Contributing

Thank you for working on goat. This page is the whole process; nothing else is required reading before your first pull request.

## The short version

1. Fork, or branch from `main` if you have write access. Branch names: `feat/<topic>`, `fix/<topic>`, `docs/<topic>`.
2. Make the change. Run the gates that apply (below). Add or update the design document in the same change when behaviour or topology changes.
3. Commit with a [Conventional Commits](https://www.conventionalcommits.org/) subject, a DCO sign-off (`-s`) and a verified signature (`-S`): `git commit -S -s -m "fix(zenoh): …"`. Set up commit signing once before your first commit; [Commits](#commits) says how. A pull request with an unsigned commit cannot merge.
4. Open a pull request against `main`. Fill the template. One approving review from a code owner merges it, by squash.

## Commits

- **Conventional Commits.** `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `ci`, with a scope in parentheses naming the component (`feat(goatbuilder): …`). The subject is ≤ 100 characters and says what changed, in the imperative.
- **DCO sign-off on every commit** (`git commit -s`). By signing off you certify the [Developer Certificate of Origin 1.1](https://developercertificate.org/): you wrote the change or have the right to submit it under this project's licence. There is no CLA. The policy is in [Policy](#policy).
- **Every commit is signed and verified.** Set up [SSH or GPG commit signing](https://docs.github.com/en/authentication/managing-commit-signature-verification) once and register the key on your GitHub account as a *signing* key. The required check `verified-commits` refuses a pull request while any of its commits is unsigned or signed with an unregistered key; re-sign with `git rebase --exec 'git commit --amend --no-edit -S' origin/main` and `git push --force-with-lease`. Automation commits through the GitHub API, which signs them, never by pushing unsigned commits. The decision is [ADR 1226](docs/adr/1226-every-commit-in-a-pull-request-is-signed-and-verified.md).
- One logical change per pull request. Refactors and behaviour changes are separate pull requests.
- **A fix shows its failure on `main`.** Before opening a `fix` pull request, reproduce the fault on a plain checkout of this repository's `main`, with a test or a command that fails there and passes with the change. A tree or host built from anything more than `main` (a local overlay, unmerged patches, another branch) can behave differently from `main`; a fault that does not appear on `main` is not fixed here, and describing it here misstates what `main` does.

## Gates

Every CI check is a script you can run locally before you push. The ones that apply to most changes:

| Change touches | Run |
|---|---|
| anything | `ops/ci/check-no-identity-spillage.sh` · `ops/ci/check-no-live-secret.sh` — no keys, addresses or credential values in a committed file |
| Rust (`crates/`) | `cd crates && cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace` |
| Go (`ops/*/go.mod`) | `go vet ./... && go test ./...` in that module |
| Ansible | `ops/ci/check-role-template-refs-resolve.sh` · `ansible-playbook --syntax-check` on the playbook you changed |
| Shell | `shellcheck` · `ops/ci/check-bash-portability.sh` (scripts must run on bash 3.2) |
| Docs | `ops/ci/check-doc-links.py` · `ops/ci/check-doc-code-citations.sh` — every `path:line` citation must point at an existing line · `ops/ci/check-doc-signer-identity.py` — every documented cosign identity is a release signer's |
| An ADR | add its row to `docs/adr/INDEX.md` in the same change; ADRs are never rewritten, they are superseded |

The pull-request workflow runs the same scripts. A red gate names the script, so the fix is reproducible locally.

## Design documents

A change to a component's behaviour, data flow, topology or integration point updates that component's design document under `docs/design/` **in the same pull request**. Each design document states, per part, whether it is *built*, *designed* or *not built*, and the status words mean exactly that:

| Word | Means |
|---|---|
| built | the code exists and runs |
| measured | a command ran and produced the stated result |
| designed | decided, not implemented |
| not built | nothing exists |

A load-bearing decision — one a later contributor would need to know the reason for — is an ADR under `docs/adr/`, with a plain-language summary at the top.

## Review

Reviewers look for: the change does what its subject says and nothing else; the gates pass; the design document moved with the code; status words are honest; no identifier of a real deployment (an address, a hostname, a key, a person) entered the tree. Review comments are about the change, not the author.

## Outside contributions

A required check, `contributor-trust`, asks who wrote each pull request ([ADR 1221](docs/adr/1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md)):

| Author | Security review needed |
|---|---|
| Maintainer (organisation member or collaborator) | No |
| Known: registered in a goat community environment with a linked GitHub account, or with an earlier merged pull request here | Only if the change touches a path in [`.github/contributor-trust/sensitive-paths.txt`](.github/contributor-trust/sensitive-paths.txt) |
| Anyone else | Yes |

When a review is needed, an owner reads the change against this list and, if it holds, applies the `security-reviewed` label:

- the change does only what its title says;
- it adds no dependency, network call, binary, generated blob or obfuscated code that the description does not explain;
- every change to a workflow, build file or pin is read line by line;
- tests exercise the change;
- a change to authentication, access control or cryptography also gets a second, independent reviewer.

A new push removes the label. The approval that merges the pull request comes from a different owner than the one who applied it. If you are registered in a community environment, link your GitHub account there and your pull requests take the faster path.

### What you can expect from the maintainers

- **Once you are validated, the maintainers aim to reply within one working day.** This is an aim, not a guarantee. You are validated when you are registered in a goat community environment with a linked GitHub account, or have an earlier merged pull request here: the "Known" row in the table above. Validation takes work from both sides and has no fixed time. The reply is in words: who has it, and what happens next.
- **Your pull request is labeled `community`.** The same check that asks who wrote a pull request applies the label to every one from outside the maintainers. Maintainers work that list first: [open community pull requests](https://github.com/usareur-af/goatcore/pulls?q=is%3Apr+is%3Aopen+label%3Acommunity).
- **Maintainers carry the rebase.** `main` moves many times a day. A pull request does not have to be up to date with `main` to merge. When yours conflicts, a maintainer rebases it; leave "Allow edits from maintainers" on so they can push to your branch.
- **Your work is not replaced without a word.** If a maintainer's pull request carries the same change, it says so in yours before yours is closed, and credits your commits.

The decision is ADR 1231 (`docs/adr/`).

## Merging

Development happens in this repository. A pull request merged to `main` is the change; there is no other upstream. A deployment picks it up by moving the commit it pins, in its own overlay repository.

`main` changes many times a day. To build on goat, pin a whole-tree release, the tag `goatcore/vX.Y.Z`, or follow the branch `stable`, which always points at the newest one. `stable` takes no pull requests: every contribution goes to `main` ([the design](docs/design/community-and-stable-line.md)).

## Reporting a bug or proposing a feature

Open an issue with the template. For a vulnerability, **do not open an issue** — follow [SECURITY.md](SECURITY.md).

## Licence

Portions written by U.S. Federal government employees within the scope of their employment are not subject to copyright in the U.S.; in countries where copyright is recognized, they and all other contributions are licensed under the [Apache License 2.0](LICENSE). See [INTENT.md](INTENT.md). Contributions are accepted under the [Policy](#policy) below.

**NOTE: This CONTRIBUTING.md is for software contributions. You do not need to follow the Developer's Certificate of Origin (DCO) process for commenting on this repository's documentation, such as CONTRIBUTING.md, INTENT.md, etc. or for submitting issues.**

## Policy

### 1. Introduction

The project maintainer for this Project will only accept contributions using the Developer's Certificate of Origin 1.1 located at [developercertificate.org](https://developercertificate.org) ("DCO"). The DCO is a legally binding statement asserting that you are the creator of your contribution, or that you otherwise have the authority to distribute the contribution, and that you are intentionally making the contribution available under the license associated with the Project ("License").

### 2. Developer Certificate of Origin Process

Before submitting contributing code to this repository for the first time, you'll need to sign a Developer Certificate of Origin (DCO) (see below). To agree to the DCO, add a `Signed-off-by` line to every commit with `git commit -s`. At a high level, signing off tells us that you have the right to submit the work you're contributing and indicates that you consent to our treating the contribution in a way consistent with the license associated with this software (as described in [LICENSE](LICENSE) and [INTENT.md](INTENT.md)) and its documentation ("Project").

### 3. Important Points

Pseudonymous or anonymous contributions are permissible, but you must be reachable at the email address provided in the Signed-off-by line.

If your contribution is significant, you are also welcome to add your name and copyright date to the source file header.

U.S. Federal law prevents the government from accepting gratuitous services unless certain conditions are met. By submitting a pull request, you acknowledge that your services are offered without expectation of payment and that you expressly waive any future pay claims against the U.S. Federal government related to your contribution.

If you are a U.S. Federal government employee and use a `*.mil` or `*.gov` email address, we interpret your Signed-off-by to mean that the contribution was created in whole or in part by you and that your contribution is not subject to copyright protections.

### 4. DCO Text

The full text of the DCO is included below and is available online at [developercertificate.org](https://developercertificate.org):

```txt
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.
1 Letterman Drive
Suite D4700
San Francisco, CA, 94129

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.

Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

# Security policy

goat handles identity, authentication and encrypted transit at the boundary between edge and cloud. Security reports are prioritised over feature work.

## Reporting a vulnerability

**Do not open a public issue or pull request to report a vulnerability.**

**[Report a vulnerability privately](https://github.com/USAREUR-AF/goatcore/security/advisories/new)**

You can also open the reporting form from this repository's *Security* tab by selecting *Report a vulnerability*.

Reports are private and accessible to repository administrators, organization owners, security managers, the reporter, and collaborators explicitly added to the advisory. Being listed in `CODEOWNERS` does not itself grant access. Reports remain private until an authorized maintainer publishes the advisory. See [GitHub's advisory permissions](https://docs.github.com/en/code-security/reference/permissions/repository-security-advisory) for details.

Include: a description of the issue and its impact; reproduction steps or a proof of concept; the affected versions, commits or deployment surfaces; whether you have shared it elsewhere.

## What to expect

| Step | Target |
|---|---|
| Acknowledgement | 3 working days |
| Triage and severity | 7 working days |
| Fix or mitigation for a confirmed high or critical issue | 30 days, with a coordinated disclosure date agreed with you |
| Public advisory | when the fix is released, crediting you unless you prefer otherwise |

## Scope

In scope: the code in this repository and the images and release artifacts built from it. Out of scope: the upstream projects goat assembles (NetBird, Zenoh, Ory, SPIRE, Postgres, WireGuard) — report those to their maintainers, and tell us too if goat's integration is affected; any deployment you do not own.

## Supported versions

Pre-1.0, the latest tagged release of each component is supported. Fixes land on `main` and are released by tag.

## Advance notice for operators of goat deployments

Organisations that run goat deployments can apply to receive advance notice of a vulnerability before its public advisory. A registered operator receives, privately, the advisory text, the fix as a patch, and fixed images and binaries built from it, so its deployments are patched by the time the advisory is published.

**When it is used.** Only for a vulnerability in this repository's own code that is not already public or easily discoverable, and only once a fix exists. A vulnerability in a dependency that already has a public advisory is handled in the open, through the normal dependency update.

**Timing.** The advance-notice window sits inside the coordinated disclosure date agreed with the reporter and never moves that date. It is 14 days by default, may be extended once by up to 14 days, and always ends at the agreed disclosure date or earlier. The advisory is published at once, ending the window, if the vulnerability is found being exploited, becomes public, or is disclosed by anyone who received notice.

**What a recipient may do during the window.** Deploy the fix to the deployments it operates, and share the notice with the people who operate them. It may not discuss the vulnerability publicly, publish the patch or the fixed artifacts, or share them outside its own organisation.

**How to apply.** Open a report through the [private reporting form](https://github.com/USAREUR-AF/goatcore/security/advisories/new) with the title `Advance notice application`, naming your organisation, a security contact (name, e-mail address and GitHub account), and the goat deployments you operate. Admission is the maintainers' decision, made per organisation, and is reviewed at least once a year. Admitted contacts are added to a private repository, from which notices, patches and artifacts are distributed.

# ADR 1252 — goatbuilder trusts a declared list of release signers, and a signer change ships first in a release that trusts the old signer and the new one

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Repo scope | goat core (`goatbuilder`) |
| Amends | [ADR 1228](1228-goatbuilder-keeps-its-four-surfaces-and-removes-only-in-a-major-release.md) (adds a fifth promise: an installed 1.x can always update to the next release) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) (goatbuilder and its deploy contract), design [`goatbuilder-distribution.md`](../design/goatbuilder-distribution.md) (the update and its signature check), design [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) ("What 1.0 promises", row E3) |

## TL;DR (for a layperson)

goatbuilder updates itself, and before it installs a new copy it checks who
signed it. Until now, a copy trusted only the repository it was itself built
in. When the project moved to a new repository, every copy built before the
move refused every release made after it, and the only way out was to
reinstall by hand. This record says that each copy carries a written list of
the signers it trusts, and that when the signer changes, one release first
trusts both the old signer and the new one before any release is signed by
the new one alone. That way every install can always reach the next release
with a plain update.

## Context

`goatbuilder update` verifies the keyless signature over a release's
`SHA256SUMS` against an identity: the GitHub repository whose
`goatbuilder-release.yml` signed it, at the release's tag
(`crates/goatbuilder/src/update/signature.rs`). Which repository that is came
from the checkout's `repo.release_source` (else `repo.source`, where the
deployment's declarations live) in `ops/goatbuilder/goatbuilder.yaml`, and, outside a checkout, from the binary's
own build provenance: `GITHUB_REPOSITORY` at build time. A configured signer
replaced the provenance rather than adding to it.

The provenance made the trust follow the build, not a decision. The
repository releases came from before the move signed every release up to the
move. After it, releases are signed by `USAREUR-AF/goatcore`. Measured on
19 November 2026: goatbuilder 0.4.2 refuses 0.7.3 served by a site's get
surface with GB-074, because the subject is `USAREUR-AF/goatcore`'s
workflow and the binary expects the previous release repository. Those
installs cannot be fixed in place: the trust they carry is compiled into
them. A second move would strand every 1.x the same way, which breaks the
promise of ADR 1228 in the one place an operator cannot work around it by
reading release notes.

The operator ordered the fix on 19 November 2026: the trust is a declared,
compiled-in list of release signers, not the binary's own provenance, and a
checkout's `repo.release_source` stays an addition, never the only anchor.

The rule "One build for every deployment" (AGENTS.md) keeps every
deployment's values out of the binary, repositories included. The release
signer is not a deployment's value: it is goatbuilder's own publisher, the
same for every deployment that runs the binary. Naming it in the code is
what makes the trust a decision someone reviews. AGENTS.md states this
exception beside the rule.

## Decision

### The declared signers

- goatbuilder carries a compiled-in list of release signers, `RELEASE_SIGNERS`
  in `crates/goatbuilder/src/update.rs`. Each entry is a repository
  (`<owner>/<repo>`), the release workflow that is triggered (its file under
  `.github/workflows/` and its `name:`), and the reusable workflow it calls,
  which signs. Today the list is one entry: `USAREUR-AF/goatcore`,
  `goatbuilder-release.yml` named `goatbuilder-release`, and
  `_release-binary.yml`.
- The first entry signs the releases built from this tree. Any entry after it
  is a successor already decided, trusted before it signs anything.
- `goatbuilder update` accepts a release whose signature verifies for any
  trusted signer. When none does, it refuses (GB-261), and the message names
  each signer it checked and why each one refused.
- The binary's own build provenance is never a signer.

### The certificate fields matched

A signature verifies for a signer only when the Fulcio certificate carries
each of these, at the served version's tag `refs/tags/goatbuilder/v<version>`
(sigstore/fulcio `docs/oid-info.md`). The declared entry's field each one
matches is named beside it.

| Certificate field | Must be | Entry field |
|---|---|---|
| OIDC issuer (1.3.6.1.4.1.57264.1.8) | `https://token.actions.githubusercontent.com` | none: every entry |
| Source repository URI (1.3.6.1.4.1.57264.1.12) | `https://github.com/<repo>` | repository |
| Source repository ref (1.3.6.1.4.1.57264.1.14) | the tag | none: the served version |
| Build Config URI (1.3.6.1.4.1.57264.1.18) | `https://github.com/<repo>/.github/workflows/<release workflow file>@<tag>` | repository, release workflow file |
| Subject alternative name | `https://github.com/<repo>/.github/workflows/<reusable workflow file>@<tag>` | repository, reusable workflow file |
| GitHub workflow repository (1.3.6.1.4.1.57264.1.5), when present | `<repo>` | repository |
| GitHub workflow ref (1.3.6.1.4.1.57264.1.6), when present | the tag | none: the served version |
| GitHub workflow name (1.3.6.1.4.1.57264.1.4), when present | the release workflow's `name:` | release workflow name |

On 0.7.3's certificate these are: the SAN
`https://github.com/USAREUR-AF/goatcore/.github/workflows/_release-binary.yml@refs/tags/goatbuilder/v0.7.3`,
the Build Config URI
`https://github.com/USAREUR-AF/goatcore/.github/workflows/goatbuilder-release.yml@refs/tags/goatbuilder/v0.7.3`,
workflow repository `USAREUR-AF/goatcore`, workflow ref
`refs/tags/goatbuilder/v0.7.3` and workflow name `goatbuilder-release`.

Repository names match without case, as GitHub matches them. goat's release
calls the same reusable workflow, so the SAN alone cannot tell the two
releases apart; the Build Config URI and the workflow name do. A reinstall
line that verifies with cosign pins the SAN as `--certificate-identity`, and
the workflow repository, ref and name with
`--certificate-github-workflow-repository`, `-ref` and `-name`: the same
definition the binary checks. The last three are Fulcio's legacy
extensions, which it marks deprecated. The binary matches them whenever the
certificate carries them, and does not require them, so a Fulcio that stops
writing them does not strand an install; the source repository, source ref,
Build Config URI and SAN still decide.

### The configured signer is an addition

The configured source, a checkout's `repo.release_source`, else its
`repo.source`, is always an addition beside the declared signers, never
instead of them, with the same workflows as the declared ones. A blank
value adds nothing, and a value that names a declared signer adds no second
entry. What an omitted `repo.release_source` means is unchanged. A
deployment that installs goatbuilder from another publisher, a fork for
example, names it there.

### The overlap rule

A change of release signer, whether a repository move, a rename or a new
release workflow, follows these steps:

1. A release adds the new signer to `RELEASE_SIGNERS` after the current one,
   and is signed by the current one. Every install trusts the current signer,
   so every install can update to it.
2. Only a later release is signed by the new signer. It keeps the old signer
   in its list or removes it.
3. An entry is removed only when no supported install still needs it to
   reach a later release.

Step 1 cannot be skipped. A release signed by the new signer alone is
refused by every install that does not yet trust that signer, and that
install can then leave its version only by a reinstall by hand. A release
gate for row E3 is to check this before a release is published, with each
supported installed version running `goatbuilder update` against the
candidate (designed, not built).

### Amendment to ADR 1228

ADR 1228's four surfaces are what other people build on. This adds what an
operator relies on when nothing else is in reach: from 1.0, an installed
1.x updates to every later 1.x release with a plain `goatbuilder update`.
The signer is never changed in a way that breaks that, and the overlap rule
is how a signer change keeps it. "What 1.0 promises" in
`docs/design/goatbuilder-v1-gate.md` states it.

### Installs from before the move

A release cannot change the trust compiled into an earlier binary. Installs
whose signer is the previous release repository are reached by one verified
reinstall, which GB-074's catalog entry and the get surface give. That bridge
is separate from this decision and is not built by it.

## Alternatives considered

**Keep the build provenance and fix the configuration.** Every deployment
names `repo.release_source`, and the provenance stays the fallback. This
leaves the trust with whichever repository last built the binary, so the next
move strands every install run outside a checkout again, and it puts a value
that is the same for every deployment into each deployment's configuration.

**Trust any repository whose `goatbuilder-release.yml` signed.** This removes
the move problem by removing the check: any GitHub repository can run a
workflow with that name, so the signature would say nothing about who
published the binary.

**A signed list of signers fetched at update time.** The list could change
without a release, but an island site and a binary that cannot reach the
list would need a fallback, and that fallback is a compiled-in list again.
It also adds a second key to protect. A compiled-in list changes only in a
reviewed release, which is the right speed for this decision.

**Move the signer only in a major release.** That still strands every install
that skips the one release that trusts both, and nothing forces a deployment
to take a major release. The overlap rule works at any release.

## Consequences

**Positive.** The trust in a release is a list someone reviewed, read from
the code, not a property of the build. A repository move is a planned change
in two releases, which the E3 release gate is to check before publish. Every release
built from this tree, from this change on, updates without a checkout.

**Negative.** A move must be planned at least one release ahead, and a fork
must name its own publisher in `repo.release_source` (or `repo.source`), as
before. A configured source widens trust beside the declared signers; it no
longer narrows it. Installs from before
the move stay stranded and need the reinstall bridge. Removing an old signer
needs care: an install that skipped the releases that trust its successor is
stranded by that removal.

## Assessor view (SCA-R)

This is supply-chain integrity for the deployment tool (NIST SP 800-53 SI-7,
software integrity, and SR-4, provenance; SP 800-171 3.14). Trust in an
update is a reviewed list in source control, changed only by a reviewed
release, with the keyless signature checked in process against Sigstore's
trusted root. The configured addition widens trust only to a repository the
deployment's own reviewed configuration names. The residual risk is a
compromise of a listed repository's release workflow, the same risk as
before; the compensating control is the signature identity check (issuer,
repository, tag and workflow), Rekor's log entry, and the release gate.

## Trigger conditions for revisit

- The release workflow moves to a new repository or a new workflow file: the
  overlap rule applies, and this record names the release that carries both.
- An install is found that cannot update to the next release with a plain
  `goatbuilder update`.
- goatbuilder's releases are signed by something other than a GitHub Actions
  workflow, which would change what an entry in the list names.

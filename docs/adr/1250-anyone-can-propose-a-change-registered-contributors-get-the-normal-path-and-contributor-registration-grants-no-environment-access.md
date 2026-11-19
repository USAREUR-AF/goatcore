# ADR 1250 — Anyone can propose a change; registered contributors get the normal path, and contributor registration grants no access to any environment

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-17 |
| Deciders | operator |
| Repo scope | Multi: every public repository of the organization, through the required `contributor-trust` check |
| Amends | [ADR 1221](1221-outside-contributions-need-a-known-author-or-a-recorded-security-review.md) (the author tiers and how an author becomes known) |
| Supersedes | — |
| Superseded by | — |
| Related | [`CONTRIBUTING.md`](../../CONTRIBUTING.md) ("Outside contributions"), [`SECURITY.md`](../../SECURITY.md) (advance notice), [ADR 1231](1231-one-development-branch-and-a-stable-line-that-follows-releases.md) (the `community` label) |

## TL;DR (for a layperson)

Anyone can still propose a change to goat. A change from someone the project does not know is treated as untrusted: it gets
more checks and waits longer before it can merge. Someone who wants the normal path registers, which means introducing
themselves, proving who they are, and getting a registered member who has worked with them to vouch for them. Registering is
quick and never required. It concerns trust in a person's code only: it gives no access to any environment where goat is
deployed, which keeps its own onboarding.

## Context

ADR 1221 made a required check, `contributor-trust`, ask who wrote each pull request. An author counts as known when they are
registered in a goat community environment with a linked GitHub account, or when they have one merged pull request in the
organization. Three gaps remain:

- **One merged pull request makes an author known, permanently.** ADR 1221 already calls it the weaker signal: a contributor can
  build trust with harmless changes before sending a harmful one. That is the pattern of the 2024 xz Utils backdoor, where an
  account contributed for about two years while other accounts pressed the overloaded maintainer to add it
  ([OpenSSF and OpenJS alert](https://openjsf.org/blog/openssf-openjs-alert-social-engineering-takeovers)).
- **Registration is tied to deployment environments.** Today's registration is onboarding to an environment where goat runs and
  where its users share data. Trust in someone's code and access to a live environment are different questions; one person can
  deserve the first and not the second.
- **Nothing records who vouched for a person, how their identity was checked, or when the trust ends.**

Other projects solved the person problem with sponsorship. Kubernetes grants organization membership only with two sponsors
who are reviewers or approvers, from different member companies, who have worked with the applicant
([community-membership.md](https://github.com/kubernetes/community/blob/master/community-membership.md)). Debian requires an
existing developer to advocate for an applicant and an identity check that is not self-declared
([Developer's Reference, ch. 2](https://www.debian.org/doc/developers-reference/new-maintainer.html)).

## Decision

- **Two registries, kept apart.**
  - The **contributor registry** answers "how much do we trust this person's code". It grants standing on pull requests only.
    It runs on an **open collaboration environment** the maintainers host for anyone, including people outside the allies and
    partners who use goat's deployment environments. That environment is separate from the deployment environments and from the
    community space that supports them, and carries only public goat material: the registry, introductions, contributor
    discussion and public documentation. Nothing about the alliance, a deployment environment or a partner is posted there.
  - **Environment onboarding** answers "may this person reach a live environment". Each environment keeps its own onboarding
    and policy.
  - The link runs one way: a person onboarded to an environment with a linked GitHub account counts as registered. Contributor
    registration never counts toward environment access. Advance notice under `SECURITY.md` stays with people who operate goat
    deployments.
- **Four standings.**

  | Standing | How a person gets it | Gates on each pull request | Earliest merge |
  |---|---|---|---|
  | Outside | Nothing needed | CI runs after an owner approves; security review by one owner, merge by a second; automated scan report; no changes to sensitive paths | Seven days after the last push |
  | Registered | Introduction, proven identity, linked GitHub account, one sponsor | Security review only when sensitive paths change (ADR 1221) | When reviewed |
  | Vouched | Registered, plus two sponsors from two different organizations | As registered | When reviewed |
  | Maintainer | Vouched, six months of contributions, approval by two platform admins | Normal review | When reviewed |

  A merged pull request no longer makes an author known by itself.
- **Registration, five steps, aimed at five working days, never blocking.**
  1. **Introduce:** a short public post in the community introductions space: name, organization, what they want to work on,
     who on the team they know.
  2. **Prove identity:** sign in to the contributor registry. Strongest: a government or national identity card through the
     registry's certificate sign-in. Acceptable: a verified work e-mail at a registered organization. Otherwise: a video call
     with the sponsor.
  3. **Link GitHub:** the person names an account, the registry invites it, and accepting proves control. A typed username
     alone proves nothing.
  4. **Sponsor vouches:** a registered member who has worked with the person says how, in a sentence or two. The sponsor's name
     is on the record, and vouching for someone who abuses it is reviewed.
  5. **Recorded:** person, verification method, sponsor, organization, affiliation category, date and a one-year expiry. The trust roster that
     `contributor-trust` reads is generated from these records; offboarding removes the entry.

  While registration is pending the person contributes through the outside lane.
- **Affiliation category.** Each record carries one category: `vendor:<organization>` for a person contributing for a company
  that sells a product goat integrates with or competes with, `government`, `academic`, `individual`, or `unspecified`.
  `unspecified` is a normal value: the project will not always know, and a person is never refused standing for leaving it
  open. The category does not change a standing or its gates. It is used for two things:
  - **Conflict of interest.** A pull request from `vendor:<organization>` that changes the integration of a different named
    vendor (for example another vendor's adapter package) needs a security review by an owner, as a sensitive path does. Each
    repository lists which paths belong to which vendor's integration beside `sensitive-paths.txt`.
  - **Visibility.** Pull requests from vendor contributors carry the label `vendor` beside `community`, so maintainers can see
    commercial contributions at a glance.
  A sponsor states the category when vouching; the person can correct it, and a change is recorded with its date.
- **Rules for every standing.** A change to a sensitive path (`.github/contributor-trust/sensitive-paths.txt`) needs two
  maintainers, one of whom did not write it. Maintainer status is never granted to relieve workload. Pressure from little-known
  accounts to add a maintainer, merge faster or take over a component is reported to the platform admins. Release signing and
  publishing stay with maintainers.
- **Nationality.** The contributor registry records it. Whether it limits contributor standing, and any sanctions screening, is a
  decision for the project's leadership and counsel; this ADR provides the verified identity such a rule needs and does not
  decide the rule.

## Alternatives considered

- **Keep merged pull requests as a path to trust.** Rejected: it is the slow-build pattern of the xz backdoor.
- **Register contributors in the deployment environments.** Rejected: it ties code trust to environment access and would grant
  network onboarding to people who only need to send a pull request.
- **Require registration before any pull request.** Rejected: it blocks good-faith first contributions; the outside lane keeps
  them possible with more scrutiny.

## Consequences

- Outside contributors wait longer and cannot change sensitive paths directly; a maintainer re-writes a good idea and credits it.
- The maintainers run and staff a registry and a sponsor process; sponsors take on accountability.
- Authors known today only through a merged pull request are asked to register, within a transition period the owners set.
- Vendor engineers can contribute on the same terms as anyone else, and a change one vendor makes to another vendor's integration always gets an owner's review.

## Designed, not built

the open collaboration environment, `contributor-trust` reading standing, expiry and affiliation category from registry records, the per-repository vendor-path lists and the `vendor` label, the outside-lane gates (no sensitive paths, seven-day
wait, CI approval, scan report), the registry with introduction, identity sign-in, sponsor vouch and generated roster, roster
removal on offboarding, and the `CONTRIBUTING.md` text. The security review, two-owner merge and label removal on push are built
(ADR 1221).

## Trigger conditions for revisit

- Registration takes longer than five working days for more than a few people, or good contributors stop at the outside lane.
- Leadership decides a nationality or sanctions rule.
- A sponsored contributor abuses their standing.

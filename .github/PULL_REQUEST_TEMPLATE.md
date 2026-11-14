<!--
PR title MUST follow Conventional Commits:
  <type>(<scope>): <subject>
Allowed types: feat, fix, chore, docs, refactor, test, perf, ci, build, revert, adr.

Scope rules (recurring trip-up):
  - Single scope, lowercase only: a-z 0-9 . / _ -
  - No commas, no uppercase. Finding/block IDs go in the subject.
  - Multi-finding closure: `docs(findings): F-074 + F-076 → Resolved`

The PR title becomes the squash-merge commit message on `main`.
See CONTRIBUTING.md and docs/adr/0109-git-collaboration-trunk-based.md.
-->

## What

<!-- 1–3 sentences: what changed and why. The "why" matters more than the "what" — diffs already show the what. -->

## Block-completion review (ADR 1015)

<!--
For block-shaped PRs (closes a Block in docs/project/implementation-plan.md):
link to your committed Tier-1 review file. The review carries the
long-form evidence; this PR description is the short-form.

For non-block PRs (typo fixes, lockstep dep bumps, single-issue patches):
mark "N/A — non-block change" and skip the classification + coverage
fields below.
-->

- [ ] Block-shaped — Tier-1 review file: `docs/project/block-XXXX-completion-review.md`
- [ ] Non-block change — N/A; skip classification + coverage fields.

### Change classification (Block 1015 §B Row 3 — block-shaped PRs only)

- [ ] **Functional** (changes happy-path behavior visible to users / operators)
- [ ] **Edge-case hardening** (timeouts / panic recovery / leak detection)
- [ ] **Error-surfacing** (logs / returns errors that were previously swallowed)
- [ ] **Test-only** (no production-path code)
- [ ] **Doc-only** (no code change)

### Test coverage (ADR 1015 §B Row 4 — block-shaped PRs only)

<!-- Functional changes need a unit (or integration) test that
exercises the behavior change. Edge-case hardening has at minimum
lint + build + goleak green. Cite test file paths + verdicts here or
in the Tier-1 review file linked above. -->

-

### Known gaps (ADR 1015 §B Row 5 — block-shaped PRs only)

<!-- Anything that can't be tested or addressed in this PR. Each gap
needs an `F-NNN` finding or follow-on Block pointer. State "none" if no
gaps. -->

- None

## Test plan

<!--
Specific, re-runnable steps a reviewer can use to verify.
Examples:
- "Ran `make test` locally; all green."
- "Re-bootstrapped integdev; smoke phase 8 passes."
- "Verified mesh peer enrollment from a fresh Orin in <2 min."
Avoid "tested locally" with no specifics.
-->

## Cross-repo coordination

<!--
Required if this change spans repos. Pick one and link sibling PRs.

- [ ] Sequenced merge — producer first, consumer follows. Sibling PRs:
- [ ] Atomic via feature flag — flag name + flip plan:
- [ ] Coordinated-release tag — tag name `coord/<feature>-<date>`:
- [ ] Single-repo change — N/A.

See docs/adr/1005-monorepo-separation-map.md and CONTRIBUTING.md
"Cross-repo coordination" for the patterns.
-->

## Documentation

<!-- Tick every box that applies; explain any unticked architectural box. -->

- [ ] No architectural change (skip the rest of this section).
- [ ] Updated relevant doc(s) in `docs/design/`.
- [ ] Updated implementation tracker in `docs/project/`.
- [ ] Added/updated an ADR in `docs/adr/` (and added a row to `docs/adr/INDEX.md`).
- [ ] Updated `CLAUDE.md` pointers if a source-of-truth moved.
- [ ] Grepped for stale references to renamed paths/components/hostnames — no drift.

## Checklist

- [ ] Commits are signed (`git log --show-signature` is clean).
- [ ] Every commit has a `Signed-off-by:` trailer (DCO).
- [ ] Every commit has a `[track: <name>]` trailer if part of an active-work track (ADR 0006).
- [ ] PR title follows Conventional Commits.
- [ ] Used `git commit <path>` form for staging (not `git add` + `git commit`) — concurrent-staging window bypassed (ADR 1015 §D Row 12).
- [ ] CI is green (or I have a tracked plan to fix what's red).
- [ ] CODEOWNERS auto-requested reviewers — I haven't bypassed.

## Security-sensitive?

<!--
If this PR touches any of:
  - crypto / TLS / WireGuard / Rosenpass
  - enrollment / setup keys / offline-CA
  - policy engine / egress / identity
  - FIPS-PQC boundary node
  - audit-log integrity tier

…then this is a security-sensitive PR. CODEOWNERS will require TWO
approvals. Confirm:
-->

- [ ] Not security-sensitive — single review is fine.
- [ ] Security-sensitive — two approvals required, both reviewers acknowledged the change rationale.

## Notes for reviewers

<!-- Anything specific you want eyes on — known unknowns, deferred follow-ups, areas you're unsure about. -->

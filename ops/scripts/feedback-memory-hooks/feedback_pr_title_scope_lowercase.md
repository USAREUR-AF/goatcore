---
name: PR titles use single lowercase scope; finding/block IDs go in the subject
description: When opening a PR for goat, the title must be Conventional Commits with a single lowercase scope (a-z 0-9 . / _ -). Multi-finding or multi-block closures put the IDs in the subject, not the scope. The validator (`.github/workflows/pr-hygiene.yml`) rejects uppercase letters and commas in scope, and sessions have repeatedly hit this — bake the convention in.
type: feedback
---

# PR titles — single lowercase scope; IDs in subject

## Rule

PR titles for `usareur-af/goatcore` (and the goat-org sibling repos
that share the same title validator) MUST be:

```
<type>(<scope>): <subject>
```

with:

- **Type** ∈ `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`, `ci`, `build`, `revert`, `adr`.
- **Scope** matches `[a-z0-9._/-]+`. Lowercase only. **No commas, no uppercase, single scope.**
- **Subject** is a short imperative-mood description, ≤99 chars after `: `, no trailing period.

**Why:** the validator regex (CI gate, fails the PR) is strict on the scope alphabet. Finding IDs (`F-074`) and ADR numbers are uppercase-canonical in the codebase, so the natural temptation is to put them in the scope — which fails.

**How to apply:** for multi-ID closures, scope is the *kind* of change, IDs go in the subject:

| Wrong (validator fails)                                                  | Right                                                              |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `feat(Block-22K,22L): turnkey bringup`                                   | `feat(block-22): 22K + 22L turnkey bringup`                        |
| `adr(0013,0017): amendments after pilot data`                            | `adr: amend 0013 + 0017 after pilot data`                          |

## Established convention from the repo (look at recent merged PRs)

- `docs(findings): F-112 + F-113 + F-108 → Resolved` (PR #15)
- `fix(f-107): full closeout for Block 50I Tier-C` (PR #18; scope is the dominant ID, additional context in subject)
- `feat(block-48q): pr-readiness gate + cleanup-on-release + drift telemetry` (PR #8)

The pattern is: **one scope token, lowercase, descriptive of the kind of work** — and everything else in the subject.

## When in doubt

- Use the kind-of-change as scope: `findings`, `block-22`, `traefik`, `mgmt-host`, `adr`, `pr-template`, `ci`.
- Don't try to encode every ID; that's what the subject + body + commit-message-trailers are for.
- If you author a PR title and the `Conventional Commits` check fails, read the validator's error message — it now explicitly calls out the scope rules + shows a multi-finding example.

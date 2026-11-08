---
name: adr
description: "Propose an architecture decision record (ADR) in goatcore: claim a number, write the file from the template, add its INDEX.md row in the same change, and supersede or amend rather than rewrite. Use BEFORE creating any file under docs/adr/, whenever a decision needs recording, whenever asked for the next free ADR number, and whenever an existing decision is changing. Names the CI checks that gate ADRs so each one can be run locally first."
allowed-tools: Bash, Read, Grep, Glob, Edit, Write
audience: both
---

# Proposing an ADR in goatcore

Canon: [`docs/adr/0001-adr-template.md`](../../../docs/adr/0001-adr-template.md) (required
sections), [`docs/adr/0002-adr-lifecycle.md`](../../../docs/adr/0002-adr-lifecycle.md)
(statuses and transitions), [`docs/adr/INDEX.md`](../../../docs/adr/INDEX.md) (the register
and its "Authoring discipline" section). The template to copy is
[`docs/adr/TEMPLATE.md`](../../../docs/adr/TEMPLATE.md).

## 1. Claim a number

ADRs are `docs/adr/NNNN-<kebab-slug>.md`, four digits. New decisions take the next number
above the highest in use (`ls docs/adr | sort | tail`). Gaps inside a band are reserved:
never fill or renumber them, and never reuse a number, even a retracted one.

```bash
ops/scripts/next-id.sh            # next free ADR across every remote-tracking ref
ops/scripts/next-id.sh --fetch    # fetch first, so open branches count
```

`ops/scripts/next-id.sh` runs `next-id` from a dots checkout through
`ops/scripts/dots-tool`; set `DOTS_DIR` to a checkout, or `DOTS_REPO_URL` to clone one at
the commit in `ops/scripts/dots.ref`. Without dots, take the highest number on `main`
plus one, and check open pull requests: the docs-guards check refuses a number another
open PR already claims.

## 2. Write the file

Copy `docs/adr/TEMPLATE.md`. The header table carries Status, Date, Repo scope,
Supersedes, Superseded by and Related; an ADR that changes part of an earlier one adds an
`Amends` row naming it (see `docs/adr/1220-*.md`). Sections, in order: TL;DR (for a
layperson), Context, Decision, Alternatives considered, Consequences, Assessor view
(SCA-R), Trigger conditions for revisit.

- **Status leads with one of four tokens:** `Proposed`, `Accepted`, `Superseded`,
  `Deprecated`. Prose may follow (`Accepted — built in ops/…`); build state never replaces
  the token.
- A new ADR is `Proposed` until the change that implements it lands; it becomes
  `Accepted` in that pull request.
- Examples name the synthetic goatnet `zagros`, documentation addresses (RFC 5737) and
  `example.org` mail. No real deployment's names, hosts, addresses or people.

## 3. Add the INDEX row in the same change

Every ADR has exactly one row in `docs/adr/INDEX.md`, under its series heading (new
1000-and-up numbers go at the end of the 1000-series table):

```markdown
| 1221 | [One-sentence decision](1221-one-sentence-decision.md) | Proposed | |
```

Columns: `#` · Title · Status · Repo scope · Diagram (`📊` when the ADR embeds a
diagram). The Status cell must agree with the ADR's own Status line.

## 4. Change a decision by superseding or amending, never by rewriting

An accepted ADR is not edited except for typo fixes and its `Superseded by` field.

- **Replacing a decision:** write a new ADR with `Supersedes | ADR-NNNN`; set the old
  ADR's `Superseded by` field and its Status to `Superseded`; update both INDEX rows.
- **Changing part of one:** write a new ADR with an `Amends` row, and note the amendment
  in the old ADR's INDEX Status cell (`Accepted; amended by 1220 (…)`).

## 5. Run the gates before pushing

All run in `.github/workflows/docs-guards.yml`; each runs locally:

| Script | Refuses |
|---|---|
| `ops/ci/check-adr-number-uniqueness.sh` | two ADR files with one number |
| `ops/ci/check-adr-index-complete.sh` | an ADR with no INDEX row |
| `ops/ci/check-adr-index-status-agrees.sh` | an INDEX Status that disagrees with the ADR |
| `python3 ops/ci/check-adr-status-vocabulary.py docs/adr` | a Status that does not lead with an ADR 0002 token |
| `ops/ci/check-adr-status-vs-code.sh` | a Status that says "not built" when the code is in the tree |
| `python3 ops/ci/check-cross-pr-id-uniqueness.py` | a number claimed by another open PR (needs `GITHUB_TOKEN` and `PR_NUMBER`; CI only) |
| `python3 ops/ci/check-doc-links.py` | a relative `.md` link that does not resolve |
| `ops/ci/check-doc-code-citations.sh` | a `path:line` citation past the end of its file |

The leak gate (`.github/workflows/leak-gate.yml`) also reads every added line.

A change that alters behaviour also updates its `docs/design/` document in the same pull
request, with the status words built, measured, designed and not built.

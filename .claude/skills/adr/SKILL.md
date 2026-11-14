---
name: adr
description: "Propose an architecture decision record (ADR) in goatcore: claim a number, write the file from the template, render its INDEX.md row with the renderer, and supersede or amend rather than rewrite. Use BEFORE creating any file under docs/adr/, whenever a decision needs recording, whenever asked for the next free ADR number, and whenever an existing decision is changing. Names the CI checks that gate ADRs so each one can be run locally first."
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

If `next-id.sh` cannot run, take the highest number on `main` plus one, and check open
pull requests: the docs-guards check refuses a number another open PR already claims.

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

## 3. Render the INDEX row; do not write it

Every ADR has exactly one row in `docs/adr/INDEX.md`. The row of an ADR above 1234 is
rendered from the ADR file itself. Write the ADR's header as `docs/adr/TEMPLATE.md`
gives it, then run the renderer and commit both files:

```bash
python3 ops/release/adr_index.py write
```

The renderer reads the number from the file name, the title from the first line
(`# ADR NNNN — <title>`), the Status from the header table's `| Status | ... |` row,
verbatim, and the ADR numbers in its `| Amends | ... |` and `| Supersedes | ... |`
rows. A Status written as a `**Status:**` line or in YAML front matter is refused;
use the table row. Never write or edit a row in the rendered table by hand:
`adr_index.py check` refuses it.

The rows of ADR 1234 and below are kept by hand, as they always were.

## 4. Change a decision by superseding or amending, never by rewriting

An accepted ADR is not edited except for typo fixes and its `Superseded by` field.

- **Replacing a decision:** write a new ADR with `Supersedes | ADR-NNNN`; set the old
  ADR's `Superseded by` field and its Status to `Superseded`. Run the renderer. If the
  old ADR is 1234 or below, change its hand-kept INDEX row's Status to match; if it is
  above 1234, the renderer does it.
- **Changing part of one:** write a new ADR with an `Amends` row. The renderer puts
  `Amends NNNN` in the new ADR's row; the old ADR's row does not change.

## 5. Run the gates before pushing

All run in `.github/workflows/docs-guards.yml`; each runs locally:

| Script | Refuses |
|---|---|
| `python3 ops/release/adr_index.py --self-test`, then `check` | an ADR above 1234 the renderer cannot read, or a rendered row that differs from its ADR |
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

# adr — human guide

> The human doorway to the `adr` skill. It points at the canonical source and does not restate it.

## When you're here

You are recording a decision in goatcore, or changing one that is already recorded.

## Where the real instructions live

- [docs/adr/0001-adr-template.md](../../../docs/adr/0001-adr-template.md): the sections every ADR carries; copy [docs/adr/TEMPLATE.md](../../../docs/adr/TEMPLATE.md).
- [docs/adr/0002-adr-lifecycle.md](../../../docs/adr/0002-adr-lifecycle.md): Proposed, Accepted, Superseded, Deprecated, and why accepted ADRs are superseded rather than edited.
- [docs/adr/INDEX.md](../../../docs/adr/INDEX.md): the register. A new ADR's row is rendered from the ADR file: run `python3 ops/release/adr_index.py write` and commit the result with the ADR.

## The agent's version

The `SKILL.md` beside this file is the same procedure written for an agent, with the number-claiming commands and the CI checks that gate an ADR.

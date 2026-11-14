---
heading: For someone contributing
title: Changed: a changelog entry is a file of its own, and a new ADR's index row is rendered
---

A pull request adds its goatbuilder changelog entry as a file in
`crates/goatbuilder/changelog.d/` instead of editing `CHANGELOG.md`;
`ops/release/changelog.py` renders the Unreleased section and folds the
entries into a release. The index row of an ADR above 1234 is
rendered from the ADR's own header by `ops/release/adr_index.py`, so adding
an ADR edits no shared file. `docs-guards` checks both (#623, #636).

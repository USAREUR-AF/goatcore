---
heading: For someone contributing
title: Added: every whole configuration example in the docs is validated
---

`crates/goatbuilder/tests/doc_configs.rs` runs every `yaml` block under
`docs/` that holds `schema: goatbuilder/v1` through this build's
`goatbuilder validate`. An example meant to be refused says so, with its
code, on its first line. The two examples that their own release refused,
in the configuration reference and tutorial 00, are fixed, and the test
runs on every pull request that touches goatbuilder or a document (#568).

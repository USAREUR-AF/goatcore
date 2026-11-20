---
heading: For someone contributing
title: Changed: the compatibility check is held to goatbuilder/v0.7.3
---

`ops/ci/check-goatbuilder-compat.sh` now compares each build with
`goatbuilder/v0.7.3`, the last published release: its command surface and its
kept `-o json` documents. The upgrade tests' list pins the four published
binaries of v0.7.2 and v0.7.3 (`crates/goatbuilder/tests/upgrade/releases.sha256`),
and the configuration files of both are kept for gate row A2 with the outcome
each release's published binary gives them.

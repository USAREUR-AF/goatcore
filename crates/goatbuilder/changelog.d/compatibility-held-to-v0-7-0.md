---
heading: For someone contributing
title: Changed: the compatibility check is held to goatbuilder/v0.7.0
---

`ops/ci/check-goatbuilder-compat.sh` now compares each build with
`goatbuilder/v0.7.0`, the last published release: its command surface and its
kept `-o json` documents. The upgrade tests' list pins v0.7.0's four
published binaries (`crates/goatbuilder/tests/upgrade/releases.sha256`), and
v0.7.0's configuration files are kept for gate row A2 with the outcome its
published binary gives each.

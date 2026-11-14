---
heading: For someone contributing
title: Changed: the compatibility check holds more of the last release, now goatbuilder/v0.6.0
---

`ops/ci/check-goatbuilder-compat.sh` compares each build with the last
published release, now `goatbuilder/v0.6.0`. It keeps `-o json` documents
for the verbs that change state (`deploy`, `rollback`, `plan`, `standup`,
`teardown`) and for run journals, and holds a field that only some entries
of a list carry from a reviewed list (`conditional.yaml`). A kept command's
exit code changes only with a reason in `exit_changed`, and a value a flag
accepted in the last release is still accepted. The published binaries'
checksums are one list, `crates/goatbuilder/tests/upgrade/releases.sha256`,
which the upgrade tests own and the check only reads, and it holds
v0.6.0's and v0.6.1's binaries. The configuration files of both are kept
for gate row A2 beside those of the earlier releases (#566, #617, #562,
#563, #631, #634, #616, #674, #675).

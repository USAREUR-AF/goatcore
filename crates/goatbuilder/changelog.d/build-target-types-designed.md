---
heading: For someone contributing
title: "Designed: every machine kind becomes a goatbuilder build target type before 1.0"
---

ADR 1257 (Proposed) and `docs/design/goatbuilder-build-targets.md` design one
file per machine kind under `ops/goatbuilder/targets/`, read by `standup`,
`join-node` and the image tooling. It replaces `join-node`'s compiled class
list, `standup`'s per-machine flags and the image builder's roles. They also
design `goatbuilder doctor --target <type>`, which checks a host's end state
however it was installed, and `goatbuilder image <type>`, an optional golden
path on known hardware.

The 1.0 gate gains group J: a CI-tier row for every type, and a hardware-tier
row for core, obs, breakglass, network-jump and the federation boundary node.
Nothing is built yet, and no command changes in this release.

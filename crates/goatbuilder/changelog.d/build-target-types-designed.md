---
heading: For someone contributing
title: "Designed: platform families, build target types and per-machine sensors, before 1.0"
---

ADR 1257 (Proposed) and `docs/design/goatbuilder-build-targets.md` declare two
things as data:

- platform families, which hold hardware and its preparation, and where the
  golden path lives; a model joins a family;
- target types, a machine's role with its plays and acceptance, read by
  `standup`, `join-node` and the image tooling.

A machine's declaration lists its wired sensors, and on any type that allows
sensors goatbuilder turns on one relay process per sensor. The design also
covers `goatbuilder doctor --target <type>`, which checks a host's end state
however it was installed, `goatbuilder image <model>` as an optional golden
path, and standup phase 11 without a manual step.

The 1.0 gate gains group J, in a seven-step build order: a CI-tier row for
every type, and a hardware-tier row for core, obs, break-glass, the network
jump box and the federation boundary. Nothing is built yet, and no command
changes in this release.

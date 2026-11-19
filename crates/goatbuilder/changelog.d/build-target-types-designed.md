---
heading: For someone contributing
title: "Designed: platforms, build target types and payloads for every machine kind, before 1.0"
---

ADR 1257 (Proposed) and `docs/design/goatbuilder-build-targets.md` keep three
things apart, each declared as data:

- the hardware platform and its preparation, where the golden path lives;
- the target type, a machine's role with its plays and acceptance, read by
  `standup`, `join-node` and the image tooling;
- payloads, software deployed onto the types that accept them.

They also design `goatbuilder doctor --target <type>`, which checks a host's
end state however it was installed, `goatbuilder image <platform>` as an
optional golden path, and standup phase 11 without a manual step.

The 1.0 gate gains group J: a CI-tier row for every type and payload, and a
hardware-tier row for core, obs, breakglass, network-jump and the federation
boundary node. Nothing is built yet, and no command changes in this release.

---
heading: For someone contributing
title: "Designed: platform families and build target types for every machine kind, before 1.0"
---

ADR 1257 (Proposed) and `docs/design/goatbuilder-build-targets.md` declare two
things as data:

- platform families, which hold hardware and its preparation, and where the
  golden path lives; a model joins a family;
- target types, a machine's role with its plays and acceptance, read by
  `standup`, `join-node` and the image tooling. The types are core, obs,
  break-glass, the network-gear jump box, octo-pod, the sensor node, the
  vendor gateway, first-bite, the federation and simple boundaries,
  goat-processors and the phone.

A sensor node runs the sensor kit's relays for the sensors declared on it. A
site's network gear is not built by goatbuilder; it is reached only through the
network-gear jump box. The design also covers `goatbuilder doctor --target
<type>`, which checks a host's end state however it was installed,
`goatbuilder image <model>` as an optional golden path, and standup phase 11
without a manual step.

The 1.0 gate gains group J: one foundation, then four lanes in parallel (host
site, boundary, processing, small computer), with a CI-tier row for every type
and hardware-tier rows where there is equipment. Nothing is built yet, and no
command changes in this release.

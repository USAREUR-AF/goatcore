---
heading: For someone deploying
title: Changed: join-node plans a network-jump machine's phases 11 to 14 at its declared address, not the mesh
---

Nothing may reach the network-gear jump box over the mesh (ADR 1131), so
`join-node --class network-jump` now finishes it at the LAN or out-of-band
address its site declares in the type's own inventory:
`<machine> ansible_host=<address>` under `[<site>_network_jump_hosts]` in
`inventory/<site>-network-jump.ini`. The plays of phases 11 and 13 are given
that inventory last, phase 12 checks the machine is declared there and not at
its mesh address, and phase 14 connects there. A run that reaches phase 11
without a usable address, or with an `--address` that names another, is
refused at its first phase, before the machine changes (GB-432). A type file
says this with `reach: declared`; `mesh` is the default. The class still
cannot enrol until it declares its own mesh groups (GB-430).

---
heading: For someone deploying
title: "Fixed: join-node reaches the node at its mesh address from phase 12 on"
---

Phase 14 of `goatbuilder join-node` unmasks the apt timers over ssh. It used
`--address`, the node's LAN address, while every play from phase 12 on already
reached the node at the mesh address phase 10 records. A node whose LAN address
no longer answered after enrolment stopped the run at phase 14. Phase 14 now
takes the node's address from the mesh inventory; `--address` is used only by
phases 1 to 11. Through an env door (`--via door`), which does not reach the
mesh, phase 14 still uses `--address`.

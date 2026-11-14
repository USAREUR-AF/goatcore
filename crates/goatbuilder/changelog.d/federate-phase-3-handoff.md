---
heading: For someone deploying
title: "Added: a two-driver peering hands phase 3 to the edge driver and joins its edge half"
---

On a peering declared `drivers: two`, the controller now stops at phase 3
with a hand-off object whose result is the edge half
(`handoff/3/edge-half.json`). On the edge driver, `goatbuilder federate
<peering> --side edge --from 3 --to 3 --expect-commit <commit>` measures the
edge half over SSH, reaching only the edge host, and prints the way back with
its confirm code. On the controller, `goatbuilder federate import <peering> 3
<file> --sha256 <code>` checks the file as the core half would (GB-114): the
code, the peering, the 24-hour bound, and the edge host the inventory names
(`--edge-group` for a group other than `<peering>_edge`). Then
`goatbuilder federate <peering> --from 3` hashes the file again and measures
the core half against it, reaching only the core host. A two-driver run from
phase 3 with `FED_MEASURE=here` is refused before it acts (GB-267). Phase 2
and phases 11 to 14 are not built yet (gate row B10).

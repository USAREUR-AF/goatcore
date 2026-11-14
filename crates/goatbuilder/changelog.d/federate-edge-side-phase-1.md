---
heading: For someone deploying
title: "Added: a two-driver peering's edge driver runs phase 1's edge checks and prints the way back"
---

On a peering declared `drivers: two`, the edge driver runs the command the
controller's phase 1 hand-off names: `goatbuilder federate <peering> --side
edge --from 1 --to 1 --expect-commit <commit>`. It refuses to run unless its
checkout is that commit with nothing changed, and each file given with
`--expect <file>=<sha256>` has that digest (GB-278). It then runs preflight's
edge checks into `<work>/handoff/1/edge.json` and prints the way back: the
confirm code, and the `import` and `then` commands for the controller. The
edge driver's run of phase 2 and later is not built yet (gate row B10).

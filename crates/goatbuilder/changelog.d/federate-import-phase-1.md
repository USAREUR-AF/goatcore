---
heading: For someone deploying
title: "Added: a two-driver peering's controller imports phase 1's edge result and runs its core half"
---

On a peering declared `drivers: two`, `goatbuilder federate import <peering> 1
<file> --sha256 <code>` takes in the edge driver's phase 1 result: it checks
the file against the confirm code the edge driver printed, places it under
`<work>/handoff/1/` and records its sha256 in the run journal. `goatbuilder
federate <peering> --from 1` then hashes the file again, runs phase 1's core
checks (preflight `--side core`), joins the two sides' results, and goes on to
phase 2's hand-off. A result that cannot be imported is GB-277. The import
of later phases is not built yet (gate row B10).

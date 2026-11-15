---
heading: For someone deploying
title: "Added: each timings row names its cause, and standup times phase 11 and sums the run"
---

Every row of the timings files that `goatbuilder standup`, `join-node` and
`teardown` write now has a sixth column, its cause: `ok` for a step that
passed, the error code a failed step's output matched, or `uncatalogued` when
no code covers it. When standup stops at phase 11 for the operator step it
writes an open row for it, and the run resumed with `--from 12` closes that
row with the time the step took. The resumed run's timing line then adds the
total since phase 1 across every resume, phase 11's time, the total without
it, and the run's stops. `ops/scripts/goat-standup.sh` writes the sixth column
too, as `ok` or `failed` (#805).

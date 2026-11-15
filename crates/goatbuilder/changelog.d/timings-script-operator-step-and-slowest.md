---
heading: For someone deploying
title: "Added: goat-standup.sh times phase 11, and standup's timing line names the slowest phases"
---

`ops/scripts/goat-standup.sh` now writes an open row to the timings file when
it stops at phase 11 for the operator step, and the run resumed with
`--from 12` closes it with the time the step took, as `goatbuilder standup`
does. A run driven by the script, or by both drivers in turn, now has phase
11's time in its record. `goatbuilder standup`'s timing line now ends with the
three slowest phases across every resume, phase 11 left out, each counted by
wall time, so steps run side by side count once (#814).

---
heading: For someone deploying
title: "Added: phase 14's acceptance can fire its probes on one machine and read the edge's trace from another"
---

For a peering driven from two machines, `acceptance.sh` takes
`FED_ACCEPTANCE_TRACE=deferred`: it fires every probe, reads no edge log,
leaves the five rows that need the edge's ACL trace pending, and writes the
ids it fired and their window to `<work>/acceptance/ids.json`. On the edge,
`ops/federation/standup/edge_trace_filter.py --ids … --window …` keeps only
the log lines that carry one of those ids, within the window and 5 minutes
either side. `FED_EDGE_TRACE_FILE=<that file>` then decides the pending rows
by id, fires nothing, and refuses a file for other ids or another window.
With neither set, phase 14 runs as before.

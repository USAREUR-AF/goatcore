---
heading: For someone deploying
title: Fixed: the last goatbuilder errors with no catalog code carry one
---

The last eight places in goatbuilder's source that raised an error with no
catalog code now raise one, so a log of a run that met one shows the code. The
federation verbs' context flags (`--context-dir`, `--cert`, `--ca-roots`,
`--context`, `--profile-dir`) and `--over-ssh` given to `goatbuilder federate
plan` or to a peering run print GB-239. `goatbuilder federate allocate
<peering>` naming a peering the config does not declare prints a new code,
GB-359. Both still exit 2. A key rotation's working files that cannot be
written print GB-254, still exit 2. A grown config that cannot be written as
YAML prints GB-322 and now exits 1, as every GB-322 does, where it exited 2:
a script that checks `goatbuilder grow`'s exit code sees the change. The check that holds the source to its list of uncoded errors now has
an empty list (#1093).

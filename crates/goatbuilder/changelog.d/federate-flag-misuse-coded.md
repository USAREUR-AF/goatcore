---
heading: For someone deploying
title: Fixed: a flag given to `federate plan` or a peering run that does not belong there carries its code
---

The six verbs' site flags (`--context-dir`, `--cert`, `--ca-roots`,
`--context`, `--profile-dir`) given to `goatbuilder federate plan` or to a
peering run are refused with GB-238, the code that already names them.
`init`'s `--over-ssh` and its host flags given there are refused with the
new code GB-336. Both still exit 2.

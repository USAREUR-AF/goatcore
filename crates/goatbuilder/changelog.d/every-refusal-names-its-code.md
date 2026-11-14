---
heading: For someone deploying
title: "Changed: every refusal names its catalog code, and an undeclared peering exits 1"
---

The refusals that printed no catalog code now print one, on the line after
the message (`(GB-NNN — goatbuilder explain GB-NNN)`): a command line goatbuilder
cannot parse (GB-242); a mutation without `--yes` and without a terminal for
`apply`, `standup`, `teardown`, `join-node` and `federate` (GB-008); a
federation verb given site flags before it (GB-238) or a flag it does not take
(GB-239); an unknown `FED_VERBS` (GB-240); `--mock` on a verb that reads the
checkout (GB-276, new); and a peering that is not declared (GB-149). The
undeclared peering now exits 1, refused, as GB-149 says and as `federate
allocate`'s same refusal already did; it exited 2 (gate row C5).

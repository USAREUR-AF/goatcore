---
heading: For someone deploying
title: "Fixed: every error `goatbuilder federate` raises carries its catalog code"
---

Fifty-one errors the federate stages raised had no catalog code, so a log of
a run that met one (stderr not a terminal, no explanation page) showed only
`goatbuilder: <text>`, with nothing to look up. Each now prints its code and
`goatbuilder explain <code>` answers it. They use existing codes where one fits
(an undeclared environment GB-241, a peering not declared GB-149, the working
folder GB-184, a declaration GB-144, a saved plan's inputs GB-200, host flags
GB-239, a confirmation with no terminal GB-008, a peering on several
environments GB-252), and six new ones: GB-320 a program the run starts could
not be started, GB-321 a file a step wrote is missing or not what it wrote,
GB-322 an internal error, GB-323 a file outside the working folder cannot be
written, GB-324 a setting in the environment has a value goatbuilder does not
take, GB-325 flags that do not go together. An error that now carries GB-144,
GB-149, GB-184 or GB-200 exits 1, as those codes do everywhere else; it exited
2 before. Every other exit is unchanged.

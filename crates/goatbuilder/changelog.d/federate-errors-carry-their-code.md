---
heading: For someone deploying
title: "Fixed: every error `goatbuilder federate` raises carries its catalog code"
---

Fifty errors the federate stages raised had no catalog code, so a log of
a run that met one (stderr not a terminal, no explanation page) showed only
`goatbuilder: <text>`, with nothing to look up. Each now prints its code and
`goatbuilder explain <code>` answers it. They use existing codes where one fits
(an undeclared environment GB-241, the working folder GB-184, a declaration GB-144, host flags GB-239, a confirmation with
no terminal GB-008, a peering on several environments GB-252), and six new
ones: GB-320 a program the run starts could
not be started, GB-321 a file a step wrote is missing or not what it wrote,
GB-322 an internal error, GB-323 a file outside the working folder cannot be
written, GB-324 a setting in the environment has a value goatbuilder does not
take, GB-325 flags that do not go together (a recheck given `--edge-group`
or `--as` other than the saved plan's among them).

A correction to the exit codes, by the contract's meanings (1 refused: a
precondition failed; 2 usage: the command line was wrong): these errors exited
2 and now exit 1, because neither is a usage error. `federate init` reading a
declaration it cannot read or parse (GB-144); a federate run that cannot write
its working folder: plan.json, the deploy baseline, phase 1's result folder or
an edge result's folder (GB-184).

The recheck given flags other than the saved plan's still exits 2. Every other
exit is unchanged.

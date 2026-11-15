---
heading: For someone deploying
title: "Fixed: every error `goatbuilder federate` raises carries its catalog code"
---

Fifty errors the federate stages and verbs raised had no catalog code, so a log
of a run that met one (stderr not a terminal, no explanation page) showed only
`goatbuilder: <text>`, with nothing to look up. Each now prints its code and
`goatbuilder explain <code>` answers it. They use existing codes where one fits
(an undeclared environment GB-241, the working folder GB-184, a declaration
GB-144, a flag for another verb GB-239, a peering on several environments
GB-252, a host's back-out survey answering for another peering GB-253, a verb
line the driver cannot parse GB-242), and eight new ones: GB-315 a program a
step ran gave output or an exit that cannot be used, GB-316 a confirmation could
not be read from the terminal, GB-320 a program the run starts could not be
started, GB-321 a file a step wrote is missing or not what it wrote, GB-322 an
internal error, GB-323 a file outside the working folder cannot be written,
GB-324 a setting in the environment has a value goatbuilder does not take, and
GB-325 flags given to a stage or verb that do not go together, or one without
the flag it needs (a recheck given `--edge-group` or `--as` other than the saved
plan's among them).

Exit codes, by the contract's meanings (1 refused: a precondition failed; 2
usage: the command line was wrong). Each change is from the last release:

- `goatbuilder federate <partner>` and its stages (plan, deploy, backout, rotate,
  status, import, the edge run): an error with no code exited 2. One that now
  carries GB-184, GB-315, GB-316, GB-320, GB-321, GB-322 or GB-323 exits 1,
  since none is a usage error. One that carries GB-241, GB-252, GB-253, GB-324
  or GB-325 still exits 2.
- The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
  and the phases that run them in process): an error with no code exited 1. A
  command-line error now exits 2, the contract's usage error: `--over-ssh` given
  to a verb other than init (GB-239); a host flag or `--side` on init without
  `--over-ssh`, `--side` without the files it reads or writes, and
  `init --over-ssh` without `-C` (GB-325). The others still exit 1 (GB-144,
  GB-322, GB-323), and a verb line the driver cannot parse still exits 2
  (GB-242).

`federate allocate <peering>` naming an undeclared peering is not coded yet and
still exits 2. Every other exit is unchanged.

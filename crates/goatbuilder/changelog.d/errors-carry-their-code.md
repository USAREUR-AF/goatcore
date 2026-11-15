---
heading: For someone deploying
title: "Fixed: the errors the rest of goatbuilder raises carry their catalog code"
---

Sixty-four more errors had no catalog code, so a log of a run that met one
(stderr not a terminal, no explanation page) showed only `goatbuilder: <text>`.
Each now prints its code. They use existing codes where one fits: a file
goatbuilder reads or writes GB-254 (a pins file, a run record, the survey page,
teardown.yaml), a program that could not be started GB-320 (bash, ansible, ssh,
git, go, a checkout's script), the mock converger's fixture GB-024, the config
GB-025, a plan file that is not JSON GB-052, a missing deploy playbook GB-005,
a component block fleet-pins.yaml lacks GB-257, the assembled tree's marker
GB-259, a release graduation that printed no record GB-020, a git command
`doctor` runs that failed GB-258, an internal error GB-322, and a mutation
without a terminal GB-008. One is new: GB-326, a file of the checkout that is
not YAML (fleet-pins.yaml, doors.yaml, a role's deploy-history.yml,
teardown.yaml). The errors of `grow`, which is not wired yet, are left to the
change that wires it.

Five checks inside `goatbuilder federate` now carry a code: phase 1's and the slot
recheck's exit disagreeing with its result GB-315, phase 1 writing no results and
an edge result naming the wrong side GB-321, and an imported edge result that
changed while it was read GB-277.

Eighty-four more errors were a file, program or JSON/YAML error passed up as it
came, with no code. Each now carries one: a file that cannot be read or written
GB-254 (it names the file), a value that cannot be written as JSON or YAML
GB-322, a program that could not be started GB-320, fleet-pins.yaml that is not
YAML GB-326, a mock fixture's pins GB-024, and the end-state grader's output
GB-066. One that carries GB-322, GB-320 or GB-066 exits 1, where it exited 2 (on
the federation verbs, an internal error already exited 1); the others still
exit 2.

Exit codes, by the contract's meanings (1 refused: a precondition failed; 2
usage: the command line was wrong). Each of these errors exited 2 with no code.
Every place here is on goatbuilder's own error path, where an uncoded error exits
2; none is on the federation verbs' path, where it exits 1. One that now carries
GB-020 (`release`), GB-025 (`validate` and every verb that loads the config: it
does not match the typed model, or does not convert to JSON), GB-258 (`doctor`'s
git, whose error every caller reads with a fallback), GB-277, GB-315, GB-320,
GB-321 or GB-322 exits 1, since none is a usage error. The others (GB-005,
GB-008, GB-024, GB-052, GB-254, GB-257, GB-259, GB-326) still exit 2. Every other exit is unchanged.

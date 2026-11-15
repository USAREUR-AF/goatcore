---
heading: For someone deploying
title: "Fixed: the errors the rest of goatbuilder raises carry their catalog code"
---

Sixty-seven more errors had no catalog code, so a log of a run that met one
(stderr not a terminal, no explanation page) showed only `goatbuilder: <text>`.
Each now prints its code. They use existing codes where one fits: a file
goatbuilder reads or writes GB-254 (a pins file, a run record, the survey page,
teardown.yaml), a program that could not be started GB-320 (bash, ansible, ssh,
git, go, a checkout's script), the mock converger's fixture GB-024, the config
GB-025, a plan file that is not JSON GB-052, a missing deploy playbook GB-005,
a component block fleet-pins.yaml lacks GB-257, the assembled tree's marker
GB-259, a release graduation that printed no record GB-020, a git command
`doctor` runs that failed GB-258, an internal error GB-322, and a mutation
without a terminal GB-008. Two are new: GB-326, a file of
the checkout that is not YAML (fleet-pins.yaml, doors.yaml, a role's
deploy-history.yml, teardown.yaml), and GB-327, growing a config that is not a
goatlite.

Five checks inside `goatbuilder federate` that compare a step's result with its
exit now carry GB-321.

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
One that now carries GB-020, GB-025 (the config does not match the typed model,
or does not convert to JSON), GB-258, GB-320 (a program could not be started),
GB-321 (the five federate checks) or GB-322 (an internal error) exits 1, since
none is a usage error. The others (GB-005, GB-008, GB-024, GB-052, GB-254,
GB-257, GB-259, GB-326, GB-327) still exit 2. Every other exit is unchanged.

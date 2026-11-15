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

Exit codes: an error that now carries GB-025 (the config does not match the
typed model, or does not convert to JSON) or GB-020 exits 1, as those codes do
everywhere else; it exited 2 before. Every other exit is unchanged.

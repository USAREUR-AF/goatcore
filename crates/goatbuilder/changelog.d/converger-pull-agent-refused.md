---
heading: For someone deploying
title: "Changed: an environment that declares converger: pull-agent is refused until pull is built"
---

No pull converger is built, yet `deploy` and the other verbs that reach real
hosts used to push to an environment declared `converger: pull-agent`, and
`doctor` passed it. Now `doctor` fails it in an `env:<name>:converger` row.
`deploy` (its dry run too), `release`'s deploy rungs, `fastlane`'s deploy and
`rollback` refuse it with GB-307 before any step. The refusal names each place
the value is declared: the environment, a site or a class. `deploy --verify-only`
still reads, `validate` still accepts the value, and `--mock` still converges
it on a fixture. Declare `converger: push-ansible` for those machines until
the pull converger is built.

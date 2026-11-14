---
heading: For someone deploying
title: Changed: a refusal names its error code
---

A refusal from `standup`, `join-node`, `release`, `rollback`, `pins`,
`doctor`, `promote`, `fastlane`, the fresh-source check, the converger,
`inspect` and the configuration loader now prints its `GB-nnn` code, so
`goatbuilder explain <code>` gives its causes and fixes. The error catalog gains
GB-238 to GB-260, GB-263 and GB-264 for refusals that had no code. A
refused `release` step ends its row with the code instead of dropping it,
and GB-008 names every verb that asks for `--yes`. A few refusals of the
command line itself and two of `federate`'s still print no code; they get
theirs in a later change (#560).

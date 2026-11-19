---
heading: For someone deploying
title: Fixed: a failed step's row prints its own catalog code
---

When a step stopped a run, its `failed` row named the phase and the exit
code but no catalog code. A drill log that kept the terminal output then
held an error without a code whenever the interpretation under it named
none, or named only another tool's code (X-ACCEPT-001 at phase 14). Each
row now prints its code: GB-183 for a `federate` phase, and five new codes:
GB-331 for a `standup` phase, GB-332 for a `join-node` phase, GB-333 for
standup's image pull, GB-334 for an observability leg and GB-335 for a
`teardown` stop. No exit code changes.

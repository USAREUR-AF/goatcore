---
heading: For someone deploying
title: "Fixed: the preflight's refusal of an unresolved secret reference has its own code, GB-368"
---

When `ops/scripts/ansible-preflight.sh` finds a variable whose value is still
an `op://`, `bao://` or `bw://` reference, it refuses with GB-368 and names the
variable. `goatbuilder explain` and a failed phase's `interpreted` row used to
read that refusal as X-ANSIBLE-005, the stale-checkout refusal, and suggest a
`git pull`; they now say to run the command through `ops/scripts/with-secrets`,
or to unset a variable the play does not need.

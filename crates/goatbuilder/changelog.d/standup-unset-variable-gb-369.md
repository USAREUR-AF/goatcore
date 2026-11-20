---
heading: For someone deploying
title: "Fixed: a standup phase that would read an unset variable refuses before it runs (GB-369)"
---

A standup phase line with no bootstrap administrator declared carries
`"$OWNER_EMAIL"`. With it unset, the phase stopped on bash's own
`unbound variable`. `ops/scripts/standup-phase-run`, which runs every standup
phase for both drivers, now checks each variable the line expands, after it
loads the secrets store, and refuses with GB-369, naming each one, before the
phase runs.

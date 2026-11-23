---
heading: For someone deploying
title: "Added: join-node checks a joining site's own copy of the seed play before phase 13 (GB-362)"
---

A site that keeps its own `ops/ansible/<site>-seed-from-peer.yml` has it
checked against the generic `site-seed-from-peer.yml` it was derived from, as
`goatbuilder standup` already did: `goatbuilder join-node` runs
`ops/scripts/check-site-play-current.py` as phase 13's first step at a joiner's
core node. A copy with no `# derived-from:` line, or one made from an older
generic play, stops the run there with GB-362, before any phase-13 play
changes the node. A site with no copy runs as before.

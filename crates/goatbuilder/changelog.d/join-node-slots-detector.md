---
heading: For someone deploying
title: "Added: join-node --slots lists a node's Spock slots and names the ones that cannot recover"
---

`goatbuilder join-node <env> --site <site> --node <node> --class <core|phone> --slots [--peer <site> ...]`
reads, in every database with a Spock node, each subscription the node holds
and each slot it keeps. It reads each slot's `wal_status` where the slot
lives: at the founder, at each `--peer`, or on the node. A slot that reads
`lost` (or `conflicting`) can never be read again. Its line says so and what
rebuilds it, and the run refuses with GB-387. It reads only and needs no
`--yes`. The script avenue is `ops/scripts/spock-reseed.py slots`, with
`--json`.

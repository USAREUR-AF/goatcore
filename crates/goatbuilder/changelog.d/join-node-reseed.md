---
heading: For someone deploying
title: "Added: `goatbuilder join-node ... --reseed` seeds a node again after its founder lost the node's slot"
---

A replicating node (class `core` or `phone`) away longer than its founder's
`max_slot_wal_keep_size` allows loses the slot the founder keeps for it, and
from then on receives nothing the founder writes. `goatbuilder join-node <env>
--site <site> --node <node> --class <class> --reseed` copies it again from the
founder in six steps, each one call of `ops/scripts/spock-reseed.py` over the
mesh: the slot is lost (GB-345), the founder has taken the node's own writes
(GB-346), the copy is well under the node's own slot cap (GB-347), the
subscription is dropped and created with no copy and each table copied again
after a local truncate, every table is copied with no slot lost (GB-348), and
every copied table is the same on both by count and digest (GB-349). The
container it reaches is `spock_mesh_postgres_container` (GB-350 when none is
set). Each step is journaled, a rerun starts at the step that stopped, and
`--dry-run` prints the six commands and runs nothing. The verb reads the
founder only; another site that lost its slot for the node is still re-seeded
by hand (docs/design/goatbuilder-hardware-measurement.md, "The re-seed in
goatbuilder").

---
heading: For someone deploying
title: "Fixed: join-node stops before phase 13 when a joining node or its founder declares no replication"
---

`join-node` phase 13 at a joining site wires Spock on the node and on the
founder, and those plays do their work only on a host that declares
`participates_in_spock`, an `inter_site_spock_topology` other than `none` and
`ory_spock_enabled`. On a founder without them, which a goatlite is until it
grows, the founder's half did nothing and phase 13 went on, so the joiner had no
Ory replication node to subscribe to. Phase 13 now reads those variables on both
sides first, changes nothing, and stops with GB-417 naming each missing value and
the group_vars it belongs in. Declare them, then resume with `--from 13`.

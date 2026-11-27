---
heading: For someone deploying
title: "Added: goat-relay and crutch-tier are opt-in components on their fixed cloud groups"
---

`ops/pins/fleet-pins.yaml` declares `goat-relay` (the Rust relay tier) and
`crutch-tier` (the public mTLS front) as opt-in components. Each is pinned to
the multi-arch digest its role pins. Their plays target a fixed group by name
(`goat_relay_hosts`, `crutch_tier`). A goatnet runs one by listing it under
`opt_in:` and listing that group among its members; `goatbuilder deploy` then
runs the play limited to that group. A deploy does not run goat-relay's
localhost play, which uploads an image archive for an offline install. Its
relays' archive-load step now runs only where that play ran, and otherwise the
role pulls the pinned image from the registry.

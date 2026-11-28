---
heading: For someone deploying
title: "Added: dataflows is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `dataflows`, goatstuff's
data-flow patterns surface, at the multi-arch index its role pins. In a tree
that layers goatstuff, a goatnet that lists it under `opt_in:` gets it with
`goatbuilder deploy dataflows` on its member groups. No other goatnet resolves,
deploys or verifies it. Opt-in is its only lock, because the role's
`dataflows_enabled` defaults to true.

---
heading: For someone deploying
title: "Added: gym is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `gym`, goatstuff's training
surface (ADR 1132). Its role names only the tag `nginx:1.27-alpine`, so the row
pins the index that tag resolves to. In a tree that layers goatstuff, a goatnet
that lists it under `opt_in:` gets it with `goatbuilder deploy gym` on its
member groups. The goatnet also sets `gym_enabled`, without which the play
refuses with the role's own message. No other goatnet resolves, deploys or
verifies it.

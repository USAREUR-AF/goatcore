---
heading: For someone deploying
title: "Added: isitup, goat-logship, surface-visit and node-exporter are opt-in components"
---

`ops/pins/fleet-pins.yaml` gains four opt-in components: `isitup`,
`goat-logship`, `surface-visit` and `node-exporter`. Each is pinned to the image
digest its role pins and deployed by its own `*-deploy.yml`. The
`node-exporter` role pinned a tag only; it now pins that tag's multi-arch digest
too. A goatnet that lists
one under its row's `opt_in:` runs it with `goatbuilder deploy <component>`
or `deploy-component.sh`; `verify` reads it back from the deploy ledger. A
goatnet that lists none resolves, deploys and verifies none of them, and its
resolved components do not change. The pins file's bytes change, so every
committed intent's `sources` hash moves and the intent is signed again, as on
an image bump.

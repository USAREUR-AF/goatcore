---
heading: For someone deploying
title: "Added: isitup, goat-logship and surface-visit are opt-in components"
---

`ops/pins/fleet-pins.yaml` gains three opt-in components: `isitup`,
`goat-logship` and `surface-visit`, each pinned to the image digest its role
pinned until now and deployed by its own `*-deploy.yml`. A goatnet that lists
one under its row's `opt_in:` runs it with `goatbuilder deploy <component>`
or `deploy-component.sh`; `verify` reads it back from the deploy ledger. A
goatnet that lists none resolves, deploys and verifies none of them, and its
resolved components do not change. The pins file's bytes change, so every
committed intent's `sources` hash moves and the intent is signed again, as on
an image bump.

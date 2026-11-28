---
heading: For someone deploying
title: "Added: goat-map is an opt-in component (gate J25)"
---

`ops/pins/fleet-pins.yaml` declares `goat-map`, the map surface, at the
multi-arch image index goatstuff built from its commit 08adeefb. It is opt-in:
a goatnet that lists it under `opt_in:` deploys it with `goatbuilder deploy
goat-map` (or `deploy-component.sh goat-map`) in a tree that layers goatstuff,
whose play and roles it runs. It runs on the one host `goat_map_host` names in
each member group, and verification expects its ledger row there only. A goatnet
that does not list it resolves, deploys and verifies no map. Its `sources` hash
moves, so every committed intent is signed again, as on an image bump.

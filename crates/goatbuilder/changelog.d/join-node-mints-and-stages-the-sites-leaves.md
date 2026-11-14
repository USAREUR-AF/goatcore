---
heading: For someone deploying
title: "Added: join-node mints and stages a joining site's leaves"
---

`goatbuilder join-node` now mints a joining site's router and service leaves
and stages them, in phase 13 before the data router. It runs
`mint-site-certs.sh <site>` from the site's manifest
(`ops/enrollment/manifests/<site>-certs.yaml`), which keeps any leaf already
minted, then `stage-service-leaves.yml` for the service leaves the manifest
carries, limited to the node. Both were hand steps after the command. A
site with no manifest is refused when the plan is made (GB-286), and so is a
manifest with no router leaf (GB-287).

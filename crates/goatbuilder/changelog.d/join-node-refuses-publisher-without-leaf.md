---
heading: For someone deploying
title: "Added: join-node refuses a health publisher with no leaf in the site's manifest before any phase runs (GB-364)"
---

A joining site's core node runs each health publisher the end-state model marks
present, with the leaf certificate phase 13 mints from
`ops/enrollment/manifests/<site>-certs.yaml`. A present publisher whose leaf the
manifest did not carry was found only when the staging play or the publisher's
role refused it, partway through phase 13. `goatbuilder join-node` now refuses
the plan before any phase runs, with GB-364, naming the publisher and the
manifest. A publisher the site declares absent needs no leaf.

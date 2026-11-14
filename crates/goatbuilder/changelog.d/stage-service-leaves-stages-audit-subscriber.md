---
heading: For someone deploying
title: "Fixed: the audit-subscriber leaf is staged with the other service leaves"
---

`stage-service-leaves.yml` now stages the audit-subscriber leaf and its key
into `/opt/audit-subscriber/certs`, where the audit-subscriber role reads
them, and `goatbuilder join-node` stages it in phase 13 when the site's
manifest carries it. Before, a site that declared its own audit-subscriber
copied the leaf by hand before `audit-subscriber-deploy.yml` could pass. The
role reads the CA root from the router's TLS directory, which the data
router's play fills, so the CA root is not staged.

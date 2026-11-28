---
heading: For someone deploying
title: "Fixed: join-node stops before phase 13 when a site does not replicate the NetBird management tables"
---

The NetBird management tables replicate between sites only where a site's
`spock_mesh_databases` names the netbird database pulling the `netbird_mgmt`
set (ADR 1084 D1). `join-node` seeds exactly those tables and checks them
before and after the Spock wiring, so a site that does not declare the set
would be seeded once and then drift from its founder. GB-417 now also refuses
a joining node or founder whose `spock_mesh_databases` lacks that entry, pulls
another set, or whose `netbird_mgmt` repset holds other tables than the seed's.

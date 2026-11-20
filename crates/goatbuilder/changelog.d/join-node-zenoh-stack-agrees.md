---
heading: For someone deploying
title: "Fixed: join-node refuses a joining site whose data router and health publishers would name its Zenoh stack differently (GB-361)"
---

`zenoh_stack` names a site's data router directories, its container and its
network. The router's role defaults it to `<site_id>-zenoh`, while each health
publisher's role defaults it to `goat-zenoh` (fabric-crawler requires it), and
Ansible gives each role only its own defaults. So a joining site whose
group_vars set no `zenoh_stack` got a router under one name and publishers that
looked for another. `goatbuilder join-node` now refuses such a site before any
phase runs, with GB-361, naming each publisher and the value it would read, and
the line to add: `zenoh_stack: <site_id>-zenoh` in `group_vars/<site>_hosts/`.
A site that sets it runs as before.

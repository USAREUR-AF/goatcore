---
heading: For someone deploying
title: "Changed: host-net-publish ends at once on a host whose switch is off"
---

`host-net-publish-deploy.yml` failed on a host where `host_net_publish_enabled`
was off, because its first task asserted the switch. It now ends at once there:
it says so in one line, changes nothing and writes no deploy-ledger row. With the switch on it runs
as before, and it still refuses a missing goatnet, ingest URL or bearer token.
A site that publishes keeps the switch on, as before. This lets a type phase
run the play on every core machine of a joining site.

---
heading: For someone deploying
title: "Added: join-node runs the decoy layer, the dead-man watcher and the Traefik compression exporter on a joining site's core machine"
---

The core type (`ops/goatbuilder/targets/core.yaml`) gains a phase that runs
`decoy-deploy.yml`, `deadman-watch-deploy.yml` and
`traefik-compression-metrics-deploy.yml` on the joining node of a joining site, as the health publishers are. Each follows the
switch the site's group_vars set: `deception_enabled`, `deadman_watch_enabled`,
`traefik_compression_metrics_enabled`. With the switch off, decoy and the
watcher end at once, and the exporter is taken down if it is there. A site that
runs none of them gets none of them. The founder's first machine is built by
`standup`, which does not read this phase; there these plays are still run by
hand, and so they are on a founder site's other core machine.

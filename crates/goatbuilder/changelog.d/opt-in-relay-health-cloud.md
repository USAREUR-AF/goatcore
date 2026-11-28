---
heading: For someone deploying
title: "Added: relay-health-cloud is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `relay-health-cloud`, the
relay-fleet health dashboard in standalone mode on the goatnet's cloud box. A
goatnet lists it under `opt_in:` and its `<site>_relay_health` group among its
members, and `goatbuilder deploy relay-health-cloud` runs
`relay-health-cloud-deploy.yml` there. The group's `relay_health_standalone`
picks the mode. A goatnet that lists both relay-health rows declares each
`absent` on the other's groups.

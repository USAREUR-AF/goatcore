---
heading: For someone deploying
title: "Added: relay-health-cloud is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `relay-health-cloud`, the
relay-fleet health dashboard in standalone mode on the goatnet's cloud box. A
goatnet lists it under `opt_in:` and its `<site>_relay_health` group among its
members, and `goatbuilder deploy relay-health-cloud` runs
`relay-health-cloud-deploy.yml` there. The group's `relay_health_standalone`
picks the mode. The row governs its own var, `relay_health_cloud_oauth2_proxy_image`, since a var belongs to one component, and the play hands it to the role as `relay_health_oauth2_proxy_image`. Each relay-health play now ends, with one line, a host of the
other's kind: `relay-health-cloud-deploy.yml` a host in no `<site>_relay_health`
group, and `relay-health-deploy.yml` a host in one. A goatnet can list both
rows.

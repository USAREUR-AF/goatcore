---
heading: For someone deploying
title: "Added: relay-health is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `relay-health`, the
relay-fleet health dashboard on a mgmt host, at the oauth2-proxy digest its role
pins. A goatnet that lists it under `opt_in:` gets it with
`goatbuilder deploy relay-health`, which runs `relay-health-deploy.yml` on the
goatnet's member groups. No other goatnet resolves, deploys or verifies it. The
play follows `relay_health_enabled`: with it off, the role takes the mesh
dashboard down and keeps the feeders the cloud dashboard reads.

---
heading: For someone deploying
title: "Added: compliance-proxy is an opt-in component"
---

`ops/pins/fleet-pins.yaml` gains an opt-in row for `compliance-proxy`, the mesh
front of a cloud compliance dashboard, at the oauth2-proxy digest its role pins.
A goatnet that lists it under `opt_in:` gets it with
`goatbuilder deploy compliance-proxy`, which runs
`compliance-proxy-deploy.yml` on the goatnet's member groups and verifies its
ledger row. No other goatnet resolves, deploys or verifies it. The play turns
the proxy on wherever it runs, and it needs the cloud function it fronts to
exist first. The Traefik route to it still follows the site's
`compliance_proxy_enabled`.

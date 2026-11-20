---
heading: For someone deploying
title: "Added: `goatbuilder join-node` runs a single-host joining site's observability stack"
---

At a joining site with no obs host of its own (no `has_obs_host: true`), the
core run's phase 13 now ends with `ops/ansible/obs.yml` on the node: it
deploys Prometheus, Alertmanager and the obs exporters. Grafana is left off
(`grafana_enabled=false`), because its OAuth2 client would be written to the
site's Hydra, a write to the identity store at a second site that gate rows I1
and I4 hold back; the plan names it under "does not run yet" with gate row I3.

The step needs `obs_colocated_site_zones` in the site's group_vars (its site_id
mapped to its DNS zone) and `<SITE>_POSTGRES_EXPORTER_PASS` in the environment.
Without the declaration the step is a SKIP that names it, and the four
services are named under "does not run yet". `obs.yml` itself no longer asks
for Grafana's two secrets when the site sets `grafana_enabled: false`.

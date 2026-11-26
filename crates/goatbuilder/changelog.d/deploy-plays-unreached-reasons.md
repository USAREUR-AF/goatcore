---
heading: For someone contributing
title: "Added: four deploy plays listed as unreached with their reason (gate J28)"
---

`ops/goatbuilder/deploy-plays.yaml` now lists four plays no goatbuilder verb
runs, each with why: `e2e-canary-secret-deploy.yml` is a test play the e2e
harness deploys from its own overlay, and `netbird-boot-selfheal-deploy.yml`,
`netbird-client-pin-deploy.yml` and `ntp-health-agent-deploy.yml` each re-run one
role that a play a verb runs already holds. `ops/ci/check-deploy-plays-reached.py`
reports a reason that names a play no verb reaches, or one that does not exist,
so a reason that points to another route cannot outlive that route. 18 gaps
remain.

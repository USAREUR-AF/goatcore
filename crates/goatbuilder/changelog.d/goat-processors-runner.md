---
heading: For someone deploying
title: "Added: the goat-processors runner deploys from its signed release (`goat-processors-runner-deploy.yml`)"
---

A goatnet that runs the goat-processors runner declares it in its own pins
table as a release component, as `ops/e2e/runner/goat-processors-runner.yaml`
shows, and sets `goat_processors_runner_config` in the group_vars of each
member group that runs it. `goatbuilder deploy` and `deploy-component.sh`
verify the release's Sigstore bundle, and the play installs the package by the
runner's deploy contract, up to its health check. With no configuration the
play stops before it installs anything (GB-395). The public pins table has no
row for it.

---
heading: For someone deploying
title: "Added: the goat-processors runner, an opt-in release component (`goat-processors-runner`)"
---

`ops/pins/fleet-pins.yaml` declares the goat-processors runner at 0.8.1 (its TLS fix release), opt-in:
a goatnet runs it only when its row lists `goat-processors-runner` under
`opt_in:`, and it then sets `goat_processors_runner_config` in the group_vars of
each member group that runs it. `goatbuilder deploy` and `deploy-component.sh`
verify the release's Sigstore bundle, and `goat-processors-runner-deploy.yml`
installs the package by the runner's deploy contract, up to its health check.
With no configuration the play stops before it installs anything (GB-395).
Every goatnet's intent is signed again after this change, as for any change of
the pins table; no goatnet's resolved components change.

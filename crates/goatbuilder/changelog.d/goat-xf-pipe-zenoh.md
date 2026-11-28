---
heading: For someone deploying
title: "Added: goat-xf-pipe-zenoh, an opt-in release component verified by its attestations (`goat-xf-pipe-zenoh`)"
---

`ops/pins/fleet-pins.yaml` declares goat-xf-pipe-zenoh at 0.1.6, opt-in: a goatnet
runs it only when its row lists `goat-xf-pipe-zenoh` under `opt_in:`, and it then
sets `goat_xf_pipe` (from_format, to_format, in, out, metrics) in the group_vars of
each member group that runs it. Each archive is verified by its own GitHub
attestation; `goat-xf-pipe-deploy.yml` unpacks it and runs it as a systemd service,
up to its health check. With no conversion the play stops before it installs
anything (GB-396). Every goatnet's intent is signed again after this change, as for
any change of the pins table; no goatnet's resolved components change.

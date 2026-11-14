---
heading: For someone deploying
title: Fixed: the docker-engine role checks the root the running containerd uses
---

The `docker-engine` role reads the root directory the running containerd uses
before it writes `/etc/containerd/config.toml`. When the two differ and
containers are running, the play fails and writes nothing; with no container
running, the role writes the file and restarts containerd in the same play.
A final check compares the running daemon's root with
`docker_engine_containerd_root`. Before, a restart the handler missed could
leave containerd on an empty root after its next restart, so `docker exec` and
every healthcheck answered `NotFound` while the services kept running. The
data-root migration runbook gives the check by hand (#664).

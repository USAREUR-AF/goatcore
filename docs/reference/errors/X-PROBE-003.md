# X-PROBE-003: a probe session did not answer, or the edge host did not say whether it runs

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-PROBE-003` is an error of `probe-launcher.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-PROBE-003` or `goatbuilder explain X-PROBE-003`.

## What it means

ops/federation/standup/probe-launcher.sh started a probe and its REST port did not answer within a minute, so the probe was removed and start exited 1; or, on alive, the edge host did not answer, so the launcher exited 2 rather than report the probe gone.

## Why it happens

- the probe container could not start: the image is not on the edge host, or its zenoh config was refused
- the probe's REST port is not reachable from where phase 14 runs (a firewall, or FED_PL_REST_IP is not an address of the edge host)
- ssh to the edge host failed (FED_PL_SSH, its key, or sudo)

## What to do

One line per step or alternative:

```text
ssh <FED_PL_SSH> sudo docker ps -a --filter label=goat-probe=1 (the probe containers and their state)
ssh <FED_PL_SSH> sudo docker logs <handle>
run phase 14 again; it stops an earlier run's probes first
```

## Read more

- [`docs/design/federation-acceptance.md`](../../../docs/design/federation-acceptance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

# X-PROBE-001: the probe launcher is missing a setting, or a setting is not well-formed

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-PROBE-001` is an error of `probe-launcher.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-PROBE-001` or `goatbuilder explain X-PROBE-001`.

## What it means

ops/federation/standup/probe-launcher.sh (phase 14's run-owned acceptance probes) refused before it touched the edge host: a setting it needs is not set, a value that goes into a command on the edge host is not of its form, or the call is not one of start, stop and alive with their arguments. The message names the setting.

## Why it happens

- FED_PL_SSH (user@address of the edge host) or FED_PL_LOCAL=1 is not set
- FED_PL_REST_IP, FED_PL_FAR_IP, FED_PL_DIR, FED_PL_PORT or FED_PL_PINS is not set, or an address is not one IPv4 address
- FED_PL_REST_IP is 0.0.0.0: the probe's REST port writes as the probe's identity, so it binds to one address
- FED_PL_DIR lacks &lt;kind>-cert.pem, &lt;kind>-key.pem or far-ca.pem for the kind asked
- a stop or alive handle is not one this launcher gave (goat-probe-&lt;kind>-&lt;run>)

## What to do

One line per step or alternative:

```text
set the setting the message names in the environment phase 14 runs in (the list is at the top of ops/federation/standup/probe-launcher.sh), then run phase 14 again
```

## Read more

- [`docs/design/federation-acceptance.md`](../../../docs/design/federation-acceptance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

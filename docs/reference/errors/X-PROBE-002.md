# X-PROBE-002: the edge host's pins carry no zenoh image pinned by digest

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-PROBE-002` is an error of `probe-launcher.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-PROBE-002` or `goatbuilder explain X-PROBE-002`.

## What it means

ops/federation/standup/probe-launcher.sh takes the probe's image from the edge host's group_vars pins.yml (FED_PL_PINS), its zenoh_router_image. The file cannot be read, or that value is not an image pinned by @sha256 digest, so no probe was started.

## Why it happens

- FED_PL_PINS names another file than the edge host's group_vars/&lt;group>/pins.yml
- the pins are not rendered yet, or zenoh_router_image is a tag or a template expression

## What to do

One line per step or alternative:

```text
goatbuilder pins (render every member's pins.yml from ops/pins/fleet-pins.yaml)
point FED_PL_PINS at the edge host's group_vars/<group>/pins.yml
```

## Read more

- [`ops/pins/fleet-pins.yaml`](../../../ops/pins/fleet-pins.yaml)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

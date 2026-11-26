---
heading: For someone deploying
title: "Added: an opt-in component runs only on a goatnet that lists it"
---

A component in `ops/pins/fleet-pins.yaml` may declare `opt_in: true`. It then
resolves only for a goatnet whose row lists it under `opt_in:`. For every other
goatnet it is absent: `goatbuilder pins` and `generate-pins.py` write no pin for
it, the goatnet's intent leaves it out, and `goatbuilder deploy`,
`deploy-component.sh` and `verify` refuse it there (GB-003). A new opt-in row
therefore changes no goatnet's resolved components. Like any edit to the pins
file, it moves each intent's `sources` hash, so intents are signed again as on
an image bump. A value deviation for a goatnet that has not opted in is refused,
and so is an `opt_in:` list naming a component that is not opt-in.

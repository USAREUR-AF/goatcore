---
heading: For someone deploying
title: "GB-384 says how to opt in when a type lists an opt-in service"
---

A target type whose `services:` names an opt-in component, on a goatnet that does
not list it under `opt_in:`, is refused before any phase with GB-384. Its help
now names that cause and its fix: list the component under the goatnet's
`opt_in:` in `ops/pins/fleet-pins.yaml`, then run `goatbuilder pins`.

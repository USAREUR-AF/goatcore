---
heading: For someone deploying
title: "Added: the `goat-processors` type, for a host-class machine"
---

`join-node --class goat-processors` builds a machine for the goat-processors
engines, on a host-class machine only. Its phase 13 names the release component
`goat-processors-runner`, the engines' runner, installed by its own play where
`ops/pins/fleet-pins.yaml` pins it for the machine's group. Where there is no
pin, phase 13 is a declared skip that says which pin is missing, and the machine
runs no engine. No release component or play for the runner ships yet, so today
every such machine is the skip. `standup` builds none.

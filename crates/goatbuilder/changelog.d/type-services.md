---
heading: For someone deploying
title: "Added: a target type lists its services, and `deploy --node` deploys to one machine of a group"
---

A type file may list `services:`, components of `ops/pins/fleet-pins.yaml`.
`join-node` phase 13 deploys each to the machine it joins, and
`doctor --target --live` reads each back. One the pins cannot supply is refused
before any phase (GB-384). No shipped type lists one yet.

`goatbuilder deploy --group <group> --node <machine>` and
`deploy-component.sh --group <group> --node <machine>` play on, and read back,
that machine alone, so a neighbour that is down does not stop it. The name must
be one host of the group in `inventory/mgmt-hosts.ini`; a group's name or a
host of another group is refused before any door opens or play runs.

The rollout preflight now grades a release component's declared `pin-revert` as
one; it read as not revertible, so every deploy of one asked for `--ack`.

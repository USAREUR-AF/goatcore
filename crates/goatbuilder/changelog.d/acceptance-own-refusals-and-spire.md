---
heading: For someone deploying
title: Added: in a bilateral session each side proves the partner's refusals and SPIRE trust on its own systems
---

Before, a bilateral acceptance session in which each side held only its own
core, edge and SPIRE server could not reach a combined PASS: the refusal rows
needed probe sessions holding the partner's identities, and
`spire/svid-chains` needed the partner's SPIRE server. Now
`FED_PROBE_LAUNCHER_OURS` names your own probe launcher: its sessions hold
your own identities and dial the partner's edge, and the run stages only
their zenoh ids beside this run's challenge, for the partner to judge on its
own edge's trace. With `FED_SPIRE_OURS` the run also stages your server's
bundle and one workload certificate, never a key, and checks the partner's
against the bundle your server holds. A staged value holding private key
material stops the run. `FED_PROBE_LAUNCHER_OURS` is refused outside a
session and beside `FED_PROBE_LAUNCHER`. Without it, the partner's refusal
rows stay BLOCKED and name you.

---
heading: For someone deploying
title: "Fixed: `teardown` says what each host had, stops the LAN surfaces, and names a refused stop's cause"
---

Each stop of `goatbuilder teardown` and `ops/scripts/goat-teardown.sh` now
asks the host whether the component is there at all: a compose project's
file or containers, a service's containers, a systemd unit. A component that
was never deployed on the host, such as a phase-12 service after a standup
paused at the browser step (phase 11), is reported as "nothing to stop" and
writes no `teardown.jsonl` row; before, each such stop wrote a row for a
service that had never run. The run ends with how many stops took a
component down and how many found nothing on the host.

`teardown --services` now also stops the LAN name server (`lan-dns`), the
LAN Traefik and the SPIRE agent, which it left running on a goatlite. STUN
(the coturn unit) is still left running, and says so.

A stop that fails names the catalogued error its output carries, in its
failure row, an `interpreted` row and the cause column of
`teardown-<site>-timings.tsv`. The playbook preflight's refusal of a stale
checkout is X-ANSIBLE-005, with the `git pull --ff-only` that fixes it; it
was recorded as "uncatalogued". GB-021, fastlane's refusal, no longer
matches every line containing "REFUSE —", so other tools' refusals are not
read as a fastlane refusal.

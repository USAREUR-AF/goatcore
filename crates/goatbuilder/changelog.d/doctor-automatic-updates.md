---
heading: For someone deploying
title: "Added: `doctor --target --live` reads whether a machine installs security updates by itself"
---

A type file may list `checks: [automatic-updates]`. `doctor --target <type>
--site <site> --live` then reads on each machine of the type, and changes
nothing: the `unattended-upgrades` package installed and its service enabled,
the `apt-daily` and `apt-daily-upgrade` timers enabled and running, and
`APT::Periodic` asking for both. Each machine gets one row, `automatic security
updates <machine>`, that names what is off; a fact it cannot read is UNKNOWN,
never a pass. Without `--live` the row says it is not checked. The `octo-pod`
type lists it.

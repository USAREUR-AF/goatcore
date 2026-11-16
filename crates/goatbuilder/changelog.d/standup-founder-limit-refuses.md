---
heading: For someone deploying
title: Fixed: a standup play limited to the founder is refused when its limit selects no host, instead of passing
---

Six standup plays run on the site's first management host alone, limited
with `-l 'mgmt-<site>-01*'`: SPIRE, the LAN service surface, the LAN TLS
routes, the dashboard, the portal and grant-keyd. When the site's founder was
named otherwise and another inventory host matched the pattern, ansible
skipped the play and exited 0, so the phase passed having done nothing. Each
of those commands now checks first that a host of `<site>_hosts` in the two
management inventories matches the limit, and refuses with GB-355, naming the
pattern, the group and the inventories, when none does. `goatbuilder standup`
and `ops/scripts/goat-standup.sh` run the same check. The naming rule is
unchanged.

---
heading: For someone deploying
title: Fixed: rollback reaches a goatnet through its door, as deploy does
---

`goatbuilder rollback --site <goatnet>`, on a site whose map is one host,
tries the host directly. When it does not answer, rollback goes through the
env door that `deploy` uses for that site (the netbird profile, the mesh
host on tcp/22, then the door registry). The connection still goes to the
ledger's own host and checks that host's key: the door only carries it,
so a door that lands on another host can never stand in for it. A door that
does not open falls back to direct ssh. Without `--site`, or for a site of
several hosts, nothing goes through a door. `ops/scripts/rollback.sh` now
reaches the ledger's host the same way, where it used to reach whatever
host the door landed on, and the live canary passes `--site` to both
rollback avenues and to `rollback --list` (#644).

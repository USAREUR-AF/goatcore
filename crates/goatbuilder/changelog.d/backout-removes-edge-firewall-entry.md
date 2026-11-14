---
heading: For someone deploying
title: "Fixed: backing a peering out removes its entry from the edge's host firewall"
---

`goatbuilder federate backout` left the backed-out peering's entry in the edge's
host firewall baseline (`/etc/goat-host-firewall/peerings/`), so its ports stayed
open. The back-out now removes that entry and no other. When another peering
stays on the edge, the ruleset is loaded again without it, through the role's
lockout check and timed rollback. When none stays, the baseline is unloaded and
removed, and the edge is as it was before its first peering. If the reload or
the unload fails, the entry is put back so the back-out can run again. A pending
rollback of an earlier run refuses (GB-214), and `--dry-run` only reports.

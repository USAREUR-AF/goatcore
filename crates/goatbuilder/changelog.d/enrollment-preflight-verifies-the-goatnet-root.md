---
heading: For someone deploying
title: "Fixed: join-node phase 9 verifies a LAN site's management against the goatnet's root"
---

The enrollment preflight checks the management URL and the clock from the new
node, and a fresh node trusts only the public roots, so on a LAN site, whose
management serves the goatnet's own certificates, both checks warned and phase 9
stopped the join. Phase 9 now gives the preflight the goatnet's root as PEM
(`NETBIRD_MANAGEMENT_CA_PEM`); only those two checks verify against it.

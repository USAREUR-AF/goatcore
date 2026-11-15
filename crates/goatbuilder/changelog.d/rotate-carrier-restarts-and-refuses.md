---
heading: For someone deploying
title: "Fixed: `federate rotate <partner> carrier` puts the new key on the carrier, refuses a peering with no carrier, and re-runs phase 3"
---

Three fixes to `goatbuilder federate rotate <partner> carrier`:
- The edge's carrier now restarts onto its new key. Before, the key file
  changed but the running carrier kept the old key, because its environment
  file names the key's path, not the key.
- A peering that declares no forwarder has no carrier. Rotating its carrier is
  now refused before anything runs (GB-189). Before, the run passed and changed
  no key.
- The rotation re-runs phase 3. After phase 11 the federation interface
  carries the agreed MTU, which is the lower of both offers. When the partner
  offered less than we did, phase 3 refused our own offer as larger than the
  interface. It now reports that cross-check as skipped and names the
  agreement.

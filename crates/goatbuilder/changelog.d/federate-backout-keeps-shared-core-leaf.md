---
heading: For someone deploying
title: Fixed: backing out one of two peerings keeps the core leaf the other still uses
---

Our core router serves one leaf to every peering. `goatbuilder federate
backout <partner>` no longer touches it while another peering is still set
up from this machine (its working folder beside this one holds an
`agreement.json`): the `core leaf` row reads `keep` and names that folder,
and the back-out removes everything else of the peering. When no other
peering is set up and nothing records what the leaf paths held before, the
back-out stops with GB-293, naming each path and the choice to make, instead
of stopping as GB-221 or showing the leaf as `absent` (#716).

---
heading: For someone deploying
title: "Fixed: a site whose Spock publisher host is a peer's is refused before anything is wired"
---

A joining site whose `signal_uri` names its founder, with no
`spock_mesh_publisher_host`, advertised the founder's own host as its Spock
publisher, so the founder's subscription pointed at itself and failed after the
joiner's own subscription was made. `spock-mesh-peering` now refuses such a
site with GB-441 before it wires anything and names the value to set.

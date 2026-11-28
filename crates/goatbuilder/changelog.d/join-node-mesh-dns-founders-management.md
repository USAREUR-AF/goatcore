---
heading: For someone deploying
title: "Fixed: join-node's mesh DNS step reaches a LAN goatlite founder's management"
---

join-node phase 13 writes the joining site's nameserver group through the
founder's management API, and it built that URL from the founder's zone. A LAN
goatlite's zone (`<name>.lan`) answers nothing: its management is
`netbird.<zone>`. The step now uses the founder's `public_mgmt_fqdn`, and the zone only when
the founder declares none.

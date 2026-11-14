---
heading: For someone deploying
title: "Added: join-node runs mesh DNS at a joining site"
---

A site that joins a goatnet now runs its own mesh DNS, so it still resolves
the goatnet's service names when it is cut off from its founder. Phase 13
of `goatbuilder join-node`, for a joining site's core node, runs a replica
of the goatnet's zone on the node and registers the site's own nameserver
group, `<site>-mesh-dns`, which lists the site's replica first and the
founder's second. The group is written through the founder's management
API, with the founder's token (`<FOUNDER>_NB_API_TOKEN`), resolved through
the secrets injector; an unset or unresolved token is refused before the
step (GB-281). A site with no zone file to serve is refused when the plan
is made (GB-280). See ADR 1235.

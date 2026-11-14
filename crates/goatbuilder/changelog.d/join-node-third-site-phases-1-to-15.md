---
heading: For someone deploying
title: Fixed: join-node adds a node at a third site, phases 1 to 15
---

Three refusals stopped `goatbuilder join-node` for a node not yet enrolled at
a goatnet that already has a joiner. Phases 3 and 7 refused the mesh-address
placeholder that phase 12 fills: they now run the config preflight with
`--before-enrolment`, which warns on that one placeholder and still fails on
any other. Pass 1 of phase 13 waited for a management service that needs
the Ory deployed by the next step: it now runs the stack plays only, as
`standup` does. A third site that declares signal federation off, with its
reason, was refused because its founder federates with another site: it now
passes, unless the founder's federation names it (#613).

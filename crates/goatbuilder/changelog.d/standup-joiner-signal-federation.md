---
heading: For someone deploying
title: "Changed: standup for a joiner site turns signal federation on"
---

`goatbuilder standup <env> --site <joiner>` now runs the joiner's signal
federation with its founder in phase 9, after the mesh enrolment and before
the data router. It uses the same steps `join-node` runs in its phase 13: a
gate on both sites' declarations, the token and proxy on the joiner and then
the founder, each node's federation router, the joiner's relay tunnel and
pin, and the delivery proof. A joiner that declares
`signal_federation_absent_reason` gets the gate and a SKIP line. Before this,
standup left signal federation off, and the operator ran those plays by hand.
`ops/scripts/goat-standup.sh` plans the same steps. A founder's plan is
unchanged. `goatbuilder teardown` refuses those steps by name, since they
leave state at the founder. (#824)

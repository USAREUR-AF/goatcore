---
heading: For someone deploying
title: "Changed: a joining site is declared to run the data router, admit-audit and SPIRE"
---

The end-state catalog declares the data router (zenoh-router) and admit-audit
at every site that replicates, a founder and a joiner alike, since
`goatbuilder join-node` builds both at a joining site's core node. A pin's
`absent: true` or the site's own `admit_audit_enabled: false` still decides.
SPIRE (spire, spire-agent) is declared at every site too. A joining site's own
SPIRE server and trust domain (ADR 1164) is designed, not built, so at a joiner
`goatbuilder endstate` grades it `GAP` and the `join-node` plan lists it under
"does not run yet", each ending "(ADR 1164)" where the other gaps name their
gate row.

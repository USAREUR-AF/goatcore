---
heading: For someone deploying
title: "Changed: diff and status report a site's declared gap as GAP"
---

`goatbuilder diff` and `goatbuilder status` read the end-state model of each
site. A component the model marks as not yet built at that site (an admin
surface at a joining site, gate rows I1 and I4) is reported as
`GAP: declared, not yet built at this site`, with the model's reason, instead of
an unknown that fails the verb. A component the model says the site runs, with
no ledger, is still unknown and still fails.

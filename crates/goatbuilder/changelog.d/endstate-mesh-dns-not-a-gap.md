---
heading: For someone deploying
title: Changed: mesh DNS is no longer a gap at a joining site
---

`goatbuilder join-node` runs a joining site's own mesh DNS replica, so the
end state no longer grades mesh DNS `GAP` at a joining site and
`join-node`'s list of what the site does not run yet no longer names it. The
admin surfaces stay gaps, with gate rows I1 and I4 (ADR 1233, ADR 1235).

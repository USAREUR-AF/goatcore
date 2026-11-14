---
heading: For someone deploying
title: "Added: join-node deploys the health publishers a joining site declares"
---

`goatbuilder join-node` now deploys a joining site's health publishers on
its core node, in phase 13 after the data router. It reads the site's end
state (`endstate.py model <site>`) and runs the deploy play of each
publisher the model marks present (fabric-health, mesh-health,
presence-health, time-health, consistency-health, fabric-crawler), for the
site's core group limited to the node. They were hand steps after the
command. A publisher the model marks present whose play is not in the
checkout is refused when the plan is made (GB-288). When the model cannot be
read, the plan says so on a SKIP line and deploys none.

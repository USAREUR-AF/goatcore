---
heading: For someone deploying
title: "Changed: `goatbuilder join-node` names each declared service its run does not build"
---

At a joining site, the dry-run plan and the run's summary already listed the
end-state model's gaps and the services the site turns off. They now also
list each service the model marks present on the node that no step of the run
builds. At a site with its own obs host, a service the node's other class
builds names that run, for example `join-node --class obs`. At a single-host
site, the observability stack the model places on the core node is not built
by any verb yet, and reads as the gap it is. A service
no step builds reads "no join-node step builds it yet (gate row I3)". The
admin surfaces keep their gap line (gate rows I1 and I4). Nothing a run does
changes (#953).

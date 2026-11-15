---
heading: For someone deploying
title: "Changed: `goatbuilder join-node` names each declared service its run does not build"
---

At a joining site, the dry-run plan and the run's summary already listed the
end-state model's gaps and the services the site turns off. They now also
list each service the model marks present on the node that no step of the run
builds. At a single-host site, where the model places the observability stack
on the core node, a core run names Prometheus, Alertmanager, Grafana and the
exporters with the run that builds them, `join-node --class obs`. A service
no step builds reads "no join-node step builds it yet (gate row I3)". The
admin surfaces keep their gap line (gate rows I1 and I4). Nothing a run does
changes.

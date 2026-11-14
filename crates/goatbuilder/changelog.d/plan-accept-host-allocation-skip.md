---
heading: For someone deploying
title: "Added: goatbuilder federate plan --accept host-allocation records a judged SKIP"
---

A deployment that declares its other peerings by hand may not be able to make
every value the host-allocation check compares resolve. That check is not run
again by deploy, so its SKIP used to stop deploy for good. `goatbuilder
federate plan <partner> --accept host-allocation` now records that SKIP with
its exact detail inside the plan and its hash. The plan view shows it at
approval, and deploy goes ahead while the check reads exactly the same. When
it reads differently, deploy refuses (GB-188) and names what it reads now and
what was accepted. Only a check deploy does not run again can be accepted,
and only when it is SKIP (#779).

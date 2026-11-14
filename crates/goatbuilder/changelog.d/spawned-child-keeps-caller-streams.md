---
heading: For someone deploying
title: Fixed: deploy no longer refuses a plan with an edge host firewall as changed (GB-188)
---

With the host firewall baseline on for an edge, `goatbuilder federate` phase 10's
deploy refused the approved plan on every attempt with GB-188 ("host_firewall
changed since the plan; host-firewall BLOCKED") and printed the renderer's JSON
into the transcript. Inside a run, a command started in the background lost
the output its caller had piped to the runner's own streaming, so deploy's
recheck read no ruleset from the renderer. The runner now leaves a stream the
caller set to the caller, and deploy's recheck reads the same ruleset and hash
the plan did.

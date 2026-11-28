---
heading: For someone deploying
title: "Changed: oci-substrate runs only where oci_substrate_enabled is set"
---

`oci-substrate-deploy.yml` has a switch, `oci_substrate_enabled`, off by
default. Its first step restarts docker, so it must not run where nobody asked
for it. A site that uses the image distribution substrate sets
`oci_substrate_enabled: true` in its host group's group_vars. Until it does, a
run of the play says in one line that the switch is off and changes nothing.
With the switch on the play runs as before. The T2 sync step keeps its own
switch, `oci_t2_sync_enabled`.

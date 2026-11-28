---
heading: For someone deploying
title: "Changed: names-collector runs only where names_collector_enabled is set"
---

`names-collector-deploy.yml` has a switch, `names_collector_enabled`, off by
default. A site that runs the self-claims collector sets
`names_collector_enabled: true` in its host group's group_vars. Until it does, a
run of the play says in one line that the switch is off and changes nothing.
With the switch on the play runs as before.

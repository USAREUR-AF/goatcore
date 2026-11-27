---
heading: For someone deploying
title: "Fixed: phase 10's admission finds the core router's play on a goatnet whose group_vars carry a rendered pins.yml"
---

`goatbuilder federate deploy` admits the edge on our core through the play
that deploys the core router, found from the fleet pin whose variable the core
group's group_vars set. A rendered `pins.yml` sets the image variable of every
component the goatnet runs, each with its own deploy play, so no single play
was found and admission refused with GB-236. It now looks first among the
variables the goat-zenoh role reads, which name the core router's play alone,
and then among all of the group's variables, as before.

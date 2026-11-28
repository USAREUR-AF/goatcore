---
heading: For someone deploying
title: "Fixed: an opt-in component's var is governed only where a goatnet opted in"
---

`generate-pins.py --check` and `goatbuilder pins --check` treated the var of
every component as governed for every member. A member of a goatnet that has not
opted in to an opt-in component may set that var by hand, for example to run the
play by hand. Both checks refused that as a hand pin shadowing a generated one,
although no pin is generated there. They now check it against
`ops/pins/unmigrated-pins.txt`, as they did before the row existed.

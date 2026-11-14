---
heading: For someone deploying
title: Changed: a back-out drops the partner's store whole on a goat-zenoh role core
---

`goatbuilder federate backout <partner>` now takes the partner's entry out of
`zenoh_sovereign_peerings` in `group_vars/<core router group>/peerings.yml`,
keeping every other line, runs the role's play, which restarts the router
without the partner's store, and then runs `ops/ansible/zenoh-drop-partner-store.yml`,
which deletes `<storage path>/sovereign_from_<partner>` once the router runs
without it (ADR 1234). Commit the edited `peerings.yml` through review. A store
the role kept because the router does not yet run without it is reported as
GB-270. The role no longer renders the shared `sovereign_from` store on
`from/**`: on a core that had it, its directory stays on disk with no retention
and is listed in `.undeclared-stores` until you dispose of it. The federation
partner bundle changed with the specification; partners holding the previous
bundle need the new one (#629).

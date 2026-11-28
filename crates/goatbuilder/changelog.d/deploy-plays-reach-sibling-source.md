---
heading: For someone contributing
title: "Changed: the deploy-play reach check credits another repository's play that goatbuilder runs by name"
---

`ops/ci/check-deploy-plays-reached.py` now counts a play in another public
repository as reached when goatbuilder's own sources run it by name and goatcore
has no play of that name, since a verb runs it from the tree's `ops/ansible/` in
a tree that layers that repository. goatstuff's
`zenoh-synth-vendors-slot-pool-deploy.yml`, which `standup` phase 12 runs, is
reached by this, and its `owed:` entry is removed.

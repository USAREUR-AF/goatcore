---
heading: For someone contributing
title: "Added: the deploy-play check reads the other public repositories' plays (gate J28)"
---

`ops/goatbuilder/deploy-plays.yaml` declares the other public repositories with
deploy plays (`repositories:`, today goatstuff and goatrodeo), each by its URL and
a full commit, with its own `unreached:` and `owed:`.
`ops/ci/check-deploy-plays-reached.py` reads their `ops/ansible/*-deploy.yml`
from a checkout at that commit (`--sibling NAME=DIR`, or `--fetch DIR`, which CI
uses). A play there is reached by a component of goatcore's pins whose play is
found in a tree that layers that repository, or whose `source:` names it.
`--strict` also fails on a declared repository that was not read, or a checkout
at another commit. Their 10 unrouted plays are listed as owed: goat-map to J25,
goat-feeds to J24, the rest to J28.

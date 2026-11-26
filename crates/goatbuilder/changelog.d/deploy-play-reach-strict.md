---
heading: For someone contributing
title: "Changed: a new deploy play no goatbuilder verb reaches fails CI (gate J28)"
---

`.github/workflows/deploy-play-reach.yml` now runs
`ops/ci/check-deploy-plays-reached.py --strict`. A new `ops/ansible/*-deploy.yml`
fails the job until a verb reaches it (a pins component, a type file's phase, or
goatbuilder's own source) or `ops/goatbuilder/deploy-plays.yaml` lists it. The
list has a new key, `owed:`, for a play a verb should run and none does yet: its
entry names the gate row that will route it, and the report counts owed plays
apart from those listed as rightly unreached. 18 plays are owed today. An entry a
verb now reaches fails too, so the change that routes a play takes it off.

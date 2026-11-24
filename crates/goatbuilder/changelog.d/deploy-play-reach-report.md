---
heading: For someone contributing
title: "Added: a report of every deploy play no goatbuilder verb reaches (gate J28)"
---

`ops/ci/check-deploy-plays-reached.py` reads every `ops/ansible/*-deploy.yml` and
says how a goatbuilder verb reaches it: a pins component `goatbuilder deploy`
finds, a play a type file names, a play goatbuilder's own source runs, or one
of those plays' imports. A play none of them reaches is a gap, unless
`ops/goatbuilder/deploy-plays.yaml` lists it with its reason. It runs as a
report today and names 22 gaps; it becomes blocking with `--strict` once each
gap has an answer.

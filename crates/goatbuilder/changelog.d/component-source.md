---
heading: For someone deploying
title: "Added: a component's play may be in another repository, at a pinned commit"
---

A component of `ops/pins/fleet-pins.yaml` may declare `source:` with
`repository`, `commit` (a full commit) and `play`, in place of
`deploy_playbook`. `goatbuilder deploy` and `ops/scripts/deploy-component.sh`
fetch that commit with your own git access, hold the copy to it on every run, and
run its play; nothing of it is put in the checkout. A source that cannot be
reached, is not exactly the commit, has no such play, or carries a role goatcore
has is refused before any host (GB-385). The script needs goatbuilder on PATH, or
`GOATBUILDER_BIN`.

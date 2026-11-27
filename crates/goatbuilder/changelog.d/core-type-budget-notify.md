---
heading: For someone deploying
title: "Added: join-node runs the budget notifier on a joining core machine"
---

The core type (`ops/goatbuilder/targets/core.yaml`) gains a phase for host
services with a switch of their own, and its first play is
`budget-notify-deploy.yml`, run on the joining node of a joining site. It
follows `budget_notify_enabled`, off by default: with it off the play says so
in one line and changes nothing.

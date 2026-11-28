---
heading: For someone deploying
title: "Added: join-node runs the self-claims collector on a joining core machine"
---

The core type's phase for host services with a switch of their own
(`ops/goatbuilder/targets/core.yaml`) also runs `names-collector-deploy.yml` on
the joining node of a joining site. It follows `names_collector_enabled`, off
by default: with it off the play says so in one line and changes nothing.

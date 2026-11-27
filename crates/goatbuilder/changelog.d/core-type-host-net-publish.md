---
heading: For someone deploying
title: "Added: join-node runs the host network health publisher on a joining core machine"
---

The core type's phase for host services with a switch of their own
(`ops/goatbuilder/targets/core.yaml`) also runs `host-net-publish-deploy.yml`
on the joining node of a joining site. It follows `host_net_publish_enabled`,
off by default: with it off the play says so in one line, changes nothing and
writes no deploy-ledger row.

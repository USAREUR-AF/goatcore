---
heading: For someone deploying
title: "Added: join-node runs the image substrate client on a joining core machine"
---

The core type's phase for host services with a switch of their own
(`ops/goatbuilder/targets/core.yaml`) also runs `oci-substrate-deploy.yml` on
the joining node of a joining site. It follows `oci_substrate_enabled`, off by
default: with it off each of the play's two plays says so in one line and
changes nothing, and docker is not restarted.

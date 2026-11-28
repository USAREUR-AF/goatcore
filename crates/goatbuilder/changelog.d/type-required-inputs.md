---
heading: For someone deploying
title: Changed: join-node refuses a network-jump machine whose herd_connector_image is unset
---

A type file can name `inputs`, values of the site's configuration a machine of
the type needs before it is changed. network-jump names `herd_connector_image`:
a machine with it unset, or set to the role's placeholder `PENDING`, is refused
before any phase (GB-414) by `join-node` (a dry run too) and by a `standup` run
that reaches phase 14, instead of failing in phase 13 after the machine was
changed. `doctor --target` shows the input (#1345).

---
heading: For someone deploying
title: "Added: standup deploys the services of a type it builds in phase 14"
---

A target type that `standup` builds in phase 14 may now list `services:`. After
the type's plays, phase 14 deploys each to the type's machines at the site
through `goatbuilder deploy`, and a listed service with no component, group,
pin or play is refused before any phase (GB-384), as `join-node` does.

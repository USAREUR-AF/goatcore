---
heading: For someone deploying
title: "Fixed: join-node refuses a core at the founder's site that declares no standby"
---

Standup builds a founder site's first core, so a core `join-node` adds to that
site is its streaming standby. A site that does not declare
`intra_site_pair: true`, such as a goatlite, is now refused with GB-440 at the
run's first phase, before the node changes. Before, the node came up as a
second primary with its own empty NetBird database, and its management service
restarted until phase 13 failed. Declare the pair, or join the machine as the
first core of a new site.

---
heading: For someone deploying
title: "Added: join-node deploys the data router at a joining site"
---

`goatbuilder join-node` now runs the site's own Zenoh router on a joining
site's core node, in phase 13 after signal federation: `zenoh-deploy.yml`
for the site's core group, limited to the node. It was a hand step after
the command. The play still needs the node's router leaf certificate,
which join-node does not stage yet.

---
heading: For someone deploying
title: Added: goatbuilder.yaml takes deploy.cloud, deploy.owner and deploy.cross_goatnet
---

Three optional `deploy` fields say what a goatlite decided from its preset,
so a full configuration grown from one behaves the same: `cloud: none`
means no play asks for a cloud account; `owner` is the bootstrap owner,
used in place of `$OWNER_EMAIL`; `cross_goatnet: false` means standup
phase 14 does not reach the cross-goatnet boxes. Undeclared, nothing
changes. `goatbuilder grow` writes them for a goatlite it hands over (#765).

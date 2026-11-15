---
heading: For someone deploying
title: Added: a peering's forwarder declares its machine's SSH user, interface and SSH sources
---

`forwarder:` on a peering in goatbuilder.yaml takes `ansible_user`,
`public_interface` and `ssh_sources`, the forwarder machine's own values.
`goatbuilder federate allocate` renders each declared one into the forwarder
group's `peering-forwarder.yml` (`ansible_user`,
`peering_forwarder_public_interface`, `peering_forwarder_ssh_sources`), so the
forwarder needs no hand-written host file. Peerings on one forwarder that
declare different values are refused (GB-339). A value left out stays where
it was: in your own group_vars file, or the role's default.

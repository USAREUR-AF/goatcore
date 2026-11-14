---
heading: For someone deploying
title: Changed: the peering plays require the host firewall baseline on an edge
---

Every play a peering standup runs on an edge (phases 2, 11 and 12) now checks
`goat_host_firewall_baseline` first, and refuses with GB-279 when it is not
true, before anything on the edge changes. Set
`goat_host_firewall_baseline: true` and `goat_host_firewall_management_sources`
in the edge's group_vars (copy
`ops/ansible/group_vars/_examples/peering-host-firewall.example.yml`), then run
the peering again; phase 11's play applies the baseline. `goatbuilder federate
plan` shows such an edge as BLOCKED with GB-279. The role's default stays false,
and a core host keeps the baseline opt-in.

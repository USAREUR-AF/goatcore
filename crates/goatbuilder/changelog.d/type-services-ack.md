---
heading: For someone deploying
title: "Added: join-node --ack, and a service that needs one is refused before any phase"
---

A service a target type lists whose rollout is not cleanly reversible (its
rollback is `none` or `restore-required`, ADR 1211) needs a name that accepts
the risk. `join-node --ack <name>` and `standup --ack <name>` pass it to that
service's deploy, which records it in the ledger row. A run without it is now
refused before its first phase (GB-421); before, it stopped at the service's
phase after the machine had changed.

---
heading: For someone deploying
title: Changed: the forwarder's carrier read-back reads each peer's public key and allowed address
---

`goat-peering-forwarder` reads each carrier back with `wg show <carrier>
allowed-ips` and `listen-port`, and a back-out checks that a carrier exists with
`listen-port`. When a carrier does not hold what the declaration says, the failure
message lists each peer's public key and allowed address, then the port the
carrier listens on (#663).

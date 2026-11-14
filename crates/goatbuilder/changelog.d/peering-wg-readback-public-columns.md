---
heading: For someone deploying
title: Changed: the WireGuard read-back of phase 11 reads each peer's public key and allowed address
---

`goat-peering-wg` reads the interface back after phase 11 with `wg show
<interface> allowed-ips`, and the carrier's with `allowed-ips` and
`latest-handshakes`. When the interface does not hold what the agreement says,
the failure message lists each peer's public key and allowed address, then the
interface's addresses. GB-271's explanation describes the message as it is now.

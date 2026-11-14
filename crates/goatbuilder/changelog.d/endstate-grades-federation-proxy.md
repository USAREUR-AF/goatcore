---
heading: For someone deploying
title: Added: the end state grades federation-proxy where a site turns it on
---

`goatbuilder endstate` now has a row for `federation-proxy`, the node-local
proxy signal federation dials its partner through. A site that sets
`signal_federation_proxy_enabled` and does not run it, or runs it without
the flag, is now reported. The flag defaults to false, so a site that does
not set it is graded as not running the proxy. A site without the
`signal-federation-transport` ledger grades its deployment as unknown, never
as failed (#637).

---
heading: For someone deploying
title: Fixed: phase 13 no longer replaces the partner's refreshed SPIFFE bundle with the older exchanged one
---

`goat-peering-spire` imported the partner's bundle from the file exchanged in
phase 5 on every phase 13, with `spire-server bundle set`. That overwrites
the bundle our server had already refreshed from the partner's bundle
endpoint. After the partner's SPIRE rotated its keys, a rerun of phase 13
put the old keys back: their new identities stopped verifying until the next
refresh, and that refresh authenticates the endpoint with the bundle held.

Phase 13 now imports the file only in two cases: when our server holds no
bundle for the partner, or when the file's `spiffe_sequence` is newer than the
one held. Otherwise it keeps the held bundle and says so. For a partner whose
SPIRE server was rebuilt with new keys, confirmed on the call, set
`peering_spire_bundle_replace: true` to import the file over any bundle held.

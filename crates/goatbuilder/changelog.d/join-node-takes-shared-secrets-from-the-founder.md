---
heading: For someone deploying
title: "Added: `goatbuilder join-node` gives a joining site its secrets in phase 3"
---

A joining site's core node used to need its secrets written by hand before
phase 13. Phase 3 now runs `ops/scripts/join-site-secrets.py`. The values
every site of a goatnet shares (NetBird's datastore key, the replicator, the
relay secret, Ory's network id, the Kratos cipher and the Hydra system secret,
the lines of `newsite-secrets.sh` marked `cluster`) are taken from the
founder's: as a reference to the founder's entry under a store of references,
or as the founder's value under a store of values. `newsite-secrets.sh` then
mints the site's own values, and each shared value is compared with the
founder's by digest, printing names only. Nothing is overwritten: a joiner
value that differs is refused and kept (GB-353), a founder value that is
missing or does not resolve is refused (GB-352), and under the `env` store a
joiner name that is not exported is refused (GB-354). `newsite-secrets.sh
--cluster-json` lists the shared names.

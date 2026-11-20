---
heading: For someone deploying
title: "Added: join-node refuses to leave a joiner subscribed over a seed the founder changed since (GB-367)"
---

A joining site's seed carries the founder's data, and its Spock subscriptions
carry only writes made after they exist, so a founder write made between the
seed and the Spock wiring never reached the joiner, and nothing said so. Phase
13 now compares every seed table on the founder and the joiner (row counts and
checksums, read-only) before it wires Spock, and again after, once the
subscriptions from the founder have caught up. A table that differs stops the
run with GB-367, naming the table and its two row counts. Before Spock is
wired, re-seed and resume with `--from 13`.

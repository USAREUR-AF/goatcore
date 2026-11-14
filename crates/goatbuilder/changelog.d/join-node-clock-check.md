---
heading: For someone deploying
title: "Added: join-node refuses a node whose clock is off, before changing it"
---

`goatbuilder join-node` phase 3 now reads the joining node's clock from its
own time service (`chronyc -c tracking`, else `timedatectl`), read-only, before
anything on the node changes. A node that is not synchronized to an outside
time source, or is more than 250 ms from UTC (`JOIN_NODE_CLOCK_BUDGET_MS`), is
refused with GB-297, naming the node and what it read. A Zenoh router refuses
a timestamp more than 500 ms ahead of its own clock, and nothing tells the
machine's owner. A clock that could not be read is reported as not verified,
never as passed. The rule is the federation preflight's clock check (GB-268).

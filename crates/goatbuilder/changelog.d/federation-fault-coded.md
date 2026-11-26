---
heading: For someone deploying
title: Fixed: a federation verb's fault prints a catalog code
---

The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
on their own or inside a peering run) printed a fault their library raised as
`goatbuilder: <text>` with no code. It now prints the new code GB-338, a
federation verb faulted, under the same message, which names what failed (a
file, a flag, a party). It still exits 1, as before and as `goat federate`
does.

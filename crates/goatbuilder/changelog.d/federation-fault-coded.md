---
heading: For someone deploying
title: Fixed: a federation verb's fault prints a catalog code
---

The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
on their own or inside a peering run) printed a fault their library raised as
`goatbuilder: <text>` with no code. It now prints a code under the same
message: GB-254 for a file the verb could not read or write, named in the
message, and the new GB-338, a federation verb faulted, for anything else.
Both still exit 1, as before and as `goat federate` does.

---
heading: For someone deploying
title: Changed: the federation library's own refusals print their own codes
---

The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
on their own or inside a peering run) now print a code of the federation
library's own for each of its refusals, in place of GB-338: X-FED-001 a file
it cannot read, X-FED-002 a file it cannot write, X-FED-003 a `--now` that is
not a date, X-FED-004 a `--key` or `--keyring` key it cannot use, X-FED-005 a
signing key that is not an Ed25519 seed, X-FED-006 `apply --confirm` without
`--out`, X-FED-007 `apply` with no party to apply for. The message is
unchanged, and `goat federate` prints the same codes. They still exit 1.
GB-338 is left for a fault with no code of its own, such as a report that
could not be written to a closed pipe.

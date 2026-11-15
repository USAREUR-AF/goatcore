---
heading: For someone deploying
title: Fixed: the C3 comparison says when a verify report read no machine
---

`ops/goatbuilder/c3-agreement.py` compares the `-o json` reports of `status`,
`diff` and `verify` for gate row C3. When a `verify` run read no machine,
because its transport or inventory read failed, the script used to say that
`verify` had not been run for that component, and exit 3 as if the three verbs
disagreed. It now names the report that read no machine and exits 2: the run is
not a C3 measurement. Fix the reach and run the sequence again. A component
with no ledger coverage (`ledger_component: null`) used to be reported as a
disagreement on every machine; it is now left out of the comparison and named
(#809).

---
heading: For someone deploying
title: Added: goatbuilder status reads real hosts
---

`goatbuilder status --env <env>` without `--mock` refused with GB-026. It now
reads each machine's deploy ledger the way `goatbuilder diff` reads it and grades
it the way `goatbuilder verify` grades it, so the three agree about every
machine. It reads only: no play runs and nothing is written. It exits 3 while
any machine is pending or unknown. GB-026 is retired: no release after this one
prints it.

---
heading: For someone deploying
title: Added: join-node and teardown keep a timings file, as standup does
---

`goatbuilder join-node` now writes a row for each step it runs to
`<state_dir>/join-node-<site>-<node>-timings.tsv`, and `goatbuilder teardown`
writes a row for each component it stops to
`<state_dir>/teardown-<site>-timings.tsv`. The columns are standup's: time
written, phase, step label, seconds, exit code. Both runs now end with
standup's timing line (the total and the five slowest steps), and join-node
prints "phase <n> took <s>s" for each step. A dry run writes nothing, and a
failed step is the last row, with its exit code (#802).

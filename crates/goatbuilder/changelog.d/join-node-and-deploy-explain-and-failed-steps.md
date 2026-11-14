---
heading: For someone deploying
title: Added: explain answers for join-node's phases and deploy's steps, and their failures name the error
---

`goatbuilder explain join-node [<phase>]` and `goatbuilder explain deploy
[<step>]` answer from the phase catalog, as they already do for `standup` and
`federate`: what the phase does, why, what it changes, how to tell it worked,
what it runs and the command by hand. With no phase given, they list the
verb's phases.

When a join-node phase or a group's deploy play fails, its output is matched
against the error catalog. The failure then carries an `interpreted` row: the
error's code, its likely cause and fix, the log, and the command that runs
that step again by hand. With no known error, the row gives the phase's own
guidance.

The standup runbook's phase table and the federate table in the sites chapter
of the reference are now rendered from the phase catalog. The runbook's table
has one row a phase, with what it does and how to tell it worked; `explain
standup <phase>` gives the rest.

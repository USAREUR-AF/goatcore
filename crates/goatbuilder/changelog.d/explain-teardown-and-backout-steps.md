---
heading: For someone deploying
title: "Added: `goatbuilder explain teardown` and `explain federate-backout` answer for each of their steps"
---

The phase catalog now describes `teardown` and `federate backout`.
`goatbuilder explain teardown` lists the standup phases teardown takes down,
in reverse (2, 5, 6, 7, 8, 9, 10, 12, 14), and `explain teardown 12` says
which of the phase's steps come down, which are kept, and which a teardown
refuses because reversing them destroys data or reaches another site.
`goatbuilder explain federate-backout` lists the back-out's seven steps
(survey, confirm, remove, recheck, core, directory, record) and answers for
each. Tests hold the entries to what each verb walks, so a new step cannot
ship without one. Nothing a run does changes.

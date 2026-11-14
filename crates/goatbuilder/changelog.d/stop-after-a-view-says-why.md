---
heading: For someone deploying
title: Fixed: a back-out that stops after its plan says why, with its code
---

`goatbuilder federate backout <partner>` printed its plan and then stopped
without saying why when the survey refused (GB-221) or the slot recheck was
blocked (GB-223). The plan was all that reached the terminal. The refusal, its code and
its fix are now printed after the plan, as are a later stop's rows: an
unconfirmed back-out, a removal play that failed (GB-225), and a back-out
that did not converge. A `federate` dry run whose values could not be listed
now says so after its plan, too.

A run record (`<state>/runs/<id>/run.json`) of a run that stopped without an
error now keeps the rows that stopped it, under `stop`, and
`goatbuilder show <run>` prints them.

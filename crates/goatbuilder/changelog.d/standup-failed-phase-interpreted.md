---
heading: For someone deploying
title: Added: a failed standup phase names the error in its output
---

When a standup phase fails, its output (the log the run kept of it) is matched
against the error catalog, as `goatbuilder federate` already does. The failure
then carries an `interpreted` row: the error's code, its likely cause and fix,
the log, and the command that runs that phase again by hand. With no known
error in the output, the row gives the phase's own guidance from the phase
catalog. This covers the numbered phases; the image pull, the observability
legs and phase 12's parallel steps are not covered yet.

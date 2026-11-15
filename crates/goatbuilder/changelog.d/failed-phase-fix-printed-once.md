---
heading: For someone deploying
title: Fixed: a failed phase prints its fix once, and an incomplete acceptance names the command that resumes it
---

A failed phase's `interpreted` row printed the error catalog's fix twice:
inside the row's text, then again on its own `Fix:` line. It now prints
it once, on the `Fix:` line, and `-o json` keeps it in the row's `fix`
field. This applies to `standup`, `join-node`, `deploy` and `federate`.
When phase 14 ends INCOMPLETE (X-ACCEPT-001), the fix now names
`goatbuilder federate <peering> --from 14` and no longer names the scripts
avenue's `ops/scripts/peering-standup.sh` (#911).

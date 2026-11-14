---
heading: For someone contributing
title: Added: standup's phases are in the phase catalog, and explain answers for them
---

`ops/phases/catalog.yaml` now describes standup's 14 phases and its named
steps (the image pull and the observability legs, with their joins), each
with what it does and why, what it changes, how to tell it worked, what it
runs, the command by hand and what to do when it failed. A test builds
standup's plan for six site shapes and holds the catalog to it, so the two
cannot drift. `goatbuilder explain standup [<phase>]` answers from the
catalog, by number or name, as it does for federate (gate row F4) (#651).

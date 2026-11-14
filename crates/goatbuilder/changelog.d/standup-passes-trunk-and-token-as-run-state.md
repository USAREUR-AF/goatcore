---
heading: For someone contributing
title: Fixed: standup passes the trunk and the mirror token to its phases as run state
---

`goatbuilder standup` no longer writes `GOAT_FRESH_SOURCE_TRUNK` and the
pub/sub-state mirror bot's token into its own process environment in the
middle of a run. Both are run state, given to each phase's process as its
own environment. The trunk goes to every phase, as before. The token goes
only to the two phases that read it, the mint step and the mirror play, and
is removed from every other phase's environment, even when the operator's
own environment holds it: least privilege for child processes. A test
allows a process-environment write only in `main`, before any thread
starts (#642).

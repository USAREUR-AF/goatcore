---
heading: For someone deploying
title: Changed: what federate says about phase 10 names the command that runs it first
---

Phase 10 admits the partner's edge on our core, and `goatbuilder federate
deploy` runs it once you approve the plan and what our core admits. A dry run's
plan page now says "At phase 10, goatbuilder runs this for you:" and names
`goatbuilder federate plan`, `deploy` and `test` before the steps by hand. When a
run stops at phase 10 without a terminal, it names `plan`, then `deploy
--approve-plan <plan hash> --approve-admission <digest>`, then `test`, and gives
admitting by hand, then `--from 11`, as the fallback, which still works.
`goatbuilder explain federate 10` leads with the same commands. Phase 10 is
still listed as a manual phase.

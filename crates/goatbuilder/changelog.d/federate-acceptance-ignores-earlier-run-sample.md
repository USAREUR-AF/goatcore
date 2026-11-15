---
heading: For someone deploying
title: Fixed: acceptance on a derived session ignores a partner sample left by an earlier run
---

Phase 14's acceptance script, run on a derived session
(`ops/federation/standup/acceptance.sh` with `FED_ACCEPTANCE_ATTEMPT`),
learns the partner's challenge from the partner's sample on our core.
Samples stay on the cores between runs, so a sample left by the partner's
earlier run taught that run's challenge: our signed result echoed a stale
one, the partner refused it (GB-273), and the two sides could end with
different verdicts until both ran again. Each run now stages the time it
started beside its samples, and a reader ignores a partner sample whose
start time is more than `FED_SESSION_WAIT` before its own, says so, and
keeps waiting for this run's. With only an earlier run's samples there, those
rows are BLOCKED and the run ends open (exit 3). A partner whose runner
stages no start time is read as before.

Limits: this rests on both sides' clocks agreeing within `FED_SESSION_WAIT`.
`goatbuilder federate test <partner> --bilateral`, which runs phase 14 on a
derived session (#720), runs phase 1's clock check (GB-268) alone first and
stops on a clock that is unsynchronized or over the budget. A partner's
earlier run that started within `FED_SESSION_WAIT` of ours is not told
apart; its result is still refused (GB-273), so the verdicts can differ
until a rerun, but a stale result never passes. Tested on the in-process
fabric only, not on hosts (#786).

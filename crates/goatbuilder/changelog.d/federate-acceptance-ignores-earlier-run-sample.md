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
different verdicts until both ran again. Each run now stages a run stamp
beside its samples, holding its challenge and the time it started in one
value. A reader learns the partner's challenge from the stamp and waits for
the sample carrying it. It ignores a stamp that started more than twice
`FED_SESSION_WAIT` before its own start, says so, and keeps waiting for this
run's; a stamp it cannot read is waited for, never taken as missing. With
only an earlier run's sample there, the delivery rows are BLOCKED, say that
an earlier run's sample is what is there, and the run ends open (exit 3). A
partner whose runner stages no stamp is read as before.

Limits: a partner's current run is kept when both sides start phase 14
within `FED_SESSION_WAIT` of each other and their clocks agree within
`FED_SESSION_WAIT`. Phase 1 does not check the clocks yet, so keep each
host's clock synchronized. An earlier run that started less than twice
`FED_SESSION_WAIT` before ours is not told apart, nor is one from a runner
that stages no stamp; its result is still refused (GB-273), so the verdicts
can differ until a rerun, but a stale result never passes.
`goatbuilder federate` does not run a derived session yet. Tested on the
in-process fabric only, not on hosts (#786, #792).

---
heading: For someone deploying
title: Fixed: acceptance on a derived session learns a partner's challenge from its run stamp
---

Phase 14's acceptance script, run on a derived session
(`ops/federation/standup/acceptance.sh` with `FED_ACCEPTANCE_ATTEMPT`),
staged only its start time beside its samples, so a partner's rerun soon
after a first run could be judged by the first run's challenge, giving a
false FAIL. Each run now stages a run stamp beside its samples, holding its
challenge and the time it started in one value. A reader learns the
partner's challenge from the stamp and waits for the sample carrying it. It
ignores a stamp that started more than twice `FED_SESSION_WAIT` before its
own start, says so, and keeps waiting for this run's; a stamp it cannot read
is waited for, never taken as missing. The challenge is the one of the stamp
whose sample it found, so a rerun whose first run's sample never arrived is
judged by the rerun's; two runs' stamps on two sets end BLOCKED, never FAIL.
With only an earlier run's sample there, the delivery rows are BLOCKED, say
that an earlier run's sample is what is there, and the run ends open
(exit 3). A partner whose runner stages no stamp is read once a whole
`FED_SESSION_WAIT` has passed with no stamp of its own, since this run's
stamp may still be in flight; once a partner's stamp has been read (a
garbled one included), a sample without one is never learned; an unstamped
sample's row says its run identity is not proven, and a second unstamped
challenge is BLOCKED as two runs.

Limits: a partner's current run is kept when both sides start phase 14
within `FED_SESSION_WAIT` of each other and their clocks agree within
`FED_SESSION_WAIT`, which `goatbuilder federate test <partner> --bilateral`
checks first (GB-268). An earlier run that started less than twice
`FED_SESSION_WAIT` before ours is not told apart, nor is one from a runner
that stages no stamp; its result is still refused (GB-273), so the verdicts
can differ until a rerun, but a stale result never passes. Tested on the
in-process fabric only, not on hosts (#792).

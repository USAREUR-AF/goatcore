---
heading: For someone deploying
title: Added: `federate test --bilateral` completes the partner's half of acceptance, opt-in
---

`goatbuilder federate test <partner> --bilateral` runs phase 14 on a session
both sides work out from the agreement, with no manifest or result file
passed between the operators. Each side signs its result with its federation
signing key and publishes it on its own core, reads the partner's, and
verifies it under the key confirmed at phase 5; the combined result in
`<evidence>/bilateral/` is then marked authenticated. Both sides start phase
14 within `FED_SESSION_WAIT` (300 seconds by default) of each other, normally
on the same call. It is opt-in: without `--bilateral`, phase 14 runs as before
and its exit is unchanged. A partner result that does not verify is GB-272,
one from another session or run GB-273, and an unset or unresolved signing key
GB-142.

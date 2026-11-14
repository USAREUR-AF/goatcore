---
heading: For someone deploying
title: Changed: phase 10 leaves our core router running when the admission it would make is already in place
---

`goatbuilder federate deploy <partner>` with the admission done by the command
no longer places, dry-runs and restarts our core router again when nothing it
would place has changed. After a restart passes, the run journal records the
admission as placed. A later phase 10 is the same admission when it has the same
digest of what is placed (the approval digest without the plan hash) and every
file on the core is byte for byte what it would place. That phase 10 then
places nothing, restarts nothing and asks for no approval. This happens when
phase 10 runs again because a rotated leaf the core does not hold changed its
inputs. A file on the core changed by hand, or an admission whose restart
failed, is admitted again as before.

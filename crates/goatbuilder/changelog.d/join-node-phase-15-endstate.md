---
heading: For someone deploying
title: "Fixed: join-node's phase 15 runs the acceptance rows against the site's end state"
---

Phase 15 ran `run-acceptance.py` with no `--endstate`. A row for a service the
site declares absent read FAIL and stopped the run. Phase 15 now runs the same
acceptance line as standup: it writes the site's end-state model to the state
directory and passes it with `--endstate`, so such a row reads N/A.

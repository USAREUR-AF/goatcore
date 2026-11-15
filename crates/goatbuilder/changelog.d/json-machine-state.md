---
heading: For someone deploying
title: Added: each machine row in -o json says its state
---

In the `-o json` and `-o yaml` documents of `deploy`, `status`, `verify`,
`rollback`, `diff` and `standup`, each row that reports one machine now
carries `"state"`: `converged`, `pending` (not yet confirmed) or `unknown`
(its record could not be read). Read the field instead of the words in
`detail`. `ok` is true only for `converged`. Nothing was removed or renamed.

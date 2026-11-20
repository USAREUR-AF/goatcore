---
heading: For someone deploying
title: "Added: status and verify say each machine's state as data in -o json"
---

Every `status` and `verify` document in `-o json` now carries `machines`: one
entry per graded machine row, with its `environment`, `host`, `component`,
`state` (`converged`, `pending` or `unknown`), a `reason` from a closed set
(`declared`, `unfinished`, `other-ref`, `no-record`, `other-image`,
`served-off`, `unreachable`, `no-answer`, `unreadable`) and `check`, the row it
grades. The list is empty when nothing was graded, and no other verb's document
carries it. A script no longer has to read the first word of a row's `detail`
to tell a machine that has not converged from one that could not be read. The
`checks` rows, the text pages and the exit codes are unchanged.

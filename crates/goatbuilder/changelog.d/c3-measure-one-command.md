---
heading: For someone deploying
title: "Added: ops/goatbuilder/c3-measure.sh runs gate row C3's check on a standing goatnet as one command"
---

`ops/goatbuilder/c3-measure.sh --env <goatnet> --out <directory>` runs
`doctor`, `status`, `diff` and `verify` for each component `status` names,
then `ops/goatbuilder/c3-agreement.py`. It changes nothing on a host, in a
ledger or in the goat context; a door the verbs open for it is closed at exit,
and one already open is left open. Each verb is stopped after
`C3_VERB_TIMEOUT` seconds (default 600). It times each verb and keeps the
reports and a `record.json` in the directory, mode 0700, or in a new
directory under `$TMPDIR` when `--out` is missing. Its last line is one
summary that names no host, and shows the goatnet only as `<env>`. It exits
0 when the three verbs agree on every machine, 3 on a disagreement, and 2
when the run is not a measurement, a machine not reached included. A value
shaped like a credential, and the value of an environment variable that
`ops/ci/check-no-live-secret.sh` treats as a live secret, is left out of
`record.json`, the summary line and the kept stderr.

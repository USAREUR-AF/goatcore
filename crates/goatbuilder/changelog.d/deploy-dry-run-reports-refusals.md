---
heading: For someone deploying
title: "Changed: `deploy --dry-run` runs every check that touches no host, and exits 1 when a real run would refuse"
---

A deploy dry run, with `goatbuilder deploy` or `deploy-component.sh`, now runs
the fresh-source check and the secrets gate as a real run does. Each refusal is
a failed row, the whole plan is still printed, and the run exits 1 (refused).
A release-ladder refusal, which ended a dry run before its plan, is reported the
same way. Before, a dry run ended `ok`, exit 0, on a tree whose pin had been
edited and with a required secret unset. A dry run now fetches the trunk
(`--offline` judges against the cached one).

Deploy itself refuses a pin with no `@sha256:` digest, before any host, in a real
run and a dry run on both avenues (new code GB-382). The pin is the pins.yml
value as YAML reads it, so a digest in a trailing comment does not count, and
the digest must be exactly 64 lowercase hex characters. `verify` and
`--verify-only` are unchanged.

`goatbuilder deploy` no longer opens a goatnet's door before refusing a run for
an unset secret; `deploy-component.sh` already refused first.

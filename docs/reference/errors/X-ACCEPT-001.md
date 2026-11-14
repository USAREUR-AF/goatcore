# X-ACCEPT-001: acceptance could not run a required check, so the peering is not accepted

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `X-ACCEPT-001` is an error of `acceptance.sh` that goat and goatbuilder carry and interpret. The same text offline: `goat explain X-ACCEPT-001` or `goatbuilder explain X-ACCEPT-001`.

## What it means

ops/federation/standup/acceptance.sh (phase 14) found no failure, but a required check was BLOCKED (its input or evidence was unavailable) or SKIPPED (not exercised). Such a check proves nothing, so the verdict in &lt;work>/evidence/acceptance/result.json is INCOMPLETE and the run exits 3, not 0.

## Why it happens

- a surface the check reads is not configured (for example the partner's REST endpoint or the SPIRE servers)
- the agreement offers nothing in one direction, so that direction's checks had nothing to send
- a probe session the check needs was not started

## What to do

One line per step or alternative:

```text
read the BLOCKED and skipped lines above the acceptance-counts line, or the why field of result.json; set what each names, then run phase 14 again
ops/scripts/peering-standup.sh <peering> --from 14
```

## Read more

- [`docs/design/federation-acceptance.md`](../../../docs/design/federation-acceptance.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

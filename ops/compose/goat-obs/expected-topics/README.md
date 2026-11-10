# Undeclared-topic monitor

The **topic-granular** arm of declared-state reconciliation — the sibling of the
[expected-vendor monitor](../expected-vendors/README.md), one level down. It
closes the gap recorded in
F-216:
the expected-vendor rules reconcile at the **anchor** layer (they sum the topic
dimension away), so an onboarded anchor publishing an **unregistered topic** under
its own prefix is invisible to them. The wire ACL grants the slot anchor its whole
`<anchor>/**` subtree
(F-181),
so the write lands without the topic ever being declared in the portal — the
actual-without-declared quadrant. This monitor reveals it.
Design: `docs/design/zenoh-write-enforcement-rollout.md`
§"General approach" / [ADR 1022](../../../../docs/adr/1022-zenoh-denial-observability.md).

## How it works

1. **`declared-topics.yml`** — declares, per anchor, the topics that *should*
   exist (the portal registry's declared set, as a deploy-carried snapshot).
2. **`generate.py`** renders it to **`textfile/goat_declared_topic.prom`** —
   committed, so the rsync deploy carries it. Each entry is reduced to the
   `<anchor>/<class>/*` form the audit-subscriber emits.
3. **node-exporter's textfile collector** (wired in `../docker-compose.yml`)
   exposes it as `goat_declared_topic{vendor,key_pattern}`.
4. **`../prometheus/rules/undeclared-topic.yml`** joins that against the
   audit-subscriber's `goat_audit_samples_total{vendor,key_pattern}`:
   - `TopicProducingUndeclared` — an anchor that *has* declared topics is
     publishing a topic-class that *isn't* declared (the F-216 gap). Scoped via
     `and on(vendor) goat_declared_topic` so it's the onboarded-anchor /
     undeclared-topic case, distinct from `UnexpectedVendorProducing`'s
     fully-unknown squat. `severity=info` until the declared set is complete
     (see below).

## Editing

```
$EDITOR declared-topics.yml        # add/remove a declared topic under an anchor
python3 generate.py                # regenerate the .prom
python3 generate.py --check        # CI guard: .prom matches the snapshot
git add declared-topics.yml textfile/goat_declared_topic.prom
# then re-deploy the obs stack (operator): ansible obs.yml — copy + compose up
```

Test the rule: `promtool test rules ../prometheus/tests/undeclared-topic.test.yml`.

## Granularity boundary (important)

The audit-subscriber's `key_pattern` label is reduced to `<anchor>/<class>/*` to
bound Prometheus cardinality. So this monitor catches an undeclared **class**
(`aprs/*` under an anchor that declared none) but not a finer drift *within* a
declared class (an anchor that declared `aprs/v1` but publishes `aprs/v2` — both
map to `aprs/*`). Full-key precision stays with the stethoscope registration
check and the Data-Access COP. This altitude catches the "publishing a whole
undeclared topic family" case, which is the one that matters for an alarm.

## Known v1 limitation

`declared-topics.yml` does **not** yet list every declared topic — it seeds the
streaming producers' known classes (hence `severity=info`). Hand-curating
declared topics is backwards: the portal **is** the registry. The durable fix is
to auto-derive this snapshot from the portal topic registry
(`GET /api/vendors/{v}/topics` / `goat topic export`) — tracked as a follow-on.
Until then, keep it synced with the producers. Mirrors the expected-vendor
`allowed`-set caveat exactly.

## Coverage scope

This monitor runs where the stack (audit-subscriber-fed Prometheus + these
rules) is deployed by `obs.yml`. A goatnet without it has no automated
declared-vs-actual detection at any granularity.

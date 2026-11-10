# Expected-vendor monitor

The runtime arm of declared-state reconciliation for the data fabric — the
operator-side silent-failure detector from [ADR 1022](../../../../docs/adr/1022-zenoh-denial-observability.md)
/ F-177.
Design: `docs/design/zenoh-write-enforcement-rollout.md` §"General approach".

## How it works

1. **`expected-vendors.yml`** — declares the expectation: `active` (should be
   streaming now → silence is an alert) + `allowed` (legitimately permitted to
   produce → not silence-alarmed).
2. **`generate.py`** renders it to **`textfile/goat_expected_vendor.prom`** —
   committed, so the rsync deploy carries it.
3. **node-exporter's textfile collector** (`--collector.textfile.directory`,
   wired in `../docker-compose.yml`) exposes it as
   `goat_expected_vendor{vendor,active}`.
4. **`../prometheus/rules/expected-vendor.yml`** joins that against the
   audit-subscriber's `goat_audit_samples_total{vendor}`:
   - `ExpectedVendorSilent` — an expected-active vendor produced nothing in 15m
     (the F-177 gate; fires even when the vendor's series is entirely absent).
   - `UnexpectedVendorProducing` — a vendor producing that isn't in the manifest
     (squat). `severity=info` until `allowed` is complete (see below).

## Editing

```
$EDITOR expected-vendors.yml      # add/remove a vendor
python3 generate.py               # regenerate the .prom
python3 generate.py --check       # CI guard: .prom matches the manifest
git add expected-vendors.yml textfile/goat_expected_vendor.prom
# then re-deploy the obs stack (operator): ansible obs.yml — copy + compose up
```

Test the rules: `promtool test rules ../prometheus/rules/expected-vendor.test.yml`.

## Coverage boundary (important)

`ExpectedVendorSilent` catches a vendor that **was producing and stopped** (cert
expiry, crash, network, or an ACL change/apply that starts denying a
previously-working vendor). It does **not** catch a vendor that **never
produced** — broken from its first write (wrong cert CN, ACL not applied, wrong
prefix), which is the likely shape of a permission misconfiguration. Such a
vendor isn't on `active` (it was never observed), and even if it were,
"misconfigured" is indistinguishable from "not started yet." The start-time case
is caught by **onboarding-time verification** (test-publish-on-mint) — see the
`zenoh-onboarding-publish-verification` track, not this monitor.

## Known v1 limitation

`allowed` does **not** yet list every onboarded vendor, so a dormant vendor
waking up trips `UnexpectedVendorProducing` (hence `severity=info`). The durable
fix is to auto-derive `allowed` from the declared pub/sub state
(`pubsub-state-as-code`) / the portal registry — tracked as a follow-on. Until
then, keep `allowed` synced with onboarding.

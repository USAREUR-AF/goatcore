# goat-moon-pod conformance smoke-test suite

| Field | Value |
|---|---|
| Status | Skeleton (Phase 2D-late / Phase 3 commitment) |
| Anchor | [`https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) |
| ADR | [`../../docs/adr/0223-goat-moon-pod-repo-containment.md`](../../docs/adr/0223-goat-moon-pod-repo-containment.md) |
| Implementation tracker | `../../docs/project/implementation-plan.md` Block 60A-extension |

## What this is

A conformance smoke-test suite that walks the 24-hour deployment path against a candidate goat-moon-pod and asserts each of the seven requirements at [`https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md).

Runs against:

- `goat-moon-open-pod` — continuously in CI on every commit to that repo.
- Vendor-built moon-pods — vendors run locally to self-check before requesting evaluation.
- Partner-built moon-pods — same as vendor-built.
- Goat-side at partnership-onboarding time — we run it as part of conformance evaluation.

## Status

Skeleton only at this commit. Full implementation lands as Block 60A-extension per `../../docs/project/implementation-plan.md`. The skeleton documents the test shape; the test logic lands when the reference pod is far enough along to test against.

## Test shape (planned)

Seven test groups, one per requirement:

```
tests/moon-pod-conformance/
├── README.md                         # this file
├── run.sh                            # entry point: ./run.sh <pod-endpoint>
├── 01-identity-assurance/            # requirement 1
│   ├── README.md
│   ├── oidc-discovery-reachable.sh
│   ├── ial2-aal2-claim-present.sh
│   └── partner-identity-binding.sh
├── 02-network-transport/             # requirement 2
│   ├── README.md
│   ├── wg-or-ipsec-or-mtls.sh
│   ├── silent-at-public-boundary.sh
│   └── sovereign-key-custody.sh
├── 03-landing-interface/             # requirement 3
│   ├── README.md
│   ├── release-namespace-discoverable.sh
│   ├── adapter-documented.sh
│   └── schema-published.sh
├── 04-audit-retention/               # requirement 4
│   ├── README.md
│   ├── 90day-retention-claim.sh
│   ├── identity-tied-events.sh
│   └── query-shape.sh
├── 05-schema-declaration/            # requirement 5
│   ├── README.md
│   ├── attest-schema-present.sh
│   └── undeclared-publish-rejected.sh
├── 06-sovereignty-floor/             # requirement 6
│   ├── README.md
│   ├── data-on-partner-infra.sh
│   ├── identity-in-partner-idp.sh
│   ├── audit-in-partner-storage.sh
│   ├── e1-partner-net-coordination.sh
│   ├── e2-bridge-in-flight-only.sh
│   ├── e3-first-boot-rotated.sh
│   └── e4-pre-imaged-hardware.sh
├── 07-reversibility/                 # requirement 7
│   ├── README.md
│   ├── namespace-stable.sh
│   ├── idp-replaceable.sh
│   └── allowlist-tied-to-identity.sh
└── 99-twentyfour-hour-walk/          # end-to-end scenario
    ├── README.md
    └── walk.sh
```

Each test produces a structured pass/fail/inconclusive verdict plus a short diagnostic message. The 24-hour walk is the integration scenario: from `install.sh` through first message flow through audit-log entry.

## How to run (planned)

```bash
# Against goat-moon-open-pod (default):
./run.sh https://moon-pod.example.local

# Against a vendor moon-pod:
./run.sh --vendor example-vendor https://vendor-pod.example.com

# Against a partner-built moon-pod:
./run.sh --custom https://partner-pod.example.org
```

Output: structured JSONL of test results, plus a summary table. Designed to be stable across moon-pod implementations — same suite, same expectations.

## Verdict shapes

- **PASS**: requirement met at boundary.
- **FAIL**: requirement violated; diagnostic includes which clause and why.
- **INCONCLUSIVE**: test could not run (e.g., pod unreachable, pre-deployment-state insufficient); not a conformance verdict on its own.

A moon-pod with all PASS verdicts on requirements 1-7 plus the 24-hour walk is **Tier-0 conformant**. A moon-pod with any FAIL is not. INCONCLUSIVE results require investigation before declaring either way.

## Out of scope

- **Performance benchmarking.** This is a conformance test, not a load test. Performance expectations are at the four-facet level (per [ADR 0821](../../docs/adr/0821-mission-partner-sharing-pattern.md)).
- **Security pen-testing.** Conformance asserts the contract; pen-testing is a separate discipline (and partnership-specific).
- **Operational drill simulation.** Runbook drills happen at partnership-operational time, not in conformance.

## Trigger conditions to revise the suite

- **A new edge case for requirement 6 surfaces.** Add a corresponding `06-sovereignty-floor/eN-*.sh` test.
- **A vendor moon-pod ships with a structurally different shape that the existing tests can't cleanly evaluate.** Add tests; do not loosen existing tests to accommodate.
- **The contract at [`moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) gains a new requirement.** Add a new test group `08-*/`.

## References

- [`https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/moon-pod-conformance.md) — the contract under test
- [`../../docs/design/goat-moon-pod.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod.md) — abstract spec
- [`../../docs/design/goat-moon-pod-builder-guide.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/goat-moon-pod-builder-guide.md) — how vendors use this suite
- [`../../docs/adr/0223-goat-moon-pod-repo-containment.md`](../../docs/adr/0223-goat-moon-pod-repo-containment.md) — repo-containment + extensibility-bones decision

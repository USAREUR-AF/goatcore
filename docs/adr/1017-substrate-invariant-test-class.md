# ADR 1017 — substrate-invariant test class

**Status:** Proposed
**Date:** 2026-05-13
**Series:** discipline (sibling to ADR 1008 soak / 1012 load / 1011 findings / 1015 code-review)
**Repo scope (post-monorepo-split):** core (applies to every repo that ships substrate).

---

> **For the technical contributor:** every substrate — wg-cp0, Spock, signal-server
> federation, bundle format, work-CLI YAML schema, OCI image-distribution mesh,
> Zenoh router topology — declares a combinatorial invariant ("any peer X can
> reach any peer Y" / "any author A's update preserves any prior author B's
> field" / etc.) and ships a test that asserts it across the cross-product, not
> just for sampled pairs. The test is the substrate's standing acceptance gate.
> CI runs it on every commit touching the substrate. A regression of the
> invariant is a release-blocking failure, regardless of which sub-block
> introduced it.
>
> **For the layperson sponsor:** the system is built from a stack of substrates
> — encrypted tunnels, replicated databases, signed bundles. Each substrate is
> a foundation; everything above it relies on the foundation behaving the same
> way no matter who's using it. Until now we tested foundations by checking
> "does this one combination work?" and shipping when it did. Real-world use
> revealed combinations we never tested, where the foundation behaved differently
> — sometimes silently, until a real workload hit the gap. This ADR codifies a
> standing test class: every substrate has to ship a "test every combination"
> sweep alongside it. Same idea as a building inspection that doesn't just
> check one corner of the foundation but walks every load-bearing wall.
>
> **For the SCA-R:** combinatorial-invariant tests are a structural assurance
> against latent defects of the form "X works for the configurations we
> tested; fails silently for configurations we didn't think to test." Required
> as a precondition for substrate-level acceptance per ADR 1015 Tier-1 review
> + ADR 1008 soak. Composes with the existing test discipline ADRs to give
> a layered assurance posture: unit tests for components, integration tests
> for pairs, soak tests for sustained behavior, load tests for capacity,
> *substrate-invariant tests for combinatorial correctness*.

---

## Context

The 2026-05-12 + 2026-05-13 sequence of substrate-level findings —
F-114 (wg-cp0
rotation drift), F-115
(substrate propagation gap), and F-116
(asymmetric `/24` ACL inversion) — share a shape that the existing
test discipline does not cover.

Each was a property of the substrate **across the combination of all
peers / fields / states**, not a property of any one component in
isolation. Unit tests caught individual function behavior. Integration
tests caught individual pair-wise behavior. Soak tests caught
sustained-load behavior on the tested configuration. Load tests caught
saturation behavior of the tested component. None of them caught the
combinatorial property "every peer in the fleet can reach every other
peer over wg-cp0" because that property was never explicitly tested
across the cross-product.

The same pattern produced the work-CLI Summary-clobber bug
(2026-05-13): a struct field that wasn't in the schema was silently
dropped on every load → save round-trip. No test asserted the
combinatorial invariant "every field that has ever appeared in
active-work.yaml survives a round-trip." The bug was fixed; the test
class that would have caught it was added in the same commit.

These are not the only examples. Looking back at the findings register,
several closed findings followed the same pattern:

- F-093 (Docker data-root + Spock unbounded WAL): two interacting
  invariants violated silently on a configuration nobody tested
  combinatorially.
- F-094 (Orin-C not on wg-cp0): combinatorial-mesh-membership invariant
  ("every mgmt-host is a wg-cp0 peer") not tested.
- F-097 (distroless health probes): "every container in the stack has
  a working healthcheck" invariant not tested across the cross-product
  of (image, healthcheck-command).
- F-098 (snitch probe hangs on missing netbird socket): "probe doesn't
  hang on any precondition shape" invariant not tested across the
  cross-product of (preconditions present, preconditions absent).

The cumulative cost of catching these one at a time, post-deployment,
is the discipline gap this ADR closes.

## Decision

**Every substrate in the project declares a combinatorial invariant
and ships a test that asserts it across the cross-product of inputs.**
Substrate-invariant tests become a required test class, sibling to
soak (ADR 1008) and load (ADR 1012).

### What qualifies as a substrate

A *substrate* is a load-bearing system that other things compose on
top of, with a property that must hold across all participants /
configurations / values. Concrete examples in scope today:

- **wg-cp0 mesh** — invariant: every peer can reach every other peer.
- **Spock replication** — invariant: every write at peer X is visible
  at every other peer Y within bounded time (for replicated tables).
- **Signal-server federation** — invariant: every peer is reachable
  via Forward RPC from every other peer in its federation set.
- **Offline-CA bundle format** — invariant: every CBOR field that has
  ever been written to a bundle is decoded correctly by every consumer
  of bundles.
- **active-work.yaml schema** (work CLI) — invariant: every field that
  has ever appeared in the file survives load → mutate → save.
- **OCI image-distribution mesh** (Block 75) — invariant: every image
  in the canonical set is available from every tier (T0 / T1 / T2) at
  any time after sync convergence.
- **Zenoh router topology** (Block 23 / ADR 0844) — invariant: every
  publication on topic T at peer X is received at every subscriber for
  T at peer Y, modulo declared partition/latency tolerance.

A *substrate* is NOT the same as a *service* or a *role*. A service
that runs in isolation (e.g., `gatewatch`, `airdefenders`) does not
need a substrate-invariant test. A service that depends on a substrate
inherits the substrate's tests; it does not need to re-implement them.

### What the test must cover

For each substrate, the test:

1. **Names the invariant** in plain text at the top of the test
   suite. The invariant is the contract; the test asserts it.
2. **Exercises the cross-product** of relevant inputs, not just a
   sampled subset. For wg-cp0 with N peers, that's N×(N-1) directed
   pairs. For active-work.yaml schema with M fields, that's all M
   fields in one fixture, asserted survivable through load → save.
3. **Runs in a bench environment** (cheap, ephemeral, sovereign). The
   wg-cp0 test runs on a 2-node EC2 substrate at ~$0.10/hr for the
   duration of the test. The work-CLI test is a Go unit test against
   the yaml library.
4. **Reports green / red as a release-blocking gate.** Every commit
   touching the substrate runs the test. A regression is treated as
   a hard failure, not a warning.

### What the test does NOT cover

Substrate-invariant tests are scoped to the **combinatorial property
of the substrate**, not to load characteristics (covered by ADR
1012) or sustained-behavior characteristics (covered by ADR 1008).
The three layers compose:

| ADR | Tests | Catches |
|---|---|---|
| 1008 (soak) | Behavior over time at production-like rates | Memory leaks, slow degradation, drift |
| 1012 (load) | Behavior under saturation | Throughput ceilings, latency tails, capacity-loss recovery |
| **1017 (this) (substrate-invariant)** | Property across combinations | Asymmetric configs, missing-field drops, untested-pair gaps |

A substrate that ships needs all three. None substitutes for the
others.

### Authorship discipline

- When a new substrate lands, its first commit MUST include the
  invariant statement + test. "We'll add the test later" is not
  permitted — it is the test class's existence that gives the
  substrate's invariant teeth.
- When a sub-block is added on top of an existing substrate, the
  sub-block author runs the substrate-invariant test in the
  block-completion checklist (per ADR 1015) and cites the result.
- When a finding is filed against a substrate, the finding's "How
  it gets durably closed" section MUST identify whether an existing
  substrate-invariant test should have caught it (and was broken)
  or whether a new substrate-invariant test should be added. The
  Block-22R asymmetric `/24` finding is the reference case:
  the test didn't exist; the closure adds it.

## Surveying prior art before primitive adoption

A close sibling discipline rule, encoded here so it has somewhere
to live:

**When picking an upstream tool for a non-dominant pattern, the
design doc must cite either (a) the upstream's documentation for
that pattern, or (b) the project that solves that pattern + what
we are keeping vs. replacing.**

F-116's root miss was using WireGuard for hub-and-spoke forwarding
without surveying the prior-art ecosystem (Tailscale / NetBird /
wg-meshconf / Headscale / ZeroTier) for the well-known patterns
that solve that specific use-case. The rule above closes the gap
by making prior-art citation a design-doc precondition for
adopting any primitive outside its dominant pattern.

Promoted to a feedback memory rule in the same change-set:
`feedback_primitive_outside_dominant_pattern.md` — every operator
session inherits it.

## Consequences

### Positive

- Combinatorial invariants are surfaced as code that can fail loud,
  not as implicit operator-side knowledge that fails silent.
- Future substrate-touching commits inherit the discipline — a
  regression of a known invariant is caught at PR time, not at the
  next time someone hits an untested combination.
- The substrate-invariant test class is **cheap to author and run**
  for most substrates (the wg-cp0 test is a `for src; for dst; ping`
  matrix; the active-work.yaml test is a Go unit test). The
  acceptance cost is bounded.

### Neutral

- Adds a new test class to the project's test pyramid. Composes with
  existing ADRs (1008 / 1012 / 1015), does not replace any.
- Substrate authors carry one additional discipline at substrate
  authorship time. The cost is paid once per substrate; subsequent
  sub-block authors inherit the discipline.

### Negative

- Substrates whose combinatorial invariant is expensive to compute
  end-to-end (e.g., "every Spock replication edge across a 100-peer
  fleet") may require a representative-sample variant of the test
  rather than the literal cross-product. The ADR allows this with
  a documented sampling rationale; the cost is the sampling
  rationale itself becomes part of the test artifact.

## Alternatives considered

### A. Status quo — pair-wise integration tests + soak + load

Rejected. The 2026-05-12 + 2026-05-13 sequence demonstrates that
pair-wise tests + soak + load do not catch combinatorial invariants
that the cross-product would expose. Cost is one major substrate
incident per quarter at current substrate count; recurring.

### B. Require formal verification of substrate invariants

Rejected as over-investment. The cost of formal verification
(specifying invariants in a tool like TLA+ or Coq, building model
checkers, etc.) exceeds the cost of "run the cross-product test on
every commit." For the small substrate count we have today, the
empirical test approach is the right altitude.

### C. Wait until a substrate fails before adding a test for it

Rejected. This is the de-facto status quo; the cost is being paid
recurringly. Catching the next one *post-hoc* costs the same as
catching this one; the discipline upgrade is the savings.

## Implementation

This ADR is design-only. Per-substrate implementation lands as part
of the substrate's own implementation block, not as a separate
fleet-wide rollout. Tracking:

- **First instance** — wg-cp0 combinatorial mesh-reach test lands as
  part of Block 22R.0 (per ADR 0321). Acceptance criterion for 22R.0.
- **Second instance** — active-work.yaml field-preservation test
  lands 2026-05-13 in the same change-set as the work-CLI bug fix
  (commit `7e3c134f`). Already in the test suite.
- **Subsequent substrates** — each substrate's owner adds its
  invariant test as part of the next sub-block authoring against
  that substrate. Block-completion checklist (ADR 1015) gains a row
  asking whether the substrate's invariant test exists + is green.

## Related

- [ADR 1008 — soak-testing discipline](1008-soak-testing-discipline.md) — sibling test class.
- [ADR 1012 — load-testing discipline](1012-load-testing-discipline.md) — sibling test class.
- [ADR 1011 — findings register](1011-findings-register.md) — the substrate of findings; ADR 1017 is the prevention substrate.
- [ADR 1015 — code-review discipline](1015-code-review-discipline.md) — substrate-invariant-test row added to the block-completion checklist.
- [ADR 0321 — wg-cp0 is a perimeter, not an ACL layer](0321-wg-cp0-perimeter-not-acl.md) — first ADR whose acceptance criterion is a substrate-invariant test (the combinatorial mesh-reach test).
- F-114 / F-115 / F-116 — origin findings that motivated this ADR.
- [docs/process/block-completion-checklist.md](../process/block-completion-checklist.md) — gains the substrate-invariant-test row.

---

*See `GLOSSARY.md` for terms and acronyms.*

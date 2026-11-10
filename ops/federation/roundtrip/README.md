# Round-trip validation — the last federation check, and the only one a publisher can make alone

Every other federation assertion we have requires shell on **both** sides. `ops/federation-drill/`
reads the partner's store directly; the smoke harness inspects both routers. A real publisher has
neither, so until now the sending side had no way to learn whether anything it published ever
arrived.

**A round trip closes that gap.** The publisher stages a nonce under one of its release sets, its
republisher lifts it onto its wire name, and it waits for the counterparty's acknowledgement to come
back under theirs. A pass means four things at once:

1. the sample crossed the peering,
2. the **partner's own router** delivered it to a process on their side,
3. the payload arrived unmangled — the ack carries a digest of what they received,
4. the answer returned across the same agreement.

It is the last stage because it is the only one that fails when any earlier stage silently
half-works. A link can be ESTABLISHED, authenticated, and carrying zero data with no denial logged
; one-way checks pass in several of those states and a round trip does not.

## It needs no additional access

The check rides entirely on the two grants the agreement already makes:

| Leg | Key | Grant it uses |
|---|---|---|
| probe out | staged `release/<set>/fedcheck/<nonce>`, crosses as `from/<us>/<set>/fedcheck/<nonce>` | our republisher, then our egress — our own wire name |
| ack back | staged `release/<their set>/fedcheck-ack/<nonce>`, arrives as `from/<them>/<their set>/fedcheck-ack/<nonce>` | their republisher, then our ingress — their landing prefix |

No new ACL rule, no new subject, no widening. If the check cannot pass, the agreement's own lanes
are not working — which is exactly what it exists to report.

## Running it

On the receiving sovereign, once:

```bash
python3 responder.py from/<them> release/<one of our sets>   # e.g. from/zagros release/weather
```

On the publishing sovereign, whenever you want the answer:

```bash
./fedcheck.sh release/<one of our sets> from/<them> [timeout]   # e.g. release/weather from/xaa 20
```

Phase 14 (`ops/federation/standup/acceptance.sh`) runs it once per set we offer, and, when the
partner's surface is reachable (a drill), once per set they offer in the other direction. It
streams `fedcheck.sh` to the host that runs it (`bash -s` over ssh); nothing is copied there.

## Measured

On a live two-region peering:

| State | Result |
|---|---|
| healthy | **PASS**, 538–539 ms |
| partner's responder stopped | **FAIL** in 8 s |
| peering edge stopped | **FAIL** in 8 s |
| edge restarted | **PASS** again, 538 ms |

The two failures matter as much as the pass: a check that cannot fail is not a check. Both
are states in which a one-way assertion still reports success.

## What it does not prove

- **Not a liveness monitor for the release set.** It proves the lanes carry; it says nothing about
  whether the data you meant to publish is being published.
- **Not an integrity guarantee end to end.** The digest covers the payload as the responder received
  it. A party that wanted to lie about receipt could.
- **Not a substitute for the refusal checks.** It proves what crosses, never what must not; keep
  asserting that an ungranted prefix stays out.

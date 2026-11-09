# Federation acceptance: what phase 14 proves, and when it may say so

| Field | Value |
|---|---|
| Status | **Built:** `ops/federation/standup/acceptance.sh` (phase 14 of both avenues, `goatbuilder federate` and `ops/scripts/peering-standup.sh`); the exit rule in §3; the in-process test fabric `ops/federation/standup/acceptance_fakefabric.py` and its tests `test_acceptance.py`. The hermetic federation end-to-end test (`ops/federation-e2e/run.sh`) runs the four tiers of §2 from both sides with every check configured; it was not re-run for this change. **Not built:** a versioned result bundle, checks generated for one-way agreements, and bilateral evidence exchange |
| Companions | [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) Steps 7 and 8 (the same checks by hand), [`ops/federation/roundtrip/README.md`](../../ops/federation/roundtrip/README.md) (the round trip), [`deployment-e2e.md`](deployment-e2e.md) §"The same pattern for a federation" |
| Audience | An operator running phase 14; whoever changes `acceptance.sh` |

Status: WIP — gate: two real environments complete phase 14 from both sides and both exit 0.

## 1. The job

Phase 14 is the last gate of a peering standup. It answers one question: does the peering
carry what the agreement grants, refuse what it does not, and can the publishing side prove it.
Every key expression it uses is read from the agreement (`<work>/agreement.json`), never typed
into the script, so the suite agrees with the peering and not with itself.

## 2. The four tiers

| Tier | Checks | Needs |
|---|---|---|
| 2 positive | a sample staged under each offered `release/<set>/...` on the sending core reads back under `from/<sender>/<set>/...` from the receiving core's store, both directions | both cores' REST surfaces (`FED_OURS_REST`, `FED_THEIRS_REST`) |
| 3 negative | an unoffered staging set is not lifted; a write outside the agreement does not cross; a session holding the partner's agreed identity is allowed in its lane and denied outside it, read from our edge's ACL trace; a forged name and a certificate bearing our core's name match no ACL subject | tier 2 passed, plus `FED_EDGE_LOG_OURS` and the probe sessions |
| 4 round trip | a nonce staged under each of our sets is acknowledged by the partner's responder | our core only |
| SPIRE | our server refreshes and holds the partner's bundle; their workload chains to it and not to ours | `FED_SPIRE_OURS`, `FED_SPIRE_THEIRS` |

Tier 1 (each phase's own end state) belongs to the phases.

## 3. Outcome and exit status

A check that runs prints `pass` or `FAIL`. A check whose inputs are missing, or that would only
pass vacuously because a delivery failed, prints `skip`. A skip proves nothing.

| Exit | Meaning |
|---|---|
| 0 | every check ran and passed |
| 1 | at least one check failed |
| 3 | none failed, at least one was skipped: the peering is not proven |
| 2 | the agreement could not be read |

The last line before the verdict is always `acceptance-counts: pass=N fail=N skip=N as=<us>`.

**Built** (this change): a skip exits 3. Before it, a run with skips exited 0 unless
`FED_ACCEPTANCE_STRICT=1` was set, so a production run with no counterparty surface passed the
gate on a single tier. `FED_ACCEPTANCE_STRICT` is still accepted and changes nothing.

A production operator holds only their own side, so a production phase 14 now exits 3 until
the partner's half is proven too. That is the intended reading: one side alone has not shown
that the peering works. The bilateral exchange that closes it is not built.

## 4. Testing without hosts

`acceptance_fakefabric.py` serves two cores' REST surfaces, both republishers, the round-trip
responder, three probe sessions judged by an edge that writes zenoh-shaped ACL trace lines,
and a `spire-server` stand-in backed by CAs made with openssl. Each part can be broken on
purpose (`Faults`). `test_acceptance.py` runs the real script against it with every check
configured (exit 0), with inputs removed (exit 3), and with a broken republisher or responder
(exit 1). `FED_ACCEPTANCE_WAIT`, `FED_ACCEPTANCE_GAP` and `FED_ACCEPTANCE_RT_TIMEOUT` shorten
the waits for the test; a real edge needs the defaults.

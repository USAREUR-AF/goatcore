# Federation acceptance: what phase 14 proves, and when it may say so

| Field | Value |
|---|---|
| Status | **Built:** `ops/federation/standup/acceptance.sh` (phase 14 of both avenues, `goatbuilder federate` and `ops/scripts/peering-standup.sh`); the exit rule in §3; the evidence bundle in §4 (`ops/federation/standup/acceptance_evidence.py`, unit tests `test_acceptance_evidence.py`); the in-process test fabric `ops/federation/standup/acceptance_fakefabric.py` and its tests `test_acceptance.py`. The hermetic federation end-to-end test (`ops/federation-e2e/run.sh`) runs the four tiers of §2 from both sides with every check configured; it was not re-run for this change. **Not built:** checks generated for one-way agreements, a required-check policy file, and bilateral evidence exchange (session manifest, exchanged bundles, report signing) |
| Companions | [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) Steps 7 and 8 (the same checks by hand), [`ops/federation/roundtrip/README.md`](../../ops/federation/roundtrip/README.md) (the round trip), [`deployment-e2e.md`](deployment-e2e.md) §"The same pattern for a federation" |
| Audience | An operator running phase 14; whoever changes `acceptance.sh` |

Status: WIP — gate: two real environments complete phase 14 from both sides and both exit 0.

## 1. The job

Phase 14 is the last gate of a peering standup. It answers one question: does the peering
carry what the agreement grants, refuse what it does not, and can the publishing side prove it.
Every key expression it uses is read from the agreement (`<work>/agreement.json`), never typed
into the script, so the suite agrees with the peering and not with itself.
It accepts only well-formed names from the agreement: sovereign ids `[a-z0-9][a-z0-9-]{1,15}`,
trust domains `spiffe://[A-Za-z0-9._-]+`, release entries `release/[A-Za-z0-9._/*-]+`; anything
else stops the run with exit 2 before any command. Each argument is quoted with `printf %q`
before it joins a command string run here or on a core host.

## 2. The four tiers

| Tier | Checks | Needs |
|---|---|---|
| 2 positive | a sample staged under each offered `release/<set>/...` on the sending core reads back under `from/<sender>/<set>/...` from the receiving core's store, both directions | both cores' REST surfaces (`FED_OURS_REST`, `FED_THEIRS_REST`) |
| 3 negative | an unoffered staging set is not lifted; a write outside the agreement does not cross; a session holding the partner's agreed identity is allowed in its lane and denied outside it, read from our edge's ACL trace; a forged name and a certificate bearing our core's name match no ACL subject | tier 2 passed, plus `FED_EDGE_LOG_OURS` and the probe sessions |
| 4 round trip | a nonce staged under each of our sets is acknowledged by the partner's responder | our core only |
| SPIRE | our server refreshes and holds the partner's bundle; their workload chains to it and not to ours | `FED_SPIRE_OURS`, `FED_SPIRE_THEIRS` |

Tier 1 (each phase's own end state) belongs to the phases.

## 3. Outcome and exit status

Every check ends with one of five outcome words:

| Outcome | Meaning |
|---|---|
| PASS | the expected evidence was observed |
| FAIL | a violation was observed |
| BLOCKED | a prerequisite or the evidence was unavailable (no counterparty surface, no ACL trace, no round-trip runner) |
| SKIP | not exercised (a refusal not attempted because a delivery failed, a negative certificate or SPIRE check not configured) |
| NOT APPLICABLE | excluded by the agreed scope; never a required check, always with a reason |

The run's verdict and exit status come from `acceptance_evidence.py` alone:

| Exit | Verdict | Meaning |
|---|---|---|
| 0 | PASS | every required check passed |
| 1 | FAIL | a check failed |
| 3 | INCOMPLETE | none failed, but a required check was BLOCKED or SKIPPED, or no required check ran (catalog code X-ACCEPT-001; both avenues' failure text names it) |
| 2 | | the agreement could not be read, or the evidence could not be written (including one check whose record failed: the bundle would lack it) |

A required check becomes PASS only by passing. No flag turns FAIL, BLOCKED or SKIP into PASS.
Every check is required today.

The counts line keeps its first four fields for the readers that grep it:
`acceptance-counts: pass=N fail=N skip=N as=<us> blocked=N not_applicable=N`.

**Built:** a check that did not run exits 3. Before this, a run with skips exited 0 unless
`FED_ACCEPTANCE_STRICT=1` was set, so a production run with no counterparty surface passed the
gate on a single tier. `FED_ACCEPTANCE_STRICT` is still accepted and changes nothing.

A production operator holds only their own side, so a production phase 14 now exits 3 until
the partner's half is proven too. That is the intended reading: one side alone has not shown
that the peering works. The bilateral exchange that closes it is not built.

## 4. The evidence bundle

**Built.** Each run writes `<dir of the agreement>/evidence/acceptance/` (the driver's
`<work>/evidence/acceptance/`; `FED_EVIDENCE_DIR` overrides it):

| File | What |
|---|---|
| `result.json` | the last run, schema `goat-federation-acceptance/v1` |
| `summary.md` | the same, one table row per check, for a person |
| `history/<run id>.json` | every run, kept |

`result.json`:

```json
{
  "schema": "goat-federation-acceptance/v1",
  "run": {"id": "acc-20261109T120000Z-4242", "started_at": "...", "ended_at": "...",
          "us": "xaa", "them": "xab", "observed_by": "xaa",
          "runner": "ops/federation/standup/acceptance.sh"},
  "agreement": {"path": "...", "sha256": "sha256:...", "spec": "federation-agreement/v1", "expires": "..."},
  "inputs": {"theirs_rest": true, "edge_log_ours": true, "probe_forged": false, "spire": true},
  "counts": {"PASS": 14, "FAIL": 0, "BLOCKED": 0, "SKIP": 1, "NOT APPLICABLE": 0},
  "verdict": "INCOMPLETE",
  "exit": 3,
  "why": "1 required check(s) did not run: refusal/forged-name",
  "checks": [
    {"name": "delivery/inbound/release/all", "title": "...", "tier": "positive",
     "direction": "inbound", "required": true, "verdict": "PASS", "reason": "...",
     "started_at": "...", "ended_at": "...",
     "observations": {"key": "from/xab/all/acceptance/<run id>", "sent": "in-<run id>", "received": "in-<run id>"}}
  ]
}
```

`direction` is `outbound` (ours to theirs), `inbound` (theirs to ours), `both` or `local`.
`inputs` names which surfaces the run had, as booleans only: an address is not evidence and
does not belong in a file that may be handed to the partner. `observations` holds what the
check saw: the key, the bytes sent and read back, the ACL trace lines, the round-trip output.
Check names are stable (`delivery/<direction>/<set>`, `republisher-boundary/<direction>`,
`out-of-lane/<direction>`, `refusal/<case>`, `roundtrip/<direction>/<set>`, `spire/<case>`), so
two runs and two parties' bundles can be compared row by row.

The bundle records local observations only (`observed_by`). Keeping a partner's reported
results beside it, distinctly labeled, is not built.

## 5. Testing without hosts

`acceptance_fakefabric.py` serves two cores' REST surfaces, both republishers, the round-trip
responder, three probe sessions judged by an edge that writes zenoh-shaped ACL trace lines,
and a `spire-server` stand-in backed by CAs made with openssl. Each part can be broken on
purpose (`Faults`). `test_acceptance.py` runs the real script against it with every check
configured (exit 0, every check PASS in `result.json`), with inputs removed (exit 3, BLOCKED or
SKIP rows), with a broken republisher or responder (exit 1), and with an unwritable evidence
directory (exit 2). `FED_ACCEPTANCE_WAIT`, `FED_ACCEPTANCE_GAP` and `FED_ACCEPTANCE_RT_TIMEOUT` shorten
the waits for the test; a real edge needs the defaults.

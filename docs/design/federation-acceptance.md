# Federation acceptance: what phase 14 proves, and when it may say so

| Field | Value |
|---|---|
| Status | **Built:** `ops/federation/standup/acceptance.sh`, phase 14 of both avenues (`goatbuilder federate` and `ops/scripts/peering-standup.sh`): the checks generated from the agreement's flows (§2), the outcome and exit rule (§3), the evidence bundle (§4, `ops/federation/standup/acceptance_evidence.py`, unit tests `test_acceptance_evidence.py`), and the in-process test fabric with the tests that run the real script against it (§5, `acceptance_fakefabric.py`, `test_acceptance.py`). The hermetic federation end-to-end test (`ops/federation-e2e/run.sh`) runs phase 14 from both sides with every check configured; it has not been run on this change. **Not built:** a required-check policy file, persistent-reception inspection of storage scope, checks of existing partnerships before and after, run-owned probe sessions with cleanup, and bilateral evidence exchange (session manifest, exchanged bundles, report signing) |
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

## 2. The checks, generated from the agreement

The script reads the parties, the offered release sets per direction and the agreed rates from
the agreement, and generates one check per direction and set. **Built.**

| Check (name) | Direction | Evidence | Needs |
|---|---|---|---|
| `delivery/<dir>/<set>` | each offered set, each direction | a sample staged under `release/<set>/acceptance/<run id>` on the sending core reads back under `from/<sender>/<set>/...` from the receiving core's store, byte for byte (both SHA-256 digests recorded) | both cores' REST surfaces |
| `fresh-keys` | both | before anything is sent, every target key is readable and empty; the run id (time, process id, 32 random bits) is in every key and payload | the same |
| `republisher-boundary/<dir>` | each direction the agreement has | a set the agreement never offered, staged beside the offered ones that just crossed, is not lifted | tier 2 passed |
| `out-of-lane/<dir>` | both | a write outside any wire name does not cross | tier 2 passed |
| `refusal/control-allowed` | inbound | a session holding the partner's agreed identity writes in its lane: ALLOWED in our edge's ACL trace and read back on our core | `FED_EDGE_LOG_OURS`, `FED_PROBE_THEIRS_REST`, an inbound lane |
| `refusal/out-of-lane-denied`, `refusal/spoof-denied` | inbound | the same session writing to `ops/**` and under our wire name: DENIED in the trace. Asserted only beside a control that landed; otherwise SKIP | the same |
| `refusal/forged-name`, `refusal/impersonation` | inbound | a certificate from their CA with a name nobody agreed, and one bearing our core's name: matched no ACL subject, zero authorizations, nothing landed | the probe sessions and their zenoh ids |
| `roundtrip/<dir>/<set>` | each direction with a return path | a nonce staged under each set is acknowledged by the far side's responder with the digest sent | outbound: our core only; inbound: their surface |
| `spire/refresh`, `spire/bundle-held`, `spire/svid-chains` | inbound | our server refreshes and holds their bundle; their workload chains to it and not to ours | `FED_SPIRE_OURS`, `FED_SPIRE_THEIRS` |

Rules that keep the checks honest:

- **A direction the agreement does not have is NOT APPLICABLE, with its reason.** A send-only
  or receive-only agreement passes on the checks its flows define. A round trip needs a
  return path, so it is NOT APPLICABLE when either direction is missing. With no inbound lane
  our edge grants the partner nothing, so there is no allowed control for a denial and the
  three `refusal/` rows that need one are NOT APPLICABLE; the forged-name and impersonation
  rows still run. An agreement with no flow at all fails.
- **A release pattern with a wildcard before its tail** names no single key to publish; its
  delivery row is BLOCKED, never silently left out.
- **An empty read means absent.** `rest-value` returns nothing both when the store holds no
  row and when a row's stored value is empty; every value the runner writes is non-empty, so for
  the runner's own keys the two cannot be confused.
- **A REST surface that does not answer is BLOCKED, never a pass.** Every read parses the JSON
  answer and the HTTP status (`acceptance_evidence.py rest-value`). Before this change a failed
  read printed nothing, which is what every refusal check wanted to see, and the value was
  found with a grep that missed `"value": "..."` written with a space.
- **Reads are queries against the receiving core's store.** The runner declares no subscriber,
  so no diagnostic interest can carry a sample the store would not have taken. Inspecting the
  effective storage scope, and publishing after test subscribers stop, is not built.
- **Rates.** Samples are spaced by the agreement's lowest `hz` (at least 0.5 seconds), because
  the receiving edge drops a second sample inside its downsampling interval with nothing
  logged. A delivery read retries up to eight times, one second apart.
- **Where the probes run.** From the operator's machine. Reads and writes go through
  `FED_SSH_OURS` / `FED_SSH_THEIRS` when set (curl runs on the core host), else directly. The
  round-trip script is streamed over that ssh with `bash -s`; nothing is copied to a host.
  The probe sessions and the responders are started by whoever runs the drill (the end-to-end
  test starts them); run-owned, time-limited probes with cleanup are not built.

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
responder on each core, three probe sessions judged by an edge that writes zenoh-shaped ACL
trace lines, and a `spire-server` stand-in backed by CAs made with openssl. Each part can be
broken on purpose (`Faults`). `test_acceptance.py` runs the real script against it:

| Case | Expected |
|---|---|
| everything configured and working | exit 0, every row PASS in `result.json`, both round-trip directions |
| no counterparty surface; no ACL trace; SPIRE not configured | exit 3, BLOCKED or SKIP rows, the outbound round trip still PASS |
| a republisher down; a responder down | exit 1 |
| a REST surface answering HTTP 500 | no negative row PASS; delivery BLOCKED |
| a surface answering every key with an old value | `fresh-keys` FAIL |
| other bytes on arrival | FAIL with both digests |
| their republisher lifting an unoffered set | `republisher-boundary/inbound` FAIL |
| an edge that logs no ACL decisions | the control FAIL, the denials SKIP |
| send-only, receive-only, no flow | NOT APPLICABLE rows with reasons and exit 0; exit 1 for no flow |
| an unwritable evidence directory | exit 2 |

`FED_ACCEPTANCE_WAIT`, `FED_ACCEPTANCE_GAP` and `FED_ACCEPTANCE_RT_TIMEOUT` shorten the waits
for the test; a real edge needs the defaults.

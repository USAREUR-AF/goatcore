# Federation acceptance: what phase 14 proves, and when it may say so

| Field | Value |
|---|---|
| Status | **Built:** `ops/federation/standup/acceptance.sh`, phase 14 of both avenues (`goatbuilder federate` and `ops/scripts/peering-standup.sh`): the checks generated from the agreement's flows (§2), persistent reception (§2.1), refusals tied to the control's session (§2.2), existing partnerships before and after, opt-in (§2.3), run-owned probes with cleanup, opt-in (§2.4), the outcome and exit rule (§3), the evidence bundle (§4, `ops/federation/standup/acceptance_evidence.py`, unit tests `test_acceptance_evidence.py`), and a bilateral session exchanged by file (§6, `acceptance_session.py`, tests `test_acceptance_session.py`), in which each side observes the partner's refusals and SPIRE trust on its own surfaces (§6.4), publishing `exchanging` only as a proposed, reviewable change (§7, `acceptance_session.py propose` and `freshness`, `ops/federation/validate-partners.py`), and the in-process test fabric with the tests that run the real script against it (§5, `acceptance_fakefabric.py`, `test_acceptance.py`). The hermetic federation end-to-end test (`ops/federation-e2e/run.sh`) runs phase 14 from both sides with every check configured; it has not been run on this change. **Not built:** a required-check policy file, the driver running `--before` ahead of phase 11 and naming the other peerings itself, a launcher for the test environment's probes, ssh host-key verification by default, phase 14 on a session both sides derive as the default (it is built behind `federate test --bilateral`, §6.2, §6.3), the command publishing and verifying a partner's signed result (the signing library is built, §6.1), and any automatic bilateral gate |
| Companions | [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) Steps 7 and 8 (the same checks by hand), [`ops/federation/roundtrip/README.md`](../../ops/federation/roundtrip/README.md) (the round trip), [`deployment-e2e.md`](deployment-e2e.md) §"The same pattern for a federation" |
| Audience | An operator running phase 14; whoever changes `acceptance.sh` |

Status: not measured: the gate names two real environments, so it waits on goatbuilder 1.0 gate row B9 — gate: two real environments complete phase 14 from both sides and both exit 0.

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
| `persistence/<dir>/<set>` | each offered set, each direction | only the receiving core's own store subscribes to the exact key of a second sample (no other session does), and that sample reads back from the store (§2.1) | both cores' REST surfaces and admin space read |
| `republisher-boundary/<dir>` | each direction the agreement has | a set the agreement never offered, staged beside the offered ones that just crossed, is not lifted | tier 2 passed |
| `out-of-lane/<dir>` | both | a write outside any wire name does not cross | tier 2 passed |
| `refusal/control-allowed` | inbound | a session holding the partner's agreed identity writes in its lane: ALLOWED in our edge's ACL trace and read back on our core; the session id on the allow line is recorded, and must equal `FED_PROBE_THEIRS_ZID` when that is set (§2.2) | `FED_EDGE_LOG_OURS`, `FED_PROBE_THEIRS_REST`, an inbound lane |
| `refusal/out-of-lane-denied`, `refusal/spoof-denied` | inbound | the same session writing to `ops/**` and under our wire name: DENIED in the trace for the control's own session id. Asserted only beside a control that landed; otherwise SKIP (§2.2) | the same |
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
- **Reads are queries against the receiving core's store.** The runner declares no subscriber
  of its own. Whether anyone else's subscriber could have carried a sample is the persistence
  check's question (§2.1).
- **Rates.** Samples are spaced by the agreement's lowest `hz` (at least 0.5 seconds), because
  the receiving edge drops a second sample inside its downsampling interval; the router logs
  one INFO line for the lot, and each drop only at trace level
  ([ADR 1244](../adr/1244-a-lanes-accept-hz-is-its-aggregate-ceiling-and-the-sender-paces-to-it.md)). A delivery or
  persistence read retries up to eight times, one second apart. That spacing holds only among
  this runner's samples: a lane's other traffic shares the edge's one window per lane, and in
  a lab run an acceptance sample landing 6 ms behind a flow sample was dropped (shown in that
  run's edge log). So, as a stopgap until every writer on the lane paces to it, a read that
  finds nothing sends a fresh value one gap later, any value sent counts, and the row records
  `values_sent` and `resent`. A row stored only after a resend, or not stored at all, names
  the rule: at most one message per 1/hz on a lane, 10 ms at 100 Hz (`acceptance.sh`, built;
  tests `test_acceptance.py`). Not yet measured on a real edge. In a bilateral session both
  runners run the round trip at once, so our probe can reach the partner's edge just behind their responder's
  acknowledgement of their own probe and be dropped (measured in the hermetic two-sided
  run). So the round trip sends a fresh probe until one is acknowledged, the first resend one
  gap after the first probe and each later one twice as long after the last, up to 4 s, never
  closer than the gap, and any acknowledged probe passes; its reads stop at its timeout
  (`fedcheck.sh`, built; tests `test_roundtrip.py`). Not yet measured on a real edge.
- **Where the probes run.** From the operator's machine. Reads and writes go through
  `FED_SSH_OURS` / `FED_SSH_THEIRS` when set (curl runs on the core host), else directly. The
  round-trip script is streamed over that ssh with `bash -s`; nothing is copied to a host.
  With `FED_SSH_KNOWN_HOSTS` (a known_hosts file holding the cores' host keys) ssh verifies
  each host's identity and refuses an unknown or changed key; unset, it runs without checking,
  as before, and the bundle records which (`inputs.ssh_host_keys_verified`). The responders are
  started by whoever runs the drill. The probe sessions are too, unless a launcher is named
  (§2.4).

Tier 1 (each phase's own end state) belongs to the phases.

### 2.1 Persistent reception

**Built.** A delivery row reads the sample back from the receiving core's store, but that store
is only half of why the sample arrived. A sample crosses the edge because some subscriber on the
receiving core matches its key. When the edge refuses the store's own subscription (a store wider
than the edge grants, `from/**`, is refused whole), a session that somebody left subscribed on the
core (a drill's test subscriber, a person's `z_sub`) supplies the interest instead, and the store
records what that session pulled across. Delivery passes, and fails again the day the session
closes.

So after the delivery rows, for each offered set and direction, the runner:

1. reads who subscribes to the exact key of a new sample, from the receiving core's admin space:
   `@/*/router` (empty when the admin space is closed, which is BLOCKED rather than read as "no
   store") and `@/*/router/subscriber/<key>`. Each row names a subscribed key expression and its
   sources by zenoh id, as zenoh 1.9 prints them (`{"clients": [...], "peers": [...],
   "routers": [...]}`). A store runs inside the router, so its subscription is sourced from the
   router's own id; any other client or peer is a session somebody opened. No storage status
   query is made, because a storage-manager status fan-out on a router with many stores is the
   path of a known deadlock (`roles/goat-zenoh` compose notes);
2. FAILs when no store of the router subscribes to the key, and is BLOCKED while another session
   does. A drill that keeps test subscribers on a core names a command that stops them and one
   that starts them again (`FED_TEST_SUBSCRIBERS_PAUSE_OURS` / `_RESUME_OURS`, and `_THEIRS`);
   the runner stops them, reads the subscribers again, and restarts them after the read. A stop
   command without a start command is refused before the run begins (exit 2). A
   subscriber that does not match the key is no reason to stop (the round-trip responder polls
   `**/fedcheck/*` over REST and subscribes to nothing);
3. publishes a new sample under `release/<set>/acceptance/<run id>/persist` on the sending core
   and reads it back from the receiving store. PASS only when the exact bytes are read.

The scope it saw (`store_subscriptions`, `diagnostic`, `remote_routers`) is kept in the check's
observations. The refusal rows that follow are gated on the delivery rows alone, so a persistence row
that is BLOCKED or FAILs does not turn them into SKIP. A subscriber that reaches the core through
another router (`remote_routers`) is recorded and not counted: a goatnet with several routers
holds stores on more than one, and telling a store from a person's session on a remote router is
not built. Measured against `eclipse/zenoh:1.9.0` with a memory store on `from/xab/**`: the
store alone answers `ready`; a client subscribed to `from/xab/all/**` answers `diagnostic`; a
client subscribed to `from/xab/**/fedcheck/*` does not count for a `.../persist` key.

### 2.2 Refusal on the receiving edge's own record

**Built.** Silence proves nothing, and neither does HTTP 200 to the sender, who gets it for both
an allowed and a refused write. So the refusal rows read the receiving edge's ACL trace
(`goat_zenoh_edge_acl_trace`), and a denial is evidence only beside an allow for the same
session. zenoh 1.9 names the session first on every decision line:

```
<zid> on Interface(eth0)+CertCommonName(<agreed name>)+Transport(Tls)+<zid> is authorized to Put (ingress) on from/<them>/acceptance-probe/<run id>
```

The runner takes the session id from the control's allow line and asserts each denial among that
session's lines only. A denial logged for another session is FAIL (the control's own write
outside its lane was not refused on the record). An allow line with no session id makes the
denials BLOCKED: they cannot be tied to the control. With `FED_PROBE_THEIRS_ZID` set, an allow
from any other session fails the control, so the drill's probe is known to be the session that
was allowed.

The whole log is read. The control and the denials are found by keys carrying this run's id, so an
earlier run's line cannot match them. The edge names a peer that matches no subject once, when its
session connects, which can be before the run began; that line carries the probe's own session id,
so it is evidence for that session. A forged-name or impersonation probe whose session the log names
nowhere is BLOCKED. (Reading only lines after a count taken at the start was tried and dropped:
it hid the connect-time line, and the federation test environment showed both rows BLOCKED.)

**Built, not measured: the trace bound by id across two drivers.** When the machine that reaches
our core cannot reach our edge (`docs/design/federation-two-drivers.md`, phase 14), the rows
above come in two parts. `FED_ACCEPTANCE_TRACE=deferred` fires every probe, reads our core as
usual, reads no edge log, records each of the five trace rows as PENDING with the observation
`pending: edge-trace`, and writes `<work>/acceptance/ids.json`
(`goatbuilder.acceptance-ids/v1`): the run id, which every probe's key carries, the zenoh ids of
the probe sessions it fired, and the window it fired them in. A PENDING check is always required
and is never a pass, so this half is INCOMPLETE (exit 3) even when every other check passed, and
its counts line ends `pending=5`. On the edge, `ops/federation/standup/edge_trace_filter.py --ids <ids> --window
<start>/<end>` keeps only the log lines that carry one of those ids and are stamped within the
window widened by 5 minutes on each side (`goatbuilder.phase14-edge/v1`); a line with no
timestamp it can read is dropped. `FED_EDGE_TRACE_FILE=<that file>` then fires nothing and reads
no surface: it refuses a file whose ids or window differ from `ids.json`, or that holds a line
the filter would not have kept (exit 2), keeps every row the deferred run decided, and decides
the pending rows by the same rules as above, from that run's reads of our core and the returned
lines. A forged-name or impersonation probe whose connect-time line is older than the widened
window is BLOCKED in this mode, so its session has to start within the run, as a run-owned probe
does (§2.4). A derived session (§6.3) is refused in both modes, because its result is signed
before the edge's lines return. Tests: `ops/federation/standup/test_acceptance_trace_by_id.py`,
against the one-process fabric and a stand-in edge log.

These rows run on our edge for the inbound direction. The outbound direction is refused, or
not, by the partner's edge, so it is the partner's inbound row in their own bundle.

### 2.3 Existing partnerships, before and after

**Built, opt-in.** A deployment for one partner must leave every other partnership as it was.
`FED_EXISTING_WORKS` names the work directories of the peerings this side already has
(colon-separated; each holds that peering's `agreement.json`). Then:

1. `acceptance.sh <agreement> --as <us> --before`, run ahead of the deployment (between phases
   10 and 11), checks only the existing partnerships and writes its own bundle to
   `<evidence>/before/`. Without `FED_EXISTING_WORKS` it exits 2.
2. The run after the deployment (phase 14) checks them again, after its own rows:

| Check | Evidence |
|---|---|
| `existing/<id>/agreement` | the agreement file's SHA-256 equals the one recorded before. BLOCKED without a snapshot from before, or with one taken for another agreement than this run's |
| `existing/<id>/roundtrip/<set>` | for each set we offer that partner, a round trip through our core is acknowledged by their responder. A FAIL says whether it passed before the deployment. NOT APPLICABLE without a return path |
| `existing/<id>/leak/<set>` | for each set we offer the new partner and not this one, a sample staged under it does not reach this partner's core, beside a control staged on a set this partner is offered that does reach it (BLOCKED when the control does not land). Read only where a drill names that core's surface (`FED_EXISTING_REST`, `<id>=<url>,...`); otherwise an optional SKIP with its reason, because one side cannot read another sovereign's core and the multi-partner test environment is where leakage is proven |

The leak row is optional when `FED_EXISTING_REST` does not name that partner's core: it is then
an optional SKIP and does not make the run INCOMPLETE. It is required wherever the drill names the
core. An existing agreement whose ids or set names are not well-formed is not used (BLOCKED).

Without `FED_EXISTING_WORKS` nothing here runs and phase 14 is as before. Running `--before`
from the driver, and finding the other peerings from the deployment instead of a variable, is not
built; it is a separate change of default.

### 2.4 Run-owned, time-limited probes

**Built, opt-in.** The refusal rows need three client sessions across the link: one holding the
partner's agreed identity, one a name nobody agreed, one our core's name. A drill used to start
them by hand and leave them running. With `FED_PROBE_LAUNCHER` the run starts them itself:

| Launcher call | Contract |
|---|---|
| `start <agreed\|forged\|impersonate> <run id> <deadline, epoch seconds>` | prints `REST=<url>`, `ZID=<zenoh id>`, `HANDLE=<opaque id>`, one per line; the session ends by itself at the deadline |
| `stop <handle>` | ends it |
| `alive <handle>` | exits 0 while it exists, 1 once it is gone |

The deadline is now plus `FED_PROBE_TTL` seconds (900). Every resource started is written to
this run's own manifest, `<evidence>/probes/<run id>.json`, before it is used. At the end the run
stops exactly those, asks whether each is gone, and records `cleanup/probes`: FAIL, and required,
when any remains. A manifest an earlier run left behind with resources not confirmed gone (the run
was killed) is cleaned first, only the resources it names, recorded as `cleanup/earlier-runs`;
a manifest whose run is still going on this machine (its process id answers) is left alone, and a
manifest that cannot be read FAILs that row, since it cannot show its resources are gone.
A manifest names its run by process id and that process's start time: a live process id with
another start time is a reused id (the run is gone, its probes are cleaned); one whose start time
cannot be compared is left alone and recorded as `cleanup/earlier-runs-unknown`, BLOCKED. Only
exit 1 from `alive` counts as gone; any other answer leaves the probe listed. The probes are
also stopped when the run leaves any other way (an EXIT trap beside the INT and TERM ones).
An interrupt or termination stops this run's probes before the run exits. A probe given directly
(`FED_PROBE_THEIRS_REST` and the rest) is not started or stopped by the run. Probe keys stay
under the reserved `acceptance-probe/`, `acceptance-forged/` and `acceptance-impersonate/`
names, and no production edge, core or tunnel is started or stopped. The synthetic samples
themselves stay in the receiving store under its normal retention; nothing claims to delete them
downstream.

The launcher is the deployment's or the drill's: it is given whatever certificates the probes
need (approved negative test identities, never the CA).

**A deployment's launcher: built, not measured.** `ops/federation/standup/probe-launcher.sh`
meets this contract on a real edge host, reached over ssh with sudo (`FED_PL_SSH`) or run on the
edge host itself (`FED_PL_LOCAL=1`). It serves as a drill's launcher (`FED_PROBE_LAUNCHER`, on the
partner's edge host with the partner's identities) or as a side's own (`FED_PROBE_LAUNCHER_OURS`,
§6.4, on our edge host with our identities); which, is only where it runs and which certificates
`FED_PL_DIR` holds. It carries no identity and mints nothing. Each probe is a zenoh client in a
container whose image is the edge host's `zenoh_router_image` from its `pins.yml`
(`FED_PL_PINS`), pinned by digest (X-PROBE-002 otherwise); its zenoh config is
`ops/federation/standup/probe.json5`. Its files live under `/run/goat-probe` (tmpfs, so a key
does not outlive a reboot), the key mode 0600 in a folder of mode 0700. Its REST port binds to
one address (`FED_PL_REST_IP`; 0.0.0.0 is refused), because whoever reaches that port writes as
the probe's identity. `start` prints the handle before the session answers, so a run stopped
from then on still cleans it up, and removes a probe whose REST port does not answer within a
minute (exit 1, X-PROBE-003). `alive` exits 1 only when the edge host answers that the probe is
gone; an edge host that does not answer is exit 2. `stop` removes the probe's folder, the key
with it, and prints `KEY_DELETED=verified` only once the edge host shows the key gone
(`KEY_DELETED=failed` and exit 1 when it is still there). A `start` that does not end with a
session that answered, whether it fails, is interrupted or is terminated, removes what it put on
the edge host in an exit trap and prints the same line. The run records each probe's answer in
its manifest as `key_deleted` (`verified`, `failed`, or `not reported` for a launcher that says
nothing, such as the test environment's), and `cleanup/probes` fails, required, on a key that
was not deleted. A value that goes into a command on the edge
host is checked for its form first, and a handle it did not give is refused (X-PROBE-001).
Tested with stubbed ssh, sudo, docker and curl (`tests/probe-launcher.bats`, an interrupted
start among them) and, for the manifest's `key_deleted`, in `test_acceptance.py`; not yet run
against a real edge host. How the identities it is given are issued is not built.

**A side's own launcher, in a bilateral session** (built in the runner and on the in-process
fabric, tests `test_own_surfaces_*` in `test_acceptance.py`; §6.4). `FED_PROBE_LAUNCHER_OURS`
takes the same contract, and its three sessions hold this side's own identities: its agreed
leaf, a name nobody agreed and a leaf naming the partner's core, the last two from its own CA,
each dialling the partner's edge. The run writes through them the keys the partner's refusal rows
look for, and stages only their zenoh ids, beside this run's challenge and this run's start
(epoch seconds), for the partner. A kind is staged only when its session answered every write
with HTTP 2xx. Each session's zenoh id must be unique to its kind and its run: the partner refuses
one id staged for two kinds, and counts only its edge's lines for an id that are stamped at or
after this run's start less 30 seconds (a fixed bound for clock skew between the two hosts), so
a line an earlier run left for a reused id proves nothing; the ids' uniqueness to their run is
the main guard. A line for the id whose timestamp cannot be read, or an authorization with
none, counts as an authorization; a log that cannot be read leaves the row BLOCKED. It is
refused beside `FED_PROBE_LAUNCHER`, which is a drill's and holds the partner's identities, and
outside a session (exit 2). The federation test environment's launcher
(`ops/federation-e2e/lib/probe-launcher.sh`) serves both, by where it runs and which
certificates it is given, and so does a deployment's (`ops/federation/standup/probe-launcher.sh`,
§2.4).

**What the script cannot verify.** It does not see a launcher's certificates. It cannot tell
whether the agreed probe holds the agreed name, whether the forged name is one nobody agreed, or
whether a certificate came from this side's CA or the partner's. The rows judge what the
receiving edge records for each session; that the sessions hold the identities they are meant to
is the launcher's, and its operator's, responsibility.

### 2.5 Our own surfaces, from the declarations

**Built.** Status: not measured: the gate names two real environments, so it waits on goatbuilder 1.0 gate row B9 — gate: phase 14 run by `goatbuilder federate <partner>` on two real
environments with none of `FED_OURS_REST`, `FED_EDGE_LOG_OURS` and `FED_SPIRE_OURS` exported,
and the refusal and SPIRE rows reaching PASS.

Phase 14 reads three surfaces on this side: our core's REST surface, our edge router's log (the
ACL trace) and our SPIRE server. They are declared already, in the inventory and group_vars the
plays of phases 11 to 13 read, so `acceptance_surfaces.py` reads them from there, each only when
the environment does not set it. The variables stay as overrides.

| Value | Read from | Runs |
|---|---|---|
| `FED_OURS_REST` | `goat_zenoh_rest_host` and `goat_zenoh_rest_port` of `<them>_core` (then `peering_cores`, `all`, the goat-zenoh role default), on the group's one host in the inventories the plays use (the peering's own `peering-<edge group>.ini` when `allocate` rendered one, then `peering-edges.ini`) | from here; when the surface is bound to loopback (the role's default), on the core, and `FED_SSH_OURS` is derived from that host's `ansible_host`, `ansible_user` and `ansible_port` |
| `FED_EDGE_LOG_OURS` | `docker logs <goat_zenoh_edge_stack>` of `<them>_edge` (the role's name, `goat-zenoh-edge-<them>`, when unset); `sudo -n` when `ansible_become` is true | on the edge host, over ssh |
| `FED_SPIRE_OURS` | `docker exec <peering_spire_container> <peering_spire_bin>` (goat-peering-spire), only when the edge group sets `peering_spire_enabled: true` | on `peering_spire_server_host`, over ssh |

The groups are the run's own: `goatbuilder federate` exports `FED_EDGE_GROUP` (the run's
`--edge-group`, replacing any value the shell held) and `FED_CORE_GROUP` (the allocated core group,
else `<with>_core`) into phase 14's environment, and `ops/scripts/peering-standup.sh` does the same;
the phase's command line is unchanged. A value that cannot be derived (no
host, several hosts, a templated value, a SPIRE server that is not an inventory host) stays unset,
stderr names what was looked for and where, and the row that needs it is BLOCKED or skipped as
before. A value set by hand runs where it always has: the edge log through `FED_SSH_OURS` when
that is set, the SPIRE command here. Our partner's surfaces (`FED_THEIRS_REST`,
`FED_SPIRE_THEIRS`) are not ours to declare and are not derived.

## 3. Outcome and exit status

Every check ends with one of five outcome words:

| Outcome | Meaning |
|---|---|
| PASS | the expected evidence was observed |
| FAIL | a violation was observed |
| BLOCKED | a prerequisite or the evidence was unavailable (no counterparty surface, no ACL trace, no round-trip runner) |
| SKIP | not exercised (a refusal not attempted because a delivery failed, a negative certificate or SPIRE check not configured) |
| PENDING | exercised, and decided by a later run from evidence another machine returns (phase 14's edge trace across two drivers); always a required check, never a pass |
| NOT APPLICABLE | excluded by the agreed scope; never a required check, always with a reason |

The run's verdict and exit status come from `acceptance_evidence.py` alone:

| Exit | Verdict | Meaning |
|---|---|---|
| 0 | PASS | every required check passed |
| 1 | FAIL | a check failed |
| 3 | INCOMPLETE | none failed, but a required check was BLOCKED, SKIPPED or PENDING, or no required check ran (catalog code X-ACCEPT-001; both avenues' failure text names it) |
| 2 | | the agreement could not be read, or the evidence could not be written (including one check whose record failed: the bundle would lack it) |

A required check becomes PASS only by passing. No flag turns FAIL, BLOCKED, SKIP or PENDING into PASS.
Every check is required today.

The counts line keeps its first four fields for the readers that grep it:
`acceptance-counts: pass=N fail=N skip=N as=<us> blocked=N not_applicable=N pending=N`.

**Built:** a check that did not run exits 3. Before this, a run with skips exited 0 unless
`FED_ACCEPTANCE_STRICT=1` was set, so a production run with no counterparty surface passed the
gate on a single tier. `FED_ACCEPTANCE_STRICT` is still accepted and changes nothing.

A production operator holds only their own side, so a production phase 14 now exits 3 until
the partner's half is proven too. That is the intended reading: one side alone has not shown
that the peering works. The bilateral session (§6) is how the partner's half is added, for the
two operators to review.

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
  "software": {"acceptance.sh": "sha256:...", "acceptance_evidence.py": "sha256:...", "...": "..."},
  "configuration": {"applied": {"<file apply rendered>": "sha256:..."}},
  "session": {"id": "ses-...", "manifest_sha256": "sha256:...", "challenge": "<ours>", "expires_at": "..."},
  "counts": {"PASS": 14, "FAIL": 0, "BLOCKED": 0, "SKIP": 1, "NOT APPLICABLE": 0, "PENDING": 0},
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
`tier` is one of `positive`, `persistence`, `negative`, `round-trip`, `spire`, `existing`,
`precondition`; `persistence` and `existing` were added to that list without a new schema version, since a reader that ignores
unknown tiers reads the bundle the same.
`inputs` names which surfaces the run had, as booleans only: an address is not evidence and
does not belong in a file that may be handed to the partner. `observations` holds what the
check saw: the key, the bytes sent and read back, the ACL trace lines, the round-trip output.
Check names are stable (`delivery/<direction>/<set>`, `republisher-boundary/<direction>`,
`out-of-lane/<direction>`, `refusal/<case>`, `roundtrip/<direction>/<set>`, `spire/<case>`), so
two runs and two parties' bundles can be compared row by row.

`software` holds the digests of the runner's own files, and `configuration.applied` the digest of
every file under `<dir of the agreement>/applied/` (what `apply` rendered), so a result is bound
to the code and configuration it was observed with. `session` is null outside a bilateral
session (§6).

The bundle records local observations only (`observed_by`). A partner's bundle is imported
beside it, never into it (§6).

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
| the partner's edge drops our first round-trip probe | the next probe, one gap later, passes the round trip (exit 0) |
| a REST surface answering HTTP 500 | no negative row PASS; delivery BLOCKED |
| a surface answering every key with an old value | `fresh-keys` FAIL |
| other bytes on arrival | FAIL with both digests |
| their republisher lifting an unoffered set | `republisher-boundary/inbound` FAIL |
| an edge that logs no ACL decisions | the control FAIL, the denials SKIP |
| send-only, receive-only, no flow | NOT APPLICABLE rows with reasons and exit 0; exit 1 for no flow |
| an unwritable evidence directory | exit 2 |
| a diagnostic subscriber carrying a lane whose store's subscription is refused | delivery PASS, `persistence/` BLOCKED, exit 3; with the pause command, FAIL and exit 1, and the subscriber restarted |
| a subscriber on other keys; a closed admin space; no store for the lane | PASS; BLOCKED (exit 3); FAIL (exit 1) |
| a denial logged for another session; an allow from a session that is not the probe; decision lines without session ids | denial FAIL (exit 1); control FAIL (exit 1); denials BLOCKED (exit 3) |
| a bilateral session, each side on its own core: first runner, second runner, first again; import twice; combine | first: inbound BLOCKED, exit 3; second: inbound PASS with the partner's challenge, outbound BLOCKED naming the partner's row; first again: inbound PASS; second import refused as a replay; outbound PASS (peer-reported), scope INCOMPLETE where refusals were not configured |
| two operators, each with their own core, edge trace, probes and SPIRE commands, run one session and exchange bundles | both `combine` results PASS |
| a session in which each side holds only its own surfaces and its own launcher; no partner probes; probe ids from an earlier run; a forged probe our edge authorizes; an SVID from our own root; both launchers; a staged value holding key material | both `combine` results PASS; our refusal rows BLOCKED naming the partner; FAIL; FAIL; FAIL; refused; no such value is staged |
| own surfaces: a valid unmatched line, a line with an impossible timestamp, then an authorization; an edge log the filter cannot read; our edge's clock 20 s behind; two minutes behind | FAIL; BLOCKED; PASS; BLOCKED |
| own surfaces: a reused id whose only unmatched line is from an earlier run; a probe session that does not answer (forged; agreed); one id staged for two kinds; key material let past the certificate filter; our own launcher outside a session | BLOCKED naming the partner's run start; not staged, our row BLOCKED naming the partner's launcher (the control BLOCKED, not "did not land"); refused, ACL trace BLOCKED; the run stops, exit 1, nothing staged; exit 2 |
| a session manifest made for another agreement | exit 2 |
| a passing bilateral result with cleanup and two approvers; any of them missing, or a level that moves nothing; an open scope; a published record after expiry, a later failure, a changed agreement | a proposed file and diff, the deployment's file unchanged, one block after a second session; refused, nothing written; refused naming the open row; stale, degraded, stale, nothing written |
| probes from a launcher; a launcher whose stop does nothing; an interrupted run's probe beside a session that is not ours; no launcher | refusal rows and `cleanup/probes` PASS, nothing left; `cleanup/probes` FAIL (exit 1); `cleanup/earlier-runs` PASS and the other session untouched; no `cleanup/` row |
| an existing partnership: unnamed; unchanged before and after; no snapshot from before; its agreement changed; its round trip broken by the deployment; a set for the new partner reaching it; no surface for it | no `existing/` rows; PASS (exit 0); BLOCKED (exit 3); FAIL; FAIL naming "before"; leak FAIL; optional SKIP (exit 0) |

`FED_ACCEPTANCE_WAIT`, `FED_ACCEPTANCE_GAP` and `FED_ACCEPTANCE_RT_TIMEOUT` shorten the waits
for the test; a real edge needs the defaults.

## 6. A bilateral session, exchanged by file

**Built** (`ops/federation/standup/acceptance_session.py`). Each operator runs phase 14 on their
own core and edge only: no counterparty surface, no shared credentials, no service in between.

1. One operator writes the session manifest:
   `acceptance_session.py new --agreement <work>/agreement.json --us <us> --out session.json`.
   It binds the agreement's digest, the acceptance policy's digest (today the built-in
   `every-generated-check-required/v1`), both participants, a unique session id, the window it
   is valid in (`not_before`, `expires_at`; 24 hours unless `--hours`), one random challenge per
   participant and the sample limits. The file goes to the partner by any means; both read its
   `digest` aloud and compare, as with the agreement.
2. Each runs phase 14 with `FED_ACCEPTANCE_SESSION=session.json`. The runner first refuses a
   manifest for another agreement, one that does not name it, one not yet valid or expired, and one whose session id,
   participant ids or challenges are not of their fixed shapes (`ses-` plus letters, digits and
   hyphens; sovereign ids; 16 to 64 lowercase hex digits), because they become keys, bytes and file
   names (exit 2); the manifest must name exactly two participants, this side being one. Then, in
   place of the rows that need the partner's core, it stages on its own core, for each offered set,
   a sample under `release/<set>/acceptance/<session id>/<itself>` and a second one under
   `.../<itself>/persist`, both carrying the challenge the manifest issued to it, plus one under a
   set nobody offered and two writes outside every lane; it then reads the partner's from its own
   core for up to `FED_SESSION_WAIT` seconds (300), and runs its own edge's refusal rows (§2.2,
   with the probes and trace it holds) beside the partner's samples that landed:

| Row | Here |
|---|---|
| `publish/outbound/<set>` | our core's REST surface accepted both of our samples for the set (HTTP 2xx), each carrying our challenge; FAIL when it refused one, BLOCKED when it did not answer. A core declares no store on the staging name `release/**`, so a sample is never read back where it was staged: its arrival is the partner's `delivery/inbound/<set>` |
| `delivery/outbound/<set>`, `persistence/outbound/<set>`, `republisher-boundary/outbound`, `out-of-lane/outbound`, `roundtrip/inbound/<their set>` | BLOCKED: observed by the partner. Each names the partner's row that observes it (`observed_by_peer`) |
| `delivery/inbound/<set>` | the partner's sample arrived on our core with the partner's challenge, byte for byte; BLOCKED while nothing has arrived (they may not have run yet) |
| `persistence/inbound/<set>` | only our core's own store subscribes to the key of the partner's second sample, and that sample reads back (§2.1). The scope is read when we read, not when they published |
| `republisher-boundary/inbound`, `out-of-lane/inbound` | the partner's unoffered set and their writes outside every lane did not arrive, asserted only beside an inbound sample that did |
| `refusal/*`, `spire/*`, `roundtrip/outbound/<set>` | as outside a session, on our own edge, SPIRE server and core. The refusal probes hold the partner's agreed identity and our SPIRE check needs the partner's `spire-server` command, so both depend on what the partner provides for acceptance |

3. Each hands the other their `result.json`. `acceptance_session.py import --manifest
   session.json --us <us> --evidence <evidence dir> <their result.json>` keeps it under
   `<evidence>/peer/<them>/` with `peer_reported: true, authenticated: false`, after refusing
   (exit 2) a bundle of another schema, session, manifest, agreement or challenge, one observed by
   us, one whose run lies outside the session window, any bundle once the session expired, a run id
   not of the form `acc-<letters, digits, hyphens>`, a bundle already imported (by its digest or
   its run id), and a bundle whose passing inbound delivery does not carry our
   challenge (it cannot be from this session's samples).
4. `acceptance_session.py combine` writes `<evidence>/bilateral/<session id>.json` and `.md`:
   our rows and theirs side by side, local and peer-reported in separate columns. A row of ours
   marked `observed_by_peer` is satisfied only by the partner's mirror row passing, and a row of
   theirs marked so is decided by our own row. The partner's bundle must hold the mirror of every
   required row of ours (our outbound is their inbound; a row without a direction keeps its name;
   `publish/`, `existing/`, `fresh-keys` and earlier-run cleanup are each side's own), and each
   of those is required whatever their bundle says; each peer row is re-checked with the same rule
   that wrote ours, and a malformed one is a FAIL. Exit 0 when
   the required scope of both sides passed, 1 on any failure, 3 otherwise.

A signature would only say who reported a result, not whether their infrastructure is honest.
Until a command verifies one (§6.1), every combined result says that peer-reported rows are
not authenticated and that it is for the two operators to review; nothing gates on it, and
phase 14's own exit status stays its local verdict. An automatic bilateral gate is not built.

### 6.1 A partner's signed result

**Built** (`crates/goat-federation/src/acceptance_sig.rs`, with unit tests), and reached by
`goatbuilder federate test <partner> --bilateral` through phase 14 on a derived session
(§6.3). It is the first part of gate row B2, in which the command completes the partner's
half of acceptance; B2 is measured (§6.3).

**The key.** Each side signs its result with its federation signing key
(`secrets.federation.signing_key`), the Ed25519 key that signs its federation package at
phase 3. The partner verifies under the key bound to that side at phase 5 and confirmed on
the call, never under a key the result carries. No second key and no second binding is made.

**What is signed.** Not the result as JSON: a result can hold `null` and fractions, which the
package's canonical form refuses. The signature covers a fixed-format statement, each line
ending in a newline:

```text
goat-acceptance-result/v1
agreement sha256:<the agreement's sha256>
session <session id>
attempt <n>
by <the signer's sovereign id>
for <the verifier's sovereign id>
echoes <the verifier's challenge, as the signer's run observed it>
result sha256:<the sha256 of the result file's exact bytes>
key sha256:<the sha256 of the signer's 32-byte public key>
```

Each field is held to its shape first (the session id, sovereign ids and challenge as in §6,
the digests as 64 lowercase hex characters), so no field can add a line. The statement is
published beside the result as a detached document, `goat-acceptance-signature/v1`, which
carries the same fields and the 64-byte signature in standard base64.

**Why the key can sign both.** Every message this key signs begins with a byte that tells its
kind:

| What | Signed by | Begins with |
|---|---|---|
| a federation package (phase 3) | `goat federate init`, `goatbuilder federate init` | `{`: the package's signing input is the canonical form of a JSON object |
| the conformance attestation | `federate verify` | `conformance ok for ` |
| a peering directory entry | `directory.py` | `{`: the entry's canonical JSON |
| a peering directory request | `directory.py`, `directory_withdraw.py` | `saltlick-request/v1` |
| an acceptance result | `acceptance_sig.rs` | `goat-acceptance-result/v1` |

So no package, attestation or directory signature can verify as a result's, and no result's
as any of them. Both signers also refuse the other's input: the package signer
(`verbs::sign_with`) and the directory signer refuse a message that begins
`goat-acceptance-result/v1`, and a result is signed only through `acceptance_sig.rs`. Tests:
`what_phase_3_signs_for_a_package_can_never_begin_with_the_result_domain` reads every vector
package's signing input; `a_package_signature_is_not_a_result_signature` and, in
`compose.rs`, `a_result_signature_is_not_a_package_signature` check the two directions;
`tests/test_fed_directory.py` holds the directory signer to it.

**Verifying.** A partner's result is refused with GB-273, before its signature is checked,
when it is for another agreement, session or attempt, is by or for another party, or does not
echo our challenge. It is refused with GB-272 when its signature document is malformed, names
a key other than the one bound to the partner, signs other result bytes, or does not verify.

**Rotation and revocation.** A result's signature is bound to the key in force for the
partner when it was signed: the verified key's sha256 and the key itself are stored beside the
evidence, in `<evidence>/bilateral/`. Rotating or revoking a party's federation key therefore
also invalidates the acceptance evidence signed with it: that evidence still says which key
signed it, but a result is never verified again under a later key, and the peering needs a new
acceptance session to hold current evidence.

The acceptance evidence is the files under `<evidence>/bilateral/` and, while a session runs,
each side's result and signature under the session's key on its own core. Each side publishes
them on its own core, and the command combines both halves (§6.3, `--bilateral`).

### 6.2 A session both sides derive

**Built** (`acceptance_session.py derive`, `derive_session` and `compare_sessions`, tests in
`test_acceptance_session.py`, class `DerivedSession`); phase 14 runs on a derived session
(§6.3). It is the second part of gate row B2.

A manifest made by `new` (§6) has to pass from one operator to the other. A derived session
does not: each side works it out from the agreement it already holds and an attempt number,
and the two get the same bytes. It holds the agreement's sha256, the policy, both
participants, the sample limits, the attempt, and a session id that is
`ses-` and the first 32 hex characters of the sha256 of
`goat-acceptance-session/v1\n<agreement sha256>\n<attempt>\n`. It lasts until the end of the
agreement's `expires` day. Nothing in it is random or read from a clock.

It holds no challenge. Each side's challenge stays its own and random, and travels with that
side's samples.

**Both sides start phase 14 within `FED_SESSION_WAIT` of each other, normally on the same
call.** That is the one rule a derived session asks of the operators, and it follows from the
next paragraph.

**Freshness rests on the challenge alone.** A derived session has no `not_before` and no
window of hours, so nothing about time keeps an old result out. What does is the challenge:
each run of phase 14 draws a new one, and a partner's result must echo the challenge of our
current run (§6.1). A result that echoes an earlier run's challenge is refused with GB-273,
even when it is from the same session and attempt
(`a_result_echoing_an_earlier_runs_challenge_in_the_same_session_is_refused`). The session's `digest` is the sha256 of its other
members in canonical form (sorted keys, no whitespace). When the two sides' sessions differ,
`compare_sessions` refuses with GB-274 and names the members that differ: the two sides hold
different agreements, or are on different attempts.

### 6.3 Phase 14 on a derived session

**Built, tested on the in-process fabric and goatbuilder's stand-in world** (`acceptance.sh`
with `FED_ACCEPTANCE_ATTEMPT`, tests in `test_acceptance.py`, the `test_derived_` cases;
`goatbuilder federate test <partner> --bilateral [<attempt>]`, tests in
`crates/goatbuilder/tests/federate_acceptance_bilateral.rs`). It is opt-in: without
`--bilateral`, phase 14 runs as before and its exit is the local verdict. Measured on hosts:
the federation test environment's `bilateral-acceptance` scenario passed on `main` at
959ebead, run `959ebeadac57-20261119T230055-2154120`, with both sides running `--bilateral` at once on
only their own surfaces (gate row B2).

With `--bilateral`, goatbuilder gives phase 14 the attempt, its own hidden commands
`goatbuilder acceptance-result sign` and `verify` as `FED_ACCEPTANCE_SIGNER` and
`FED_ACCEPTANCE_VERIFIER`, and `FED_KEYRING`, the peering's keyring (`<work>/keys.txt`) that
phase 5 writes once the partner's key is confirmed on the call. `sign` signs with our
federation signing key (`FED_SIGNING_KEY`) and refuses one that is unset, unreadable or still
an unresolved secret reference with GB-142. `verify` takes the partner's key from that
keyring only, never from the result.

With `FED_ACCEPTANCE_ATTEMPT=<n>` and no manifest file, a run:

1. derives the session (§6.2) and draws its challenge: 128 random bits from the operating
   system's CSPRNG (`secrets`), new for every run and never derived;
2. stages its samples as in §6, then reads the partner's from its own core for up to
   `FED_SESSION_WAIT` seconds (300 by default), printing
   `waiting for <partner>'s phase 14, <n>s left` so the operator knows to start the other side.
   The partner's challenge is the one their sample carries, held to its shape; it is never
   taken from a file;
3. runs its rows and writes its result, as before;
4. signs the result with `FED_ACCEPTANCE_SIGNER` (§6.1, echoing the partner's challenge) and
   publishes it on its own core, the result under
   `release/<first offered set>/acceptance/<session id>/<us>/result` (base64, so its bytes
   survive) and the signature document beside it under `.../signature`. Both travel to the
   partner's core as any value in that offered set does;
5. reads the partner's from its own core, again for up to `FED_SESSION_WAIT`, and verifies
   it with `FED_ACCEPTANCE_VERIFIER` under the key bound to the partner at phase 5. A result
   that echoes an earlier run's challenge (GB-273) is left and the run waits for this run's;
   any other refusal (GB-272) ends the run with exit 2 and nothing is combined;
6. imports the verified result marked `authenticated: true`, with the partner's key and its
   sha256 kept beside it, and combines. The combined result in `<evidence>/bilateral/` says
   `authenticated: true` and carries the key as `peer_key`. The run's exit is the combined
   verdict: 0 PASS, 1 FAIL, 3 open.

A run with no signer or no verifier is refused before it starts (exit 2): no result is
published unsigned. When nothing of the partner's arrives, the run publishes nothing and ends
open (exit 3, X-ACCEPT-001); a retry draws a new challenge and needs the partner to run within
its wait.

**A partner sample from an earlier run** (built, tests
`test_derived_a_partner_sample_left_by_an_earlier_run_is_ignored_and_both_verdicts_agree`,
`test_derived_each_run_stamps_its_challenge_and_start_time_in_one_value` and the
`test_derived_a_partner_*` and `test_derived_clocks_*` cases beside them). Samples stay on the
cores between runs, and an earlier run's sample carries that run's challenge. So each run stages,
beside each sample and before it, a run stamp: `release/<set>/acceptance/<session id>/<us>/run`,
holding `<us>-<challenge>-<set>@<started>`, the start time in seconds since the epoch (UTC). The
challenge and the start time are one value in one write, so a reader never pairs one run's start
time with another run's challenge, whatever order the keys arrive in and whichever write fails.

A reader takes the partner's challenge from the stamp whose sample it found, and only from that
one (tests `test_derived_a_rerun_within_twice_the_wait_is_judged_by_the_stamp_whose_sample_matched`
and the `test_derived_two_runs_*` and `test_derived_an_unstamped_*` cases beside it). It reads each
of the partner's sets in turn:

- A stamp that started before the reader's own start minus twice `FED_SESSION_WAIT` (`2W`) is an
  earlier run's. Its challenge is never learned, the run prints that the partner's sample is from an
  earlier run, and it keeps waiting for a stamp started within `2W` of its own.
- A stamp started within `2W` is read, and the sample carrying its challenge waited for. A later
  stamp replaces it while the reader waits, so a rerun whose first run's sample never arrived is
  judged by the rerun's stamp and sample. When an earlier set already fixed the challenge and this
  set's stamp carries another, the set holds two of the partner's runs: its delivery and persistence
  rows end BLOCKED ("two of the partner's runs' stamps"), never a FAIL on other bytes.
- A stamp that cannot be read is not there yet; the reader waits. It is never taken for a partner
  whose runner stages no stamp.
- No stamp, read cleanly. A sample with no stamp beside it is taken only from a partner whose runner
  stages none, and the reader concludes that only after a whole `FED_SESSION_WAIT` on one set with no
  stamp of the partner's on any set; later sets are then read at once. Until then it waits, because
  this run's stamp may still be in flight: the first run after the partner upgrades, on a session id
  that its older runner used before, finds that runner's unstamped sample, and learning it would
  have the partner refuse our result (GB-273). Once the reader has read any stamp of the partner's,
  it never learns from an unstamped sample; that set ends BLOCKED. A garbled stamp counts as a stamp
  read. The cost: a partner on an older runner is read one `FED_SESSION_WAIT` later than before.
  An unstamped sample proves no run's identity: its delivery row records `run_identity:
  unstamped: run identity not proven`, and a later set whose unstamped sample carries another
  challenge than the one learned is BLOCKED as two runs, never a FAIL on other bytes. An earlier
  run's unstamped sample can still pass a set alone; its result is refused (GB-273, §6.1).
- The delivery row reads the reason a set is not this run's before it compares bytes, so the stamp
  decides a set whenever it says the sample is not this run's. When the wait ends without the
  sample, the row is BLOCKED and its reason names what is there: a stamp from the partner's earlier
  run, a stamp started within `2W` beside an earlier run's sample, that stamp with no sample, two
  runs' stamps, or an unstamped sample from a partner that stamps. The run ends open (exit 3), with
  no FAIL row. A sample of another shape beside a stamp started within `2W` is a FAIL (other bytes),
  as on any lane. A stamp's own sample read at the end of the wait is this run's, and passes.

**The clock tolerance.** Let `W` be `FED_SESSION_WAIT`, `x` how long before us the partner's run
started, and `D` how far the partner's clock runs behind ours (negative when ahead). A fresh sample
is kept when `x + D <= 2W`. Both sides start within `W` of each other (`x <= W`), so a fresh sample
is kept whenever the two clocks agree within `W`. An earlier run is told apart when it started
more than `2W - D` before ours, that is more than `2W` with agreeing clocks. An earlier run that
started closer than that is not told apart; its result is still refused when it echoes another
challenge (GB-273, §6.1), so it can leave the two verdicts unequal until a rerun, never pass a
stale result.

This rests on the two sides' clocks agreeing within `FED_SESSION_WAIT`. Phase 1's preflight
checks that each of our hosts' clocks is synchronized and within its budget (GB-268), and
`goatbuilder federate test <partner> --bilateral` runs that clock check again, alone, right
before phase 14 (`preflight.py --only clock`, into `<work>/preflight/clock-before-acceptance.json`),
stopping on a FAIL (built, tests `with_bilateral_phase_1_checks_the_clock_alone_before_phase_14`
and `a_stage_plan_table_lists_only_the_stage_phases`).

### 6.4 A combined PASS with each side's own surfaces only

**Built** in the runner (`acceptance.sh`, `FED_PROBE_LAUNCHER_OURS` and `FED_SPIRE_OURS` in a
session) and tested on the in-process fabric (`test_own_surfaces_*` in `test_acceptance.py`:
two sides holding only their own surfaces reach a combined PASS; without the partner's probes
our refusal rows are BLOCKED naming it; probe ids from an earlier run, a forged probe our edge
authorizes and an SVID from our own root fail; no staged value is key material). The
federation test environment's bilateral unit gives each side its own launcher and requires both
combined verdicts to be PASS (`federation-e2e-environment.md`, "The partner's half of
acceptance"). **Not measured:** that unit has not been run on this change, and no two real
environments have run it (goatbuilder 1.0 gate rows B2 and B9).

In a session, each side holds only its own core, edge and SPIRE server. Before this, two groups
of rows could not pass that way, so a combined verdict stayed open (exit 3) even when everything
else passed:

- the refusal rows (§2.2) need client sessions that hold the partner's identity: the partner's
  agreed name, a name nobody agreed and our core's name, each on a certificate from the
  partner's CA;
- `spire/svid-chains` needs the partner's `spire-server` to show that their workload chains to the
  bundle we hold for them.

Each side holds what the other's rows need. So each row moves to the side that owns its surface,
and the partner's half crosses in the signed result, as the delivery rows already do
(`observed_by_peer`, §6):

| Row | Who acts | Who observes, on what |
|---|---|---|
| `refusal/*` on our edge | the partner's runner starts its three probe sessions from its own side, with its own CA, against our edge, and stages each session's zenoh id and run id beside its samples | our runner, on our own edge's ACL trace (`FED_EDGE_LOG_OURS`), matching the decision lines to the session ids the partner staged (§2.2's rule, with those ids in place of `FED_PROBE_THEIRS_ZID`) |
| `refusal/*` on the partner's edge | our runner starts our probes, with our CA, against their edge | the partner's runner, on its own trace; our row is BLOCKED and names theirs (`observed_by_peer`) |
| `spire/refresh`, `spire/bundle-held` | none | each side on its own server (`FED_SPIRE_OURS`), as today |
| `spire/svid-chains` | each runner stages its own server's bundle and one workload SVID certificate (never its key) beside its challenge | the other runner verifies that certificate against the bundle its own server holds for the partner, and that it does not chain to its own trust domain |

Nothing new is trusted. The probe session ids and the SVID chain reach the partner as staged
values in an offered set, and each side's verdict on them arrives in its result, signed under the
key bound at phase 5 (§6.1). `combine` already requires a row marked `observed_by_peer` to be
decided by the partner's mirror row. The probes are the side's own: its launcher,
`FED_PROBE_LAUNCHER_OURS` (§2.4), starts sessions holding its own identities, so it is an own
surface. `FED_PROBE_LAUNCHER` starts sessions holding the partner's identity, which only a drill
owning both sides can do; the two are refused together. Every staged value is checked for private
key material before it is written, and the run stops (exit 1) on one; the receiver also empties
such a value and its row is BLOCKED. The partner's staged ids and certificate are taken only
beside this run's challenge of theirs, so values from an earlier run prove nothing. The forged
and impersonation rows count only the edge's lines for the staged id stamped at or after the
partner's staged run start less 30 seconds, never failing open on a line whose timestamp cannot
be read, and the three ids must differ (§2.4); a kind the partner did not
stage, because its launcher started none or its session did not answer, leaves our row BLOCKED,
naming the partner. The time rule rests on the two sides' clocks agreeing, as the session's
timing does (§6.3).

A session's own probes run with phase 14 in one run. Phase 14 split across two drivers
(`FED_ACCEPTANCE_TRACE=deferred`, then `FED_EDGE_TRACE_FILE`; federation-two-drivers.md,
"Phase 14 binds by id") is refused (exit 2) with `FED_PROBE_LAUNCHER_OURS` and in a session,
before anything is fired. **Not built**: own probes in the split run. The forged and
impersonation rows say "could not be read" only when the read of our edge's log failed (the
log command's exit status, or a refused edge file); a log that reads and names no line for the
probe is "names that session nowhere".

A session run with own surfaces only can now reach a combined PASS. The lab-only variant, which
gives each side the partner's `FED_PROBE_LAUNCHER` and `FED_SPIRE_THEIRS`, proves the same rows
a second way. It is not a gate.

## 7. Publishing `exchanging`

**Built.** A partner file's `level` is the grant (ADR 1157): `apply` renders it, and data moves
only at `exchanging`. Acceptance does not touch it. What acceptance publishes is a record that
the agreed scope was proven on both sides, as an `acceptance:` block in the same partner file,
and only through the deployment's own review of a change:

```
acceptance_session.py propose --manifest session.json --us <us> --evidence <evidence dir> \
  --partner-file <overlay>/ops/federation/partners/<them>.yaml \
  --approved-by <us>=<name> --approved-by <them>=<name> --out <dir>
```

writes `<dir>/<them>.yaml` (the file with the block) and `<dir>/<them>.yaml.diff`, and never edits
the deployment's file: it refuses an `--out` that is the partner file's own directory. It also
refreshes the combined result under `<evidence>/bilateral/`, as `combine` does. It refuses unless the combined bilateral result (§6) is PASS, both sides'
bundles show `cleanup/probes` PASS (§2.4), both operators are named, the session has not
expired, the file declares this session's partner, and its `level` already grants `exchanging`.
The block holds the session, the digests of the manifest, agreement, both bundles and the
combined result, the time, the expiry (the agreement's), and the two approvers, written as JSON
strings so a name lands exactly; a name with a control character, or longer than 120 characters, is
refused. A later session
replaces the block.

**What is reachable on this stack.** `propose` can succeed: a bilateral session (§6) reaches a
combined PASS when each side runs phase 14 in the session with its own edge's trace and refusal
probes (holding the partner's agreed identity for acceptance), both sides' `spire-server`
commands, and a probe launcher, so that its bundle carries `cleanup/probes` PASS (§2.4); the test
`test_two_operators_each_on_their_own_systems_reach_a_bilateral_pass` shows it end to end. A
bundle from a drill run (with `FED_THEIRS_REST`, outside a session) cannot be used: `import`
refuses it, because its payloads carry the run id, not the session's challenge. `propose` rewrites
`<evidence>/bilateral/<session id>.json` and `.md` each time, as `combine` does.

`ops/federation/validate-partners.py` checks the block: known fields only; status `exchanging`,
`stale` or `degraded`; every field present for `exchanging`; digests well formed; both approvers
named; and `exchanging` only beside `level: exchanging`, so the record cannot claim more than the
grant. An expired record is reported as stale in a note and does not fail validation.

`acceptance_session.py freshness --partner-file F --agreement A [--evidence DIR]` says whether a
published record is current, stale (the agreement's digest changed, or it expired) or degraded (a
local run after it FAILED), and prints the status change a person should propose. It writes
nothing: drift and expiry never rewrite a grant or a record by themselves.

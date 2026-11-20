# ADR 1245 — Phase 14 passes on the combined two-sided verdict

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-13 |
| Deciders | operator |
| Repo scope | goat core (`crates/goatbuilder/src/federate/`, `ops/federation/standup/acceptance.sh`, `ops/federation/standup/acceptance_session.py`, `docs/design/federation-acceptance.md`) |
| Amends | [ADR 1225](1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) decision 5, its "no automatic bilateral gate until report signing and replay rules exist" (inside a session the command runs, phase 14's outcome is the combined verdict) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the peering) · [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) (a channel is a cache of the binding, never its source) · [ADR 1229](1229-a-partners-new-package-is-confirmed-within-confirmation-1.md) (the other amendment to ADR 1225) · [`federation-acceptance.md`](../design/federation-acceptance.md) §6 |

## TL;DR (for a layperson)

After two environments are linked, each side tests that data really crosses the link. Each
side can only watch its own end, so a full test needs both sides' results put together. Today
the two operators swap result files by hand, and the command reports only its own side's half.
With this decision the command swaps the results itself, over the link it is testing, each one signed with the key the two
operators already confirmed by reading it aloud, and the test passes only when both halves
pass. While the partner has not run its half yet, the command says it is waiting, not that it
failed.

## Context

Phase 14 of `goatbuilder federate` runs acceptance on our own core, edge and SPIRE server. In a
bilateral session (`federation-acceptance.md` §6) the rows that only the partner can observe,
our outbound delivery and persistence among them, are BLOCKED with `observed_by_peer`. The
partner's result is imported by hand (`acceptance_session.py import`), the two are joined by
`acceptance_session.py combine`, and the combined result is for the two operators to read.
Phase 14's own outcome stays the local verdict.

ADR 1225 decision 5 took that from the first sovereign federation's proposal: "no automatic
bilateral gate until report signing and replay rules exist". Without a signature, an imported
result could have come from anyone who could write the file, so a gate on it would gate on an
unauthenticated claim.

Gate row B2 of [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) asks for each
side's acceptance report to include the other side's signed result, with no step outside the
command. `federation-acceptance.md` §6.1 to §6.3 build that: a session both sides derive from
the agreement, and results signed with the sovereign key each side bound in phase 5, verified
only against the keyring, and bound to the session, its kind and its signer, so a result cannot
be forged under a third key, moved to another session or replayed from an earlier one. Those are
the signing and replay rules decision 5 waited for.

## Decision

- **Inside a session the command runs** (`goatbuilder federate test <partner> --bilateral`,
  `federation-acceptance.md` §6.3), phase 14's outcome
  is the combined two-sided verdict of `acceptance_session.py combine`, computed over our
  result and the partner's result imported with `--keyring` (`authenticated: true`):
  - `passed` (exit 0) only when the required scope of both sides passed;
  - `failed`, GB-183 (exit 1) when any required row failed, ours or the partner's;
  - `waiting`, GB-199 (exit 3) while the partner's signed result has not arrived, or every open
    required row is one the partner observes;
  - `incomplete`, X-ACCEPT-001 (exit 3) when a required row of ours did not run.

  When more than one applies, the first in this order decides: `failed`, then `incomplete`,
  then `waiting`.
- **A partner's result counts only when it is signed.** It must verify under the partner's key
  in `<work>/keys.txt`, for this session, as kind `result`. A result that does not verify is
  refused at import (exit 2) and never counts towards `passed`.
- **Only the partner's latest result counts.** Of the partner's signed results for this
  session, only the newest one counts, by the time of its run, which its signature covers. An earlier one, offered again after a newer one was
  imported, is refused at import and never replaces it.
- **Outside such a session** (a plain run, or a session run by hand with `FED_ACCEPTANCE_SESSION`
  and unsigned files), phase 14's outcome is the local verdict, as today, and a combined result
  stays a record for the operators.
- **The results travel over the peering itself.** Both sides derive the session from the
  agreement, so there is no manifest to exchange (§6.2). Each side publishes its signed result
  and signature document on its own core, under its first offered set
  (`release/<set>/acceptance/<session id>/<us>/result` and `.../signature`), and they reach the
  partner's core as any value in that set does; each side reads the partner's from its own core
  and verifies it under the key read aloud in phase 5 (§6.3). Nothing is sent or imported by
  hand. A result exchanged as a file (`acceptance_session.py import`, §6) stays unauthenticated
  and peer-reported: it never decides phase 14's outcome. The directory carries no results.
- **What a signature does not say.** It says the holder of the partner's sovereign key reported
  these rows, not that the partner's systems are honest. Peer-reported rows stay in their own
  column in the combined record.
- **What it depends on.** The outcomes `waiting` and `incomplete`, and GB-199, are introduced by
  the change that makes phase 14 report a missing partner half as waiting
  (not yet on `main`). The session itself, its signing and the run over the fabric are built
  (`federation-acceptance.md` §6.1 to §6.3). The outcomes land before this ADR is accepted.
- **Publishing `exchanging` is unchanged.** It stays a reviewed change to the partner file
  (`federation-acceptance.md` §7). A combined `passed` makes `propose` possible; it never grants
  anything by itself.

## Alternatives considered

**Keep phase 14 local and the combined result advisory, even when signed.** No ADR change.
Rejected: gate row B2 cannot pass, a peering whose partner's half failed shows `passed` on our
side, and an operator must still read a second file to know whether the link works.

**Gate on the combined result without signatures.** Rejected: anyone who can write the partner's
result key on its core, or hand over a file, could make our phase 14 pass. That is what decision 5 refused.

**Carry the results through the peering directory, or by hand with an outbox and inbox.**
Considered when this record was first drafted: new saltlick routes, each result checked under the
key read aloud in phase 5. Not built: carrying the results over the peering itself needs no new
service, no second channel to secure, and no file an operator moves. Its cost is stated under
Consequences.

**Wait inside phase 14 until the partner answers.** Rejected as the only behaviour: the partner
may run its half hours later. Phase 14 waits up to `FED_SESSION_WAIT` seconds, then ends
`waiting`, and running it again picks up where it stopped.

## Consequences

**Positive.** Both operators see one verdict for the whole peering, from the one command, and a
failure on either side fails both. No result file is made, sent or imported by hand. Waiting for
the partner is not reported as a failure.

**Negative.** Our phase 14 cannot pass before the partner runs its half, so a peering's first
complete run takes both operators. A pipeline that read exit 0 from phase 14 as "our half
passed" now gets 3 until the partner's half arrives. The results need a lane each way: on a
peering that carries no data, the case where a FAIL report is most needed, or on a one-way
agreement, the partner's result never arrives, and phase 14 stays open (exit 3) rather than
passing. The operators then read each other's evidence; nothing passes on it.

## Assessor view (SCA-R)

A cross-boundary acceptance claim is accepted only when it is signed by a key bound through an
out-of-band confirmation, which maps to IA-3 and SC-12, and to SI-7 (software, firmware and
information integrity) for the result's integrity in transit. Binding each signature to the
session, its kind and its signer makes a result replay-resistant, the property IA-2(8) asks of
authentication. Each imported result is kept with its envelope and digest under
`<work>/evidence/`, which supports AU-10 (non-repudiation) and AU-2. Residual risk: a partner
whose own systems report falsely under its genuine key; the compensating control is that
peer-reported rows stay labelled as such, and that publishing `exchanging` still needs a
reviewed change approved by both operators.

## Trigger conditions for revisit

- A report-signing identity separate from the sovereign signing key is introduced: the key in
  the keyring that verifies results changes.
- A partner runs a goatcore release that cannot sign results: decide whether such a peering
  falls back to the local verdict or stays `waiting`.
- A peering that must reach a combined verdict without a lane each way (a one-way agreement, or
  one that carries no data): a second, signed channel for the results is needed, such as the
  directory routes considered above.

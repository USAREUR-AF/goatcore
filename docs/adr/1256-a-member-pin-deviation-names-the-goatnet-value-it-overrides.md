# ADR 1256 — A member pin deviation names the goatnet value it overrides

| Field | Value |
|---|---|
| Status | Accepted — built in `goat_consistency/resolve.py` and `crates/goatbuilder/src/pins.rs`, both avenues held equal by `ops/goatbuilder/parity-pins-member-over.sh` |
| Date | 2026-11-19 |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1110](1110-typed-consistency-foundation-and-console.md) D2, [ADR 1120](1120-release-graduation-ladder.md), [ADR 1127](1127-fastlane-urgent-fleet-wide-delivery.md), [`environment-parity.md`](../design/environment-parity.md) §2–§3 |

## TL;DR (for a layperson)

The pin table says which image each part of a goatnet runs. One host group can be
told to run something different from the rest of its goatnet. Before this decision,
that exception kept winning even after the goatnet as a whole moved on, for example
to a security fix. The group silently kept the old image, and nothing warned. Now
every such exception has to say which goatnet value it was written against. When the
goatnet value changes, the tools stop and ask the operator to decide again: take the
new value, or keep the exception deliberately.

## Context

A component's pin resolves as member deviation, then goatnet deviation, then the
fleet value (`environment-parity.md` §3; the one resolver of ADR 1110 D2, in Python
and in `goatbuilder pins`). Each deviation was validated on its own: a known scope, a
reason, a value or `absent`. Nothing related a member deviation to its goatnet's
resolution.

The release ladder resolves a rung at goatnet level, and `graduate.py hotfix` (ADR 1120)
tells the operator to add the hotfix as a goatnet deviation. So a hotfix applied that
way never reached a member host-group that had an older member deviation of the same
component. The ladder reported the goatnet on the hotfix ref, and generation, the
parity report and CI stayed green. This was measured on a goatnet. Fastlane (ADR 1127)
moves `fleet:` and clears value deviations, member ones included, so it was not
affected.

## Decision

1. A member deviation with a `value:` carries `over:`. That is the value its goatnet
   resolves the component to without the member deviation, exactly as written in the
   table: a ref, `absent` (the goatnet declares the component absent), or `none`
   (`fleet: null` and no goatnet deviation).
2. The resolver refuses, on every avenue that validates the table (`generate-pins.py`,
   `report.py`, `goatbuilder pins`):
   - **GB-357**: `over:` no longer equals the goatnet's current resolution. The goatnet
     moved after the member deviation was written.
   - **GB-358**: a member value deviation has no `over:`, or `over:` sits where it
     means nothing (a goatnet deviation, or an `absent: true` member).

   Both messages print the exact `over:` line to paste. The two ways out are to delete
   the member deviation (it takes the goatnet's value) or to set `over:` to the current
   value (it keeps its own pin, deliberately).
3. A member deviation that pins what its goatnet already resolves to is a warning, not
   a refusal.
4. `graduate.py hotfix` names every member of the goatnet whose own deviation would
   keep it off the hotfix, with both ways out. `graduate.py status` lists every member
   deviation off its rung's value.
5. There is no automatic stamping of `over:` onto existing tables. A table written
   before this decision refuses until each member deviation is decided by hand: that
   decision is the control.

## Alternatives considered

- **Warn instead of refusing.** A warning is what the measured case lacked: nothing
  stopped the run, and the shadow would have stayed. Rejected for GB-357 and GB-358.
  It is kept for the redundant case, which shadows nothing today.
- **A reason that names what it overrides, in prose.** A reason is read by people, not
  checked by tools, so a stale prose claim passes. Rejected in favour of a field the
  resolver compares.
- **Detect a newer goatnet change by date or git history.** This needs the history of
  the table, and a squash or a copy of the table into another repository loses it. The
  `over:` field carries the comparison inside the table itself.
- **Refuse only when the goatnet deviation is marked as a hotfix.** A fleet bump or a
  graduation can be the same fix and would be missed. The rule applies to any goatnet
  value change.
- **Stamp `over:` automatically on existing tables.** It would bless every shadow that
  exists today, including the one measured. Rejected.

## Consequences

**Positive.** A goatnet-level change can no longer miss a member silently. Every
exception is re-decided when the value it was written against moves. The hotfix
command and the ladder status name the members a hotfix would miss.

**Negative.** Every deployment's pin table that has a member value deviation fails
validation until `over:` is added to each one. That happens when the deployment next
moves its goatcore pin. Each goatnet-level change to a component with member
deviations needs one more edit per member: delete the member deviation, or re-stamp
it. Two implementations carry the rule, held equal by a parity gate.

## Assessor view (SCA-R)

This is configuration management (NIST SP 800-53 CM-2 baseline, CM-3 change control,
CM-6 settings) and flaw remediation (SI-2). A remediation applied at goatnet level is
either applied to every host group, or the exception is re-approved by name in the
committed table. Silence is not an outcome. The control is preventive, at generation
and in CI, not a detective report. Residual risk: an operator can re-stamp `over:`
without reading it. The stamp is a reviewed diff in a signed commit, and the
deviation's `reason:` is still required.

## Trigger conditions for revisit

- A third precedence level is added to pin resolution, for example per-host.
- Pin tables move out of YAML into a store with its own versioning.
- Fastlane or graduation start editing member deviations themselves, so the
  re-decision is no longer the operator's.

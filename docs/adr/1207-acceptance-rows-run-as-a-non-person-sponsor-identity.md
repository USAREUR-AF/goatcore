# ADR 1207 — Acceptance rows run as a non-person sponsor identity, and no row needs an operator key on a host

| Field | Value |
|---|---|
| Status | **Accepted** |
| Date | 2026-10-29 |
| Deciders | operator |
| Evidence | #2100 — `rc.sh` could not pass zagros's live rows through the door: the door host held no goat context, and one row ignored the door |
| Related | [ADR 1173](1173-release-candidate-tags-trigger-affected-flow-validation.md) (release candidates and their live rows) · [ADR 1117](1117-per-env-doors-cross-env-management.md) (doors) · [ADR 1071](1071-sponsorship-authorization-at-onboarding.md) (sponsorship and its tier cap) · F-561 (an operator key cannot be scoped) · [`release-candidate-validation.md`](../design/release-candidate-validation.md) · [`three-stage-candidate-onboarding.md`](../design/three-stage-candidate-onboarding.md) |

## TL;DR

The release tooling tests each candidate by onboarding a fake person on the goatnet and then cleaning them up. That test needs a login. It now uses a dedicated login that belongs to no person, can only admit people at the lowest level, and cannot act as an administrator. It is installed only on the management hosts the test runs from, and it can be cut off with one command.

## Decision

The operator asked:

> should it be my own keys or should we make a new key associated with this non person entity and scope only to this prupose and permission?

and added:

> let's keep an eye on the elevated security risk of placing certificates to take automated actions on the machines. doesn't mean it's wrong, but we need to be cognizant of it and how we handle it

Applied:

1. **Each goatnet the rows run on has one non-person sponsor identity, `goat-acceptance`.** It is a portal org, display name "Acceptance checks (non-person, ADR 1207)", with sponsorship capped at `browse` and no operator power. Its portal `whoami` reads `operator: false`, `sponsor_authorized: true`, `sponsor_max_tier: browse`.
2. **The two candidate-intake rows run as it**, through the goat context `acceptance` on the management host the rows run `goat` on. The rows name it as `goat_sponsor_context`, written by `ops/validation/generate-values.py`. Rows that read the fabric keep `goat_context`.
3. **No row needs an operator key on a host.** The sponsor-stage row vouches for its synthetic candidate and then withdraws the vouch as the same sponsor. A sponsor may withdraw a vouch until the admin decides: `/sponsor-reject` accepts `sponsor_approved` as well as `proposed`. The console advertises this at `GET /api/onboard/capabilities` (`sponsor_withdraw: true`). The smoke checks it before vouching, and against an older console it reports UNTESTED and creates nothing.
4. **Host rows reach a door-only goatnet through its door.** When `rc.sh` or `goatbuilder release` opens a door it passes `GOAT_ROWS_SSH_JUMP`, and every row's ssh jumps through it to the host's mesh address. The door host forwards TCP only; ssh still authenticates end to end with the operator's key.
5. **The identity is created with one command**, `ops/scripts/acceptance-identity.py`; rotation and cut-off are under Handling. The key is never printed or written to the operator's disk. It goes to 1Password (`goat-site-services/<env>-acceptance-sponsor`, field `api_key`) and over ssh stdin to the host.

## What a management host holds, and what it can do

A key on a host can be used by anyone who can read it there. The table states each one, by what it can do.

| Host | Context | Key | What the key can do | Status |
|---|---|---|---|---|
| zagros-dev management host | `acceptance` | `goat-acceptance` participant key | Mint invites and vouch, at `browse` only, for its own invitees; withdraw its own vouch; list its own invitees' proposals; act in the portal as its own org (its own slot and topics only) | **Live** 2026-10-29 |
| zagros management host | `acceptance` | `goat-acceptance` participant key | Same | **Live** 2026-10-29 |

Each file is mode 0600 under `~goat/.goat/contexts/`, readable by that user and root. The new identity's portal org also carries a fabric credential bundle link from creation. It was never collected, works once, and expires about 24 hours after creation.

## Handling

| Event | Command (`--context` = an operator context on that goatnet) |
|---|---|
| Create on a new goatnet | `ops/scripts/acceptance-identity.py <env> <admin-ssh> <admin-context> <target-ssh> [--port N]` |
| Rotate the key | `goat admin rotate goat-acceptance --key-only`, then install the new key the way the script does |
| Cut it off at once | `goat admin sponsorship goat-acceptance --revoke`: the console refuses the key (`403 not_sponsor`) on the next call |
| Remove it | `goat admin onboard goat-acceptance --revoke`, and delete `~/.goat/contexts/acceptance` on the host |
| Audit what it did | The console records every invite and decision with the sponsor `goat-acceptance`; every synthetic candidate has an `@example.invalid` address |

## Consequences

- `rc.sh` passes the zagros-dev and zagros intake rows without a person's key and without switching the laptop's NetBird profile. Measured 2026-10-29 from a laptop on the zagros-dev profile: on zagros-dev both rows PASS as `acceptance`; on zagros, through the door, the intake row and `onboard-runner-reaches-mgmt-api` PASS.
- The sponsor-stage row reports UNTESTED on a goatnet until its console carries this change (the next `goat-id-console` release). It creates nothing there in the meantime.
- zagros-prod has no identity. The release ladder runs no rows on prod.
- The fabric half of the same idea, a non-person fabric leaf for the rows that read the fabric, is #2139 (context `assay-probe`).

## Alternatives

- **The operator's own keys on the host.** Rejected: every automated act is recorded as that person, and taking the key off the host means revoking it for the person.
- **A dedicated operator key.** Rejected: an operator key is full admin on both planes and cannot be scoped.
- **Run the rows from a laptop on the goatnet's mesh.** Rejected as the standing path: it needs a person to switch NetBird profiles for every candidate, which the doors exist to avoid. It still works.
- **Let `goat` reach the console through a SOCKS tunnel over the door.** Rejected: the `goat` binary fails as soon as a SOCKS proxy is set, while plain HTTPS through the same tunnel works (measured 2026-10-29).

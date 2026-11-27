# ADR 1272 — Every offboarding writes a departure record and sweeps the external accounts a deployment declares

| Field | Value |
|---|---|
| Status | **Proposed** (2026-11-27). Built: the departure record, the reason class, the confirmation and the result table in `ops/scripts/offboard.sh`. Designed, not built: the external-account plug-ins (§3), the access review (§5), `goat admin offboard` (§6) |
| Date | 2026-11-27 |
| Deciders | operator |
| Repo scope | goat core (`ops/scripts/offboard*.py`, `ops/offboard/`, `ops/errors/catalog.yaml`); each deployment's own checkout holds its records and declarations |
| Supersedes | — |
| Amends | ADR 1187 (one offboarding driver over every plane): adds the record, the reason, the confirmation and the accounts outside the goatnets |
| Related | [ADR 1031](1031-credential-revocation-mesh-first-offboarding.md) (revocation is reversible; purge is separate), [ADR 1062](1062-three-stage-delegated-candidate-onboarding.md) (onboarding, which this undoes), [ADR 1088](1088-membership-transfer-engine.md) (membership transfer), [ADR 1182](1182-platform-admin-lifecycle-one-command.md), [ADR 1201](1201-two-avenues-scripts-documented-cli-supported.md), [ADR 1204](1204-declared-secrets-posture-and-zero-saas-quickstart.md) (a deployment declares its own services); [offboarding-one-driver.md](../design/offboarding-one-driver.md) |

## TL;DR (for a layperson)

When a person leaves a team that runs goat, two things are now written down and one thing is widened. The team records that the person left, why, and who confirmed it. The command that removes the person's access asks for that reason, shows who is about to lose access and how much, and waits for a yes. And the removal reaches beyond the goat environments to the other accounts the team says it uses, such as a password manager, a paging service or a cloud account; an account the command cannot handle is listed as work to do by hand and stays open in the record until someone closes it.

## Context

The operator, 2026-11-27:

> "we need to keep track of when someone offboards due to moving to a new position in the mil/gov or when they leave the service like [a named person] did and have a throguh offboarding process for all their privs across all accounts and envs."

The driver of ADR 1187 removes a person from twelve access planes on every goatnet a checkout declares and fails when a plane cannot run. Three things were outside it:

- **Nothing recorded a departure.** The driver's receipt is one file per run on the operator's computer. No shared place answered "has this person left, why, and are they fully off?".
- **The reason was not asked.** A person who moved to another government position and a person who left the service were removed the same way, and nothing said how either may return.
- **Accounts outside the goatnets were not swept.** A team holds access for a person in services goat does not run: the password manager account itself, a rescue network, a paging service, cloud accounts, device and site logins. Which services these are differs for every team.

## Decision

### 1. The departure record

Every `--apply` run writes a departure record before the first removal and adds its result at the finish.

| Property | Rule |
|---|---|
| Home | `ops/departures/<handle>-<effective date>.yml` in the deployment's checkout, committed by pull request like any other declaration. The driver writes the file; it does not commit |
| Content | handle; reason class and text; effective date; who confirmed; who ran it; how the person may return; the goatnets swept; one entry per run with one row per plane and account (where, what, found, result, verified, and why when it failed); the by-hand rows and who closed each |
| What it never holds | a login email, a credential, or anything a plane returned beyond the row's result. Failure text has the login replaced before it is written |
| State | `complete` when the latest run verified every row **and** every by-hand row is closed; otherwise `open` |
| Re-run | a later run for the same departure adds a run to the same record. A run that gives a different reason for an open record is refused |
| Retention | kept; the period is the deployment's decision (see Open decisions) |
| Tools | `python3 ops/scripts/offboard_departure.py list`, `show <handle>`, `close <handle> <row> --by <handle>` |

The driver's per-run receipt (ADR 1187 §5) and the per-plane `OperatorActionV1` events stay as they are. The receipt is the detailed evidence of one run; the record is the shared answer to who has left.

### 2. The reason class, and what it changes

`--reason` is required with `--apply` and is one of three classes. **Every class removes all access.** A person in a new position keeps nothing by default: access there is granted to the position, not carried over.

| Class | Meaning | How the person may return | By-hand row the record keeps open |
|---|---|---|---|
| `new-position` | moved to another military or government position | a new sponsorship through onboarding (ADR 1062), at the tier the new position warrants and under the new sponsor's ceiling; the old login's groups are never restored | `handover`: hand over what the person held for others — people they sponsored, an organisation's sharing authority, memberships that move with the position (`goat admin members move`, ADR 1088) |
| `left-service` | left the service | none; a later request is a new onboarding from the start | `purge-decision`: decide whether the retained, inactive login is purged (ADR 1031) |
| `other` | anything else; `--reason-text` is required | a new sponsorship through onboarding | none |

`--confirmed-by <handle>` is required with `--apply` and names the person who confirmed the departure. A checkout that declares `goat_offboard_confirmers` in `ops/ansible/group_vars/all.yml` limits it to those handles.

### 3. All accounts and environments

**Environments.** `--all-goatnets` runs the twelve planes on every goatnet `ops/goatnets/cross-env.yaml` declares (built, ADR 1187).

**External accounts** are declared per deployment in `ops/offboard/accounts.yaml` and handled by plug-ins. Designed, not built:

| Rule | Detail |
|---|---|
| Declaration | one entry per account: an id, a plug-in type, the plug-in's targets, how the person is matched there, and the **names** of the variables its credential arrives in. Never a value |
| Plug-in contract | an executable `ops/offboard/plugins/<type>` with two verbs, `check` (does this person hold access?) and `remove`, JSON in and JSON out. A deployment adds its own beside the ones goat ships |
| Matching | by the login email, the manifest-bound GitHub login, or a handle the person's manifest declares for that account (`identity.accounts.<id>`). Never by a name pattern. A person may hold more than one identity in one account; each is declared |
| A plug-in that cannot run | fails the run, exactly as a plane does: the row is `NOT RUN`, the exit code is non-zero, the record stays open |
| An account with no plug-in | declared with type `manual`. It is printed in the run's table as "not automated: do by hand", written to the record as an open by-hand row, and never skipped. The record cannot reach `complete` until someone closes it by name |
| Order | external accounts run after the goatnet planes; within them, credentials that work without a login go first |

Plug-ins goat ships are listed in the design document with each one's status.

### 4. Confirmation and one table

After the preflight has read every plane and before the first removal, the driver prints the person's handle and login, the reason, the goatnets, and in how many of the places access was found, and takes the handle typed back. `--yes` answers for a run with no terminal; with neither, nothing is removed.

Every run ends with one table: one row per plane per goatnet, one per team-wide plane, one per external account, with what was found, the result (`removed`, `none held`, `kept for teammates`, `FAILED`, `NOT RUN`, `do by hand`), whether it was read back, and why when it failed.

### 5. The access review

A read-only report per goatnet: accounts with no recent sign-in, accounts whose sponsor has a departure record, holders of admin groups, and accounts the identity source marks inactive that have no departure record. It reports; it never removes. Designed, not built.

### 6. Both avenues

`ops/scripts/offboard.sh` is the scripts avenue. `goat admin offboard` takes the same arguments and runs the same driver from the checkout, so the two cannot differ (ADR 1201). Designed, not built.

### 7. Offboarding undoes what onboarding does

Onboarding is gaining four facts per person. Each has its undo here, so the two stay inverse:

| Onboarding records or does | Offboarding undoes it by |
|---|---|
| owning team, separate from role | the manifest's status leaves `active` (the fabric plane); the owning-team value stays in the record's history and stops counting in any roster |
| a phone number for paging | removed with the paging account (the paging plug-in) and cleared from the manifest; the departure record never holds it |
| enrolment in the paging service | the paging account's plug-in, or its `manual` row |
| enrolment in the secrets store | the store planes of ADR 1187 (password-manager groups, self-hosted store sessions) plus the store-account plug-in |

A new thing onboarding grants needs its row in this table and its plane or plug-in before it ships.

## Alternatives considered

- **Record the reason only in the run's receipt.** The receipt is per run and private to one computer; "who has left" needs one shared list.
- **Let the reason class decide how much is removed.** Partial removal for a move leaves access that belonged to the old position. Access in the new position is a new decision by a new sponsor.
- **Hard-code the external services in the driver.** Every team uses different ones; a built-in list would name one team's choices in a public tool and miss everyone else's.
- **Skip an account that has no plug-in.** A silent skip is the failure ADR 1187 exists to stop; the `manual` row keeps it visible and keeps the record open.

## Consequences

- An offboarding without a reason and a confirmer does not run.
- A deployment's checkout gains `ops/departures/` and, once §3 is built, `ops/offboard/accounts.yaml`.
- A record stays `open` until its by-hand rows are closed, so unfinished departures can be listed.
- A run with no terminal needs `--yes`.

## Open decisions (operator)

| Decision | Proposed here |
|---|---|
| How long a departure record is kept | kept without an end; the record holds no login and no credential |
| Who may confirm a departure | any handle, unless the checkout declares `goat_offboard_confirmers` |
| The rule for a move to a new position | all access removed; return by a new sponsorship at the new position's tier; the `handover` row covers what the person held for others |

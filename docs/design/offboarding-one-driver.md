# Offboarding — one driver over every plane

**Status:** Driver built (`ops/scripts/offboard.sh`, plan-only by default). In
apply mode every plane is executed and read back, and an atomic JSON receipt is
saved on both success and failure. Decision: ADR 1187.
**Departure record, reason class, confirmation, result table:** built
([ADR 1272](../adr/1272-every-offboarding-writes-a-departure-record-and-sweeps-declared-external-accounts.md), Proposed).
**External accounts** (declaration, plug-in contract, `manual` rows, plug-ins `pagerduty` and `aws-iam`): built; the two plug-ins are unmeasured against the live services.
**Id console:** the Offboard button takes the reason and day and `/departures` is the register: built. The console running every plane and every goatnet: designed, not built.
**Access review** (`/access-review` in the id console): built.
**`goat admin offboard`:** designed, not built.
**Not this document:** granting or demoting a platform admin is [platform-admin-lifecycle.md](platform-admin-lifecycle.md) (ADR 1182). This is for a person who has LEFT. Credential inventory and rotation runbooks live in the private `goat-credentials` repo.

## The question this answers

*Is this person fully off?* Before this driver the answer was somebody's
recollection of about fifteen commands. Now it is one receipt, one exit code,
and every plane read back.

## The departure record

A departure is written down before access is removed: who left, why, who
confirmed it, and what each plane's result was. `--apply` refuses without
`--reason` and `--confirmed-by`.

```bash
ops/scripts/offboard.sh <handle> --all-goatnets                      # plan
ops/scripts/offboard.sh <handle> --all-goatnets --apply \
  --reason left-service --confirmed-by <handle>                      # remove
python3 ops/scripts/offboard_departure.py list                       # who has left; which records are open
python3 ops/scripts/offboard_departure.py close <handle> <row> --by <handle>
```

| Flag | Meaning |
|---|---|
| `--reason new-position` | moved to another military or government position. Return is a new sponsorship at the tier the new position warrants. The record keeps a `handover` row open: people the person sponsored, an organisation's sharing authority, memberships that move with the position |
| `--reason left-service` | left the service. No return path. The record keeps a `purge-decision` row open (ADR 1031) |
| `--reason other --reason-text "<why>"` | anything else |
| `--confirmed-by <handle>` | who confirmed the departure. A checkout that declares `goat_offboard_confirmers: "a,b"` in `ops/ansible/group_vars/all.yml` accepts only those handles |
| `--effective YYYY-MM-DD` | the day the departure took effect; today when omitted |
| `--yes` | remove without the typed confirmation, for a run with no terminal |

Every reason class removes all access. The class sets how the person may
return and which follow-on row stays open.

The record is `ops/departures/<handle>-<effective date>.yml` in the checkout
the driver runs from (`--departure-dir` moves it), schema `goat-departure/v1`.
It names the person by handle, holds one entry per run with one row per plane,
and never holds a login email or a credential: failure text has the login
replaced before it is written. It is `complete` when the latest run verified
every row and every by-hand row is closed. The driver writes the file; commit
it by pull request. A second run for the same departure adds a run to the same
record; a run giving a different reason for an open record is refused.

**Audit before removal.** The last preflight step writes one
`offboard.preflight` operator-action entry on each selected goatnet. When that
write fails the run stops with nothing removed: a run that cannot write its
audit entries would remove access it cannot attribute.

**Confirmation.** After the preflight has read every plane and before the first
removal, the driver prints the handle and login, the reason, the goatnets and
in how many places access was found, then takes the handle typed back. With no
terminal and no `--yes`, nothing is removed.

**One table per run.** The run ends with one row per plane per goatnet and one
per team-wide plane and one per external account: where, what, found, result
(`removed`, `none held`, `kept for teammates`, `removed, login unbound`,
`removed, audit not recorded`, `FAILED`, `NOT RUN`, `do by hand`), whether
it was read back, whether its audit entry reached the fabric, and why when
either did not. A removal that was read back but whose audit entry did not
reach the fabric shows `removed, audit not recorded`: the removal is verified,
the run still fails, and the departure record gains the by-hand row
`audit-entries`, closed once the entries are re-emitted from the local
fallback log with `goat audit emit`.

## External accounts

Accounts outside the goatnets (a password manager account, a rescue network, a
paging service, cloud accounts, device and site logins) are declared per
deployment and swept after the goatnet planes. Built: the declaration, the
plug-in contract, `manual` accounts, the driver's rows, and two plug-ins
(`pagerduty`, `aws-iam`).

**Declare.** `ops/offboard/accounts.yaml`, schema `goat-offboard-accounts/v1`
(start from `ops/offboard/accounts.yaml.example`). No file means no accounts.

| Key | Meaning |
|---|---|
| `id` | the row's name in the table and the record |
| `type` | the plug-in, `ops/offboard/plugins/<type>`; or `manual` |
| `match` | who the person is there: `email` (login email), `github` (the manifest-bound GitHub login), or `declared` |
| `credential_env` | the **names** of the variables the plug-in reads its credential from; never a value |
| `options` | passed to the plug-in unchanged |
| `what` | for `manual`: the work to do by hand |

**Match.** Never by a name pattern. A person's handle in an account goes in
their manifest as `identity.accounts.<id>` (one handle or a list, for a person
with more than one identity there; `"-"` states they hold none), or on the
command line as `--account-handle <id>=<handle>`. An account that matches by
`declared` with no handle from either place fails the run before any removal.

**Run.** The preflight calls every plug-in's `check` before anything is
removed; the counts in the confirmation include the accounts. After the goatnet
planes, each account with something held gets `remove`, then `check` again;
anything still held fails the run. Each account emits one `OperatorActionV1`
per selected goatnet, action `offboard.account.<id>`.

| Case | Result |
|---|---|
| plug-in file missing or not executable | `NOT RUN`; the run fails before any removal |
| a `credential_env` variable is unset | `NOT RUN`; the run fails before any removal |
| plug-in exits non-zero, prints no JSON, or breaks the answer contract | `NOT RUN` at preflight, `FAILED` after removal |
| type `manual` | table row `do by hand`, "not automated: do by hand"; an open by-hand row in the departure record; the run's exit code is unaffected and the record cannot reach `complete` until the row is closed |

The plug-in contract (verbs, JSON in and out, rules) is `ops/offboard/README.md`.

**Plug-ins shipped.** Each is tested against a synthetic peer
(`ops/offboard/test_plugins.py`); none has been run against the live service.

| Type | Matches by | `check` reports | `remove` does | Status |
|---|---|---|---|---|
| `pagerduty` | `email`, compared exactly | the account's users with that login | deletes each user; a user still on a schedule or escalation policy is refused by the service and the run fails with its reason | built, unmeasured |
| `aws-iam` | `declared` (the IAM user name) | a console password and each Active access key | deletes the console password and sets every access key Inactive; the IAM user and its policies are kept, and deleting the user is a separate decision. `options.aws_command` names the command, so a deployment can wrap it | built, unmeasured |

Not built: a password-manager member account, a rescue network. Declare
those `manual`. GitHub enterprise teams are part of the GitHub plane (below).

## In the id console

One click on a person starts the offboarding; `/departures` shows where every
one stands. ADR 1272 §6.

**Built.** The Offboard button on the onboarding page (`POST
/onboarding/{id}/offboard-cut`) takes three fields: `reason_class`
(`new-position`, `left-service`, `other`), `reason_text` (required for
`other`, one line, 300 characters at most) and `effective` (`YYYY-MM-DD`, today
when empty). A cut without a valid reason is refused with HTTP 400 before the
step-up and before anything is queued. The three values are written to the
proposal row (`offboard_reason_class`, `offboard_reason_text`,
`offboard_effective`) in the same guarded update as the requester, reach the
runner, and go into the `admin.offboard` operator-action record as
`reason=<class> effective=<day>`. A re-queue after a failed cut keeps the
recorded reason. `GET /departures` (admins) lists every person whose cut was
queued, is running, finished or failed: handle, reason, day, state, who asked,
and a link to `/offboard-verify`. It shows no login email.

**Designed, not built.** The runner executing the full driver (every plane of
this document and every declared account) and writing one row per plane back;
the console queuing the same departure on every declared goatnet; by-hand rows
closed in the console. Until then the console's cut reaches the NetBird
devices and setup keys, the fabric slot, the Ory login and the operator keys;
run `ops/scripts/offboard.sh` for the other planes.

## Access review

A read-only report per goatnet, in the id console: `GET /access-review`
(admins; `?idle_days=N`, default 90; `&format=json` for the same data as JSON).
Built. It reports and never removes. ADR 1272 §5.

| Finding | Derived from |
|---|---|
| `idle` | an active login with no live session issued in the last N days. The identity service lists live sessions only, so a person whose session expired reads as idle. Left out when the session list cannot be read |
| `sponsor-left` | the sponsor on the account's onboarding row has a departure in the console's register |
| `inactive-no-departure` | the identity service marks the login inactive and the register holds no departure for the person |
| `departed-still-active` | a finished cut is in the register and the login is active |
| `admin` | an active login holding the platform-admin group or any group named `<PLANE>-ADMIN` |

## Where a person's access lives

Twelve places, none of which knows about the others. The first seven are
per-goatnet; the last five are team-wide or per-store.

| # | Plane | What it is | Cut by |
|---|---|---|---|
| 1 | Host SSH keys | `authorized_keys` on every inventory host **and every INFRA-class mesh peer** | `platform-admin revoke --full`, matched on key body |
| 2 | Operator keys | the `op_` key that authorizes `goat admin` | portal operator-key API; the key file leaves the portal's directory |
| 3 | NetBird devices | the person's laptops, phones and boxes | **deleted** — regrouping is not enough |
| 4 | Setup keys | keys naming the person, which can enrol new devices | revoked |
| 5 | NetBird user | dashboard role and login | role `user`, blocked, auto-groups emptied |
| 6 | Fabric | Zenoh org slot, shared certificate bundle, ACL subject | `onboard --revoke`; the org remains when another active manifest on that goatnet shares its tag |
| 7 | Sponsorship | the right to invite and vouch | `goat admin sponsorship --revoke` |
| 8 | Ory | login, groups, sessions | login inactive and sessions ended; groups unchanged; driver verifies inactive and zero active sessions |
| 9 | Collaboration Kanidm | chat, files, Outline | account **deleted** by `onboard-runner -collab revoke` |
| 10 | Public-door Kanidm | the public front door directory | account expiry set |
| 11 | 1Password | the team's shared secrets | group removal |
| 11b | Team credential store (OpenBao) | sessions in the self-hosted store, where the operator's host is pointed at one (`BAO_ADDR`) | `ops/secrets/openbao/revoke-person` ends every token of the person's store identity (`--openbao-user`), then a dry run reads back zero; the next sign-in is already stopped by plane 9's Kanidm groups ([ADR 1203](../adr/1203-self-hosted-credential-store-openbao.md)) |
| 12 | GitHub | org membership, team membership, per-repo grants, pending org and repository invitations | manifest-bound login; org, collaborator, and invitation APIs |

**Order is part of the design.** Credentials that work without a login go
first: a shell, an API key, a device. The login goes later, so nothing the
person holds becomes useful while the run is in progress.

## Subject resolution

Run the driver from the repository containing the operator manifests. Python 3
and PyYAML are required, as for the existing operator validation tools.
Each selected goatnet needs `ops/operators/<goatnet>/<handle>.yml` with an
`identity.handle` matching the filename and a valid `identity.email`. When
present, the optional `identity.github` must be a valid bare GitHub login.

The driver resolves either input form against these local manifests before any
revocation begins. Every selected goatnet must resolve uniquely to the same
login email. Multiple handles for one email, missing manifests, malformed YAML
or conflicting identity fields refuse the entire run. Correct or synchronize
the registry through the existing manifest workflow, validate it, and run the
plan again. The driver does not fetch, create or overwrite manifests.

`platform-admin revoke` and `platform-admin status` receive the resolved email;
`onboard --revoke` receives the resolved handle. Plan mode shows both identifiers
and creates no receipt directory. This check establishes local mapping
consistency; it does not prove that the registry matches current live identity
state or that every offboarding plane is implemented.

The driver passes the manifest-resolved login to `platform-admin revoke` as
`--github`, so that login's current public keys define the GitHub part of the
host SSH key set. If the newest portal proposal names a different login, the
revoke reports both names but does not fetch or act on the proposal login's
keys. An old login can have been reassigned to another person, so key removal
must not follow stale proposal data.

The GitHub login is resolved before any plane can mutate. All selected
manifests that name `identity.github` must agree case-insensitively. A supplied
`--github-user` is only a cross-check and a mismatch refuses the entire run.
When none of the manifests names a login, the driver refuses by default; the
operator must supply both `--github-user <login>` and
`--github-user-unverified` to acknowledge that the login is not bound to the
subject's manifests. The driver verifies that the resolved login exists on
GitHub during preflight. Plan mode prints the login and whether its source is
`manifest` or `operator-unverified`.

The live manifest sync copies `identity.github` only when the candidate's
stored manifest supplied it; the sync does not infer or enrich that field.
Therefore a person without `identity.github` can reach `incomplete` at best,
never `success`, until someone adds the login to their manifest. Live operator
manifests are deliberately untracked; this checkout contained zero local live
manifests to inspect.

An operator-supplied login acknowledged with `--github-user-unverified` is
removed and read back, but its GitHub and host SSH rows are
`removed-login-unbound`, not verified. The receipt outcome is `incomplete` and
the command exits non-zero because the login is not independently bound to the
person. Add `identity.github` to every selected manifest and run the driver
again; there is no second binding mechanism.

If the person renamed their GitHub account, correct `identity.github` in their
local operator manifest on every selected goatnet to the current login, then
run the driver again. A deleted account cannot pass while a manifest names a
login that no longer exists. Remove `identity.github` and re-run with
`--github-user <login> --github-user-unverified` only when a live login is
known; otherwise, offboarding a deleted GitHub account is a known limit.

## The rules

**A plane it cannot run is a failure.** No leg may defer because a credential
is missing. `--apply` exits non-zero, names the plane and the credential, and
the receipt marks it `UNRUN`. This is the whole reason the driver exists:
`goat admin onboard --revoke` reported `peers_cut=0` and success while its
NetBird leg had no token and four devices stayed enrolled.

**`0` must mean two different things distinguishably.** "owned no devices" and
"did not look" are different facts. The receipt separates them.

**No notice to the subject.** A person who has left hears it from their
employer. Where a plane's API mails on its own — the console's group patch does
— the driver takes the path that does not.

**Read every plane back.** A plane whose `after` is not empty fails the run,
except that a fabric org is `retained-for-teammates` when the pre-revoke census
found another active manifest with the same `org.short_tag` (or legacy
`vendor.short_tag`) directly in that goatnet's operator directory. The census
skips unreadable and malformed files, ignores subdirectories and non-active
manifests, and excludes the subject by handle. The retained org is verified and
its shared certificate bundle is reported for separate assessment; without an
active teammate, a present org still fails the run.

The collaboration Kanidm account is deleted, matching `onboard-runner -collab
revoke`, whose provider issues `DELETE /v1/person`. This deliberately differs
from issue #1569's word "disabled": the existing collaboration revoke contract
is deletion. The public-door account is retained for reversibility and disabled by
setting `account_expire` to the Unix epoch. The driver identifies each account
by its mail attribute and reads it back after the write. It uses
`COLLAB_KANIDM_URL` plus `COLLAB_KANIDM_IDM_ADMIN_PW` for the collaboration
door (the onboarding service token cannot delete people), and
`PUBLIC_DOOR_KANIDM_URL` plus `GOAT_IDM_ADMIN_PW` for the public door; when
`GOAT_IDM_ADMIN_PW` is unset it reads the variable the checkout names as
`public_door_idm_admin_pw_env` in `ops/ansible/group_vars/all.yml`. The GitHub
organisations are `GOAT_OFFBOARD_GITHUB_ORGS`, else `goat_offboard_github_orgs`
in the same file.

The driver enumerates every 1Password group, including built-in groups, tries
to remove the person from every group where they appear, and enumerates every
group again. If the CLI refuses a built-in-group removal, the run fails closed;
this behavior has not been exercised against a live account. The CLI
documentation provides no safe named exception proving that a built-in group
cannot grant vault access, so the driver silently skips none. GitHub organisations
are set with `GOAT_OFFBOARD_GITHUB_ORGS`. GitHub removal first removes the person
from every team of each enterprise named in `GOAT_OFFBOARD_GITHUB_ENTERPRISES`, else
`goat_offboard_github_enterprises`, and reads each team back: while an enterprise
team grants organisation membership, an organisation owner cannot remove it (GitHub
answers 403). Reading enterprise teams needs a signed-in GitHub account that owns
the enterprise and a token with `admin:enterprise`; without them the preflight fails
and nothing is removed. With no enterprise declared, an enterprise-granted membership
fails the organisation removal and the error names the setting. It then removes organisation
membership, then removes the resulting outside-collaborator grant so direct
repository access cannot remain. It also enumerates pending organisation
invitations (matching login or the subject's email) and every owned repository's
pending invitations (matching login), deletes matches, and reads both lists
back. Any unreadable list or remaining invitation fails the plane. The GitHub
receipt row records the exact login and its source. Its per-plane audit event
carries the same login and source.

Each receipt has one row for every plane under every selected goatnet. A row is
`removed-and-checked`, `not-applicable-no-keys`,
`removed-login-unbound`, `not-run`, or `check-failed`, and carries separate
`before` and `after` values. `unknown` means the driver did not obtain evidence;
`absent` means it looked and found no access. Pass the operator's kebab-case
handle with `--operator` (or `GOAT_OPERATOR_HANDLE`). Each row records the
method, operator, service identity, and verification result. The driver emits
one `OperatorActionV1` for every plane on every selected goatnet under the
operator handle given by `--operator` or `GOAT_OPERATOR_HANDLE`. If an audit
emit fails, that plane is `check-failed` and the run fails even when removal was
verified: an unattributed removal does not meet ADR 1187 section 6. Its row
keeps the read-back `before` and `after` values and says in `detail` that the
audit emit failed.

`not-applicable-no-keys` is verified and counts as passing, but only with a
complete zero-key record: the pre-removal SSH status was readable; the console
had zero SSH rows in every status; no `--pubkey-file` key was supplied; the
manifest-bound login was passed as the authoritative `--github` value; its
GitHub key read succeeded and returned zero keys; and therefore the host
playbook received zero keys and did not run. The row records that login, both
zero counts, and the readback time. Any missing or contradictory fact is
`check-failed`; an operator-unverified login is always
`removed-login-unbound` instead.

The receipt is the detailed per-plane record (`before`, `after`, `how`, and
`verified`). It is saved privately on the computer that ran the driver under
`$GOAT_PROFILE_DIR/offboard/`, or `~/.goat/offboard/` when that variable is
unset or empty; `--receipt` overrides the path. The tamper-resistant evidence
an auditor can rely on is the per-plane `OperatorActionV1` events on each
goatnet, encrypted to the operator roster and retained in that goatnet's audit
store. The operator provides the local receipt to an auditor when asked.

**Shared credentials are reported, never rotated here.** A shared secret the
person could know (the portal's legacy `PORTAL_OPERATOR_API_KEY`, a goatnet CA
age recipiency, or an org fabric certificate bundle retained for active
teammates) has a blast radius beyond the person, so rotation is its own decision
with its own owner. The driver prints and records them so they cannot be
forgotten.

## Limits, stated

- **Enterprise-managed GitHub membership** needs an enterprise owner's
  account. Org-owner rights are refused by GitHub; the driver names the owner
  rule and the `admin:enterprise` scope. Built, not run against a live
  enterprise.
- **The driver needs every plane's credential on the host that runs it.** That
  is deliberate. A partial run is the failure this design exists to prevent.
- **A key installed by hand cannot always be attributed.** If it has no console
  row, was not supplied with `--pubkey-file`, and is not published by the
  person's current GitHub account, the driver makes no claim that it found or
  removed that key. Host matching remains by key body because key comments are
  not reliable identity evidence.
- **A new plane must be added here and in the driver.** A goatnet that grows an
  access surface the driver does not know about is not covered by it, and the
  driver cannot detect that on its own — the plane list is a human decision,
  reviewed like any other.

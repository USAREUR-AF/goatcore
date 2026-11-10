# Offboarding — one driver over every plane

**Status:** Driver built (`ops/scripts/offboard.sh`, plan-only by default). In
apply mode every plane is executed and read back, and an atomic JSON receipt is
saved on both success and failure. Decision: ADR 1187.
**Not this document:** granting or demoting a platform admin is [platform-admin-lifecycle.md](platform-admin-lifecycle.md) (ADR 1182). This is for a person who has LEFT. Credential inventory and rotation runbooks live in the private `goat-credentials` repo.

## The question this answers

*Is this person fully off?* Before this driver the answer was somebody's
recollection of about fifteen commands. Now it is one receipt, one exit code,
and every plane read back.

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
are set with `GOAT_OFFBOARD_GITHUB_ORGS`. GitHub removal first removes organisation
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

- **Enterprise-managed GitHub membership** cannot be removed with org rights.
  The driver reports it and prints the command that needs enterprise scope.
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

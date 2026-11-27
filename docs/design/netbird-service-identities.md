# NetBird service identities — one registry, one mint, three gates

| Field | Value |
|---|---|
| Status | Built. Registry, mint/check script, deploy gate, standup phase and CI check merged together |
| Registry | [`ops/netbird/service-identities.yaml`](../../ops/netbird/service-identities.yaml) |
| Script | [`ops/scripts/netbird-service-identities.py`](../../ops/scripts/netbird-service-identities.py) |
| Deploy gate | [`ops/ansible/tasks/require-netbird-service-identity.yml`](../../ops/ansible/tasks/require-netbird-service-identity.yml) |
| CI | [`ops/ci/check-netbird-service-identities.py`](../../ops/ci/check-netbird-service-identities.py), in `ci.yml`'s `ops-ci-guards` job, with its `--self-test` |
| Related | mesh-identity-operations.md (service users vs people), [live-state-guard-scheduling.md](live-state-guard-scheduling.md) §4 (minting is operator-run), [goatnet-standup.md](../operations/goatnet-standup.md) phases 11 and 12 |

## 1. What this is

Services on a goatnet read the NetBird management API with tokens that belong to
named service users, one per automation, so each can be revoked alone and shows
up by name in the activity log. The registry lists every such identity; a
goatnet's `group_vars` decide which ones it needs; one script creates the
missing ones; and the standup, every deploy and CI refuse to proceed when the
registry and reality disagree.

Not every automation needs one. The acceptance assay's probe identity is a
fabric leaf only (CN `goat-assay-probe-trial-peer`), with no NetBird or portal
credential; it lives in the cert manifest and the router ACL, not here
(goatnet-acceptance-suite.md §"The probe identity").

Before this, a token lived in three unconnected places: the role template, a
hand-typed variable name in each goatnet's `group_vars`, and a one-off mint
script if anyone had written one.

## 2. The registry

Each identity declares:

| Field | Meaning |
|---|---|
| `service_user` | NetBird service user name; `{goatnet}` expands to the group's `site_id` |
| `declared_names` | live names that predate the convention, per goatnet; recorded, not renamed |
| `token_env` | the operator variable, `{SITE}_...` (`GALAXY_PORTAL_NB_SVC_TOKEN`) |
| `required_when` | `always`, or `{var, role}`: the goatnet's `group_vars` value, else that role's default |
| `onepassword` | item and field in the team vault (`<goatnet>-portal-nb-svc` / `token`) |
| `consumers` | roles in this repository that read the token; each includes the require task |
| `external_consumers` | services that read the token but whose roles live in another repository (goatstuff); where the role is present, as in an assembled tree, the check refuses it unless it includes the require task, and refuses a role listed under both `consumers` and `external_consumers` |
| `bootstrap` | exactly one identity (`netbird-infra`) is minted by hand; it mints the rest |

Six identities are declared: `netbird-infra`, `portal-mint`,
`console-enrollwatch`, `provenance-guard`, `membership-reconcile`,
`warrant-membership`.

**Adding a consumer or an identity is one registry edit plus one include in the
role.** CI refuses the role until both exist.

## 3. The script

```
ops/scripts/netbird-service-identities.py <goatnet>            # offline: required? token present?
ops/scripts/netbird-service-identities.py <goatnet> --live     # + does each token authenticate as its user
ops/scripts/netbird-service-identities.py <goatnet> --mint     # create what is missing
ops/scripts/netbird-service-identities.py <goatnet> --sync-1password
```

- **Check.** Reads the goatnet's `group_vars` and the operator environment (the
  process first, then the primary checkout's `.envrc.local.d/*.envrc`, the files
  `ansible-preflight.sh` sources).
- **Verify (`--live`).** NetBird never returns a token's value or maps a value to
  its record, but it stamps `last_used` on the record when a request
  authenticates. The script makes one request with the token, then requires the
  expected service user's token record to show a `last_used` after that moment.
  It also reports days to expiry and fails under 30.
- **Mint (`--mint`).** Needs the bootstrap token. For each required identity with
  no token: reuse the service user if it exists, else create it (`role: admin`,
  `is_service_user: true`, no auto-groups); mint a 365-day token; store it in
  1Password through a JSON template on stdin, so the value never appears on a
  command line; append a reference to the item (`op://...`, never the value) to
  `.envrc.local.d/30-site-services.refs`. A token that cannot be stored is
  deleted again. Other postures: a value store takes the value itself; openbao,
  custom and env still append the export to `30-site-services.envrc`. It never
  prints a token, never overwrites a variable already set, and never overwrites a
  different value in 1Password.

Every identity is `role: admin`, because NetBird has no read-only role that can
list what these consumers read. A dedicated identity scopes revocation and
attribution. It does not reduce privilege.

## 4. The three gates

| Where | What refuses | Catches |
|---|---|---|
| Standup phase 12 (`goat-standup.sh`) | `--mint`, then `--live` as a GATE | a goatnet leaving standup without an identity its config needs |
| Every deploy of a consumer role | `require-netbird-service-identity.yml`: token empty, or read from a variable other than the registry's name for this goatnet | a service deploying without its capability; `group_vars` reading another goatnet's token |
| CI (`ops-ci-guards`) | malformed registry; a consumer role without the include, or an external consumer's role without it where the tree holds that role (an assembled tree; built, tested by `--self-test-assembled`, which `--self-test` also runs); a role listed under both `consumers` and `external_consumers`; a role rendering a `..._NB_..._TOKEN` variable that no identity lists; a `group_vars` name that disagrees with the registry | a new consumer added without a registered identity; name drift |

The standup ordering changed to make phase 12 possible: the browser step that
creates the bootstrap identity is phase 11, right after the dashboard, and mesh
CoreDNS moved into phase 12 as a scripted step after the account baseline.

## 5. Limits

- The registry covers tokens read by **deployed services**. Operator playbooks
  and scripts run with the operator's `<SITE>_NB_API_TOKEN` (the infra identity)
  and are not listed per script.
- `required_when` is evaluated offline, so the variable it names must be a
  literal boolean in `group_vars` or the role defaults. A Jinja expression stops
  the script with that message.
- The 1Password write needs `op` signed in to the team account. When it cannot
  write, `--mint` still writes the envrc, reports `NOT stored`, exits 1, and
  `--sync-1password` completes it later.
- The ownership check proves the expected user's token was used during the
  probe. A second request by another holder of the same token in that second
  would pass it too.
- LAN standups have no driver pause for the browser step; the bootstrap identity
  is created the same way from the LAN dashboard before phase 12.

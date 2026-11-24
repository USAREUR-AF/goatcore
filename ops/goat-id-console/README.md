# goat-id-console

Admin console for the Ory Kratos + Hydra identity plane. One front door
for platform operators to manage identities + groups + invites across
both Kratos instances on nova-a (tenant IdP on port 4434 + operator IdP on
port 4436).

Design: `docs/design/id-admin-console.md`.
ADR: [`docs/adr/0860-id-admin-console-separate-tenant.md`](../../docs/adr/0860-id-admin-console-separate-tenant.md).
Runbook: `docs/operations/id-admin-console-runbook.md`.
Permissions YAML: [`ops/permissions/group-permissions.yml`](../permissions/group-permissions.yml).

## Layout

```
cmd/console/      entry point
internal/
  config/         env loading + IdP map
  kratos/         Kratos admin HTTP client
  hydra/          Hydra admin HTTP client (consent sessions)
  audit/          Audit emitter interface + stderr impl + ring buffer
  store/          Invite token store (in-memory v1)
  permissions/    YAML loader for the permission matrix
  handlers/       HTTP handlers + templates wiring
web/
  templates/      html/template page templates
  static/         CSS + favicon
```

## Build

```
go build ./...
go test ./...
```

Vendored deps: `gopkg.in/yaml.v3` only.

## Run locally (against a real Kratos)

```
LISTEN_ADDR=:8080 \
IDP_TENANT_KRATOS_ADMIN_URL=http://127.0.0.1:4434 \
IDP_TENANT_HYDRA_ADMIN_URL=http://127.0.0.1:4445 \
IDP_OPERATOR_KRATOS_ADMIN_URL=http://127.0.0.1:4436 \
IDP_OPERATOR_HYDRA_ADMIN_URL=http://127.0.0.1:4447 \
PERMISSIONS_YAML_PATH=../permissions/group-permissions.yml \
DEV_BYPASS_AUTH=1 \
go run ./cmd/console
```

`DEV_BYPASS_AUTH=1` short-circuits the `X-Auth-Request-Groups` check for
local development against a Kratos without an oauth2-proxy in front. Never
set this in any deployed environment.

## Status

v1 scaffold: listing (with MFA column + client-side filter) + group editing
+ recovery-link mint + second-factor (2FA) reset + invite-link generation +
permissions render + in-memory audit. Postgres-backed invite + audit store
and Zenoh audit fan-out are TODO behind interfaces.

## Move a peer between mesh groups

1. As a `PLATFORM-ADMIN`, open **Users → Access → Peers** on the user's detail
   page and select **Move Group** for the device. Choose a current source
   group and a different destination group. The destination search matches
   group names case-insensitively.
2. Select **Preview Move**. The console uses the `goat admin members move`
   planner to show the proposed mesh membership change or explain a refusal.
   A preview leaves membership unchanged.
3. Review the plan and select **Confirm Move**. The console rechecks
   authorization, membership authority, and the plan. If the plan changed
   or the five-minute confirmation window expired, review a fresh preview
   before confirming.
4. For a warrant-governed scope, **Declaration ready for review** shows the
   target file and copyable YAML. In the goatnet configuration repository,
   follow the displayed instruction to replace the existing device entry,
   append a new entry to the existing file, or create the named file. Review
   the current default branch and open a pull request. After approval and
   merge, deploy the declaration through the goatnet's normal process.
   See the [warrant runbook](../../docs/operations/warrant-runbook.md#4a-membership--which-peer-holds-which-group)
   for validation and deployment.

The console can preview and confirm plans for scopes without enforced
declarations. In those scopes confirmation ends after review. The console
does not submit pull requests, change live membership, or report whether a
declaration has converged. The workflow acts on the selected NetBird peer's
mesh groups. Ory identity groups are managed separately.

### Reopen a governed move result

Reopen **Move Group** for the same peer after reloading the page. The
confirming administrator can recover saved YAML or a declaration preparation
failure and its next step for seven days. Repeating the same confirmation
returns the saved result under the same operation ID. The console records
the initial result through its audit path. Saved move reviews use PostgreSQL
and are omitted from legacy SQLite exports.

### Configure the planner

- Set `MOVE_PLANNER_GOAT_BIN` to the absolute path to a compatible `goat`
  executable and `MOVE_PLANNER_PROFILE_DIR` to its absolute console-owned
  profile directory. Supply both paths inside the container for container
  deployments. The standard console image ships without the executable.
- Set `NOTIFY_GOATNET`, `NOTIFY_MGMT_URL`, and `ENROLL_WATCH_NB_TOKEN` for
  the target NetBird instance. The console's token needs read access to
  peers, groups, and policies. End users authenticate to the console with
  their normal account.
- For governed confirmation, supply a CLI build that supports
  `--prepare-declaration`. Set `MEMBERSHIP_AUTHORITY=scope` and
  `MEMBERSHIP_SCOPE_PATH` to the absolute path of the reviewed
  `_scope.yaml` with `converge: enforce`. Mount the full scope directory
  read-only so the planner can find existing peer entries across fragments.

See the [browser test setup](web/e2e/README.md#move-group) for local
verification against synthetic NetBird data.

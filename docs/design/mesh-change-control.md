# Mesh change control — read-only dashboard, one write door, warrant owns membership

| Field | Value |
|---|---|
| Status | **Built.** |
| Decision | [ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md) |
| Extends | [ADR 1129](../adr/1129-warrant-scoped-expiring-network-authorizations.md) (warrant) from access rules to peer membership |
| Supersedes | the live-write path of [ADR 1088](../adr/1088-membership-transfer-engine.md) and the hold records of [ADR 1089](../adr/1089-probation-isolation-lifecycle.md) / [ADR 1091](../adr/1091-quarantine-isolation-lane.md), on every scope warrant governs |
| Operator doc | [warrant-runbook.md](../operations/warrant-runbook.md) §"Membership" |

## The approach

Everyone who can open the NetBird dashboard can look and cannot change anything.
Three named platform admins have a second dashboard address where they can also
write, and a write there is a break-glass repair. Every change to the network is
made through warrant: a reviewed diff in git, applied by a service identity, read
back continuously. One-off actions — minting an enrollment key, removing a
device, granting a role — keep their audited commands, and any of
them that changes membership writes a warrant entry instead of the live account.

Three rules:

1. **The dashboard is for looking.**
2. **Warrant is how the network changes.**
3. **Lifecycle actions keep their commands, and hand membership to warrant.**

## 1. Who can do what

| Who | Address | NetBird role | Can change |
|---|---|---|---|
| Everyone the dashboard admits: Ory `PLATFORM-ADMIN` or `IAM-ADMIN` | `dashboard.<site>` | `auditor` | nothing |
| Holders of Ory `NETBIRD-ADMIN` | `dashboard.<site>` day to day; `admin-dashboard.<site>` to write | `admin` | anything, from the admin address |
| Service identities in [`ops/netbird/service-identities.yaml`](../../ops/netbird/service-identities.yaml), including `svc-<site>-warrant` | management API | `admin` | their own kind of change |

`NETBIRD-ADMIN` is held and removed with `goat admin role`, which writes an
operator-action record. It is a service-permission group like every other Ory
group ([`ops/permissions/group-permissions.yml`](../../ops/permissions/group-permissions.yml)).

## 2. The two addresses

**Plain:** a person's NetBird role is the most they can ever do; the address they
use is what that session does.

- **Role.** `netbird-admin-reconcile` maps Ory groups to NetBird roles through
  `netbird_mgmt_role_map`, an ordered list of `{group, role}`. On a governed
  goatnet: `NETBIRD-ADMIN → admin`, `PLATFORM-ADMIN → auditor`,
  `IAM-ADMIN → auditor`. The highest role a person's groups map to wins. The
  reconciler only ever raises a role (`user < auditor < admin`); lowering is the
  one-time `netbird-role-realign.yml` play and, after that, the stray-admin
  report plus the same play. `owner` is never touched. The default map
  (`PLATFORM-ADMIN → admin`) keeps ungoverned goatnets exactly as they were.
- **Everyday address.** `dashboard.<site>` keeps its gate. When
  `netbird_dashboard_split_enabled` is true its API endpoint is
  `mgmt-read.<site>`, a Traefik route that forwards only `GET`, `HEAD` and
  `OPTIONS` on `/api` to the management server. Everything else on that host is
  refused.
- **Admin address.** `admin-dashboard.<site>` is a second dashboard container
  behind its own oauth2-proxy (`allowed_groups = NETBIRD-ADMIN`). Its API
  endpoint is the full API on `mgmt.<site>`, except on a goatnet where warrant
  also owns membership (`warrant_membership_enabled`): there it is
  `mgmt-admin.<site>`, a Traefik route that forwards everything under `/api/`
  except `POST`/`PUT`/`PATCH`/`DELETE` on `/api/groups` and `/api/groups/*`,
  which it answers 403.
- **The admin address and membership.** On a governed scope a group write from
  the admin address is refused at `mgmt-admin.<site>` rather than accepted and
  put back by the converger five minutes later. Membership is never hand-edited
  on a governed scope: an urgent removal is an entry (CI proves a narrowing, so
  it needs one owner's click); a device that must leave now is
  `goat admin peer cut`; and when the converger or its declaration is doing the
  wrong thing it is paused on the management host with
  `sudo -u root goat warrant pause --scope <site> --for 1h --reason "..."` while
  the fix is merged. A pause makes every converger run on that scope
  report-only; it lasts at most 24h, expires on its own, and is recorded as a
  `warrant.membership.pause` operator action. `goat warrant resume --scope <site>`
  ends it early. Two paths the route does not close, both reverted by the
  converger on its next pass: a token used directly against `mgmt.<site>`, and a
  user's `auto_groups` change (`PUT /api/users/*`) with NetBird's group
  propagation on. The admin address is for what warrant does not cover.
- **Why both.** The role alone would leave the three writers writing from the
  everyday address; the address alone would leave any admin token writing
  through `mgmt.<site>`. Each closes a path the other cannot.
- All three names (`admin-dashboard`, `mgmt-read`, and `mgmt-admin` where
  membership is governed) are mesh-only: an `isitup_services` row each (the mesh
  zone is generated from those rows), a SAN on the site-CA dashboard
  certificate, and Hydra redirect URIs for the admin address.

## 3. Every kind of change has one home

| Change | Home |
|---|---|
| Access policies | warrant rule intents (built) |
| Groups | `netbird_acl_taxonomy.yml` + `netbird-acl-bootstrap.yml` (built) |
| Peer membership | **warrant membership (this doc)** |
| Nameserver groups, routes | authored by the bootstrap playbooks; warrant reads them back |
| Enrollment keys | onboarding pipeline (`goat admin invite`, `--reissue-device-key`) |
| Removing or renaming a peer | `offboard.sh`, `goat admin peer cut`, machine enrollment |
| Roles, tokens, service users, owner | `goat admin platform-admin`, `goat admin role`, `netbird-service-identities.md` |

A pull request is the right home for how the network should stay set up, and the wrong one for a one-off
event: it cannot mint a one-time key and hand it to a person.

## 4. Warrant membership

### 4.1 The declaration

Each goatnet (taxonomy scope) has a directory, `ops/warrant/membership/<scope>/`,
split into files by who owns each kind of exception. The converger and CI read
the directory as one merged declaration.

```
ops/warrant/membership/zagros/
  _scope.yaml                 # the switch: converge report | enforce   (platform admins)
  _baseline.generated.yaml    # generated from operator manifests — never edited by hand
  substrate.yaml              # management and infrastructure hosts
  services.yaml               # external service hosts (ADR 1178) — written by external-service-finish.py
  holds.yaml                  # quarantine and probation — written by tooling
  people.yaml                 # exceptions for people's devices — written by members move / platform-admin
  orgs/<org>.yaml             # an organisation's own exceptions, reviewed by that org's admins
```

**`_scope.yaml` saying `converge: enforce` is the switch:** such a scope is
governed by warrant. With `converge: report` the directory is an adoption draft:
the converger reports against it, and the older audited tools
(`goat admin members move --apply`, `goat admin platform-admin`) still write
membership live, exactly as on a scope with no directory. The CLI decides from
the checkout, and outside a checkout from each scope's `converge:` compiled into
the binary; a `converge:` that cannot be read counts as governed.

Splitting by owner does three things a single file cannot: each file gets its own
reviewer through `CODEOWNERS` (warrant's existing way of routing a change to the
people who own it); tools write to their own file, so their pull requests do not
collide; and an organisation can own its exceptions without touching anyone
else's. Rules that span the whole declaration — one entry per device, one
`_scope.yaml` — are checked across the merged view. The single-file layout
(`<scope>.yaml`) is refused.

A fragment holds only entries:

```yaml
# ops/warrant/membership/zagros-dev/services.yaml
version: 1
entries:
  - peer: vendor-registry-01             # exact NetBird peer name
    exact: [ZAGROS-DEV-EXTERNAL-INFRA]      # the whole managed set; replaces the baseline
    owner: user369x
    reason: external service host (ADR 1178)
    expires: 2026-12-31
```

```yaml
# ops/warrant/membership/zagros-dev/people.yaml
version: 1
entries:
  - peer: peer-alice-zagros-dev-2026-09-01-laptop
    grant: [ZAGROS-DEV-P2P]                 # added to the baseline
    absent: [ZAGROS-DEV-OBSERVER]           # removed from it, and must stay removed
    owner: alice
    reason: development clique (p2p-graduation.md reason 1)
    expires: 2026-12-01
```

```yaml
# ops/warrant/membership/zagros-dev/holds.yaml
version: 1
entries:
  - peer: <peer>
    hold: quarantine                      # the managed set becomes <P>-QUARANTINED alone
    owner: user369x
    reason: signal flood, see F-565
    expires: 2026-09-30                   # review date
```

Fields: `peer` (required, unique across the directory), `owner` (required),
`reason` (required, ≤200 chars), `expires` (`YYYY-MM-DD`), and exactly one of:
`grant`/`absent` (either or both), `exact`, `hold: quarantine`. Group names are
full names carrying the scope's `site_prefix`.

**Not adopted: selector rules** ("every device of org X gets group Y"). They
shorten the file but widen what one line can grant, and the narrowing check can
no longer reason device by device. Revisit only if per-device entries still grow
into the thousands after the declared baseline below has removed the entries that
exist only to correct a bad baseline.

### 4.2 Desired membership

The converger computes, for each device, the set of **managed groups** — groups
whose name starts with the scope's `site_prefix` + `-`. `All` and groups of other
prefixes (`mesh-dns`, `EX-*`) are never touched and are reported as unmanaged.

| Entry | Desired managed set |
|---|---|
| `hold: quarantine` | `{<P>-QUARANTINED}` |
| `exact: [...]` | exactly that list |
| `grant`/`absent`, or no entry | baseline ∪ `grant` − `absent` |

**The baseline is what the device should hold by default.** It comes from the
first of these that answers, and each report row says which one did
(`baseline_source`):

1. **Declared** — the device belongs to a person (its name parses with the
   peer-name grammar, `ops/identity/peer-name-grammar.yaml`) whose operator
   manifest is in `ops/operators/<env>/`. The groups are that manifest's
   `netbird.auto_groups`, prefixed with the scope's `site_prefix`; for a phone or
   tablet (its enrollment key name ends `-handset`) they are `<P>-OBSERVER` only,
   which is what phone and tablet keys are minted with. `<P>-HANDSET` is never
   generated: no ACL rule names it. `ops/warrant/membership.py --generate-baseline`
   writes these into `_baseline.generated.yaml`, together with a top-level
   `identity_hashes` map containing the SHA-256 of each lower-cased login email.
   Keeping this map outside the strict per-person rows lets the prior CLI read
   the new file during rollout and rollback; the new CLI accepts its absence
   but refuses any person quarantine transition whose join is unavailable. The
   hashes join owners to live Ory without deploying or printing email addresses.
   The deployed converger uses this generated baseline only. The direct
   `goat admin members move` command also reads active local operator manifests
   and adds owner joins absent from the baseline; declared baseline rows retain
   their group intent. A conflicting identity hash makes that owner unprovable
   and therefore refused. Multiple matching handles are allowed only when every
   possible owner has one live, ordinary Ory identity and an ordinary declared
   baseline, and the peer's live and resolved baseline are also ordinary.
   Manifest entries prove ownership only and never replace the generated or
   setup-key group baseline. The privilege read and Ory write use the same
   resolved, profile-first Ory configuration.
   The operator manifests are
   gitignored, so CI checks the file's shape but cannot prove it is current;
   `membership.py --check --operators-dir <clone>/ops/operators`, run from a clone
   that holds the manifests, refuses a stale file.
2. **Key** — the `auto_groups` of the enrollment key that minted the device, found
   through its `Peer added` event (`meta.setup_key_name`). This covers machines and
   anything not named after a person.
3. **None** — no manifest and no surviving key. Such a device, with no `exact` or
   `hold` entry, is **unknown**: reported, never changed.

The manifest is preferred over the key because keys are a poor record of intent:
they are deleted, some are minted with no groups, and a person's tier can change
after their key was minted. With the declared baseline, the directory holds only
real exceptions — service hosts, P2P grants, holds — and expiry dates plus
`expire.py` keep those from accumulating.

### 4.3 Validation in CI

`ops/warrant/membership.py --check` (run by `warrant-registry.yml`):

- schema; one entry per peer; `site_prefix` matches the scope; every group exists
  in the taxonomy for the scope;
- `expires` required on `hold`, and on any `grant`/`exact` naming
  `EXTERNAL-INFRA`, `PLATFORM-ADMIN`, `BREAKGLASS` or `P2P`; a past `expires` fails
  (the removal PR is opened by `expire.py`, as for rule intents);
- **`EXTERNAL-INFRA` only as `exact: [<P>-EXTERNAL-INFRA]`** — an ADR 1178
  service host holds it alone; a peer can never hold it beside a person role;
- `PLATFORM-ADMIN` only on a person device (peer name `peer-…`), never a machine.
- a quarantine entry is rejected when any matched person's declared baseline
  contains `PLATFORM-ADMIN`. An ambiguous match whose candidates are all
  ordinary is accepted; CI checks only these declared facts and does not claim
  to have queried live Ory.

**Narrowing** (`ops/ci/check-warrant-narrowing.py --membership`): a diff is a
narrowing when every changed entry only removes `grant` groups, adds `absent`
groups, adds a `hold`, shrinks an `exact`, or deletes an entry that only granted.
CI proves a narrowing and labels it; the approving click is still a person's,
because GitHub will not let a workflow approve its own pull request (warrant
§6.2, runbook "What this does not do"). Anything else waits for review by the
`CODEOWNERS` owners of `ops/warrant/membership/` (`@platform`,
`@maintainers`).

### 4.4 The converger

`goat warrant apply --membership --scope <scope> --file <path>` —
[`crates/goat-cli/src/cmd/warrant.rs`](../../crates/goat-cli/src/cmd/warrant.rs),
built on the transfer-engine primitives in `members.rs`.

- It reads the goatnet's ACL declaration (`netbird_acl_taxonomy.yml`,
  `netbird_acl_matrix.yml`) as every `goat warrant` verb does
  ([`warrant.md`](warrant.md)). On the management host the `warrant-membership`
  role installs both files in `/etc/goat/warrant/acl/` from the checkout it
  deploys from, and the unit names that directory in
  `GOAT_ACL_DECLARATION_DIR`.
- `--mode report|enforce`; the file's `converge:` is the ceiling — the CLI can
  lower `enforce` to `report`, never raise it.
- Enforce adds before it removes, per group, and re-reads to verify (ADR 1088's
  mirror-first). It refuses to add a peer to a group no enabled policy covers.
- Before either entering or leaving `QUARANTINED`, report and enforce both join
  each person peer's generated email hash to exactly one live Ory identity and
  evaluate Ory `PLATFORM-ADMIN` / `IAM-ADMIN` / `ADMINS`, live mesh
  `PLATFORM-ADMIN`, and the declared baseline. An ambiguous person join is
  allowed only when every candidate is proven ordinary; a privileged candidate
  or any missing, conflicting, unavailable, or non-unique identity proof marks
  only that row blocked and leaves it untouched. Other rows converge normally. Enforce mode reports
  exit 4 after the run when any row was blocked by this gate; report mode stays
  successful while retaining the blocked row and
  `goat_warrant_membership_privileged_quarantine_blocked` metric. A proven
  uniquely owned privileged subject points to ADR 1091 severance plus credential
  rotation; ambiguous ownership with a privileged possible owner is reported as
  unproven instead. An unproven join tells the operator to repair the join before
  deciding whether severance applies. Machine/system peers have no Ory join and are
  evaluated from live and declared mesh state. `PROBATION-*` dormancy release
  is outside this security transition and remains allowed under its normal
  checks.
- **`--max-changes` (default 10)**: if a run would make more peer-group changes
  than that, it makes none and exits non-zero. A broken declaration or an empty
  event log cannot mass-revert a goatnet.
- Writes `goat_warrant_membership_*` gauges to `--textfile-dir` and one
  `OperatorActionV1` (`warrant.membership.apply`) per enforce run that changed
  something. The role renders the operator roster's age recipients
  (`GOAT_OPERATOR_AGE_RECIPIENTS`) into the unit's env and the router endpoint
  into its goat profile, the same two values the onboard-runner carries.
  NetBird's event log names `svc-<site>-warrant` as the
  initiator of every change.
- Runs on each governed goatnet's management host from the `warrant-membership`
  role: a systemd timer every 5 minutes with jitter, as `svc-<site>-warrant`,
  reading the declaration directory the role installs at
  `/etc/goat/warrant/membership/<scope>/` (files removed from the repo are removed
  there too). The unit passes `_scope.yaml`'s `converge:` as `--mode` (the CLI's
  own default is `report`).
- **Pause.** `goat warrant pause --scope <scope> --for <duration> --reason <text>`
  writes `/var/lib/goat-warrant/pause-<scope>.json` (`--state-dir`). While
  `until` is in the future the converger runs as `report` whatever `--mode`
  says, prints `paused until <until> by <actor>: <reason>`, adds `"paused"` to
  `--json`, and sets `goat_warrant_membership_paused{scope}` to 1. At most 24h;
  an empty reason is refused. An expired file is ignored and reported as
  expired, not deleted. A file it cannot read (or whose `until` is more than 24h
  away) is ignored with a warning and the run converges: a broken pause does not
  disable the converger. `goat warrant resume` removes the file; each verb
  records one `OperatorActionV1` (`warrant.membership.pause` / `.resume`).

### 4.5 Tools that used to write membership live

On a governed scope each of these writes or prints the entry instead:

| Tool | On a governed scope |
|---|---|
| `goat admin members move` | refuses `--apply`; prints the entry, `--write-entry` adds it to `people.yaml` (a hold to `holds.yaml`), or edits the device's entry in place in whichever file already holds it |
| `goat admin platform-admin grant\|revoke` | mesh leg prints the entry for `people.yaml`; other legs unchanged |
| `ops/scripts/external-service-finish.py` | writes the host's `exact` entry into `services.yaml` in the same PR |
| `ops/scripts/lifecycle-sweep.py` | dormancy holds become `exact: [<P>-PROBATION-<role>]` entries for `holds.yaml` in a PR (a narrowing) |

### 4.6 Quarantine and release

Quarantine is a `hold: quarantine` entry; release is reverting the commit that
added it. A peer has one entry, so a hold replaces any `grant`/`absent`/`exact`
entry the peer had; reverting the hold's commit puts that entry back, and the
desired set is recomputed from it — nothing else records prior groups. (If the
peer had no entry before the hold, reverting simply deletes it.) Containment
takes a pull request plus one converger pass. `goat admin peer cut` stays the
immediate action.

The authority boundary is the same on both sides of adoption. On an
ungoverned or report-only scope, `goat admin members move` performs the live
Ory/mesh privilege check using the generated baseline plus active local
operator manifests before dry-run output or apply. On an
enforcing scope it performs that check before printing or writing an entry;
the static validator rejects declared-baseline conflicts, and the deployed
converger repeats the live Ory/mesh check before report or enforce. Therefore a
hand-written entry cannot bypass the runtime decision, while CI remains honest
that it cannot query Ory.

On a governed scope whose generated baseline does not list a person, the converger cannot prove that person peer's owner and refuses its
quarantine transition. Add the scope to the person's manifest `netbird.sites`,
then regenerate with `python3 ops/warrant/membership.py --generate-baseline
--scope <scope>`; machines are unaffected.

## 5. Read-back

- **Membership:** `goat warrant apply --membership --mode report` is the read-back
  — one computation, not a second implementation. On a governed scope the
  `mesh-membership-reconcile` timer is retired (`mesh_membership_reconcile_enabled:
  false`).
- **Nameserver groups and routes:** `ops/warrant/reconcile.py --network` lists each
  live nameserver group and route as declared or undeclared against
  `ops/warrant/network/<scope>.yaml`.
- **Policies:** unchanged — `ops/warrant/reconcile.py` (generated / orphaned /
  undeclared).

## 6. Alignment with existing design

| Principle | How this follows it |
|---|---|
| Git is the system of record for network policy (warrant.md thesis) | Membership joins access rules in the same registry, CODEOWNERS and narrowing prover |
| One fact, one home; generation over lint (environment-parity.md, F-197) | The membership file is the only record; the converger generates live state from it |
| Guards that read live state run on the management host (live-state-guard-scheduling.md) | The converger and read-back run there, where the token already lives |
| Staged delivery: one driver, idempotent phases, fail-closed gates (staged-delivery-pattern.md) | `report` before `enforce`; `--max-changes`; unknown baseline is never changed |
| Promote automatically, never demote automatically (platform-admin-lifecycle.md) | Role map is promotion-only; demotion is an explicit, audited play |
| Least privilege; one door per class (ssh-key-self-service.md) | One write door, three holders, every other admin read-only |

### Practice borrowed

- **OpenGitOps v1.0** — desired state declarative, versioned, pulled, continuously
  reconciled. Membership meets all four except "pulled": the role installs the
  declaration. The next increment makes membership a subject of the signed
  `IntentV1` fetched from `get.<site>` (consistency-foundation.md §3), which closes
  it and gives the converger verify-at-read.
- **Tailscale GitOps for ACLs** — the admin console can be locked with "prevent edits
  in the admin console" and a link to the repository holding the policy. The
  read-only everyday address is the same idea for NetBird.
- **Argo CD `selfHeal`** — manual changes to a managed object are reverted to git.
  `converge: enforce` is self-heal for membership; `prune` has no analogue because
  the converger never deletes peers or groups.
- **AWS break-glass guidance** — a separate, monitored path used only when normal
  paths fail, alarmed on every use and reviewed afterwards. The admin address is
  that path; its access log and the undeclared-change report are the alarm.

## 7. Rollout

(omitted)

## 8. What this does not do

- It does not manage enrollment keys, peer removal or roles — those are events.
- It does not change access rules; they were already warrant's.
- It does not make the three writers' breaks invisible: a write at the admin
  address shows as drift until it is adopted into git or reverted.

# A goatnet's declared end state

| Field | Value |
|---|---|
| Status | **Built** (2026-10-29/30): the catalog, the derivation, the grades with all six edge kinds, the flag every deploy obeys, both avenues, the CI gates |
| Decision | [ADR 1206](../adr/1206-goatnet-declared-end-state.md), realising [ADR 1125](../adr/1125-fleet-reconciliation-join-and-graduation-sets.md) for service presence |
| Commands | `goatbuilder endstate <site>` · `ops/scripts/goatnet-endstate.sh <site>` · `python3 ops/goatbuilder/endstate/endstate.py model <site>` |
| Files | `ops/goatbuilder/endstate/catalog.yaml` (what each service is), `endstate.py` (derive, observe, grade, check), `edges.py` (the zenoh, NetBird and secrets edges), `baseline.txt` (today's declaration gaps), `ops/goatbuilder/fixtures/endstate/`, `ops/ci/check-role-honours-presence-flag.py` |
| Errors | GB-066 (cannot derive or read), GB-067 (a new or stale declaration gap), GB-068 (not in its declared end state) |

## What it answers

`goatbuilder endstate <site>` answers one question: is this goatnet running exactly what it says it should, and is each of those services up, healthy, doing its job and wired to the services around it? It reads only. Exit 0 means yes; 3 means something is off, and every off grade is printed with its reason.

```text
goatbuilder endstate zagros-dev                 # derive, read the host once, run the assay, grade
goatbuilder endstate zagros-dev --no-assay      # skip the acceptance rows (functional reads unknown)
goatbuilder endstate zagros-dev -o json         # one check per service and per orphan
python3 ops/goatbuilder/endstate/endstate.py model zagros-dev   # the derived end state alone, no host read
```

## Where the end state comes from

**Plain:** nothing new is typed out per goatnet. A catalog says what each service is; each goatnet's existing configuration says which services it runs and on how many machines; the end state is the two joined.

**Detail.** Five sources each answer part of "which services does this goatnet run", and none answers all of it. Measured 2026-10-29:

| Source | What it actually declares |
|---|---|
| `ops/pins/fleet-pins.yaml` | Versions. It renders all 31 components for every founder member (zagros, zagros-dev, zagros-prod, zagros-cloud) and declares absence only for joiners (`absent: true`) |
| group_vars `<name>_enabled` flags | Presence for about 20 services; the rest have no flag at all |
| `isitup_services` | The served names and their health paths (35 on zagros-dev, 45 on zagros) |
| repo ledger `ops/ansible/roles/*/deploy-history.yml` | Rollback host rows: 4 of zagros-dev's 31 pinned components carry an zagros-dev row |
| whichever plays were run | Everything else: minio, airdefenders, goat-get, goat-docs, admit-audit and lan-dns run on zagros-dev with no flag of their own in any role |

The end state therefore uses each source only for what it is authoritative for:

| Fact | Read from |
|---|---|
| What a service is: containers, class, ledger, served names, OIDC clients, SPIRE paths, acceptance tokens | `ops/goatbuilder/endstate/catalog.yaml` (not per-goatnet) |
| Whether the site runs it | the site's group_vars flag (else the role default, else the catalog's default) where the service has one; otherwise the pin table and the founder rule. A pin that merely renders a component is its version, not a presence vote; only `absent: true` speaks to presence |
| Which machines | the inventory group; the observability class on `<site>_obs_hosts` when `has_obs_host: true`, else on the management host |
| The served name and health path | the site's `isitup_services` row |
| The declared version | the site's rendered `pins.yml` |

Every presence declaration consulted must give the same answer: an `absent: true` against a flag that says on is a **contradiction**; none at all is **undeclared**. Neither is guessed.

**The flag is what the deploy obeys.** Each role (or play) that deploys a catalog service reads the service's flag: the role's `tasks/main.yml` imports its tasks under `when: <name>_enabled | default(true) | bool`, its rollback-ledger dependency carries the same condition (so a skipped deploy writes no ledger row, F-530), and the observability plays bring up only the services the site's `prometheus_enabled`, `alertmanager_enabled`, `grafana_enabled` and `obs_exporters_enabled` select. Unset means on, so no site's deploy changed when the guards landed. `ops/ci/check-role-honours-presence-flag.py` refuses a catalog service whose flag no deploying code reads.

## Declaring a goatnet's shape

**Plain:** to run without a service, set its flag to false in the goatnet's group_vars. To run on one machine, declare no observability host.

| The goatnet | Declare in `ops/ansible/group_vars/<site>_hosts/vars.yml` |
|---|---|
| has no Grafana | `grafana_enabled: false` (likewise `prometheus_enabled`, `alertmanager_enabled`, `obs_exporters_enabled`) |
| has no sandbox | `zenoh_sandbox_enabled: false` |
| has no notional pub/sub | `synth_vendors_enabled: false` |
| runs a service no flag names yet | `<catalog var>: true`, the `var:` its catalog row names (e.g. `minio_enabled`) |
| runs on one machine | `has_obs_host: false`; the observability services are graded on the management host |
| is a joiner | `goatnet_founder: false`; the founder-only control plane reads absent |
| should not run a pinned component | `absent: true` for the member in `ops/pins/fleet-pins.yaml` (then regenerate and re-sign the pins) |

`python3 ops/goatbuilder/endstate/endstate.py model <site>` prints the result: every service with present, absent, undeclared or contradicted, and the declaration behind each answer.

**What a single machine gives up.** The site has one failure domain: a host fault takes the control plane and the observability that would report it down together, and the obs stack's disk and CPU contend with the services it watches. The end state grades the same services; it does not hide the trade.

## The grades

**Plain:** every service the goatnet declares is checked five ways, and its connections to the identity and security services are checked one by one.

| Grade | PASS means | Read from |
|---|---|---|
| presence | the declarations agree it runs here | the model |
| deployed | the host ledger's newest row is `action: deploy` and names the declared digest; for a serving surface (goat-get, goat-docs) the row's `served` payloads are also the ones main declares, judged by `ops/scripts/served-payloads.py` | `/var/lib/goat/<ledger>/deploy-history.jsonl` |
| active | every catalog container is running; a one-shot (a migration) exited 0 | `docker inspect` |
| healthy | the docker healthcheck is `healthy`, and isitup's probe of the declared health path matches the row's `expect_codes` (default `2\|3`) | the container's own address, as isitup probes it |
| functional | every acceptance row whose `requires` names one of the service's tokens passes, less a site-wide row its catalog entry excludes (`assay_exclude`) | `goatbuilder assay` rows (ADR 1115) |

**Edges** are the zero-trust threads between services:

| Edge | Wired means |
|---|---|
| route `<name>` | the served name resolves on the site's host, the certificate it presents verifies for that name, and the front door does not answer 404/502/503/no answer. A row declaring `probe_front_door: false` (or `probe_gate: false`) is not a front door and is skipped |
| oidc-client `<id>` | the client exists in Hydra's admin API and one of its redirect URIs is on the service's own name |
| spiffe `<path>` | SPIRE has a registration entry for `spiffe://<trust domain><path>` |
| zenoh-cert | the fabric client's certificate (read from its container's environment or its mounted cert) chains to the router's root CA, is unexpired, and its CN is named by an ACL subject |
| zenoh-grant `put\|sub <key>` | the router's live `access_control` (its rendered `config.json5`, parsed on the host) allows the client's CN each key its catalog `zenoh:` row declares, decided as zenohd 1.9.0 decides: a deny inside a subject wins over that subject's allows, the first allow across subjects wins, otherwise `default_permission` |
| zenoh-denied `put <key>` | the paired negative: a put on the operator-audit trail (`ops/audit/operator/…`) and on a key no service declares is refused to the client; an allow is an over-grant |
| netbird-path `<GROUP>→INFRA <proto>/<port>` | the site's declared matrix (`ops/ansible/netbird_acl_matrix.yml`, the founder's for a joiner) admits the group on the port (tcp/443 for any front door; the catalog's `mesh.reach` for the router and DNS), and the live account carries one of the admitting rules enabled |
| netbird-closed `<GROUP>→INFRA <proto>/<port>` | the paired negative: the catalog's `mesh.internal` ports (postgres 5432, MinIO 9000) are admitted to no participant group (OBSERVER, DEVELOPER, MONITOR) |
| netbird-acl declared-only | on the NetBird management service: every enabled live policy is one the matrix declares and none is an All-group rule (ADR 1185); read with `<SITE>_NB_API_TOKEN` |
| secrets | every secret the service's deploy play templates (`lookup('env', …)` bindings it references) is available to a deploy from this checkout: the shell, the declared posture's references, and, until ADR 1186 retires them, the main checkout's legacy `.envrc.local.d/*.envrc`, which the grade names. Names only; no value is read |

**Orphans** are the negative side: a container, Hydra client or SPIRE entry (other than the agents) that no present service declares. A client or entry left behind is still a way to obtain a credential, so it fails the verdict.

**UNKNOWN and UNOBSERVED are different.** UNKNOWN is a grade that exists but could not be read this time (a host unreachable, the assay not run); it keeps the verdict red. UNOBSERVED is a grade nothing could read because the service declares no signal for it (no healthcheck and no isitup row; no acceptance row; no deploy ledger). It is counted and listed apart, as ADR 1125 separates coverage from sync and health, and `--strict` fails it.

## Both avenues and CI

`goatbuilder endstate` and `ops/scripts/goatnet-endstate.sh` run the same `endstate.py` (ADR 1201). CI (`goatbuilder-pr.yml`) runs:

- `endstate.py --self-test`: the edge primitives on their own (the JSON5 reader on comments, unquoted keys and `//` inside strings; key-expression inclusion with `*`, `**`, `$*` and verbatim `@` chunks; the zenoh decision order; NetBird port ranges and bidirectional rules), then three fixtures: a single-host site in its end state (exit 0), a site with its own obs host and no Grafana or sandbox (exit 0, both absent, no orphans), and a site with orphans, a missing OIDC client, a contradiction, a dead container, an over-granted fabric publisher, a participant path to postgres and a missing secret (exit 3, each named);
- `check-role-honours-presence-flag.py --self-test` and the check itself: every catalog flag is read by the code that deploys the service;
- `endstate.py check`: every governed site models, every catalog row is well formed, and `baseline.txt` only shrinks (GB-067);
- `parity-endstate.sh`: both avenues print the same lines and exit the same on every fixture;
- the binary's unit and golden tests.

The assay runner accepts the model: `run-acceptance.py --endstate <model.json>` grades a row N/A when every service it requires is declared absent on the site (the `endstate.py model` output is the file).

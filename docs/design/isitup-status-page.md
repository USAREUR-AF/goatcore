# isitup — the goatnet status page

**Status:** built (role `ops/ansible/roles/isitup/`, page `ops/status/index.html`). The #1518 changes (row list, lifecycle, `/external`, collapsible sections, registry-filtered probes) are written and tested in CI, and not yet deployed to any goatnet. The list-only play, the list stamp in `meta.json` and the stale-list check ("Keeping the list current") are built and tested in CI, and not yet deployed to any goatnet; the page line that shows the stamp, the merge trigger and the pulled list are designed, not built.

Each goatnet serves one status page at `https://isitup.<site>.<zone>/`, behind the goatnet's own sign-in. It has two jobs. It is the **home page**, where a newcomer finds what they can open. It is also the **health board**, where an operator sees what is down. This document describes how the page decides which rows exist, how it lays them out for those two readers, and how a row's state reaches alerting.

## Who reads it

| Reader | What they need from the page |
|---|---|
| Newcomer or candidate (OBSERVER) | The few things they can open now: sign-in, the guide, docs, the CLI download |
| Everyday mesh user (DEVELOPER) | Whether the tools they use are up, so they know whether a problem is theirs |
| Partner | Their own service and the shared surfaces, clearly apart from goat's own, with an owner to contact |
| Operator or on-call (MONITOR, PLATFORM-ADMIN) | A banner that goes red only when a live, committed service is down |
| External-service owner | Control over which of their hostnames are listed, under their own name |

## Where rows come from

Every row on the page comes from one list, built once per render, so the rows that are probed and the rows that are described are always the same set.

`templates/isitup-rows.json.j2` builds the list into the `isitup_rows` fact (`tasks/rows-build.yml`, "Build the status-page row list"). It merges three inputs in precedence order, and a row whose id is already on the list is skipped:

1. `isitup_services`, the goatnet's own services (`group_vars/<site>_hosts/vars.yml`). Source `core`, or `offmesh` for a row with `category: external`.
2. `isitup_external_services`, the shared off-mesh catalog (`isitup_external_services_catalog` in `group_vars/all.yml`, opted into per site). Source `offmesh`.
3. External-service declarations, `external_service_*` in `group_vars/<site>_hosts/external-service-<name>.yml` (ADR 1178). There is one row per listed hostname, and a hostname that an earlier row already names is skipped. Source `external`.

A hidden row (`listed: false` or past `retire_after`) still claims its id and hostname. The catalog's copy of a hidden inline row, or a declaration for the same hostname, therefore stays hidden too. zagros and zagros-prod carry the seven catalog rows inline as well (`group_vars/zagros_hosts/vars.yml`, "Off-mesh rows recovered"); the inline copy wins, and each renders once.

Two files are rendered from `isitup_rows`:

- `services.env` is the pipe-delimited list `files/check.sh` probes every five minutes. Its column format (`APP_SERVICES_FORMAT=3`) is unchanged.
- `data/meta.json` records, per row id, the facts the probe does not know: `lifecycle`, `source`, `owner` and `retire_after`. It also records, under `rendered`, the stamp of the list itself (see "Keeping the list current"). It sits in the `data/` directory bind mount, so a deploy updates it without recreating the container. nginx serves it `no-store`, like `current.json`.

The page joins `current.json` (probe results) with `meta.json` by row id. A goatnet deployed before `meta.json` existed has none, and every row then reads as a live core service, which is how the page behaved before this change.

## Keeping the list current

The row list on a host changes only when a play renders it. A row added to or removed from the declaration reaches the page when that render runs, and until then the page probes the previous list: a service that was removed is still probed and shows as down. Four parts keep the rendered list and the declaration together.

| Part | Status | Where |
|---|---|---|
| List-only render | Built | `ops/ansible/isitup-rows.yml`, `roles/isitup/tasks/rows-only.yml` |
| List stamp in `meta.json` | Built | `roles/isitup/tasks/rows-build.yml`, `templates/meta.json.j2` |
| Stale-list check | Built | `ops/scripts/isitup-list.py check`, `isitup-rows.yml -e isitup_rows_verify_only=true` |
| Stamp shown on the page | Designed, not built | `index.html` (not in this repository) |
| Change detection and driver | Built | `ops/scripts/isitup-list.py changed`, `apply` |
| Trigger on merge | Designed, not built | a workflow in the environment repository |
| Pulled list | Designed, not built | "Pulled list" below |

### List-only render

`isitup-rows.yml` renders `services.env` and `data/meta.json` from the current declaration and nothing else. It reads no OIDC client secret and no cookie secret, does not run the `isitup-oidc-client` role, and leaves `isitup.env`, the Traefik routers, the compose overlay and the `oauth2-proxy-isitup` container as they are. When `services.env` changed, or the running container still holds an earlier copy of it, the play recreates the `isitup` container alone, then runs `check.sh` once so `current.json` follows the new list.

```
cd ops/ansible
../scripts/ansible-preflight.sh \
  -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini \
  -e target=zagros_hosts isitup-rows.yml
```

The play refuses a host where the page is not deployed; `isitup-deploy.yml` is the first deploy and stays the play for the sign-in client, the secrets, the routers and the image. Both plays build and write the list with the same two task files (`rows-build.yml`, `rows-render.yml`), so they cannot render different lists from one declaration. The list-only play writes no deploy-ledger row, because it deploys no image. A run against an unchanged declaration reports no change and restarts nothing.

### The list stamp

`data/meta.json` carries the stamp of the list beside the per-row facts:

```json
{
  "rows": { "portal": { "lifecycle": "live", "source": "core", "owner": "", "retire_after": "" } },
  "rendered": {
    "sha256": "<digest of the row list>",
    "rows": 42,
    "at": "<UTC time>",
    "source": { "goatcore": "<commit>", "overlay": "<commit>" }
  }
}
```

- `sha256` is the digest of the row list as canonical JSON (keys sorted, no spaces, ASCII escapes). The same declaration gives the same digest on any machine on the same UTC day. The day matters because a row past its `retire_after` date leaves the list, and the digest moves with it.
- `source` names the commits the list was rendered from, in the form every deploy-ledger row uses (`ops/scripts/lib/source_of_record.py describe`): `goatcore` and `overlay` (and `goatstuff`) for an assembled tree, read from its `.goat-assembled` marker, or `commit` for a plain clone. The row declaration lives in the overlay of an assembled tree, so `overlay` is the commit a reader compares with the declaration's history.
- `at` is the time this list was first rendered from this source. A later run that renders the same list from the same source keeps it, so the file does not change on every run.

A `meta.json` written before the stamp existed has `rows` only. The page reads that shape as before, and the check reports it as unknown, not as current.

**On the page (designed, not built).** The footer shows one line from the stamp: *Service list rendered from `<short commit>`, `<date>`*, where the commit is `rendered.source.overlay`, else `rendered.source.commit`, and the date is `rendered.at`. With no `rendered` object the line is absent. The page cannot decide by itself that the list is behind, because the host does not hold the declaration; the check below does that.

### Stale-list check

`ops/scripts/isitup-list.py check <goatnet> --meta <file | URL | ->` renders the row list from the current tree on the machine it runs on, through `ops/ansible/isitup-rows-render.yml` and the same `rows-build.yml`, and compares its digest with `rendered.sha256` in the given `meta.json`. It contacts no host.

| Exit | Meaning |
|---|---|
| 0 | The page carries the declared list |
| 1 | The lists differ. The message names the rows on the page that are not declared, and the declared rows that are not on the page |
| 3 | The `meta.json` has no stamp |

```
$ ops/scripts/isitup-list.py check zagros --meta meta.json
zagros: BEHIND. The page's row list is not the list this tree declares.
  page:     5b0c41d2a9e7, rendered from 2f6a90c1b3d4 at 2026-01-01T00:00:00Z
  declared: 91e3f7a04c22, 41 rows, from 8d17c5e0a6b9
  on the page, not declared (probed, and shown as down once the service is gone): board
  refresh it: ops/scripts/isitup-list.py apply zagros
```

The comparison is on the digest, not on the commit: a later commit that leaves the rows as they were is not a stale list. The page sits behind the goatnet sign-in, so a URL fetch needs a session header (`--header`); the same comparison runs on the host, with no sign-in, as `isitup-rows.yml -e isitup_rows_verify_only=true` (`isitup-list.py apply <goatnet> --verify-only`), which writes nothing and fails when the host is behind.

### From a merged change to the render

`ops/scripts/isitup-list.py` is the one driver.

- `changed --base <commit> [--head <commit>] [--repo DIR]` prints the goatnets whose row declaration a change can reach. The rule is by path: a file under `ops/ansible/group_vars/<site>_hosts/` names that goatnet; `group_vars/all.yml`, the role's row templates and an overlay's `goatcore.ref` or `goatstuff.ref` name every goatnet with `isitup_enabled: true`. The rule also names a goatnet whose change left its rows as they were; the render is idempotent, so that run changes nothing.
- `apply <goatnet> ...` or `apply --base <commit>` runs `isitup-rows.yml` for those goatnets through `ansible-preflight.sh`, over the mesh, an env door or the base inventory, in that order of preference. It needs no secret. `--dry-run` prints each command.

`external-service.yml` re-renders the page as its last step for one declared service. A change to `isitup_services`, to the off-mesh catalog, or to a declaration's `status:` block outside that play is carried by `isitup-list.py apply`.

**Trigger on merge (designed, not built).** The row declaration is in the environment repository, so the trigger is a workflow there. On a push to its trunk it assembles the tree at the merged commit, runs `isitup-list.py changed --base <before> --head <after> --repo <the environment clone>`, and for each goatnet named runs `isitup-list.py apply <goatnet>` from a runner that has a route to that goatnet. Where no runner has a route, the same workflow runs `apply <goatnet> --verify-only` or `check` on a schedule and fails for a goatnet that is behind, so the gap is reported until an operator runs the render.

### Pulled list

**Status: Designed, not built.**

Fleet convergence is pulled ([ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md), [fleet-management](fleet-management.md) §5): a node fetches its intent and applies it, and no controller has to reach the node. The row list is intent of that kind. The list-only render above is a push, and its trigger needs a runner with a route to every goatnet. The pulled form removes that need.

The rows are templated by Ansible from group_vars, so a host cannot render them from the declaration. The rendered list is therefore published as an artifact, and the host applies the artifact.

| Step | Where it runs | What it does |
|---|---|---|
| Publish | The environment repository's CI, on every trunk commit and once a day | Assembles the tree, renders each goatnet's list with `rows-build.yml`, and publishes one artifact per goatnet |
| Fetch | The management host, on a systemd timer | Fetches the goatnet's artifact from a location inside the site's own reach |
| Verify | The management host | Checks the signature, the digest and the serial. Refuses an artifact that fails any of the three |
| Apply | The management host | Installs `services.env` and `meta.json`, recreates the `isitup` container when `services.env` changed, runs `check.sh` once |

**The artifact.** One per goatnet: the final `services.env`, the final `meta.json` (with its `rendered` stamp) and a manifest holding the goatnet's `site_id`, the list digest, the source commits and a serial that only increases. The host installs the two files as they are and does no templating, so one renderer exists (the role) and the host needs no copy of the declaration. The manifest is signed, and the host verifies the signature before it applies anything; the signed, versioned name snapshot ([name-fallback-resolver](name-fallback-resolver.md)) is the pattern.

**Rules the host applies.**

- A wrong signature, a `site_id` that is not the host's own, a digest that does not match the files, or a serial lower than the one last applied: keep the current list and record the refusal.
- The fetch interval is jittered and grows after a failure (fleet-management §3 P9).
- The result of the last fetch (time, serial, applied or refused and why) is written beside `meta.json`, so the page line and the stale-list check read whether the host is fetching at all.
- A host that cannot reach the artifact keeps its last list. A site that is cut off keeps probing that list, and the stamp on the page shows its age.

**Daily publish.** A row leaves the list when its `retire_after` date passes, with no commit. The publish step therefore also runs once a day, and publishes only when the digest changed.

**What stays.** `isitup-deploy.yml` remains the first deploy. `isitup-rows.yml` remains the path for a site that is cut off from the artifact location and for a change that should not wait for the timer. `isitup-list.py check` compares the same digest in either model.

**Decided before building.** Where each goatnet's artifact is published so that the host fetches it from inside its own site; which key signs the manifest and who holds it; whether a site that is cut off may publish a list for itself.

## Row state

Each row can say whether it should be counted, shown and alerted on, independently of whether its probe passes.

| Field | Where | Effect |
|---|---|---|
| `lifecycle: live` | default | Shown, counted in the banner, probed by the obs host |
| `lifecycle: planned` | `isitup_services` row, declaration `status.lifecycle`, or `status.hostnames.<h>.lifecycle` | Shown with a *Planned* pill and a muted row. Not counted in the banner or section summaries. Not a blackbox target |
| `lifecycle: retiring` | same | Shown muted with *Retiring* and its date. Not counted. Not a blackbox target |
| `listed: false` | `isitup_services` row; `status.list: false` or `status.hostnames.<h>.list: false` on a declaration | Not on the page, not probed by isitup, not a blackbox target. The row's mesh DNS record, certificate name and gym link are untouched, because those read `isitup_services` and `hostnames:` directly |
| `retire_after: YYYY-MM-DD` | row or declaration | Dropped from `isitup_rows` once the date has passed, and hidden by the page from `meta.json`, so it disappears even if nobody re-runs the play |

The role asserts that every `lifecycle` is one of the three values ("Require every row's lifecycle to be one the page understands"). A typo would otherwise be counted as live on the page and ignored by the obs filter.

## Layout

**Home page (`/`).** This shows the goatnet's own rows: sources `core` and `offmesh`. Sections follow `GROUPS` in `index.html`, in newcomer order: account, learn, tools, live, sandbox, utilities, and so on. Each section is a `<details>` element:

- *Sign-in & onboarding*, *Docs & briefings* and *Tools & downloads* start open. Every other section starts closed.
- A closed section's heading carries its row count and any fault, for example `15 · 1 down · 2 not live`.
- A section with a down or degraded live row is always open.
- A reader's own open or close choice is remembered in `localStorage`, keyed per page. Only the reader's clicks are recorded, not a section the page opened for a fault, so that section closes again once the fault clears. This is a convenience only: every call is wrapped in try/catch, and the page renders the same without it.
- A planned or retiring row's dot is the neutral grey ring whatever its probe says; the detail line still shows the measurement.

**External services (`/external`).** This page is the same SPA and reads the same data. nginx routes `/external` (with or without a trailing slash) to `index.html` (`templates/nginx.conf.j2`), and the page picks its view from the path. It shows only `source: external` rows, grouped under their declaration's `owner`, with a *Hosted by …* line on each row and the `retire_after` date when there is one. A note at the top says these services are run by their owners, not the goat core team. The home page carries a one-line summary linking here, for example *External services — 9 hosted by members, not the core team · 1 with a fault*.

The *This goatnet's services / External services* switch appears only when external rows exist, because the same `index.html` also serves the legacy relay-tier board (`ops/status/`), which has no `/external` route.

**Banner.** On each page the banner counts only that page's `live` rows. Planned, retiring and past-date rows never reach it.

## Alerting

Paging comes from the obs hosts' blackbox exporter, not from isitup. A probe failure there fires the TLS and surface alerts routed by Alertmanager to ntfy (ADR 1168). The status page itself sends nothing. Taking a row out of alerting is one edit to its registry row on both obs stacks:

- **goat-obs stack** (`ops/compose/goat-obs/prometheus/prometheus.yml`, zagros). Blackbox targets are rendered from `isitup_services`. The `blackbox-https` and `blackbox-surface-app` loops skip any row that is not `lifecycle: live`, is `listed: false`, or is past its `retire_after` date.
- **obs-host role** (`ops/ansible/roles/obs-host/`, zagros-prod). Targets are the hand-kept `obs_blackbox_targets` in `group_vars/<site>_obs_hosts/vars.yml`. The role reads the mgmt host's vars through `hostvars`, from the group named by `obs_blackbox_registry_group` (for zagros-prod, `zagros_prod_hosts`), and renders `templates/registry-rows.json.j2`. That gives one `{fqdn, lifecycle, listed}` per `isitup_services` row and per external-service hostname, with a declaration's `status` settings, `enabled: false` and a past `retire_after` folded into `listed`. `templates/prometheus.yml.j2` and `templates/docker-compose.yml.j2` then drop every target whose host is on a non-live or unlisted row. A named group that is absent from the inventory fails the play.

Two CI checks hold this in place:

- `ops/ci/check-blackbox-covers-registry.sh` requires every live, listed `probe_gate` row to have a target, and requires `obs_blackbox_registry_group` to name the site's own mgmt group on every goatnet with blackbox enabled.
- `ops/ci/check-prometheus-template-renders.sh` renders the goat-obs template with planned, retiring, unlisted (including a quoted `"false"`) and past-retire fixture rows plus a live control row, and fails if any of them is a target or the control row is dropped. It renders the obs-host templates with the real zagros-prod registry and declarations, and again with the first target's row marked planned. It also checks `registry-rows.json.j2` directly: a declaration's lifecycle, a per-hostname `list: false`, `enabled: false` and a past `retire_after` must each reach the obs host.

## Tests

`ops/ansible/roles/isitup/tests/syntax-check.sh` is the role's CI entry point (`.github/workflows/ansible-syntax-check.yml`). It runs:

- `row-list.py`: which rows reach the page. Covers dedup by id, `listed`, `retire_after`, lifecycle, source, owner, per-hostname `list` and `health_path`, and checks that `services.env` and `meta.json` describe the same set. Also covers the list stamp: `meta.json` carries the digest, row count, source and time, and the digest changes when a row is removed or passes its `retire_after` date.
- `free-text-columns.sh`: `name` and `desc` survive quoting into `services.env`.
- `front-door-probe.sh`: the check.sh front-door grading.
- `list-freshness.sh`: the list-only play and the render play parse; the row list renders from a declaration that sets no sign-in secret; the stamped digest is the sha256 of the canonical row list and matches the test harness; `isitup-list.py check` exits 0, 1 and 3 for a current, a behind and an unstamped `meta.json`.
- `ops/scripts/test_isitup_list.py`: which goatnets a set of changed paths names, the three verdicts, and `apply --dry-run`.

## Related

- [external-service-golden-path](external-service-golden-path.md) §"On the status page": the declaration fields an owner sets.
- dns-service-discovery: why the off-mesh catalog is a separate list from `isitup_services`.
- Gym module `read-the-status-page` (`ops/gym/gym.yaml`, `ops/gym/content/read-the-status-page.md`).
- Issue #1518.

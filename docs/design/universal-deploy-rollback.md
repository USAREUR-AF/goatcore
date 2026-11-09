# Universal deploy-rollback ledger

**Status:** **Accepted — deployed + live-fired on ZAGROS 2026-06-25 (M-075).** Phase A (this doc + [ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md), PR #1307) landed the design; Phase B (PR #1310) built the shared role + `rollback.sh` host-truth/dispatch upgrade + the ratchet (pilots proven locally); Phase C (PR #1314) fanned the meta-dependency out to all 55 deploying roles, removed all 22 hand-rolled tasks, converted `airdefenders` to `import_role`, and emptied the ratchet allowlist. See the built-vs-designed map below.

Generalizes [`easy-rollback.md`](easy-rollback.md) / [ADR 0859](../adr/0859-easy-rollback-engineering-principle.md) from a per-role opt-in into a background capability every deploying component gets automatically.

## Why (the gap, re-verified this session — evidence, not inference)

`ops/ansible/roles/` greps:

- 94 roles; **55** deploy a container (the rollback universe — the authoritative count is the ratchet's detector over `tasks/`+`handlers/`; an initial narrow grep under-counted at ~47 by missing handler-driven `docker compose … up -d --force-recreate` deploys like the dashboards).
- **35** have an in-repo `deploy-history.yml`; **22** carry the hand-copied runtime-ledger task; the rest deploy with no rollback substrate.
- **12** group_vars files pin a `*_image:` outside role defaults. Both the CI gate (`check-deploy-history.sh` scans only `roles/*/defaults/main.yml`) and `rollback.sh` (reads `${ROLE_DIR}/deploy-history.yml`) are blind to them.

Root causes: (1) opt-in copy-paste → drift; (2) the tool reads the role-default ledger, not the host record of what actually ran; (3) "rollback" is one verb for heterogeneous artifacts, some forward-only.

## The shared role contract — `roles/rollback-ledger/`

Every deploying role gains the ledger by declaring a meta-dependency. No tasks in the service role; the shared role owns the append.

```yaml
# roles/<service>/meta/main.yml
dependencies:
  - role: rollback-ledger
    vars:
      rollback_component: goat-zenoh-portal     # ledger dir + tool key
      rollback_artifact_type: image             # image|config|acl|data|cert|forward-only
      rollback_ref: "{{ portal_image }}"        # the var actually deployed (role-default OR group_vars OR env)
      rollback_reversible: true                 # false → tool refuses
      rollback_restore_pointer: ""              # optional; required for data/cert/forward-only
```

`roles/rollback-ledger/`:

- **`meta/main.yml`** sets `allow_duplicates: true` — **mandatory**. Without it Ansible de-dupes the meta-dependency and only the first service role in a play writes a row.
- **`tasks/main.yml`**: ensure `/var/lib/goat/<component>/`; read the last ledger line (`slurp` + last-line); append a row **only if `ref` changed**, or, for a deploy from an assembled tree, the row's `source_tree` names a different overlay commit or set of pre-disclosure patches than the previous row (a patch to role files alone changes no ref) (dedup — an improvement over the 0859 tasks, which appended one row per ansible run because the templated `ts` defeated `lineinfile` idempotence); set `prior_ref` from the previous row's `ref`. `changed_when: false` (audit-only).
- `defaults/main.yml`: `rollback_artifact_type: image`, `rollback_reversible: true`, `rollback_restore_pointer: ""`.

**Ordering caveat (honest).** Meta-dependencies run at role *entry*, so the ledger records the ref the run *intends* to converge to, not a post-success fact. Mitigation: the tool reconciles every rollback target against the live container (`docker inspect`) before acting, so an optimistic row from a failed mid-deploy never drives a wrong rollback; the next successful run is a deduped no-op. The in-repo `deploy-history.yml` (CI-gated, merged-image record) remains the second source during the transition.

**Invocation caveat (honest).** Meta-dependencies fire for `roles:` and `import_role` (static) but NOT for `include_role` (dynamic). The fleet is overwhelmingly `roles:` (73 playbooks vs 4 `include_role`, 3 `import_role`). Of the deploying roles invoked via `include_role`: **`goat-relay`** (also invoked via `roles:` in `goat-relay-deploy.yml`/`goat-relay-tune.yml` — the ledger fires there; the `include_role` is only the tier4b load-bench path, left as-is because it has a documented recursion hazard) and **`zenoh`** (integdev-lab only since the 2026-08-02 role-selection fence; invoked via `roles:` in `integdev-bootstrap.yml` / `integdev-refresh.yml`, where the ledger fires — its site-bootstrap `include_role` call-site is gone, and `nova-zenoh-redeploy.yml` with it, so no goatnet reaches this role at all) are both already covered by their `roles:` deploy paths. **`airdefenders`** had *no* `roles:` path — its only deploy is the `include_role` in `airdefenders.yml`, so Phase C converted that one call-site to `import_role` (unconditional, no per-invocation vars → a safe static swap). The coverage ratchet checks for the meta-dependency stanza, not the invocation style.

## Ledger schema

`/var/lib/goat/<component>/deploy-history.jsonl`, append-only NDJSON, newest line last:

```json
{"component":"goat-zenoh-portal","artifact_type":"image","ref":"ghcr.io/...@sha256:...","prior_ref":"ghcr.io/...@sha256:...","ts":"2026-06-23T...Z","operator":"maintainer@host","reversible":true,"action":"deploy","image":"ghcr.io/...@sha256:...","via":"ansible-role-rollback-ledger"}
```

Additive over the 0859 jsonl: `image`/`action`/`via` retained for back-compat; `ref` aliases `image` for image type. `restore_pointer` present for data/cert/forward-only. Rollback writes its own `{"action":"rollback",...}` rows as today.

`ts` is when the play reached the ledger role, which runs first as a meta-dependency, so it is the play's START. The row is written `"action":"pending"` there and promoted to `"deploy"` on hosts that did not fail; since 2026-10-29 the promote also stamps `promoted_at`, the host's UTC clock at that moment, so the row carries both when the deploy started and when it finished. `ts` keeps its meaning for every existing reader.

**Two promotion modes, one promotion (`rollback_promote_mode`, F-530 fix 2).** The default is `handler`: the row is queued at role entry and promoted by the end-of-play handler, which ansible runs at the end of the play and only for hosts that did not fail — exactly the "did the host survive?" signal the ledger needs. That is correct *only* for a role that runs no `meta: flush_handlers` before its own bring-up/health/smoke. `meta: flush_handlers` is PLAY-scoped: a role that flushes mid-role (to land a compose or cert change before it smoke-tests) also fires the promote handler at that flush, stamping `deploy`+`promoted_at` BEFORE its health gate — so a task failing after the flush left a `deploy` row (with a finish time it never reached) for a deploy that did not finish, and `deploy-component.sh` / goatbuilder read the host as converged. The ~19 such roles set `rollback_promote_mode: explicit` in their rollback-ledger dependency stanza: the meta-dep still writes the `pending` row at entry but does NOT queue it for the handler, and the role promotes itself from the END of its deploy path with `ansible.builtin.include_role: { name: rollback-ledger, tasks_from: promote }`, passing the same `rollback_component` + `rollback_ref`, as the LAST step after health/smoke. Position is the correctness argument (as for the render manifest): a task placed after the gate runs only if the role reached it, so a role that fails earlier leaves its row `pending` — its truth. Both paths run the one `promote-pending.py`, so the row they write is identical, and promotion is idempotent (a no-op on a row already `deploy`). `ops/ci/test-ledger-promote-pending.py` enforces the invariant across the repo — every role that both depends on rollback-ledger and runs `meta: flush_handlers` is `explicit` and carries the promote include — so it can only hold, never regress. `rollback --list`, `history`, `verify` and `diff` print `promoted_at` as "finished" when a row has it. A host whose newest row is `"action":"rollback"` names the declared ref only as its `prior_ref`; both avenues report it as rolled back from that ref, not as an unfinished play.

**Served payloads.** A role that serves release assets it downloads at deploy time (goat-get: the goat CLI, goatbuilder, the grant-crypto SDK, the Jetson kmod, octo-pod; goat-docs: the goat book and the goatbuilder book) is versioned by role vars (`*_release_tag`, `*_enabled`), not by its image, so a row naming the right nginx digest says nothing about which release the page carries. Such a role declares its payloads once, in its rollback-ledger stanza: `rollback_served` (each `{name, tag_var, enabled_var?, file}`) under `rollback_served_root`. The row then carries `served_declared` (what the play's vars meant) and, written by the promote at the end of the play, `served` — each file read back from the host (`absent` when not there, `unreadable` when it names no tag). A payload change with an unchanged image is a change, so it writes a row. `ops/scripts/served-payloads.py` resolves the same declaration from the repository for a member group (member group_vars → group_vars/all → role defaults) and judges a row: both `deploy-component.sh --verify-only` and `goatbuilder deploy`/`verify`/`diff` call it, so a host serving another release than main declares is drift (exit 1 / 3), and a row that records no `served` is unknown, never converged. This replaced the hand-kept `served_*_release_tag` fields in the roles' `deploy-history.yml`. For the declared-end-state model, the payload map per host is a fact like the image digest. The same stanza covers a role that INSTALLS a release rather than serving it: `onboard-runner` records the host `goat` it bind-mounts (a marker written from `goat --version`), judged against the goat-cli goat-get declares — `tag_role: goat-get` names the role whose defaults carry the tag, so main's one `goat_get_release_tag` governs both; `snitch-bundle` includes the ledger from its Linux tasks (a Darwin peer has none) with the staged release tag as its ref (`config`: rollback is re-staging the previous tag with `snitch_bundle_force_refresh`) and the staged release as `served`. Other roles that fetch a pinned third-party release at deploy time (ntp-hygiene's chrony_exporter, ollama's native binary, oci-substrate-t2-sync's crane) are not yet recorded this way.

## Tool — dispatch by artifact type

`ops/scripts/rollback.sh <component> [N]` (eventual `goat admin rollback`), upgraded to **read the on-host jsonl as authoritative** (falling back to the in-repo `deploy-history.yml` only for the `hosts:` map + during transition), then dispatch:

| type | tool v1 behavior |
|---|---|
| `image` | **automated** — re-pin compose/env + `docker compose up -d --force-recreate` (the 0859 path), reconciled vs live `docker inspect`. |
| `config` | **guided** — print the git SHA to revert + the redeploy command. Auto in v2 (ADR 0224). |
| `acl` | **guided / refuse** — print the revert SHA + warn that a full re-render clobbers portal-appended rules; require a surgical edit. |
| `data` | **refuse** + print `restore_pointer` (pgBackRest stanza / Object-Lock bucket). A swap cannot undo a committed write. |
| `cert` | **refuse auto** + print `goat admin rotate <anchor>` (ADR 1044). |
| `forward-only` | **refuse** — `reversible:false` / `do_not_roll_to`. |

`--list` / `--status` extend to read the host ledger. Refusals are explicit and name the safe alternative.

**One goatnet per roll — fail-closed (GB-055).** A component's ledger lists the hosts of every goatnet it runs on (`goat-get`: nova-a, zagros-dev, zagros, zagros-prod). A roll with no `--site` would flip them all at once, prod included, so it is refused when the ledger spans more than one goatnet: the error names the goatnets and the operator re-runs with `--site <goatnet>`. A component on a single goatnet still rolls back with no `--site`. `--site` narrows the roll to the hosts whose `site:` is that goatnet — the host→goatnet fact `generate-pins.py` resolves and records in the ledger, and the same field `--list` / `--status` and a rollback plan narrow on. A plan carries its `--site` through to `apply`, which refuses (GB-053) if the goatnet's host set moved under it, so a reviewed plan rolls back only the goatnet it was reviewed for. Both avenues carry the guard (ADR 1201): `goatbuilder rollback` always; `rollback.sh` wherever jq is present (the same gate as its host-truth path).

**Reaching a goatnet through its door.** With `--site`, both avenues reach a goatnet whose map for that site is one host through its env door ([env-doors.md](env-doors.md)) when the mesh does not reach it. `rollback.sh` tries direct ssh first and then the door that `door-open.sh` opens. `goatbuilder rollback` calls `deploy`'s own transport decision (`push::transport`: the netbird profile, then the mesh host on tcp/22, then the door registry), and when that decision is the door it reaches the host at `127.0.0.1:<port>` as the map's `ssh_user`, where `env-door.sh <site> --ssh-target` names the port. A door that does not open stops the rollback with GB-006, the same error `deploy` gives. With no `--site`, both avenues use direct ssh, and a multi-host map never goes through a door, because a door lands on one host. **Built** (issue 261): `rollback.rs` tests `a_site_behind_its_door_is_reached_the_way_deploy_reaches_it`, `a_site_with_no_door_is_reached_directly_as_deploy_does` and `a_door_that_does_not_open_stops_the_rollback_as_it_stops_deploy`, on a fixture registry. Status: WIP — gate: the live canary's rollback step passes through the door of a dev goatnet.

## CI coverage ratchet — fail-closed

`ops/ci/check-rollback-coverage.sh` (mirrors `check-vendor-surface-ratchet.sh`): enumerate deploying roles (a role whose `tasks/` OR `handlers/` references a container-deploy primitive — `docker compose … up`, `--force-recreate`/`force_recreate`, the `docker_compose`/`docker_container` modules; README/`files/` prose is excluded, bare `restart` is not a deploy; unioned with roles that already carry a `deploy-history.yml`); for each, require `meta/main.yml` to depend on `rollback-ledger`. A deploying role without the dependency fails the build. A baseline allowlist carries the not-yet-migrated roles during Phase C and only shrinks — coverage can only grow. The existing `deploy-history-check.yml` (image-pin → in-repo ledger append) stays as-is; the two gates are complementary.

## Built vs designed (honest map)

| Element | State |
|---|---|
| ADR 1060 + this design doc | **Phase A — built** (PR #1307). |
| `roles/rollback-ledger/` shared role (meta-dep, `allow_duplicates`, dedup-on-ref, prior_ref tracking) | **Phase B — built.** Proven for-real against localhost: deduped an identical re-run, tracked prior_ref img:v1→img:v2, emitted a proper JSON bool. |
| `rollback.sh` reads host-truth jsonl + dispatch-by-type + reversible refusal | **Phase B — built.** End-to-end rehearsal of the real script (transport + dockerd shimmed; sed/jsonl/dispatch logic real): 14/14 — image flip via host-truth target resolution, idempotence, data/cert/forward-only refuse (+pointer), config/acl guided (no auto-flip), `--status` artifact type. |
| `check-rollback-coverage.sh` + `rollback-coverage.yml` workflow | **Built.** After Phase C: 55 deploying / **55 covered** / 0 allowlisted / baseline 0; fail-closed proven (a new uncovered deploying role → exit 1). |
| Pilots migrated: portal (group_vars pin) + panoscope (role-default pin) | **Built + live.** Hand-rolled ledger tasks removed; meta-dep added; the portal's meta-dep fires in production on ZAGROS. |
| Fan-out: meta-dep on all 55 deploying roles; migrate the 22 hand-rolled tasks; convert the `include_role` call-site | **Phase C — built.** All 55 covered (2 pilots + 53 fanned out); 22 hand-rolled tasks removed; `airdefenders` → `import_role` (`goat-relay`/`zenoh` already covered via their `roles:` paths); allowlist empty. Per-goatnet ledgers populate on each env's next deploy. |
| Deploy + live-fire | **Done — M-075.** `cert`/`forward-only` refusals stay unit-level (no deploying role declares those types). |
| `config` auto-revert (vs guided) | Designed, deferred to ADR 0224. |
| `goat admin rollback` (vs `rollback.sh`) | Designed; bash script is the floor (0859 open question). |

## SRE strategy — rollback is a standing build-out invariant (canon)

Deploy-rollback is part of how we stand up and run every goatnet, not a one-off. Two layers, with different enforcement:

- **Write (universal, fail-closed, automatic).** Every deploying role carries the `rollback-ledger` meta-dependency, enforced by the `check-rollback-coverage.sh` ratchet (allowlist empty, baseline 0). A new deploying component **cannot ship without recording its deploy ledger** — so the "what ran, and what ran before it" record exists fleet-wide, for free, on every deployment. This is the foundational SRE guarantee and it is on.

- **Target (per-goatnet, wired at standup).** To *execute* a rollback, `rollback.sh` needs the component's connection map — `{host, ssh_user, container_name, compose_path | compose_files}` per goatnet — in its `deploy-history.yml` `hosts:` block. This is **per-goatnet curation**: the role→container→compose mapping is heterogeneous (renamed containers, multi-container roles, mgmt-host overlay services with multi-file compose), so it is recorded, not derived. The map is a connection map only — the on-host jsonl ledger remains the source of truth for *what* to roll to.

**Build-out obligation (every new goatnet, in the standup sequence):** after the stack is up, run the coverage report and close any gap:

```
ops/scripts/rollback-coverage.sh <site> <host> [ssh_user]
```

It cross-references each deploying role's on-host ledger (deployed-here) against its `deploy-history.yml` hosts-map (wired) and prints the **deployed-but-unwired** set — the exact components to add a hosts entry for. A goatnet is rollback-complete when that set is empty.

**Ledger-file gap closed repo-side, 2026-08-03 (PR #2058).** Until then, 22 of the 68 deploying roles carried the meta-dependency but shipped **no `deploy-history.yml` at all**, which made them a different and worse problem than an unwired host: `wired()` returns false when the file is absent and `rollback.sh` refuses outright, so no `hosts:` entry could make them targetable on any goatnet. All 68 now have a ledger. Roles that run on no goatnet ship `hosts: []` with their container name and compose path pre-filled in a comment, so standup supplies only host and site. This closes only the **repo half** — per-goatnet wiring at standup is unchanged and still required.

Two schema edges surfaced while doing it, both recorded rather than papered over: `artifact_type: script` (used by `host-net-publish` and `netbird-boot-selfheal`) is not one of the six types `rollback.sh` dispatches on, so those fall through to the image path and skip the host; and an env-var-pinned **mgmt-host compose overlay** cannot be fully rolled by either branch — the `env_file`/`env_var` branch flips the durable pin but then recreates from the compose dir without the `-f` overlay, and the `compose_files` branch cannot rewrite an `image: ${VAR:-...}` line. The env branch is the one to prefer (it leaves the pin correct), finished by an explicit two-`-f` recreate.

**Honest scope of the target layer:** `image` rollback is automated (single- or multi-file compose); `data`/`cert`/`forward-only` refuse with a pointer; `config`/`acl` are guided. Rolling a multi-container or mgmt-host-overlay infra component is supported (`compose_files:`) but is a heavier, operator-careful action than rolling a single goat service.

## Composition

[ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md) · [`easy-rollback.md`](easy-rollback.md) / [ADR 0859](../adr/0859-easy-rollback-engineering-principle.md) · [ADR 0224](../adr/0224-progressive-config-rollout-with-auto-revert.md) (config) · [ADR 1044](../adr/1044-credential-rotation-in-place-reissue.md) (cert) · [ADR 0811](../adr/0811-audit-log-integrity.md) + [`pgbackrest.md`](pgbackrest.md) (data) · `instance-variability-model.md` (pin classes) · `goatnet-turnkey-standup.md` (the build-out sequence that wires rollback-targetability per goatnet) · [`fleet-management.md`](fleet-management.md) / [ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md) — this ledger is the fleet-management dimension's **undo** primitive, and the on-host `deploy-history.jsonl` it writes is the data source a fleet-wide desired-vs-actual view reads (its FM-1 increment).

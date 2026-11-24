# Pod carriage at the core router

| Field | Value |
|---|---|
| Status | Built: the portal renders the carriers, the router template binds them behind `zenoh_carriage_core_enabled` (off by default). Proved with the router's decision rules (`tests/carriage-core-check.py`) and measured on one goatnet's live router, on and off, with a rollback and a roll-forward |
| Design | [`pod-carriage-and-provenance.md`](../design/pod-carriage-and-provenance.md) §2, §7.3, §8 phase 4; [ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md) |

A device behind a pod publishes under its own slot. The core router sees the pod's certificate, not the device's, so it
admits those writes only when the pod is a carrier of that slot. The portal keeps the carriers in the slot fragment, which it
already splices into the router's config for slot subjects:

```text
carry-<anchor>   the leaf CNs allowed to write <anchor>'s slot on its behalf
```

A pod is a carrier of every source its rule generation lists. An ingress adapter is a carrier of every keyless source that
names it. The router template binds `carry-<anchor>` to the slot's own write rule `slot-w-<anchor>` only: a carrier writes
exactly what the source may write, and gains no read.

## Turning it on for a goatnet

| Step | How |
|---|---|
| 0. The portal can sign pod generations | Set `portal_carriage_enabled: true` in the goatnet's group_vars for the portal host, set `portal_carriage_signer_key_pem` (ECDSA P-256 private key) and `portal_carriage_signer_cert_pem` (its certificate) in the overlay's group_vars from the goatnet's own prefixed environment names (`<SITE>_PORTAL_CARRIAGE_SIGNER_KEY_PEM` / `_CERT_PEM`, references through `ops/scripts/with-secrets`), and run the portal role. The role writes them under `<portal data dir>/carriage/` (key 0600, never logged), mounts them read-only and sets `PORTAL_CARRIAGE_SIGNER_KEY/_CERT`. The deploy driver's secrets check names a missing one before it touches a host, and with the flag on the role refuses. With both unset and the flag off, the role prints one line and the generation route answers 503 `config` |
| 1. The portal renders `carry-` subjects | Any portal with this build does, at boot, on every carriage change and every minute; nothing to set |
| 2. The router binds them | Set `zenoh_carriage_core_enabled: true` in the goatnet's group_vars for the router hosts and run the router role once |
| 3. Check | `goat-zenoh-portal carriage-drift --db-dsn <dsn> --acl-config <router config> --roster-path <roster.json>` on the portal host, or `GET /api/carriage/drift` with an operator key; `in sync` or one line per difference |

Steps 0 and 2 are the only role runs. If the router config was rendered from a fragment older than the portal upgrade, each carriage
policy is bound to an inert `carry-<anchor>-none` subject and the drift check reports `inert` for every carried slot; one role
run after step 1 binds the real subjects. After that, a rule change reaches the router through the fragment alone.

## Turning it off, and rolling the portal back

The core switch and the portal roll back separately. Each step leaves the goatnet in a state it can stay in.

| Step | How | Check |
|---|---|---|
| 1. Before the portal upgrade | `pg_dump -Fc` of the portal's database, kept on the host with mode 0600 | `pg_restore --list` reads it |
| 2. Core switch off | `zenoh_carriage_core_enabled: false`, run the router role. The router restarts | No policy names a `carry-` subject. A pod writes its own slot and is refused a carried one. The rendered config equals the render of a template without carriage |
| 3. Compare the dump with the live database | Restore the dump into a throwaway Postgres with no network; compare row counts and content per table, and list `audit_events` newer than the dump | Every row the restore would drop has a known owner. Otherwise stop: the restore discards it |
| 4. Stop the portal, dump again | `docker stop`, then `pg_dump -Fc` under a new name | The newest dump is the roll-forward source |
| 5. Restore | Rename the live database aside, create an empty one with the same owner, `pg_restore --exit-on-error` the dump from step 1 | Schema version and row counts equal the dump's; tables owned by the portal's role |
| 6. Older portal image | Revert the portal pin, set `portal_carriage_enabled: false`, run the portal role | Portal healthy; `GET /api/carriage/pods` answers 404; the slot fragment it renders holds no `carry-` subject |

Machines enrolled after the dump lose their slot until the roll-forward, and the router restarts when the older portal
re-renders the slot fragment.

Roll forward in the reverse order: stop the portal, check that the restored database took no writes, swap the databases back,
deploy the newer portal, then the router with the switch on. Generate the forward intent from the rollback's
committed intent, so its serial is higher than the one the rollback stamped on the host. Remove the spare database after
the roll-forward is verified.

Merge the trunk into the deploy tree immediately before step 4: the deploy driver refuses a tree behind the trunk, and by then the
portal is stopped.

## When a change takes effect

zenohd 1.9 reads its ACL only at start (F-156). The router host's reload watcher
(`roles/goat-zenoh/files/slot-fragment-reload.py`) recreates the router:

| Change | Reload |
|---|---|
| A carrier removed (rule revoked or expired, opt-out, exclusion, pod or adapter revoked) | Prompt: within the watcher's minimum gap |
| A carrier added (pod registered, rule granted or accepted, reactivated, ratified) | Batched: within the watcher's interval |

Until the router reloads, `GET /api/carriage/drift` shows the difference under `loaded` (the fragment the router runs) while
`config` is already in sync.

## Drift check output

| Line | Meaning | Fix |
|---|---|---|
| `<cn> should carry <slot>, the router does not admit it` | A rule makes `<cn>` a carrier; the router config or loaded fragment lacks it | Wait for the reload (`loaded` only), or check that the portal can write the router config |
| `<cn> carries <slot> with no rule` | The router admits a carrier the rules do not name: a hand edit, or a revocation not yet loaded | Never hand-edit carriage; the next render removes it and the watcher reloads promptly |
| `no carry- subject for <slot>` | The fragment predates carriage | Start the portal with this build; it re-renders at boot |
| `carriage policy of <slot> binds the inert -none subject` | The router role last rendered before the portal's `carry-` subjects | Run the router role once |
| `the router template binds no carry- subject` | `zenoh_carriage_core_enabled` is off while rules name carriers | Step 2 above |

## Carriage health per pod

`GET /api/carriage/pods` (`goat carriage pod ls`) carries `health` for each pod:

| Field | Meaning |
|---|---|
| `state` | `never_pulled`: the pod has not fetched a generation. `current`: it holds what the rules say. `behind`: the rules changed since its last pull |
| `last_pulled_generation`, `last_pulled_at`, `pull_age_seconds` | The last generation the portal served the pod, and when |
| `last_ledger_at`, `ledger_age_seconds` | The pod's last ledger upload |
| `pending_local_rules` | Local rules from the pod waiting for an org lead |

A pod that is cut off keeps enforcing its last generation; its `pull_age_seconds` grows, and its state turns `behind` as soon
as a rule covering it changes.

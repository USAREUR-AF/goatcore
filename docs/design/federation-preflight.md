# Federation preflight (phase 1)

## Status

**Built:** the phase-1 checks below in `ops/federation/standup/preflight.py` and
`preflight_live.py`, run by both avenues (`goatbuilder federate` and
`ops/scripts/peering-standup.sh`) as the same phase-1 command; the read-only
probe play `ops/ansible/peering-preflight.yml`; input derivation in
`crates/goatbuilder/src/federate/preflight.rs`; codes GB-130 to GB-149 in
`ops/errors/catalog.yaml`. Unit tests cover each check with fakes.
**Not built:** the allocated per-peering values are not yet passed in (the
caller passes `None` until allocation lands, and preflight.py reads the group's
own vars and then the roles' port defaults); the phase-10 restart step that calls
the slot recheck. **Not measured:** no run against real hosts.

Status: WIP — gate: the federation test environment's port-collision, wide-store
and slot-drift scenarios refuse at phase 1 on this branch, and a clean two-party
peering passes phase 1 with no input exported by hand, on both avenues.

## What it stops

Phase 1 runs before anything is measured, signed or deployed. Each failure the
first driven runs hit late is checked here first, and each check names its fix:

| id | Checks | Code when it does not pass |
|---|---|---|
| `inputs` | every input the later phases need is present; each missing one is listed with where to declare it | GB-130 |
| `edge-ports` | the tunnel listen port (UDP) and the edge router listen port (TCP, by address) are free on the edge; NetBird holds UDP 51820 on a mesh host | GB-131, GB-133 |
| `core-ports` | the republisher metrics port is free on the core (only when we offer a release set) | GB-132, GB-133 |
| `slot-anchors` | the committed slot list equals the registry (`sync-slot-anchors.py --check`) | GB-134, GB-135 |
| `core-router` | the core router container is found, by the selection phase 3 uses | GB-138, GB-139 |
| `ingress-stores` | once the core admits the peering, it has a receive store equal to each scope the edge admits (only when we accept data; before admission, NOT APPLICABLE) | GB-136, GB-137 |
| `signing-ca` | our CA certificate is a CA, and its signing key resolves and matches it | GB-140, GB-141 |

The checks that were already there keep running and now carry codes: the
declaration and profile (GB-144), the signing key (GB-142), the peering
directory values (GB-147), the working directory (GB-148), the inventory groups
(GB-143), C1 (GB-149), the core certificate's SANs (GB-145) and the local
`docker compose` (GB-146).

## Outcomes

Every check reports one word: PASS, FAIL, BLOCKED (an input it needs is
missing), SKIP (not checked; never counted as a pass) or NOT APPLICABLE (the
deployment has nothing for it to look at, such as no lazy slot storage, or a
send-only declaration for the receive stores). Phase 1 refuses on any FAIL or
BLOCKED. A SKIP is printed in the final line as not verified; with
`FED_PREFLIGHT_STRICT=1` it refuses too. `FED_PREFLIGHT_NO_LIVE=1` skips the
live reads, and each is reported SKIP.

The results are written to `<work>/preflight/result.json` (schema
`goat-federation-preflight/v1`): one object per check with `check`, `label`,
`outcome`, `detail`, `code` and `fix`.

## Inputs: derived, not exported

`goatbuilder federate` derives each input from the deployment and exports the
ones the operator has not set before it builds the plan, so phase 1, the
commands of phases 3 to 14 and their helpers read one value. A variable the
operator sets always wins, which keeps the script avenue working with values
exported by hand. A value no source declares stays unset and GB-130 names it.
A declared value holding a quote, `$`, a backtick, a backslash or a control
character is not exported either, because the phase commands run through
`bash -c` with values in single quotes; preflight then names it as missing.

| Variable | Read by | Derived from |
|---|---|---|
| `FED_WORK` | driver, script, directory.py | `--work`, else `/var/lib/goat-federation/<peering>` |
| `FED_COUNTERPARTY_ID` | preflight, directory.py | `peerings: with` |
| `FED_CORE_GROUP` | preflight | the allocated core group, else `<with>_core` |
| `FED_SLOT_ENV` | preflight | `peerings: environment` |
| `GOAT_AWS_PROFILE` | script, plays, preflight probe | `deploy.aws_profile`, else `default` |
| `FED_WG_INTERFACE`, `FED_WG_LISTEN_PORT`, `FED_EDGE_LISTEN_PORT`, `FED_REPUBLISHER_METRICS_PORT` | preflight | the allocated values; preflight.py then reads `peering_wg_*`, `goat_zenoh_edge_listen_port` and `goat_fed_republisher_metrics_port` from the group's vars, its parent group's (`peering_edges`, `peering_cores`) and `all`, then the roles' defaults. A templated declaration is not guessed: the port check is BLOCKED until the value is exported. host_vars are not read |
| `FED_SIGNING_KEY` | preflight, phase 3, directory.py | `goat_federation_signing_key` in the edge group's vars |
| `FED_DIRECTORY_URL`, `FED_NO_DIRECTORY`, `FED_PACKAGE_URL` | preflight, phase 5 | `goat_federation_directory_url`, `goat_federation_no_directory`, `goat_federation_package_url` |
| `FED_CA_CERT` | preflight, phase 5, phase 8 | `goat_federation_ca_cert`, else `ops/ansible/enrollment/ca/<site>/root-cert.pem` (the portal role's `portal_ca_root_src`) |
| `FED_CA_KEY` | phase 8 | `goat_federation_ca_key` (a plaintext PEM path) |
| `FED_CA_KEY_FROM_AGE` | phase 8 | `goat_federation_ca_key_from_age: true` gives `1`; unset otherwise (phase 8 then opens no age-encrypted key) |
| `FED_CA_KEY_AGE`, `FED_AGE_IDENTITY` | preflight, phase 8 | `goat_federation_ca_key_age`, else `<deploy.state_dir>/<site>-ca/ca.key.age` (`portal_ca_key_src`); `goat_federation_age_identity`, else `~/.config/age/keys.txt` |
| `FED_CORE_SAN` | preflight, phase 8 | the addresses `group_vars/<edge>/deployment.yml` dials |
| `FED_ROUTER_DOCKER_HOST` | preflight, phase 3 | `ssh://<ansible_user>@<ansible_host>` of the one host in `[<core group>]` (`deploy.ssh_user` when the host names no user); nothing when the group has several hosts |
| `FED_SLOT_DB` or `FED_SLOT_REGISTRY_STORE` with `FED_SLOT_PG_SSH` | preflight | not derived; `portal_store` and `portal_db_*` in `group_vars/<env>_hosts/` are read for the rest |

Preflight proves the age-encrypted key opens and matches the certificate,
decrypting in memory and writing nothing. Phase 8 opens the same key itself when
no plaintext key file is given and the deployment opts in
([federation-certificate-lifecycle.md](federation-certificate-lifecycle.md#the-ca-key)).

### Inputs declared in goatbuilder.yaml

Built: `crates/goatbuilder/src/federate/declared.rs` reads the optional
declarations below and merges them into the inputs above. A deployment that
declares none of them gets exactly the inputs it got before. A declared value
fills an input nothing else derived, and replaces one derived only by convention
(for example the enrollment CA path); a value in the edge group's group_vars
still wins over it, and a variable set in the environment wins over both.

Status: WIP — gate: the federation test environment's nothing-hand-written
scenario no longer lists these variables when its deployment declares them.

| Declaration | Gives |
|---|---|
| `secrets.federation.signing_key`, `.ca_cert`, `.ca_key` | `FED_SIGNING_KEY`, `FED_CA_CERT`, `FED_CA_KEY`. Paths only. A relative path is inside the active goat context directory (`$GOAT_PROFILE_DIR`, else `~/.goat`, then `contexts/<context>/`, the context from `$GOAT_CONTEXT`, else `current_context` in its `config.toml`, else `default`). The age-encrypted CA key stays opt-in (`goat_federation_ca_key_from_age`) |

## Live reads

Nothing on a host is changed.

- **Ports.** `ops/ansible/peering-preflight.yml` runs `ss`, `wg show all
  listen-port`, `docker ps` and `docker inspect` on the edge and core groups,
  through the same inventory and AWS wrapper the peering plays use, and writes one
  JSON answer per host on the controller under `<work>/preflight/probe/` (the old
  answers are removed first). Python judges them. A port held by this peering's
  own WireGuard interface, or by a process whose container is this peering's edge
  router or republisher (directly, or as the container publishing the port), is a
  re-run, not a collision. A kernel socket that no WireGuard interface owns, any
  other process, and any other container publishing the port (with Docker's
  userland proxy off a published port has no socket for `ss` to show) is a
  collision. No answer for a host is GB-133, never a
  pass.

  The edge router's TCP port is judged by address and port, because several
  peerings share one edge and each router binds its own link address (apply
  renders `goat_zenoh_edge_bind_address` from the agreement). This router's
  address is `FED_EDGE_BIND_ADDRESS`; else apply's rendered value; else what the
  deployment declares for the edge group; else pending, until phase 9 renders the
  link address. A holder collides when it listens on the same address, or when
  either side is a wildcard (`0.0.0.0`, `::`, or a container publishing on every
  address). While the address is pending, only a wildcard holder collides, and
  another peering's router on its own link address is reported, not refused.
  Overlapping link subnets are the federation plan's host-level check. The tunnel's UDP port is still
  judged by port alone: a WireGuard interface listens on every address.
- **Core router.** The same selection as `goat federate init`
  ([federation-router-discovery.md](federation-router-discovery.md)), against the
  same daemon: `FED_ROUTER_DOCKER_HOST`, else an ambient `DOCKER_HOST`, else the
  local runtime; with a daemon named, Docker only, as phase 3. The contract has a
  Rust and a Python implementation; both are tested against
  `ops/federation/standup/fixtures/router-discovery.tsv`, so one cannot change
  without the other failing.
- **Receive stores.** `docker exec <router> cat /etc/zenoh/config.json5`
  (`FED_CORE_CONFIG_PATH` for another path), judged by the filter the edge role
  already uses ([federation-ingress-storage-preflight.md](federation-ingress-storage-preflight.md)).
  The required scopes are the rendered `goat_zenoh_edge_ingress_key_exprs` when
  apply has run, else the spec's wire name for the partner, `from/<partner>/**`.
  An unreadable configuration is SKIP with GB-137; the configuration is never
  printed.

  The store is required only once the core admits the peering. The goat-zenoh
  role renders the per-partner store `sovereign_from_<them>` (key `from/<them>/**`)
  only for a peering listed in `zenoh_sovereign_peerings`
  (`roles/goat-zenoh/templates/config.json5.j2`, the `{% for _p in
  zenoh_sovereign_peerings %}` storage block). The list is empty by default
  (`roles/goat-zenoh/defaults/main.yml`), and the runbook adds the peering when phase 10
  admits the edge (`docs/operations/sovereign-federation-runbook.md`, the
  phase table: "review what apply rendered; admit the edge on our core"). A new
  peering's store therefore cannot exist at phase 1. The check reads admission
  from the same configuration: the ACL subject `sovereign-<them>` (the role) or
  `fed-<them>` (apply's spliced fragment). Without it, NOT APPLICABLE ("not yet
  expected; created at phase 10"). With it (a re-run, or a deploy after phase
  10), a missing or too-wide store is a FAIL with GB-136. The plan's recheck,
  which the deploy runs before phases 11 to 13, reruns this check, so the store
  is enforced after admission. A core whose configuration lives elsewhere (a
  hand-run core) needs `FED_CORE_CONFIG_PATH`; without it the check is SKIP
  (GB-137), not a pass.
- **Slot list.** `ops/scripts/sync-slot-anchors.py --env <environment> --check`.
  Exit 1 is drift (GB-134); exit 2 is an unreadable registry (GB-135), which is
  not a drift verdict. NOT APPLICABLE when the environment has no
  `zenoh-lazy-storage.yml`.
- **CA.** `openssl x509` reads the certificate and its basicConstraints;
  `openssl pkey -pubout` compares the key's public half with the certificate's.

## The restart recheck

Slots minted between fixing the list and restarting the core router drift it
again. The step that restarts the core router calls the slot check once more,
immediately before the restart:

- Rust: `federate::preflight::recheck_slot_anchors(repo, peering, work)` returns a
  `SlotRecheck`; `allows_restart()` is true only for PASS and NOT APPLICABLE.
  It writes its own result file, `<work>/preflight/slot-recheck.json`, removed
  before each run, and refuses when the exit status and the result disagree, so a
  stale answer or the full phase-1 result never stands in for the recheck.
- Shell: `python3 ops/federation/standup/preflight.py <peering> --work <dir> --only slot-anchors --result <file>`
  (exit 0 or 1).

## Dry run

`goatbuilder federate <partner> --dry-run` lists the phase-1 checks under the
plan table. With `-o json` it adds one `preflight-check` row per check and one
`preflight-input` row per input, with its derived value and source, or the place
to declare it. A dry run exports nothing, so its phase commands stay the
script's, word for word (the parity gate `ops/goatbuilder/parity-federate.sh`).

## Reverting this

Reverting restores the earlier phase 1: the same static checks without codes,
and every input exported by hand again. No other phase reads the result file.

# onboard-runner role

Deploys the **onboard-runner** auto-executor (source `ops/onboard-runner/`) on the
site mgmt host — the daemon that makes the three-stage flow's *"admin approves → it just
happens"* real. It runs as a **container with its own scoped portal identity** (the
reshape from the earlier host-process-reusing-the-operator's-login shape). Design +
credential review:
[`docs/design/operator-onboard-intake.md`](../../../../docs/design/operator-onboard-intake.md)
§"Fast-follow".

## What it deploys

- the `onboard-runner` **container** (image `ghcr.io/usareur-af/onboard-runner`,
  digest-pinned in group_vars) on the shared `mgmt` Docker network, so it reaches
  the portal + Kratos **by container name** (no host mesh DNS)
- a compose stack at `/opt/onboard-runner/` (`docker-compose.yml` + `.env`, the
  `.env` `0600` with the mint credentials — never committed)
- a repo-root tree at `/var/lib/onboard-runner/repo` (the manifest schema + a
  writable `ops/operators/<env>/` the native mint reads/writes), bind-mounted rw and
  owned by the container's non-root uid
- the selected environment's affiliation catalog under `ops/identity/orgs/`
  when `onboard_runner_aff_trait_enabled` is true. Each convergence refreshes it
  from `onboard_runner_catalog_dir_src`, preferring JSON over YAML as the CLI
  does. Staging and Compose emission use the same boolean interpretation,
  including string-valued CLI overrides such as `false`. A missing catalog fails staging. Switching formats removes the obsolete
  file for that environment so an old JSON file cannot shadow updated YAML.
- the site `goat` binary is **bind-mounted** from the host `/usr/local/bin/goat`
  (role-managed, one version source) — NOT baked into the image

## Why a container (vs the earlier host process)

- **Own scoped identity** — no copy of a human operator's login on the mgmt host. The
  runner carries its own portal key (`onboard-runner-<site>:<key>`), independently
  revocable. It uses *only* a portal bearer token + the site NetBird token; the
  native onboard never touches the operator mTLS key.
- **Sidesteps the host-integration gaps** — a bridge container resolves the portal
  and Kratos by container name, so the mesh-DNS `/etc/hosts` hack and the systemd
  `ProtectHome` handling the host process needed are both gone.

## Default-off go-live gate

This is a **mint-anything capability**, so the role is safe by default: it renders
the compose + creds but leaves the container **down**. Nothing mints until you
re-run with `-e onboard_runner_enabled=true`. Toggling back to `false` brings the
stack **down**.

The role requires Ansible Core, `community.docker >=4.8.9`, Docker Engine
and the Docker Compose plugin. Install the repository's collection requirements
with `ansible-galaxy collection install -r ops/ansible/requirements.yml`.
The dedicated lifecycle CI job tests the collection floor with Ansible Core 2.20.11.

The lifecycle regression requires Linux, a local Docker daemon and a C compiler
that can build a static test binary. The production teardown task converges
when the disposable stack is absent, removes a running stack and stays
converged on a repeat. Separate stand-in Docker CLI cases check the task
result for an exit-zero Compose JSON warning and for the same warning combined
with an unrelated error. The fixture exercises the teardown task in isolation.

## Prerequisites

1. `goat >= 0.5.14` (the role default `onboard_runner_min_goat_version`; 0.5.14 is the first that writes groups to `metadata_public`) on the mgmt host at `onboard_runner_goat_bin_host` (native onboard +
   the `GOAT_ONBOARD_KRATOS_ADMIN_URL` override reaching Kratos by container name).
   **Auto-managed:** on every deploy the role syncs the host binary to whatever
   goat-get serves **on this host** — it reads `goat-get`'s on-disk served file
   (`onboard_runner_goat_served_dir`, default `/opt/goat-get/www`), verifies it
   against the sibling `.sha256`, installs atomically, and restarts the container
   on a swap — so a CLI-release redeploy keeps the runner current with **no second
   pin and no out-of-band install** (`onboard_runner_goat_autoupdate`, default
   true). It reads the on-disk file rather than fetching `get.<site>` over HTTP
   because the mgmt host **cannot resolve its own `get.<site>` hostname** (NXDOMAIN
   — the mgmt host's resolver has no mesh zone), so an HTTP pull would silently no-op on
   the very host that runs the runner. The role still **asserts** the floor version
   afterward, so a missing/too-old binary (e.g. goat-get not yet deployed on a
   fresh host) fails closed — bootstrap it once from the surface / goat-cli release.
   This auto-sync is why the `traits.vendor`→`org` migration can't strand the
   runner on a stale binary again.
2. `<SITE>_ONBOARD_RUNNER_PORTAL_KEY` + `<SITE>_NB_API_TOKEN` in the deploy env, AND
   the same runner key present in the portal's `PORTAL_OPERATOR_API_KEYS` as
   `onboard-runner-<site>:<key>`.

## Credential surface

- **Portal** (`ONBOARD_RUNNER_PORTAL_KEY`) — the runner's own operator bearer, used
  both for the claim/result contract AND (written to the goat profile's
  `portal.token` by the image entrypoint) the native Zenoh slot mint. Reaches the
  portal at `http://goat-zenoh-portal:8090` on the `mgmt` network.
- **NetBird** (`<SITE>_NB_API_TOKEN`) — the native setup-key mint, over the public
  mgmt API (internet-reachable, no mesh).
- **Ory** — `GOAT_ONBOARD_KRATOS_LOCAL=1` + `GOAT_ONBOARD_KRATOS_ADMIN_URL=`
  `http://mgmt-host-kratos-1:4434`, so the Ory leg runs `curl` in-container against
  the sibling Kratos admin API by name: **no SSH key needed**.

## Deploy

```
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ../scripts/ansible-preflight.sh -i inventory/mgmt-hosts-mesh.ini \
  onboard-runner-deploy.yml -e target=zagros_hosts
# then, to go live:
#   ... onboard-runner-deploy.yml -e target=zagros_hosts -e onboard_runner_enabled=true
```

Rollback: covered by the universal ledger (ADR 1060), `image`-class (re-pin), with
per-goatnet targetability in `deploy-history.yml`.

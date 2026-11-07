# goat-zenoh-portal role

Deploys the **vendor self-service portal + message inspector** for the
Zenoh trial fabric onto a single host. Per design at
`docs/design/zenoh-trial-fabric.md`
§"Vendor self-service portal + inspector."

## What it deploys

- A docker-compose stack with one container running the
  `ghcr.io/<ghcr_org>/goat-zenoh-portal:<tag>` image (built from
  `ops/goat-zenoh-portal/Dockerfile`).
- A persistent volume holding the SQLite registry + minted vendor
  tarballs.
- Bind-mounts for the **live** zenohd config-dir + the host docker
  socket + the per-vendor rocksdb storage parent dir. The portal
  applies marker-based incremental edits to the live `config.json5`
  on vendor approval, mkdirs the per-vendor storage dir, then SIGHUPs
  the zenohd container by POSTing to the Docker Engine API over the
  mounted socket. No shell-out (compatible with the Chainguard
  distroless image).
- A bind-mount of the operator's age-identity file (read-only) so the
  portal can shell out to `cert-sign --age-identity ...` for vendor
  mint, AND a bind-mount of the goatnet's CA bundle directory.
- A Traefik route (mesh-only; **no public DNS**) at
  `portal.<trial-zone>` per the operator's `site_dns_zone`.

## Variables (defaults in `defaults/main.yml`)

| Variable | Purpose | Default |
|---|---|---|
| `portal_image` | image ref | none: the goatnet's pin sets it |
| `portal_listen_port` | host port on the loopback | `8090` |
| `portal_data_dir` | base for SQLite + bundle tarballs | `/opt/goat-zenoh-portal` |
| `portal_zenoh_rest_url` | zenohd REST plugin URL | `http://zenohd:8000` |
| `portal_acl_config_path` | live zenohd `config.json5` the portal edits in place | `/etc/goat-zenoh/config.json5` |
| `portal_acl_container_name` | zenohd container — SIGHUP target | `goat-zenoh-router` |
| `portal_acl_docker_socket` | Docker Engine API socket (mounted into portal) | `/var/run/docker.sock` |
| `portal_acl_storage_dir_prefix` | per-vendor rocksdb parent dir | `/var/lib/goat-zenoh/rocksdb` |
| `portal_acl_reload_fake` | skip SIGHUP step (marker edits still run) | `false` |
| `portal_ca_bundle_dir` | mounted ro into container | `/etc/<site_id>-ca` |
| `portal_age_identity_path` | mounted ro into container | `/etc/<site_id>-age/keys.txt` |
| `portal_ca_root_cert` | path inside container | `/etc/<site_id>-ca/root-cert.pem` |
| `portal_onboarding_packet_path` | mounted ro inside container | `/etc/goat-zenoh-portal/zenoh-trial-onboarding.md` |
| `portal_operator_api_key` | env-supplied; required | (env: `PORTAL_OPERATOR_API_KEY`) |
| `portal_operator_approval_required` | flips first-cohort vendor approval | `true` |
| `portal_traefik_host` | Traefik router rule | `portal.{{ site_dns_zone }}` |
| `portal_sunset_date` | shown in UI banner | `""` |
| `portal_release_sets` | this goatnet's release sets, `[{set, prefix, members}]` in the shape of `ops/permissions/release-sets.yaml`; declared in the goatnet's `group_vars/<site>_hosts/data-access.yml`. Non-empty: the role renders the portal's registry from it (`templates/release-sets.yaml.j2`). Empty: the role copies the generic `ops/permissions/release-sets.yaml` | `[]` |
| `portal_release_sets_path` | where the registry lands on the host; `""` disables the Data-Access release reveal | `{{ portal_data_dir }}/release-sets.yaml` |

## Prerequisites

- Docker + docker-compose-plugin on the host (the existing mgmt-host
  baseline covers this).
- The goat-zenoh role's zenohd container running on the same host
  so `portal_acl_config_path` (the same live `config.json5` zenohd reads)
  is bind-mounted into the portal AND the SIGHUP target is reachable
  via the host docker socket.
- An age-identity file at `portal_age_identity_path` containing the
  operator's age private key — used by `cert-sign` to decrypt the
  goatnet's CA key for per-vendor leaf issuance.
- A `cert-sign` binary on the host PATH **inside the container**.
  Achieved by either (a) using the goat-zenoh-portal image that already
  bundles `cert-sign`, or (b) bind-mounting a host-side `cert-sign`
  into `/usr/local/bin/cert-sign` of the container. Defaults assume (b).
  The role builds and installs it from this checkout when it is missing.
- The Subject O of minted leaves is `portal_cert_sign_org`, defaulting to
  `goat_cert_organization` (the same fact the operator plays pass to
  cert-sign). It reaches the portal as `PORTAL_CERT_SIGN_ORG` and the portal
  passes `cert-sign --org`; empty omits the flag and cert-sign's built-in
  default applies. When the pinned image reads the setting and the host
  cert-sign predates `--org`, the role rebuilds cert-sign from this checkout
  and restarts the portal.
- (Optional) A `bundle-create` binary on the host, same placement pattern
  as `cert-sign`, to mint the goat-cli `.cbor` into the vendor tarball.
  OFF by default (`portal_bundle_create_bin: ""`). To enable: build
  `bundle-create` **with `-tags nopiv`** — that drops the piv-go/YubiKey
  dependency (prod mode is unavailable; the portal only ever mints in
  age/dev mode), so it builds CGO-free and fully static, cross-compilable
  from any host and runnable in the distroless portal container:

      cd ops/enrollment
      CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build -tags nopiv \
        -trimpath -ldflags '-s -w' -o bundle-create ./cmd/bundle-create

  Place the binary on the host (e.g. `/usr/local/bin/bundle-create`,
  `chmod 0755`), then set `portal_bundle_create_bin` to that path in
  group_vars and redeploy. Setting the path before the binary exists fails
  the bind-mount.

## What this role does NOT do

- Deploy `zenohd` itself (Track A / `roles/zenoh/`).
- Mint the goatnet's CA root or seed the age identity (an operator
  ceremony).
- Provision a public DNS record. Trial fabric is mesh-only.

## Tests

`tests/syntax-check.sh` runs `ansible-playbook --syntax-check` against
a parse-only fixture. Behavioral coverage rides the portal's own Go
test suite (`go test ./...` in `ops/goat-zenoh-portal/`).

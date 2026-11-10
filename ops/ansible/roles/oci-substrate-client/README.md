# oci-substrate-client

Client-side wiring for the OCI image distribution substrate per
ADR 0838
+ design doc §5.

Applied to every consumer host that pulls images for compose stacks
(today: mgmt-host pair Orin-A/B at multi-Orin sites; single-Orin sites
get a T1-only variant via empty `oci_substrate_t2_endpoint`).

## What it does

1. Reads existing `/etc/docker/daemon.json` and merges (does not replace).
2. Adds the per-site Orin-C Forgejo endpoint to `insecure-registries`
   (Forgejo today is HTTP-only on `:3030`; HTTPS lands when the internal
   X.509 CA cert covers Forgejo's port).
3. Optionally adds the per-site Orin-C `docker.io` pull-through cache to
   `registry-mirrors`.
4. Triggers `systemctl restart docker` via handler — bounces all running
   containers; schedule role applies during planned maintenance windows.

## What it does NOT do (today)

- **Multi-upstream client-side mirror redirection.** Docker daemon's
  `registry-mirrors` only handles `docker.io` upstream. For `cgr.dev`,
  `nvcr.io`, etc., either the compose `*_IMAGE` env-var override
  pattern is used (today's interim path) or containerd hosts.toml
  takes over (Block 16 Quadlet trigger per [ADR 0207](../../../../docs/adr/0207-docker-compose-orchestration.md)).
- **Containerd hosts.toml.** Variable `oci_substrate_enable_containerd_hosts_toml` is reserved for future use; defaults off.
- **Cosign trust-anchor delivery.** Lands with first cosign-verify gate at consumer pull-time (Block 75G).
- **R2 / ECR credentials.** Lands with T1 wiring (deferred from today's scope).

## Variables

| Variable | Default | Purpose |
|---|---|---|
| `oci_substrate_t2_endpoint` | `""` | Per-site Orin-C Forgejo `host:port` (e.g. `192.168.1.253:3030`). Empty = single-Orin-site no-op. |
| `oci_substrate_t2_insecure_http` | `true` | Add T2 endpoint to docker daemon `insecure-registries` list. |
| `oci_substrate_t2_mirrors` | `{docker_io: 5000, ghcr_io: 5001, ...}` | Per-upstream Orin-C pull-through ports (informational; only `docker_io` is consumed by docker daemon today). |
| `oci_substrate_enable_dockerio_mirror` | `false` | Add Orin-C's `docker.io` pull-through to `registry-mirrors`. |
| `oci_substrate_enable_containerd_hosts_toml` | `false` | Drop containerd `hosts.toml` (Block 16 Quadlet trigger). |

## Per-site config example (group_vars/<site>_hosts/vars.yml)

```yaml
oci_substrate_t2_endpoint: "192.168.1.253:3030"   # Orin-C LAN address       
oci_substrate_t2_insecure_http: true              # Forgejo HTTP-only on :3030
oci_substrate_enable_dockerio_mirror: false       # not yet wired
```

## Idempotency

Role is fully idempotent. Re-running with no group_vars change is a
no-op. Re-running after a config change re-renders `daemon.json`,
backing up the previous version (to `daemon.json.<timestamp>~`), and
restarting docker only when the daemon.json content actually changed.

## When this role applies vs Block 71's docker-registry-mirror role

| | Block 71 `docker-registry-mirror` | Block 75 `oci-substrate-client` |
|---|---|---|
| Role of host | Server (Orin-C) — runs registry:2 instances | Client (mgmt-hosts, single-Orin sites) — pulls from Orin-C / T1 |
| Configures | `registry:2` containers per upstream | `daemon.json` insecure-registries + registry-mirrors |
| Applied to | `[*_obs_hosts]` inventory groups | `[*_hosts]` inventory groups (mgmt) |

The two roles cooperate: Block 71's server-side runs on Orin-C; this
role's client-side runs on every consumer.

## Cross-refs

- ADR 0838
- Design doc §5
- Block 75 implementation plan
- [Block 71 — docker-registry-mirror server-side role](../docker-registry-mirror/)
- [ADR 0207 — orchestrator decision (Compose now, Quadlet later)](../../../../docs/adr/0207-docker-compose-orchestration.md)

# goat-logship

Vector-based cyber-logging collector. Normalizes host/OS + fabric security
events to **OCSF** at the edge and ships them **off the host** to an
Object-Lock S3 bucket, retaining near-zero locally. Absence-alarmed so a
silent stream pages instead of vanishing.

Anchor: ADR 1078 ·
design `cyber-logging-and-conmon-export.md`.

## What it does

- **Sources** (§4): `journald` (always), `auditd`, the existing Zenoh-audit
  NDJSON, container stdout, egress-proxy logs — each opt-in via a toggle
  because on-disk paths are host-specific.
- **Normalize** (VRL): every event → an OCSF envelope stamped with a
  `metadata.dictionary_id` from [`dictionary.yaml`](../../../../ops/logging/goat-logship/dictionary.yaml).
  Edge redaction strips secret-shaped tokens.
- **Sink 1 — the durable landing (pluggable, `logship_durable_sink_mode`):**
  - `mock` (default) — a local newline-delimited OCSF/JSON tail
    (`{{ logship_mock_sink_dir }}/ocsf-*.json`). Stores nothing off-box;
    proves collect→normalize→ship with zero cloud dependency.
  - `s3` — `aws_s3` to a dedicated Object-Lock bucket, OCSF/JSON
    partitioned `date/host/class`, disk-buffered. A one-line flip (the sink
    component is always `durable`, so metrics + absence-alarm are unchanged).
- **Sink 2 — external-collector egress (default-off):** CEF-over-syslog-5424-TLS,
  rendered only at the boundary, enabled only against a named collector.
- **Absence-alarm:** `internal_metrics` → `prometheus_exporter`; the rules in
  `prom-rules-goat-logship.yml.j2` fire on stream silence (closes F-169).

## Privilege and exposure (GHSA-x44g)

- **Not root.** Vector runs as the host system user `logship_user`
  (`goat-logship`), with no capabilities and `no-new-privileges`. It reads
  each log path through that path's owning group (added with `group_add`):
  the journal's `systemd-journal`, and the audit log's group. auditd writes
  `audit.log` root-only unless `auditd.conf` names a `log_group`, so the role
  sets `log_group = adm` and regroups the current log (0640) and its
  directory (0750); `logship_auditd_manage_log_group: false` turns that off.
  A path an enabled source reads that only root can read stops the play.
- **No Docker socket.** The admit-audit, Traefik and oauth2-proxy sources
  read container stdout through `goat-logship-docker-proxy`, an nginx
  container (the repo's pinned `nginx:1.27-alpine`) that alone mounts the
  socket and passes on only GET/HEAD for ping, version, events, container
  list, inspect and logs (`templates/docker-proxy.nginx.conf.j2`). It has no
  network: it listens on a unix socket in `logship_docker_proxy_socket_dir`,
  a directory owned by its own user (`goat-logship-dproxy`) and the
  collector's group, mode 0710, with the socket 0660, so no other host user
  and no other container can connect (a TCP listener on a Docker network,
  even an internal one, is reachable from every process on the host). Vector
  mounts that directory read-only. The proxy is not deployed when those three
  sources are off. The json-file logs under `/var/lib/docker/containers` are
  not used instead: they are named by container id, not name, and Docker
  makes each container's directory root-only.
- **Metrics port.** 9598 publishes on loopback, the mesh address
  (`logship_mesh_iface`, default `wt0`; the play stops if it has no IPv4)
  and the default Docker bridge gateway (the co-located goat-obs Prometheus
  scrapes `host.docker.internal:9598`; `logship_metrics_bind_docker_gateway`).
  `goat-logship-expose-guard.service` loads an nftables table that drops
  traffic to the mesh address that does not arrive on the mesh interface,
  and traffic to the gateway address that does not arrive on the bridge of
  `logship_metrics_scraper_network` (default `mgmt`, the goat-obs
  Prometheus's network) or loopback. Without that network the gateway
  binding is left out. `logship_bind_all: true` restores publishing on every
  interface and removes the guard.
- **Data dir.** The collector can write its data dir, so the root-run tasks
  on it never follow a link (`follow: false`), and walk the tree only once,
  to hand over a dir a root-run collector wrote (`tasks/datadir.yml`).

Residual: the proxy's inspect endpoint returns other containers'
environment, and its logs endpoint serves any container's output, so a
compromised collector can still read those; it can no longer create, start,
exec into or change anything. Other containers on the goat-obs Prometheus's
network (`mgmt`) can also reach the gateway binding.

## Deploy

Default (`mock`) needs no credentials — deploy straight to a host:
`ansible-playbook goat-logship-deploy.yml -e target=zagros_hosts`.

For `s3` mode: set `logship_durable_sink_mode=s3`, apply the bucket's Terraform,
and export `LOGSHIP_S3_ACCESS_KEY_ID` / `LOGSHIP_S3_SECRET_ACCESS_KEY`
(ingest-only key; never committed).

Wire per goatnet by adding the role to the
site play with the source toggles the host actually has. Removable by
pin-revert with no effect on the audit/metrics planes (ADR 1078 §10).

## Confirm at standup (per ADR 1078 §9)

- Exact S3 bucket name (see the role defaults).
- The Zenoh-audit NDJSON on-disk glob for this host (`logship_zenoh_audit_glob`,
  off until confirmed).
- external collector endpoint (gates enabling Sink 2 beyond default-off).

# goat-relay Ansible role

Deploys the [goat-relay](../../../../goat-relay/) Block 33B Rust relay-
server data-plane on a Linux host (Ubuntu 22.04 / 24.04, Debian 12,
RHEL 9 — anywhere with a recent kernel + docker).

## Operational invariants encoded

This role is the canonical deploy automation that encodes two paired
invariants surfaced during Block 33B step 8 tier 4a:

1. **F-023** — `/etc/modules-load.d/goat-relay-tls.conf` containing
   `tls`, plus `modprobe tls` now. Without this the cfg(linux) kTLS
   branch falls back to userspace TLS silently.
2. **F-027** — docker container runs with `network_mode: host`. With
   bridged networking the container's per-netns
   `/proc/sys/net/ipv4/tcp_available_ulp` doesn't inherit host's
   modprobe'd modules; F-023 runtime probe correctly falls back to
   userspace silently.

**Both required; neither sufficient alone.** Either falls back to
userspace TLS without operator-visible failure — the relay accepts
traffic, the bytes flow, only the per-packet AEAD bypass benefit
(ADR 0709 D8) is lost.

## Variables

See [`defaults/main.yml`](defaults/main.yml). Common overrides:

```yaml
goat_relay_image: ghcr.io/usareur-af/goat-relay:v0.1.0
goat_relay_bind_addr: "0.0.0.0:33073"
goat_relay_cert_host_path: /etc/goat-relay/cert.pem
goat_relay_key_host_path: /etc/goat-relay/key.pem
goat_relay_hc_attempt_threshold: 1
```

## Pre-requisites

The role assumes:

- Docker installed + running on the host.
- Ansible's `community.docker` + `community.general` collections
  available (the project-wide `requirements.yml` already includes
  these).
- TLS cert + key pre-staged at `goat_relay_cert_host_path` /
  `goat_relay_key_host_path`. Production: real CA-signed delivered
  out-of-band. First-light: self-signed via `openssl req -x509 ...`
  (the role does NOT generate certs — security-relevant; out of
  scope).

## Usage

```yaml
# play.yml
- hosts: goat_relays
  become: true
  roles:
    - role: goat-relay
      vars:
        goat_relay_image: ghcr.io/usareur-af/goat-relay:v0.1.0
```

```bash
ansible-playbook -i inventory/hosts.ini play.yml
```

## What this role does NOT do

- Generate TLS certs (operator responsibility).
- Configure firewalls / security groups (handled by
  `terraform/modules/relay/` or equivalent — the SG must open TCP+UDP
  33073 inbound).
- Wire goat-relay into the netbird-mgmt relay registry (operator
  task; `--advertised-addr` flag will land in a later step).
- Tier 4b throughput validation (full netbird daemon + iperf3 ramp;
  that's 33D scope per
  `docs/operations/goat-relay-bench-env-smoke.md`).

## Verification

After running, the role asserts:

1. The `tls` ULP module is registered on the host.
2. The container is started with `network_mode: host`.
3. Container startup logs are captured (the F-023 probe verdict logs
   on first inbound connection; absence of an inbound connection
   yet is fine — the role's success means the deploy is set up to
   fire kTLS once traffic arrives).

End-to-end production validation (real netbird daemons connecting
through this relay + iperf3 throughput against the M-006-tuned
≥110 Mbps baseline) is Block 33D scope.

## Related

- [ADR 0709](../../../../docs/adr/0709-block-33-relay-rewrite-architecture.md)
  — D7-D13 architectural decisions; D8 is the kTLS hand-off this
  role makes operational.
- F-023
  + F-027
  — the empirical findings the role's invariants encode.
- M-009
  — the measurement that surfaced both invariants.
- [`docs/operations/goat-relay-bench-env-smoke.md`](../../../../docs/operations/goat-relay-bench-env-smoke.md)
  — the runbook that covers tier-1 → tier-4b validation steps.

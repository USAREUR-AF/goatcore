# mgmt-host compose

Docker Compose for the per-site Netbird management plane (Spock PG +
federated signal server + Netbird management + Traefik + dnsmasq + rathole).
Runs on any `mgmt-host`.

Design context: `docs/design/ha-multi-site.md`
§4 (per-site stack), §13 (phases).

## Status

Scaffold (2026-04-19). This compose file currently brings up just
`postgres-a` and `signal-a` — enough to validate the Spock image boots with
the baked config and to have a signal server listening. Netbird management,
Traefik, dnsmasq, and rathole land in separate PRs as Phase 1 proceeds.
Phase 2 adds a second instance of each via a `docker-compose.phase2.yml`
override so a single host exercises multi-master Spock + signal federation.

## Layout

```
ops/compose/mgmt-host/
├── docker-compose.yml   base stack (single instance of each service)
├── .env.example         template — Ansible renders the real .env from env vars
└── README.md
```

## Bring-up (on a provisioned mgmt-host)

```
cd /opt/mgmt-host
cp .env.example .env      # Ansible does this from env vars in production
# edit .env with the site-specific values
docker compose pull
docker compose up -d
docker compose ps
docker compose logs -f signal-a
```

Smoke-test the Spock image is running correctly:

```
docker compose exec postgres-a psql -U postgres -tAXc "SHOW wal_level;"
# expected: logical

docker compose exec postgres-a psql -U postgres -d netbird -tAXc \
    "SELECT extname FROM pg_extension WHERE extname='spock';"
# expected: spock
```

## Phase 2 (two instances on one pilot)

Once the single-instance stack is proven, Phase 2 adds a second instance of
each service on the same host via a compose override and sets up the Spock
subscription between PG-A and PG-B.

```
docker compose -f docker-compose.yml -f docker-compose.phase2.yml up -d
```

(That override file is TBD — tracked in
`docs/design/ha-multi-site.md` §13
Phase 2.)

## Notes

- Ports bind to `127.0.0.1` only. External exposure happens via Traefik
  (TLS, Let's Encrypt DNS-01 via Cloudflare) in a later PR.
- All container state lives under `/opt/docker-data/` (NVMe on production
  Orins — mount provisioned by Ansible / cloud-init per
  [`../../../docs/design/orin-os-decision.md`](../../../docs/design/orin-os-decision.md)).
  Docker's data root is pointed at that path; volumes land under it.
- Secrets (PG passwords, Netbird setup key, replicator credentials) are
  NOT in this compose file or .env beyond placeholders. Ansible renders
  them into `/opt/secrets/` at deploy time; LUKS (key custody via OP-TEE
  on Orin / TPM on x86) protects that mount.

## Rollout notes: mesh-only allowlist and Traefik's publish (GHSA-p22f)

- **The allowlist admits the `mgmt` network's own subnet, not 172.16.0.0/12.**
  `ops/ansible/tasks/mgmt-network-subnet.yml` reads it from the live network
  (`docker network inspect mgmt`) before traefik-acme.yml, lan-traefik-routes.yml
  and the icam-mtls-proxy role render their allowlists, and fails if the network
  does not exist yet. Bring the stack up first, or set
  `mgmt_docker_network_subnet` in group_vars.
- **Do not add `ipam` to the `mgmt` network in docker-compose.yml.** Any ipam
  change, even to the network's current subnet, makes Compose recreate the
  network on the next `up`. With other projects attached as `external` the
  removal fails ("active endpoints") and the base containers stay stopped.
- **A host process calling the host's own LAN address on :443 is refused.**
  Docker DNATs that connection with the host's address, which is not in the
  mgmt subnet. Under the old /12 it passed only on a host whose LAN was itself
  inside 172.16.0.0/12 (an AWS default VPC). Call 127.0.0.1 or the mesh address.
- **Detection gap: Docker's DNAT rule for :443.** The live rows
  `mgmt-websecure-listens-on-ipv4` and `mgmt-websecure-has-no-ipv6-listener`
  read the listener, not the path. If the `nat` table's DOCKER DNAT rule for
  :443 is missing (for example after an nftables `flush ruleset` on an
  iptables-nft host), docker-proxy on `0.0.0.0:443` accepts off-host IPv4
  clients and Traefik sees every one of them as the mgmt gateway, which the
  allowlist admits. Both rows stay green. No default mgmt-host role flushes the
  table today. After any firewall change, check:
  `sudo iptables -t nat -S DOCKER | grep -- '--dport 443 '` must print one
  DNAT rule to Traefik. An off-host probe that expects 403 on a protected host
  would close the gap; it is not written yet.
- **Relay configs already on a host keep their old target until re-rendered.**
  `/etc/rathole/client-*.toml` written before this change point at
  `127.0.0.1:443`. Re-run the rathole client play so they point at
  `127.0.0.1:8443`.

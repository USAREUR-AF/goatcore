# Federation end to end

Two sovereigns, `pasture` and `meadow`, federate through the real driver and are proved
from the receiving side. The same scenario runs in two places.

| Script | Where | What it proves | Record |
|---|---|---|---|
| `run.sh` | CI job `federation-e2e`, four privileged containers on one runner | Every phase of both avenues, both orders | M-124 |
| `live.sh` | Two real Ubuntu hosts, one per sovereign, driven from a laptop over SSH | The same phases and proofs across a real network | M-125 |

`env` is the environment around them: several sovereigns at once, any goatcore ref as the
tree under test, a scenario catalog with outcome words, and a watcher for branch heads
([docs/design/federation-e2e-environment.md](../../docs/design/federation-e2e-environment.md)).

```text
ops/federation-e2e/env list
ops/federation-e2e/env run --ref origin/main --avenue goatbuilder --scenario two-party-full
ops/federation-e2e/env run --ref <ref> --avenue both --scenario all --jobs 3 --out <dir>
ops/federation-e2e/env down --lab <n>
```

Both need only the checkout: the SPIRE image is the `spire` component of
`ops/pins/fleet-pins.yaml`, and `overlay.py` writes each sovereign's whole
`ops/goatbuilder/goatbuilder.yaml`, inventory and edge `group_vars` into its clone.
`run.sh --prepare-only --goatbuilder <bin>` stops before the first container;
`ops/ci/check-federation-e2e-prepare.sh` runs it on a copy with every
`group_vars/*/pins.yml`, the checkout's `goatbuilder.yaml` and the committed intents
removed and no goatnet in `fleet-pins.yaml`, as a public checkout has.

Both run phase 14, `ops/federation/standup/acceptance.sh`, from both sides with
`FED_ACCEPTANCE_STRICT=1`. A skipped check fails the run.

## live.sh

```text
ops/federation-e2e/live.sh --dry-run --pasture-host <ip> --meadow-host <ip>
ops/federation-e2e/live.sh --goat <bin> --goatbuilder <bin> --pasture-host <ip> --meadow-host <ip> \
  [--pasture-key <key>] [--meadow-key <key>] [--record <dir>]
```

The binaries are the `federation-live-binaries` artifact of a green `federation-e2e` run,
so the live run ships the commit CI measured. `live.sh` refuses uncommitted changes under
`ops/` because it ships `git archive HEAD ops`.

`live-host.sh` is the host half: `live.sh` copies it to `/opt/fed-live/` and calls one verb
at a time. It installs everything below except the first four rows, and every verb can be
re-run. Each scenario starts with `reset`, so it begins with new CAs, new keys and a new
agreement. `--keep` leaves the hosts as the last scenario left them.

### One host per sovereign departs from the runbook

The runbook keeps the edge off the core's machine. `live.sh` puts both on one host per
sovereign, because the drill provides one host per sovereign. The core router listens on
a dummy-interface address (`10.77.1.10` and `10.77.2.10`), its REST on loopback, and the
SPIRE bundle endpoint on the same host-local address. The edge reaches the core through
the host. A real deployment must not accept that an edge compromise is a core compromise.
The run keeps three things: WireGuard between the public addresses is the only path between
the hosts, nothing core-side is reachable off-host, and the bundle endpoint crosses inside
the tunnel. The run checks the second of these before any phase runs.

### What a real host must provide

This is the drill's checklist. Each row is something the container run gets for free.

| Requirement | Why | Container run | Real host |
|---|---|---|---|
| Ubuntu 24.04, x86_64 | The binaries are built on the CI runner's Ubuntu 24.04 and need its glibc | 24.04 image | The partnerenv roots default to `noble` 24.04; do not set `jammy` |
| The `wireguard` kernel module | `goat-peering-wg` creates a kernel interface | The runner's module, mounted | In the stock Ubuntu kernel; `prereqs` runs `modprobe wireguard` and refuses without it |
| Inbound UDP 51820 from the other host's public address | The only inter-host path | One bridge, nothing filtered | A security-group rule on both hosts, below |
| SSH from the laptop with passwordless sudo | Every verb runs as root | `docker exec` | The `ubuntu` user on the AWS image; SSH admitted from the laptop's /32 |
| A token that can read `ghcr.io/usareur-af/zenoh-pqc` | The image is private; `live.sh` logs each host in with it over SSH stdin and logs out at the end | The runner is logged in | `--ghcr-token-env NAME`, default `GHCR_READ_TOKEN`; user `--ghcr-user`, default `x-access-token` |
| Outbound to Ubuntu apt, PyPI, Ansible Galaxy, `ghcr.io` and Docker Hub | Packages, `ansible-core` in a venv, collections, both images by digest | Pre-built image | Default egress |
| Docker, compose v2, `wireguard-tools`, `nftables`, `python3-venv`, git, openssl, curl, jq | The plays and the driver | Pre-built image | `prereqs` installs them from apt |
| Ansible on the edge host, local connection | The driver runs where the fabric is, and the plays target this host | SSH to `127.0.0.1` | `overlay.py --local`: `ansible_connection=local`; the driver runs as root |
| The partner's public address is literal | `goat-peering-wg` filters the tunnel port on it | Literal | Pass the public IPs, not names |
| Ports 7447, 8000, 8081, 8443, 18443 and 51820 free | The core, its REST, SPIRE, the two bundle forwards, the tunnel | Fresh container | Fresh instance |

The SPIRE server binds its agent API on `0.0.0.0:8081` (`server.conf.j2`). Only the
security group keeps that off the internet on a real host.

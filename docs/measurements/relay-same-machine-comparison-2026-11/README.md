# goat-relay and the NetBird Go relay on the same machines (November 2026)

The two relays, measured in turn on the same three machines, with the same clients and
the same kernel. Issue: #949. Requested on
[netbirdio/netbird discussion 8624](https://github.com/netbirdio/netbird/discussions/8624).

## Result

| Transport | Throughput | Relay CPU per Gbit/s |
|---|---|---|
| WebSocket over TLS | The Go relay levels off at about 850 Mbit/s with the stock client, whatever the number of flows. goat-relay reaches 1.2 to 1.3 Gbit/s with 8 or 16 flows, but a single flow gets only 220 to 500 Mbit/s and varies from run to run. | goat-relay 0.9 (kernel TLS) to 1.1 (TLS in user space) cores; Go relay 2.2 to 2.4 cores |
| QUIC | Both relays carry 1.05 to 1.15 Gbit/s with 4 or more flows; goat-relay about 5% more. | goat-relay 0.9 to 1.0 cores; Go relay 3.1 to 3.4 cores |

- Most of the CPU difference is not kernel TLS: goat-relay with TLS in user space uses
  about 1.1 cores per Gbit/s against 0.9 with kernel TLS, and the Go relay 2.2 to 2.4.
- The client socket buffer change from netbirdio/netbird#6774 raises single-flow
  throughput through either relay (Go relay over WebSocket: 828 to 965 Mbit/s) and
  changes little at 8 or 16 flows.
- The Go relay built from NetBird `main` at `983b8258a` (24 Nov, includes
  netbirdio/netbird#8679, a send queue per destination peer) carried about the same as
  v0.83.1, a little more over WebSocket at 8 flows, and used 6 to 8% more CPU per Gbit/s.
  Two runs per cell are too few to call either difference.
- goat-relay's slow, variable single flow over WebSocket is a goat-relay defect to
  investigate; it is not present over QUIC.

## Setup

| Item | Value |
|---|---|
| Machines | Relay and two peers: AWS `c7i.2xlarge` (8 vCPU), one cluster placement group, one availability zone. Management server on a `t3.medium` outside the group. Ubuntu 24.04, kernel 7.0.0-1014-aws on all. |
| Path | `iperf3` from peer A to peer B through the NetBird tunnel, forced through the relay (`NB_FORCE_RELAY=true`), one transport at a time (`NB_RELAY_TRANSPORT=ws` or `quic`), lazy connections off. Each run checked `connectionType: Relayed`. |
| Clients | NetBird 0.83.1 (`stock`), and 0.83.1 with netbirdio/netbird#6774 (`pr6774`). |
| Management | NetBird combined server 0.83.1, SQLite, one fresh account per pass. |
| Go relay | NetBird relay 0.83.1 (pass 1 and 2); NetBird `main` at `983b8258a` (pass 2). TLS in user space. |
| goat-relay | 0.1.0 as built on 16 Nov 2026 from goatcore `main` (the build did not record a commit). kTLS when the kernel `tls` module is loaded (`ktls`); with the module blocked it uses TLS in user space (`user`). The relay logs which one it uses at start. |
| Relay CPU | `scripts/sampler.py` reads the relay process's CPU time and resident memory once a second; CPU is averaged over each run, excluding the first 3 seconds. |

## Pass 1, 16 Nov 2026: throughput, 1 to 16 flows

30-second `iperf3` runs; Mbit/s received at peer B, median of 3 runs, with the range in
brackets. Runs were interleaved across relays. The Go relay offers QUIC and WebSocket on
one port and always uses TLS in user space.

**WebSocket over TLS, stock client**

| Flows | goat-relay, kTLS | goat-relay, user-space TLS | Go relay |
|---|---|---|---|
| 1 | 430 (233–494) | 400 (280–555) | 828 (806–837) |
| 4 | 1010 (607–1120) | 948 (769–1040) | 855 (826–856) |
| 8 | 1210 (938–1240) | 1080 (1060–1150) | 845 (817–851) |
| 16 | 1290 (1140–1290) | 1200 (1140–1220) | 847 (816–868) |

**WebSocket over TLS, client with #6774**

| Flows | goat-relay, kTLS | goat-relay, user-space TLS | Go relay |
|---|---|---|---|
| 1 | 721 (387–745) | 656 (317–999) | 965 (910–989) |
| 4 | 1090 (992–1160) | 1050 (927–1130) | 977 (934–997) |
| 8 | 1230 (1210–1260) | 1190 (1070–1220) | 987 (930–995) |
| 16 | 1290 (1270–1310) | 1270 (1070–1290) | 984 (925–1010) |

**QUIC, stock client**

| Flows | goat-relay | Go relay |
|---|---|---|
| 1 | 932 (732–1040) | 863 (856–886) |
| 4 | 1110 (981–1190) | 1050 (1010–1060) |
| 8 | 1140 (1070–1220) | 1080 (1040–1090) |
| 16 | 1150 (1110–1260) | 1090 (1050–1110) |

**QUIC, client with #6774**

| Flows | goat-relay | Go relay |
|---|---|---|
| 1 | 984 (763–1070) | 966 (942–984) |
| 4 | 1130 (1030–1210) | 1060 (1030–1090) |
| 8 | 1165 (1100–1240) | 1090 (1050–1120) |
| 16 | 1145 (334–1210) | 1110 (1060–1130) |

One goat-relay QUIC run at 16 flows carried 334 Mbit/s; the two others carried 1140
and 1210.

## Pass 2, 25 Nov 2026: relay CPU and memory, and NetBird `main`

Same machines, stock client, 20-second runs after a 2-second omit, 2 runs per cell.
Mbit/s and relay CPU in cores (1.00 = one vCPU busy) for each run; peak resident memory.

| Transport | Relay | Flows | Mbit/s | Relay cores | Cores per Gbit/s | Peak memory |
|---|---|---|---|---|---|---|
| WebSocket | goat-relay, kTLS | 1 | 452 / 224 | 0.44 / 0.18 | 0.91 | 6 MiB |
| WebSocket | goat-relay, kTLS | 8 | 1008 / 847 | 0.99 / 0.62 | 0.87 | 6 MiB |
| WebSocket | goat-relay, user-space TLS | 1 | 297 / 334 | 0.31 / 0.34 | 1.03 | 6 MiB |
| WebSocket | goat-relay, user-space TLS | 8 | 975 / 872 | 1.02 / 1.01 | 1.10 | 6 MiB |
| WebSocket | Go relay 0.83.1 | 1 | 787 / 838 | 1.76 / 1.78 | 2.18 | 26 MiB |
| WebSocket | Go relay 0.83.1 | 8 | 841 / 890 | 1.94 / 1.96 | 2.25 | 26 MiB |
| WebSocket | Go relay `main` | 1 | 813 / 872 | 2.00 / 1.96 | 2.35 | 26 MiB |
| WebSocket | Go relay `main` | 8 | 898 / 1053 | 2.30 / 2.36 | 2.39 | 27 MiB |
| QUIC | goat-relay | 1 | 892 / 549 | 0.96 / 0.44 | 0.97 | 7 MiB |
| QUIC | goat-relay | 8 | 1042 / 855 | 1.09 / 0.60 | 0.89 | 7 MiB |
| QUIC | Go relay 0.83.1 | 1 | 802 / 805 | 2.56 / 2.56 | 3.19 | 28 MiB |
| QUIC | Go relay 0.83.1 | 8 | 968 / 940 | 2.99 / 2.97 | 3.12 | 29 MiB |
| QUIC | Go relay `main` | 1 | 798 / 802 | 2.74 / 2.73 | 3.42 | 29 MiB |
| QUIC | Go relay `main` | 8 | 961 / 963 | 3.26 / 3.24 | 3.38 | 30 MiB |

## Not covered yet

Issue #949 asks for more than these two passes give:

| Asked for | State |
|---|---|
| At least two fresh placements | One placement. |
| Relay CPU for every run, at 1, 4, 8 and 16 flows | Pass 2 only, at 1 and 8 flows. |
| Relay memory at idle and under 16 flows | Under 1 and 8 flows only. |
| `goat-relay` at a named tag or commit | Version 0.1.0; the build did not record its commit. |

## Rerun

`scripts/` holds what ran on the machines, as root, from `/opt/rcmp/`:
`control-ctl.sh` (management server and setup key), `relay-ctl.sh` (one relay at a time;
`ktls` or `user` loads or blocks the kernel `tls` module), `peer-ctl.sh` (one NetBird
client, forced through the relay on one transport) and `sampler.py` (relay CPU and
memory). `pass2-drive.sh` ran pass 2 from a workstation over SSH. The binaries are
expected in `/opt/rcmp/bin/`: `goat-relay`, `netbird-relay`, `netbird-relay-main`,
`netbird-stock`, `netbird-pr6774` and `netbird-server`.

## Data

- `data/pass1-runs.csv`: every pass 1 run, with its relay, TLS mode, client, transport,
  flows and Mbit/s.
- `data/pass1-iperf3-server.log`: peer B's `iperf3` server output for all 124 pass 1
  runs, in run order.
- `data/pass2/`: per run, `iperf3 -J` output (`<rep>-<relay>-<tls>-<transport>-P<flows>.json`),
  the relay sampler output (`.csv`), what the relay printed at start (`.relay.txt`), and
  `drive.log`.

Machine addresses in the data are replaced by `RELAY`, `PEER_A`, `PEER_B`, `CONTROL`,
`MESH_IP` and `HOST`.

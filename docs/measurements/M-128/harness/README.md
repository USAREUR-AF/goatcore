# Relay comparison harness

Scripts that measure goat-relay and the upstream NetBird relay (Go) on the
same machines, one relay at a time, per transport. The result and method are
in [M-128](../../M-128-relay-same-machine-comparison.md).

## Machines

Four Ubuntu 24.04 machines reachable over SSH as a user with passwordless
sudo:

| Role | What runs there |
|---|---|
| control | the upstream combined server (management, signal, STUN) |
| relay | one relay at a time, as a plain process, nothing else |
| peer-a | NetBird client and the iperf3 client |
| peer-b | NetBird client and the iperf3 server |

The relay and both peers should be the same instance type in one cluster
placement group. The Terraform in [`../terraform/`](../terraform/) creates
that shape in AWS. The peers need TCP and UDP to the relay port (33080) and
TCP 8080 and UDP 3478 to the control machine.

## Steps

```bash
cp hosts.env.example hosts.env     # fill in SSH targets and private addresses
bash fetch-binaries.sh             # relays and server from upstream images; clients built from source
bash prepare.sh ../raw/placement-1
bash run-matrix.sh ../raw/placement-1
python3 summarize.py --csv ../raw/runs.csv ../raw/placement-1 [../raw/placement-2 ...]
```

`run-matrix.sh` takes about two hours with the defaults (three runs of five
relay configurations, two client builds, four flow counts, 30 s tests).
`RUNS`, `FLOWS`, `DURATION`, `CONFIGS` and `CLIENTS` narrow it.

## What each test records

Per test, under `<placement>/<relay>-<tls>-<transport>-<client>/r<run>/f<flows>/`:

- `iperf.json`: iperf3's JSON output; throughput is the receiver's figure.
- `relay-samples.csv`: the relay process's CPU ticks and resident memory
  once a second, and the host's CPU ticks.
- `relay-counters-before.txt`, `relay-counters-after.txt`: the relay
  machine's `/proc/net/snmp`, `/proc/net/netstat` and `/proc/net/tls_stat`.
  The TCP and UDP deltas show which transport carried the test; the number
  of live kernel TLS sessions shows whether the relay used kTLS.

Per configuration and run: the relay's start-up log, its memory with no
peers and with the two peers connected and idle, and both peers' `netbird
status --json`.

`summarize.py` leaves out any test whose measured transport or TLS path does
not match its configuration, and lists it.

## How the conditions are set

- **Relayed only.** Both clients run with `NB_FORCE_RELAY=true`.
- **Transport.** `NB_RELAY_TRANSPORT=ws` or `quic` makes the client dial only
  that transport.
- **Connected at once.** `NB_LAZY_CONN=off`.
- **kTLS or TLS in user space.** goat-relay hands the TLS session to the
  kernel when the `tls` module is available. For the user-space runs the
  module is unloaded and blocked from loading, so the same binary falls back.
  The Go relay uses TLS in user space either way; its runs keep the module
  available and the counters show no kernel TLS session.
- **Socket buffer change.** Two client builds from the same tag with the same
  toolchain image and flags; one has the change of netbirdio/netbird#6774
  applied.

## Local check

`local-lab.sh` stands up the four machines as containers on one host so the
harness can be checked without cloud machines (`env bridge ssh` exercises
the SSH path). Numbers from it are not results.

## Before publishing raw output

`prepare.sh` and `run-matrix.sh` end by running `scrub.py`, which replaces
every machine address named in `hosts.env` with its role name and maps
NetBird mesh addresses into `100.127.0.0/16`.

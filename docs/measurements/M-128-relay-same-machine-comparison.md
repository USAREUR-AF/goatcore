# M-128 — goat-relay and the NetBird relay (Go) on the same machines, per transport

| Field | Value |
|---|---|
| Status | In progress: harness checked on one local host; cloud runs not done |
| Issue | [#949](https://github.com/USAREUR-AF/goatcore/issues/949) |
| Relays | NetBird relay 0.83.1 (`netbirdio/relay:0.83.1`); goat-relay `goat-relay/v0.1.0` (`ghcr.io/usareur-af/goat-relay:v0.1.0`) |
| Clients | NetBird 0.83.1 built from tag `v0.83.1`, with and without the change of netbirdio/netbird#6774 |
| Control plane | NetBird combined server 0.83.1 (`netbirdio/netbird-server:0.83.1`) |
| Test class | Ceiling (ADR 1012) |
| Driver | [`M-128/harness/`](M-128/harness/README.md) |
| Raw output | `M-128/raw/` |

## What is measured

Throughput through one relay between two NetBird peers that are forced to
relay, for each relay, transport and client build, at 1, 4, 8 and 16 parallel
TCP flows inside the tunnel, with the relay's CPU time and memory during each
test. The transports are WebSocket over TLS with the TLS record layer in the
kernel (kTLS), WebSocket over TLS in user space, and QUIC. kTLS applies only
to goat-relay; the Go relay's TLS is in user space.

## How

See the harness [README](M-128/harness/README.md). In short: one relay
machine and two peer machines of the same instance type in one cluster
placement group, and a small control machine; each relay in turn as a plain
process with nothing else running on its machine; three runs per cell on each
of at least two fresh placements; 30 s iperf3 tests with the first 3 s
excluded; transport and TLS path checked from the relay machine's counters for
every test.

## Harness check (not a result)

On 2026-11-16 the harness ran every configuration on one 16-core host with
the four machines as containers: the control plane came up through
`/api/setup`, both peers connected through each relay over the requested
transport, kernel TLS sessions were seen on the relay for the kTLS
configuration when the relay shared the host's network namespace, and the
check left out kTLS runs when it did not. Those numbers share one host's
cores between the relay and both peers and are not reported.

## Result

Not yet measured.

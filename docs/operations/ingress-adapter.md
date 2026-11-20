# Ingress adapter (`goat-ingress`)

| Field | Value |
|---|---|
| Status | Built (`ops/provenance/goat-ingress`); ASTERIX over UDP only |
| Design | [`pod-carriage-and-provenance.md`](../design/pod-carriage-and-provenance.md) §3, §7.2, §7.3; [ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md) |

The adapter brings a keyless source (a radar or legacy feed that holds no goat key) into goat under the source's own slot.
For each declared source it receives ASTERIX datagrams on one UDP port, accepts only the declared sender, checks the block
framing, and publishes each frame on

```text
<source-slot>/stream/asterix/<cat>/raw/v1
```

with `adapter-attested` provenance in the sample attachment: the source slot, the adapter's certificate CN, the interface,
the port, the sender address and a SHA-256 of the payload, signed with the adapter's leaf key.

## What it needs

| Need | How |
|---|---|
| The adapter is a machine with its own slot and certificate | Enroll it like any machine; give it a goat context directory: `profile.toml` (with `router_endpoint`), `mtls.cert.pem`, `mtls.key.pem` (P-256), `ca-roots.pem` |
| Each source is enrolled as keyless, naming this adapter | `keyless: true`, `adapters: [<adapter slot>]`, `receive` (interface, port, sender) at enrollment (§7.3) |
| The router admits the adapter's writes to the source's slot | At a pod: the adapter's CN is in the source's `principals` in the pod's rule generation. At a core router: the adapter's CN is in the source's `carry-<anchor>` subject, rendered from the keyless record ([pod-carriage.md](pod-carriage.md)) |
| Storage on `<adapter-slot>/_goat/prov/cert` | The adapter publishes its certificate there at start; subscribers fetch it to verify |

Without the write admission the router drops the samples silently; `goat sub '<source-slot>/stream/asterix/**'` on the
far side shows nothing.

## Configure

One TOML file. Each `[[source]]` is one keyless source.

```toml
# The adapter's own goat context directory.
context = "/etc/goat-ingress/context"
# Optional: overrides router_endpoint from the context's profile.toml.
# router = "tls/203.0.113.5:7447"

[[source]]
slot = "00112233445566778899aabbccddeeff"   # the keyless source's slot
listen = "0.0.0.0:30048"                     # where to receive
interface = "udp"                            # recorded in prov.via.iface (default "udp")
sender = "192.0.2.10"                        # the only sender accepted; "192.0.2.10:5000" pins the port too
format = "asterix:034,048"                   # declared categories; plain "asterix" accepts any
volatile = false                             # true publishes under volatile/<slot>/... (routed, never stored)
```

Two sources may not share a listen address.

## Run

```bash
cd ops/provenance && cargo build --release -p goat-ingress
target/release/goat-ingress --config /etc/goat-ingress/ingress.toml --check
target/release/goat-ingress --config /etc/goat-ingress/ingress.toml
```

`--check` reads the file, loads the adapter's certificate and key, resolves the router and exits. Without it the adapter
runs until SIGINT or SIGTERM. Logs go to stderr (`RUST_LOG` sets the level; default `goat_ingress=info,zenoh=warn`).
Every 60 seconds it logs per-source totals: `published`, `wrong_sender`, `lint_dropped`, `failed`. A dropped datagram is
logged on its 1st, 2nd, 4th, 8th… occurrence, so a flood cannot fill the log.

Under systemd:

```ini
[Unit]
Description=goat ingress adapter
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/goat-ingress --config /etc/goat-ingress/ingress.toml
Restart=on-failure
# A system user that owns /etc/goat-ingress and can read the context's key.
User=goat-ingress
ReadOnlyPaths=/etc/goat-ingress

[Install]
WantedBy=multi-user.target
```

## What is dropped

| Datagram | Result |
|---|---|
| From any address (or port, when pinned) other than `sender` | Dropped, counted as `wrong_sender` |
| Block framing does not hold (short header, LEN below 3, LEN past the end, trailing bytes) | Dropped, counted as `lint_dropped` |
| Carries a category not in `format` | Dropped, counted as `lint_dropped` |
| Mixes categories | Published as one sample per category, each category's blocks in received order |

## Check it from a subscriber

```bash
goat sub '00112233445566778899aabbccddeeff/stream/asterix/**' --provenance --count 5
```

(`--provenance` is built and held until crates/Cargo.lock can change; until then the `Verifier` in
`ops/provenance/goat-provenance` is the way to grade samples.) Each line reads `<key> [adapter-attested by <adapter-cn> (udp port 30048 from 192.0.2.10:5000)] <payload>`. `INVALID`
with a reason means the signature, digest, key or certificate did not check out; `INVALID: no signer certificate on <adapter-slot>/_goat/prov/cert`
means the adapter's certificate is not readable there.

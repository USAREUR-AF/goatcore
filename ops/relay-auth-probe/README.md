# relay-auth-probe

Live behavioral probe for **deployed `goat-relay` instances** — drives the real
WS/Auth handshake (reusing `goat-relay-protocol`'s `Auth` frame) against a running
relay and asserts the relay's auth control: **accept where intended, reject where
intended**. This is the "is the deployed relay enforcing peer-centric auth?" check
that unit/listener tests in CI can't give you (they run against a locally-spawned
relay, not the live tier). First used to validate the nova Rust relay tier — see
`docs/measurements/M-050-goat-relay-rust-tier-live-on-nova.md`.

Standalone crate (not a workspace member), per the `ops/*` tool pattern.

## Cases (per relay)

| Case | Expected |
|---|---|
| valid algo-1 (unbound) token | **accept** |
| valid algo-2 (peer-bound) token, claiming its own `peer_id` | **accept** |
| algo-2 token bound to peer A, claiming peer **B** (impersonation) | **reject** |
| algo-2 token signed with the wrong secret | **reject** |
| malformed / garbage token | **reject** |

Exit code 0 if all cases match expectations on all hosts, 1 otherwise.

## Run

```bash
# RELAY_SECRET = the relay's shared secret (e.g. $NOVA_RELAY_SECRET); the probe
# hashes it (SHA256) internally, matching the relay's key derivation.
RELAY_SECRET="$NOVA_RELAY_SECRET" cargo run -- \
  198.51.100.162 198.51.100.98 198.51.100.60 203.0.113.225
# at least one host is required; with none the probe prints its usage and exits 2
```

## Caveats

- **TLS verification is disabled** (accepts any server cert) so it works against
  the first-light **self-signed** relay cert. This is a *probe*, not a peer; do
  not lift this pattern into anything that carries traffic.
- Validates the **auth handshake** (accept/reject), not data-plane frame
  forwarding/throughput — that's covered by M-009 / M-014 on the same image.
- Connects to `:33073` over `wss://<host>:33073/relay`.

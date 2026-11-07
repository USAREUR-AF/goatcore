# Signal federation between a founder and a joiner

**Status:** written; the gate (`ops/scripts/signal-federation-check.py`), the
per-node assert (`ops/ansible/tasks/signal-federation-assert.yml`), the plays
(`signal-federation-transport.yml`, `signal-federation-verify.yml`) and the
`goatbuilder join-node` phase are built and unit-tested; the sequence is
unmeasured end to end through these plays on hardware.
`goatbuilder standup --site <joiner>` and `goat-standup.sh` do not run the
federation steps; a joiner turns it on through `join-node` phase 13 or the
plays below.

A NetBird peer holds its signalling stream on exactly one signal server. When a
goatnet has two management nodes, an offer from a peer homed on one to a peer
homed on the other only arrives if the two signal servers forward to each
other. Turning that on is part of adding a joiner site's core node, in the same
change as its Spock replication (ADR 1084; transport ADR 1085 Amendment 1;
storm shedding ADR 1081 L2).

## What a federating pair declares

Both sites' `group_vars/<site>_hosts/` carry the same keys, crossed: each side's
peer address and name are the other side's own.

| Key | Value on each side |
|---|---|
| `federation_peers_for_a` | `federation-proxy:10001` (signal dials its local dual-path proxy). Empty = this side does not forward |
| `signal_federation_proxy_enabled` | `true` |
| `federation_peer_mesh_addr` | the peer's `mesh_ip_a` with `:10000` (mesh leg, primary) |
| `federation_peer_public_sni` | the peer's `signal_federation_ingress_fqdn` (public leg, backup) |
| `federation_public_relays` | `"{{ stun_hosts }}"` (the relay fleet) |
| `signal_federation_ingress_enabled` | `true` |
| `signal_federation_ingress_fqdn` | this node's own name, e.g. `federation-a.<zone>` / `federation-b.<zone>` |
| `signal_federation_token` | `"{{ lookup('env', '<GOATNET>_SIGNAL_FEDERATION_TOKEN') }}"`, the same variable on both sides |
| `signal_negative_cache_ttl` | `20` (seconds; caches a recipient no server holds) |
| `mgmt_host_compose_watch_services` | includes `federation-proxy` |

Outside group_vars, two more facts:

- **Relay pin.** The founder's relay inventory
  `inventory/<founder>-goat-relay-alias.ini` runs ingress dispatch
  (`rathole_ingress_dispatch_enabled=true`) and pins each federation name to its
  own node's backend only: `sni_pin` of the backend whose `service` is that
  node's `rathole_client_service_name`. A name in the balanced pool reaches
  whichever node the hash picks, which for a forward is the sender itself.
- **DNS.** Each federation name has A records over the relay addresses (one per
  relay, not a CNAME to the apex), in the goatnet's DNS tree. The proxy dials
  relays directly with the SNI, so the records serve debugging and the gate;
  the certificate is issued by DNS-01 either way.

A joiner that keeps federation off declares `signal_federation_absent_reason`
in its group_vars, with `federation_peers_for_a` empty on both sides.

## The gate

`ops/scripts/signal-federation-check.py <joiner-site> [--peer <founder>] --resolve --require-env`
reads the repository and refuses, one line per defect naming key and file:

- a proxy with an empty peer address, SNI or relay list (the container exits at
  startup); signal pointed at the proxy with the proxy off; an ingress without
  its name
- an ingress or peer list without a token, or a literal token value
- one side forwarding and the other not; a peer list other than the proxy
- crossed values that do not match (peer address against the peer's
  `mesh_ip_a`, SNI against the peer's ingress name, the two token variables)
- the relay pin missing, pinned twice, or pinned to another node's service
- `--resolve`: a federation name that does not resolve, or resolves outside the
  relay addresses; `--require-env`: the token variable unset (value unread)
- both sides off with no `signal_federation_absent_reason`

Exit 0 = on and consistent, or off with a reason. `mgmt-host.yml` and both
plays include the per-node subset (`tasks/signal-federation-assert.yml`), so a
routine converge also refuses a node whose containers could not run its config.
A node with the proxy on and `federation_peers_for_a` empty passes the per-node
assert: it is standing by and forwards nothing.

## The sequence

`goatbuilder join-node <env> --site <joiner> --node <n> --class core` plans it in
phase 13, after the founder-side mirror and the joiner's second Spock wiring.
The token is enforced before any public path reaches a node.

| # | Kind | Command (from `ops/ansible`) | Proves |
|---|---|---|---|
| 1 | GATE | `../scripts/signal-federation-check.py <joiner> --peer <founder> --resolve --require-env` | the repository holds every fact above |
| 2 | A | `signal-federation-transport.yml -e federation_site=<joiner>` | proxy + signal running; signal-a logs `federation auth enforced` |
| 3 | A | the same with `-e federation_site=<founder>` | the same on the founder |
| 4 | A | `traefik-acme.yml -e target=<joiner>_hosts`, then `-e target=<founder>_hosts` | each node's federation router and certificate |
| 5 | A | `<founder>-rathole-client.yml` (else `site-rathole-client.yml`) with `-i inventory/hosts-<founder>.ini -e rathole_client_target=<joiner>_hosts` | the joiner's tunnel to every relay |
| 6 | A | `relay-ingress-add-site.yml -i inventory/hosts-<founder>.ini -i inventory/<founder>-goat-relay-alias.ini -e target=goat_relay_hosts -e founder_site=<founder> -e join_site=<joiner>` | the joiner backend and its pin live on every relay |
| 7 | GATE | `signal-federation-verify.yml -e federation_site=<joiner> -e federation_peer_site=<founder>` | transport up on both nodes; an offer forwarded and delivered |

Every play runs through `../scripts/ansible-preflight.sh` with the goatnet's
inventories, under `ops/scripts/with-secrets` so the token variable is set.

## The proof

`signal-federation-verify.yml`, from each node toward the other:

- **Transport:** `federation-proxy` and `signal-a` running; the token enforced;
  a TCP connect to the peer's mesh address `:10000`; a TLS handshake to each
  relay `:443` with the peer's federation name as SNI, verified against that
  name (the path relay pin → tunnel → peer Traefik).
- **Delivery:** the `signal-federation-probe` image (`--mode=validation`) offers
  to the peer node's own NetBird key, and the sending node's
  `signal_server_forward_sent_total{outcome="delivered"}` must rise.

A direction whose target key is homed on the sender's own signal is delivered
with no forward and reported UNPROVEN. That is the steady state while every
peer reaches signal through the goatnet apex and the relays send the apex to
one node (the joiner's balanced weight 0): the founder holds every peer, so
only joiner → founder is provable. The play passes on transport both ways plus
one proven direction. The other direction becomes provable when peers home on
the joiner, by balanced weight or a signal home pin (ADR 1085).

While the joiner holds no peers, every forward from the founder misses; the
negative cache (`signal_negative_cache_ttl`) bounds the repeats per offline
recipient, and the mesh leg carries them, not the relays, while the mesh is up.

## Turning it off

Set `federation_peers_for_a: ""` on both sides in one change and run
`signal-federation-transport.yml`'s rollback (its header) or `mgmt-host.yml`
on both nodes. To remove the proxy too, `signal_federation_proxy_enabled:
false`. Declare `signal_federation_absent_reason` so the gate passes.

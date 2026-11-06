# goat-peering-wg

One WireGuard tunnel between two edge hosts belonging to different sovereigns.
Neither host joins the other's mesh. The tunnel carries the far edge's fabric port
and the ports the deployment names, and the role's firewall drops the rest.

The driver runs this in two steps. `goat federate init` reads the interface's public
key with `wg show`, so the interface has to exist before our half is signed.

| Phase | Play | What exists after it |
|---|---|---|
| 2 | `peering-prepare.yml` (`tasks/prepare.yml`) | The interface and a private key that stays on this host. No address and no peer. |
| 11 | `peering-wg.yml` (`tasks/main.yml`) | The agreed address, the one peer, the firewall. Read back from the interface. |

Phase 11 runs only when the agreement's `transport.kind` is `wireguard`, and the role
refuses under any other kind. `goat-peering-edge` is the same step for
`ipsec-fips`.

## Where every value comes from

| Value | Source |
|---|---|
| Link addresses, MTU, who listens | the agreement, rendered by `goat federate apply` into `federation-<them>-transport.yml`. `parties[0]` holds the first address of the /31 or /30. |
| Far public key and endpoint | the counterparty package's `transport_offer`, the signed file phase 6 composed from |
| Interface name | `peering_wg_interface`, the same string as `[transport] interface` in `federation.toml` |
| Listen port, extra ports, FIPS waiver | this deployment's `group_vars/<group>/deployment.yml` |

The listening side configures no endpoint and waits. The other side sets the endpoint
and a keepalive.

## The FIPS posture

WireGuard's primitives are not FIPS-approved, so this role never makes the claim. The
claim sits in the Zenoh TLS session inside the tunnel
(`cross-sovereign-federation.md` §5.1).
Waiver: DBB-WVR-001.

| `peering_wg_fips_required` | `peering_wg_fips_waived_reason` | Outcome |
|---|---|---|
| true (default) | set | `proceed-waived`: configures, prints a deviation banner, records the reason |
| true | empty | `refuse`, having written the state file first |
| false | any | `proceed-no-claim`: configures and records that no claim is made |

## Firewall

`peering-wg.nft.j2` holds one table per interface, and it filters at prerouting because
the edge router's published port is a DNAT that never reaches the input chain.

- **From the tunnel:** established traffic, ICMP, and the listed TCP ports from the far
  link address to ours. Everything else is counted and dropped.
- **Onto the tunnel:** the same ports to the far link address. Everything else is
  counted and dropped.
- **The tunnel port:** answers only the address in the counterparty's offer, when that
  offer is a literal address.
- **MSS:** clamped to the route's MTU on forwarded traffic, because the edge's session
  is forwarded from a compose network.

## Behind a peering forwarder

When neither edge accepts inbound UDP, this edge listens through a forwarder
(`roles/goat-peering-forwarder`, `peering-forwarder.md`). Off by default; with
`peering_wg_via_forwarder: false` every file this role renders is unchanged.

| Variable | Default | Set to |
|---|---|---|
| `peering_wg_via_forwarder` | `false` | `true` |
| `peering_wg_forwarder_endpoint` | `""` | `<forwarder>:51821`, from the forwarder play |
| `peering_wg_forwarder_carrier_public_key` | `""` | the key the forwarder play prints |
| `peering_wg_carrier_interface` | `wg-carry0` | |
| `peering_wg_carrier_local_ip` / `_remote_ip` | `169.254.30.1` / `169.254.30.0` | the forwarder entry's remote / local |
| `peering_wg_carrier_listen_port` | `51822` | the local port the carrier dials from |
| `peering_wg_carrier_mtu` | `1420` | |
| `peering_wg_carrier_keepalive` | `25` | |

- **Phase 2** makes the carrier key and prints it. With the forwarder's key and
  endpoint set, it brings up `goat-peering-wg@wg-carry0`, dialling the forwarder.
- **Phase 11** refuses unless this side is the listener and the agreed MTU is at most
  `peering_wg_forwarder_max_mtu` (1340); offer `measured_mtu = 1300`. It reads the
  carrier back and reports its latest handshake.
- **Firewall:** the peering port is accepted only on `iifname wg-carry0` from the
  forwarder's carrier address; anything else on the carrier, and the peering port on
  any other interface, is counted and dropped. The source-address rule is not written.

## Cutting it

`systemctl stop goat-peering-wg@<interface>` removes the interface and the firewall
table. It is local, immediate, and needs nothing from the other side.

## State file

`/var/log/goat-peering-wg-<interface>-state.json` records the decision, both sovereign
ids, the far key, the link and what the kernel reports for FIPS. It is written before
the FIPS gate, so a refusal leaves its evidence on the host.

## Evidence

`ops/federation-e2e/run.sh` runs this role through the real driver between two
hermetic sovereigns in CI, and the proof is read back from the receiving side.

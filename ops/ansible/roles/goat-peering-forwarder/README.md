# goat-peering-forwarder

A host with a fixed public address that relays one UDP port per peering to an edge
that sits behind NAT. It carries ciphertext only. Design:
[`peering-forwarder.md`](../../../../docs/design/peering-forwarder.md), ADR 1218.

```
partner edge ──wg-fed0──▶ forwarder <public>:51820
                           │ DNAT to 169.254.30.1:51820, masquerade to 169.254.30.0
                           ▼
               wg-carry0 ◀── dialled by our edge to <public>:51821, keepalive 25 s
                           ▼
                    our edge wg-fed0 (listener)
```

Play: `ops/ansible/peering-forwarder.yml`, host group `peering_forwarders`. Off unless
`peering_forwarder_enabled: true`.

## What an operator sets

```yaml
peering_forwarder_enabled: true
peering_forwarder_public_interface: eth0        # default: the default-route interface
peering_forwarder_peerings:
  - name: xaa                                   # the partner; a label
    edge_carrier_public_key: "<44 base64>"      # printed by the edge's prepare run
    partner_sources: [192.0.2.20]               # optional; empty = any source
```

Each entry takes the rest from `peering_forwarder_peering_defaults`: `interface`
`wg-carry0`, `peering_port` 51820, `edge_peering_port` 51820, `carrier_port` 51821,
`carrier_local_ip` 169.254.30.0, `carrier_remote_ip` 169.254.30.1, `carrier_prefix_len`
31, `carrier_mtu` 1420. A second peering is a second entry with its own interface,
peering port, carrier port and carrier subnet; the role refuses two entries that share
any of them.

## Bring-up order

1. Edge: `peering-prepare.yml` with `peering_wg_via_forwarder: true`. It makes the
   edge's carrier key and prints it.
2. Forwarder: this play, with that key as `edge_carrier_public_key`. It prints its own
   carrier key and the values the edge sets.
3. Edge: `peering-prepare.yml` again with `peering_wg_forwarder_carrier_public_key` and
   `peering_wg_forwarder_endpoint`. The carrier comes up.
4. The edge's `federation.toml` `[transport]` offers `endpoint = "<forwarder>:51820"`,
   `listener = true`, `measured_mtu = 1300`, and the runbook continues at step 1.

## What it does on the host

| Item | Where |
|---|---|
| One carrier key per peering, made on the host, never from a variable | `/etc/goat-peering-forwarder/<interface>.key` |
| Carrier environment: listen port, address, the one edge peer, AllowedIPs = edge /32 | `/etc/goat-peering-forwarder/<interface>.env` |
| One nftables table, `inet goat_peering_forwarder` | `/etc/goat-peering-forwarder/forwarder.nft` |
| IPv4 forwarding | `/etc/sysctl.d/60-goat-peering-forwarder.conf` |
| A unit per carrier, enabled, survives reboot | `goat-peering-forwarder@<interface>.service` |
| State: edge key, forwarder key, ports, `holds_peering_material: false` | `/var/log/goat-peering-forwarder-state.json` |

The table admits, on the public interface, each peering port (optionally only from
`partner_sources`), each carrier port, replies, ICMP and DHCP, and drops the rest. Each
peering port is destination-translated to its edge's carrier address and masqueraded
out of its carrier. The forward chain admits only that flow and its replies and drops
everything else. Nothing is admitted from a carrier to the forwarder itself. TCP 22 is
closed unless `peering_forwarder_ssh_sources` names an address.

## Refusals

The role stops before changing the host when:

- the host is a mesh peer: interface `wt0` exists or `netbird.service` is running;
- `/etc/goat-peering-wg` exists, so the host is a peering edge;
- `/etc/goat-peering-forwarder` holds any file other than the carrier keys, carrier
  environments and table this role writes;
- an edge carrier key is not 44 base64 characters ending in `=`, or two peerings share
  an interface, port, address, name or edge key.

`ops/ci/check-peering-forwarder-holds-nothing.py` refuses, in CI, a change that would
let the role be given a peering key, CA, SVID, fabric certificate or mesh enrolment.

## Cutting it

`systemctl stop goat-peering-forwarder@<interface>` removes that carrier and severs
that peering. The table stays loaded, so the public interface stays closed.

## Evidence

`ops/ci/test-peering-forwarder-e2e.sh` runs the rendered templates of this role and of
`goat-peering-wg` (with `peering_wg_via_forwarder: true`) in four containers, with our
edge behind NAT, and checks the carrier, the peering handshake through the forwarder,
traffic both ways and the drops. The role's task flow runs under the syntax check.

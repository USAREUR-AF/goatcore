# goat-host-firewall

The baseline host firewall of a peering edge host and of the core router host it dials:
inbound default deny in one nftables table of its own (`inet goat_host_firewall`), accept rules
derived from the peering's own values, and a per-deployment override that tightens freely and
loosens only a named rule with a recorded reason.

Design, rule table and override contract: [docs/design/peering-host-firewall.md](../../../../docs/design/peering-host-firewall.md).

## Where it runs

Opt-in: `goat_host_firewall_baseline: true` in the host's group_vars. Off by default, and with it
off nothing here runs.

| Play | Profile | Phase |
|---|---|---|
| `peering-wg.yml` | edge, with the WireGuard peering's values | 11 |
| `peering-edge.yml` | edge, with the IPsec peering's values | 11 |
| `peering-republisher.yml` | core | 12 |

Required when on: `goat_host_firewall_management_sources`, the addresses or CIDRs SSH is admitted
from. Everything else has a default that belongs to no deployment (see `defaults/main.yml`).

## What it does, in order

1. Refuses missing inputs (GB-210), a pending rollback of an earlier run (GB-214) and a host that
   carries the other profile (GB-210); reads the peering services, the listening sockets and the
   SSH session running the play.
2. Renders on the controller with `files/goat-host-firewall-render.py`, which refuses an override
   naming an unknown or fixed rule (GB-211), a loosening without a reason (GB-212), a ruleset
   that would drop the managing session (GB-213), and one that would close a listening port
   nobody named in `goat_host_firewall_closed_listeners` (GB-215). On an edge, a peering service
   with no stored entry is refused (GB-210) after this run's entry is stored. Nothing else on the
   host has changed yet.
3. In check mode, prints the ruleset's hash and stops.
4. When the hash, or the loaded table, differs from what the last run recorded: `nft -c`, copies
   the last confirmed ruleset for the rollback, arms a `systemd-run` rollback, loads the new table
   in one `nft -f`, drops its connection, makes a fresh one, and only then, while the timer is
   still armed, disarms it and records the hash (GB-214 when it cannot).
5. Stores the edge's peering in `/etc/goat-host-firewall/peerings/<if>.json`, so the next
   peering's run on the same host keeps it, and enables `goat-host-firewall.service` for boot.

## Tests

```
python3 -m pytest -q ops/ansible/roles/goat-host-firewall/tests/test_render.py
bash ops/ansible/roles/goat-host-firewall/tests/syntax-check.sh
bash ops/ansible/roles/goat-host-firewall/tests/netns-behavior.sh     # needs root; namespaces only
bash ops/ci/test-host-firewall-role.sh                                 # needs docker --privileged
```

## Taking it off a host

`systemctl disable --now goat-host-firewall` (stopping it deletes the table), then set
`goat_host_firewall_baseline: false`.

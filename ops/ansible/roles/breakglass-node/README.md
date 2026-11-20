# breakglass-node

The host-side facts a `BREAKGLASS` node ([ADR 1114](../../../../docs/adr/1114-breakglass-node-class-and-two-path-reach.md)) needs that no other role owns. Playbook: `ops/ansible/breakglass-nodes.yml`. Every part is opt-in from the site's group_vars; `defaults/main.yml` says why each exists.

1. The bad-day transport's package, and tailscale's netfilter mode asserted so the two transports coexist.
2. A time source and a boot-time clock step for sites where NTP cannot work.
3. A DNS-over-TLS resolver for sites that block port 53 egress.
4. The out-of-band interface's route metric (ADR 1114 §5).

## The out-of-band route metric (ADR 1114 §5)

| Part | Status |
|---|---|
| `breakglass_oob_interface` names the out-of-band interface; `breakglass_oob_route_metric` (default 1000) sets its metric, under NetworkManager or systemd-networkd, whichever owns the interface | **built**, not measured on a real node |
| After applying, every default route via that interface sits strictly above every other default route, or the converge fails, naming each route's interface and metric | **built**, not measured on a real node |

Tested by `tests/test_breakglass_oob_route_metric.py`, against stand-in `nmcli`, `networkctl` and `ip`. The tasks touch only the named interface; the test checks that no command names the primary.

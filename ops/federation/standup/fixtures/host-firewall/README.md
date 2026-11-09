# Host firewall plan fixture

Synthetic deployments (peering `velmara`, documentation addresses) that both
sides of the plan/deploy equality test read. Each variant has `repo/` (the
inventory and vars), `work/` (agreement, apply's rendered values, the partner's
package, phase 1's probe answer) and `expected/<host>.json` (the plan's input).

- `crates/goatbuilder/src/federate/hostfw.rs` (test
  `the_plan_builds_the_recorded_input`) asserts the plan builds exactly
  `expected/<host>.json` for every variant, and that each renders.
- `tests/test_host_firewall_plan_input.py` has Ansible evaluate the role's and
  the plays' own tasks and variables, copied from their YAML, on the same `repo/`
  and `work/`, and asserts the renderer gives the same sha256 for that input and
  for `expected/<host>.json`.

Together they show the plan's ruleset hash equals the deploy's for these
declarations. Regenerate `expected/` with
`GOAT_RECORD_HOST_FIREWALL=1 cargo test -p goatbuilder --bin goatbuilder hostfw`
and review the diff.

| Variant | Covers |
|---|---|
| `wireguard` | `ansible_port` from three group layers (a shallower group's playbook group_vars outrank a deeper group's inventory group_vars and `[group:vars]`), host_vars over the inventory line, apply's values over a stale group_vars value, a peering another run stored on the edge and a stale stored copy of this run's entry, the SPIRE bundle port on the SPIRE server's core host |
| `ipsec` | an ipsec-fips agreement (peering-edge.yml, which loads no apply values on the edge), the edge's SPIRE forward with a literal server address, QUIC on the core, an empty mesh DNS port, an override that drops a rule, SPIRE on but the core not the SPIRE server |
| `forwarder` | WireGuard behind a forwarder with a literal forwarder address, a partner endpoint that is a name (left unfiltered), a non-default interface and carrier port, an override that loosens a rule with a reason (written into the ruleset text) |

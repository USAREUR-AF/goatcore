# Host firewall plan fixture

One synthetic deployment (peering `velmara`, documentation addresses) that both
sides of the plan/deploy equality test read:

- `crates/goatbuilder/src/federate/hostfw.rs` (test
  `the_plan_builds_the_recorded_input`) asserts that the plan builds exactly
  `expected/<host>.json` from `repo/` and `work/`.
- `tests/test_host_firewall_plan_input.py` has Ansible evaluate the role's and
  the plays' own expressions, read from their YAML, on the same `repo/` and
  `work/`, and asserts the renderer gives the same sha256 for that input and for
  `expected/<host>.json`.

Together they show the plan's ruleset hash equals the deploy's for this
declaration. Regenerate `expected/` with
`GOAT_RECORD_HOST_FIREWALL=1 cargo test -p goatbuilder --bin goatbuilder hostfw` and
review the diff.

It covers: an `[group:vars]` value (`ansible_port`), a stale group_vars value
that apply's rendered file overrides (`goat_peering_remote_link_ip`), host_vars
over an inline inventory var (`goat_host_firewall_mesh_interface`), a peering
another run stored on the edge and a stale stored copy of this run's own entry,
and the SPIRE bundle port on the SPIRE server's core host.

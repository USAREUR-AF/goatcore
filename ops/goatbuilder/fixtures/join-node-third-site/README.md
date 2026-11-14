# join-node-third-site: a third site joins a goatnet that already has a joiner

A checkout-shaped fixture for gate row H1 (docs/design/phone-node.md, "What the
run needed outside the tool", rows 1 to 3). The synthetic goatnet `zagros` has
its founder site `alpha`, the joiner `beta`, which federates signal with
`alpha`, and a third site `gamma`, which declares signal federation off with
its reason. `gamma`'s node `gamma-core-1` is not enrolled yet: its mesh
address is the placeholder `TBD_GAMMA_MESH_IP_A` until phase 12.

`crates/goatbuilder/tests/join_node_third_site.rs` copies this tree, adds the
real `standup-config-preflight.py` and `signal-federation-check.py` and
stand-ins for every program that reaches a host, and runs `join-node` from
phase 1 to 15.

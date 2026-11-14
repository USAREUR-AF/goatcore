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

`ops/pins/fleet-pins.yaml` is the goatnet's pin table as far as the health
publishers go: the end-state model, which the test also copies, reads it and
marks fabric-health and mesh-health present at `gamma` and time-health absent.
join-node deploys the publishers the model marks present.

## The phone site (gate rows H3, H4, H5)

A fourth site, `delta`, is a phone site: its one node `delta-phone-1` is of
the machine class `phone`. It is listed under `[class_phone]` in
`ops/ansible/inventory/mgmt-hosts.ini`, has no LAN address there, and
`group_vars/delta_hosts/vars.yml` states none of what the class declares
(`ops/ansible/group_vars/class_phone.yml`, which the test copies in).

`crates/goatbuilder/tests/join_node_phone.rs` runs
`join-node zagros --site delta --node delta-phone-1 --class phone --via usb`
on a copy of this tree. A stand-in `adb` answers for the phone, and a stand-in
`ssh` carries each connection through the proxy command the tool wrote and
runs what is piped to the VM against a directory that stands for the VM's
filesystem.

---
heading: For someone deploying
title: Added: goat-peering-edge rotates an IPsec peering's leaf from staged material, and puts the old one back if the link does not come up
---

An `ipsec-fips` peering's edge role, `goat-peering-edge`, rotates its IPsec
leaf when `peering_edge_rotation` names a rotation id. The role still writes
no secret: the offline ceremony issues the new leaf, and an operator stages it
beside the live files as `.next` files. The role checks the staged pair
(GB-289, GB-290), sets the live pair aside once in `.previous-<id>/`, switches,
re-authenticates the IKE SA, and puts the old pair back if the SA does not
come up (GB-302). An `ipsec-ca` rotation also needs the partner's read-back of
our new CA's fingerprint (GB-303). `peering_edge_remote_ca_next_path` accepts a
partner's new CA beside the current one during their rotation. It is opt-in,
and without a rotation id nothing changes. `goatbuilder federate rotate` does
not drive it yet.

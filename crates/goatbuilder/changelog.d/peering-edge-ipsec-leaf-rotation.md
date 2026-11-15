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
come up (GB-302); a rotation rolled back stays reported as rolled back on
every later run of its id. An `ipsec-ca` rotation also needs the partner's
read-back of our new CA's fingerprint (GB-303) before our new CA replaces the
current one. `peering_edge_rotation_retire` removes the set-aside key only once
the confirm recorded the new leaf live (`.previous-<id>/confirmed`, the IKE SA
the re-authentication made), and never moves the live pair, the staged pair or
our CA (GB-309 for an id with nothing to retire, or a rotation not confirmed
or with an unreadable record; a re-run with the same id confirms it). The
confirm is recorded only once the SA from before the re-authentication is gone,
and a roll-back removes the record. The confirm counts only an IKE SA the
re-authentication made, not the old one still up beside it, and the role now
sets `make_before_break = yes` in its charon drop-in on every IPsec peering
edge, instead of leaving it to the packaged strongSwan's default
(`peering_edge_make_before_break`). The first apply after this change restarts
strongSwan once on each ipsec-fips edge, which drops that peering's link
briefly: apply both sides of a peering in one window. The rotation always
rekeys the IKE SA before the re-authentication, so no rekey of either side, which
keeps the old authentication, can pass the confirm on the old leaf, and it refuses
an IKE rekey interval too short to hold the confirm. `peering_edge_remote_ca_next_path` accepts a
partner's new CA beside the current one during their rotation. It is opt-in,
and without a rotation id nothing changes. `goatbuilder federate rotate` does
not drive it yet.

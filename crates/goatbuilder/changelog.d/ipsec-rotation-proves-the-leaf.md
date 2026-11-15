---
heading: For someone deploying
title: Changed: an IPsec rotation proves the leaf charon signs with, and checks the strongSwan version
---

`goatbuilder federate rotate <partner> ipsec-leaf` and `ipsec-ca` now check
more on the edge (`goat-peering-edge`). Before anything moves, the rotation
reads the running charon's version (`swanctl --version --daemon --raw`) and
refuses a strongSwan series it was not tested with, or a version it cannot
read, with GB-341. The tested series are 5.9 and 6.0, in
`peering_edge_rotation_strongswan_tested`; add a series there only after
running the rotation on it. It also refuses a staged leaf on the live leaf's
key (GB-290), and refuses, naming the peering, when another peering on the
edge lacks its certificate, key or CA files (GB-343). Once the new IKE SA is
up, the rotation reloads the credentials from the files alone
(`swanctl --load-creds --clear`), which drops the old certificate that a
plain `--load-creds` keeps listed. The confirm then needs charon to list
exactly one certificate of our id: the live leaf's serial, holding its
private key, on a key other than the old leaf's. Otherwise the old pair is
put back (GB-342), and the message names the serials. `swanctl --list-sas`
names only the identities, which an old and a new leaf share, so the SA alone
never showed which leaf it ran on. The confirm record
`.previous-<id>/confirmed` now also names the serial, both keys' digests and
the strongSwan version. Measured on a real charon, strongSwan 5.9.8 and
6.0.4, with one and with two peerings on the edge.

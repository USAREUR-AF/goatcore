---
heading: For someone deploying
title: Added: `goatbuilder federate rotate <partner> ipsec-leaf` and `ipsec-ca` rotate an ipsec-fips peering's leaf
---

On a peering whose agreement chose ipsec-fips, `goatbuilder federate rotate
<partner> ipsec-leaf` runs phases 11 to 14 and passes the rotation to
goat-peering-edge, which checks the leaf an operator staged, sets the live
pair aside once, switches and rolls back if the link does not come up.
`rotate <partner> ipsec-ca` also needs `--partner-holds-ipsec-ca <sha256>`,
the fingerprint the partner read back after placing our new CA (GB-303).
The rotation's record, `<work>/.rotation-ipsec`, lets a stopped run resume
with the same id; a run of another kind is refused (GB-298). `--retire <id>`
removes the set-aside key once both sides passed phase 14. A WireGuard
peering's keys are rotated with `leaves`, `carrier` or `transport`.

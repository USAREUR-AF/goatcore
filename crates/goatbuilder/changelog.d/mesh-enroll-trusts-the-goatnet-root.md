---
heading: For someone deploying
title: "Fixed: a LAN site's NetBird client trusts the goatnet's root before it enrols"
---

A LAN site's management serves certificates from the goatnet's own CA, and a
fresh host trusted only the public roots, so its NetBird client could not
verify management and `netbird up` timed out. Standup phase 12 stopped there
on a goatlite founder that declares `mesh_enrollment_url`, and join-node
phase 10 enrols with the same play. `mesh-enroll-host.yml` now gives the
goatnet's root to the NetBird client alone first, where the site serves file
certificates and the root is in the checkout: the file in
`/etc/goat/netbird-trust/` and a `netbird.service` drop-in that adds that
directory to `SSL_CERT_DIR`. The host's system store is not changed.
`netbird_api_ca: public` leaves the host alone.

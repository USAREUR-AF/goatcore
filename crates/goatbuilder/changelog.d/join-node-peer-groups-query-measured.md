---
heading: For someone contributing
title: Added: join-node's peer-groups query is measured against NetBird's own schema
---

`ops/netbird/tests/peer-groups-query.sh` starts the pinned NetBird management
on a disposable Postgres, enrols two real clients with keys from
`mgmt-host-bootstrap`, and runs the query `mesh-enroll-host.yml` uses at
join-node's phase 10 to read the groups the minting host's peer holds.
`tests/test_mesh_enroll_peer_groups.py` holds the query's shape always, and
its result when the script runs it. The query was designed and is now
measured.

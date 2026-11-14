---
heading: For someone deploying
title: "Changed: join-node has no manual phase after reaching the node"
---

`goatbuilder join-node` stopped at phase 8, to mint a setup key, and at
phase 10, to enrol the node by hand. Now phase 10 enrols the node itself. It
mints a single-use, one-hour key where the NetBird database is (for a joiner
site, the founder's first host), hands it to `netbird up` through a 0600 file
on the node, and checks that the node is on the kernel ACL plane. The key
never reaches your machine and is not stored. Phase 10 then writes the node's
mesh address into your checkout; the run's summary and its run record name
each file it wrote, for you to review and commit. Phase 8 now checks that the
site declares `mesh_enrollment_url`, and phase 9 sends no setup key. Phase 2
resolves the first-login password through the secrets injector and refuses
one that still holds a reference (GB-269); the password reaches `sudo` on
standard input. Only phase 1, reaching the node, stops the run.

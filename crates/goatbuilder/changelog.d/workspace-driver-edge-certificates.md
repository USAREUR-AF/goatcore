---
heading: For someone deploying
title: Fixed: an edge deployed from a workspace gets the certificates it mounts, or phase 12 says why
---

`goatbuilder federate` run from a machine that is not the edge host (an operator's
workspace) used to deploy an edge whose leaves and CAs lived only on the
workspace, in `<work>/pki`: the edge started, mounted nothing and never reached
the core. Phase 12 now refuses first (GB-275) when the edge group's vars point
the edge at files in `<work>/pki`, the inventory names the edge as another
machine, and `goat_zenoh_edge_install_certs` is off; the fix is to turn that
switch on. With it on, the `goat-zenoh-edge` role now also installs our CA's
certificate as `<cert_dir>/our-ca.pem`, from `<work>/pki/our-ca.pem` (never the
CA's key: a source holding a private key is refused), and `goatbuilder federate
allocate` renders our CA's path and the core endpoint's override at that path.

---
heading: For someone deploying
title: "Added: the `octo-pod` type, installed at octo-pod's pinned, signed release"
---

`join-node --class octo-pod` builds an octo-pod by push, on a small computer or
a host-class machine; the type requires an encrypted root. Its phase 13 installs
the release component `octo-pod` where `ops/pins/fleet-pins.yaml` pins it for the
pod's group: goatbuilder verifies the release's signed checksums file on the
operator's machine (phase 3 runs `goatbuilder deploy --dry-run` for it first),
and `ops/ansible/octo-pod-deploy.yml` installs the package and runs octo-pod's
own `bootstrap`, `init` (for a pod with no identity, named for the machine),
`apply` and `verify`. Where there is no pin, phase 13 is a declared skip that
says which pin is missing, and the machine is not a pod. The public pins carry
no octo-pod row; a deployment with pods adds one. `standup` builds none.

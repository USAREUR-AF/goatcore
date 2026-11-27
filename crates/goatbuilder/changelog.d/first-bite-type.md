---
heading: For someone deploying
title: "Added: the `first-bite` type, installed at first-bite's pinned, attested release"
---

`join-node --class first-bite` builds a decoder by push, on a small computer or
a host-class machine. The machine declares `first_bite_interface`, the interface
it captures on, and may name one of the release's dictionaries in
`first_bite_dictionary`; a missing interface or an unknown dictionary is refused
(GB-415). Its phase 13 installs the release component `first-bite` where
`ops/pins/fleet-pins.yaml` pins it for the machine's group: each archive is held
to its own GitHub artifact attestation on the operator's machine, and
`ops/ansible/first-bite-deploy.yml` runs the archive's own installer, then
checks the settings it keeps (GB-416) and the running service. Where there is no
pin, phase 13 is a declared skip and the machine decodes nothing. `standup`
builds none.

---
heading: For someone deploying
title: "Added: the `network-tap` type; a tap states its mode, and inline is shown fail-closed"
---

`ops/goatbuilder/targets/network-tap.yaml` is the network tap's type. A machine
of it declares `tap:` in its host_vars: `mode: listen` (fail-open, with its
`source`) or `mode: inline` (fail-closed: the link is down whenever the machine
is), its `ports` and its `management` interface. There is no default, and
`join-node` and `doctor --target` refuse a machine with none or a wrong one
(GB-380), as they refuse `tap:` set in more than one host_vars file. Both print
`inline, fail-closed` on an inline tap. The type requires an encrypted root,
which `doctor --target` reads from the checkout only, so it reports it UNKNOWN,
not as a pass.

Nothing installs the tap itself yet: goatrope, its installer, has no release.
A machine joined as `network-tap` is not a working tap.

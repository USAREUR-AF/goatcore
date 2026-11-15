---
heading: For someone deploying
title: "Added: a two-driver peering's phase 2 runs on the edge driver and the controller"
---

A peering declared `drivers: two` used to stop at phase 2's hand-off with
nothing to run on the other side. Now the edge driver runs the hand-off's
command, `goatbuilder federate <peering> --side edge --from 2 --to 2
--expect-commit <commit>`. It makes the federation interface on the edge and
prints the way back with a confirm code. The result holds the interface's
public key, and no private key leaves the edge. On the controller,
`goatbuilder federate import <peering> 2 <file> --sha256 <code>` takes it in,
and `--from 2` runs the controller's half: the SPIRE bundle export on the
SPIRE server host, which never reaches the edge host. Phase 3's import then
refuses an edge half that measured another interface or another public key
than phase 2's result handed over (GB-114). A phase 2 that also runs
a forwarder's steps is refused as not built (GB-267), as are the edge's own
leaf keys, which phase 8 still makes on the controller.

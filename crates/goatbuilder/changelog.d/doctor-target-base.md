---
heading: For someone deploying
title: "Added: `doctor --target <type> --site <site>` checks a site's machines of one type against its file"
---

`goatbuilder doctor --target <type> --site <site>` (and `--node <machine>` for
one) reads the type's file and the checkout's inventory: the platform family
each machine of the type declares, accepted or refused as `join-node` would
(GB-376), and whether the type's plays are in this tree. It reaches no host and
changes nothing. The machine's end state (OS version, encryption, chrony before
docker and the rest of the list in docs/design/goatbuilder-build-targets.md
section 3.5) is a row that says it is not checked yet.

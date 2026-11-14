---
heading: For someone deploying
title: Added: `goatbuilder federate rotate <partner> leaves` and `carrier` replace a peering's keys, and an interrupted rotation finishes
---

`goatbuilder federate rotate <partner> leaves` replaces every peering leaf of
one running peering and runs the command again from phase 8 to 14. Phase 10
stops, as in every run, for the review of the new leaves, and phase 12 installs
them. Once phase 8 has passed, the command that carries on is the plain resume.

Phase 8's rotation is now resumable. The old pairs are copied aside at mode
0600 under the peering's own `pki` folder, and removed once the new pairs are
installed and validated. A run killed during the rotation is finished by the
next run from the pairs it had already minted, or, if they do not validate,
rolled back to the old pairs. No run mints a third pair.

`goatbuilder federate rotate <partner> carrier` replaces the forwarder carrier
keys of one running peering and runs the command again from phase 2 to 14.
Phase 2 sets each carrier key aside once and makes a new one. Until phase 2
passes, running the same `rotate <partner> carrier` again is the resume: it
reuses the rotation that is under way and sets nothing aside twice.

The other keys are refused before anything runs, each saying why: `transport`
and `signing` with GB-189, `transport` while the partner cannot yet receive
our new package; `ca` with GB-282, because rotating our CA needs an ADR first.

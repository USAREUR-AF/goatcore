---
heading: For someone deploying
title: Fixed: after a signing-key rotation, phase 5 hands the directory over to the new key
---

The peering directory binds our identity to the first key that wrote it and
takes a new key only on a request signed by the old one. Phase 5's directory
step now reads the key the directory binds. When it is not ours, it signs
the pointer's write with the previous seed, `FED_PREVIOUS_SIGNING_KEY` (or
`--previous-key`), and prints both keys. Before, every write after a
rotation was refused. If the previous seed is missing, unreadable or of a
third key, it prints GB-285 and publishes nothing; the step stays advisory
and phase 5 goes on. The `rotate … signing` stage is not built (#683).

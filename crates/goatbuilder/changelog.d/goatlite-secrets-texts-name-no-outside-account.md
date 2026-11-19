---
heading: For someone deploying
title: "Fixed: a goatlite standup no longer points at 1Password or goatbuilder.yaml"
---

Phase 6 of a goatlite standup told the learner to copy the bootstrap setup key
to 1Password. It now says to keep the key only until it enrols the first peer,
and only in the goatnet's local secrets store. Phase 1 told a goatlite to
declare `secrets: {store: env-file}` in `ops/goatbuilder/goatbuilder.yaml`, a
file a goatlite does not have; under goatlite it no longer gives that advice.
The full preset's text is unchanged in both places.

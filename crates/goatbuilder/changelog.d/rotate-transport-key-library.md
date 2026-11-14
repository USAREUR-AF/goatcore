---
heading: For someone contributing
title: Added: rotating the WireGuard peering key, as a library and in the role, not yet on the command line
---

`rotation::transport_run` rotates a peering's WireGuard key (`wg-fed0`). It
writes `<work>/rotation.json` naming the rotation once, keeps the rotation's
id in `<work>/.rotation-transport` until phase 2 passes, and runs from phase
2. `goat-peering-wg`'s `tasks/transport-key.yml` then sets the key aside once,
in a `.previous-<id>/` directory, and makes a new one, and
`tasks/transport-live.yml` restarts the link while the interface holds a key
other than the one on disk. Without a rotation id, phase 2 never rotates the
key. The `rotate` verb does not call it yet, and the partner taking the new
package is not built (#680).

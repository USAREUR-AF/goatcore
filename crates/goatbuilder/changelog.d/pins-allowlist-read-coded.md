---
heading: For someone deploying
title: Fixed: an unmigrated-pins allowlist that cannot be read is refused with GB-254
---

`goatbuilder pins`, and the verbs that run its check (`release`, the
preflight, the fast lane), read `ops/pins/unmigrated-pins.txt` and passed a read
error up as it came, with no code and no file name (for example
`Is a directory (os error 21)`). It now carries GB-254 and names the file,
as the other files `pins` reads already do.

---
heading: For someone deploying
title: Added: goatbuilder image rpi5 builds a Raspberry Pi 5 one proven step at a time
---

`goatbuilder image rpi5` runs `ops/rpi-base-image/`'s numbered steps one at a
time: each step's script, or, where a person is needed (the card, the console
during encryption, the YubiKey), the builder's directions. A step runs only
when the step before it is proven and the Pi's copy of its script is this
checkout's; its own result is read after it, and a step whose result already
holds runs nothing. Two steps are new: `verify-image` (`01b-verify-image.sh`
reads the flashed card back against the pinned image and this checkout) and
`verify-box` (every `doctor --target --live` small-computer row). With no
`--step` it reads every proof it can and names the next step. GB-411, GB-412
and GB-413. The README gains a bench procedure (#1346).

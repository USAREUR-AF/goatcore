---
heading: For someone deploying
title: "Fixed: `fastlane` reads and pins an image under the owner its fleet ref names"
---

A component whose image another organisation builds is pinned as an absolute
`ghcr.io/<owner>/<name>` ref. `goatbuilder fastlane` looked it up under this
checkout's own owner, and would have pinned it there. It now reads and pins it
under the owner the ref names, as `ops/scripts/fastlane.sh` does.

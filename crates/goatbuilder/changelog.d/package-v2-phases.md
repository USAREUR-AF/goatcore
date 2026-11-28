---
heading: For someone deploying
title: "Added: a peering with `package: v2` emits package v2 at phase 3 and composes the v2 pair at phase 6"
---

A peering that declares `package: v2` in `goatbuilder.yaml` (ADR 1242) is no
longer refused by `goatbuilder validate`. Phase 3 emits a v2 package and phase 6
composes the v2 pair (`goatbuilder federate init` and `compose` take
`--package-version v2`, and the selection reaches the phases as
FED_PACKAGE_VERSION). A v1 peering's phases are unchanged. A v2 peering's partner
must select v2 too: a pair of one v1 and one v2 package is refused at phase 6
before any agreement is written. Nothing yet announces a next CA.

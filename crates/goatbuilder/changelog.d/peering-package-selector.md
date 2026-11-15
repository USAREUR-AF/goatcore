---
heading: For someone deploying
title: "Added: `peerings[].package` selects a peering's federation package version, v1 by default"
---

A peering in `goatbuilder.yaml` may now declare `package: v1` or `package: v2`
(ADR 1242). Absent, it is v1, as every peering has run until now, and any other
value is refused by `goatbuilder validate`. v2 adds the announcement of a next
CA, so a CA can rotate without a gap; both parties select it in reviewed
configuration before either emits it. With `package: v2`, phase 3
emits a v2 package and phase 6 composes the v2 pair (`goatbuilder federate init`
and `compose` take `--package-version v2`, and the selection reaches the phases
as FED_PACKAGE_VERSION). A v1 peering's phases are unchanged. A v2 peering's
partner must select v2 too: a pair of one v1 and one v2 package is refused at
phase 6 before any agreement is written. Nothing yet announces a next CA.

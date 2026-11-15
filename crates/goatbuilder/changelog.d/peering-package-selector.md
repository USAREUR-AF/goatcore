---
heading: For someone deploying
title: "Added: `peerings[].package` selects a peering's federation package version, v1 by default"
---

A peering in `goatbuilder.yaml` may now declare `package: v1` or `package: v2`
(ADR 1242). Absent, it is v1, as every peering has run until now, and any other
value is refused by `goatbuilder validate`. v2 adds the announcement of a next
CA, so a CA can rotate without a gap; both parties select it in reviewed
configuration before either emits it. This release reads the selection and
nothing yet acts on it: no peering emits or composes v2.

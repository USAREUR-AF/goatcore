---
heading: For someone deploying
title: "Changed: a release component's `release.signer` is a workflow with no ref, and its tag may hold a slash"
---

A release component's `release.signer` is the workflow that signs its
releases, `https://github.com/<owner>/<repository>/.github/workflows/<file>.yml`,
with no ref: the pin's tag is its ref, so one declaration serves every pin.
`goatbuilder pins` and `goat_consistency.resolve` refuse any other shape
(GB-365). A pin's tag may hold a slash (`tool/v1.2.0`).

Fetching and verifying a release component's file on the operator's machine
(new code GB-381), and the `release-file` role that installs it, records it in
the deploy ledger and rolls it back, are built. Neither deploy driver calls
them yet: both still refuse a release component before any host (GB-379).

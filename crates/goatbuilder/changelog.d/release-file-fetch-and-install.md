---
heading: For someone deploying
title: "Changed: a release component's `release.signer` is a workflow with no ref, and its tag may hold a slash"
---

A release component's `release.signer` is the workflow that signs its
releases, `https://github.com/<owner>/<repository>/.github/workflows/<file>.yml`,
with no ref: the pin's tag is its ref, so one declaration serves every pin.
`goatbuilder pins` and `goat_consistency.resolve` refuse any other shape
(GB-365). A pin's tag may hold a slash (`tool/v1.2.0`). A release pin's
repository must be its signer's repository, compared without regard to case,
and each file name in `release:` is one segment of `[A-Za-z0-9._+-]`, as the
role on the host requires.

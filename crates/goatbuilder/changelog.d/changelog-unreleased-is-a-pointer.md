---
heading: For someone contributing
title: Changed: a changelog entry is committed alone; the Unreleased section is one pointer line
---

A pull request that adds an entry in `crates/goatbuilder/changelog.d/` no
longer commits its render into `CHANGELOG.md`. The Unreleased section there is
one fixed line, and `changelog.py check` refuses anything else, so merging one
pull request no longer makes every other open one with an entry conflict.
`python3 ops/release/changelog.py show` prints the entries as the next
release will, and the `docs-guards` run puts that in its summary. A pull
request that only adds entry files merges at any base; one that changes
`CHANGELOG.md` itself (a release, or release notes) still has to be on a base
that holds every release.

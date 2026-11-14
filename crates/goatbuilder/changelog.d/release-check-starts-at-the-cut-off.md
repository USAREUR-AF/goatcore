---
heading: For someone contributing
title: Added: the release check starts at the cut-off, and the tag's tree check decides a rerun
---

The rules that hold while a release is cut (`docs/reference/goatbuilder/02-install.md`)
gain a fourth: the release check in the federation test environment starts on the version
commit at the cut-off, before the version pull request is opened. The tag goes on that pull
request's merge commit. Before the tag, a diff between the checked commit and the merge
commit must name nothing under `crates/`, `ops/ansible/` or `ops/federation-e2e/`, or each
stage those paths feed runs again on the merge commit.

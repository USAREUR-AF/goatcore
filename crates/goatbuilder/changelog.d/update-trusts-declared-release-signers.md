---
heading: For someone deploying
title: Fixed: goatbuilder update trusts the release signers it declares, so a repository move no longer strands an install
---

`goatbuilder update` used to trust only the repository that built the running
binary, or the one a checkout's `repo.release_source` named instead. A binary
built before the repository move therefore refused every later release.
From this release, goatbuilder carries a list of the release signers it
trusts, and checks a release against each of them. A checkout's
`repo.release_source` is trusted beside that list, never instead of it, and
an update run outside a checkout no longer stops with GB-078. A signature
must also name the signer's reusable release workflow at the release's tag
as its certificate's subject. **Changed:** `repo.source` no longer counts as
a release signer. A deployment that left `repo.release_source` out and
relied on `repo.source` to name its goatbuilder publisher names it in
`repo.release_source` instead, or its updates are refused with GB-261. When no signer
matches, GB-261 names each signer it checked and why each one refused. When
the signer changes in future, one release first trusts both the old signer and
the new one (ADR 1252). An install from before the move still needs one
reinstall: see GB-074. (#PR)

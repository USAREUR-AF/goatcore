---
heading: For someone deploying
title: Fixed: goatbuilder update trusts the release signers it declares, so a repository move no longer strands an install
---

`goatbuilder update` used to trust only the repository that built the running
binary, or the one a checkout's `repo.release_source` named instead. A binary
built before the repository move therefore refused every later release.
From this release, goatbuilder carries a list of the release signers it
trusts, and checks a release against each of them. A checkout's
`repo.release_source` (else `repo.source`) is trusted beside that list,
never instead of it, and
an update run outside a checkout no longer stops with GB-078. A signature
must also name the signer's reusable release workflow at the release's tag
as its certificate's subject, and, when the certificate carries them, its
GitHub workflow repository, ref and name: the fields a cosign reinstall line
pins. With `repo.release_source` omitted, `repo.source` is the addition, as
before. When no signer
matches, GB-261 names each signer it checked and why each one refused. When
the signer changes in future, one release first trusts both the old signer and
the new one (ADR 1252). An install from before the move still needs one
reinstall: see GB-074. (#PR)

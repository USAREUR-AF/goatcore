---
heading: For someone deploying
title: "Added: a release component whose checksums file is signed as a Sigstore bundle (`release.bundle: sigstore`)"
---

A release component may say `bundle: sigstore` in its `release:` declaration.
goatbuilder then verifies `<checksums>.sigstore.json`, a Sigstore bundle of
media type `application/vnd.dev.sigstore.bundle.v0.3+json`, and nothing else.
Without it, or with `bundle: cosign`, it verifies `<checksums>.cosign-bundle`
as before. Who must have signed, at which tag, is the same for both. The pins
choose the format; one is never tried after the other.

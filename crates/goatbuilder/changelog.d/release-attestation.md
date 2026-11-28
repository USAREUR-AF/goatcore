---
heading: For someone deploying
title: "Added: a release component whose files are attested, not its checksums file (`release.bundle: attestation`)"
---

A release component may say `bundle: attestation` in its `release:`
declaration. Its checksums file is fixed by the pin's digest and carries no
signature; each file is verified by its own GitHub artifact attestation (SLSA
provenance v1 in a DSSE envelope, Sigstore bundle v0.3), signed by the
component's `release.signer` at the pin's tag and naming the file and its
SHA-256. goatbuilder reads it from GitHub's attestations API, or as
`<file>.attestations.json` from a mirror (`GOATBUILDER_RELEASE_MIRROR`) or the
files an earlier run kept.

# Federation package v2 conformance vectors

Package v2 (ADR 1242) is version 1 with one member added: `identity.ca_fingerprint_next`. It is the
fingerprint of the CA that will issue the party's leaves next, present only during a planned CA
rotation. The agreement (`federation-agreement/v2`) copies it into that party as
`parties[n].ca_fingerprint_next` when present and leaves the member out otherwise. The text is
`docs/design/federation-package-v2-spec.md`.

Version 1's vectors in `../vectors/` are unchanged and stay v1's: they are run through the
version-pinned v1 entry point, and `refuse-both-v2` and `refuse-version-mismatch` there still
refuse a v2 package under v1.

| Directory | Kind | Expected |
|---|---|---|
| `compose-no-next/` | composing pair | the `compose-full` pair as v2, no next CA: the v1 agreement with `spec` `federation-agreement/v2` |
| `compose-next-one-side/` | composing pair | xaa announces a next CA; its party carries `ca_fingerprint_next`, xab's does not |
| `compose-next-both-sides/` | composing pair | both announce one; both parties carry it |
| `refuse-mixed-versions/` | refusal pair | one v1 package and one v2: `C_SPEC_MIXED` |
| `refuse-both-v3/` | refusal pair | a pair agreeing on a version this build does not implement: `C_SPEC_PIN` |
| `malformed/next-equals-current.json` | single package | `ca_fingerprint_next` equal to `ca_fingerprint`: `M_CA_NEXT_SAME` |
| `malformed/next-not-a-fingerprint.json` | single package | `ca_fingerprint_next` not in the fingerprint format: `M_CA_FINGERPRINT` |

The packages are `../vectors/compose-full`'s pair with `spec` changed and the member added, signed
with `../vectors/testkeys/`, which are public test keys. The next-CA fingerprints are the SHA-256 of
the text `xaa next CA (test vector)` and `xab next CA (test vector)`, so they name no real CA.
Canonical forms, signing inputs, signatures and digests follow §4, §5 and §8 of the specification.
They were computed by a standalone program written from that text (the directory step's own
canonical form and Ed25519 signer in `ops/federation/standup/directory.py`, a third implementation
apart from `crates/goat-federation` and the Go reference), not by the implementation they test.
The malformed packages are unsigned, as `../vectors/malformed/` explains.

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
| `refuse-downgrade-next-kept/` | refusal pair | xaa's v2 package relabelled v1 after signing, its next CA kept, beside a valid v1 partner: `C_SIGNATURE` on xaa |
| `refuse-downgrade-next-dropped/` | refusal pair | the same with the next CA removed: `C_SIGNATURE` on xaa |
| `refuse-relabel-v1-as-v2/` | refusal pair | xaa's v1 package relabelled v2 after signing, beside a valid v2 partner: `C_SIGNATURE` on xaa |
| `malformed/next-equals-current.json` | single package | `ca_fingerprint_next` equal to `ca_fingerprint`: `M_CA_NEXT_SAME` |
| `malformed/next-not-a-fingerprint.json` | single package | `ca_fingerprint_next` not in the fingerprint format: `M_CA_FINGERPRINT` |

The packages are `../vectors/compose-full`'s pair with `spec` changed and the member added, signed
with `../vectors/testkeys/`, which are public test keys. The next-CA fingerprints are the SHA-256 of
the text `xaa next CA (test vector)` and `xab next CA (test vector)`, so they name no real CA. The
relabelled packages keep the signature made before the relabelling: `spec` is inside the signed
bytes (§5: the signing input is the canonical form without `signature` only), so verification fails
and a package cannot change version after it is signed. The malformed packages are unsigned, as
`../vectors/malformed/` explains.

## Provenance

- **What generated the bytes.** `generate.py` in this directory wrote every byte here: the
  canonical forms, signing inputs, signatures, agreements and digests. It uses the canonical form
  and the Ed25519 signer of `ops/federation/standup/directory.py`.
- **When.** The composing, mixed, v3 and malformed vectors came in at commit `ca81dd5a`, the
  relabelled ones at `1d218c79`. Run again, `generate.py` writes the same bytes, because Ed25519
  signatures are deterministic.
- **What that shows.** `crates/goat-federation` was then tested against these files and
  reproduces every one of them. `directory.py` and `crates/goat-federation` are both this
  project's code, written by the same team. Their agreement shows the two are consistent with each
  other. It is not external validation: no implementation outside this project has computed these
  vectors.
- **Unlike v1.** Version 1's vectors were derived by hand from the text, and the Go reference
  implementation (`ops/federation/reference`) checks them independently. Nothing outside the Rust
  library composes v2 yet.

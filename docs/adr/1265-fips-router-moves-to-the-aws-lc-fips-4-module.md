# ADR 1265 — The FIPS Zenoh router moves to the AWS-LC FIPS 4.0.0 module, built from the source set its certificate names

| Field | Value |
|---|---|
| Status | Accepted (2026-11-26, operator: "yes do it all", said in answer to moving the FIPS module) |
| Date | 2026-11-26 |
| Repo scope | goat core (`ops/zenoh/pqc-image/`, `ops/ansible/roles/goat-zenoh/`) |
| Supersedes | — |
| Superseded by | — |
| Amends | [ADR 1079](1079-zenoh-pqc-hybrid-tls.md) (the module pin: AWS-LC FIPS 3.1.0 via aws-lc-fips-sys =0.13.11) |
| Related | [`zenoh-pqc-tls.md`](../design/zenoh-pqc-tls.md) §5, [`zenoh-version-pinning.md`](../design/zenoh-version-pinning.md), [ADR 1262](1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md) |

## TL;DR (for a layperson)

The FIPS version of the data-fabric router uses a government-validated cryptography module.
The module it used, AWS-LC FIPS 3.1.0, is tied to an old wrapper library, and that wrapper
blocks security fixes in the TLS library the router depends on. A newer module, AWS-LC FIPS
4.0.0, received its validation certificates on 19 November 2026. The router now builds on that
module, compiled from exactly the source files the certificate lists, and takes the current
TLS library with its fixes.

## Context

The FIPS router (`ops/zenoh/pqc-image/Dockerfile.fips`) reaches the module through
rustls `fips` → aws-lc-rs `fips` → aws-lc-fips-sys. ADR 1079 pinned aws-lc-fips-sys =0.13.11
because it vendors exactly the AWS-LC-FIPS-3.1.0 tag that CMVP #5298/#5314 validate. That pin
holds aws-lc-rs at 1.17.1, and rustls ≥ 0.23.44 and rustls-webpki ≥ 0.103.14 require
aws-lc-rs ^1.18, which requires aws-lc-fips-sys ^0.14. The FIPS build was therefore held at
rustls 0.23.43 / rustls-webpki 0.103.13, without the fixes for RUSTSEC-2026-0285,
RUSTSEC-2026-0346 to -0349 (rustls) and RUSTSEC-2026-0382 (rustls-webpki).

Facts, from primary sources, read 2026-11-26:

| Item | Fact | Source |
|---|---|---|
| aws-lc-fips-sys 0.13.11 | Vendors AWS-LC FIPS 3.1.0; its `crypto/fipsmodule/` is identical to tag `AWS-LC-FIPS-3.1.0` | crate source, [aws-lc tag](https://github.com/aws/aws-lc/tree/AWS-LC-FIPS-3.1.0) |
| AWS-LC FIPS 3.1.0 | CMVP [#5314](https://csrc.nist.gov/projects/cryptographic-module-validation-program/certificate/5314) (static, 2026-06-05) and [#5298](https://csrc.nist.gov/projects/cryptographic-module-validation-program/certificate/5298) (dynamic, 2026-06-03), Active | CMVP |
| AWS-LC FIPS 4.0.0 | CMVP [#5596](https://csrc.nist.gov/projects/cryptographic-module-validation-program/certificate/5596) (AWS-LC 4 Cryptographic Module, static) and [#5595](https://csrc.nist.gov/projects/cryptographic-module-validation-program/certificate/5595) (dynamic), validated 2026-11-19, sunset 2031-11-18, Active; ML-KEM-512/768/1024 KeyGen/Encaps/Decaps approved | CMVP; security policy [140sp5596.pdf](https://csrc.nist.gov/CSRC/media/projects/cryptographic-module-validation-program/documents/security-policies/140sp5596.pdf) |
| Validated 4.0.0 source set | Security policy §11.1: the archive of aws/aws-lc commit `1276fc7d9cd0f509000294e9657914c3a23e65d2`, SHA2-256 `33b5443d2213bfca45071b300daeb45decf13e3b46f726890b48d6267968f049`, built with `-DFIPS=1 -DCMAKE_BUILD_TYPE=RelWithDebInfo`, `bssl isfips` = 1, reports "AWS-LC FIPS 4.0.0"; "no additions, deletions, or alterations" | security policy; archive digest checked 2026-11-26 and matches |
| aws-lc-fips-sys 0.14.0 | Vendors a later 4.x snapshot (`base.h` says 4.1.0); its `crypto/fipsmodule/` differs from the validated set in `e_aesccm.c`, `dh.c`, `p_hkdf.c`, `p_hmac.c`, `p_kem.c`, `CMakeLists.txt` | crate source vs the archive |
| aws-lc-fips-sys 0.14.2–0.14.4 | Vendor AWS-LC FIPS 4.2.0 (branch `fips-2025-09-12-lts`); the same differences plus `cipher/cipher.c` | crate source; [aws-lc-rs v1.18.1 notes](https://github.com/aws/aws-lc-rs/releases/tag/v1.18.1) |
| aws-lc-rs pairing | 1.18.0 requires aws-lc-fips-sys ^0.14.0; 1.18.1, 1.18.2 and 1.19.0 require ^0.14.2 | crates.io |
| System-library path | aws-lc-fips-sys 0.14.x links a pre-built AWS-LC FIPS install through `AWS_LC_FIPS_SYS_SYSTEM_DIR` when its FIPS module version is ≥ 4, and checks it with `BORINGSSL_integrity_test()` | crate `builder/system_library.rs`, README |

No published aws-lc-fips-sys vendors the validated 4.0.0 source set. ADR 1079's rule is that
the module is the exact source the certificate names.

## Decision

1. **The FIPS router builds on AWS-LC FIPS 4.0.0, CMVP #5596 (static).** `Dockerfile.fips`
   downloads the archive of commit `1276fc7d…`, verifies the security policy's SHA2-256 digest,
   checks `AWSLC_VERSION_NUMBER_STRING` is `4.0.0`, builds with the policy's options (plus
   `BUILD_TESTING=OFF`, which skips test binaries only), requires `bssl isfips` = 1, and
   installs to `/opt/aws-lc-fips`. No file of the archive is changed.
2. **aws-lc-fips-sys links that install instead of its bundled copy.** aws-lc-fips-sys =0.14.4
   through `AWS_LC_FIPS_SYS_SYSTEM_DIR` (static), aws-lc-rs =1.19.0, both asserted in the
   lock. Rust bindings are generated by bindgen-cli 0.72.1 from
   `ops/zenoh/pqc-image/aws-lc-fips/rust_wrapper.h` (aws-lc-fips-sys 0.14.4's crypto header
   list), with the arguments AWS-LC's own `GENERATE_RUST_BINDINGS` option uses. Every header
   in that list exists in the validated set, and every AWS-LC symbol aws-lc-rs 1.19.0 and
   aws-lc-fips-sys 0.14.4 reference is declared there.
3. **The build proves which module is inside.** `zenohd` and `fips-probe` must each contain
   exactly one `AWS-LC FIPS <version>` string, `AWS-LC FIPS 4.0.0` (the module's show-version
   service, policy §11.1). A bundled aws-lc-fips-sys build would show 4.1.0 or 4.2.0 and fail.
   The existing gates stay: `try_fips_mode()` at zenohd start, fips-probe's X25519MLKEM768
   assertions at build, and the live `--connect` handshake in the CI smoke.
4. **rustls and rustls-webpki take their newest compatible releases, with floors.** The
   `RUSTLS_FIPS_HOLD` / `RUSTLS_WEBPKI_FIPS_HOLD` pins are removed. The image build runs
   `cargo update` on both (zenoh 1.10.2's lock carries rustls 0.23.45 and rustls-webpki
   0.103.15, below the fixed releases) and asserts rustls ≥ 0.23.46 and rustls-webpki
   ≥ 0.103.16; on 2026-11-26 that resolves to 0.23.46 and 0.103.17, the versions
   fips-probe's lock carries. bindgen, a build-dependency of aws-lc-fips-sys, is held at
   0.73.0, whose syn 3 / prettyplease 0.3 requirements are consistent with zenoh's lock.
5. **The module moves only by ADR.** `AWS_LC_FIPS_COMMIT`, `AWS_LC_FIPS_ZIP_SHA256`,
   `AWS_LC_FIPS_MODULE_VERSION`, `AWS_LC_FIPS_SYS_VERSION` and `AWS_LC_RS_VERSION` move
   together, to a newer CMVP-validated source set.
6. **Attestation names the module the image carries.** The goat-zenoh role writes
   `goat_zenoh_fips_module` into `fips-attest.json`, default
   `AWS-LC FIPS 4.0.0 (CMVP #5596/#5595)`; a goatnet pinned to an image built before this ADR
   sets `AWS-LC FIPS 3.1.0 (CMVP #5298/#5314)` until its image pin moves.

Permitted vocabulary is unchanged in kind: "built on the CMVP #5596-validated module". The
tested operational environments are Amazon Linux 2023 on EC2 (Xeon, Graviton4, EPYC); our
Debian-based image rides the same user-affirmation posture as before.

## Consequences

- The FIPS router takes the rustls and rustls-webpki fixes listed in Context.
- The libcrypto in the image is AWS-LC at commit `1276fc7d…`. AWS-LC advisories fixed after
  that commit are present in it: GHSA-65p9-r9h6-22vj (AES-CCM tag timing),
  GHSA-9f94-5g5w-gf6r (CRL distribution point scope), and AWS-LC's advisories of March 2026
  on X.509 name constraints and on `PKCS7_verify`. The router does not
  reach them: rustls's FIPS provider offers only AES-GCM suites (no CCM), and certificates are
  verified by rustls-webpki, not by AWS-LC's X.509 or PKCS7 code. 3.1.0 carried the first two
  under the same reasoning (accepted-risk entries in `ops/scripts/trivy-scan.py`, removed here
  because they named aws-lc-fips-sys 0.13.11).
- Dependency scanners read aws-lc-fips-sys 0.14.4 from the lock and cannot see the self-built
  module. Its advisories are tracked by reading AWS-LC's advisories against commit
  `1276fc7d…` at each module move.
- The image build adds a bindgen-cli install and an AWS-LC build to the builder stage.

## Alternatives considered

- **aws-lc-fips-sys 0.14.4 with its bundled source (AWS-LC FIPS 4.2.0).** Simplest build, but
  its module source differs from the validated set in seven files under `crypto/fipsmodule/`
  and reports 4.2.0, not the certificate's 4.0.0. Rejected: the certificate names a source set.
- **aws-lc-fips-sys 0.14.0 with its bundled source.** Closest crate, still six module files
  and the version string away from the validated set, and pairs only with aws-lc-rs 1.18.0.
  Rejected for the same reason.
- **Keep 3.1.0 and the rustls hold.** Leaves the rustls-webpki and rustls advisories in the
  FIPS router. Rejected now that a validated module that takes current rustls exists.
- **Backport the rustls fixes onto 0.23.43.** A private rustls fork to carry and re-verify on
  every advisory. Rejected.

The grant-crypto SDK (ADR 1106): the Rust core behind a versioned C ABI, and the Go, Python and TypeScript bindings that link against it.

**Assets.** `grant-crypto-sdk-<ver>-src.tar.gz`, the SDK workspace with the wire spec and the shared conformance vectors (its tests run from the unpacked tarball); `grant-crypto-sdk-docs-<ver>.tar.gz`, the SDK's documentation site; `grant-crypto-capi-<os>-<arch>.tar.gz` for `linux-amd64`, `linux-arm64`, `darwin-amd64` and `darwin-arm64`, each holding the shared library, the static library, `include/grant_crypto.h` and `INSTALL.txt`; a `.sha256` for each tarball; `SHA256SUMS` over all of them; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow.

**Download and verify.** Replace `<tag>` with this release's tag and `<os>-<arch>` with your platform.

```bash
base=https://github.com/USAREUR-AF/goatcore/releases/download/<tag>
curl -fL --remote-name-all "$base/grant-crypto-capi-<os>-<arch>.tar.gz" "$base/SHA256SUMS" "$base/SHA256SUMS.cosign-bundle"
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS        # on macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
tar xzf grant-crypto-capi-<os>-<arch>.tar.gz
```

The cosign step is optional and needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/); the checksum step is not.

**macOS.** The dylib is ad-hoc signed and not notarized.

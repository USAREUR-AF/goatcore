`goatbuilder`, the build, release, deploy and standup tool for a goatnet.

**Assets.** `goatbuilder-linux-amd64`, `goatbuilder-linux-arm64` (both run on glibc 2.31 and later: Ubuntu 20.04, Debian 11 and later), `goatbuilder-darwin-amd64` (Intel), `goatbuilder-darwin-arm64` (Apple Silicon), each with a `.sha256`; `SHA256SUMS` over all four; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow.

**Download, verify, install.** Replace `<tag>` with this release's tag and `<os>-<arch>` with `linux-amd64`, `linux-arm64`, `darwin-amd64` or `darwin-arm64`.

```bash
base=https://github.com/USAREUR-AF/goatcore/releases/download/<tag>
curl -fL --remote-name-all "$base/goatbuilder-<os>-<arch>" "$base/SHA256SUMS" "$base/SHA256SUMS.cosign-bundle"
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS        # on macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
install -m 0755 goatbuilder-<os>-<arch> ~/.local/bin/goatbuilder
goatbuilder --version
```

The cosign step is optional and needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/); the checksum step is not.

**macOS.** The binary is ad-hoc signed and not notarized. A copy downloaded with a browser carries the quarantine attribute and macOS refuses to open it; clear it once after the checksum passes: `xattr -d com.apple.quarantine ~/.local/bin/goatbuilder`. A copy fetched with `curl` has no quarantine attribute.

**Windows.** No Windows build; run goatbuilder from Linux, macOS or WSL.

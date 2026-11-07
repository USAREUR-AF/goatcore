wg-cp0, the operator command for the WireGuard control-plane substrate (ADR 0322): `validate`, `list`, `show` and `diff` read the declared peer set and its difference from the live one; `reconcile`, `onboard`, `rotate`, `remove`, `resurrect`, `add-relay`, `update-endpoints` and `update-ttl` change it.

**Assets.** `wg-cp0-<os>-<arch>` for `linux`, `darwin` and `windows` (`.exe`), each on `amd64` and `arm64`, built with CGO off; a `.sha256` for each; `SHA256SUMS` over all of them; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow.

**Download and verify.** Replace `<tag>` with this release's tag and `<os>-<arch>` with your platform.

```bash
base=https://github.com/USAREUR-AF/goatcore/releases/download/<tag>
curl -fL --remote-name-all "$base/wg-cp0-<os>-<arch>" "$base/SHA256SUMS" "$base/SHA256SUMS.cosign-bundle"
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS        # on macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
chmod +x wg-cp0-<os>-<arch>
```

The cosign step is optional and needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/); the checksum step is not.

**macOS.** The binary is not signed or notarized; clear the quarantine attribute with `xattr -d com.apple.quarantine wg-cp0-darwin-<arch>` before the first run.

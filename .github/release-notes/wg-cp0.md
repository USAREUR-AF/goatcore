wg-cp0, the operator command for the WireGuard control-plane substrate (ADR 0322): `validate`, `list`, `show` and `diff` read the declared peer set and its difference from the live one; `reconcile`, `onboard`, `rotate`, `remove`, `resurrect`, `add-relay`, `update-endpoints` and `update-ttl` change it.

**Assets.** `wg-cp0-<os>-<arch>` for `linux`, `darwin` and `windows` (`.exe`), each on `amd64` and `arm64`, built with CGO off; a `.sha256` for each; `SHA256SUMS` over all of them; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow; `<file>.sigstore.json` for each binary, a keyless cosign signature over that file; `wg-cp0.intoto.jsonl`, the SLSA build provenance of every binary.

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

**Check one file by its own signature, or by its build provenance.** Each binary and archive also has a signature of its own, `<file>.sigstore.json`, and the release has one provenance file, `wg-cp0.intoto.jsonl`, that names every binary and archive by digest and the workflow run that built it. Both checks are optional: the cosign and checksum steps above already cover the file.

```bash
curl -fL --remote-name-all "$base/wg-cp0-<os>-<arch>.sigstore.json" "$base/wg-cp0.intoto.jsonl"
cosign verify-blob --new-bundle-format --bundle wg-cp0-<os>-<arch>.sigstore.json \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com wg-cp0-<os>-<arch>
gh attestation verify wg-cp0-<os>-<arch> --bundle wg-cp0.intoto.jsonl --repo USAREUR-AF/goatcore \
  --signer-workflow USAREUR-AF/goatcore/.github/workflows/wg-cp0-release.yml
```

The first command needs cosign 2.4 or later; the second needs the [GitHub CLI](https://cli.github.com/).

The cosign step is optional and needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/); the checksum step is not.

**macOS.** The binary is not signed or notarized; clear the quarantine attribute with `xattr -d com.apple.quarantine wg-cp0-darwin-<arch>` before the first run.

`goat`, the operator CLI: health, people, topics, publish and subscribe on a goatnet.

**Assets.** `goat-linux-amd64`, `goat-linux-arm64` (both run on glibc 2.31 and later: Ubuntu 20.04, Debian 11 and later), `goat-darwin-amd64` (Intel), `goat-darwin-arm64` (Apple Silicon), `goat-windows-amd64.exe`, each with a `.sha256`; `goat-docs.tar.gz`, the field guide built from this tag (the site the goat-docs role serves), with its `.sha256`; `SHA256SUMS` over all six; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow.

**Download, verify, install.** Replace `<tag>` with this release's tag and `<os>-<arch>` with `linux-amd64`, `linux-arm64`, `darwin-amd64` or `darwin-arm64`; Windows is below.

```bash
base=https://github.com/USAREUR-AF/goatcore/releases/download/<tag>
curl -fL --remote-name-all "$base/goat-<os>-<arch>" "$base/SHA256SUMS" "$base/SHA256SUMS.cosign-bundle"
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS        # on macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
install -m 0755 goat-<os>-<arch> ~/.local/bin/goat
goat --version
```

The cosign step is optional and needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/); the checksum step is not.

**macOS.** The binary is ad-hoc signed and not notarized. A copy downloaded with a browser carries the quarantine attribute and macOS refuses to open it; clear it once after the checksum passes: `xattr -d com.apple.quarantine ~/.local/bin/goat`. A copy fetched with `curl` has no quarantine attribute.

**Windows.** In PowerShell:

```powershell
$base = "https://github.com/USAREUR-AF/goatcore/releases/download/<tag>"
iwr "$base/goat-windows-amd64.exe" -OutFile goat.exe
iwr "$base/goat-windows-amd64.exe.sha256" -OutFile goat.exe.sha256
(Get-FileHash goat.exe -Algorithm SHA256).Hash.ToLower() -eq (Get-Content goat.exe.sha256).Split(' ')[0]   # must print True
.\goat.exe --version
```

Move `goat.exe` to a folder on `PATH`. `goat client`, which talks to the local client daemon, is not available on Windows; every other command is.

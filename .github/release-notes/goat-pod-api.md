goat-pod-api, the pod-side carriage runtime (ADR 1251): it pulls and verifies the pod's signed rule generation, renders the pod router's access control with local rules, applies it (a failed addition rolls back to the last known good configuration; a failed removal never restores a removed source), keeps the signed local-rule ledger, and serves the pod's local API and the Fieldkit Gatepost page. Operator page: `docs/operations/goat-pod-api.md`.

**Assets.** `goat-pod-api-linux-amd64` and `goat-pod-api-linux-arm64`, static (CGO off); `gatepost.html`; a `.sha256` for each; `SHA256SUMS` over all of them; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's `goat-pod-api-release.yml` workflow; `<file>.sigstore.json` for each asset; `goat-pod-api.intoto.jsonl`, the SLSA build provenance of every asset.

**Install on a pod.** The pod repository's first-boot does this when its profile sets `POD_API_SIGNER_CERT` (zagros-moon-pod `host/install-goat-pod-api.sh`, release pinned in `host/goat-pod-api.pin`). By hand, replace `<tag>` with this release's tag and `<arch>` with `amd64` or `arm64`:

```bash
base=https://github.com/USAREUR-AF/goatcore/releases/download/<tag>
curl -fL --remote-name-all "$base/goat-pod-api-linux-<arch>" "$base/gatepost.html" "$base/SHA256SUMS" "$base/SHA256SUMS.cosign-bundle"
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github.com/USAREUR-AF/goatcore/\.github/workflows/goat-pod-api-release\.yml@refs/tags/goat-pod-api/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
sudo install -m 0755 goat-pod-api-linux-<arch> /usr/local/bin/goat-pod-api
sudo install -D -m 0644 gatepost.html /usr/local/share/goat-pod-api/gatepost.html
```

The signature check needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) 2.4 or later.

`goatbuilder`, the build, release, deploy and standup tool for a goatnet.

**New in 0.5.2:** the federation documents are corrected against the code, and GB-129 now says that a narrower `offer:` is refused only for a hand-written federation.toml. `status` reads the hosts themselves. Declared paths and profiles reach standup's phases as one word each. When the edge and the core are on separate hosts, the edge's ingress check now reads the core, and a check that cannot run stops phase 12 (a behaviour change). An interrupted phase 8 no longer leaves private keys in its staging directory. `validate` names GB-025 on every failing row, and `release` prints commits at a fixed 12 characters. CI no longer depends on the Ubuntu archive: when a runner cannot reach it, the jobs fail over to the project's own package cache.

**New in 0.5.0 and 0.5.1: federation is in goatbuilder.** (0.5.0 was tagged and not published; 0.5.1 is the same goatbuilder and the first published release that carries it.) You federate with a partner using goatbuilder alone. You make one decision (the partner, and the release sets it receives) and answer the run's prompts at phases 5, 6 and 10: the partner's key and CA fingerprint, the agreement digest, then the plan and what your core admits. The full list is in `crates/goatbuilder/CHANGELOG.md`.

- `goatbuilder federate <partner>`: the whole procedure in one run. At a terminal it asks for the partner's key and CA fingerprint and for the agreement digest. It makes the plan and asks you to approve it, shows what our core will admit and asks again, then carries on through acceptance. A run that stops resumes where it stopped.
- `goatbuilder federate plan <partner>`, `deploy <partner>`, `test <partner>`, `status [<partner>]` and `backout <partner>`: each stage on its own. `backout --dry-run` surveys what backing one peering out would remove.
- `goatbuilder federate allocate [<peering>]`: allocates one peering's values and renders its inventory, both deployment halves and its `federation.toml`. It runs beside peerings that are still written by hand.
- Admission on a core deployed by the goat-zenoh role: the role's own play, org fragment, router leaf and `zenoh_sovereign_peerings`. No restart command needs to be typed.
- Values come from the declarations the deployment already has:
  - `secrets.federation` for the signing key and CA;
  - `environments[].peering_directory` for the directory and its CA;
  - the site inventory for the peering's machines.

  `FED_*` variables stay as overrides.
- `goatbuilder federate init|lint|compose|diff|verify|apply` are the package verbs `goat federate` had, with the same flags and exit codes.
- **Deprecated: `goat federate`.** It still works in 0.5.0 and tells you the goatbuilder command to run instead. It will be removed from `goat` 30 days after this release, and not before one peering between two standing environments has been completed with `goatbuilder federate`. After that, `goat federate` only answers with an error that names `goatbuilder federate`. The script `ops/scripts/peering-standup.sh` stays as it is: documented, not supported.

**Assets.** `goatbuilder-linux-amd64`, `goatbuilder-linux-arm64` (both run on glibc 2.31 and later: Ubuntu 20.04, Debian 11 and later), `goatbuilder-darwin-amd64` (Intel), `goatbuilder-darwin-arm64` (Apple Silicon), each with a `.sha256`; `goatbuilder-docs.tar.gz`, the goatbuilder book built from this tag, with its `.sha256`; `SHA256SUMS` over all five; `SHA256SUMS.cosign-bundle`, a keyless cosign signature over `SHA256SUMS` made by this repository's release workflow.

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

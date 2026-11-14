---
heading: For someone deploying
title: "Added: `goatbuilder update verify-release` verifies a downloaded goatbuilder release without installing it"
---

`goatbuilder update verify-release <dir> --signer <owner>/<repo> --version
<x.y.z> [--trusted-root <file>]` verifies a downloaded goatbuilder release
without installing it, with the checks `goatbuilder update` makes before it
installs: the keyless signature over `SHA256SUMS` (`SHA256SUMS.cosign-bundle`)
for the release workflow of `<owner>/<repo>` at `goatbuilder/v<version>`, and
every other file in `<dir>` against that `SHA256SUMS`. It reads only. A
refusal names the failed check: GB-261 for the signature, GB-074 for a file.

---
heading: For someone deploying
title: "Added: a pins component may ship as a release file (`artifact_type: release`)"
---

`ops/pins/fleet-pins.yaml` takes a component that ships as a file, not an image
(gate J23). It declares `artifact_type: release` and a `release:` mapping: the
release's checksums file, the file for each architecture (`amd64`, `arm64`) and
the signer its signature or attestation must name. Its pin is
`<owner>/<repository>@<tag>@sha256:<digest of the checksums file>`. `goatbuilder
pins` checks the declaration and every pin (GB-365). Nothing changes for an
image component.

---
heading: For someone deploying
title: "Added: `deploy`, `verify` and `rollback` work on a release component"
---

A component that ships as a file (`artifact_type: release`) is fetched and
verified on the operator's machine before any host is touched: the checksums
file's digest, its signature by the declared workflow at the pin's tag, and each
architecture's file (GB-381). `goatbuilder deploy` and
`ops/scripts/deploy-component.sh` then install the host's own architecture's
file with the `release-file` role, and refuse in the same words. The script
needs goatbuilder on PATH, or `GOATBUILDER_BIN`.

`deploy --dry-run` fetches and verifies too, prints the whole plan and exits 1
when a real run would refuse. `verify` also reads the file each host runs and
requires the pin's. `rollback` switches every host of one goatnet back to the
file it ran before, with no fetch, one step (GB-379 for more). Verified files
are kept under the state directory; `GOATBUILDER_RELEASE_MIRROR` names a
directory to read instead of GitHub.

A deploy that follows a rollback to the same ref now writes its own ledger row,
for an image component too. Before, the host kept reading as rolled back.
`status`, `diff` and `survey` now list release components.

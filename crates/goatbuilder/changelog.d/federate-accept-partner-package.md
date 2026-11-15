---
heading: For someone deploying
title: "Added: a partner's new package is taken at phase 5 once its digest is compared, and `rotate <partner> transport` runs"
---

When a partner publishes a new package signed by the key already bound (after
rotating its WireGuard key, for example), the directory step saves it at
`<work>/theirs.incoming.json` and places nothing. Phase 5 of
`goatbuilder federate <partner>` shows which members changed and the new
package's digest, and places it only once a person confirms the digest compared
on the call: `yes` at a terminal, or `--accept-partner-package <digest>`.
`--yes` never confirms it. The old package is kept as
`theirs.canonical.json.previous-<UTC time>`. A package not signed by the bound
key is refused (GB-191) and set aside. A package issued before the one held is
refused (GB-308) even with its digest given, so a replay is never asked about.
This follows ADR 1229, which is proposed.

`goatbuilder federate rotate <partner> transport` now replaces our WireGuard
peering key from phase 2. The tunnel is down until the partner runs
`goatbuilder federate <us> --from 5 --accept-partner-package <digest>` with the
digest compared on the call. Neither is measured on a running peering.

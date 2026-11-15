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
`--yes` never confirms it. A scripted run that gives only the keys
(`goatbuilder federate <partner> --partner-key <key> --partner-ca-fingerprint <fp>`)
now stops at phase 5 with GB-192 and exit 0 when a new package waits, so a script
cannot tell that stop from a pass by its exit code; it reads the stop line. The old package is kept as a copy,
`theirs.canonical.json.previous-<UTC time>`, and the place is never empty; if the
folder cannot be synced after the new package is placed, the run says the new one
is in place, not confirmed, and the old one is at the copy (GB-184). A
package not signed by the bound key, one the parser refuses, or one issued more
than 7 days after today (UTC) is refused (GB-191) and set aside, and the
directory step does not save one issued that far ahead; one that came
through the partner's single pointer is asked about saying so. A package that names a
party other than the peering's partner (`peerings: with`) is refused (GB-191)
before its signature is checked, placed by hand or offered by the directory, and
the directory step does not save one for phase 5. The directory step run by hand
(`ops/federation/standup/directory.py`) without `--their-id` now offers no new
package at all, since it has no partner to check one against; goatbuilder always
passes it. A package issued before the one held, or without a
readable issued date while the held one has one, is refused (GB-308) even with
its digest given, so a replay is never asked about. When the keys confirmed before are
unchanged, the new package's digest is the only question: phase 5 carries on
without asking the keys again, and phase 6 does not ask an agreement digest
already compared with the partner. Before each phase after 5, whatever phase a run
starts from, the package on disk must be the last one confirmed, with the keys
confirmed for it (GB-191, naming phase 5). `goatbuilder federate compose` run as a verb
applies the same check to a package in a peering's working folder: one that is
not the package confirmed at phase 5 there is refused (GB-191, exit 1, the
contract's "refused: a precondition failed"); a package outside a working
folder composes as before. A peering confirmed by an earlier build has
its held package recorded as confirmed on its first run, as a migration, and the
run prints that package's digest to compare with the partner at the next call; one
that cannot be read stops every run that executes a phase (deploy, test, a resume)
with GB-310, exit 1. This follows ADR 1229, which is
accepted.

`goatbuilder federate rotate <partner> transport` now replaces our WireGuard
peering key from phase 2, and carries our run past phase 5 and phase 6 when the
keys and the agreement are as confirmed before; any change to either stops at phase 5. The tunnel is down until the partner runs
`goatbuilder federate <us> --from 5 --accept-partner-package <digest>` with the
digest compared on the call. Neither is measured on a running peering.

# One peering between two standing goatnets, measured

**What this is.** The procedure for goatbuilder 1.0 gate row B9: federate two goatnets that are
already running, outside the test laboratory, with the released `goatbuilder` binary, and keep
both sides' acceptance reports as the measurement. Two operators do it together, one on each
goatnet, in about the time a call takes. The full procedure and what each phase does are in
[sovereign-federation-runbook.md](sovereign-federation-runbook.md); this page is the order of
acts and what to keep.

Status: WIP — gate: one peering between two standing goatnets, set up with the released binary,
with both sides' acceptance reports kept (goatbuilder 1.0 gate row B9).

The examples call the two goatnets `xaa` and `xab`. Each operator runs everything below on their
own side, with `<us>` their own id and `<partner>` the other's. `<work>` is the peering's working
directory: `--work` if you give it, else `$FED_WORK`, else `/var/lib/goat-federation/<partner>`.

## Before the call (each side, alone)

0. **Check with the partner, before anything else:**
   - **Both sides run goatcore at or after the stage 2 commit** (`feafbd3b`, which carries
     `ops/federation/standup/acceptance_session.py`). The commit that counts is the goatcore
     commit the deployment pins (its overlay's `goatcore.ref`), never the overlay's own commit.
     Run the check in the goatcore checkout at that pin, not in the overlay:

     ```bash
     git -C <goatcore checkout> merge-base --is-ancestor feafbd3b HEAD && echo ok
     ```

     The bilateral acceptance session (step 6) needs it on both sides. If the partner's
     goatcore is older, the session cannot run: your local acceptance result alone ends with
     exit 3, and B9 is not met. Record that as the outcome rather than a measurement.
   - **How the packages are exchanged.** Through the peering directory, or by hand. Use the
     hand exchange when either side cannot reach a directory, and when either side already runs a
     peering through a directory that holds one package per party: publishing a second package
     would replace the first, until one package per counterparty is the default (gate row B6).
     For the hand exchange, set `goat_federation_no_directory: true` in the edge group's
     group_vars (or `FED_NO_DIRECTORY=1`); phase 5 then names the files to send and where to put
     the partner's.
   - **Other peerings on your core.** If your core already carries peerings with other partners,
     phase 10 restarts your core router, which drops their sessions while it restarts. Tell those
     partners when the window is, and check them before and after (step 5).

1. **The released binary.** Install the published release as its release notes say (download,
   verify `SHA256SUMS` and its signature, install), then record its version:

   ```bash
   goatbuilder --version
   ```

   Both sides use a published release, not a build from a checkout.

2. **The declarations**, in your goatnet's own repository, reviewed and merged like any other
   change:
   - the peering in `goatbuilder.yaml` `peerings:` (`with: <partner>`, `environment`, `site`,
     `edge_class`), with `offer:` only if this partner receives fewer release sets than
     `federation.toml` offers;
   - `secrets.federation` (the paths of your signing key, CA certificate and CA key) and
     `environments[].peering_directory` (the directory's `url`, and `ca_cert` if its certificate
     is not a public one);
   - the edge and core in your site inventory, if the peering does not name `hosts:`;
   - if your core is deployed by the goat-zenoh role and already admits other partners, each
     `zenoh_sovereign_peerings` entry's `sovereign` written as a literal id, not a template.
     Admission finds a partner's existing entry by that literal value; an entry whose
     `sovereign` is templated is not found, and a second entry for the same partner is added.

3. **The per-peering files.** Write them, read what changed, and commit it:

   ```bash
   goatbuilder federate allocate <partner> --dry-run
   goatbuilder federate allocate <partner>
   ```

4. **The dry run**, to read every phase and command before anything changes:

   ```bash
   goatbuilder federate <partner> --env <env> --dry-run
   ```

## On the call (both sides, at the same time)

5. **Run the command at a terminal**, each on your own side:

   ```bash
   ops/scripts/with-secrets goatbuilder federate <partner> --env <env>
   ```

   If your core already carries other peerings, stop before phase 10 instead and check them
   first. `FED_EXISTING_WORKS` names their working directories, separated by `:`:

   ```bash
   ops/scripts/with-secrets goatbuilder federate <partner> --env <env> --to 9
   export FED_EXISTING_WORKS=/var/lib/goat-federation/<other partner>[:...]
   ops/federation/standup/acceptance.sh <work>/agreement.json --before
   ops/scripts/with-secrets goatbuilder federate <partner> --env <env>
   ```

   The `--before` run keeps the other peerings' state in `<work>/evidence/acceptance/before/`.
   With `FED_EXISTING_WORKS` still set, phase 14 and the session's `test` run check them again
   and compare ([federation-acceptance.md §2.3](../design/federation-acceptance.md#23-existing-partnerships-before-and-after)).

   It stops and asks at three points. Each is a decision; answer only what you have checked
   with the other operator on the call.

   | Phase | It asks | You check |
   |---|---|---|
   | 5 | type the verifying key the partner reads aloud; confirm they read their CA fingerprint | the key and fingerprint they read, not the ones the directory delivered. If their package is not in the directory yet, the run says so: run the same command again once they have run theirs |
   | 6 | is the agreement digest the same on both sides | read yours aloud; theirs must match digit for digit |
   | 10 | approve the plan's hash; then approve what your core admits | the plan's changes, and the access fragment, certificates and files the summary lists. Typing `pr` writes the summary for review instead, and stops |

   After phase 10 the run carries on through phase 14 by itself, and **phase 14 is reported as
   failed: this is expected.** Phase 14 can observe only your own side, so the checks that need
   the partner's side read BLOCKED and acceptance exits 3. goatbuilder then prints
   `!! Phase 14 (...) FAILED — exit 3`, records phase 14 as failed in the journal (GB-183) and
   exits 1. Phases 1 to 13 are done, and nothing needs resuming: step 6 proves the partner's half.

6. **The acceptance session.** One operator (say `xaa`) writes the session manifest and sends it
   to the other by any means; both read its digest aloud and compare:

   ```bash
   S=ops/federation/standup/acceptance_session.py
   python3 $S new --agreement <work>/agreement.json --us <us> --out <work>/session.json   # one side
   python3 $S digest <work>/session.json                                                   # both sides
   ```

   The receiving side saves the copy it was sent as its own `<work>/session.json`. Give the
   manifest as an absolute path from here on: goatbuilder runs acceptance from the repository
   root, not from the directory you are in.

   Then each side, in the same window:

   ```bash
   FED_ACCEPTANCE_SESSION=<work>/session.json goatbuilder federate test <partner> --env <env>
   ```

   **This run, too, is reported as failed on both sides, even when everything works.** In a
   session, the checks of what your side sent (its delivery and persistence on the partner's
   core) can only be observed by the partner, so they read BLOCKED here, acceptance exits 3, and
   goatbuilder prints `!! Phase 14 (...) FAILED — exit 3` and exits 1. Do not stop there: go on to
   `import` and `combine`. Only `combine` decides whether the peering is proven.

   Exchange your `<work>/evidence/acceptance/result.json` files, and on each side:

   ```bash
   python3 $S import --manifest <work>/session.json --us <us> --evidence <work>/evidence/acceptance <their result.json>
   python3 $S combine --manifest <work>/session.json --us <us> --evidence <work>/evidence/acceptance
   ```

   `combine` exits 0 only when the required checks of both sides passed. Exit 1 is a failure to
   fix before going on; exit 3 means a required check did not run. Either way the peering is not
   proven, and B9 is not measured.

## What to keep as the measurement

Keep these from **each** side, in your goatnet's own private records (they name your hosts and
identities, so they do not go into a public repository):

| What | Where |
|---|---|
| the goatbuilder version and the goatcore commit | `goatbuilder --version`; `git -C <goatcore checkout> rev-parse HEAD`, in the goatcore checkout at the deployment's pin. Never the overlay's commit: it belongs to a private repository and does not go in a public row |
| the plan and its hash | `<work>/plan.json`, and the hash the phase 10 prompt showed |
| the agreement digest, and the session manifest's digest | from the phase 6 prompt; `acceptance_session.py digest <work>/session.json` |
| your own acceptance result, observed locally | `<work>/evidence/acceptance/result.json` and `summary.md`, and its run id (`run.id`) |
| the partner's result, as imported | `<work>/evidence/acceptance/peer/<partner>/`, which `import` marks peer-reported and unauthenticated: it is what the partner reports, not something you observed |
| the combined bilateral result | `<work>/evidence/acceptance/bilateral/<session id>.json` and `.md` |
| the existing-partnership checks, before and after | `<work>/evidence/acceptance/before/`, and the existing-partnership rows of the result above (only when the core carries other peerings) |
| the run journal, and what `status` says afterwards | `<work>/state.json`; `goatbuilder federate status <partner> -o json` |

What goes into the public gate row B9 is only: both acceptance run ids, the session id, the
combined result's verdict, the agreement digest, and the goatbuilder version and goatcore commit.

## After the measurement

Publishing `exchanging` is a separate act, a reviewed change to the partner file by both
operators (`acceptance_session.py propose`; the runbook, step 1, `test`). B9 does not need it. If
acceptance fails and the cause cannot be fixed on the call, back the peering out on each side and
leave every other peering as it was:

```bash
goatbuilder federate backout <partner> --env <env> --dry-run
goatbuilder federate backout <partner> --env <env>
```

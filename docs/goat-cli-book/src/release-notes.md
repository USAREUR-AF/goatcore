# Release notes

What changed in each `goat` release, newest first. To see which version you're
running: `goat --version`. To get the latest, see [Install & verify](install.md).

> **A note on languages.** These release notes are kept in English. The rest of
> this guide is translated into every supported language, refreshed on larger
> releases — a small release ships its notes here in English first, and the full
> translated guide catches up on the next sizeable release. So if you're reading
> a translated page and this one is in English, that's expected, not a gap.

## v0.5.10

- **A goatnet's membership declaration is now a directory, not one file.**
  `ops/warrant/membership/<scope>/` holds `_scope.yaml` (the scope and its
  `converge: report|enforce`), any number of `*.yaml` files of entries (for
  example one per organisation under `orgs/`), and a generated
  `_baseline.generated.yaml`. `goat warrant apply --membership --file
  ops/warrant/membership/<scope>` (or `--dir`) reads them all as one list, and
  refuses two entries for the same peer in different files, naming both. The
  old single `<scope>.yaml` file is refused with a message pointing at the new
  layout, and a goatnet that still has one is treated as governed, so the old
  commands do not write membership directly.
- **A person's devices start from a declared baseline.** When a peer's name
  says whose device it is and that person is in `_baseline.generated.yaml`, the
  converger starts from that person's `device` groups, or their `handset`
  groups for a handset. Other peers start from their enrolment key, as before.
  Each row of `--json` says which one was used (`baseline_source`: `declared`,
  `key` or `none`), and `--textfile-dir` adds
  `goat_warrant_membership_baseline_declared_total`.
- **The commands that write entries put them in the right file.** `goat admin
  members move --write-entry` writes a new entry to `holds.yaml`,
  `services.yaml` or `people.yaml` by kind, and edits a peer's existing entry
  in whichever file already holds it. `goat admin platform-admin grant|revoke`
  gains `--write-entry`, which does the same for the mesh leg's entries.

## v0.5.9

- **The membership converger can be paused, with a reason and an end time.**
  On a goatnet whose membership is declared in git, `goat warrant pause --scope
  <scope> --for 1h --reason "..."`, run on the management host, holds the
  converger in report mode so an emergency edit is not put back. It lasts at
  most 24 hours, ends on its own, and is recorded as an operator action. `goat
  warrant resume --scope <scope>` ends it early. While it is in force, `goat
  warrant apply --membership` says who paused it, until when and why, and
  `--json` carries a `paused` field. A pause file that cannot be read is
  ignored with a warning, so a broken pause never stops the converger.
- **A membership file that says `converge: report` no longer blocks the old
  commands.** Only a file that says `converge: enforce` hands a goatnet's
  membership to warrant. With `converge: report` the file is a draft: `goat
  warrant apply --membership` reports against it, and `goat admin members move
  --apply` and `goat admin platform-admin` change groups directly, as they do on
  a goatnet with no file.

## v0.5.8

- **Peer membership can be declared in git and converged.** `goat warrant apply
  --membership --scope <scope> --file ops/warrant/membership/<scope>.yaml`
  compares each peer's groups with the declaration and reports the difference.
  With `--mode enforce`, it also makes the changes, but only when the file
  itself says `converge: enforce`. It stops without changing anything if a run
  would make more than `--max-changes` changes (default 10). It never changes a
  peer whose enrolment key it cannot find. `--json` and `--textfile-dir` give
  the same result to scripts and to Prometheus.
- **On a goatnet whose membership is declared in git, the commands that used
  to change groups directly now print the change instead.** `goat admin members
  move --apply` refuses and prints the entry. `--write-entry` adds the entry to
  the membership file for you to put in a pull request. `goat admin
  platform-admin grant|revoke` prints the mesh-group entry and still changes
  every other system as before. On every other goatnet both commands work
  exactly as they did.

## v0.5.7

- **A mesh key that expired can be reissued.** When your setup key runs out of
  uses or passes its expiry date, an operator runs
  `goat admin onboard <you> --env <goatnet> --reissue-device-key --uses <N>`,
  or clicks **Reissue device key** in the onboarding console. Your old key
  stops working, a new one with N enrolments is issued, and the welcome package
  is sent again with it. Devices already on the goatnet stay on — a setup key
  governs new enrolments only.
- **Every key reissue says who asked for it.** Both this command and
  `--reissue-handset-key` now write an operator-action record naming the
  operator, the person the key is for, the key, its reuse count and its
  lifetime — never the key itself.

## v0.5.5

- **One command grants or revokes platform admin.** `goat admin platform-admin
  status|grant|revoke <email> --env <goatnet>` covers every system that carries
  the role — the Ory login group, the NetBird dashboard role, the person's
  peers' mesh groups, host SSH keys, their operator key, their right to sponsor
  people, and the 1Password Platform group. It shows what it will change,
  changes nothing without `--apply`, and reads every system back afterwards.
  `revoke` leaves the person a developer; `--full` offboards them. Sponsorship
  and the operator key follow the level the person ends up at, so a demotion
  leaves them with what everyone else at that level has.
- **`goat access strays` no longer names people the portal onboarded** as
  strays.
- **A signing root that is expired or not yet valid says so**, instead of
  reporting an untrusted CA.

## v0.5.4

- **The fabric tail works on every goatnet.** `goat sub --literal '<key>'`
  subscribes on the key exactly as written. On the zagros goatnets the
  first key segment is normally a slot alias, so a seeded vendor prefix
  such as `airdefenders/` used to be refused as an unknown alias and the
  Air Defenders demo could not be watched from a terminal there. The flag
  is read-only: a typo gives an empty tail, never a stray write.
- **`goat demo airdefenders` uses it automatically**, so the one-command
  demo (browser + terminal tail) now runs on zagros, zagros-prod and zagros-dev.

## v0.5.3

- **A key for your phone.** Onboarding now hands you two mesh keys: the device
  key for laptops, and a *handset* key for phones and tablets. The handset key
  gives a phone read-only reach (OBSERVER), so a phone never carries your
  laptop's access, and the phone appears on the goatnet under your name rather
  than as "iPhone". The welcome package tells you where to type it in the
  NetBird app.
- **Lost a phone?** An operator runs
  `goat admin onboard <you> --reissue-handset-key`: your old handset key stops
  working and a new one is issued.
- **Clearer error when a profile is locked.** A profile directory you cannot
  read now says `E_PROFILE_DENIED` instead of claiming no profile exists.

## v0.4.8

- **`goat who` — the identity common operating picture.** A searchable,
  master-detail roster of every identity in your goatnet — declared in the
  registry *or* seen live on the bus — default-sorted by name. Pick one and the
  detail pane shows what it may do across all four domains (identity, mesh,
  data, release) and what it's doing right now (live presence, throughput,
  band). `/` filters incrementally (`live:on`, `use:on`, or any substring of a
  name); `s` cycles the sort; `a` fills in permissions for the whole roster;
  `↑/↓` `j/k` `PgUp/PgDn` `g/G` navigate and the list scrolls to follow you;
  `r` retries a permission read that timed out. It is read-only — it reveals,
  never mutates. `--once` prints a roster snapshot for scripts.
- **Human names, not UUIDs.** Each identity shows its name — the slot alias, or
  the registry display name — and falls back to a short ID only when neither
  exists.
- **Usable over a slow mesh.** Portal reads now allow up to 20 s (override with
  `GOAT_PORTAL_TIMEOUT_SECS`), so names and permissions still load over a
  degraded link instead of reading as "unavailable." A read that can't reach
  the portal in time says so plainly — and `r` retries it.

## v0.4.6

- **`goat top` — the live fabric dashboard.** One auto-refreshing screen
  that composes every environment-health domain (mesh, fabric, time,
  presence) with a sortable per-producer table — who's publishing, on which
  class, how close to their limit. `/` filters the table live (htop style),
  `--filter` pre-filters, `--once` prints a single batch for scripts. It
  opens read-only, so watching never competes with real traffic.
- **Per-actor throughput band in `goat top`.** Each producer's row shows a
  `BAND` verdict — `LOW` / `OK` / `HIGH` / `OVER` — so you see not just the
  rate but whether that rate is normal *for that actor*. Below-floor is a
  fault signal too, not just floods.
- **`goat admin peer ls|cut`** — list mesh peers and cut one in a single
  command, the emergency lever when an actor is flooding the fabric.
- **`goat admin rotate <anchor|alias>`** re-issues a peer's leaf cert in
  place for the same anchor — expiry renewal or post-compromise re-key with
  no namespace/ACL change. `--org` rotates a whole org's certs at once.
- **`goat admin onboard --org`** extends revocation to an org-scoped
  cascade, cutting an org's people and systems together.
- **`goat audit ops`** reads the operator-action audit stream — who ran
  which privileged command — decrypting the entries you're a recipient of.
- **Steadier sessions on a busy fabric.** Session-open now retries past a
  transient peer link-cap, the connect timeout is tunable, and a hit
  link-cap reports an actionable error instead of a bare failure.

## v0.4.5

- **The environment-health board.** `goat health --domain <mesh|time|presence>`
  renders each domain with the *why* in words — mesh-link health from your
  vantage, time-sync confidence, and live presence — alongside the fabric
  health you already had. Every reading is honesty-graded (never a false
  green; idle/unmetered when it can't be grounded).
- **`goat presence`** shows who is actually on the bus right now via native
  liveliness — one-shot, `--watch` for join/leave edges, or
  `--subscribers-of` a topic — instead of guessing from traffic.
- **`--narrate-json`** emits the substrate cascade as NDJSON, so the
  step-by-step explanation of what a command did is machine-readable.

## v0.4.4

- **The full send/receive lifecycle is in the CLI.** New commands: `goat
  schema` (publish-format lifecycle), `goat access` (see who can read/write
  what, and why), `goat blob` (bounded large-file send/get over the
  non-persisted class), `goat topic export|import`, and `goat replay`
  (re-read history through the audit tail, which also unblocks `narrate`
  and `sub --since`).
- **Short names everywhere.** `doctor`, `pub`, and `sub` accept and render
  slot aliases (e.g. `maintainer-operator/…`) in place of raw UUID anchors;
  `--uuid` shows the machine form. Machine paths are unchanged — aliases
  are display + input only.
- **`goat pub --encoding`** stamps the published sample with a
  self-describing encoding so inspection surfaces can decode it from the
  wire.
- **`goat admin lan scan|doctor`** finds a host that powered up but never
  appeared on the WAN — the LAN-side triage half of the relocation runbook.
- **Install snippets in this guide are goatnet-neutral.** The placeholders
  now read `get.<your-goatnet>` / `docs.<your-goatnet>` instead of baking in
  one goatnet's hostname; the served pages still substitute your real hosts.
- Fixed: `goat blob` uses a random session id so concurrent `send`+`get`
  on the same anchor don't collide.

## v0.4.3

- **`goat` now follows your selected context.** When your `config.toml` sets a
  `current_context`, every command resolves against that goatnet — no more
  silently falling back to the wrong one.
- **`goat doctor --topic <topic>` checks a write before you send it.** It tells
  you up front whether a publish to that topic would be accepted or quietly
  dropped, so a denied write never disappears without a word.
- **`goat --env <goatnet>` for operators who work across goatnets.** Onboarding
  and admin commands take an `--env` so you can target nova, ZAGROS, or
  another goatnet without juggling separate setups.

## v0.4.2

- Fixed the language picker on the hosted docs site so the flag menu renders and
  switches correctly.

## v0.4.1

- The full guide is now translated into all supported languages. Use the flag
  picker in the top bar, or `goat --lang <code> docs` from the CLI.

## v0.4.0

- **`goat docs`** — a browsable, searchable field guide, served on the mesh at
  `docs.<your-goatnet>`, always matching the binary you downloaded.

## v0.3.0

- **Onboarding that doesn't need credentials.** `goat hello` runs against a
  fully simulated, clearly-marked sandbox; `goat tutor` walks you through the
  tool hands-on; first-run hints point you to the next step.

## v0.2.0

- **Windows builds.** `goat-windows-amd64.exe` joins the macOS and Linux
  binaries. Every command works natively except the tunnel daemon (`goat
  client`), which stays on macOS and Linux for now.

## v0.1.0

- First release of the unified `goat` operator CLI: profile setup, connectivity
  `doctor`, topic browse and CRUD, publish and subscribe, and stethoscope
  delegation.

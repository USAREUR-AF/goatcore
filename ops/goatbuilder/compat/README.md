# goatbuilder's promise to the last release

Four surfaces of goatbuilder are what other people build on: the
configuration, the commands and flags, the `-o json` and `-o yaml` documents,
and the error codes. This directory holds what the last release did on the
last three, and `compat.py` checks that a build still does it. The rules are
the ones in "What 1.0 promises" in the goatbuilder 1.0 gate (rows A3 to A5).
The configuration files of every release (row A2) are kept with the crate's
tests and checked by `cargo test -p goatbuilder --test config_compat`; see
[crates/goatbuilder/tests/compat/README.md](../../../crates/goatbuilder/tests/compat/README.md).

```bash
cd crates && cargo build -p goatbuilder && cd ..
bash ops/ci/check-goatbuilder-compat.sh crates/target/debug/goatbuilder
```

The script runs the check's own tests and then the check. CI runs it in
`.github/workflows/goatbuilder-pr.yml` on every pull request that touches the
crate, the schema, the catalog or this directory.

## The last release

`baseline.yaml` names it: `goatbuilder/v0.5.1`, commit `89e7cc1a`, with the
SHA-256 of the released `goatbuilder-linux-amd64` binary the baseline was
recorded from and of the `SHA256SUMS` that listed it. `releases.sha256` pins
every published binary a baseline was recorded from. Nothing here is read from the network at check time: the
baseline is committed, so a check's result depends only on the tree and the
build.

| Row | Rule | Kept here | Recorded from |
|---|---|---|---|
| A3 | No command or flag of the last release is gone or renamed, unless `deprecations.yaml` names it. A flag that took a value still takes one; an optional positional argument does not become required; no positional argument is dropped | `surface.json` | `goatbuilder docs --markdown` of the released binary, which equals the release's `docs/reference/goatbuilder/command-reference.md` |
| A4 | Each kept document is still produced by the same invocation, with the same version (`schema`; a document that had none may gain one), the same exit code, and no field removed or retyped. A list that had entries must still have entries. Added fields pass | `documents/manifest.yaml`, `documents/<name>.<format>`, `documents/fixtures/` | the released binary, run on the fixtures kept beside the documents |
| A5 | Every `GB-nnn` code of the last release is in `ops/errors/catalog.yaml` with the same title. A retitled entry passes only when it carries `retired:`. A code that is gone always fails: a retired code stays in the catalog | `errors.yaml` | `ops/errors/catalog.yaml` at the release tag |

## When a release is published

The release's own pull request does not move the baseline: the check holds
the release to the one before it. After the tag is published, one pull request
moves the baseline to it:

```bash
v=0.6.0
curl -fsSLO https://github.com/USAREUR-AF/goatcore/releases/download/goatbuilder/v$v/goatbuilder-linux-amd64
curl -fsSLO https://github.com/USAREUR-AF/goatcore/releases/download/goatbuilder/v$v/SHA256SUMS
grep linux-amd64 SHA256SUMS | sha256sum -c && chmod +x goatbuilder-linux-amd64
python3 ops/goatbuilder/compat/compat.py refresh --tag goatbuilder/v$v --bin ./goatbuilder-linux-amd64 --sums SHA256SUMS
```

`refresh` refuses a binary whose SHA-256 is not the goatbuilder binary entry
of that `SHA256SUMS`, one that disagrees with `releases.sha256` for a release
already pinned there, and one whose `--version` is not the tag's: a local
build of main reports the same version as the last release, so the version
alone proves nothing. It pins the binary's SHA-256 in `releases.sha256`, and
`check` refuses a `baseline.yaml` whose binary that file does not pin. It rewrites
`surface.json`, `errors.yaml`, the documents and their exit codes, and
`baseline.yaml`. Review the diff: a document or code that changed between the two
releases is a change the release made, and its notes should say so.

**The reviewer checks the new pin against the release page.** For a release
not yet pinned, `refresh` trusts the `SHA256SUMS` it is given: the pin is only
as good as that file. The pull request that moves the baseline adds one line
to `releases.sha256` and sets `sums_sha256` in `baseline.yaml`; its reviewer
compares that line with the `goatbuilder-linux-amd64` entry of `SHA256SUMS` on
the tag's release page (and may verify `SHA256SUMS.cosign-bundle`). A
follow-up has `refresh` fetch `SHA256SUMS` from the tag's release URL itself.

**There are two pin lists.** The upgrade test (gate E1) pins the same
published binaries in `crates/goatbuilder/tests/upgrade/releases.sha256`, in
its own format. Nothing ties that file to `releases.sha256` here yet; they
agree today where they overlap (v0.4.3 and v0.5.1, linux-amd64). A follow-up
keeps one of them, or adds a gate that compares the two.

## Changing a surface on purpose

- **A command or flag to be removed or renamed.** Add an entry to
  `deprecations.yaml` naming the command (and flag), its replacement and the
  release that first warns. The deprecation period itself is a decision that is
  not yet taken (row A1).
- **An error code no longer printed, or whose fault is renamed.** Keep the
  entry in the catalog and add `retired: goatbuilder/vX.Y.Z`. A different fault
  gets a new code.
- **A kept exit code that changes** (before 1.0 only; exit codes are a
  promised surface). Edit that invocation's `exit` in
  `documents/manifest.yaml` in the same pull request and give the reason in
  `exit_changed:`. The next refresh records the release's own exit and drops
  the reason. `validate-not-yaml` keeps 0.5.1's exit 2 for this purpose.
- **A kept document whose shape must change.** It cannot within one version:
  add the new field beside the old one, or publish a new document version.
- **A field a format carries only on some entries** (conditional fields). The
  A4 check merges the fields of every entry of a list, so a field that only some
  entries carry looks gone when no kept entry carries it any more. List it in
  `conditional.yaml`: the format (`schema:`), the list (`items: hits[]`), the
  field, `present_when:` (the entries that carry it) and the reason. The check
  then holds it per entry: it may be absent from an entry the condition does not
  name, and stays, with the release's type, on every entry it does. Today's one
  entry is the interpretation's `hits[].score`, carried by a lexical guess only;
  a refusal that gains its own code turns its guesses into one code hit with no
  score.

## What is not checked here

- The text of tables printed for a person, and the shell scripts beside the
  binary: neither is promised.
- The meaning of a flag beyond whether it takes a value, and exit codes beyond
  the kept invocations.
- Documents of verbs that need a host or write state (`deploy`, `standup`,
  `federate` past its first refusal): no read-only invocation of them on a
  fixture produces a document yet.
- `explain <code> -o json` prints the catalog entry without a `schema` field,
  so that document carries no version yet. A document kept with no version
  may gain one (it is an added field); a document kept with a version must
  keep exactly that version.

# Error interpretation — every error goat and goatbuilder print says what happened, why, and what to do

| Field | Value |
|---|---|
| Status | **Built** (2026-10-28): the catalog, the shared crate, `goat explain` and `goatbuilder explain`, both binaries' top-level error paths, the CI gate and the generated reference. The semantic layer is **designed, not built** (§"The fourth layer") |
| Decision | [ADR 1199](../adr/1199-one-error-catalog-interpreted-by-both-clis.md) |
| Sources | `ops/errors/catalog.yaml` (the catalog) · `crates/goat-errors/` (the crate) · `crates/goat-cli/src/cmd/explain.rs`, `crates/goatbuilder/src/main.rs` (the verbs and the error paths) · `ops/ci/check-error-catalog.py` + `.github/workflows/error-catalog.yml` (the gate) · [`docs/reference/errors.md`](../reference/errors.md) (generated) |
| Audience | platform admins and operators who read the errors; anyone adding an error to either binary |

## What this is

An operator who sees an error from `goat` or `goatbuilder` must be able to act on it without asking anyone. Every error either binary prints therefore carries four things: what happened, the likely cause, what to do (a command where one exists), and where to read more. Errors that come from a tool the binary called — ansible, ssh, op, aws-vault, git, gh, cosign, docker, netbird — are carried through and interpreted the same way, because the words an operator is left with are often that tool's, not ours.

One catalog holds all of it: `ops/errors/catalog.yaml`, 88 entries at the time of writing, compiled into both binaries at build time so `goat explain` and `goatbuilder explain` work offline and on a machine with no checkout. Adding an error is a YAML block in that file and nothing else.

## What the operator sees

A refusal from either binary prints its own line in the words it always had, then — at a terminal — the interpretation. For a goatbuilder deploy whose play failed on a host because SSH refused the key:

```
goatbuilder: deploy FAILED — log /var/folders/…/goat-deploy-netvis-zagros_dev_hosts-20261028T073148Z.log
  [matched: zagros_dev_hosts | UNREACHABLE! => {"msg": "Failed to connect to the host via ssh: … Permission denied (publickey)."}]
  X-SSH-001 — the host refused the key
    what: the connection reached the host and SSH rejected every key offered.
    why:  your public key is not on the host's operator user (ssh-key-self-service)
          the wrong user (hosts take `goat`)
          the key is not loaded in the agent
    fix:  ssh -v goat@<host> true 2>&1 | grep -i 'offering\|denied'
          goat admin ssh-key …   (register the key through the goatnet)
    read: docs/design/ssh-key-self-service.md
  [matched: … UNREACHABLE! …]
  X-ANSIBLE-001 — a host was unreachable
    …
  [code]
  GB-029 — the play failed on a group
    what: ansible-playbook exited non-zero for the group; the log path is in the row.
    …
```

The order is innermost first: the tool's own words before the goat verb that carried them, so the first fix printed is the one closest to the cause.

The interpretation goes under the error on stderr when stderr is a terminal; a captured run (a log file, a CI step, a parity gate, a golden test) keeps the one-line error it always had, plus `(GB-029 — goatbuilder explain GB-029)` when the error carries a code. `GOAT_EXPLAIN=1` prints it always, `GOAT_EXPLAIN=0` never. `-o json` and `-o yaml` on goatbuilder always carry the full interpretation on stdout (`goatbuilder.error/v1`: `error`, `code`, `interpretation`), with the one-line error on stderr.

The same interpretation is available on demand, for any text, from either binary:

```
goat explain E_OFF_MESH                 # one code
goatbuilder explain gb-7                # codes in any form: gb-7, GB-007, x-ssh-1, profile-missing
goat explain "Permission denied (publickey)"
pbpaste | goatbuilder explain -         # any pasted error text, from stdin
goat explain --list                     # every code the catalog knows
goat explain -o json …  /  goat explain --json …
```

When nothing in the catalog matches, the binary says so and names the one thing to do: `goat explain --report -` (or `goatbuilder explain --report -`) writes the text and its interpretation to `~/.goat/error-reports/<program>-<stamp>.txt`, a file to share so the catalog can grow. The three nearest catalog entries are printed with that line when any score above the threshold.

## The catalog

One entry per error. The fields:

| Field | Holds |
|---|---|
| `code` | stable: `E_*` is goat's own namespace (unchanged, from `crates/goat-cli/src/errors.rs`), `GB-NNN` goatbuilder's, `X-<TOOL>-NNN` a tool either binary calls |
| `tool` | who prints it: `goat`, `goatbuilder`, or the tool (`ansible`, `ssh`, `op`, `aws`, `git`, `gh`, `cosign`, `docker`, `netbird`, `http`, `dns`, `tls`, `python`) |
| `title` | the claim, in the operator's words |
| `what` | what happened |
| `causes` | likely reasons, most likely first |
| `fixes` | what to do; commands where a command exists |
| `read` | a repo path to read more; the gate checks it exists |
| `exit` | the exit code the binary returns (goat and goatbuilder codes only) |
| `matches` | regular expressions over the whole text of an error, for errors that arrive as text; any one matching is a hit |

The catalog is the lexical data store the operator asked for: expandable by a YAML block, read by both binaries, no code to change for a new pattern. `ops/ci/check-error-catalog.py` holds it to the contract (§"The gate").

## How a text is interpreted

`goat_errors::interpret(text)` runs three matchers in order over the **whole** text, not its first line, and returns every hit:

| Layer | Finds | How |
|---|---|---|
| **code** | the codes the text carries | `\b(GB-\d{3}\|X-[A-Z]+-\d{3}\|E_[A-Z][A-Z_]+)\b` anywhere in the text; each resolves to its entry. A goatbuilder refusal is a `goat_errors::Coded` error whose code the binary reads back directly (`interpret_coded`); its message stays in the script's own words, which the parity gates compare, and every `GB-` entry carries a `matches:` pattern for those words, so the same message pasted from a log is recognised by the pattern layer |
| **pattern** | entries whose `matches:` regex hits anywhere in the text | every pattern of every entry is tried; a passthrough error (a goat line carrying an ansible line carrying an ssh line) yields a hit per layer of it |
| **lexical** | the nearest entries when no code and no pattern matched | IDF-weighted overlap between the text's word set and each entry's (title, what, tool, causes, pattern literals): the weight of the shared words over the weight of the text's words, threshold 0.45 and at least two shared words, at most three entries, ties by code. Deterministic; no model |

Hits are ordered innermost first: external tools' entries before goat's or goatbuilder's own, and within a group by where the evidence sits in the text. An interpretation is `unrecognised` when the first two layers found nothing; its hits, if any, are the lexical neighbours.

The interface is one trait, `Matcher { fn find(&self, cat, text) -> Vec<Hit> }`, with the three implementations above.

### The fourth layer

A semantic matcher (an embedding model with a vector index) would implement the same trait and sit after the lexical layer. It is deferred: a shipped embedding model adds tens of megabytes to two binaries that run on islanded sites, its answers are not reproducible between versions, and every error either binary can meet today comes from its own code or from a fixed set of tools whose words are known and catalogued. What would bring it in is a measured miss rate: `explain --report` files that keep arriving for texts the lexical layer missed. Until then, each such file becomes a catalog entry.

## Where the binaries call it

| Place | What it does |
|---|---|
| `goatbuilder` top-level error path (`crates/goatbuilder/src/main.rs`) | prints `goatbuilder: <message>` then, at a terminal or with `GOAT_EXPLAIN=1`, the interpretation (else the one-line `(GB-NNN — goatbuilder explain GB-NNN)` hint); `-o json\|yaml` keeps that line on stderr and prints `{schema: goatbuilder.error/v1, error, code, interpretation}` on stdout, where every report goes; the exit code is the coded entry's `exit`, else 2 |
| goatbuilder refusal sites (`push.rs`, `release.rs`, `rollback.rs`, `fastlane.rs`, `verbs.rs`, `converger.rs`, `config.rs`, `pins.rs`, `promote.rs`, `main.rs`) | `Err(goat_errors::coded("GB-NNN", message))`: 54 sites, every `bail`/`anyhow!` that was a refusal; promote's `die()` codes every graduation refusal GB-033 in one place |
| `goatbuilder doctor` | rows that name what a run needs on this machine before anything is cut — each either a FAIL with the exact fix or, for a check that was not run, a `skip` that says so, from the blockers two operators' first releases met (none of them the candidate): `secrets:op-session` (`op whoami`) and `tool:aws-vault` (`aws-vault exec <profile> --no-session -- aws sts get-caller-identity`, naming `AWS_VAULT_KEYCHAIN_NAME` when the environment sets it, and the line that unsets it) -- both of which read a credential, so they run only under `--credentials` and otherwise read as *not checked*, counted apart and never a pass, since unlocking a credential store asks a person for a password, `tool:crane` (rc.sh resolves digests with crane and discards its error, so a missing crane read as "image not found"), `tool:goat-contexts:<goatnet>` (for every runner with `goat_via: ""`, the sponsor context its values file names, `goat_sponsor_context`, exists under `~/.goat/contexts` with an `onboard_endpoint`, else onboarding calls go to the portal and answer 405); and the two below. `env:op-refs` scans the environment for `op://` values with fewer than vault/item/field and names the variable and the way around it (`env -u <VAR> op run …`), because `op run` validates every such variable, not only the env-file, and refuses the whole run; `tool:live-rows` checks the console e2e suite's `node_modules`, a Playwright that answers and a chromium in its browsers directory, because the live acceptance rows run `ops/scripts/onboard-intake-smoke.sh` from the laptop and exit 2 there without them, which reads as a failure on the goatnet (catalog `X-SMOKE-001`; the fix is the suite's own install line). Only a release runs those rows, so without `--browser-tests` a missing toolchain is a passing row with its fix, printed as a note, and with `--browser-tests` it is a FAIL (built). The `checkout` row passes an assembled tree on its detached HEAD when the tree's `.goat-assembled` marker names the goatcore commit at HEAD, and names what the marker records; without the marker a detached HEAD fails as F-586 (built) |
| `goat` top-level error path (`crates/goat-cli/src/main.rs`, `report_anyhow`) | a `GoatError` renders as before (`error:` / `help:` / `code:`) and then, at a terminal or with `GOAT_EXPLAIN=1`, any tool's words its hint carried are interpreted; every other error prints `error: <message>` then, under the same rule, what the catalog knows about it. Captured output is unchanged, so the `trycmd` golden tests hold |
| `goat explain`, `goatbuilder explain` | the verb: a code, or text, or `-` for stdin; `--list`; `--report`; machine output (`--json` on goat, `-o json\|yaml` on goatbuilder) |

The goat `E_*` namespace, its exit codes and `docs/design/goat-cli-error-codes.md` are unchanged; the catalog carries the same codes with the same exit codes, and the gate refuses a drift between the two.

## The gate

`ops/ci/check-error-catalog.py` (workflow `error-catalog.yml`, on any change to the catalog, the crate, the citing code, the codes doc or the reference):

- every entry has `code`, `tool`, `title`, `what`, at least one cause and one fix, and a `read` path that exists; codes are unique and shaped; goat and goatbuilder codes carry `exit`
- every `matches:` pattern compiles (Python `re`; the crate's own tests compile them with the Rust engine, which is authoritative)
- every code a binary cites in Rust exists in the catalog; every `#[error("E_…")]` variant in `errors.rs` exists in the catalog
- every row of `docs/design/goat-cli-error-codes.md` is in the catalog with the same exit code, and every `E_*` entry has a row there
- `docs/reference/errors.md` equals what `--write` renders from the catalog

The crate's tests prove the three layers on real messages: a passthrough deploy failure (goatbuilder → ansible → ssh) reports all three codes with the ssh entry first; the `op run` malformed-reference stderr resolves to `X-OP-002`; a paraphrase with no pattern resolves lexically to `X-SSH-001`; nonsense is unrecognised with no hit.

## Adding an error

1. Add the entry to `ops/errors/catalog.yaml`. For a tool's error, put the exact words it prints in `matches:` (quote the stderr; keep the pattern literal).
2. If a binary should emit the code itself, cite it: `Err(goat_errors::coded("GB-032", …))` in goatbuilder; a `GoatError` variant plus a row in `goat-cli-error-codes.md` in goat.
3. `python3 ops/ci/check-error-catalog.py --write` regenerates the reference; `cargo test -p goat-errors` compiles the patterns.

## Hosting the reference

`docs/reference/errors.md` is generated in this repository. The operator-facing book (`docs/goat-cli-book/`, served at `docs.<site>`) is translated into 29 languages under a currency gate, so a chapter there is a translation campaign, not a file copy; the reference goes into the book's Troubleshooting chapter as a link in the next book release, and the book's generated `reference.md` carries `goat explain`'s own help already.

# goatbuilder output: what a person sees

Status: standard. Applies to every goatbuilder verb. Enforced by
`crates/goatbuilder/tests/output_standard.rs` and the unit tests named below.

## Why

goatbuilder had a versioned contract for machines (`-o json`,
`goatbuilder.output/v1`) and none for people. Each verb invented its own
human layout, and nothing tested it. A first operator walkthrough of the tutor
(#2341) found the result: `FAIL` rows before anything had run, a dry run that
said every ledger agreed when none was read, phase banners printed as passing
checks, commands that zsh could not run, placeholders recorded as the actor,
and data pasted into shell code. Each was a local choice; together they made
the tool hard to trust. This standard is the contract for the human surface,
held by tests, so a new verb cannot drift.

## Non-goals

- It does not change `-o json` or `-o yaml`. Those keep their versioned schemas.
- It does not restyle every verb at once. Verbs that do not meet a rule yet are
  listed in the test's allowlist, which may only shrink.

## The rules

### 1. Two surfaces, and gates read the machine one

The human table is for people and may change. `-o json` is the contract.
Scripts, parity gates and CI checks read `-o json`, never the table. When a
verb prints its own human view (a plan, a lesson page), its rows still go to
`-o json` and to the tutor's log unchanged. A view stands for the rows it was
printed for; a row after them that did not pass (a stop's verdict, code and
reason, such as a back-out refused after its plan, GB-221 or GB-223) is still
printed in table mode, after the view (`output::after_view`, from
`Report::view_rows`). Built for `federate backout`'s plan and `federate`'s dry
run, the two verbs whose view is printed before their run can stop; the other
views (doctor's summary, the mock status page) print every row they are given.

`goatbuilder federate status -o json` is its own versioned document,
`goatbuilder.federate.status/v1`, and a real federate run keeps its journal in
`<work>/state.json` (`goatbuilder.federate.state/v1`); both are contracts in
the same sense ([goatbuilder.md](goatbuilder.md), federate).

Enforced by: every parity gate reading `-o json`
(`ops/goatbuilder/parity-federate.sh`, `parity-standup.sh`,
`parity-teardown.sh`, `parity-deploy.sh`, `parity-verify.sh`) and
`ops/ci/check-phase-numbering.sh`; `output_standard.rs` checks every corpus
command's `-o json` parses and carries a versioned `goatbuilder.*/vN` schema.

### 2. Honest states

- A dry run says what it *would* do. It never claims a result it did not
  observe: no "every host's ledger names the declared ref", no
  "(ledger-verified)", without a ledger read.
- A step nobody has tried yet is never shown as `FAIL`.
- A title is not a check. Section headings print as headings, and the verdict
  line counts checks only -- including the verb's own summary row, which is the
  verdict and not a check of its own.
- A check that did not run is a third state, never a pass. It reads `skip` in a
  full listing, is counted apart in the verdict ("58 checks passed. 3 problems
  found, and 2 checks did not run."), and the page names which ones and what
  runs them. "All N checks passed" is only true when N is every check there was.
  In `-o json` it carries `"skipped": true`, with `ok` still true because it did
  not fail, so a program reads the state rather than the words.
- A mark means something happened. `✓` is a thing done, `✗` a thing that
  failed, `!` a thing not set. A row that says what a *real* run would do has
  not happened, so it takes a bullet and no colour. A plan owes nothing, so a
  by-hand step in a plan is numbered rather than marked.
- Every row is in the count. A state the verb's vocabulary does not name is
  still named and counted, so the numbers add up to the rows ("1 of 4 same,
  1 drift, 1 unknown, 1 in neither"), and a table that counts only the rows it
  kept does not print at all: a run with a failure in it reads as its steps.
- A count of things is a count: `5 machines`, `1 machine`, never `5 machine(s)`.
- A cause list is labelled `Causes`, because a catalog entry lists the causes
  an error can have and that is not a claim about this one.

Enforced by: `output_standard.rs` (no dry run claims a ledger read; no `===`
banner in any table), `tutor::tests::lesson_page_shows_steps_as_a_checklist_not_fail`,
`output_standard.rs` (`a_failing_row_is_on_the_page_a_person_reads`, on a real
terminal).

### 3. One layout

- Pages wrap at 80 columns, the standard terminal width.
- A phase is a heading: `Phase 5 · core · netbird-mgmt`.
- A plan is a table (PHASE, TYPE, STEP), then its commands.
- A table has column titles wherever its rows have columns, like `kubectl`.
- A state listing is a table (COMPONENT, MACHINE, STATE, RUNNING, DECLARED).
  Pending and unknown are states, so they never print as `FAIL`.
- A command too long for a line breaks with ` \` before an option, never inside
  quotes, `$(...)` or a `--flag value` pair. It must still run as printed in
  bash and in zsh. A command with nowhere safe to break stays on one long line:
  a line a person can copy is worth more than a line that fits and does not run.
  A sentence wraps anywhere, and a fix written as several steps with semicolons
  reads as one line per step.
- Where a table's own column leaves no room to read in, the text wraps under its
  row rather than running off the page. A page gives way to 80 columns; only a
  single unbreakable token (a path, a digest) may pass it.
- A page a person reads ends where the next shell prompt can start: one trailing
  newline, and never a command list split by a line printed to stderr.
- A page about something that needs doing names the command that does it. A
  table of machines that do not match, a plan, a list of checks: each ends with
  the next command, not with the last row.

Enforced by: `output_standard.rs` (width for the verbs past the allowlist),
`tutor::tests::every_command_runs_as_printed_in_bash_and_zsh`,
`tutor::tests::lesson_pages_wrap_to_the_page_width`,
`federate::tests::a_dry_run_prints_a_plan_table`,
`verbs::status_page_tests::pending_and_unknown_are_states_in_a_table`,
`output::tests::a_terminal_table_has_column_titles_and_a_script_does_not`.

### 4. Plain language

Short sentences, one idea each, terms defined the first time they appear, no
em dashes, and every message that stops the reader names the next command to
run. Plain English also translates; the tutor's 29 languages are generated
from it.

Insider words (`rung`, `soak`, `fail closed`, `candidate`, `skew`) stay out of
a person's page. `-o json` and a pipe keep the words the gates compare.

Enforced by: `output_standard.rs` (no em or en dash in the verbs past the
allowlist), `output::tests::a_persons_page_is_plain_english`.

### 5. Placeholders never reach a record

`--ack` refuses a value that is still a placeholder (`<your name>`). No
command the tutor prints contains a placeholder.

Enforced by: `ack_tests::a_placeholder_ack_is_refused`,
`tutor::tests::every_command_runs_as_printed_in_bash_and_zsh`.

### 6. Data is never pasted into shell code

A value read from a file, a host or a counterparty is shell-quoted before it
goes into a command run with `bash -c`, or the message is printed by the binary
instead of by `echo`.

Enforced by: `federate::tests::an_agreement_value_never_reaches_the_shell`.

### 7. Fun follows the three rules of goat delight

The goat and next-step hints print only to a terminal, never to a
pipe, a file or `-o json`, and `GOAT_NO_FUN=1` turns them all off
([goat-cli-onboarding-and-delight.md](goat-cli-onboarding-and-delight.md)).

A hint also belongs to the thing it is about. A step that ticks says so
wherever it was run from, because a learner who forgot the export is still
doing the lesson -- but "that command is not part of lesson N" prints only when
the run itself named the tutor's workspace. An operator at work on their own
goatnet does not want a line about a lesson under every failure.

Enforced by: `tutor::tests::a_run_that_ticks_a_step_says_so` (the hint) and
`output_standard.rs` (no banner when the output is not a terminal).

### 8. Show the problems, not every check

A verb that runs many checks prints what passed as one count, then one entry per
problem with its fix. Rows with one cause are one entry. A flag prints every row,
and `-o json` always has them all.

Enforced by: `doctor::summary_tests`,
`output_standard.rs` (`doctor_prints_problems_and_keeps_every_row_for_machines`).

## Adding a verb

Run the verb through `output_standard.rs` by adding it to the corpus. If it
cannot meet a rule yet, add it to that rule's allowlist with the issue that
will fix it. Removing an entry is always welcome; adding one needs a reason in
review.

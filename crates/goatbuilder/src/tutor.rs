//! `tutor`: vimtutor-style lessons completed by running the real verbs
//! (docs/design/goatbuilder.md §"Verbs"). Each lesson is a short text and the
//! exact commands to type; the learner runs them against a throwaway copy of a
//! fixture (or, read-only, against this checkout), and the tutor checks what
//! those runs actually did — their exit codes and their rows — before the next
//! lesson unlocks.
//!
//! How the tutor sees a run: every `goatbuilder` invocation whose `--mock` or
//! `-C` path lies inside the tutor's workspace, or that runs with
//! `$GOATBUILDER_TUTOR` set, appends one line to `<workspace>/log.jsonl`
//! (`log_run`). Progress lives beside it in `progress.json`; `--reset` starts
//! over with fresh copies. Nothing the tutor does touches a host.
//!
//! The tutor teaches the CLI only. The older scripts it replaced (ADR 1201)
//! are mapped in docs/reference/goatbuilder/appendix-b-script-equivalents.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::CommandFactory;
use serde::{Deserialize, Serialize};

use crate::{Check, Cli, OutputFormat, Report, EXIT_OK, EXIT_REFUSED};

/// The file that marks a directory as a tutor workspace (and makes `--reset`
/// safe: it never deletes a directory without it).
const MARKER: &str = ".goatbuilder-tutor";

/// Where a run happened, which a lesson can require.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// This checkout (or any repository outside the workspace).
    Repo,
    /// A fixture copy in the workspace, by directory name.
    Fixture(&'static str),
    /// The workspace's skeleton copy of the repository (`<ws>/repo`).
    Skeleton,
}

/// One thing the learner must do in a lesson, matched against logged runs.
pub struct Expect {
    /// The step's instruction, shown on the lesson page (and translated as
    /// tutor-l<N>-s<K>); English also names the step in -o json rows.
    pub label: &'static str,
    /// The command for this step, exactly as typed, `{ws}` being the workspace.
    pub command: &'static str,
    /// Every token must appear in the run's arguments; the first is the verb.
    pub words: &'static [&'static str],
    pub on: On,
    /// Accepted exit codes.
    pub exits: &'static [i32],
    /// Every string must appear in the run's rows.
    pub contains: &'static [&'static str],
    /// Said when a run of the same verb misses this expectation.
    pub hint: &'static str,
}

pub struct Lesson {
    pub title: &'static str,
    pub text: &'static str,
    /// Commands to run once before the steps, `{ws}` being the workspace.
    pub setup: &'static [&'static str],
    pub expects: &'static [Expect],
    /// A verb this build must have for the lesson to run.
    pub needs_verb: Option<&'static str>,
    /// A path in the checkout the lesson's fixture comes from.
    pub needs_path: Option<&'static str>,
    /// A runtime dependency the lesson's verbs shell out to, named up front —
    /// with the one command that installs it — so a missing tool is a heads-up
    /// before the commands rather than a failure partway through them. Unlike
    /// `needs_verb`, it does not gate or skip the lesson; it is shown and the
    /// learner installs it. The value is a command/tool line, not translated.
    pub prereq: Option<&'static str>,
}

const ANY: &[&str] = &[];

pub const LESSONS: &[Lesson] = &[
    Lesson {
        title: "Check your setup",
        text: "The goatbuilder CLI builds, deploys and releases GOAT software. Before you use it, check your setup with two commands:\n- The goatbuilder `validate` command checks the config file, `ops/goatbuilder/goatbuilder.yaml`. It reads only this checkout and needs no network.\n- The goatbuilder `doctor` command checks what a real deploy needs: your tools, your sign-ins, whether this checkout matches main, and how goatbuilder reaches each goatnet. A goatnet is one GOAT environment.\n\nThe goatbuilder `doctor` command prints how many checks passed, then each problem with its fix. In this lesson a problem is information, not a mistake. Read the fixes, but you do not have to apply them now.\n\nMost lessons need no sign-in. Lessons 2 to 5, 10, 11 and 16 use a practice copy. Lessons 8, 9, 12, 13, 15, 17 and 18 only read this checkout. Lesson 6 needs `gh` and the mesh, lesson 7 needs `gh`, and lesson 14 needs the mesh. No lesson needs 1Password.",
        setup: &[],
        expects: &[
            Expect { label: "Check the config file", command: "goatbuilder validate -C {ws}/repo", words: &["validate"], on: On::Skeleton, exits: &[0], contains: ANY, hint: "validate refuses only a config that breaks the schema; regenerate it with `python3 ops/goatbuilder/gen-config.py`." },
            Expect { label: "Ask the doctor about one goatnet", command: "goatbuilder doctor --env {goatnet}", words: &["doctor", "--env"], on: On::Repo, exits: &[0, 1], contains: ANY, hint: "pass --env with a goatnet key from the pins; `goatbuilder doctor --all` lists them." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Read state: converged, pending, unknown",
        text: "The goatbuilder `status` command compares two things for each machine: the version main declares for it, and the version its own record says it runs. It changes nothing.\n\nEach machine is in one of three states:\n- converged: its record names the declared version. This is done.\n- pending: its record names an older version, or nothing yet.\n- unknown: goatbuilder cannot read its record.\n\nOnly converged counts as done. If any machine is pending or unknown, `status` exits with code 3.\n\nFrom this lesson on, `--mock` points goatbuilder at a practice copy in your tutor folder instead of real machines. Nothing you do there reaches a real machine.\n\nNothing has been deployed to the practice copy's `dev` yet. So expect four machines pending and one unknown. You will see converged machines in lesson 3.",
        setup: &[],
        expects: &[Expect { label: "Read the state of dev", command: "goatbuilder --mock {ws}/fixture status --env dev", words: &["status", "--env", "dev"], on: On::Fixture("fixture"), exits: &[3], contains: &["pending", "unknown"], hint: "run it on the tutor's fixture copy with --mock, against the dev environment." }],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Deploy: plan it, run it, prove it",
        text: "Deploying puts the declared version on every machine, then reads each machine's record to prove it arrived. A deploy is done only when every machine's record names that version. A deploy that ran without errors is not enough.\n\nYou will deploy `portal` to the practice copy's `preprod`. `--dry-run` shows what would happen and changes nothing. `--ack` records who did it; the command fills in your name from git. After the deploy, `status` should show both machines as `ok` and converged.",
        setup: &[],
        expects: &[
            Expect { label: "Plan the deploy", command: "goatbuilder --mock {ws}/fixture deploy --env preprod --component portal --dry-run", words: &["deploy", "--env", "preprod", "--dry-run"], on: On::Fixture("fixture"), exits: &[0], contains: &["DRY"], hint: "plan first: add --dry-run, which publishes nothing." },
            Expect { label: "Deploy portal to preprod", command: "goatbuilder --mock {ws}/fixture deploy --env preprod --component portal --ack \"$(git config user.name)\" --yes", words: &["deploy", "--env", "preprod", "--yes"], on: On::Fixture("fixture"), exits: &[0], contains: &["converged on"], hint: "deploy to preprod (every machine there converges), with --ack and --yes." },
            Expect { label: "Check that both machines converged", command: "goatbuilder --mock {ws}/fixture status --env preprod --component portal", words: &["status", "--env", "preprod"], on: On::Fixture("fixture"), exits: &[0], contains: &["converged on"], hint: "read preprod's state; it is green only after the deploy above." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Drift and history",
        text: "Two commands help you understand a goatnet before and after a change. Both only read.\n- The goatbuilder `diff` command lists every machine whose record differs from the declared version. A difference is called drift.\n- The goatbuilder `history` command lists every version a component had on each machine, newest first.\n\nOn `dev`, every machine is behind. The history of `portal` on `preprod` now includes your deploy from lesson 3.",
        setup: &[],
        expects: &[
            Expect { label: "List the drift on dev", command: "goatbuilder --mock {ws}/fixture diff --env dev", words: &["diff", "--env", "dev"], on: On::Fixture("fixture"), exits: &[0, 3], contains: ANY, hint: "run diff on the fixture copy's dev environment." },
            Expect { label: "Read the history of portal on preprod", command: "goatbuilder --mock {ws}/fixture history --env preprod --component portal", words: &["history", "--component", "portal"], on: On::Fixture("fixture"), exits: &[0], contains: ANY, hint: "name the component with --component." },
        ],
        needs_verb: Some("diff"),
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Roll back, and see an unsafe rollback refused",
        text: "A rollback declares an earlier version and deploys it like any other version. The goatbuilder CLI then reads each machine's record to prove it arrived.\n\nSome changes cannot be undone this way. Deploying an older version cannot undo data a component has written, a certificate, or a change that only moves forward. The goatbuilder CLI refuses those with exit code 3 and says what to do instead.\n\nThe practice copy's `db-schema` component holds data, so the goatbuilder CLI refuses to roll it back. That refusal is the expected result.",
        setup: &[],
        expects: &[
            Expect { label: "Plan the rollback", command: "goatbuilder --mock {ws}/fixture rollback portal 1 --env preprod --dry-run", words: &["rollback", "portal", "--dry-run"], on: On::Fixture("fixture"), exits: &[0], contains: &["DRY"], hint: "plan first with --dry-run." },
            Expect { label: "Roll portal back one version", command: "goatbuilder --mock {ws}/fixture rollback portal 1 --env preprod --ack \"$(git config user.name)\" --yes", words: &["rollback", "portal", "--yes"], on: On::Fixture("fixture"), exits: &[0], contains: &["declared"], hint: "roll back on preprod, with --env preprod, --ack and --yes." },
            Expect { label: "Try to roll back db-schema, which goatbuilder refuses", command: "goatbuilder --mock {ws}/fixture rollback db-schema 1 --env preprod --ack \"$(git config user.name)\" --yes", words: &["rollback", "db-schema"], on: On::Fixture("fixture"), exits: &[3], contains: &["REFUSING"], hint: "db-schema is a data artifact; the refusal, exit 3, is the point." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Release a new version: dev, then preprod, then prod",
        text: "A release moves a new version through three goatnets in order: dev, then preprod, then prod.\n\n1. Someone tags the version on main, and CI builds it.\n2. dev: goatbuilder deploys it and runs the acceptance tests.\n3. preprod: goatbuilder deploys it again, runs the tests again and records the results.\n4. The version then waits on preprod for a set time. Nothing moves to prod by itself.\n5. prod: after that wait, a person runs the release with `--from prod`. The goatbuilder CLI deploys to prod and checks all three goatnets.\n\nThe goatbuilder `promote status` command shows where every component is. A candidate is a version being tested. The general form of a release is `goatbuilder release <component> --tag <tag> --from dev`. The commands below use one component of this checkout and an example tag. Each one has `--dry-run`, so nothing is tagged or deployed.\n\nThe dry runs need `gh` and the mesh. If you are not an admin of the repository, put `GOAT_RC_ROLE_CHECK=0` before each `release` command.",
        setup: &[],
        expects: &[
            Expect { label: "See where every component is", command: "goatbuilder promote status", words: &["promote", "status"], on: On::Repo, exits: &[0], contains: ANY, hint: "`promote status` reads only." },
            Expect { label: "Dry-run the release from dev", command: "goatbuilder release {component} --tag {component}/v0.4.1-rc.1 --from dev --ack \"$(git config user.name)\" --dry-run", words: &["release", "--from", "dev", "--dry-run"], on: On::Repo, exits: &[0, 1], contains: ANY, hint: "a dry run that refuses at preflight names its fix: do what it says and run it again; the step counts once the dry run walks the release's steps (its rows start `==>`)." },
            Expect { label: "Dry-run the prod step", command: "goatbuilder release {component} --tag {component}/v0.4.1-rc.1 --from prod --ack \"$(git config user.name)\" --dry-run", words: &["release", "--from", "prod", "--dry-run"], on: On::Repo, exits: &[0, 1], contains: ANY, hint: "--from prod with --dry-run shows the prod step and the wait on preprod without acting." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: Some("`release` needs Python with PyYAML and jsonschema. Check: python3 -c 'import yaml, jsonschema'. If that fails, install them once: pip install pyyaml jsonschema"),
    },
    Lesson {
        title: "Fastlane, and when not to use it",
        text: "Fastlane puts one version on every goatnet at once. It skips the wait on preprod and the release records.\n\nUse it only for an urgent fix that you can undo by going back to the previous version. Never use it for a database change, an identity or access change, or anything you cannot undo that way.\n\nFastlane refuses a version that CI has not built, and a component that is part of a release still in progress.\n\nThe general form is `goatbuilder fastlane <component> --ref <version> --reason <why>`. Run the example below. It has `--dry-run`, so nothing changes. It needs `gh`.",
        setup: &[],
        expects: &[Expect { label: "Dry-run a fastlane", command: "goatbuilder fastlane {component} --ref sha-e4201ea --reason \"urgent fix example\" --ack \"$(git config user.name)\" --dry-run", words: &["fastlane", "--dry-run"], on: On::Repo, exits: &[0, 1], contains: ANY, hint: "in the tutor a fastlane always carries --dry-run; without it, it ships to every goatnet." }],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Pins: find drift and fix it the right way",
        text: "One file sets which version each goatnet runs: `ops/pins/fleet-pins.yaml`. The goatbuilder CLI generates the other version files from it: the `pins.yml` files and the signed intent files.\n\nThe goatbuilder `pins --check` command compares the generated files with `fleet-pins.yaml` and refuses any file that differs. The goatbuilder `pins` command regenerates them.\n\nYour tutor folder has a copy of the repository in which someone edited one `pins.yml` file by hand. Never fix a hand edit with another hand edit. Change `fleet-pins.yaml`, then regenerate.",
        setup: &[],
        expects: &[
            Expect { label: "Check this checkout", command: "goatbuilder pins --check", words: &["pins", "--check"], on: On::Repo, exits: &[0], contains: ANY, hint: "run it in a current checkout of main." },
            Expect { label: "Check the copy and find the hand edit", command: "goatbuilder -C {ws}/repo pins --check", words: &["pins", "--check"], on: On::Skeleton, exits: &[1], contains: &["out of date"], hint: "point it at the skeleton copy with -C {ws}/repo." },
            Expect { label: "Regenerate the copy's files", command: "goatbuilder -C {ws}/repo pins", words: &["pins"], on: On::Skeleton, exits: &[0], contains: &["wrote"], hint: "`pins` without --check regenerates from the table." },
            Expect { label: "Check the copy again", command: "goatbuilder -C {ws}/repo pins --check", words: &["pins", "--check"], on: On::Skeleton, exits: &[0], contains: ANY, hint: "after regenerating, the check is clean." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Explain an error",
        text: "When goatbuilder refuses, it prints a code. Codes that start with `GB-` come from goatbuilder, `E_` from the goat CLI, and `X-` from another tool that goatbuilder ran.\n\nThe goatbuilder `explain` command takes a code, or the text of an error line, and tells you what happened, why, how to fix it, and where to read more.\n\nAsking for a goatnet this checkout does not declare makes a real error to explain.",
        setup: &[],
        expects: &[
            Expect { label: "Make a real error with a goatnet that does not exist", command: "goatbuilder deploy --env no-such-goatnet --component {component} --dry-run", words: &["deploy", "--env", "no-such-goatnet"], on: On::Repo, exits: &[2], contains: &["GB-002"], hint: "type the goatnet as printed, no-such-goatnet: the refusal is the point." },
            Expect { label: "Explain the error by its code", command: "goatbuilder explain GB-002", words: &["explain", "GB-002"], on: On::Repo, exits: &[0], contains: &["GB-002"], hint: "pass the code the refusal printed." },
            Expect { label: "Explain the error by its text", command: "goatbuilder explain \"goatbuilder: unknown goatnet no-such-goatnet\"", words: &["explain"], on: On::Repo, exits: &[0], contains: &["GB-002"], hint: "pass the error line in quotes; the catalog finds the code from the words." },
        ],
        needs_verb: Some("explain"),
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Tutorial 1: build a new goatnet's first site",
        text: "This lesson builds a whole goatnet in a practice copy. The example goatnet is `zagros`. Its first site, `alpha`, has two core machines, one monitoring machine and one rescue machine.\n\nThe goatbuilder `standup` command sets up every machine in a fixed order and checks every record. Its last line says the site is done. On real machines, that line also shows how long it took.\n\nThe full tutorial is `docs/tutorials/01-new-goatnet.md`.",
        setup: &[],
        expects: &[
            Expect { label: "Build the first site, alpha", command: "goatbuilder --mock {ws}/zagros-1 standup zagros --site alpha --ack \"$(git config user.name)\" --yes", words: &["standup", "zagros", "--site", "alpha"], on: On::Fixture("zagros-1"), exits: &[0], contains: &["declared → verified"], hint: "run it on the zagros-1 copy, site alpha, with --yes." },
            Expect { label: "Check the result", command: "goatbuilder --mock {ws}/zagros-1 status --env zagros", words: &["status", "--env", "zagros"], on: On::Fixture("zagros-1"), exits: &[0], contains: ANY, hint: "status is green only after the standup." },
        ],
        needs_verb: None,
        needs_path: Some("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder"),
        prereq: None,
    },
    Lesson {
        title: "Tutorial 2: add a second site",
        text: "This lesson starts where tutorial 1 ended. It adds a second site, `beta`, in a new practice copy, `zagros-2`.\n\nA second site copies its user accounts from the first site. Its standup also changes the first site once, so that copying works in both directions. A step named `alpha/core` does this.\n\nThe full tutorial is `docs/tutorials/02-first-federated-node.md`.",
        setup: &[],
        expects: &[
            Expect { label: "Build the second site, beta", command: "goatbuilder --mock {ws}/zagros-2 standup zagros --site beta --ack \"$(git config user.name)\" --yes", words: &["standup", "zagros", "--site", "beta"], on: On::Fixture("zagros-2"), exits: &[0], contains: &["alpha/core"], hint: "run it on the zagros-2 copy, site beta." },
            Expect { label: "Check both sites", command: "goatbuilder --mock {ws}/zagros-2 status --env zagros", words: &["status", "--env", "zagros"], on: On::Fixture("zagros-2"), exits: &[0], contains: ANY, hint: "status covers the founder's carried records and the joiner's." },
        ],
        needs_verb: None,
        needs_path: Some("ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node"),
        prereq: None,
    },
    Lesson {
        title: "Add a machine to a running site",
        text: "The goatbuilder `join-node` command adds one more machine to a site that is already running. It prints a plan first, with one row per phase. Each phase has a type:\n- tool: goatbuilder runs a script.\n- ansible: goatbuilder runs a playbook.\n- gate: a check that must pass before the next phase.\n- manual: you do this yourself, and the run pauses.\n\nThe command below has `--dry-run`. It reads this checkout and changes nothing. Under the table, read what you do by hand.",
        setup: &[],
        expects: &[
            Expect { label: "Plan adding one machine", command: "goatbuilder join-node {goatnet} --site {site} --node node-x --class breakglass --dry-run", words: &["join-node", "--dry-run"], on: On::Repo, exits: &[0], contains: ANY, hint: "join-node with --dry-run touches nothing." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Connect to another organization's goatnet",
        text: "The goatbuilder `federate` command connects your goatnet to a goatnet that another organization runs. Each side builds its half of an agreement. You swap halves, and each side combines them into the same agreement.\n\nConfirm the other side's key in a separate way, such as a call you trust. A key that arrives inside their package proves nothing.\n\nThe command below has `--dry-run`, so it changes nothing. A real run stops at phase 4, because you do that phase by hand.",
        setup: &[],
        expects: &[
            Expect { label: "Plan a federation", command: "goatbuilder --mock {ws}/zagros-3 federate hello-ally --work {ws}/federate --dry-run", words: &["federate", "--dry-run"], on: On::Fixture("zagros-3"), exits: &[0, 1], contains: &["Phase 1 [GATE]"], hint: "federate with --dry-run prints the peering's phases." },
        ],
        needs_verb: None,
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Keep goatbuilder up to date",
        text: "The goatbuilder CLI can update itself from the copy that your goatnet serves at `get.<site>.<zone>/goatbuilder/`.\n\nThe goatbuilder `update --check` command only reads. It compares your version with the served version and changes nothing. Exit code 0 means you are up to date. Exit code 3 means a newer version exists.\n\nA real update, without `--check`, verifies the download's checksum and signature before it replaces anything. It then swaps the file in one step.\n\nThe goatbuilder CLI needs to know which goatnet to ask: pass `--site` with that goatnet, or set `GOATBUILDER_UPDATE_URL`. You must be on that goatnet's mesh. If you are not, `--check` says that it cannot reach the server and names the address it tried. That is a correct result for this lesson.",
        setup: &[],
        expects: &[Expect { label: "Ask whether a newer goatbuilder exists", command: "goatbuilder update --check --site {goatnet}", words: &["update", "--check"], on: On::Repo, exits: &[0, 1, 2, 3], contains: ANY, hint: "run `goatbuilder update --check --site {goatnet}`; it reads only. Current is 0, behind is 3, and unreachable from here is fine: the point is the command and what it reports." }],
        needs_verb: Some("update"),
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Assay: check how a goatnet was built",
        text: "The goatbuilder `assay` command checks that a goatnet was built correctly, one layer at a time, using the acceptance tests. Each test gets one of three results:\n- PASS: the test passed.\n- FAIL: the goatnet has a problem.\n- UNKNOWN: nothing could answer the test.\n\nOnly PASS counts. Any FAIL or UNKNOWN makes the exit code 3.\n\n`--plan` lists the tests that apply to a goatnet, by layer, and touches no machine. Use it to see what a real assay would cover. The general form is `goatbuilder assay <goatnet> --plan`.",
        setup: &[],
        expects: &[Expect { label: "List the tests that apply to one goatnet", command: "goatbuilder assay {goatnet} --plan", words: &["assay", "--plan"], on: On::Repo, exits: &[0], contains: ANY, hint: "run `goatbuilder assay {goatnet} --plan`; it derives the site's values from its group_vars and lists the rows, touching no host." }],
        needs_verb: Some("assay"),
        needs_path: None,
        prereq: Some("`assay` needs Python with PyYAML. Check: python3 -c 'import yaml'. If that fails, install it once: pip install pyyaml"),
    },
    Lesson {
        title: "Survey: are the versions consistent?",
        text: "The goatbuilder `survey` command checks in one pass whether the same versions run where they should:\n- Inside each goatnet: does every machine run the declared version, and do machines that should match actually match?\n- Across dev, preprod and prod: one row per component, one column per goatnet.\n\nA difference that the release process expects is reported but is not an error. An example is a new version that is on dev but not on prod yet. A difference that nothing declares reads as `difference not declared`. A machine goatbuilder cannot read is unknown, never consistent.\n\nRun it on the practice copy. Its `dev` is behind on purpose, so expect findings and exit code 3.",
        setup: &[],
        expects: &[Expect { label: "Survey the practice copy", command: "goatbuilder --mock {ws}/fixture survey", words: &["survey"], on: On::Fixture("fixture"), exits: &[0, 3], contains: ANY, hint: "run it on the tutor's copy with --mock {ws}/fixture; the fixture drifts on purpose, so expect findings and exit 3." }],
        needs_verb: Some("survey"),
        needs_path: None,
        prereq: None,
    },
    Lesson {
        title: "Endstate: does a goatnet run exactly what it declares?",
        text: "The goatbuilder `endstate` command checks two things about a goatnet:\n1. Every service it declares is deployed, running, healthy and correctly connected.\n2. Nothing runs that it does not declare. An undeclared service is called an orphan.\n\nThe goatbuilder CLI works out what a site should run from the site's own settings, not from a separate list.\n\nA check that has nothing to read is marked UNOBSERVED. That is an honest answer, not a failure. A FAIL or an orphan makes the exit code 3.\n\nThe general form is `goatbuilder endstate <goatnet>`. The command below checks an example site from saved data, so it touches no machine.",
        setup: &[],
        expects: &[Expect { label: "Check the example site", command: "goatbuilder endstate alpha --root ops/goatbuilder/fixtures/endstate/single-host --facts ops/goatbuilder/fixtures/endstate/single-host/facts.json --assay-rows ops/goatbuilder/fixtures/endstate/single-host/assay.jsonl", words: &["endstate", "alpha", "--root"], on: On::Repo, exits: &[0], contains: ANY, hint: "run the long form shown: --root points endstate at the fixture tree, --facts and --assay-rows read its recorded observations instead of a live goatnet, so it grades offline." }],
        needs_verb: Some("endstate"),
        needs_path: Some("ops/goatbuilder/fixtures/endstate/single-host"),
        prereq: Some("`endstate` needs Python with PyYAML. Check: python3 -c 'import yaml'. If that fails, install it once: pip install pyyaml"),
    },
    Lesson {
        title: "Bonus: be curious",
        text: "This lesson is optional. Run the command below and the tutor is finished.\n\nThe goatbuilder CLI has a few small hidden features. They are not in the help or in these lessons. Some answer to words that long-time Unix users often type into a new program, just to see what happens. Most things you try will do nothing.\n\nThis lesson does not say what they are. Finding them is the point.\n\nThe command below lists every error that goatbuilder can explain. It is a good place to start exploring.",
        setup: &[],
        expects: &[Expect { label: "List every error goatbuilder can explain", command: "goatbuilder explain --list", words: &["explain", "--list"], on: On::Repo, exits: &[0], contains: ANY, hint: "run `goatbuilder explain --list`; it reads only, and finishing it finishes the tutor." }],
        needs_verb: Some("explain"),
        needs_path: None,
        prereq: None,
    },
];

/// The goatnet, site and component the lessons that read the checkout run
/// against, taken from that checkout so a lesson works in any project.
static NAMES: std::sync::OnceLock<(String, String, String)> = std::sync::OnceLock::new();

/// Read the names once, from `goatbuilder.yaml`: the first stage of the
/// promotion path (else the first environment), its first site (else the
/// goatnet with hyphens), and the first component with a role and its own
/// deploy play.
pub fn init(repo: &Path) {
    let _ = NAMES.set(names(repo));
}

fn names(repo: &Path) -> (String, String, String) {
    let v = crate::config::load_value(&repo.join("ops/goatbuilder/goatbuilder.yaml")).ok();
    let str_at = |p: &str| v.as_ref().and_then(|v| v.pointer(p)).and_then(|x| x.as_str()).map(String::from);
    let goatnet = str_at("/promotion/path/0/environment").or_else(|| str_at("/environments/0/name")).unwrap_or_else(|| "<goatnet>".into());
    let site = v
        .as_ref()
        .and_then(|v| v.get("environments")?.as_array()?.iter().find(|e| e.get("name").and_then(|n| n.as_str()) == Some(goatnet.as_str())).cloned())
        .and_then(|e| e.pointer("/sites/0/name").and_then(|n| n.as_str()).map(String::from))
        .unwrap_or_else(|| goatnet.replace('_', "-"));
    let component = v
        .as_ref()
        .and_then(|v| v.get("components")?.as_object().cloned())
        .and_then(|c| {
            c.into_iter()
                .map(|(name, spec)| (spec.get("ledger").and_then(|l| l.as_str()).unwrap_or(&name).to_string(), name))
                .find(|(ledger, name)| repo.join("ops/ansible/roles").join(ledger).is_dir() && repo.join(format!("ops/ansible/{name}-deploy.yml")).is_file())
                .map(|(_, name)| name)
        })
        .unwrap_or_else(|| "<component>".into());
    (goatnet, site, component)
}

/// A lesson's command or hint with the workspace and the checkout's names in it.
fn with_names(text: &str, ws: &str) -> String {
    // A unit test runs no main, so it reads the repository it is built in.
    #[cfg(test)]
    let _ = NAMES.set(names(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")));
    let (g, site, c) = NAMES.get().map(|(g, s, c)| (g.as_str(), s.as_str(), c.as_str())).unwrap_or(("<goatnet>", "<site>", "<component>"));
    text.replace("{ws}", ws).replace("{goatnet}", g).replace("{site}", site).replace("{component}", c)
}

/// One logged run.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Run {
    pub args: Vec<String>,
    #[serde(default)]
    pub mock: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    pub exit: i32,
    #[serde(default)]
    pub rows: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Progress {
    done: Vec<usize>,
    /// Lesson number → how many runs were logged when it was opened; only
    /// later runs count for it.
    opened: BTreeMap<usize, usize>,
}

pub fn default_workspace() -> PathBuf {
    if let Some(p) = std::env::var_os("GOATBUILDER_TUTOR") {
        return PathBuf::from(p);
    }
    home_workspace()
}

/// `~/.goatbuilder/tutor`: where the tutor lives when nobody names a workspace,
/// and where a run logs itself without the learner exporting anything.
fn home_workspace() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".goatbuilder/tutor")
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn workspace_of(p: &Path) -> Option<PathBuf> {
    let mut cur = Some(canon(p));
    while let Some(c) = cur {
        if c.join(MARKER).is_file() {
            return Some(c);
        }
        cur = c.parent().map(Path::to_path_buf);
    }
    None
}

/// Append this run to the tutor's log when it belongs to a workspace.
/// Log a run for the tutor, and say so when it ticked a step: the returned
/// line goes under the command's own output (table output only), so the
/// learner sees the step done and what to do next without asking.
pub fn log_run(cli: &Cli, report: &Result<Report>) -> Option<String> {
    let named = std::env::var_os("GOATBUILDER_TUTOR")
        .map(PathBuf::from)
        .filter(|p| p.join(MARKER).is_file())
        .or_else(|| cli.mock.as_deref().and_then(workspace_of))
        .or_else(|| cli.repo.as_deref().and_then(workspace_of));
    // Nothing named a workspace: the learner's own, if the tutor made one and
    // has lessons left. Asking for an export first only ever meant every step
    // silently failing to tick for whoever forgot it.
    let ws = named.clone().or_else(|| {
        let home = home_workspace();
        let open = home.join(MARKER).is_file() && load(&home).done.len() < LESSONS.len();
        open.then_some(home)
    })?;
    let (exit, rows) = match report {
        Ok(r) => (r.exit, r.checks.iter().map(|c| format!("{} {}", c.name, c.detail)).collect()),
        Err(e) => {
            let mut rows = vec![format!("error {e:#}")];
            if let Some(c) = e.downcast_ref::<goat_errors::Coded>() {
                rows.push(format!("code {}", c.code));
            }
            // The exit the process really ends with: the coded error's own
            // (main.rs maps it the same way), else usage.
            let exit = e
                .downcast_ref::<goat_errors::Coded>()
                .and_then(|c| goat_errors::explain(&c.code))
                .and_then(|x| x.exit)
                .unwrap_or(crate::EXIT_USAGE);
            (exit, rows)
        }
    };
    let run = Run {
        args: std::env::args().skip(1).collect(),
        mock: cli.mock.as_deref().map(|p| canon(p).display().to_string()),
        repo: cli.repo.as_deref().map(|p| canon(p).display().to_string()),
        exit,
        rows,
    };
    if let Ok(line) = serde_json::to_string(&run) {
        append_run(&ws, &line);
    }
    // A next-step hint: stderr, a terminal, a person reading a table, and
    // GOAT_NO_FUN unset (the Three Rules of goat delight).
    if !matches!(cli.output, OutputFormat::Table) || !fun_on(std::io::IsTerminal::is_terminal(&std::io::stderr())) {
        return None;
    }
    let color = crate::output::want_color(cli.color);
    // A step that ticked says so, wherever it was run from: a learner who
    // forgot the export is still doing the lesson.
    //
    // A command that failed and ticked nothing is told which lesson is open --
    // but only when the run itself named the tutor's workspace. Otherwise this
    // is an operator at work on their own goatnet, whose failed command has
    // nothing to do with a lesson, and saying so on every failure is noise.
    step_done(&ws, color).or_else(|| (exit != 0 && named.is_some()).then(|| where_you_are(&ws, color)).flatten())
}

/// The line to print when the run just logged ticked a step of the lesson
/// that is open: which step, or that the lesson is complete, and the one
/// command that moves on. Nothing when the run ticked nothing.
/// Append one run to the tutor's log, which is the learner's own record of
/// what they ran: readable by them alone (it holds the commands they typed and
/// what every check said), and capped, since it is a teaching aid and not an
/// audit trail. A log that cannot be written is not an error worth a word: the
/// lesson still works, only the next page has less to go on.
fn append_run(ws: &Path, line: &str) {
    use std::io::Write;
    const CAP: u64 = 1 << 20; // a megabyte
    const KEEP: usize = 200; // a run can be a kilobyte, so a trim leaves room
    let path = ws.join("log.jsonl");
    private(ws, true);
    if std::fs::metadata(&path).map(|m| m.len() > CAP).unwrap_or(false) {
        // The newest runs are the ones the next page reads.
        if let Ok(text) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            let tail = lines[lines.len().saturating_sub(KEEP)..].join("\n");
            let _ = std::fs::write(&path, format!("{tail}\n"));
        }
    }
    let mut open = std::fs::OpenOptions::new();
    open.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    if let Ok(mut f) = open.open(&path) {
        let _ = writeln!(f, "{line}");
        private(&path, false);
    }
}

/// Keep a path in the tutor's workspace to its owner. Directories need the
/// execute bit to be entered at all, so they take 0700 and files take 0600.
fn private(path: &Path, dir: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let want = if dir { 0o700 } else { 0o600 };
        if let Ok(m) = std::fs::metadata(path) {
            if m.permissions().mode() & 0o777 != want {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(want));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (path, dir);
}

/// Which lesson is open, for a learner who ran something that failed. `None`
/// once every lesson is done: then nothing is waiting for them.
fn where_you_are(ws: &Path, color: bool) -> Option<String> {
    let progress = load(ws);
    let n = (1..=LESSONS.len()).find(|k| !progress.done.contains(k))?;
    let line = crate::i18n::prose_args(
        "tutor-where",
        format!("That command is not part of lesson {n}. Run `goatbuilder tutor` to see where you are."),
        &[("n", n.to_string())],
    );
    Some(ink(&format!("\n{}", fill(&line, "", "").trim_end()), color))
}

fn step_done(ws: &Path, color: bool) -> Option<String> {
    use crate::i18n::prose_args;
    let progress = load(ws);
    let n = *progress.opened.keys().filter(|k| !progress.done.contains(k)).max()?;
    let l = LESSONS.get(n.checked_sub(1)?)?;
    let runs = read_runs(ws);
    let since = progress.opened[&n].min(runs.len());
    let after = evaluate(l, &runs[since..], ws).0;
    let before = evaluate(l, &runs[since..runs.len().saturating_sub(1).max(since)], ws).0;
    if after <= before {
        return None;
    }
    let total = l.expects.len();
    let text = if after < total {
        prose_args(
            "tutor-after-step",
            format!("Step {after} of {total} done. Next:"),
            &[("met", after.to_string()), ("total", total.to_string())],
        )
    } else if n < LESSONS.len() {
        prose_args(
            "tutor-after-lesson",
            format!("Lesson {n} is complete. Run `goatbuilder tutor` to start lesson {}. To read it again: `goatbuilder tutor --lesson {n}`.", n + 1),
            &[("n", n.to_string()), ("next", (n + 1).to_string())],
        )
    } else {
        prose_args(
            "tutor-after-final",
            format!("Lesson {n} is complete, and so is the tutor. Run `goatbuilder tutor` to see it marked done. To read it again: `goatbuilder tutor --lesson {n}`."),
            &[("n", n.to_string())],
        )
    };
    // Wrapped before the mark is coloured, so colour codes never count as width.
    let page = fill(&text, &format!("{DONE} Tutor: "), "");
    let page = if color { page.replacen(DONE, &format!("{GREEN}{DONE}{RESET}"), 1) } else { page };
    let mut out = format!("\n{}", page.trim_end());
    // The next step's command, so the learner keeps going without leaving the flow.
    if let Some(next) = l.expects.get(after).filter(|_| after < total) {
        let label = crate::i18n::prose(&format!("tutor-l{n}-s{}", after + 1), next.label);
        let command = shell_wrap(&with_names(next.command, &ws.display().to_string()), "    ");
        out.push_str(&format!("\n\n{}\n\n", fill(&format!("{label}:"), "", "").trim_end()));
        out.push_str(&if color { format!("{BOLD}{}{RESET}", command.trim_end()) } else { command.trim_end().to_string() });
    }
    Some(ink(&out, color))
}

fn read_runs(ws: &Path) -> Vec<Run> {
    std::fs::read_to_string(ws.join("log.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn ran_on(run: &Run, on: On, ws: &Path) -> bool {
    let ws = canon(ws);
    match on {
        On::Fixture(name) => run.mock.as_deref().map(|m| Path::new(m) == ws.join(name)).unwrap_or(false),
        On::Skeleton => run.repo.as_deref().map(|r| Path::new(r) == ws.join("repo")).unwrap_or(false),
        On::Repo => run.mock.is_none() && run.repo.as_deref().map(|r| !Path::new(r).starts_with(&ws)).unwrap_or(true),
    }
}

/// Why `run` does not satisfy `e`, or None when it does.
fn miss(run: &Run, e: &Expect, ws: &Path) -> Option<String> {
    if let Some(w) = e.words.iter().find(|w| !run.args.iter().any(|a| a == *w)) {
        return Some(format!("it has no `{w}`"));
    }
    if !ran_on(run, e.on, ws) {
        return Some(match e.on {
            On::Fixture(n) => format!("it did not run on the tutor's copy (--mock {}/{n})", ws.display()),
            On::Skeleton => format!("it did not run on the skeleton copy (-C {}/repo)", ws.display()),
            On::Repo => "it ran inside the tutor's workspace; this step runs on your checkout".into(),
        });
    }
    if !e.exits.contains(&run.exit) {
        return Some(format!("it exited {}, the lesson expects {}", run.exit, e.exits.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" or ")));
    }
    let all = run.rows.join("\n");
    if let Some(c) = e.contains.iter().find(|c| !all.contains(*c)) {
        return Some(format!("its rows do not say \"{c}\""));
    }
    None
}

/// Walk the runs in order against the lesson's expectations. Returns how many
/// are met and, when a run of the next expected verb missed, why.
pub fn evaluate(lesson: &Lesson, runs: &[Run], ws: &Path) -> (usize, Option<String>) {
    let mut met = 0;
    // The most useful reason wins: a run with every word of the step that
    // failed on place, exit or rows says more than one that merely shares the
    // verb (a `--from prod` run is no news while the `--from dev` step is due).
    let mut close_miss: Option<String> = None;
    let mut verb_miss: Option<String> = None;
    for run in runs {
        let Some(e) = lesson.expects.get(met) else { break };
        let verb_matches = run.args.iter().any(|a| a == e.words[0]);
        let all_words = e.words.iter().all(|w| run.args.iter().any(|a| a == w));
        match miss(run, e, ws) {
            None => {
                met += 1;
                close_miss = None;
                verb_miss = None;
            }
            Some(_) if (met + 1..lesson.expects.len()).any(|j| miss(run, &lesson.expects[j], ws).is_none()) => {
                // A later step done early: steps count in order, so say so
                // rather than leaving the reader to wonder why it did not tick.
                let j = (met + 1..lesson.expects.len()).find(|&j| miss(run, &lesson.expects[j], ws).is_none()).unwrap_or(met);
                close_miss = Some(format!(
                    "you ran step {} before step {}. Steps count in order: do step {} first, then run step {} again",
                    j + 1,
                    met + 1,
                    met + 1,
                    j + 1
                ));
            }
            Some(why) => {
                let refusal = run.rows.iter().find(|r| r.contains("REFUSE") || r.contains("refused") || r.starts_with("error ")).map(|r| format!(" It said: {}.", r.chars().take(200).collect::<String>())).unwrap_or_default();
                let text = format!("you ran `goatbuilder {}`: {why}.{refusal} {}", run.args.join(" "), with_names(e.hint, &ws.display().to_string()));
                if all_words {
                    close_miss = Some(text);
                } else if verb_matches {
                    verb_miss = Some(text);
                }
            }
        }
    }
    (met, close_miss.or(verb_miss))
}

fn available(lesson: &Lesson, repo: &Path) -> std::result::Result<(), String> {
    if let Some(v) = lesson.needs_verb {
        if Cli::command().find_subcommand(v).is_none() {
            return Err(format!("needs the `{v}` verb, which this build does not have"));
        }
    }
    if let Some(p) = lesson.needs_path {
        if !repo.join(p).is_dir() {
            return Err(format!("needs {p} in the checkout"));
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from).with_context(|| from.display().to_string())? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dst)?;
        } else {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// A fresh workspace: the marker, a copy of each fixture, and a skeleton of the
/// repository with one generated pins.yml edited by hand (lesson 8's drift).
fn setup(repo: &Path, ws: &Path) -> Result<()> {
    std::fs::create_dir_all(ws)?;
    std::fs::write(ws.join(MARKER), "goatbuilder tutor workspace. Safe to delete; `goatbuilder tutor --reset` recreates it.\n")?;
    copy_dir(&repo.join("ops/goatbuilder/fixtures/tutor"), &ws.join("fixture"))?;
    for (src, dst) in [("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder", "zagros-1"), ("ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node", "zagros-2"), ("ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node", "zagros-3")] {
        if repo.join(src).is_dir() {
            copy_dir(&repo.join(src), &ws.join(dst))?;
        }
    }
    std::fs::create_dir_all(ws.join("federate"))?;
    let sk = ws.join("repo");
    for d in ["ops/pins", "ops/ansible/group_vars", "ops/consistency/schema", "ops/consistency/intent", "ops/enrollment/public-keys", "ops/goatbuilder/schema"] {
        if repo.join(d).is_dir() {
            copy_dir(&repo.join(d), &sk.join(d))?;
        }
    }
    // Lesson 1 validates the skeleton's config. A deployment checkout has its
    // own; a public clone carries none (it is a deployment's file), so the
    // skeleton takes the worked example's, and lesson 1 works from any clone.
    let own = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let example = repo.join("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder/goatbuilder.yaml");
    if let Some(src) = [own, example].into_iter().find(|p| p.is_file()) {
        std::fs::create_dir_all(sk.join("ops/goatbuilder"))?;
        std::fs::copy(src, sk.join("ops/goatbuilder/goatbuilder.yaml"))?;
    }
    if let Ok(rd) = std::fs::read_dir(repo.join("ops/ansible/roles")) {
        for role in rd.flatten() {
            let defaults = role.path().join("defaults");
            if defaults.is_dir() {
                copy_dir(&defaults, &sk.join("ops/ansible/roles").join(role.file_name()).join("defaults"))?;
            }
        }
    }
    save(ws, &Progress::default())
}

/// Lesson 8's drift: restore every generated pins.yml of the skeleton from the
/// checkout, then flip one hex digit of the first digest in the first of them —
/// a hand edit `pins --check` must find.
fn inject_drift(repo: &Path, ws: &Path) -> Result<()> {
    let sk = ws.join("repo");
    copy_dir(&repo.join("ops/ansible/group_vars"), &sk.join("ops/ansible/group_vars"))?;
    copy_dir(&repo.join("ops/consistency/intent"), &sk.join("ops/consistency/intent"))?;
    let mut gv: Vec<PathBuf> = std::fs::read_dir(sk.join("ops/ansible/group_vars")).map(|r| r.flatten().map(|e| e.path().join("pins.yml")).filter(|p| p.is_file()).collect()).unwrap_or_default();
    gv.sort();
    if let Some(p) = gv.into_iter().find(|p| std::fs::read_to_string(p).map(|t| t.contains("@sha256:")).unwrap_or(false)) {
        let t = std::fs::read_to_string(&p)?;
        let i = t.find("@sha256:").expect("checked") + 8;
        let swapped = if &t[i..i + 1] == "0" { "1" } else { "0" };
        std::fs::write(&p, format!("{}{}{}", &t[..i], swapped, &t[i + 1..]))?;
    }
    Ok(())
}

/// The state a lesson needs, made when it is first opened, so earlier
/// experiments in the workspace cannot leave it unwinnable.
fn prepare(n: usize, repo: &Path, ws: &Path) -> Result<()> {
    let fresh = |src: &str, dst: &str| -> Result<()> {
        let d = ws.join(dst);
        if d.exists() {
            std::fs::remove_dir_all(&d)?;
        }
        copy_dir(&repo.join(src), &d)
    };
    match LESSONS[n - 1].title {
        t if t.starts_with("Pins:") => inject_drift(repo, ws),
        t if t.starts_with("Tutorial 1:") => fresh("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder", "zagros-1"),
        t if t.starts_with("Tutorial 2:") => fresh("ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node", "zagros-2"),
        _ => Ok(()),
    }
}

/// Open lesson `n` once: note where its runs start, and prepare its state.
fn open(n: usize, progress: &mut Progress, runs: usize, repo: &Path, ws: &Path) -> Result<usize> {
    if let Some(s) = progress.opened.get(&n) {
        return Ok(*s);
    }
    prepare(n, repo, ws)?;
    progress.opened.insert(n, runs);
    save(ws, progress)?;
    Ok(runs)
}

fn load(ws: &Path) -> Progress {
    std::fs::read_to_string(ws.join("progress.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(ws: &Path, p: &Progress) -> Result<()> {
    std::fs::write(ws.join("progress.json"), serde_json::to_string_pretty(p)?)?;
    Ok(())
}

fn row(name: String, ok: bool, detail: String) -> Check {
    Check { name, ok, detail, fix: None }
}

/// The printed lesson page's fixed lines, as Fluent entries with their
/// variables; the English text is the `format!` each call site uses, so English
/// output is unchanged byte for byte. Report rows stay English (i18n.rs doc).
#[cfg(test)]
pub const CHROME: &[(&str, &str)] = &[
    ("tutor-heading", "Lesson { $n } of { $total } · { $title }"),
    ("tutor-setup", "First, run this once in this terminal:"),
    ("tutor-not-recording", "This terminal is not recording your runs, so the tutor cannot tick your steps. Run this once, then run the steps:"),
    ("tutor-type", "Do these steps in your terminal ({ $met } of { $total } done):"),
    ("tutor-prereq", "Before you start: { $prereq }"),
    ("tutor-not-yet", "Not yet: { $why }"),
    ("tutor-rerun", "After each step, run `goatbuilder tutor` again. It checks what you did and ticks the step."),
    ("tutor-review", "You finished this lesson, so this page is only to read. Run `goatbuilder tutor` to go back to where you are."),
    ("tutor-where", "That command is not part of lesson { $n }. Run `goatbuilder tutor` to see where you are."),
    ("tutor-other-lessons", "To read a lesson again: `goatbuilder tutor --lesson 1`. To see them all: `goatbuilder tutor --list`."),
    ("tutor-welcome", "Welcome to GOATBUILDER Tutor"),
    ("tutor-bar-progress", "Progress"),
    ("tutor-bar-done", "{ $done } of { $total } lessons done"),
    ("tutor-after-step", "Step { $met } of { $total } done. Next:"),
    ("tutor-after-lesson", "Lesson { $n } is complete. Run `goatbuilder tutor` to start lesson { $next }. To read it again: `goatbuilder tutor --lesson { $n }`."),
    ("tutor-after-final", "Lesson { $n } is complete, and so is the tutor. Run `goatbuilder tutor` to see it marked done. To read it again: `goatbuilder tutor --lesson { $n }`."),
    ("tutor-lesson-done", "Lesson { $n } done: { $title }."),
    ("tutor-all-done-reset", "Every lesson is done. `goatbuilder tutor --reset` starts over; the tutorials go further: docs/tutorials/."),
    ("tutor-all-done", "Every lesson is done. The tutorials go further: docs/tutorials/."),
    ("tutor-workspace", "Workspace: { $path }"),
];

/// Every lesson's prose as (catalog id, English): titles and texts. Commands,
/// the scripts-avenue line, codes and paths are never translated.
#[cfg(test)]
pub fn prose_ids() -> Vec<(String, String)> {
    let mut v = Vec::new();
    for (i, l) in LESSONS.iter().enumerate() {
        v.push((format!("tutor-l{}-title", i + 1), l.title.to_string()));
        v.push((format!("tutor-l{}-text", i + 1), l.text.to_string()));
        for (k, e) in l.expects.iter().enumerate() {
            v.push((format!("tutor-l{}-s{}", i + 1, k + 1), e.label.to_string()));
        }
    }
    v
}

fn title(n: usize, l: &Lesson) -> String {
    crate::i18n::prose(&format!("tutor-l{n}-title"), l.title)
}

/// The page width a lesson wraps to: the standard 80-column terminal (the
/// Google style guides' line limit). Fixed rather than read from the
/// terminal, so a page reads the same everywhere and tests can pin it.
const WIDTH: usize = 80;
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DONE: &str = "\u{2713}"; // check mark
const TODO: &str = "\u{2717}"; // ballot x

/// Whether a flourish may print (docs/design/goat-cli-onboarding-and-delight.md,
/// the Three Rules of goat delight): only to a terminal, never to a pipe or a
/// file, and never when GOAT_NO_FUN is set.
fn fun_on(stream_is_terminal: bool) -> bool {
    stream_is_terminal && std::env::var_os("GOAT_NO_FUN").is_none()
}

/// The banner on the first lesson, and one progress line over every page: how far through
/// the tutor you are. The goat on the welcome is a flourish (fun_on); the
/// welcome and the progress line always print.
fn status(n: usize, done: usize, color: bool) -> String {
    status_for(n, done, color, fun_on(std::io::IsTerminal::is_terminal(&std::io::stdout())))
}

fn status_for(n: usize, done: usize, color: bool, fun: bool) -> String {
    use crate::i18n::{prose, prose_args};
    let total = LESSONS.len();
    let done = done.min(total);
    let mut s = String::new();
    // The first page welcomes the learner, and the goat leads it there. The
    // progress line below is the same on every page, and carries no flourish.
    if n == 1 {
        // Two spaces: the goat is a wide glyph, and one space reads as none.
        let goat = if fun { "\u{1f410}  " } else { "" };
        let welcome = prose("tutor-welcome", "Welcome to GOATBUILDER Tutor");
        let welcome = if color { format!("{BOLD}{welcome}{RESET}") } else { welcome };
        s.push_str(&format!("{goat}{welcome}\n\n"));
    }
    let filled = "\u{2588}".repeat(done);
    let empty = "\u{2591}".repeat(total - done);
    let bar = if color { format!("{GREEN}{filled}{RESET}{empty}") } else { format!("{filled}{empty}") };
    let label = prose("tutor-bar-progress", "Progress");
    let count = prose_args("tutor-bar-done", format!("{done} of {total} lessons done"), &[("done", done.to_string()), ("total", total.to_string())]);
    s.push_str(&format!("{label}: {bar}  {count}\n"));
    s
}

/// Visible width: backticks are markup and are never printed.
fn vis(s: &str) -> usize {
    s.chars().filter(|&c| c != '`').count()
}

/// Wrap one run of prose to `WIDTH`, every line indented by `first` (the first
/// line) or `rest` (the others). Words are never split.
pub(crate) fn fill(words: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    let mut line = first.to_string();
    let mut lw = vis(first);
    let mut empty = true;
    for w in words.split_whitespace() {
        let ww = vis(w);
        if !empty && lw + 1 + ww > WIDTH {
            out.push_str(line.trim_end());
            out.push('\n');
            line = rest.to_string();
            lw = vis(rest);
            empty = true;
        }
        if !empty {
            line.push(' ');
            lw += 1;
        }
        line.push_str(w);
        lw += ww;
        empty = false;
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}

/// A list item's marker ("1. ", "- "), if the line starts with one.
fn marker(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ") {
        let _ = rest;
        return Some("- ");
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && line[digits..].starts_with(". ") {
        return Some(&line[..digits + 2]);
    }
    None
}

/// Lay out a lesson's text for a terminal: flush left, wrapped to WIDTH, with a
/// blank line between paragraphs and around a list, so a page never reads as one
/// block.
fn layout(text: &str) -> String {
    let mut out = String::new();
    let mut prose = String::new();
    let mut in_list = false;
    let flush = |prose: &mut String, out: &mut String| {
        if !prose.is_empty() {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&fill(prose, "", ""));
            prose.clear();
        }
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut prose, &mut out);
            continue;
        }
        match marker(line) {
            Some(m) => {
                flush(&mut prose, &mut out);
                if !in_list && !out.is_empty() {
                    out.push('\n');
                }
                in_list = true;
                let hang = " ".repeat(2 + m.chars().count());
                out.push_str(&fill(&line[m.len()..], &format!("  {m}"), &hang));
            }
            None => {
                in_list = false;
                if !prose.is_empty() {
                    prose.push(' ');
                }
                prose.push_str(line.trim());
            }
        }
    }
    flush(&mut prose, &mut out);
    out
}

/// Split a shell command into words, keeping quoted strings and `$(...)`
/// whole, so a line break never lands inside one.
fn shell_words(c: &str) -> Vec<String> {
    let (mut words, mut cur) = (Vec::new(), String::new());
    let (mut quote, mut depth) = (None::<char>, 0usize);
    let mut prev = ' ';
    for ch in c.chars() {
        match ch {
            '"' | '\'' if quote.is_none() => quote = Some(ch),
            q if Some(q) == quote => quote = None,
            '(' if prev == '$' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ' ' if quote.is_none() && depth == 0 => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                prev = ch;
                continue;
            }
            _ => {}
        }
        cur.push(ch);
        prev = ch;
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Print a command at `pad`, breaking it with ` \` when it would pass WIDTH.
/// It breaks only before an option, so a `--flag value` pair (and the command
/// with its leading arguments) stays on one line; only a group that is itself
/// too long breaks between its words. Continuation lines are indented two
/// more. bash and zsh read the result as the same one command.
pub(crate) fn shell_wrap(c: &str, pad: &str) -> String {
    // Groups: the leading words, then one group per option and the words that
    // follow it up to the next option.
    let mut groups: Vec<Vec<String>> = Vec::new();
    for w in shell_words(c) {
        if w.starts_with('-') || groups.is_empty() {
            groups.push(vec![w]);
        } else {
            groups.last_mut().expect("a group").push(w);
        }
    }
    let cont = format!("{pad}  ");
    let room = WIDTH.saturating_sub(2); // keep space for " \\"
    let mut out = String::new();
    let mut line = pad.to_string();
    let mut at_start = true;
    let put = |w: &str, line: &mut String, out: &mut String, at_start: &mut bool| {
        if !*at_start && line.chars().count() + 1 + w.chars().count() > room {
            out.push_str(line);
            out.push_str(" \\\n");
            *line = cont.clone();
            *at_start = true;
        }
        if !*at_start {
            line.push(' ');
        }
        line.push_str(w);
        *at_start = false;
    };
    for (k, g) in groups.iter().enumerate() {
        let joined = g.join(" ");
        let fits_here = line.chars().count() + 1 + joined.chars().count() <= room;
        let fits_alone = cont.chars().count() + joined.chars().count() <= room;
        // Never leave the command name alone on the first line: when the next
        // group does not fit beside it, fill that group in word by word.
        if k == 1 && !fits_here && groups[0].len() == 1 {
            for w in g {
                put(w, &mut line, &mut out, &mut at_start);
            }
            continue;
        }
        if !at_start && !fits_here && fits_alone {
            out.push_str(&line);
            out.push_str(" \\\n");
            line = cont.clone();
            at_start = true;
        }
        if fits_alone || at_start && line.chars().count() + joined.chars().count() <= room {
            if !at_start {
                line.push(' ');
            }
            line.push_str(&joined);
            at_start = false;
        } else {
            for w in g {
                put(w, &mut line, &mut out, &mut at_start);
            }
        }
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// Backticked spans print bold on a colour terminal and plain otherwise; the
/// backticks themselves never print.
fn ink(s: &str, color: bool) -> String {
    let mut out = String::new();
    let mut open = false;
    for c in s.chars() {
        if c == '`' {
            open = !open;
            if color {
                out.push_str(if open { BOLD } else { RESET });
            }
        } else {
            out.push(c);
        }
    }
    if open && color {
        out.push_str(RESET);
    }
    out
}

fn render(n: usize, l: &Lesson, ws: &Path, met: usize, why: Option<&str>, color: bool, done: usize) -> String {
    use crate::i18n::{prose, prose_args};
    let ws_s = ws.display().to_string();
    let t = title(n, l);
    let heading = prose_args(
        "tutor-heading",
        format!("Lesson {n} of {} · {}", LESSONS.len(), l.title),
        &[("n", n.to_string()), ("total", LESSONS.len().to_string()), ("title", t)],
    );
    let rule = "─".repeat(heading.chars().count().min(WIDTH));
    let head = if color { format!("{BOLD}{heading}{RESET}") } else { heading };
    let mut s = format!("\n{}\n{head}\n{rule}\n\n", status(n, done, color));
    s.push_str(&layout(&prose(&format!("tutor-l{n}-text"), l.text)));
    if let Some(p) = l.prereq {
        let line = prose_args("tutor-prereq", format!("Before you start: {p}"), &[("prereq", p.to_string())]);
        s.push('\n');
        s.push_str(&fill(&line, "", ""));
    }
    // Commands are never wrapped, so each stays one line to copy.
    let cmd = |c: &str, pad: &str| {
        let c = shell_wrap(&with_names(c, &ws_s), pad);
        if color { format!("{BOLD}{c}{RESET}") } else { c }
    };
    // A run is recorded only where $GOATBUILDER_TUTOR names this workspace
    // (log_run). A new terminal does not have it, and nothing else would say
    // so: the steps would just never tick. Every page says it when it is missing.
    // A run logs itself to the learner's own workspace with no export; only a
    // workspace somewhere else needs one, and the page below says so.
    let recording = std::env::var_os("GOATBUILDER_TUTOR").is_some_and(|v| Path::new(&v) == ws) || canon(ws) == canon(&home_workspace());
    let reading = met == l.expects.len();
    if reading {
        // A finished lesson is here to read: nothing to set up or record.
    } else if !l.setup.is_empty() {
        s.push_str(&format!("\n{}\n", prose("tutor-setup", "First, run this once in this terminal:")));
        for c in l.setup {
            s.push_str(&cmd(c, "  "));
        }
    } else if !recording {
        s.push('\n');
        s.push_str(&fill(&prose("tutor-not-recording", "This terminal is not recording your runs, so the tutor cannot tick your steps. Run this once, then run the steps:"), "", ""));
        s.push_str(&cmd("export GOATBUILDER_TUTOR={ws}", "  "));
    }
    let total = l.expects.len();
    let head = prose_args(
        "tutor-type",
        format!("Do these steps in your terminal ({met} of {total} done):"),
        &[("met", met.to_string()), ("total", total.to_string())],
    );
    s.push_str(&format!("\n{head}\n"));
    // One list: each step's instruction, its command and its mark: a green
    // check when done, a red x until then. Never the word FAIL: the ok/FAIL
    // table is for results. "Not yet" sits under the step it is about.
    for (k, e) in l.expects.iter().enumerate() {
        let label = prose(&format!("tutor-l{n}-s{}", k + 1), e.label);
        let mark = if k < met { DONE } else { TODO };
        let first = format!("  {mark} {}. ", k + 1);
        let pad = " ".repeat(first.chars().count());
        s.push('\n');
        // The instruction ends in a colon, then a blank line, then its command.
        // The mark is coloured after wrapping, so colour codes never count as width.
        let step = fill(&format!("{label}:"), &first, &pad);
        let step = if color {
            step.replacen(mark, &format!("{}{mark}{RESET}", if k < met { GREEN } else { RED }), 1)
        } else {
            step
        };
        s.push_str(&step);
        s.push('\n');
        s.push_str(&cmd(e.command, &pad));
        if k == met {
            if let Some(w) = why {
                s.push_str(&fill(&prose_args("tutor-not-yet", format!("Not yet: {w}"), &[("why", w.to_string())]), &pad, &pad));
            }
        }
    }
    s.push('\n');
    if reading {
        s.push_str(&fill(&prose("tutor-review", "You finished this lesson, so this page is only to read. Run `goatbuilder tutor` to go back to where you are."), "", ""));
    } else {
        s.push_str(&fill(&prose("tutor-rerun", "After each step, run `goatbuilder tutor` again. It checks what you did and ticks the step."), "", ""));
    }
    s.push('\n');
    s.push_str(&fill(&prose("tutor-other-lessons", "To read a lesson again: `goatbuilder tutor --lesson 1`. To see them all: `goatbuilder tutor --list`."), "", ""));
    ink(&s, color)
}

/// `--list` as a person reads it: number, title and state in columns.
fn list_page(report: &Report, color: bool) -> String {
    let parsed: Vec<(String, String, String)> = report
        .checks
        .iter()
        .map(|c| {
            let num = c.name.trim_start_matches("lesson ").to_string();
            // 62 characters of title, then the state. A translated title can
            // put a multi-byte character across byte 62, which splitting by
            // byte would panic on.
            let cut = c.detail.char_indices().nth(62).map_or(c.detail.len(), |(i, _)| i);
            let (title, state) = c.detail.split_at(cut);
            (num, title.trim_end().to_string(), state.trim().to_string())
        })
        .collect();
    let width = parsed.iter().map(|(_, t, _)| t.chars().count()).max().unwrap_or(5);
    let head = format!("{:>2}  {:<width$}  STATE", "#", "LESSON");
    let mut out = if color { format!("{BOLD}{head}{RESET}") } else { head };
    for (num, title, state) in &parsed {
        let shown = match (color, state.as_str()) {
            (true, "done") => format!("{GREEN}{state}{RESET}"),
            (true, "next") => format!("{BOLD}{state}{RESET}"),
            _ => state.clone(),
        };
        out.push_str(&format!("\n{num:>2}  {title:<width$}  {shown}"));
    }
    out
}

pub fn run(repo: &Path, ws: &Path, lesson: Option<u32>, list: bool, reset: bool, output: OutputFormat, color: bool) -> Result<Report> {
    if reset && ws.exists() {
        if !ws.join(MARKER).is_file() {
            return Err(goat_errors::coded("GB-040", format!("{} is not a tutor workspace (no {MARKER}); refusing to delete it", ws.display())).into());
        }
        std::fs::remove_dir_all(ws)?;
    }
    if !ws.join(MARKER).is_file() {
        if ws.exists() && std::fs::read_dir(ws).map(|mut r| r.next().is_some()).unwrap_or(false) {
            return Err(goat_errors::coded("GB-040", format!("{} exists and is not a tutor workspace; pass --workspace <an empty or new directory>", ws.display())).into());
        }
        setup(repo, ws)?;
    }
    let mut progress = load(ws);
    let runs = read_runs(ws);
    let table = matches!(output, OutputFormat::Table);

    // Open lessons are judged against the runs logged since they were opened;
    // a lesson whose runs satisfy it is marked done here.
    let next_open = |p: &Progress| (1..=LESSONS.len()).find(|n| !p.done.contains(n) && available(&LESSONS[n - 1], repo).is_ok());

    if list {
        let next = next_open(&progress);
        let mut checks = Vec::new();
        for (i, l) in LESSONS.iter().enumerate() {
            let n = i + 1;
            let state = match available(l, repo) {
                Err(why) => format!("needs a newer build: {why}"),
                Ok(()) if progress.done.contains(&n) => "done".into(),
                Ok(()) if Some(n) == next => "next".into(),
                Ok(()) => "locked".into(),
            };
            checks.push(row(format!("lesson {n}"), true, format!("{:<62} {state}", title(n, l))));
        }
        let mut report = Report::new("tutor", checks, EXIT_OK);
        if table {
            let path = ws.display().to_string();
            println!("{}\n", crate::i18n::prose_args("tutor-workspace", format!("Workspace: {path}"), &[("path", path.clone())]));
            println!("{}", list_page(&report, color));
            println!("\n{}", ink(&fill(&crate::i18n::prose("tutor-other-lessons", "To read a lesson again: `goatbuilder tutor --lesson 1`. To see them all: `goatbuilder tutor --list`."), "", ""), color).trim_end());
            report.printed = true; // the rows stay for -o json
        }
        return Ok(report);
    }

    let n = match lesson {
        Some(n) => n as usize,
        None => match next_open(&progress) {
            Some(n) => n,
            None => {
                if table {
                    println!(
                        "\n{}\n",
                        crate::i18n::prose("tutor-all-done-reset", "Every lesson is done. `goatbuilder tutor --reset` starts over; the tutorials go further: docs/tutorials/.")
                    );
                }
                return Ok(Report::new("tutor", vec![row("tutor".into(), true, format!("all {} lessons done", progress.done.len()))], EXIT_OK));
            }
        },
    };
    let Some(l) = LESSONS.get(n.wrapping_sub(1)) else {
        return Err(goat_errors::coded("GB-041", format!("there is no lesson {n}; there are {} (goatbuilder tutor --list)", LESSONS.len())).into());
    };
    if let Err(why) = available(l, repo) {
        if table {
            println!("{}", render(n, l, ws, 0, Some(&format!("this lesson {why}; `goatbuilder tutor` moves on without it.")), color, progress.done.len()));
        }
        return Ok(Report::new("tutor", vec![row(format!("lesson {n}"), false, why)], EXIT_REFUSED));
    }
    if let Some(blocking) = (1..n).find(|k| !progress.done.contains(k) && available(&LESSONS[k - 1], repo).is_ok()) {
        return Ok(Report::new(
            "tutor",
            vec![Check { name: format!("lesson {n}"), ok: false, detail: format!("locked: lesson {blocking} is not done yet"), fix: Some(format!("goatbuilder tutor {blocking}")) }],
            EXIT_REFUSED,
        ));
    }
    // A finished lesson named on the command line opens to read. It is not
    // judged, and where the learner is stays where it is.
    if lesson.is_some() && progress.done.contains(&n) {
        let mut checks = Vec::new();
        if table {
            println!("{}", render(n, l, ws, l.expects.len(), None, color, progress.done.len()));
        } else {
            checks = l.expects.iter().enumerate().map(|(i, e)| row(format!("lesson {n}/{}", i + 1), true, e.label.to_string())).collect();
            checks.push(row(format!("lesson {n}"), true, "done".into()));
        }
        return Ok(Report::new("tutor", checks, EXIT_OK));
    }
    let since = open(n, &mut progress, runs.len(), repo, ws)?;
    let (met, why) = evaluate(l, &runs[since.min(runs.len())..], ws);
    let mut checks: Vec<Check> = l
        .expects
        .iter()
        .enumerate()
        .map(|(i, e)| row(format!("lesson {n}/{}", i + 1), i < met, e.label.to_string()))
        .collect();
    if met == l.expects.len() {
        if !progress.done.contains(&n) {
            progress.done.push(n);
            progress.done.sort();
            save(ws, &progress)?;
        }
        let next = next_open(&progress);
        if let (false, Some(m)) = (table, next) {
            open(m, &mut progress, runs.len(), repo, ws)?;
        }
        if table {
            println!(
                "\n{}",
                crate::i18n::prose_args("tutor-lesson-done", format!("Lesson {n} done: {}.", l.title), &[("n", n.to_string()), ("title", title(n, l))])
            );
            match next {
                Some(m) => {
                    open(m, &mut progress, runs.len(), repo, ws)?;
                    println!("{}", render(m, &LESSONS[m - 1], ws, 0, None, color, progress.done.len()));
                }
                None => println!("{}", crate::i18n::prose("tutor-all-done", "Every lesson is done. The tutorials go further: docs/tutorials/.")),
            }
        }
        checks.push(row(format!("lesson {n}"), true, "done".into()));
        if table {
            checks.clear(); // "Lesson N done" and the next page above say it
        }
        return Ok(Report::new("tutor", checks, EXIT_OK));
    }
    if table {
        println!("{}", render(n, l, ws, met, why.as_deref(), color, progress.done.len()));
        // The page above says where the lesson stands, so a table under it
        // would only repeat it: no rows prints no table. A program reading
        // -o json still gets one row per step (below).
        checks.clear();
    } else {
        // For a program reading -o json: the commands to type, as typed.
        let ws_s = ws.display().to_string();
        let typed = l.setup.iter().copied().chain(l.expects.iter().map(|e| e.command));
        for (i, c) in typed.enumerate() {
            checks.push(row(format!("lesson {n}/type {}", i + 1), true, with_names(c, &ws_s)));
        }
        if let Some(w) = &why {
            checks.push(row(format!("lesson {n}/not yet"), true, w.clone()));
        }
    }
    // Showing a lesson in progress is not a failure: exit 0, the rows say what is left.
    let mut r = Report::new("tutor", checks, EXIT_OK);
    r.exit = EXIT_OK;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> PathBuf {
        PathBuf::from("/tmp/goatbuilder-tutor-unit")
    }

    /// Steps count in order. A later step run early does not tick, and the
    /// reason says so (the walkthrough ran lesson 11's step 4 before step 3).
    #[test]
    fn a_step_run_early_says_to_do_the_earlier_one_first() {
        let l = &LESSONS[7];
        assert_eq!(l.expects.len(), 4, "lesson 8 has four steps");
        let r: Vec<Run> = l.expects.iter().map(happy).collect();
        let (met, why) = evaluate(l, &[r[0].clone(), r[1].clone(), r[3].clone()], &ws());
        assert_eq!(met, 2);
        let why = why.expect("a reason");
        assert!(why.contains("you ran step 4 before step 3") && why.contains("do step 3 first"), "{why}");
        let (met, _) = evaluate(l, &[r[0].clone(), r[1].clone(), r[3].clone(), r[2].clone(), r[3].clone()], &ws());
        assert_eq!(met, 4, "step 4 run again after step 3 counts");
    }

    /// A run that ticks a step says so under its own output: which step, or
    /// that the lesson is complete and which command moves on. A run that
    /// ticks nothing says nothing. (The walkthrough finished lesson 10 with no
    /// sign that it was done or what came next.)
    #[test]
    fn a_run_that_ticks_a_step_says_so() {
        // Its own folder: other tests compare paths under ws() and must never
        // find it existing (on macOS that changes how /tmp resolves).
        let w = crate::testdir::path("goatbuilder-tutor-stepdone-");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(w.join(MARKER), "").unwrap();
        std::fs::create_dir_all(w.join("fixture")).unwrap(); // the copy lesson 3 runs on
        let mut p = Progress::default();
        p.opened.insert(3, 0);
        save(&w, &p).unwrap();
        let l = &LESSONS[2];
        // log_run records resolved paths (on macOS /tmp is /private/tmp).
        let (from, to) = (ws().display().to_string(), w.display().to_string());
        let real = |mut r: Run| {
            r.mock = r.mock.map(|m| canon(Path::new(&m.replacen(&from, &to, 1))).display().to_string());
            r.repo = r.repo.map(|m| canon(Path::new(&m.replacen(&from, &to, 1))).display().to_string());
            r
        };
        let log = |runs: &[Run]| std::fs::write(w.join("log.jsonl"), runs.iter().map(|r| serde_json::to_string(r).unwrap() + "\n").collect::<String>()).unwrap();
        let first = real(happy(&l.expects[0]));
        log(std::slice::from_ref(&first));
        let said = step_done(&w, false).expect("the first step ticked");
        assert!(said.contains(&format!("{DONE} Tutor: Step 1 of {} done", l.expects.len())), "{said}");
        // The next step's own instruction and command, ready to run.
        assert!(said.contains(&format!("{}:", l.expects[1].label)), "{said}");
        let next = with_names(l.expects[1].command, &w.display().to_string());
        let flat = said.replace(" \\\n", " ").split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(&next.split_whitespace().collect::<Vec<_>>().join(" ")), "{said}");
        let mut all: Vec<Run> = l.expects.iter().map(|e| real(happy(e))).collect();
        log(&all);
        let said = step_done(&w, false).expect("the last step ticked");
        assert!(said.contains("Lesson 3 is complete") && said.contains("lesson 4"), "{said}");
        assert!(said.contains("goatbuilder tutor --lesson 3"), "{said}");
        assert!(said.lines().all(|l| l.chars().count() <= WIDTH), "{said}");
        assert!(!said.contains('`'), "backticks never print: {said}");
        let again = all.last().unwrap().clone();
        all.push(again);
        log(&all);
        assert_eq!(step_done(&w, false), None, "a repeat that ticks nothing says nothing");
        // The log is the learner's own: nobody else on the machine reads the
        // commands they typed or what every check said.
        append_run(&w, "{\"args\":[\"first\"]}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&w.join("log.jsonl")), 0o600, "the log is the learner's own");
            assert_eq!(mode(&w), 0o700, "so is the workspace");
        }
        // It is a teaching aid, not an audit trail, so it stops growing.
        let fat = "{\"args\":[\"".to_string() + &"x".repeat(4096) + "\"]}";
        for _ in 0..300 {
            append_run(&w, &fat);
        }
        // 300 runs of four kilobytes each is over a megabyte written; what is
        // kept is bounded, and what is kept is the newest.
        let text = std::fs::read_to_string(w.join("log.jsonl")).unwrap();
        assert!(text.len() < 1 << 21, "capped, not endless: {} bytes", text.len());
        assert!(text.lines().count() < 300, "{} lines for 300 runs", text.lines().count());
        assert!(text.lines().last().unwrap().contains("xxx"), "the newest run is the one kept");
        // A learner whose command failed is told which lesson is open.
        let lost = where_you_are(&w, false).expect("a lesson is open");
        assert!(lost.contains("not part of lesson 1") && lost.contains("goatbuilder tutor"), "{lost}");
        let _ = std::fs::remove_dir_all(&w);
    }

    #[test]
    fn a_translated_title_splits_where_characters_end() {
        // 62 characters of title, with a multi-byte character straddling byte 62.
        let title = "\u{e9}".repeat(40) + "abcdefghijklmnopqrstuv";
        assert_eq!(title.chars().count(), 62);
        let r = Report::new("tutor", vec![row("lesson 1".into(), true, format!("{title} done"))], crate::EXIT_OK);
        let page = list_page(&r, false);
        assert!(page.contains("done"), "{page}");
        assert!(page.contains(&title), "the whole title: {page}");
    }

    /// Every "Run these" line runs as printed in bash and in zsh: no `<…>`
    /// placeholder (zsh and bash read `<x>` as a redirect), no comment lines,
    /// and each one parses (`-n` checks syntax and runs nothing). zsh is
    /// checked where it is installed; bash always.
    #[test]
    fn every_command_runs_as_printed_in_bash_and_zsh() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let shells: Vec<&str> = ["bash", "zsh"]
            .into_iter()
            .filter(|sh| std::process::Command::new(sh).arg("-c").arg("true").output().map(|o| o.status.success()).unwrap_or(false))
            .collect();
        assert!(shells.contains(&"bash"), "bash is needed to check the lessons");
        for (i, l) in LESSONS.iter().enumerate() {
            for c in l.setup.iter().copied().chain(l.expects.iter().map(|e| e.command)) {
                let c = with_names(c, "/tmp/goatbuilder-tutor-unit");
                assert!(!c.contains('<') && !c.contains('>'), "lesson {}: a placeholder or redirect in a command: {c}", i + 1);
                assert!(!c.trim_start().starts_with('#'), "lesson {}: a comment is not a command: {c}", i + 1);
                // As written, and as printed (wrapped with " \\" at the page width).
                let wrapped = shell_wrap(&c, "       ");
                for form in [c.as_str(), wrapped.as_str()] {
                    for sh in &shells {
                        let out = std::process::Command::new(sh).args(["-n", "-c", form]).output().expect("run the shell");
                        assert!(out.status.success(), "lesson {}: {sh} cannot parse `{form}`: {}", i + 1, String::from_utf8_lossy(&out.stderr));
                    }
                }
                assert_eq!(shell_words(wrapped.replace(" \\\n", " ").trim()).join(" "), shell_words(&c).join(" "), "lesson {}: wrapping changed the command", i + 1);
            }
        }
    }

    /// The page wraps to WIDTH: no printed line is wider, except a command,
    /// which stays whole on one line so it can be copied.
    #[test]
    fn paragraphs_and_lists_keep_their_breaks() {
        assert_eq!(layout("One.\n\nTwo.\n\n- a\n- b\n\nThree."), "One.\n\nTwo.\n\n  - a\n  - b\n\nThree.\n");
    }

    /// Nobody has to export anything for the default workspace, and a workspace
    /// somewhere else says plainly that runs are not being recorded yet.
    #[test]
    fn the_default_workspace_needs_no_export_and_a_custom_one_says_so() {
        assert!(LESSONS[0].setup.is_empty(), "lesson 1 asks for no setup command");
        let elsewhere = render(1, &LESSONS[0], Path::new("/no/such/workspace"), 0, None, false, 0);
        assert!(elsewhere.contains("not recording"), "{elsewhere}");
        let own = render(1, &LESSONS[0], &home_workspace(), 0, None, false, 0);
        assert!(!own.contains("not recording"), "{own}");
    }

    /// The first page welcomes the learner and carries the only goat; every
    /// page shows progress, and a pipe or GOAT_NO_FUN prints no goat at all.
    #[test]
    fn the_first_page_welcomes_and_carries_the_only_goat() {
        let first = status_for(1, 0, false, true);
        assert!(first.starts_with("\u{1f410}  Welcome to GOATBUILDER Tutor\n\n"), "{first:?}");
        assert_eq!(first.matches('\u{1f410}').count(), 1, "{first:?}");
        let later = status_for(2, 1, false, true);
        assert!(!later.contains("Welcome") && later.contains("1 of 18 lessons done"), "{later:?}");
        assert!(!later.contains('\u{1f410}'), "the progress line carries no goat: {later:?}");
        let off = status_for(1, 0, false, false);
        assert!(!off.contains('\u{1f410}') && off.contains("Welcome to GOATBUILDER Tutor"), "{off:?}");
    }

    #[test]
    fn lesson_pages_wrap_to_the_page_width() {
        for (i, l) in LESSONS.iter().enumerate() {
            let page = render(i + 1, l, &ws(), 0, Some("a reason long enough that it has to wrap onto a second line of the page to fit inside the width"), false, 0);
            let w = ws().display().to_string();
            let cmds: Vec<String> = l.setup.iter().copied().chain(l.expects.iter().map(|e| e.command)).map(|c| with_names(c, &w)).collect();
            for line in page.lines() {
                // A single word longer than the page (a long path) cannot wrap.
                if !line.trim().contains(' ') || cmds.iter().any(|c| line.trim_start() == c && !c.contains(' ')) {
                    continue;
                }
                assert!(line.chars().count() <= WIDTH, "lesson {}: a line wider than {WIDTH}: {line:?}", i + 1);
                assert!(!line.contains('`'), "lesson {}: a backtick printed: {line:?}", i + 1);
            }
        }
    }

    /// A lesson page lists its steps as a checklist: untried steps carry the
    /// x mark, met steps the check mark, and nothing on the page says FAIL before you have run
    /// anything (the walkthrough found lesson 1 opening with two FAIL rows).
    #[test]
    fn lesson_page_shows_steps_as_a_checklist_not_fail() {
        for (i, l) in LESSONS.iter().enumerate() {
            let fresh = render(i + 1, l, &ws(), 0, None, false, 0);
            // The lesson prose may explain what FAIL means; the step list must not use it.
            let steps = &fresh[fresh.find("Do these steps").expect("the steps heading")..];
            assert!(!steps.contains("FAIL"), "lesson {}: an untried step must not say FAIL:\n{steps}", i + 1);
            for (k, e) in l.expects.iter().enumerate() {
                assert!(fresh.contains(&format!("  {TODO} {}. {}", k + 1, e.label)), "lesson {}: step `{}` missing as not done", i + 1, e.label);
                let joined = fresh.split(" \\\n").map(str::trim_start).collect::<Vec<_>>().join(" ");
                assert!(joined.contains(&with_names(e.command, &ws().display().to_string())), "lesson {}: step `{}` must show its command", i + 1, e.label);
            }
            if l.expects.len() > 1 {
                let half = render(i + 1, l, &ws(), 1, None, false, 0);
                assert!(half.contains(&format!("  {DONE} 1. {}", l.expects[0].label)), "lesson {}: a met step shows the check mark", i + 1);
                assert!(half.contains(&format!("  {TODO} 2. {}", l.expects[1].label)), "lesson {}: the next step stays not done", i + 1);
            }
        }
    }

    /// The run that satisfies an expectation, built from the expectation itself.
    fn happy(e: &Expect) -> Run {
        let w = ws();
        let mut args: Vec<String> = e.words.iter().map(|s| s.to_string()).collect();
        let (mock, repo) = match e.on {
            On::Fixture(n) => (Some(w.join(n).display().to_string()), None),
            On::Skeleton => (None, Some(w.join("repo").display().to_string())),
            On::Repo => (None, None),
        };
        if e.words == ["pins"] {
            args.retain(|a| a != "--check");
        }
        Run { args, mock, repo, exit: e.exits[0], rows: e.contains.iter().map(|c| format!("row {c}")).collect() }
    }

    /// Every lesson's happy path: one satisfying run per expectation, in order,
    /// completes it.
    #[test]
    fn every_lesson_completes_on_its_happy_path() {
        for (i, l) in LESSONS.iter().enumerate() {
            let runs: Vec<Run> = l.expects.iter().map(happy).collect();
            let (met, why) = evaluate(l, &runs, &ws());
            assert_eq!(met, l.expects.len(), "lesson {} ({}): {:?}", i + 1, l.title, why);
        }
    }

    /// Every lesson's wrong move: the first step run with an exit the lesson
    /// does not accept leaves it undone and says why, naming the exit.
    #[test]
    fn every_lesson_refuses_a_wrong_exit_and_says_why() {
        for (i, l) in LESSONS.iter().enumerate() {
            let mut bad = happy(&l.expects[0]);
            bad.exit = 42;
            let (met, why) = evaluate(l, &[bad], &ws());
            assert_eq!(met, 0, "lesson {}", i + 1);
            let why = why.unwrap_or_default();
            assert!(why.contains("exited 42"), "lesson {}: {why}", i + 1);
        }
    }

    /// A step run in the wrong place (the real checkout instead of the tutor's
    /// copy, or the reverse) does not count.
    #[test]
    fn a_run_in_the_wrong_place_does_not_count() {
        let l = &LESSONS[1]; // status on the fixture
        let mut r = happy(&l.expects[0]);
        r.mock = None;
        let (met, why) = evaluate(l, &[r], &ws());
        assert_eq!(met, 0);
        assert!(why.unwrap().contains("did not run on the tutor's copy"));
    }

    /// Order matters: deploying before planning leaves the plan step undone.
    #[test]
    fn steps_count_only_in_order() {
        let l = &LESSONS[2];
        let runs = vec![happy(&l.expects[1]), happy(&l.expects[0])];
        let (met, _) = evaluate(l, &runs, &ws());
        assert_eq!(met, 1, "the deploy before the plan does not count; the plan does");
    }

    /// The lessons that need a verb this build lacks are reported, not run.
    #[test]
    fn a_lesson_needing_a_missing_verb_is_unavailable() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for l in LESSONS {
            if let Some(v) = l.needs_verb {
                let has = Cli::command().find_subcommand(v).is_some();
                assert_eq!(available(l, &repo).is_ok(), has, "{}", l.title);
            }
        }
    }
}

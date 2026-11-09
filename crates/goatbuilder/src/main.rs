//! goatbuilder — the build/release command-line for a goat environment.
//!
//! Design of record: docs/design/goatbuilder.md (ADR 1198). The model is
//! declare → converge → verify; this binary only ever declares and verifies,
//! and it enforces the deploy contract C1–C10 in one place. The config it reads
//! is the versioned schema in ops/goatbuilder/schema/, embedded at build time so
//! the contract travels with the tool.
//!
//! This is the first buildable increment: `validate` and `doctor`, the output
//! formats and typed exit codes every later verb shares (contract C10). Each
//! later verb lands in this binary and deletes the script it replaces, behind
//! the parity gate (docs/design/goatbuilder-conformance.md).

mod assay;
mod awake;
mod complete;
mod config;
mod converger;
mod docs;
mod doctor;
mod endstate;
mod fastlane;
mod federate;
mod fresh;
mod fun;
mod i18n;
mod inspect;
mod joinnode;
mod output;
mod pins;
mod preflight;
mod promote;
mod push;
mod release;
mod rollback;
mod secrets;
mod settings;
mod standup;
mod survey;
#[cfg(test)]
mod testdir;
mod trace;
mod tutor;
mod update;
mod verbs;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use serde::Serialize;

/// Exit codes are the machine contract (C10): a pipeline branches on them.
pub const EXIT_OK: i32 = 0;
/// Refused with a named reason and, where one exists, a recovery command.
pub const EXIT_REFUSED: i32 = 1;
/// Usage, a missing dependency, or an unreadable input.
pub const EXIT_USAGE: i32 = 2;

/// `--ack` names the person on the record (C4). A value that is still a
/// placeholder (`<your name>`, `<name>`) would put that placeholder on the
/// record, so it is refused before anything runs.
fn parse_ack(s: &str) -> Result<String, String> {
    if s.contains('<') && s.contains('>') {
        return Err(format!("`{s}` is still a placeholder: put your own name, for example --ack \"$(git config user.name)\""));
    }
    Ok(s.to_string())
}
/// The verb ran, but some machine is still pending or unknown (C7).
pub const EXIT_NOT_CONVERGED: i32 = 3;

#[derive(Parser, Debug)]
#[command(
    name = "goatbuilder",
    version,
    about = "Build and ship a goat environment: declare, converge, verify — under one deploy contract",
    long_about = None
)]
pub struct Cli {
    /// Output format: a table for a person; JSON or YAML for a program.
    #[arg(short = 'o', long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    pub output: OutputFormat,

    /// The repository root. Defaults to the checkout this binary is run in.
    #[arg(short = 'C', long, global = true, value_name = "DIR")]
    pub repo: Option<PathBuf>,

    /// Confirm a declarative verb without prompting (for automation).
    #[arg(long, global = true)]
    pub yes: bool,

    /// Refuse any run that would need to fast-forward the checkout (strict C3).
    #[arg(long, global = true)]
    pub frozen: bool,

    /// An islanded site: do not fetch the trunk; judge the checkout against the
    /// cached one, and record the run as offline (C3; ops/scripts/fresh-source.sh).
    #[arg(long, global = true)]
    pub offline: bool,

    /// Run against a fixture directory instead of hosts: no registry, no
    /// transport, deterministic (goatbuilder-conformance.md §"The fixture").
    #[arg(long, global = true, value_name = "DIR")]
    pub mock: Option<PathBuf>,

    /// Language for help and error explanations (a tag such as fr, uk, de);
    /// output a pipeline reads stays English. Default: the OS locale.
    #[arg(long, global = true, env = "GOAT_LANG", value_name = "TAG")]
    pub lang: Option<String>,

    /// Color the table's ok/FAIL glyph and verdict (auto|always|never). `auto`
    /// colors only a terminal; NO_COLOR and CLICOLOR_FORCE override. JSON and
    /// YAML are never colored.
    #[arg(long, global = true, value_enum, default_value_t = output::ColorChoice::Auto)]
    pub color: output::ColorChoice,

    /// Log what a run does — the external commands it shells out to, and the
    /// network phase of `update` — to stderr, so `-o json|yaml` stays a clean
    /// document. Or set RUST_LOG=goatbuilder=debug.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Seconds to bound the connect/reach phase before failing fast — the one
    /// phase that can block on an unreachable host (`update`'s fetch of the
    /// served version). Does NOT cap a play's own run or the binary download
    /// (stall-bounded). Also reads GOATBUILDER_TIMEOUT.
    #[arg(long, global = true, env = "GOATBUILDER_TIMEOUT", value_name = "SECS")]
    pub timeout: Option<u64>,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Yaml,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Check a config against the versioned schema and the cross-field rules, offline.
    Validate {
        /// The config file. Defaults to ops/goatbuilder/goatbuilder.yaml under the repo,
        /// or goatlite.yaml at its root when the checkout has no config of its own.
        config: Option<PathBuf>,
    },
    /// Probe what a run needs before anything is cut: deps, checkout freshness,
    /// each environment's transport, and that every component's playbook
    /// resolves to a definite target mode.
    Doctor {
        /// Limit the environment checks to one environment.
        #[arg(long)]
        env: Option<String>,
        /// Also scan every input a standup of this site will reach (role asserts,
        /// `| mandatory`, the secrets its plays template) and list each one that is
        /// missing, all at once, before any host is touched.
        #[arg(long)]
        site: Option<String>,
        /// Print every check, not only the problems. `-o json` always has them all.
        #[arg(long)]
        all: bool,
        /// Check the stored credentials as well: the AWS profile, and whether a
        /// 1Password session is open. Either can prompt you (a Keychain or
        /// biometric prompt, or a wait on a passphrase), so a plain run reads
        /// only what is declared on this machine and prompts for nothing.
        #[arg(long)]
        credentials: bool,
        /// Require the browser tests' toolchain (the console's Playwright suite
        /// the release's live rows run). Without this flag a missing toolchain
        /// is a note, not a failure.
        #[arg(long)]
        browser_tests: bool,
    },
    /// Declared against recorded, per machine: converged, pending or unknown
    /// (C7, C8). Reads only. Exit 3 while any machine is not converged.
    Status {
        /// The environment to read.
        #[arg(long)]
        env: String,
        /// One component; default every component declared for the environment.
        #[arg(long)]
        component: Option<String>,
    },
    /// Put the pin main declares onto one environment and prove it: resolve
    /// the component and the environment's groups from the pins, the play and
    /// its target mode, every secret the play templates, the transport; run the
    /// play through ansible-preflight; read the deploy ledger back from every
    /// host and call it done only when the newest row names the declared
    /// digest with action=deploy (C7). On a fixture (`--mock`) the scripted
    /// converger stands in for the hosts.
    Deploy {
        /// The environment (the pins' goatnet key) to converge.
        #[arg(long)]
        env: String,
        /// One component. Required on real hosts; on a fixture, default every component declared for the environment.
        #[arg(long)]
        component: Option<String>,
        /// Who is declaring (C4: every declaration and record carries the actor).
        #[arg(long, value_name = "NAME", default_value = "", value_parser = parse_ack)]
        ack: String,
        /// One member group of the environment instead of every member.
        #[arg(long)]
        group: Option<String>,
        /// Decide everything and print it; touch no host.
        #[arg(long)]
        dry_run: bool,
        /// Skip the play; only read the ledgers back.
        #[arg(long)]
        verify_only: bool,
    },
    /// Before a rollout, the readiness and reversibility readout (ADR 1211):
    /// three facts — State (stateless | stateful), Rollback (pin-revert |
    /// restore-required | none) and Scope (single | coupled) — declared per
    /// component and FAIL-CLOSED when undeclared, with the readiness half the
    /// other verbs prove (ref pullable, pins --check, the secrets gate, fresh
    /// source, the declared end state) and a validation of the rollback fact
    /// (rollback --dry-run for pin-revert, the retention window for
    /// restore-required). `deploy` runs it first; honesty-graded, never a false
    /// green. Reads only; exit 3 when not clean.
    Preflight {
        /// The component (a fleet-pins key).
        component: String,
        /// The environment (the pins' goatnet key).
        #[arg(long)]
        env: String,
        /// On real hosts: the `site:` whose ledger the rollback validation reads
        /// (as `ops/ansible/group_vars` names it), so one goatnet's reversibility is checked and no other.
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
    /// Take a release candidate from a tag on main through the dev goatnet's
    /// live validation to the preprod rung (ADR 1173); after the soak, with
    /// `--from prod`, graduate it, fold it into fleet:, deploy prod and
    /// re-verify every rung. Replaces ops/scripts/rc.sh step for step.
    Release {
        /// The component (a fleet-pins key).
        component: String,
        /// X.Y.Z — the version to cut (or pass --tag to validate an existing tag).
        #[arg(long)]
        version: Option<String>,
        /// The rc number; default the next free one on origin.
        #[arg(long)]
        rc: Option<u32>,
        /// Who nominates (required: a candidate reaches preprod and is attributed).
        #[arg(long, value_name = "NAME", default_value = "", value_parser = parse_ack)]
        ack: String,
        /// Resume at a step: tag | dev | rows | preprod | preprod-rows | prod
        /// (preprod-rows: preprod already runs the tag; redo only its deploy
        /// check, live rows and evidence, without reopening the record or re-pinning).
        #[arg(long, default_value = "tag")]
        from: String,
        /// An existing tag to validate as-is instead of cutting one.
        #[arg(long)]
        tag: Option<String>,
        /// Print every command; mutate nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Roll a deployed component back to its N-th most recent ref, by artifact
    /// type, reading the on-host ledger as the source of truth (ADR 1060).
    /// Replaces ops/scripts/rollback.sh: exit 3 refused unsafe (data, cert,
    /// forward-only, reversible:false), 4 guided (config, acl).
    Rollback {
        /// The component (an Ansible role with a deploy-history.yml).
        component: String,
        /// Steps back (1..7).
        #[arg(default_value_t = 1)]
        n: u32,
        /// Show the deploy history, newest first (host ledger if reachable, else in-repo).
        #[arg(long)]
        list: bool,
        /// Show the running ref and the artifact type per host.
        #[arg(long)]
        status: bool,
        /// Print the remote shell each host would run; touch nothing.
        #[arg(long)]
        dry_run: bool,
        /// The environment, on a fixture (`--mock`): the rollback declares the
        /// earlier ref there and converges it (C9). Real hosts read it from the ledger.
        #[arg(long)]
        env: Option<String>,
        /// On real hosts: only the `hosts:` entries of deploy-history.yml with this
        /// `site:`, so one goatnet is rolled back and no other.
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
        /// Who is declaring, on a fixture (C4: the declaration and every record carry the actor).
        #[arg(long, value_name = "NAME", default_value = "", value_parser = parse_ack)]
        ack: String,
    },
    /// One command from a merged commit to every goatnet (ADR 1127): the
    /// deliberate exception to the ladder. Replaces ops/scripts/fastlane.sh
    /// step for step; refuses an unpinned digest or an open graduation set,
    /// and runs the deploys through this binary.
    Fastlane {
        /// The component (a fleet-pins key).
        component: String,
        /// The sha-tag or full ref CI built from a merged commit.
        #[arg(long, value_name = "REF", default_value = "")]
        r#ref: String,
        /// Why this cannot wait for the ladder (recorded).
        #[arg(long, default_value = "")]
        reason: String,
        /// Who ships it (required: a fastlane reaches production).
        #[arg(long, value_name = "NAME", default_value = "", value_parser = parse_ack)]
        ack: String,
        /// Resolve and report; stop before any mutation.
        #[arg(long)]
        dry_run: bool,
        /// Land the pin only; deploy per goatnet afterwards.
        #[arg(long)]
        no_deploy: bool,
    },
    /// A site from nothing to verified: the standup's phases in the doctrine's
    /// order with a fail-closed gate at each end, the observability host as a
    /// parallel leg, the rescue classes and the cross-goatnet surfaces last
    /// (ADR 1183). Replaces ops/scripts/goat-standup.sh step for step. On a
    /// fixture (`--mock`) every component of every class is converged by the
    /// scripted converger and the declared → verified time is the one-hour
    /// goatnet's measure.
    Standup {
        /// The environment (goatnet) the site belongs to.
        env: String,
        /// The site id (the `site_id` in the site's group_vars).
        #[arg(long)]
        site: Option<String>,
        /// Topology profile, lan|internet; default read from the site's config.
        #[arg(long)]
        profile: Option<String>,
        /// Who is declaring (C4).
        #[arg(long, value_name = "NAME", default_value = "", value_parser = parse_ack)]
        ack: String,
        /// Resume from this phase, 1..14 (every phase is idempotent).
        #[arg(long, default_value_t = 1, value_parser = standup::phase_arg)]
        from: u32,
        /// Stop after this phase, 1..14.
        #[arg(long, default_value_t = 14, value_parser = standup::phase_arg)]
        to: u32,
        /// Build no observability host even when the site declares one.
        #[arg(long)]
        skip_obs: bool,
        /// Run only the observability legs, in the foreground.
        #[arg(long)]
        obs_only: bool,
        /// Print every phase and command; run nothing.
        #[arg(long)]
        dry_run: bool,
        /// Run phase 12's independent steps together, wave by wave, as
        /// ops/goatbuilder/standup-dag.yaml declares (the script's --parallel).
        #[arg(long)]
        parallel: bool,
        /// Run the acceptance phase (13) on a goatlite, where it is off by
        /// default; `goatbuilder assay <site>` runs it on its own. Every other
        /// goatnet runs it always.
        #[arg(long)]
        check: bool,
    },
    /// A site from verified to nothing — standup's mirror, records kept (a
    /// burn-down appends teardown rows, never erases). On a fixture (`--mock`)
    /// it rehearses the burn-down in reverse phase order; on real hosts it
    /// prints the order and refuses to execute until each phase declares its
    /// teardown play (docs/design/goatbuilder-speed.md).
    Teardown {
        /// The environment (goatnet) the site belongs to.
        env: String,
        /// The site id.
        #[arg(long)]
        site: String,
        /// Topology profile, lan|internet; default read from the site's config.
        #[arg(long)]
        profile: Option<String>,
        /// Print the burn-down order; run nothing.
        #[arg(long)]
        dry_run: bool,
        /// Stop the services only (compose-down steps, volumes kept) and leave
        /// every step that holds data, or is not built, in place.
        #[arg(long)]
        services: bool,
        /// First standup phase to take down, 1..14.
        #[arg(long, default_value_t = 1, value_parser = standup::phase_arg)]
        from: u32,
        /// Last standup phase to take down, 1..14.
        #[arg(long, default_value_t = 14, value_parser = standup::phase_arg)]
        to: u32,
    },
    /// One more machine of a class at a live site, declared → converged →
    /// verified: the node-addition sequence of
    /// docs/tutorials/02-first-federated-node.md as numbered phases with its
    /// gates (reach, key, repo work first, baseline, kernel WireGuard before and
    /// after enrolment, the kernel ACL plane as the enrolment gate, the class's
    /// converge, acceptance).
    JoinNode {
        /// The environment (goatnet).
        env: String,
        /// The site the node joins.
        #[arg(long)]
        site: String,
        /// The node's inventory name.
        #[arg(long)]
        node: String,
        /// The machine class: core | obs | breakglass | network-jump.
        #[arg(long)]
        class: String,
        /// The node's LAN address (required to run; phase 1 says how to find it).
        #[arg(long)]
        address: Option<String>,
        /// The env var holding the node's first-login password (default <SITE>_NODE_PASS).
        #[arg(long)]
        pass_var: Option<String>,
        /// How this machine reaches the goatnet's hosts: `mesh` (this machine is on
        /// its mesh) or `door` (through its env door; the plan opens it first).
        #[arg(long, default_value = "mesh", value_parser = ["mesh", "door"])]
        via: String,
        /// Resume from this phase.
        #[arg(long, default_value_t = 1)]
        from: u32,
        /// Stop after this phase.
        #[arg(long, default_value_t = 99)]
        to: u32,
        /// Print every phase and command; run nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// A sovereign peering declared, converged and verified (ADR 1157): our
    /// half measured, the packages exchanged through the peering directory, one agreement
    /// composed and verified, the leaves minted from it, the rules rendered by
    /// the one writer, the edge deployed, the transport the agreement chose
    /// opened, acceptance in all four tiers. Replaces
    /// ops/scripts/peering-standup.sh step for step.
    ///
    /// `goatbuilder federate init|lint|compose|diff|verify|apply` runs one of
    /// the six federation verbs in process, with the flags and exit codes of
    /// `goat federate <verb>`. `goatbuilder federate plan <partner>` writes
    /// the bounded plan alone.
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Federate {
        /// One federation verb, or one stage of the one command (`plan`),
        /// instead of the whole peering.
        #[command(subcommand)]
        verb: Option<federate::plan::Stage>,
        #[command(flatten)]
        site: Box<federate::verbs::SiteArgs>,
        /// The peering name (the counterparty, as declared), or a stage run
        /// alone: `status [<partner>]`.
        #[arg(required = true)]
        with: Option<String>,
        /// The partner, after a stage word (`status <partner>`).
        partner: Option<String>,
        /// The environment the peering is declared on (checked against goatbuilder.yaml peerings:).
        #[arg(long)]
        env: Option<String>,
        /// Which party we are (default: read from the declaration).
        #[arg(long = "as", value_name = "SOVEREIGN")]
        as_party: Option<String>,
        /// The ansible group the edge runs in (default <peering>_edge).
        #[arg(long)]
        edge_group: Option<String>,
        /// The working directory (default $FED_WORK, else /var/lib/goat-federation/<peering>).
        #[arg(long)]
        work: Option<String>,
        /// Resume from this phase.
        #[arg(long, default_value_t = 1)]
        from: u32,
        /// Stop after this phase.
        #[arg(long, default_value_t = 99)]
        to: u32,
        /// Print every phase and command; run nothing.
        #[arg(long)]
        dry_run: bool,
        /// Skip the phases the run journal (<work>/state.json) shows passed,
        /// whose inputs and local outputs are unchanged now; checks and plays
        /// always run again.
        #[arg(long)]
        resume: bool,
    },
    /// ONE resolution pass with TWO renderings: every listed member's
    /// group_vars pins.yml and every listed goatnet's IntentV1 artifact, from
    /// ops/pins/fleet-pins.yaml. Replaces ops/pins/generate-pins.py byte for
    /// byte. `--check` validates, regenerates and refuses drift, an unsigned or
    /// tampered intent, and a governed pin defined outside pins.yml (exit 1).
    Pins {
        /// Validate + diff instead of writing; exit 1 on drift.
        #[arg(long)]
        check: bool,
    },
    /// Guide and observe release graduation across the goatnet ladder (ADR
    /// 1120, 1125): status, open a candidate, graduate, graduate a set, the
    /// hotfix path, review, readiness, abandon, check. Replaces
    /// ops/release/graduate.py step for step. Nothing here graduates by itself:
    /// a mutation runs only when invoked, prints what it did, and asks for
    /// --yes without a terminal (C10, ADR 1177).
    Promote {
        #[command(subcommand)]
        cmd: promote::Sub,
    },
    /// Interpret an error: a code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), or any
    /// error text — this binary's own, or a tool's words it carried (ansible,
    /// ssh, op, aws, git, gh, cosign, docker, netbird). Prints what happened,
    /// why, what to do and where to read more; `-` reads the text from stdin.
    Explain {
        /// A code, or words of the error. `-` reads stdin. A flag (`-o`,
        /// `--lang`) parses wherever it appears; to read text that itself
        /// starts with a hyphen, put `--` before it.
        #[arg(value_name = "CODE|TEXT", num_args = 0..)]
        input: Vec<String>,
        /// Also write the text and its interpretation to ~/.goat/error-reports/
        /// (a file to share when the catalog does not know the error).
        #[arg(long)]
        report: bool,
        /// List every code the catalog knows.
        #[arg(long)]
        list: bool,
    },
    /// Lessons, vimtutor-style: each names the exact command to run against a
    /// throwaway copy of a fixture (or, read-only, against this checkout), then
    /// checks what that command actually did — its exit and its rows — before
    /// the next unlocks. `tutor` shows the lesson you are on and marks it done
    /// when your runs satisfy it; `tutor <n>`, `--list`, `--reset`.
    Tutor {
        /// The lesson to open (1..); default the first not yet done. A finished
        /// lesson opens to read again and changes nothing.
        lesson: Option<u32>,
        /// The same as the number on its own: `tutor --lesson 2`.
        #[arg(long = "lesson", value_name = "N", conflicts_with = "lesson")]
        lesson_flag: Option<u32>,
        /// Every lesson, with done / next / locked / needs-a-newer-build.
        #[arg(long)]
        list: bool,
        /// Start over: a fresh copy of every fixture, no progress.
        #[arg(long)]
        reset: bool,
        /// Where the fixtures' copies, the progress and the run log live
        /// (default $GOATBUILDER_TUTOR, else ~/.goatbuilder/tutor).
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Hidden (src/fun.rs).
    #[command(hide = true)]
    Make {
        #[arg(num_args = 0.., trailing_var_arg = true, allow_hyphen_values = true)]
        words: Vec<String>,
    },
    /// Hidden (src/fun.rs).
    #[command(hide = true)]
    Argan,
    /// The declared secrets posture (ADR 1204): which store is in force
    /// (1password, openbao, sops, git-crypt, env-file, env), what is available,
    /// how to supply what is missing, and the store's own checks. `init --env
    /// <goatnet>` writes the zero-SaaS quickstart's env file: a strong random
    /// value for every variable the goatnet's plays template, 0600, gitignored,
    /// never rotating a value already there. Same module as ops/scripts/with-secrets.
    Secrets {
        /// store | available | missing NAME… | hint NAME… | doctor | init --env GOATNET [--file PATH] [--force] | exec -- CMD…
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },
    /// Read every host's newest deploy-ledger row for one component and say
    /// whether it names the ref main declares (C7). Reads only; exit 3 while any
    /// host is pending or unknown. The same check `deploy --verify-only` runs.
    Verify {
        /// The component (a fleet-pins key).
        component: String,
        /// The environment (the pins' goatnet key).
        #[arg(long)]
        env: String,
        /// One member group of the environment instead of every member.
        #[arg(long)]
        group: Option<String>,
        /// Print what would be read and the digest required; read nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// What is different from main right now (C8): main's declared ref against
    /// each host's newest ledger row, for one component or, without one, every
    /// component of the environment, read in one pass per member group. Reads
    /// only; exit 3 on any drift or unreadable host.
    Diff {
        /// The component (a fleet-pins key); omit it for every component.
        component: Option<String>,
        /// The environment (the pins' goatnet key).
        #[arg(long)]
        env: String,
        /// Every component the environment runs (the default without a component).
        #[arg(long, conflicts_with = "component")]
        all: bool,
    },
    /// The deploy ledger as a timeline, newest first, on every host of the
    /// environment (C8): one row per record, with the machine, the time, the
    /// action (deploy or rollback) and the version. Each record's prior version
    /// and the operator who ran it are in `-o json`. Reads only.
    History {
        /// The component (a fleet-pins key or a role with a ledger).
        #[arg(required_unless_present = "component_flag")]
        component: Option<String>,
        /// The component, as a flag (the same as the argument).
        #[arg(long = "component", value_name = "COMPONENT", conflicts_with = "component")]
        component_flag: Option<String>,
        /// The environment (the pins' goatnet key).
        #[arg(long)]
        env: String,
    },
    /// Write what a declarative verb would do to a plan file for review: the
    /// declared refs, the play and target, the ledger, main's commit, the
    /// transport (deploy); the artifact type and the target ref (rollback).
    /// Touches no host's state. Run it as reviewed with `apply`.
    Plan {
        /// deploy | rollback
        verb: String,
        /// The component.
        component: String,
        /// The environment (deploy).
        #[arg(long)]
        env: Option<String>,
        /// One member group (deploy).
        #[arg(long)]
        group: Option<String>,
        /// Steps back (rollback).
        #[arg(long, default_value_t = 1)]
        n: u32,
        /// One goatnet to roll back (rollback), its `site:` in the ledger
        /// (its `site_id`). Required for a component that spans more than
        /// one goatnet; carried in the plan so apply stays narrowed to it.
        #[arg(long)]
        site: Option<String>,
        /// Where to write the plan.
        #[arg(long = "out", value_name = "FILE")]
        out: PathBuf,
    },
    /// Run exactly a reviewed plan. Refuses (exit 1, naming what moved) when
    /// main's declaration, the pins commit, the play or the rollback target
    /// moved since the plan was written; a changed transport is reported, not refused.
    Apply {
        /// The plan file `goatbuilder plan` wrote.
        plan: PathBuf,
    },
    /// Declare what an environment already runs: read every host's ledgers
    /// and propose a pins deviation for each component whose running ref
    /// differs from main's declaration. A report by default; with --yes the
    /// edits are written into a fresh worktree of main for review. Never deploys.
    Adopt {
        /// The environment (the pins' goatnet key).
        #[arg(long)]
        env: String,
    },
    /// Check that a goatnet was built correctly, layer by layer: every
    /// acceptance check that applies to the site, including the checks an
    /// operator runs with the goat CLI. The standup's acceptance phase as its
    /// own command. Reads only; exit 3 while any check fails or cannot answer
    Assay {
        /// The goatnet site (its `ops/ansible/group_vars/<site>_hosts` name).
        site: String,
        /// List the checks that apply to this site, by layer; probe nothing.
        #[arg(long)]
        plan: bool,
        /// Run only these checks, by the names `--plan` prints (comma-separated).
        #[arg(long)]
        only: Option<String>,
        /// Override one value for this run (`k=v`), e.g. `goat_via=user@host`
        /// for a machine that holds the goat context.
        #[arg(long = "var", value_name = "K=V")]
        vars: Vec<String>,
        /// Also write each row's verdict to this file as JSON lines.
        #[arg(long = "rows-out", value_name = "FILE")]
        rows_out: Option<PathBuf>,
        /// Use this value set instead of deriving one from the site's
        /// group_vars (a fixture's, for a rehearsal with --plan).
        #[arg(long, value_name = "FILE")]
        values: Option<PathBuf>,
    },
    /// Check that a goatnet is in its declared end state: every service its own
    /// declarations say it runs is deployed, running, healthy and doing its job,
    /// with its zero-trust edges wired, and nothing runs that it does not declare.
    /// Reads only; exit 3 while anything is off
    Endstate {
        /// The goatnet site (its `ops/ansible/group_vars/<site>_hosts` name).
        site: String,
        /// Grade facts read earlier (`endstate.py observe`) instead of reading the hosts.
        #[arg(long, value_name = "FILE")]
        facts: Option<PathBuf>,
        /// The assay's rows (`goatbuilder assay --rows-out`) for the functional grade,
        /// instead of running the assay first.
        #[arg(long = "assay-rows", value_name = "FILE")]
        assay_rows: Option<PathBuf>,
        /// Skip the assay: the functional grade then reads unknown.
        #[arg(long = "no-assay")]
        no_assay: bool,
        /// A grade nothing can observe (no health signal, no acceptance row) fails too.
        #[arg(long)]
        strict: bool,
        /// Read the declarations from this tree instead of the repository (a fixture).
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
    },
    /// How consistent the software is inside each goatnet and across the
    /// ladder's goatnets, read in one parallel pass. Reads only.
    ///
    /// Inside a goatnet: main's declared ref against each host's newest ledger
    /// row and the containers it runs from that image (a running one; a stopped
    /// one only on a warm standby with none running), hosts of one goatnet that
    /// disagree, the goat CLI on each host against the one its goatnet serves,
    /// and what goat-get serves against main. Across goatnets: one row per
    /// component, one column per goatnet, with the ladder's grade for every
    /// difference, as promote status grades it. Exit 3 on any drift or anything unread; a
    /// difference the ladder declares is reported, not failed.
    Survey {
        /// One goatnet (repeatable); default the ladder's rungs (dev, preprod, prod).
        #[arg(long = "env", value_name = "GOATNET")]
        envs: Vec<String>,
        /// Every goatnet fleet-pins.yaml declares, not only the ladder's.
        #[arg(long, conflicts_with = "envs")]
        all: bool,
        /// Only the findings inside each goatnet.
        #[arg(long, conflicts_with = "inter")]
        intra: bool,
        /// Only the findings across goatnets, and the matrix.
        #[arg(long)]
        inter: bool,
        /// Also write a self-contained HTML report to this file (or `survey.html` in this directory).
        #[arg(long, value_name = "FILE")]
        html: Option<PathBuf>,
    },
    /// Replace this binary with the release its goatnet serves at
    /// `get.<site>.<zone>/goatbuilder/`: the download is checked against its
    /// `.sha256` and the signed `SHA256SUMS` before anything is touched, and the
    /// swap is atomic. Already current: one row, exit 0. `--check` reads only
    /// (exit 3 when behind). The surface: `GOATBUILDER_UPDATE_URL`, else `--site`
    /// from its group_vars, else the one the last update used.
    Update {
        /// The goatnet that serves goatbuilder (its group_vars site).
        #[arg(long)]
        site: Option<String>,
        /// Report running against served; change nothing.
        #[arg(long)]
        check: bool,
        /// Reinstall even when already current (re-verifies the checksum).
        #[arg(long, conflicts_with = "check")]
        force: bool,
    },
    /// Print a shell completion script to stdout (`goatbuilder completion zsh >>
    /// ~/.zshrc`). For live candidates a static script cannot know — the sites,
    /// components and goatnets this checkout declares — wire a hook to
    /// `goatbuilder _complete` (see the generated script's header).
    Completion {
        /// The shell to generate for.
        shell: clap_complete::Shell,
    },
    /// The command reference: `goatbuilder docs` points at it; `--markdown`
    /// emits the whole thing, walked from the live command tree so it can never
    /// drift from `--help`.
    Docs {
        /// Emit the full command reference as Markdown (for the committed
        /// reference or offline reading).
        #[arg(long)]
        markdown: bool,
    },
    /// Hidden: emit live completion candidates for shell hooks, one per line
    /// (the sites, components and goatnets this checkout declares). Reads only,
    /// never fails loudly — a completion hook must not corrupt the prompt.
    #[command(name = "_complete", hide = true)]
    Complete {
        #[command(subcommand)]
        op: complete::CompleteCmd,
    },
    /// Hidden (crates/goat-fun).
    #[command(hide = true)]
    Bound,
}

/// One check's result. `fix` is the recovery command when the check refuses.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    /// A check that did not run: it reads "not checked", and `ok` only says it
    /// did not fail. `-o json` marks it `"skipped": true`, so a program need not
    /// parse the words (docs/design/goatbuilder-output.md rule 2).
    pub fn skipped(&self) -> bool {
        self.ok && self.detail.starts_with("not checked")
    }
}

impl Serialize for Check {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let skipped = self.skipped();
        let mut st = s.serialize_struct("Check", 3 + usize::from(self.fix.is_some()) + usize::from(skipped))?;
        st.serialize_field("name", &self.name)?;
        st.serialize_field("ok", &self.ok)?;
        st.serialize_field("detail", &self.detail)?;
        if let Some(fix) = &self.fix {
            st.serialize_field("fix", fix)?;
        }
        if skipped {
            st.serialize_field("skipped", &true)?;
        }
        st.end()
    }
}

/// The one shape every verb renders (C10): versioned, so a consumer can pin to it.
#[derive(Serialize, Debug)]
pub struct Report {
    pub schema: &'static str,
    pub verb: &'static str,
    pub ok: bool,
    pub exit: i32,
    pub checks: Vec<Check>,
    /// The verb already printed its own human view (a federate dry run's plan
    /// table), so the rows are not printed again as a table. They still go to
    /// -o json and to the tutor's log.
    #[serde(skip)]
    pub printed: bool,
}

impl Report {
    pub fn new(verb: &'static str, checks: Vec<Check>, exit_when_refused: i32) -> Self {
        let ok = checks.iter().all(|c| c.ok);
        Report {
            schema: "goatbuilder.output/v1",
            verb,
            ok,
            exit: if ok { EXIT_OK } else { exit_when_refused },
            checks,
            printed: false,
        }
    }

    /// Append rows and recompute the verdict (doctor's `--site` scan rides on the base report).
    pub fn with_checks(mut self, extra: Vec<Check>) -> Self {
        self.checks.extend(extra);
        let ok = self.checks.iter().all(|c| c.ok);
        if !ok && self.ok {
            self.exit = EXIT_REFUSED;
        }
        self.ok = ok;
        self
    }
}

/// One row of `explain --list` under `-o json`/`-o yaml`: the same fields the
/// plain table prints (code, tool, title), plus the exit code a single-code
/// lookup already exposes, so the two output shapes agree on what a catalog
/// entry is.
#[derive(Serialize)]
struct ListRow<'a> {
    code: &'a str,
    tool: &'a str,
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit: Option<i32>,
}

/// Without `-C`, the repository is the git toplevel of the current directory —
/// so a run from a subdirectory reads the right config — falling back to the
/// current directory outside a checkout.
/// `goatbuilder explain`: one code, or any text, interpreted against the
/// embedded catalog (ADR 1199). Exit 0 when recognised, 1 when not.
/// A terminal gets the laid-out page; a pipe gets the catalog's text unchanged.
fn explanation_for_stdout(text: &str) -> String {
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        output::explanation_page(text, output::want_color(output::ColorChoice::Auto))
    } else {
        text.to_string()
    }
}

fn explain(input: &[String], report: bool, list: bool, format: OutputFormat) -> i32 {
    let cat = goat_errors::catalog();
    if list {
        match format {
            OutputFormat::Table if std::io::IsTerminal::is_terminal(&std::io::stdout()) => {
                let (wc, wt) = (cat.entries.iter().map(|e| e.code.len()).max().unwrap_or(4), cat.entries.iter().map(|e| e.tool.len()).max().unwrap_or(4));
                println!("{:<wc$}  {:<wt$}  MEANING", "CODE", "TOOL");
                let hang = " ".repeat(wc + wt + 4);
                for e in &cat.entries {
                    let first = format!("{:<wc$}  {:<wt$}  ", e.code, e.tool);
                    println!("{}", tutor::fill(&e.title, &first, &hang).trim_end());
                }
            }
            OutputFormat::Table => {
                for e in &cat.entries {
                    println!("{:<22} {:<16} {}", e.code, e.tool, e.title);
                }
            }
            OutputFormat::Json | OutputFormat::Yaml => {
                let rows: Vec<ListRow> = cat
                    .entries
                    .iter()
                    .map(|e| ListRow { code: &e.code, tool: &e.tool, title: &e.title, exit: e.exit })
                    .collect();
                match format {
                    OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default()),
                    OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&rows).unwrap_or_default()),
                    _ => unreachable!("matched above"),
                }
            }
        }
        return EXIT_OK;
    }
    let text = if input.is_empty() || input == ["-"] {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s);
        s
    } else {
        input.join(" ")
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        eprintln!("goatbuilder explain: give a code or an error text, or `-` to read stdin");
        return EXIT_USAGE;
    }
    // One token that is a code: the entry, no interpretation ceremony.
    if !text.contains(char::is_whitespace) {
        if let Some(e) = goat_errors::explain(&text) {
            match format {
                OutputFormat::Table => print!("{}", explanation_for_stdout(&goat_errors::render_entry(e, ""))),
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(e).unwrap_or_default()),
                OutputFormat::Yaml => println!("{}", serde_yaml::to_string(e).unwrap_or_default()),
            }
            return EXIT_OK;
        }
    }
    let interp = goat_errors::interpret(&text);
    match format {
        OutputFormat::Table => {
            println!("{}", interp.summary);
            print!("{}", explanation_for_stdout(&goat_errors::render_human(&interp, "goatbuilder")));
        }
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&interp).unwrap_or_default()),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&interp).unwrap_or_default()),
    }
    if report {
        match goat_errors::write_report(&text, &interp, "goatbuilder") {
            Ok(p) => eprintln!("report written: {}", p.display()),
            Err(e) => eprintln!("report not written: {e}"),
        }
    }
    if interp.unrecognised { EXIT_REFUSED } else { EXIT_OK }
}

fn repo_root() -> std::io::Result<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() {
                return Ok(PathBuf::from(s));
            }
            std::env::current_dir()
        }
        _ => std::env::current_dir(),
    }
}

/// The verbs that read a fixture directory instead of hosts, as `--mock`
/// promises. Every one of them is a `match &cli.mock` arm in the dispatch, and
/// the list is named in the refusal, so adding a verb means adding it to both.
const FIXTURE_VERBS: &[&str] = &[
    "status", "diff", "history", "deploy", "verify", "rollback", "preflight", "plan", "apply", "adopt", "standup", "teardown", "federate", "survey",
];

fn takes_fixture(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Status { .. }
            | Cmd::Diff { .. }
            | Cmd::History { .. }
            | Cmd::Deploy { .. }
            | Cmd::Verify { .. }
            | Cmd::Rollback { .. }
            | Cmd::Preflight { .. }
            | Cmd::Plan { .. }
            | Cmd::Apply { .. }
            | Cmd::Adopt { .. }
            | Cmd::Standup { .. }
            | Cmd::Teardown { .. }
            | Cmd::Federate { .. }
            | Cmd::Survey { .. }
            | Cmd::Tutor { .. }
    )
}

/// The verb as it is typed, for a message about it.
fn verb_name(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Validate { .. } => "validate",
        Cmd::Doctor { .. } => "doctor",
        Cmd::Update { .. } => "update",
        Cmd::Release { .. } => "release",
        Cmd::Fastlane { .. } => "fastlane",
        Cmd::Pins { .. } => "pins",
        Cmd::Promote { .. } => "promote",
        Cmd::Secrets { .. } => "secrets",
        Cmd::Endstate { .. } => "endstate",
        Cmd::Assay { .. } => "assay",
        Cmd::JoinNode { .. } => "join-node",
        Cmd::Explain { .. } => "explain",
        _ => "this verb",
    }
}

/// The verbs that drive hosts for minutes to an hour; a fixture run takes
/// milliseconds and needs no sleep assertion.
fn long_running(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Deploy { .. }
            | Cmd::Rollback { .. }
            | Cmd::Release { .. }
            | Cmd::Fastlane { .. }
            | Cmd::Standup { .. }
            | Cmd::JoinNode { .. }
            | Cmd::Federate { .. }
            | Cmd::Teardown { .. }
            | Cmd::Apply { .. }
            | Cmd::Assay { .. }
    )
}

fn main() {
    // The language is chosen before clap builds the command, so the help clap
    // prints is already localized (goat's order, docs/design/goat-cli-i18n.md).
    i18n::select_language(i18n::preparse_lang().as_deref());
    let matches = i18n::localize_command(Cli::command()).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    if cli.offline {
        // The plays and the ledger row read it from the environment, as from the script avenue.
        std::env::set_var("GOAT_DEPLOY_OFFLINE", "1");
    }
    // `-v` (or RUST_LOG) raises goatbuilder's own events to stderr; off by default.
    trace::init(cli.verbose);
    // completion scripts, the reference and the candidate lists are machine-
    // consumed stdout: no human notice, no translation warning.
    let machine_surface = matches!(cli.cmd, Cmd::Completion { .. } | Cmd::Docs { .. } | Cmd::Complete { .. });
    if !machine_surface {
        i18n::warn_if_machine_translated();
    }
    // Verbs that need no repository: the eggs touch nothing (src/fun.rs); the
    // completion script and the reference are walked from the clap tree; the
    // candidate lists read the checkout leniently and never fail loudly.
    // The six federation verbs read only the files they are named, and the
    // site (federate/verbs.rs): no repository, no config.
    if let Cmd::Federate { verb: Some(federate::plan::Stage::Verb(_)), .. } = &cli.cmd {
        // The verbs read no repository, fixture or table output, so a flag that
        // would choose one is refused rather than silently ignored: under
        // --mock, `apply --confirm` would still write the files it names.
        if cli.mock.is_some() || cli.repo.is_some() || !matches!(cli.output, OutputFormat::Table) {
            eprintln!(
                "goatbuilder: --mock, -C/--repo and -o/--output do not apply to goatbuilder federate init|lint|compose|diff|verify|apply: \
                 the verbs read only the files they are named and the party's context, and run for real; \
                 drop the flag (the verbs take --json for a JSON report)"
            );
            std::process::exit(2);
        }
        if let Cmd::Federate { verb: Some(federate::plan::Stage::Verb(verb)), site, .. } = cli.cmd {
            std::process::exit(federate::verbs::exit_of(verb, &site));
        }
    }
    match &cli.cmd {
        Cmd::Make { words } => std::process::exit(fun::make(words, cli.yes)),
        Cmd::Argan => std::process::exit(fun::argan()),
        Cmd::Bound => std::process::exit(goat_fun::bound("goatbuilder")),
        Cmd::Completion { shell } => {
            let mut c = Cli::command();
            let name = c.get_name().to_string();
            // Written through a buffer: a closed pipe (`completion zsh | head`)
            // is the reader's right, not an error, so EPIPE is not a panic.
            let mut buf = Vec::new();
            clap_complete::generate(*shell, &mut c, name, &mut buf);
            use std::io::Write;
            let _ = std::io::stdout().write_all(&buf);
            let _ = std::io::stdout().flush();
            std::process::exit(EXIT_OK);
        }
        Cmd::Docs { markdown } => {
            let exit = match docs::run(*markdown) {
                Ok(()) => EXIT_OK,
                Err(e) => {
                    eprintln!("goatbuilder: {e}");
                    EXIT_USAGE
                }
            };
            std::process::exit(exit);
        }
        Cmd::Complete { op } => {
            let repo = cli.repo.clone().or_else(|| repo_root().ok()).unwrap_or_else(|| PathBuf::from("."));
            complete::run(&repo, op);
            std::process::exit(EXIT_OK);
        }
        _ => {}
    }
    // A fixture is not a flag a verb can half-honour: a run that names one and
    // then reads the real checkout tells the person about a file they never
    // pointed it at (output standard, rule 2).
    if cli.mock.is_some() && !takes_fixture(&cli.cmd) {
        let text = format!(
            "--mock reads a fixture instead of hosts, and {} reads this checkout, so the fixture would have been ignored. The verbs that take one: {}.",
            verb_name(&cli.cmd),
            FIXTURE_VERBS.join(", "),
        );
        eprintln!("goatbuilder: {text}");
        if goat_errors::explain_on_stderr() {
            eprint!("{}", goat_errors::render_human(&goat_errors::interpret_coded("GB-039", &text), "goatbuilder"));
        }
        std::process::exit(EXIT_USAGE);
    }
    // Long-running verbs hold the Mac awake for their own lifetime (#2122).
    awake::hold(cli.mock.is_none() && long_running(&cli.cmd));
    let repo = match cli.repo.clone().map(Ok).unwrap_or_else(repo_root) {
        Ok(p) => p,
        Err(e) => {
            let text = format!("cannot resolve the repository root: {e}");
            eprintln!("goatbuilder: {text}");
            if goat_errors::explain_on_stderr() {
                eprint!("{}", goat_errors::render_human(&goat_errors::interpret_coded("GB-027", &text), "goatbuilder"));
            }
            std::process::exit(EXIT_USAGE);
        }
    };
    output::set_machine_output(!matches!(cli.output, OutputFormat::Table));
    settings::init(&repo);
    tutor::init(&repo);
    if let Some(owner) = config::owner(&repo) {
        output::set_owner(owner);
    }

    tracing::debug!(repo = %repo.display(), cmd = ?cli.cmd, "dispatch");

    // At most once a day, on a terminal: this binary is behind the one its goatnet serves.
    if !matches!(cli.cmd, Cmd::Update { .. } | Cmd::Explain { .. }) {
        update::notice(cli.output);
    }

    if let Cmd::Explain { input, report, list } = &cli.cmd {
        let exit = explain(input, *report, *list, cli.output);
        // The tutor sees an explain run by the codes it named.
        let text = input.join(" ");
        let codes: Vec<String> = match goat_errors::explain(text.trim()) {
            Some(e) => vec![e.code.clone()],
            None => goat_errors::interpret(&text).hits.iter().map(|h| h.code.clone()).collect(),
        };
        let rows = codes.into_iter().map(|c| Check { name: "explain".into(), ok: true, detail: c, fix: None }).collect();
        let mut r = Report::new("explain", rows, exit);
        r.exit = exit;
        if let Some(done) = tutor::log_run(&cli, &Ok(r)) {
            eprintln!("{done}");
        }
        std::process::exit(exit);
    }

    let report = match &cli.cmd {
        Cmd::Explain { .. } => unreachable!("explain exits above"),
        Cmd::Make { .. } | Cmd::Argan | Cmd::Bound => unreachable!("the hidden verbs exit above"),
        Cmd::Completion { .. } | Cmd::Docs { .. } | Cmd::Complete { .. } => unreachable!("the machine surfaces exit above"),
        Cmd::Validate { config: path } => {
            // A checkout with no config of its own (the open-source export) is judged by
            // the file that declares its goatnet, goatlite.yaml, when that is all it has.
            let own = repo.join("ops/goatbuilder/goatbuilder.yaml");
            let lite = repo.join("goatlite.yaml");
            let path = path.clone().unwrap_or_else(|| if !own.is_file() && lite.is_file() { lite } else { own });
            config::validate_report(&path)
        }
        Cmd::Doctor { env, site, all, credentials, browser_tests } => doctor::run(&repo, env.as_deref(), cli.frozen, *credentials, *browser_tests).map(|mut r| {
            if let Some(site) = site {
                let extra = doctor::site_inputs(&repo, site);
                r = r.with_checks(extra);
            }
            if let Some(row) = update::doctor_row(&repo, site.as_deref()) {
                r = r.with_checks(vec![row]);
            }
            // The rows stay on the report for -o json and the tutor's log.
            if !*all && matches!(cli.output, OutputFormat::Table) {
                println!("{}", doctor::summary(&r, &repo, env.as_deref(), site.as_deref(), output::want_color(cli.color)));
                r.printed = true;
            }
            r
        }),
        Cmd::Update { site, check, force } => update::run(&repo, &update::Options { site: site.as_deref(), check: *check, force: *force, output: cli.output, timeout: cli.timeout }),
        Cmd::Status { env, component } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|f| verbs::status(&f, env, component.as_deref())).map(|mut r| {
                // The rows stay on the report for -o json and the tutor's log.
                if matches!(cli.output, OutputFormat::Table) {
                    println!("{}", verbs::status_page(&r, env, output::want_color(cli.color)));
                    r.printed = true;
                }
                r
            }),
            None => Err(goat_errors::coded(
                "GB-026",
                "status reads hosts only through a converger, and the push converger's status path is not built yet: run it with --mock <fixture>, or read the ledgers with `goatbuilder deploy --env <env> --component <c> --verify-only`",
            )
            .into()),
        },
        Cmd::Deploy { env, component, ack, group, dry_run, verify_only } => match &cli.mock {
            Some(dir) => {
                if *dry_run {
                    verbs::Fixture::open(dir).and_then(|f| verbs::deploy_plan(&f, env, component.as_deref()))
                } else if !cli.yes && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    Err(goat_errors::coded("GB-008", "deploy is a mutation: pass --yes without a terminal (C10)").into())
                } else {
                    verbs::Fixture::open(dir).and_then(|mut f| verbs::deploy(&mut f, env, component.as_deref(), ack, cli.frozen))
                }
            }
            None => match component.as_deref() {
                None => Err(goat_errors::coded("GB-018", "deploy on real hosts takes one --component").into()),
                Some(c) => {
                    if !cli.yes && !*dry_run && !*verify_only && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        Err(goat_errors::coded("GB-008", "deploy is a mutation: pass --yes without a terminal (C10)").into())
                    } else {
                        push::deploy(&repo, &push::Options { component: c, goatnet: env, only_group: group.as_deref(), dry_run: *dry_run, verify_only: *verify_only, ack })
                    }
                }
            },
        },
        Cmd::Preflight { component, env, site } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|f| preflight::run_mock(f, env, component)),
            None => preflight::run(&repo, &preflight::Options { component, goatnet: env, site: site.as_deref() }),
        },
        Cmd::Release { component, version, rc, ack, from, tag, dry_run } => release::From::parse(from).and_then(|from| {
            if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                return Err(goat_errors::coded("GB-008", "release is a mutation: pass --yes without a terminal (C10)").into());
            }
            release::release(&repo, &release::Options { component, version: version.as_deref(), rc: *rc, ack, from, tag: tag.as_deref(), dry_run: *dry_run })
        }),
        Cmd::Fastlane { component, r#ref, reason, ack, dry_run, no_deploy } => {
            if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                Err(goat_errors::coded("GB-008", "fastlane is a mutation: pass --yes without a terminal (C10)").into())
            } else {
                fastlane::fastlane(&repo, &fastlane::Options { component, reference: r#ref, reason, ack, dry_run: *dry_run, deploy: !*no_deploy })
            }
        }
        Cmd::Pins { check } => pins::run(&repo, *check),
        Cmd::Promote { cmd } => promote::run(&repo, cmd, cli.yes),
        Cmd::Secrets { args } => secrets::run(&repo, args),
        Cmd::Tutor { lesson, lesson_flag, list, reset, workspace } => {
            let lesson = &lesson.or(*lesson_flag);
            let ws = workspace.clone().unwrap_or_else(tutor::default_workspace);
            tutor::run(&repo, &ws, *lesson, *list, *reset, cli.output, output::want_color(cli.color))
        }
        Cmd::Rollback { component, n, list, status, dry_run, env, site, ack } => {
            if let Some(dir) = &cli.mock {
                match env {
                    None => Err(goat_errors::coded("GB-039", "rollback on a fixture takes --env <environment>").into()),
                    Some(_) if *list || *status => Err(goat_errors::coded("GB-039", "--list and --status read host ledgers; on a fixture, read the state with `status --env <env>`").into()),
                    Some(e) => {
                        if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                            Err(goat_errors::coded("GB-008", "rollback is a mutation: pass --yes without a terminal (C10)").into())
                        } else {
                            verbs::Fixture::open(dir).and_then(|mut f| verbs::rollback(&mut f, e, component, *n, ack, *dry_run))
                        }
                    }
                }
            } else if *list {
                rollback::list(&repo, component, site.as_deref())
            } else if *status {
                rollback::status(&repo, component, site.as_deref())
            } else if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                Err(goat_errors::coded("GB-008", "rollback is a mutation: pass --yes without a terminal (C10)").into())
            } else {
                rollback::roll(&repo, component, *n, *dry_run, site.as_deref())
            }
        }
        Cmd::Verify { component, env, group, dry_run } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|f| inspect::verify_mock(&f, component, env)),
            None => inspect::verify(&repo, component, env, group.as_deref(), *dry_run),
        },
        Cmd::Diff { component, env, all: _ } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|f| inspect::diff_mock(&f, env, component.as_deref())),
            None => inspect::diff(&repo, env, component.as_deref()),
        },
        Cmd::History { component, component_flag, env } => {
            let component = component.as_deref().or(component_flag.as_deref()).unwrap_or_default();
            match &cli.mock {
                Some(dir) => verbs::Fixture::open(dir).and_then(|f| inspect::history_mock(&f, component, env)),
                None => inspect::history(&repo, component, env),
            }
        }
        Cmd::Plan { verb, component, env, group, n, site, out } => inspect::plan(&repo, cli.mock.as_deref(), &inspect::PlanOpts { verb, component, env: env.as_deref(), group: group.as_deref(), n: *n, site: site.as_deref(), out }),
        Cmd::Apply { plan } => {
            if !cli.yes && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                Err(anyhow::anyhow!("apply is a mutation: pass --yes without a terminal (C10)"))
            } else {
                inspect::apply(&repo, cli.mock.as_deref(), plan)
            }
        }
        Cmd::Endstate { site, facts, assay_rows, no_assay, strict, root } => endstate::run(
            &repo,
            &endstate::Options {
                site,
                facts: facts.as_deref(),
                assay_rows: assay_rows.as_deref(),
                run_assay: !*no_assay && facts.is_none(),
                strict: *strict,
                root: root.as_deref(),
                stream: matches!(cli.output, OutputFormat::Table),
            },
        ),
        Cmd::Assay { site, plan, only, vars, rows_out, values } => assay::run(
            &repo,
            &assay::Options { site, plan: *plan, only: only.as_deref(), vars, json_out: rows_out.as_deref(), values: values.as_deref(), stream: matches!(cli.output, OutputFormat::Table) },
        ),
        Cmd::Survey { envs, all, intra, inter, html } => {
            let html = html.as_deref().map(survey::html_path);
            let o = survey::Options { envs, all: *all, intra: *intra || !*inter, inter: *inter || !*intra, html: html.as_deref(), output: cli.output };
            match &cli.mock {
                Some(dir) => verbs::Fixture::open(dir).and_then(|f| survey::run_mock(&f, &o)),
                None => survey::run(&repo, &o),
            }
        }
        Cmd::Adopt { env } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|f| inspect::adopt_mock(&f, env, cli.yes)),
            None => inspect::adopt(&repo, env, cli.yes),
        },
        Cmd::Standup { env, site, profile, ack, from, to, skip_obs, obs_only, dry_run, parallel, check } => match &cli.mock {
            // The site is derived when --site is left out (one box, ADR 1213), and a
            // rehearsal still honours --dry-run, --from and --to.
            Some(dir) => verbs::Fixture::open(dir).and_then(|mut f| {
                let site = standup::site_for(&repo, Some(&f.config), env, site.as_deref())?;
                standup::mock(&mut f, env, &site, ack, standup::MockRun { dry_run: *dry_run, from: *from, to: *to })
            }),
            None => standup::site_for(&repo, None, env, site.as_deref()).and_then(|site| {
                if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    Err(anyhow::anyhow!("standup is a mutation: pass --yes without a terminal (C10)"))
                } else {
                    standup::standup(&repo, &standup::Options { env, site: &site, profile: profile.as_deref(), from: *from, to: *to, skip_obs: *skip_obs, obs_only: *obs_only, dry_run: *dry_run, parallel: *parallel, check: *check })
                }
            }),
        },
        Cmd::Teardown { env, site, profile, dry_run, services, from, to } => match &cli.mock {
            Some(dir) => verbs::Fixture::open(dir).and_then(|mut f| standup::teardown_mock(&mut f, env, site, standup::MockRun { dry_run: *dry_run, from: *from, to: *to })),
            None if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) => {
                Err(anyhow::anyhow!("teardown is a mutation: pass --yes without a terminal (C10)"))
            }
            None => standup::teardown(&repo, &standup::Options { env, site, profile: profile.as_deref(), from: *from, to: *to, skip_obs: false, obs_only: false, dry_run: *dry_run, parallel: false, check: false }, *services),
        },
        Cmd::JoinNode { env, site, node, class, address, pass_var, via, from, to, dry_run } => {
            if !cli.yes && !*dry_run && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                Err(anyhow::anyhow!("join-node is a mutation: pass --yes without a terminal (C10)"))
            } else {
                joinnode::join_node(&repo, &joinnode::Options { env, site, node, class, address: address.as_deref(), pass_var: pass_var.as_deref(), via, from: *from, to: *to, dry_run: *dry_run })
            }
        }
        Cmd::Federate { verb: Some(stage), site, .. } => {
            if !site.is_empty() {
                Err(anyhow::anyhow!("--context-dir, --cert, --ca-roots, --context and --profile-dir apply to the six verbs (goatbuilder federate init|lint|compose|diff|verify|apply), not to goatbuilder federate plan"))
            } else {
                federate::plan::run(&repo, stage, cli.mock.as_deref())
            }
        }
        Cmd::Federate { verb: None, site, with, partner, env, as_party, edge_group, work, from, to, dry_run, resume } => {
            let with = with.as_deref().unwrap_or_default();
            let cfg_path = cli.mock.as_ref().map(|m| m.join("goatbuilder.yaml")).unwrap_or_else(|| repo.join("ops/goatbuilder/goatbuilder.yaml"));
            if with == "status" {
                let format = match cli.output {
                    OutputFormat::Table => federate::status::Format::Table,
                    OutputFormat::Json => federate::status::Format::Json,
                    OutputFormat::Yaml => federate::status::Format::Yaml,
                };
                federate::status::status(&cfg_path, partner.as_deref(), env.as_deref(), work.as_deref(), format)
            } else if let Some(extra) = partner {
                Err(anyhow::anyhow!("unexpected argument '{extra}': only a stage word (status) takes a partner after it"))
            // A mock executes nothing, with or without --dry-run, so it mutates nothing.
            } else if !site.is_empty() && goat_federation::verbs::VERB_NAMES.contains(&with) {
                // `federate --context-dir D init`: clap read the verb as a partner.
                Err(anyhow::anyhow!("put --context-dir, --cert, --ca-roots, --context and --profile-dir after the verb: goatbuilder federate {with} --context-dir <dir> ..."))
            } else if !site.is_empty() {
                Err(anyhow::anyhow!("--context-dir, --cert, --ca-roots, --context and --profile-dir apply to the six verbs (goatbuilder federate init|lint|compose|diff|verify|apply), not to a peering run"))
            } else if !cli.yes && !*dry_run && cli.mock.is_none() && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                Err(anyhow::anyhow!("federate is a mutation: pass --yes without a terminal (C10)"))
            } else {
                federate::federate_with(&repo, &federate::Options { with, env: env.as_deref(), r#as: as_party.as_deref(), edge_group: edge_group.as_deref(), work: work.as_deref(), mock: cli.mock.as_deref(), from: *from, to: *to, dry_run: *dry_run, table: matches!(cli.output, OutputFormat::Table) }, &federate::Surface { resume: *resume })
            }
        }
    };

    // A run inside the tutor's workspace (or with $GOATBUILDER_TUTOR set) is
    // logged there, so `goatbuilder tutor` can check what the learner did.
    // `secrets exec -- <command>` carries someone else's command line, which
    // can hold anything; the tutor has no lesson that needs it recorded.
    let tutor_done = if matches!(cli.cmd, Cmd::Tutor { .. } | Cmd::Secrets { .. }) { None } else { tutor::log_run(&cli, &report) };

    match report {
        Ok(report) => {
            // A verb that printed a document of its own (`promote status --json`,
            // `promote readiness`) carries no rows; nothing is appended to it.
            let already_shown = report.printed && matches!(cli.output, OutputFormat::Table);
            if !report.checks.is_empty() && !already_shown {
                println!("{}", output::render(&report, cli.output, cli.color));
            }
            if let Some(done) = &tutor_done {
                eprintln!("{done}");
            }
            std::process::exit(report.exit);
        }
        Err(e) => {
            // Every error leaves this binary interpreted (ADR 1199): the
            // message in the script's own words, then — at a terminal, or
            // with GOAT_EXPLAIN=1 — what it means and what to do, for our own
            // coded refusal and for any tool's words carried inside it.
            let text = format!("{e:#}");
            let code = e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone());
            let interp = match &code {
                Some(c) => goat_errors::interpret_coded(c, &text),
                None => goat_errors::interpret(&text),
            };
            match cli.output {
                OutputFormat::Table => {
                    let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
                    let on = output::want_color(cli.color);
                    if terminal {
                        eprintln!("{}\n", output::refusal_line(&text, on));
                    } else {
                        eprintln!("goatbuilder: {text}");
                    }
                    if goat_errors::explain_on_stderr() {
                        let human = goat_errors::render_human(&interp, "goatbuilder");
                        if terminal {
                            eprint!("{}", output::explanation_page(&human, on));
                        } else {
                            eprint!("{human}");
                        }
                    } else if let Some(c) = &code {
                        eprintln!("  ({c} — goatbuilder explain {c})");
                    }
                    if let Some(done) = &tutor_done {
                        eprintln!("{done}");
                    }
                }
                // The human line stays on stderr; the structured document goes
                // to stdout like every report, so a consumer parses one stream.
                OutputFormat::Json => {
                    eprintln!("goatbuilder: {text}");
                    println!("{}", serde_json::to_string_pretty(&ErrorOut { schema: "goatbuilder.error/v1", error: &text, code: code.as_deref(), interpretation: &interp }).unwrap_or_default());
                }
                OutputFormat::Yaml => {
                    eprintln!("goatbuilder: {text}");
                    println!("{}", serde_yaml::to_string(&ErrorOut { schema: "goatbuilder.error/v1", error: &text, code: code.as_deref(), interpretation: &interp }).unwrap_or_default());
                }
            }
            let exit = code.as_deref().and_then(goat_errors::explain).and_then(|e| e.exit).unwrap_or(EXIT_USAGE);
            std::process::exit(exit);
        }
    }
}

/// The machine shape of an interpreted error (`-o json|yaml`).
#[derive(Serialize)]
struct ErrorOut<'a> {
    schema: &'static str,
    error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    interpretation: &'a goat_errors::Interpretation,
}

#[cfg(test)]
mod ack_tests {
    use super::parse_ack;

    /// A placeholder never reaches the record as the actor; a name does, and
    /// an empty value (no --ack given) is left for each verb to require.
    #[test]
    fn a_placeholder_ack_is_refused() {
        assert!(parse_ack("<your name>").is_err());
        assert!(parse_ack("<name>").is_err());
        assert_eq!(parse_ack("Test Person").unwrap(), "Test Person");
        assert_eq!(parse_ack("").unwrap(), "");
    }
}

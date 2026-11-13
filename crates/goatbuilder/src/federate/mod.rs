//! `federate`: a sovereign peering declared, converged and verified — the verb
//! that replaces ops/scripts/peering-standup.sh behind the parity gate. The
//! script's phases, in its order, with its words:
//!
//!   1  GATE   preflight, fail-closed (ops/federation/standup/preflight.py)
//!   2  A      prepare what the package measures: the federation interface and
//!             the SPIRE bundle to exchange
//!   3  T      init — our half, read from the running system
//!   4  T      lint our package against the spec
//!   5  MANUAL exchange through the directory; read the key and CA fingerprint aloud
//!   6  T      compose — one agreement, one digest, no perspective
//!   7  T      verify the agreement from our side
//!   8  T      mint the peering leaf certificates from the agreement
//!   9  T      apply — render the access rules and configuration (the only writer)
//!  10  MANUAL review what apply rendered; admit the edge on our core
//!  11  A/GATE open the transport the AGREEMENT chose — or refuse when no play
//!             stands that transport up
//!  12  A      deploy the sovereign edge router and the outbound republisher
//!             (the republisher on our core host, inventory group <peering>_core)
//!  13  A      federate the SPIRE trust domains (ADR 1157 step 2)
//!  14  GATE   acceptance — carries, refuses, and round-trips
//!
//! `--dry-run` is the script's `--plan`: every phase and command, nothing run.
//! `--mock <fixture>` never executes either, with or without `--dry-run`: the
//! phases are planned and recorded against the fixture, because a worked example
//! has no hosts and a mock that ran real commands would run them on the
//! learner's machine.
//!
//! The peering must be declared in goatbuilder.yaml `peerings:` (C1: declared
//! first). An empty list declares nothing, so it refuses every name.
//!
//! Phases 3, 4, 6, 7 and 9 are the federation verbs. By default they run in
//! process, through the library `goat federate` also runs (`verbs.rs`), and the
//! plan prints them as `goatbuilder federate <verb> ...`, the words that are
//! parsed and run; the script calls `goatbuilder federate <verb>` too.
//! `FED_VERBS=goat` runs them as `goat federate <verb>` in a shell instead,
//! as before, on both avenues.

/// `goatbuilder federate init|lint|compose|diff|verify|apply` (the six verbs).
pub mod verbs;
pub mod runenv;

use std::path::Path;

pub mod backout;
pub mod rotation;

use anyhow::{anyhow, Context, Result};

use crate::config;

// Phase 3 measured over SSH: `goatbuilder federate init --over-ssh`, and the
// driver's phase 3 unless FED_MEASURE=here.
pub mod measure;
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub mod alloc;
pub mod declaration;
pub mod federation_toml;
pub mod declared;
pub mod forwarder;
pub mod hostalloc;
pub mod hostfw;
pub mod peering_files;
mod admit;
mod admit_leaf;
mod admit_role;
mod confirm;
pub mod deploy;
pub mod site_values;
pub mod switches;
pub mod site_hosts;
pub mod lock;
pub mod prompt;
mod run;
pub mod state;
pub mod status;
pub use admit::Admission;
pub use confirm::Values;
pub use run::Surface;
pub mod plan;
pub mod preflight;

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub with: &'a str,
    pub env: Option<&'a str>,
    pub r#as: Option<&'a str>,
    pub edge_group: Option<&'a str>,
    /// The working directory; default `$FED_WORK`, else /var/lib/goat-federation/<peering>.
    pub work: Option<&'a str>,
    /// A fixture directory whose flat `goatbuilder.yaml` supplies the config
    /// (its `peerings:` and environments), instead of `<repo>/ops/goatbuilder/`.
    /// The same `--mock` every other verb honors, so a worked-example peering is
    /// planned self-contained; the plan's commands still name the repository's
    /// own scripts, because a real run executes those, not the fixture's.
    pub mock: Option<&'a Path>,
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
    /// Output is the human table: a dry run then prints the plan as a plan
    /// (header, phase table, commands) instead of one row per line.
    pub table: bool,
}

struct Phase {
    n: u32,
    kind: &'static str,
    label: String,
    /// What the plan prints; a shell command, run with `bash -c` unless `verb` is set.
    cmd: String,
    /// A federation verb run in process. `cmd` is these words, printed.
    verb: Option<VerbCall>,
}

/// A federation verb run in process: the environment it runs with, and its
/// words after `goatbuilder federate`.
type VerbCall = (Vec<(String, String)>, Vec<String>);

/// Whether phases 3, 4, 6, 7 and 9 run in process (unset or `goatbuilder`, the
/// default) or spawn `goat federate` (`FED_VERBS=goat`).
fn verbs_in_process() -> bool {
    in_process_by(&env_or("FED_VERBS", String::new()))
}

/// The choice `verbs_in_process` makes, from the value of `FED_VERBS`.
fn in_process_by(fed_verbs: &str) -> bool {
    fed_verbs != "goat"
}

/// A verb's words as a shell line: the verb and each flag bare, every value
/// single-quoted, the way the script prints them.
fn shell_words(words: &[String]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(i, w)| if i == 0 || w.starts_with("--") { w.clone() } else { format!("'{w}'") })
        .collect::<Vec<_>>()
        .join(" ")
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

fn env_or(name: &str, default: String) -> String {
    runenv::var(name).ok().filter(|v| !v.is_empty()).unwrap_or(default)
}

/// `$(command -v goat || echo goat)`.
fn goat_bin() -> String {
    env_or("GOAT_BIN", {
        let path = runenv::var("PATH").unwrap_or_default();
        path.split(':').map(|d| Path::new(d).join("goat")).find(|p| p.is_file()).map(|p| p.display().to_string()).unwrap_or_else(|| "goat".to_string())
    })
}

/// Whether a value read from a file may go into a shell command as it is:
/// these commands run through `bash -c`, so anything else (a quote, `$(...)`)
/// could run as code (docs/design/goatbuilder-output.md, rule 6). A transport
/// kind or a `sovereign=key` line never needs more than this.
fn shell_safe(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._:+/=-".contains(c))
}

/// The directory step (ops/federation/standup/directory.py), run inside
/// phase 5 before it stops. `None` when FED_DIRECTORY_URL is unset, or when it,
/// FED_PACKAGE_URL, FED_COUNTERPARTY_ID or FED_CA_CERT holds anything but plain URL
/// characters: the values are printed into the plan and passed to the helper.
fn directory_step() -> Option<(String, Option<String>, Option<String>)> {
    let url = env_or("FED_DIRECTORY_URL", String::new());
    if url.is_empty() {
        return None;
    }
    let pkg = env_or("FED_PACKAGE_URL", String::new());
    let id = env_or("FED_COUNTERPARTY_ID", String::new());
    let ca = env_or("FED_CA_CERT", String::new());
    let plain = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "._:/?=&%~+@-".contains(c));
    if !(plain(&url) && plain(&pkg) && plain(&id) && plain(&ca)) {
        return None;
    }
    Some((url, Some(pkg).filter(|v| !v.is_empty()), Some(id).filter(|v| !v.is_empty())))
}

/// Whether phase 5 asks the directory step for one package per counterparty
/// (`--per-counterparty`; gate row B6, ADR 1225 decision 6). On unless the
/// operator sets FED_DIRECTORY_PER_COUNTERPARTY: then the command adds no flag
/// and directory.py reads the variable itself (1, true or yes on; 0, false,
/// no or empty off; any other word refused, the step doing nothing). Off is
/// the single slot and pointer, for a directory or partner that has no
/// per-counterparty slots.
fn per_counterparty(switch: Option<std::ffi::OsString>) -> bool {
    switch.is_none()
}

fn per_counterparty_by_default() -> bool {
    per_counterparty(std::env::var_os("FED_DIRECTORY_PER_COUNTERPARTY"))
}

/// Phase 5's label and body: the script's words, and the switch the script
/// leaves off.
fn exchange(r: &str, ours: &str, work: &str, counterparty: &str, keyring: &str) -> (&'static str, String) {
    let directory_label = "exchange through the directory; read the key and CA fingerprint aloud";
    let (label, head) = if env_or("FED_NO_DIRECTORY", String::new()) == "1" {
        (
            "exchange packages by hand (FED_NO_DIRECTORY=1); read the key and CA fingerprint aloud",
            format!("No directory: FED_NO_DIRECTORY=1 is for a counterparty that can reach none.\n     Send {ours} and our CA certificate to the counterparty, with {work}/ours.spiffe-bundle if phase 2 wrote one.\n     Obtain theirs and place them at: {counterparty}\n       {work}/theirs-ca.pem\n       {work}/theirs.spiffe-bundle"),
        )
    } else {
        match directory_step() {
            Some((url, pkg, id)) => (
                directory_label,
                format!(
                    "Directory step, run before this stop (it grants nothing):\n       python3 '{r}/ops/federation/standup/directory.py' --url '{url}' --ours '{ours}'{} --their-id '{}'{}\n     It uploads our package (unless FED_PACKAGE_URL names our own host), CA and\n     SPIFFE bundle (when phase 2 wrote one), publishes our pointer, fetches\n     theirs to {counterparty}, {work}/theirs-ca.pem and\n     {work}/theirs.spiffe-bundle, checks digest, signatures and CA\n     fingerprint, and prints both verifying keys and CA fingerprints in groups\n     of four and a compatibility report. Their entry not there yet: run phase 5\n     again after they run theirs.{}",
                    pkg.map(|p| format!(" --package-url '{p}'")).unwrap_or_default(),
                    id.unwrap_or_else(|| "<unset>".to_string()),
                    if per_counterparty_by_default() { " --per-counterparty" } else { "" },
                    if per_counterparty_by_default() {
                        "\n     One package per counterparty: ours also goes to our slot for them, and\n     theirs is looked up first in their slot for us, so each of our peerings\n     keeps its own package. Until their key is in the keyring, their CA and\n     SPIFFE bundle wait beside it as <file>.unconfirmed: once both keys are\n     recorded, run phase 5 again to place them. A package that reached us only\n     through their pointer is held until it is confirmed on the call.\n     FED_DIRECTORY_PER_COUNTERPARTY=0 is the single slot and pointer alone."
                    } else {
                        ""
                    }
                ),
            ),
            None if !env_or("FED_DIRECTORY_URL", String::new()).is_empty() => (
                directory_label,
                "FED_DIRECTORY_URL, FED_PACKAGE_URL, FED_COUNTERPARTY_ID or FED_CA_CERT holds\n     characters outside a plain URL; phase 1 refuses this run.".to_string(),
            ),
            None => (
                directory_label,
                "Set FED_DIRECTORY_URL, FED_COUNTERPARTY_ID and FED_SIGNING_KEY; phase 1 refuses\n     without them. The directory step then uploads our package and CA, fetches\n     theirs, and prints both keys and CA fingerprints and a compatibility report.\n     FED_NO_DIRECTORY=1 is for a counterparty that can reach no directory.".to_string(),
            ),
        }
    };
    (
        label,
        format!("{head}\n     On one call, each side reads its verifying key and CA fingerprint aloud and\n     the other reads them back: a key taken from the package it verifies proves\n     nothing. Then record BOTH parties' keys, one per line, at:\n       {keyring}\n     as   <sovereign_id>=<44 base64 characters>\n     Phase 6 passes each line to compose as --key; compose refuses (exit 20)\n     for any party whose key is not bound. After compose, read the agreement\n     digest to each other the same way."),
    )
}

/// The directory step's arguments, for a real run of phase 5. No shell: each
/// value is one argument.
fn directory_args(repo: &Path, work: &str) -> Option<Vec<String>> {
    let (url, pkg, id) = directory_step()?;
    let mut a = vec![repo.join("ops/federation/standup/directory.py").display().to_string(), "--url".into(), url, "--ours".into(), format!("{work}/ours.canonical.json")];
    if let Some(p) = pkg {
        a.extend(["--package-url".into(), p]);
    }
    let key = env_or("FED_SIGNING_KEY", String::new());
    if !key.is_empty() {
        a.extend(["--key".into(), key]);
    }
    let ca = env_or("FED_CA_CERT", String::new());
    if !ca.is_empty() {
        a.extend(["--ca".into(), ca]);
    }
    a.extend(["--theirs".into(), env_or("FED_COUNTERPARTY", format!("{work}/theirs.canonical.json"))]);
    if let Some(i) = id {
        a.extend(["--their-id".into(), i]);
    }
    a.extend(["--keyring".into(), env_or("FED_KEYRING", format!("{work}/keys.txt")), "--work".into(), work.to_string()]);
    if per_counterparty_by_default() {
        a.push("--per-counterparty".into());
    }
    Some(a)
}

fn plan(repo: &Path, o: &Options, work: &str, edge_group: &str, resume: &str) -> Vec<Phase> {
    plan_with(repo, o, work, edge_group, resume, verbs_in_process(), measure::driver_measures_over_ssh())
}

/// `plan`, with the choice of who runs the federation verbs made by the caller.
#[cfg(test)]
fn plan_verbs(repo: &Path, o: &Options, work: &str, edge_group: &str, resume: &str, in_process: bool) -> Vec<Phase> {
    plan_with(repo, o, work, edge_group, resume, in_process, false)
}

/// `plan_verbs`, and whether phase 3 measures over SSH (FED_MEASURE=ssh): then
/// phase 3 runs `goatbuilder federate init --over-ssh` in process, whoever runs
/// the other verbs, and no DOCKER_HOST is handed to it.
fn plan_with(repo: &Path, o: &Options, work: &str, edge_group: &str, resume: &str, in_process: bool, measure_ssh: bool) -> Vec<Phase> {
    let r = repo.display().to_string();
    let goat = goat_bin();
    let aws = crate::settings::aws_profile(repo);
    let signing = env_or("FED_SIGNING_KEY", String::new());
    let counterparty = env_or("FED_COUNTERPARTY", format!("{work}/theirs.canonical.json"));
    let keyring = env_or("FED_KEYRING", format!("{work}/keys.txt"));
    let ours = format!("{work}/ours.canonical.json");
    let agreement = format!("{work}/agreement.json");
    let applied = format!("{work}/applied");
    let pki = format!("{work}/pki");
    let as_arg = o.r#as.map(|a| format!(" --as '{a}'")).unwrap_or_default();
    let p = |n: u32, kind: &'static str, label: &str, cmd: String| Phase { n, kind, label: label.to_string(), cmd, verb: None };
    // A federation verb: `goat federate <words>` in a shell, or the same words
    // run in process and printed as `goatbuilder federate <words>`.
    let t = |n: u32, label: &str, env: Vec<(String, String)>, words: Vec<String>| {
        let prefix: String = env.iter().map(|(k, v)| format!("{k}='{v}' ")).collect();
        if in_process {
            Phase { n, kind: "T", label: label.to_string(), cmd: format!("{prefix}goatbuilder federate {}", shell_words(&words)), verb: Some((env, words)) }
        } else {
            Phase { n, kind: "T", label: label.to_string(), cmd: format!("{prefix}{goat} federate {}", shell_words(&words)), verb: None }
        }
    };
    let w = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let as_words: Vec<String> = o.r#as.map(|a| vec!["--as".to_string(), a.to_string()]).unwrap_or_default();

    let router_host = env_or("FED_ROUTER_DOCKER_HOST", String::new());
    let init_env: Vec<(String, String)> = if router_host.is_empty() { Vec::new() } else { vec![("DOCKER_HOST".into(), router_host.clone())] };
    let mut init_words = w(&["init", "--with-transport", "--with-data"]);
    if !signing.is_empty() {
        init_words.extend(w(&["--signing-key", &signing]));
    }
    // The peering's generated federation.toml (FED_DECLARATION, federate/preflight.rs).
    let declaration = env_or("FED_DECLARATION", String::new());
    if !declaration.is_empty() {
        init_words.extend(w(&["--declaration", &declaration]));
    }
    init_words.extend(w(&["--out", &ours]));
    // This peering's own inventory file (`goatbuilder federate allocate`, when
    // not every peering declares hosts:), read beside the hand-written one.
    let own_inv = format!("inventory/peering-{edge_group}.ini");
    let more_inv = if repo.join("ops/ansible").join(&own_inv).is_file() { format!(" -i {own_inv}") } else { String::new() };
    let play_on = |limit: &str, name: &str| format!("cd '{r}/ops/ansible' && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini{more_inv} --limit '{limit}' -e fed_edge_group='{edge_group}' -e fed_work='{work}' -e fed_counterparty='{counterparty}' {name}");
    let play = |name: &str| play_on(edge_group, name);
    // The core host the edge dials: the outbound republisher runs there, beside
    // our core router, and both of its sessions dial that router (phase 12).
    let core_group = format!("{}_core", o.with);

    let mut ev = vec![
        p(1, "GATE", "peering preflight (fail-closed)", format!("python3 '{r}/ops/federation/standup/preflight.py' '{}' --work '{work}' --edge-group '{edge_group}'", o.with)),
        p(2, "A", "prepare what the package measures: the federation interface and the SPIRE bundle", play("peering-prepare.yml")),
        if measure_ssh {
            let words = measure::phase3_words(init_words, o.with, edge_group, runenv::var("FED_ROUTER_CONTAINER").ok().as_deref(), repo);
            Phase { n: 3, kind: "T", label: "init — our half, read from the running system".into(), cmd: format!("goatbuilder federate {}", shell_words(&words)), verb: Some((Vec::new(), words)) }
        } else {
            t(3, "init — our half, read from the running system", init_env, init_words)
        },
        t(4, "lint our package against the spec", Vec::new(), w(&["lint", &ours])),
        { let (label, body) = exchange(&r, &ours, work, &counterparty, &keyring); p(5, "MANUAL", label, body) },
    ];
    // Each `sovereign=key` line in the keyring becomes one --key argument; a
    // missing keyring is left for compose's own refusal to name.
    let mut compose_words = w(&["compose", &ours, &counterparty]);
    if let Ok(text) = std::fs::read_to_string(&keyring) {
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let clean: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            // An unsafe line is left out: compose then refuses, naming the
            // party whose key is not bound (fail closed).
            if shell_safe(&clean) {
                compose_words.extend(w(&["--key", &clean]));
            }
        }
    }
    compose_words.extend(w(&["--out", &agreement]));
    ev.push(t(6, "compose — one agreement, one digest, no perspective", Vec::new(), compose_words));
    ev.push(t(7, "verify the agreement from our side", Vec::new(), [w(&["verify", &agreement]), as_words.clone()].concat()));
    ev.push(p(8, "T", "mint the peering leaf certificates from the agreement", format!("'{r}/ops/federation/standup/mint-peering-leaves.sh' '{agreement}' '{pki}'{as_arg}")));
    ev.push(t(9, "apply — render the access rules and configuration", Vec::new(), [w(&["apply", &agreement]), as_words.clone(), w(&["--edge-group", edge_group, "--out", &applied, "--confirm"])].concat()));
    ev.push(p(10, "MANUAL", "review what apply rendered, and admit the edge on our core", format!(
            "Review {applied} against the agreement. The review is the control.\n     Check {work}/theirs-ca.pem against the fingerprint `apply` printed for the\n     counterparty, then place it where goat_zenoh_edge_partner_ca_path says.\n     Splice {applied}/ops/federation/acl/<them>-fragment.json5 into our core's\n     access control with roles/goat-zenoh/files/acl-assemble.py, and give the core\n     {pki}/core.pem and its key. The core has to admit the edge before phase 12's\n     inward-link check can pass, and only this party can change this party's core.")));
    // The transport the agreement chose, read from the agreement itself.
    let transport = std::fs::read_to_string(&agreement)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("transport").and_then(|t| t.get("kind")).and_then(|k| k.as_str()).map(String::from))
        .filter(|k| shell_safe(k))
        .unwrap_or_default();
    let transport_play = match transport.as_str() {
        "wireguard" => Some("peering-wg.yml"),
        "ipsec-fips" => Some("peering-edge.yml"),
        _ => None,
    };
    match transport_play {
        Some(name) => ev.push(p(11, "A", &format!("open the {transport} transport"), play(name))),
        None => {
            let shown = if transport.is_empty() { "unknown".to_string() } else { transport.clone() };
            let unreadable = if transport.is_empty() { "<unreadable>".to_string() } else { transport.clone() };
            ev.push(p(11, "GATE", &format!("open the transport — REFUSED, no play for '{shown}'"), format!(
                "echo \"    The agreement selected transport '{unreadable}' and this repository\";\n     echo \"    has no play that stands one up: wireguard is peering-wg.yml, ipsec-fips is peering-edge.yml.\";\n     echo \"\";\n     echo \"    Do NOT substitute the other transport: a tunnel the two parties did not\";\n     echo \"    agree is exactly the drift between what was agreed and what got deployed\";\n     echo \"    that this flow exists to prevent.\";\n     false")));
        }
    }
    ev.push(p(12, "A", "deploy the sovereign edge router and the outbound republisher", format!("{} && {}", play("peering-zenoh-edge.yml"), play_on(&core_group, "peering-republisher.yml"))));
    ev.push(p(13, "A", "federate the SPIRE trust domains (ADR 1157 step 2)", play("peering-spire.yml")));
    ev.push(p(14, "GATE", "acceptance — carries, refuses, and round-trips", format!("'{r}/ops/federation/standup/acceptance.sh' '{agreement}'{as_arg}")));
    let _ = resume;
    ev
}

/// Our sovereign id when `--as` is absent, by the same rule on both avenues
/// (ops/scripts/peering-standup.sh): `sovereign_id` of the declaration in the
/// goat context the verbs read (`federation.toml`, found as `verbs::resolve_site`
/// finds it), else `sovereign_id` of our half, `<work>/ours.canonical.json`.
/// None when neither names a well-formed id.
pub(crate) fn our_id(work: &str) -> Option<String> {
    let well_formed = |s: &String| {
        (2..=32).contains(&s.len()) && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let declared = verbs::resolve_site(&verbs::SiteArgs::default())
        .ok()
        .and_then(|site| std::fs::read_to_string(site.declaration_path()).ok())
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.get("sovereign_id").and_then(|x| x.as_str()).map(String::from))
        .filter(well_formed);
    declared.or_else(|| {
        std::fs::read_to_string(format!("{work}/ours.canonical.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("sovereign_id").and_then(|x| x.as_str()).map(String::from))
            .filter(well_formed)
    })
}

/// How a dry run reads to a person, the way kubeadm prints its phases: a
/// header, one aligned table of phases, then each phase's command wrapped to
/// the page, with this checkout's path and the home folder shortened.
/// What federate's phase `n` is for, in a person's words: the plan page's step
/// column and the runner's frame read it.
fn purpose(n: u32) -> &'static str {
    match n {
        1 => "Check this site is ready to peer",
        2 => "Make the link interface and export our trust bundle",
        3 => "Build our half of the agreement from the running system",
        4 => "Check our half is valid",
        5 => "Swap halves through the directory, and read their key back",
        6 => "Combine both halves into one agreement",
        7 => "Check the agreement from our side",
        8 => "Create the certificates for this peering",
        9 => "Write the access rules and settings",
        10 => "Review what was written, and let the router onto our core",
        11 => "Open the connection the agreement chooses",
        12 => "Deploy the edge router, and the republisher on our core",
        13 => "Trust the other party's identity service",
        14 => "Test that allowed traffic passes and the rest is refused",
        _ => "",
    }
}

fn plan_view(repo: &Path, o: &Options, work: &str, edge_group: &str, phases: &[Phase]) -> String {
    let home = runenv::var("HOME").unwrap_or_default();
    // The resolved path: with `-C ../..` the literal "../../" must not be
    // stripped from paths that merely contain it.
    let repo_s = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf()).display().to_string();
    let quoted = regex::Regex::new(r"'([A-Za-z0-9_./:@=+,~-]+)'").expect("regex");
    // A real folder name (never `.` or `..`) followed by `/../`.
    let updir = regex::Regex::new(r"(?:^|/|\s)[A-Za-z0-9_][A-Za-z0-9_.-]*/\.\./").expect("regex");
    let short = |s: &str| {
        let mut s = quoted.replace_all(s, "$1").to_string();
        s = s.replace(&format!("{repo_s}/"), "").replace(" — ", ": ");
        // a/b/../../c reads as c: fold each `dir/../` until none is left.
        while let Some(m) = updir.find(&s) {
            // keep the separator the match began with
            let keep = s[m.start()..].chars().next().filter(|c| *c == '/' || c.is_whitespace()).map(String::from).unwrap_or_default();
            s.replace_range(m.range(), &keep);
        }
        if home.is_empty() { s } else { s.replace(&home, "~") }
    };
    fn kind(k: &'static str) -> &'static str {
        match k {
            "T" => "tool",
            "A" => "ansible",
            "GATE" => "gate",
            "MANUAL" => "manual",
            other => other,
        }
    }
    let agreement_known = Path::new(&format!("{work}/agreement.json")).is_file();
    // The steps in plain words, for a person. The rows and -o json keep the
    // labels the parity gate compares with the script.
    let step = |ph: &Phase| {
        if ph.n == 11 && agreement_known && ph.label.contains("REFUSED, no play for '") {
            let chose = ph.label.split("no play for '").nth(1).and_then(|r| r.split('\'').next()).unwrap_or("?");
            return format!("Open the connection: refused, the agreement chose '{chose}'");
        }
        let plain = purpose(ph.n);
        if !plain.is_empty() {
            return plain.to_string();
        }
        let l = ph.label.replace(" \u{2014} ", ": ");
        let mut c = l.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let shown: Vec<&Phase> = phases.iter().filter(|p| p.n >= o.from && p.n <= o.to).collect();
    let color = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let mut s = format!("{} federate {}\n", bold("Plan:"), o.with);
    s.push_str("Nothing has run. This only shows the steps, so there is nothing to do now.\n\n");
    s.push_str(&format!("  Working folder:  {}\n  Router group:    {edge_group}\n\n", short(work)));
    s.push_str("PHASE  TYPE     STEP\n");
    for ph in &shown {
        s.push_str(&format!("{:<6} {:<8} {}\n", ph.n, kind(ph.kind), step(ph)));
    }
    if let Some(ph) = shown.iter().find(|p| p.kind == "MANUAL") {
        let found = |re: &str| regex::Regex::new(re).ok().and_then(|r| r.captures(&ph.cmd).map(|m| short(&m[1])));
        let (ours, theirs, keys) = (found(r"Send (\S+) (?:and|to)"), found(r"place (?:it|them) at:\s+(\S+)"), found(r"line, at:\s+(\S+)"));
        s.push_str(&format!("\n{}\n", bold(&format!("What a real run would leave to you, at phase {}:", ph.n))));
        match (ours, theirs, keys) {
            (Some(ours), Some(theirs), Some(keys)) => {
                let steps: [(&str, Option<&str>, &str); 4] = [
                    ("No directory: send your half and your CA certificate to the other party", Some(&ours), ""),
                    ("Get their half, CA certificate and trust bundle, and save them here", Some(&theirs), ""),
                    ("Read your key and CA fingerprint aloud on one call", None, "Each side reads the other's back. A key that arrives inside their package proves nothing."),
                    ("Save both keys here, one per line, as sovereign_id=key", Some(&keys), ""),
                ];
                for (i, (title, file, note)) in steps.iter().enumerate() {
                    s.push('\n');
                    s.push_str(&crate::output::hand_step(i + 1, title, *file, note, color));
                }
            }
            (None, None, Some(keys)) => {
                let steps: [(&str, Option<&str>, &str); 3] = [
                    ("Phase 5 exchanges your half and CA certificate with theirs through the peering directory (saltlick)", None, "It uploads yours, fetches theirs, checks both, and prints both keys and CA fingerprints and a compatibility report. It grants no access. If their entry is not there yet, run phase 5 again after they run theirs."),
                    ("Read your key and CA fingerprint aloud on one call", None, "Each side reads the other's back. A key that arrives inside their package proves nothing."),
                    ("Save both keys here, one per line, as sovereign_id=key", Some(&keys), ""),
                ];
                for (i, (title, file, note)) in steps.iter().enumerate() {
                    s.push('\n');
                    s.push_str(&crate::output::hand_step(i + 1, title, *file, note, color));
                }
            }
            // The script's words changed: show them rather than guess.
            _ => {
                for line in ph.cmd.lines() {
                    s.push_str(&crate::tutor::fill(&short(line.trim()), "  ", "    "));
                }
            }
        }
    }
    if shown.iter().any(|p| p.n == 1) {
        s.push_str(&format!("\n{}\n", bold("Phase 1 checks these before anything changes:")));
        for (id, what, _) in preflight::CHECKS {
            s.push_str(&format!("  {id:<15} {what}\n"));
        }
    }
    s.push_str(&format!("\n{} add -o json to see every phase's command, or read\ndocs/operations/sovereign-federation-runbook.md.\n", bold("To see more:")));
    match shown.iter().find(|p| p.n == 11) {
        Some(_) if !agreement_known => {
            s.push_str("Phase 11 depends on what phase 6 agrees: wireguard and ipsec-fips each have\na play. Any other choice stops the run there.\n");
        }
        Some(ph11) if ph11.label.contains("REFUSED") => {
            s.push_str(&format!("{} the agreement chose a transport with no play. Only wireguard and\nipsec-fips have one, so a real run stops at phase 11.\n", bold("Refused:")));
        }
        _ => {}
    }
    // The same run again: everything this plan was aimed with, or a resume aims
    // the later phases at the defaults.
    let again = |from: u32| {
        let mut c = format!("goatbuilder federate {} --from {from}", o.with);
        for (flag, value) in [("--env", o.env), ("--as", o.r#as), ("--edge-group", o.edge_group), ("--work", o.work)] {
            if let Some(v) = value {
                c.push_str(&format!(" {flag} {v}"));
            }
        }
        c
    };
    // A long path takes its own line, so the command still runs as printed.
    let laid = |c: String| {
        let w = crate::tutor::shell_wrap(&c, "    ");
        match o.work.filter(|v| v.len() > 40) {
            Some(v) => w.replace(&format!("--work {v}"), &format!("--work \\\n        {v}")),
            None => w,
        }
    };
    let manual = shown.iter().find(|p| p.kind == "MANUAL").map(|p| p.n);
    match manual {
        Some(n) => s.push_str(&format!("\n{} remove --dry-run. The run stops at phase {n} for you. When you\nfinish those steps, carry on with:\n\n{}\n", bold("To run it for real:"), bold(laid(again(n + 1)).trim_end()))),
        None => s.push_str(&format!("\n{}\n\n{}\n", bold("To run it for real:"), bold(laid(again(o.from)).trim_end()))),
    }
    s
}

/// A run as it has always been: recorded in the journal, nothing skipped.
#[cfg_attr(not(test), allow(dead_code))]
pub fn federate(repo: &Path, o: &Options) -> Result<Report> {
    federate_with(repo, o, &mut Surface::default())
}

/// A run with what the caller asked for beyond the phases (run.rs). A real
/// run holds the peering's lock and is recorded in `<work>/state.json`.
pub fn federate_with(repo: &Path, o: &Options, s: &mut Surface) -> Result<Report> {
    // `goatbuilder federate allocate`: the per-peering values and files (a reserved word, never a partner).
    if o.with == "allocate" {
        return peering_files::run(repo, o);
    }
    // An unknown FED_VERBS is refused, as the script refuses it, so a typo
    // cannot pick a different path on each avenue.
    let fed_verbs = env_or("FED_VERBS", String::new());
    if !matches!(fed_verbs.as_str(), "" | "goat" | "goatbuilder") {
        return Err(anyhow!("FED_VERBS is goat or goatbuilder, not '{fed_verbs}'"));
    }
    // `--mock <dir>` reads the flat `<dir>/goatbuilder.yaml`, the same fixture
    // shape standup opens; without it, the repository's own config.
    let cfg_path = match o.mock {
        Some(m) => m.join("goatbuilder.yaml"),
        None => repo.join("ops/goatbuilder/goatbuilder.yaml"),
    };
    let cfg = config::load(&cfg_path)?;
    measure::check_driver_measure()?;
    if let Some(env) = o.env {
        if !cfg.environments.iter().any(|e| e.name == env) {
            return Err(anyhow!("unknown environment \"{env}\"; declared: {:?}", cfg.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()));
        }
    }
    // C1: an empty list declares nothing, so it refuses every name. It used to pass
    // every name, which made the check vacuous on the one config that declares no
    // peering: the repository's own.
    if !cfg.peerings.iter().any(|p| p.with == o.with && o.env.map(|e| p.environment == e).unwrap_or(true)) {
        let declared = cfg.peerings.iter().map(|p| format!("{} on {}", p.with, p.environment)).collect::<Vec<_>>().join(", ");
        return Err(anyhow!("peering '{}' is not declared in {} peerings: (declared: {}) — declare it first (C1)", o.with, cfg_path.display(), if declared.is_empty() { "none" } else { &declared }));
    }
    // THE PROFILE DIRECTORY, resolved once for this run, so preflight and every
    // `goat` call below agree (a run under sudo reads the invoking user's HOME).
    // Kept in the run's environment (runenv), never the process's.
    if runenv::var("GOAT_PROFILE_DIR").map(|v| v.is_empty()).unwrap_or(true) {
        let home = runenv::var("HOME").unwrap_or_default();
        let dir = if is_root() { "/root/.goat".to_string() } else { format!("{home}/.goat") };
        runenv::set("GOAT_PROFILE_DIR", dir);
    }
    // A peering's offer: may narrow federation.toml's release sets, never widen them.
    if o.mock.is_none() {
        declaration::driver_check(&cfg, o.with, o.env, repo)?;
    }
    let work = work_dir(o);
    // Phases 7, 8, 9 and 14 always name our party: without --as, our
    // sovereign id from the declaration, as the help says, else from our
    // half. Mint and acceptance otherwise take the agreement's first party,
    // which is us only when we sort first. A mock plans the same commands
    // the script avenue prints, so it reads them too.
    let our = if o.r#as.is_none() { our_id(&work) } else { None };
    let with_us;
    let o = if our.is_some() {
        with_us = Options { r#as: our.as_deref(), ..*o };
        &with_us
    } else {
        o
    };
    let edge_group = o.edge_group.map(String::from).unwrap_or_else(|| format!("{}_edge", o.with));
    let resume = format!("goatbuilder federate {}{}", o.with, o.env.map(|e| format!(" --env {e}")).unwrap_or_default());
    // Once `goatbuilder federate allocate` has recorded this peering, the run must use its values.
    peering_files::driver_agrees(&cfg, cfg_path.parent().unwrap_or(repo), o, &work, &edge_group)?;
    let executes = !o.dry_run && o.mock.is_none();
    // A deploy takes the peering's lock and runs the planner's live recheck
    // first, before the phase-1 inputs are exported below.
    let mut prelude = None;
    if executes {
        if let Some(plan) = &s.plan {
            prelude = Some(run::Prelude::deploy(o.with, &work, plan, o.from, o.to)?);
        }
    }
    // Phase 1's inputs, derived from the deployment (federate/preflight.rs). A
    // real run exports each one the operator has not set, before the plan reads
    // them; a dry run lists them and changes no command (the parity gate).
    let peering = cfg.peerings.iter().find(|p| p.with == o.with && o.env.map(|e| p.environment == e).unwrap_or(true)).expect("C1 found it");
    // With what goatbuilder.yaml itself declares for the run merged in (federate/declared.rs).
    let inputs = preflight::derive_inputs(repo, &cfg, peering, &work, &edge_group, None);
    if executes {
        preflight::export(&inputs);
        // Phase 14 reads our core and edge declarations by group
        // (acceptance_surfaces.py): the edge group this run uses, whatever the
        // environment held, so --edge-group is never typed a second time.
        runenv::set("FED_EDGE_GROUP", &edge_group);
    }
    let mut phases = plan(repo, o, &work, &edge_group, &resume);
    // A peering a forwarder relays: the forwarder runs inside phase 2 (federate/forwarder.rs).
    forwarder::splice(&mut phases, &cfg, cfg_path.parent().unwrap_or(repo), o, &edge_group)?;
    if s.rotate_leaves {
        rotation::leaves(&mut phases);
    }
    let banner_mode = if o.dry_run {
        "(DRY RUN — nothing executes)"
    } else if o.mock.is_some() {
        "(MOCK — nothing executes)"
    } else {
        ""
    };
    let mut checks = vec![row("banner", format!("federate: peering={}  work={work}  edge-group={edge_group}  {banner_mode}", o.with))];
    // A plan or a mock runs nothing, and that includes making its working
    // directory -- on a real operator's machine this is /var/lib/goat-federation.
    if executes {
        let _ = std::fs::create_dir_all(format!("{work}/pki"));
    }
    let mut run = None;
    if executes {
        let (r, rows) = run::Run::begin(o.with, &work, &phases, o.from, o.to, s, prelude.take())?;
        checks.extend(rows);
        run = Some(r);
    }
    let mut exit = EXIT_OK;
    let mut stopped = false;
    // Phase 5 confirmed in this run writes the keyring phase 6 reads: the
    // phases after it are planned again from what is there now.
    let mut replanned: Option<Vec<Phase>> = None;
    for planned in &phases {
        let ph = replanned.as_ref().and_then(|v| v.iter().find(|p| p.n == planned.n)).unwrap_or(planned);
        if !s.runs(ph.n, o.from, o.to) {
            continue;
        }
        if let Some(r) = run.as_mut().filter(|r| r.skips(ph)) {
            checks.push(r.skipped(ph));
            continue;
        }
        checks.push(row("phase", format!("=== Phase {} [{}] {} ===", ph.n, ph.kind, ph.label)));
        checks.push(row("cmd", ph.cmd.clone()));
        if o.dry_run {
            continue;
        }
        // The commands that follow are this phase's: the runner frames and
        // records them under it (crates/goatbuilder/src/runner.rs).
        crate::runner::phase(&ph.n.to_string(), &ph.label, purpose(ph.n));
        // A mock has no hosts. Recording the phase is the whole of what it does.
        if !executes {
            checks.push(row("mock", format!("phase {} recorded against the fixture; not executed", ph.n)));
            continue;
        }
        if let Some(r) = run.as_mut() {
            r.start(ph);
            if let Some(c) = r.guard(ph) {
                let d = c.detail.clone();
                checks.push(c);
                r.end(ph, state::Outcome::Failed, &d);
                exit = EXIT_REFUSED;
                break;
            }
        }
        if ph.kind == "MANUAL" {
            // The directory step, inside phase 5 before the stop. It is
            // advisory: its exit status is never read.
            let phase_started = std::time::SystemTime::now();
            if ph.n == 5 {
                if let Some(args) = directory_args(repo, &work) {
                    let _ = runenv::command("python3").args(&args).current_dir(repo).status();
                    checks.push(row("directory", "the directory step ran; it is advisory and its result changes no phase".into()));
                }
            }
            // Confirmation 1, when asked for: a package the directory held
            // back is placed only once a person confirms it is ours.
            if let (5, Some(r), true) = (ph.n, run.as_mut(), s.values.active()) {
                // The directory step again, its report on stderr (stdout
                // carries -o json); success is judged by what it placed.
                let rerun = || directory_args(repo, &work).map(|a| runenv::command("python3").args(&a).env("FED_DIRECTORY_PLACE_POINTER_PACKAGE", "1").current_dir(repo).stdout(std::process::Stdio::from(std::io::stderr())).status().is_ok()).unwrap_or(false);
                if let Some(step) = confirm::pointer(&r.paths, s, &mut r.state, &r.id.clone(), Some(phase_started), &rerun) {
                    checks.extend(step.rows);
                    match step.verdict {
                        confirm::Verdict::Continue => {}
                        confirm::Verdict::Stop(d) => {
                            r.end(ph, state::Outcome::Stopped, &d);
                            stopped = true;
                            break;
                        }
                        confirm::Verdict::Fail(d) => {
                            r.end(ph, state::Outcome::Failed, &d);
                            exit = EXIT_REFUSED;
                            break;
                        }
                    }
                }
            }
            // Confirmation 1, when asked for: the command binds the keys
            // and carries on instead of stopping.
            if let (5, Some(r), true) = (ph.n, run.as_mut(), s.values.active()) {
                let rerun = || directory_args(repo, &work).map(|a| runenv::command("python3").args(&a).current_dir(repo).stdout(std::process::Stdio::from(std::io::stderr())).status().is_ok()).unwrap_or(false);
                let step = confirm::exchange(&r.paths, s, &mut r.state, &r.id.clone(), &rerun);
                checks.extend(step.rows);
                match step.verdict {
                    confirm::Verdict::Continue => {
                        r.end(ph, state::Outcome::Passed, "keys confirmed");
                        replanned = Some(plan(repo, o, &work, &edge_group, &resume));
                        if s.rotate_leaves {
                            if let Some(p) = replanned.as_mut() {
                                rotation::leaves(p);
                            }
                        }
                        continue;
                    }
                    confirm::Verdict::Stop(d) => {
                        r.end(ph, state::Outcome::Stopped, &d);
                        stopped = true;
                        break;
                    }
                    confirm::Verdict::Fail(d) => {
                        r.end(ph, state::Outcome::Failed, &d);
                        exit = EXIT_REFUSED;
                        break;
                    }
                }
            }
            // Confirmation 2, when asked for: the command admits the edge on
            // our core after the person approves what it admits.
            if let (10, Some(r), true) = (ph.n, run.as_mut(), admit::active(s)) {
                let env_name = o.env.map(String::from).or_else(|| cfg.peerings.iter().find(|p| p.with == o.with).map(|p| p.environment.clone())).unwrap_or_default();
                let step = admit::admit(repo, &r.paths, o.with, &env_name, &edge_group, s, &mut r.state, &r.id.clone());
                checks.extend(step.rows);
                match step.verdict {
                    confirm::Verdict::Continue => {
                        r.end(ph, state::Outcome::Passed, "admitted by the command");
                        continue;
                    }
                    confirm::Verdict::Stop(d) => {
                        r.end(ph, state::Outcome::Stopped, &d);
                        stopped = true;
                        break;
                    }
                    confirm::Verdict::Fail(d) => {
                        r.end(ph, state::Outcome::Failed, &d);
                        exit = EXIT_REFUSED;
                        break;
                    }
                }
            }
            // Phase 10 of a plain run: admitted by the command through an
            // approved plan. With a person at the terminal the run hands over
            // to plan, deploy (which asks to approve the plan, then what our
            // core admits) and test; without one, the stop names them.
            if ph.n == 10 && s.plan.is_none() && s.only.is_none() {
                if s.hand_off_at_10 {
                    checks.push(row("note", format!("phase 10 by the command: making the plan for {}, then deploy asks to approve it and what our core admits, and runs 10 to 13; then acceptance", o.with)));
                    if let Some(r) = run.as_mut() {
                        r.end(ph, state::Outcome::Stopped, "handed to plan, deploy and test");
                    }
                    s.handed_off = true;
                    stopped = true;
                    break;
                }
                checks.push(row("note", format!("or by the command: goatbuilder federate plan {w}, then goatbuilder federate deploy {w} (it asks to approve the plan and what our core admits, at a terminal), then goatbuilder federate test {w}", w = o.with)));
            }
            checks.push(row("note", format!("^ OPERATOR STEP — do the above, then re-run with --from {}", ph.n + 1)));
            if let Some(again) = &s.again {
                checks.push(row("note", format!("the command that carries on: {again} --from {}", ph.n + 1)));
            }
            if let Some(r) = run.as_mut() {
                r.end(ph, state::Outcome::Stopped, "waiting for the operator step");
            }
            stopped = true;
            break;
        }
        let rc = match &ph.verb {
            // A federation verb, in process: the same words the plan printed.
            Some((env, words)) => run_verb_in_process(repo, env, words),
            None => {
                // -e, like standup and join-node: a phase that is several statements
                // stops at the first failure instead of running on past it.
                let status = runenv::command("bash").args(["-euo", "pipefail", "-c", &ph.cmd]).current_dir(repo).status().context("bash")?;
                if status.success() { 0 } else { status.code().unwrap_or(1) }
            }
        };
        if rc != 0 {
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the peering is left partway through phase {}. `goat federate` exit codes are specific: 20 composition refused, 21 lint refused, 22 diff refused, 23 a value could not be derived. Phase 14 (acceptance) exits 3 when a required check was BLOCKED or skipped: nothing was proven by it (X-ACCEPT-001); exit 2 when the agreement or the evidence bundle could not be read or written. docs/operations/sovereign-federation-runbook.md names this phase's \"worked when\"; Steps 7 and 8 are the acceptance criteria.", ph.n, ph.label, ph.n), fix: Some(match &s.again {
                Some(again) => format!("fix the cause, then run the stage again (its checks and plays all run again): {again}"),
                None => format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}", ph.n),
            }) });
            if let Some(r) = run.as_mut() {
                r.end(ph, state::Outcome::Failed, &format!("GB-183: phase {} failed with exit {rc}", ph.n));
            }
            exit = EXIT_REFUSED;
            break;
        }
        if let Some(r) = run.as_mut() {
            // Confirmation 1, second half: the agreement digest after compose.
            if ph.n == 6 && s.values.active() {
                let step = confirm::agreement(&r.paths, s, &mut r.state, &r.id.clone());
                checks.extend(step.rows);
                match step.verdict {
                    confirm::Verdict::Continue => {}
                    confirm::Verdict::Stop(d) => {
                        r.end(ph, state::Outcome::Stopped, &d);
                        stopped = true;
                        break;
                    }
                    confirm::Verdict::Fail(d) => {
                        r.end(ph, state::Outcome::Failed, &d);
                        exit = EXIT_REFUSED;
                        break;
                    }
                }
            }
            r.end(ph, state::Outcome::Passed, "");
            rotation::after_phase(s, ph.n);
        }
    }
    if let Some(r) = run.as_mut() {
        checks.extend(r.finish(if exit != EXIT_OK { "failed" } else if stopped { "stopped" } else { "passed" }));
    }
    let printed = o.dry_run && o.table;
    if printed {
        // The plan is the human answer; the rows still go to -o json and the
        // tutor's log, but are not printed again as a table.
        print!("{}", plan_view(repo, o, &work, &edge_group, &phases));
    }
    if o.dry_run {
        checks.extend(preflight::plan_rows(&inputs));
        checks.push(row("footer", "PLAN ONLY — nothing above executed. Phases 5 and 10 are MANUAL and stop the run; resume with --from 6 and --from 11. Acceptance criteria for every phase: docs/operations/sovereign-federation-runbook.md. The shape and why: https://github.com/usareur-af/goatdocs/blob/main/docs/design/staged-delivery-pattern.md".into()));
    }
    let mut r = Report::new("federate", checks, EXIT_REFUSED);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    r.printed = printed;
    Ok(r)
}

/// One federation verb in process, run in `dir` (the repository, where the
/// shell phases run, so a relative path means the same thing either way) with
/// `env` set for its duration (phase 3's DOCKER_HOST). Returns its exit code:
/// 0, a verb's own 20 to 24, 2 for a refused command line, a coded refusal's
/// catalog code (GB-100: 2), 101 for a panic, or 1 for any other fault, which
/// is printed.
/// The names the federation library reads from the process environment itself
/// (goat-federation's derive.rs, during an in-process `init`); every other value
/// an in-process verb reads, it reads through [`runenv`].
const READ_FROM_THE_PROCESS_BY_THE_LIBRARY: [&str; 2] = ["DOCKER_HOST", "FED_ROUTER_CONTAINER"];

/// Of a verb's values, the ones that go into the process environment for its
/// duration: only those the library reads from the process itself.
fn for_the_process(all: &[(String, String)]) -> Vec<&(String, String)> {
    all.iter().filter(|(k, _)| READ_FROM_THE_PROCESS_BY_THE_LIBRARY.contains(&k.as_str())).collect()
}

fn run_verb_in_process(dir: &Path, env: &[(String, String)], words: &[String]) -> i32 {
    // The verb runs in this process. goatbuilder's verbs read their values
    // through runenv, which is this thread's, so the verb's own values go there
    // for its duration and are put back after. Only the few names the library
    // reads from the process environment itself are put there, and restored:
    // set_var changes process-global state, so this stays the one place it
    // happens, for a real run's in-process verb, one at a time. So does
    // set_current_dir. A driver that runs verbs concurrently must pass both to
    // the verb instead.
    let all: Vec<(String, String)> = runenv::pairs().into_iter().chain(env.iter().cloned()).collect();
    let to_process = for_the_process(&all);
    let saved: Vec<(String, Option<std::ffi::OsString>)> = to_process.iter().map(|(k, _)| (k.clone(), std::env::var_os(k))).collect();
    for (k, v) in &to_process {
        std::env::set_var(k, v);
    }
    let saved_run: Vec<(String, Option<String>)> = env.iter().map(|(k, _)| (k.clone(), runenv::recorded(k))).collect();
    for (k, v) in env {
        runenv::set(k, v.clone());
    }
    let here = std::env::current_dir().ok();
    let rc = guarded(|| std::env::set_current_dir(dir).map_err(anyhow::Error::from).and_then(|_| verbs::run_argv(words)));
    if let Some(h) = here {
        let _ = std::env::set_current_dir(h);
    }
    for (k, v) in saved_run {
        match v {
            Some(v) => runenv::set(&k, v),
            None => runenv::unset(&k),
        }
    }
    for (k, v) in saved {
        match v {
            Some(v) => std::env::set_var(&k, v),
            None => std::env::remove_var(&k),
        }
    }
    rc
}

/// Run an in-process verb so that whatever goes wrong becomes an exit code,
/// and the caller's failure row, resume hint, working directory and
/// environment are kept: a fault is printed as `goatbuilder federate <verb>`
/// prints it (a refused command line 2, GB-100 its catalog code, else 1), and
/// a panic exits 101, as a panicking `goat` process would.
fn guarded(f: impl FnOnce() -> Result<i32>) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => verbs::fault_exit(&e),
        Err(_) => {
            eprintln!("goatbuilder federate: the verb panicked (above); nothing after it ran");
            101
        }
    }
}

/// The working directory: `--work`, else `$FED_WORK`, else
/// /var/lib/goat-federation/<peering>.
pub fn work_dir(o: &Options) -> String {
    o.work.map(String::from).unwrap_or_else(|| env_or("FED_WORK", format!("/var/lib/goat-federation/{}", o.with)))
}

fn is_root() -> bool {
    runenv::command("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_package_per_counterparty_is_the_default_and_a_set_variable_is_the_directory_steps() {
        assert!(per_counterparty(None));
        for v in ["0", "", "no", "1", "yes", "on"] {
            assert!(!per_counterparty(Some(v.into())), "{v:?}");
        }
    }

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A peering whose inventory `allocate` rendered into its own file (not
    /// every peering declares hosts:) has every play read that file beside the
    /// hand-written one; without the file the commands are as before.
    #[test]
    fn the_plays_read_the_peerings_own_inventory_file_when_there_is_one() {
        let r = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "xab", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let before = plan(r.path(), &o, &work_s, "xab_edge", "resume");
        assert!(before.iter().all(|p| !p.cmd.contains("peering-xab_edge.ini")));
        let inv = r.path().join("ops/ansible/inventory");
        std::fs::create_dir_all(&inv).unwrap();
        std::fs::write(inv.join("peering-xab_edge.ini"), "[xab_edge]\n").unwrap();
        let after = plan(r.path(), &o, &work_s, "xab_edge", "resume");
        let plays: Vec<&Phase> = after.iter().filter(|p| p.cmd.contains("ansible-preflight.sh")).collect();
        assert!(!plays.is_empty());
        for p in plays {
            assert!(p.cmd.contains("-i inventory/peering-edges.ini -i inventory/peering-xab_edge.ini "), "{}", p.cmd);
        }
    }

    /// A value from the agreement or the keyring never runs as shell code: the
    /// phases run through `bash -c`, and keys.txt holds a counterparty's value.
    /// An unsafe transport reads as unknown; an unsafe key line is left out, so
    /// compose refuses for the unbound party (rule 6).
    #[test]
    fn an_agreement_value_never_reaches_the_shell() {
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        std::fs::write(work.path().join("agreement.json"), r#"{"transport":{"kind":"$(touch /tmp/gb-pwned)"}}"#).unwrap();
        std::fs::write(work.path().join("keys.txt"), "alpha=QUJD\nbeta='; touch /tmp/gb-pwned; '\n").unwrap();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let phases = plan(&repo(), &o, &work_s, "velmara_edge", "resume");
        let by = |n: u32| phases.iter().find(|p| p.n == n).unwrap();
        assert!(!by(11).cmd.contains("$(") && !by(11).cmd.contains("gb-pwned"), "{}", by(11).cmd);
        assert!(by(11).label.contains("'unknown'"), "{}", by(11).label);
        assert!(by(6).cmd.contains("--key 'alpha=QUJD'"), "a good key still passes: {}", by(6).cmd);
        assert!(!by(6).cmd.contains("gb-pwned"), "{}", by(6).cmd);
        assert!(shell_safe("ipsec-fips") && shell_safe("wireguard") && shell_safe("alpha=QUJD+/=="));
        assert!(!shell_safe("") && !shell_safe("a b") && !shell_safe("a'b") && !shell_safe("$(x)") && !shell_safe("`x`"));
    }

    /// `rotate <partner> leaves` (docs/design/federation-key-rotation.md §5,
    /// K3): phase 8 replaces every leaf with --rotate, and only phase 8's
    /// command changes; the run is 8 to 14.
    #[test]
    fn rotating_the_leaves_asks_phase_8_to_replace_them_and_nothing_else() {
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let before = plan(&repo(), &o, &work_s, "velmara_edge", "resume");
        let mut after = plan(&repo(), &o, &work_s, "velmara_edge", "resume");
        rotation::leaves(&mut after);
        for (a, b) in before.iter().zip(after.iter()) {
            if a.n == 8 {
                assert_eq!(b.cmd, format!("{} --rotate", a.cmd));
            } else {
                assert_eq!(a.cmd, b.cmd, "phase {}", a.n);
            }
        }
        let (o2, s) = rotation::leaves_run(&o);
        assert_eq!((o2.from, o2.to), (8, 14));
        assert!(s.rotate_leaves && !s.resume);
        assert_eq!(s.stage, "rotate");
        assert_eq!(s.again.as_deref(), Some("goatbuilder federate rotate velmara leaves"));
        // The carry-on after the phase-10 stop names --from 11: that start
        // reaches the run, and the leaves are not minted again.
        let (o3, s3) = rotation::leaves_run(&Options { from: 11, ..o });
        assert_eq!((o3.from, o3.to), (11, 14));
        assert!(!s3.rotate_leaves, "phase 8 is not in the run, so nothing is re-minted");
        assert!(s3.again.is_none(), "its stop and failure hints offer the plain resume, never rotate again");
        // Once phase 8 has passed, the command that carries on is the plain
        // resume, so following a stop or a failure never mints another set.
        let mut s4 = rotation::leaves_run(&o).1;
        rotation::after_phase(&mut s4, 7);
        assert!(s4.again.is_some());
        rotation::after_phase(&mut s4, 8);
        assert!(s4.again.is_none() && !s4.rotate_leaves);
    }

    /// Conformance check (d), for federate: every phase the plan can hold, for
    /// each transport an agreement may select (or none), has its entry in the
    /// phase catalog (ops/phases/catalog.yaml), with every field `explain`
    /// answers from.
    #[test]
    fn every_federate_phase_has_a_phase_catalog_entry() {
        for transport in [None, Some("wireguard"), Some("ipsec-fips"), Some("quic")] {
            let work = tempfile::tempdir().unwrap();
            if let Some(t) = transport {
                std::fs::write(work.path().join("agreement.json"), format!(r#"{{"transport":{{"kind":"{t}"}}}}"#)).unwrap();
            }
            let work_s = work.path().display().to_string();
            let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
            let phases = plan(&repo(), &o, &work_s, "velmara_edge", "resume");
            assert_eq!(phases.len(), 14, "{transport:?}");
            for ph in &phases {
                let e = goat_errors::phases::explain_phase("federate", &ph.n.to_string())
                    .unwrap_or_else(|| panic!("federate phase {} ({}) has no phase-catalog entry", ph.n, ph.label));
                assert!(!e.what.is_empty() && !e.why.is_empty() && !e.by_hand.is_empty(), "federate phase {}", ph.n);
            }
        }
    }

    /// The catalog's `by_hand` is the command the plan runs: for every phase
    /// the plan runs a command for (not the MANUAL stops, whose `by_hand` is
    /// what the person does), the entry's `by_hand` with its placeholders
    /// filled from the plan's own values, `[...]` an optional part, equals the
    /// phase's command once the repository prefix and the quoting are taken
    /// out. Someone who copies it reaches the hosts and credentials the tool
    /// would.
    #[test]
    fn the_catalog_by_hand_is_the_command_the_plan_runs() {
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("agreement.json"), r#"{"transport":{"kind":"wireguard"}}"#).unwrap();
        std::fs::write(work.path().join("keys.txt"), "xaa=QUJD\nvelmara=QUJE\n").unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: Some("xaa"), edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        // The checkout as it is, and one whose peering has its own inventory
        // file (allocate renders inventory/peering-<edge-group>.ini), which the
        // plays then read beside the hand-written one.
        let own = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(own.path().join("ops/ansible/inventory")).unwrap();
        std::fs::write(own.path().join("ops/ansible/inventory/peering-velmara_edge.ini"), "").unwrap();
        for r in [repo(), own.path().to_path_buf()] {
        let phases = plan(&r, &o, &work_s, "velmara_edge", "resume");
        let root = format!("{}/", r.display());
        let norm = |s: &str| -> String {
            s.replace(&root, "").replace('\'', "").split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let fill = |s: &str| -> String {
            s.replace("<work>", &work_s)
                .replace("<peering>", "velmara")
                .replace("<edge-group>", "velmara_edge")
                .replace("<core-group>", "velmara_core")
                .replace("<counterparty-file>", &format!("{work_s}/theirs.canonical.json"))
                .replace("<us>", "xaa")
                .replace("<profile>", &crate::settings::aws_profile(&r))
                .replace("<repository>", &r.display().to_string())
                .replace("<keys>", "--key xaa=QUJD --key velmara=QUJE")
                .replace("<declaration>", "")
                .replace("<router>", "")
        };
        // `[...]` is optional: every combination of each part kept or left out.
        fn readings(s: &str) -> Vec<String> {
            let Some(i) = s.find('[') else { return vec![s.to_string()] };
            let Some(j) = s[i..].find(']').map(|j| i + j) else { return vec![s.to_string()] };
            let (head, part, tail) = (&s[..i], &s[i + 1..j], &s[j + 1..]);
            readings(tail)
                .into_iter()
                .flat_map(|t| [format!("{head}{part}{t}"), format!("{head}{t}")])
                .collect()
        }
        let mut wrong = Vec::new();
        for ph in phases.iter().filter(|p| p.kind != "MANUAL") {
            let e = goat_errors::phases::explain_phase("federate", &ph.n.to_string()).unwrap();
            let want = norm(&ph.cmd);
            if !readings(&fill(&e.by_hand)).iter().any(|b| norm(b) == want) {
                wrong.push(format!("phase {}:\n    by_hand: {}\n    the plan: {want}", ph.n, norm(&fill(&e.by_hand))));
            }
        }
        assert!(wrong.is_empty(), "by_hand is not the command the plan runs ({}):\n  {}", r.display(), wrong.join("\n  "));
        }
    }

    /// A dry run reads as a plan: one aligned phase table, then the commands;
    /// no `=== Phase ===` banners, no em dashes, no `a/b/../../c` paths, and
    /// phase 11 says it waits on the agreement rather than "REFUSED".
    /// A federate run on one thread changes nothing another thread reads:
    /// the profile directory, the edge group and every derived input reach the
    /// run's own reads and its children through runenv, never the process
    /// environment. Tests run in parallel; the plan tests read FED_* and
    /// GOAT_* while other tests run federate.
    #[test]
    fn a_run_on_another_thread_leaves_this_threads_environment_alone() {
        let names = ["GOAT_PROFILE_DIR", "FED_EDGE_GROUP", "FED_GB_ENV_PROBE"];
        let before: Vec<_> = names.iter().map(|n| runenv::var_os(n)).collect();
        let seen = std::thread::spawn(|| {
            let work = tempfile::tempdir().unwrap();
            let work_s = work.path().display().to_string();
            let fx = fixture();
            let o = Options { with: "hello-ally", env: None, r#as: Some("zagros"), edge_group: None, work: Some(&work_s), mock: Some(&fx), from: 1, to: 1, dry_run: true, table: false };
            let _ = federate(&repo(), &o);
            preflight::export(&[preflight::Input { name: "FED_GB_ENV_PROBE", value: Some("x".into()), source: "test".into() }]);
            // The run that exported it reads it.
            runenv::var("FED_GB_ENV_PROBE").ok()
        })
        .join()
        .unwrap();
        assert_eq!(seen.as_deref(), Some("x"), "the exporting run reads its own value");
        let after: Vec<_> = names.iter().map(|n| runenv::var_os(n)).collect();
        assert_eq!(before, after, "the process environment is unchanged");
        assert!(runenv::var("FED_GB_ENV_PROBE").is_err(), "and this thread does not see the other run's value");
    }

    #[test]
    fn a_dry_run_prints_a_plan_table() {
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: true };
        let phases = plan(&repo(), &o, &work_s, "velmara_edge", "goatbuilder federate velmara");
        let view = plan_view(&repo(), &o, &work_s, "velmara_edge", &phases);
        assert!(view.contains("PHASE  TYPE     STEP"), "{view}");
        assert!(!view.contains("==="), "{view}");
        assert!(!view.contains('\u{2014}'), "{view}");
        assert!(view.contains("peering directory (saltlick)") && !view.contains("/../"), "{view}");
        assert!(view.contains("depends on what phase 6 agrees") && !view.contains("REFUSED"), "{view}");
        assert!(view.contains("Nothing has run. This only shows the steps"), "{view}");
        for gone in ["counterparty", "out of band", "no perspective", "Mint", "fail-closed", "sovereign edge", "ansible-preflight", "Only ipsec-fips"] {
            assert!(!view.contains(gone), "{gone}: {view}");
        }
        assert!(view.contains("1. Phase 5 exchanges your half") && view.contains("Read your key and CA fingerprint aloud on one call"), "{view}");
        assert!(!view.contains('\u{2717}'), "a plan owes nothing, so it marks nothing: {view}");
        // Every line fits 80 columns, but a line that names the working folder:
        // that is the test's temporary path, as long as the machine's TMPDIR.
        assert!(view.lines().filter(|l| !l.trim_start().starts_with("Working folder:")).all(|l| l.chars().count() <= 80 || !l.trim().contains(' ')), "{view}");
        let is_row = |l: &&str| l.split_whitespace().next().is_some_and(|w| w.parse::<u32>().is_ok());
        assert_eq!(view.lines().filter(is_row).count(), 14, "one table row per phase:\n{view}");
    }

    /// An agreement that chose a transport with no play: the page says refused,
    /// and names the two transports that have one.
    #[test]
    fn a_plan_with_a_refused_transport_says_so_on_the_page() {
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("agreement.json"), r#"{"transport":{"kind":"quic"}}"#).unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let phases = plan(&repo(), &o, &work_s, "velmara_edge", "goatbuilder federate velmara");
        let view = plan_view(&repo(), &o, &work_s, "velmara_edge", &phases);
        assert!(view.contains("refused, the agreement chose 'quic'"), "{view}");
        assert!(view.contains("a real run stops at phase 11"), "{view}");
        assert!(!view.contains("Open the connection the agreement chooses"), "{view}");
    }

    /// A wireguard agreement is not refused on the page.
    #[test]
    fn a_plan_with_a_wireguard_agreement_opens_it_on_the_page() {
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("agreement.json"), r#"{"transport":{"kind":"wireguard"}}"#).unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let phases = plan(&repo(), &o, &work_s, "velmara_edge", "goatbuilder federate velmara");
        let view = plan_view(&repo(), &o, &work_s, "velmara_edge", &phases);
        assert!(view.contains("Open the connection the agreement chooses") && !view.contains("Refused"), "{view}");
    }

    /// FED_MEASURE=ssh: phase 3 runs `goatbuilder federate init --over-ssh` in
    /// process with no DOCKER_HOST, whoever runs the other verbs; its words
    /// parse as the verb line, so the plan runs what it prints. Every other
    /// phase is the plan it was.
    #[test]
    fn phase_3_measures_over_ssh_when_chosen_and_nothing_else_moves() {
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        let o = Options { with: "velmara", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        for in_process in [false, true] {
            let here = plan_with(&repo(), &o, &work_s, "edge-x", "resume", in_process, false);
            let ssh = plan_with(&repo(), &o, &work_s, "edge-x", "resume", in_process, true);
            let p3 = ssh.iter().find(|p| p.n == 3).unwrap();
            let (env, words) = p3.verb.as_ref().expect("phase 3 runs in process");
            assert!(env.is_empty(), "no DOCKER_HOST over SSH: {env:?}");
            assert_eq!(p3.cmd, format!("goatbuilder federate {}", shell_words(words)));
            let joined = words.join(" ");
            assert!(joined.starts_with("init --with-transport --with-data"), "{joined}");
            assert!(joined.contains("--over-ssh velmara --ssh-edge-group edge-x"), "{joined}");
            assert!(joined.ends_with(&format!("--repo {}", repo().display())), "{joined}");
            for (a, b) in here.iter().zip(ssh.iter()).filter(|(a, _)| a.n != 3) {
                assert_eq!((a.n, &a.cmd), (b.n, &b.cmd));
            }
        }
    }

    /// FED_VERBS=goatbuilder: phases 3, 4, 6, 7 and 9 run in process, and the
    /// plan prints exactly the words that are parsed and run. The default
    /// spawns goat, as the script does.
    #[test]
    fn the_verb_phases_run_in_process_when_chosen_and_print_what_runs() {
        let work = tempfile::tempdir().unwrap();
        let work_s = work.path().display().to_string();
        std::fs::write(work.path().join("keys.txt"), "xaa=QUJD\n").unwrap();
        let o = Options { with: "velmara", env: None, r#as: Some("xaa"), edge_group: None, work: Some(&work_s), mock: None, from: 1, to: 99, dry_run: true, table: false };
        let verbs = [3, 4, 6, 7, 9];
        let inproc = plan_verbs(&repo(), &o, &work_s, "velmara_edge", "resume", true);
        for ph in &inproc {
            if verbs.contains(&ph.n) {
                let (env, words) = ph.verb.as_ref().unwrap_or_else(|| panic!("phase {} runs in process", ph.n));
                let prefix: String = env.iter().map(|(k, v)| format!("{k}='{v}' ")).collect();
                assert_eq!(ph.cmd, format!("{prefix}goatbuilder federate {}", shell_words(words)), "phase {}", ph.n);
            } else {
                assert!(ph.verb.is_none(), "phase {} is not a verb", ph.n);
            }
        }
        let by = |n: u32| inproc.iter().find(|p| p.n == n).unwrap();
        assert_eq!(by(4).verb.as_ref().unwrap().1, vec!["lint".to_string(), format!("{work_s}/ours.canonical.json")]);
        assert!(by(6).cmd.contains("--key 'xaa=QUJD'"), "{}", by(6).cmd);
        assert!(by(9).cmd.ends_with(&format!("--as 'xaa' --edge-group 'velmara_edge' --out '{work_s}/applied' --confirm")), "{}", by(9).cmd);
        let spawned = plan_verbs(&repo(), &o, &work_s, "velmara_edge", "resume", false);
        assert!(spawned.iter().all(|p| p.verb.is_none()), "the default runs every phase in a shell");
        assert!(spawned.iter().find(|p| p.n == 4).unwrap().cmd.contains(" federate lint '"), "goat federate lint");
        assert!(!spawned.iter().any(|p| p.cmd.contains("goatbuilder federate")));
    }

    /// The default runs the verbs in process; only FED_VERBS=goat spawns goat.
    #[test]
    fn the_verbs_run_in_process_unless_goat_is_chosen() {
        assert!(in_process_by(""), "unset is the default: in process");
        assert!(in_process_by("goatbuilder"));
        assert!(!in_process_by("goat"), "FED_VERBS=goat keeps the previous path");
    }

    /// A panic in an in-process verb exits 101 and a refused command line 2,
    /// so the driver writes its normal failure row instead of unwinding.
    #[test]
    fn an_in_process_panic_or_parse_error_becomes_an_exit_code() {
        assert_eq!(guarded(|| panic!("a verb panicked")), 101);
        assert_eq!(guarded(|| Ok(23)), 23);
        assert_eq!(guarded(|| verbs::run_argv(&["lint".into(), "--no-such-flag".into()])), 2);
        assert_eq!(guarded(|| Err(anyhow!("a fault"))), 1);
    }

    /// Each of federate's fourteen phases has a purpose, for the runner's frame.
    #[test]
    fn every_phase_has_a_purpose_for_the_frame() {
        for n in 1..=14 {
            assert!(!purpose(n).is_empty(), "phase {n}");
        }
        assert!(purpose(15).is_empty());
    }

    /// Of a verb's values, DOCKER_HOST and FED_ROUTER_CONTAINER reach the process
    /// environment during an in-process verb, and nothing else does.
    #[test]
    fn only_the_two_names_the_library_reads_go_to_the_process() {
        let all: Vec<(String, String)> = ["DOCKER_HOST", "FED_ROUTER_CONTAINER", "FED_WORK", "FED_KEYRING", "GOAT_PROFILE_DIR", "FED_ROUTER_DOCKER_HOST"]
            .iter()
            .map(|k| (k.to_string(), "v".to_string()))
            .collect();
        let names: Vec<&str> = for_the_process(&all).iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["DOCKER_HOST", "FED_ROUTER_CONTAINER"]);
        assert!(for_the_process(&[("FED_WORK".into(), "v".into())]).is_empty());
    }

    /// An in-process verb returns the verb's own exit code (21 for a malformed
    /// package), and the environment it set is put back afterwards.
    #[test]
    fn an_in_process_verb_exits_like_goat_and_restores_its_environment() {
        // Run in the directory the tests already run in: the process has one
        // working directory, and other tests read it concurrently.
        let here = std::env::current_dir().unwrap();
        let v = repo().join("ops/federation/vectors");
        let good = v.join("compose-full/package-xaa.canonical.json").display().to_string();
        let bad = v.join("malformed/no-signature.json").display().to_string();
        assert_eq!(run_verb_in_process(&here, &[], &["lint".into(), good]), 0);
        // The verb's own value is the run's, for the verb's duration: never the
        // process environment's, and gone afterwards.
        let key = "GOATBUILDER_TEST_VERB_ENV";
        assert_eq!(run_verb_in_process(&here, &[(key.into(), "x".into())], &["lint".into(), bad]), 21);
        assert!(runenv::var_os(key).is_none(), "the verb's environment leaked");
        assert!(std::env::var_os(key).is_none(), "the verb's environment reached the process");
        assert_eq!(run_verb_in_process(&here, &[], &["lint".into(), "/no/such/package".into()]), 1, "a fault exits 1, as in goat");
        // A relative path is relative to the directory the verb runs in (the
        // repository, in the driver), as it is for a shell phase.
        assert_eq!(run_verb_in_process(&here, &[], &["lint".into(), "../../ops/federation/vectors/compose-full/package-xaa.canonical.json".into()]), 0);
    }

    fn fixture() -> std::path::PathBuf {
        repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node")
    }

    /// The plan is the script's fourteen phases in order, with phase 11 refusing
    /// when no agreement names a transport, and nothing executed.
    #[test]
    fn dry_run_plans_the_scripts_phases_and_refuses_an_unknown_transport() {
        let held = crate::testdir::path("goatbuilder-federate-");
        let work = held.join("work");
        let work_s = work.display().to_string();
        let fx = fixture();
        let o = Options { with: "hello-ally", env: None, r#as: None, edge_group: None, work: Some(&work_s), mock: Some(&fx), from: 1, to: 99, dry_run: true, table: false };
        let r = federate(&repo(), &o).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        let phases: Vec<&str> = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert_eq!(phases.len(), 14, "{phases:?}");
        assert!(phases[0].starts_with("=== Phase 1 [GATE] peering preflight"));
        assert!(phases[1].starts_with("=== Phase 2 [A] prepare what the package measures"));
        assert!(phases[4].starts_with("=== Phase 5 [MANUAL] exchange through the directory"));
        assert!(phases[9].starts_with("=== Phase 10 [MANUAL] review what apply rendered"));
        assert!(phases[10].starts_with("=== Phase 11 [GATE] open the transport — REFUSED, no play for 'unknown'"), "{}", phases[10]);
        assert!(phases[12].starts_with("=== Phase 13 [A] federate the SPIRE trust domains"));
        assert!(phases[13].starts_with("=== Phase 14 [GATE] acceptance"));
        let cmds: Vec<&str> = r.checks.iter().filter(|c| c.name == "cmd").map(|c| c.detail.as_str()).collect();
        assert!(cmds[8].contains("--edge-group 'hello-ally_edge'"), "{}", cmds[8]);
        assert!(cmds[11].contains("peering-zenoh-edge.yml"));
        assert!(cmds[11].contains("--limit 'hello-ally_core'") && cmds[11].ends_with(" peering-republisher.yml"), "{}", cmds[11]);
        assert!(!work.exists(), "a plan creates nothing");
    }

    fn phase_11(kind: &str) -> (Vec<String>, String) {
        let work = crate::testdir::path(&format!("goatbuilder-federate-{kind}-"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("agreement.json"), format!(r#"{{"transport":{{"kind":"{kind}"}}}}"#)).unwrap();
        let work_s = work.display().to_string();
        let fx = fixture();
        let o = Options { with: "hello-ally", env: None, r#as: Some("zagros"), edge_group: Some("edge-x"), work: Some(&work_s), mock: Some(&fx), from: 11, to: 11, dry_run: true, table: false };
        let r = federate(&repo(), &o).unwrap();
        let phases = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.clone()).collect();
        let cmd = r.checks.iter().find(|c| c.name == "cmd").unwrap().detail.clone();
        let _ = std::fs::remove_dir_all(&work);
        (phases, cmd)
    }

    #[test]
    fn an_ipsec_agreement_opens_the_ipsec_transport() {
        let (phases, cmd) = phase_11("ipsec-fips");
        assert_eq!(phases, ["=== Phase 11 [A] open the ipsec-fips transport ==="]);
        assert!(cmd.contains("--limit 'edge-x'"), "{cmd}");
        assert!(cmd.ends_with(" peering-edge.yml"), "{cmd}");
    }

    #[test]
    fn a_wireguard_agreement_opens_the_wireguard_transport() {
        let (phases, cmd) = phase_11("wireguard");
        assert_eq!(phases, ["=== Phase 11 [A] open the wireguard transport ==="]);
        assert!(cmd.ends_with(" peering-wg.yml"), "{cmd}");
    }

    /// F-495: a transport with no play refuses and never substitutes one that has.
    #[test]
    fn an_unknown_transport_refuses_and_substitutes_nothing() {
        let (phases, cmd) = phase_11("quic");
        assert_eq!(phases, ["=== Phase 11 [GATE] open the transport — REFUSED, no play for 'quic' ==="]);
        assert!(cmd.trim_end().ends_with("false"), "{cmd}");
    }

    #[test]
    fn an_undeclared_environment_is_refused() {
        let o = Options { with: "hello-ally", env: Some("nowhere"), r#as: None, edge_group: None, work: None, mock: None, from: 1, to: 99, dry_run: true, table: false };
        assert!(federate(&repo(), &o).is_err());
    }

    /// C1 with an empty list: the repository's own config declares no peering, and
    /// an empty list refuses every name rather than passing it.
    #[test]
    fn an_empty_peerings_list_refuses_every_name() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let o = Options { with: "no-such-peer", env: None, r#as: None, edge_group: None, work: None, mock: None, from: 1, to: 99, dry_run: true, table: false };
        let Err(e) = federate(&repo(), &o) else { panic!("an undeclared peering must be refused") };
        assert!(e.to_string().contains("declare it first (C1)"), "{e}");
    }

    /// `--mock <fixture>` reads the fixture's flat goatbuilder.yaml: the peering
    /// it declares plans (C1 is satisfied by the fixture), and one it does not
    /// declare is refused. The worked example is the tutorial-3 fixture, so this
    /// also holds the fixture and the verb to the same declaration.
    #[test]
    fn a_mock_fixture_supplies_the_declared_peering() {
        let fx = fixture();
        let held = crate::testdir::path("goatbuilder-federate-mock-");
        let work = held.join("work");
        let work_s = work.display().to_string();
        let declared = Options { with: "hello-ally", env: None, r#as: Some("zagros"), edge_group: None, work: Some(&work_s), mock: Some(&fx), from: 1, to: 99, dry_run: true, table: false };
        let r = federate(&repo(), &declared).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        assert_eq!(r.checks.iter().filter(|c| c.name == "phase").count(), 14);
        let undeclared = Options { with: "no-such-peer", mock: Some(&fx), ..declared };
        assert!(federate(&repo(), &undeclared).is_err(), "an undeclared peering must be refused (C1)");
        let _ = std::fs::remove_dir_all(&work);
    }

    /// A mock executes nothing real, without `--dry-run` as well as with it. The
    /// goat binary below records that it ran; a mock run must leave no such record,
    /// and must not create its working directory.
    #[test]
    fn a_mock_without_dry_run_executes_nothing_and_records_every_phase() {
        let root = crate::testdir::path("goatbuilder-federate-mock-exec-");
        std::fs::create_dir_all(&root).unwrap();
        let sentinel = root.join("ran");
        let stub = root.join("goat-stub");
        std::fs::write(&stub, format!("#!/bin/sh\ntouch '{}'\n", sentinel.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        runenv::set("GOAT_BIN", stub.display().to_string());
        let work = root.join("work");
        let work_s = work.display().to_string();
        let fx = fixture();
        let o = Options { with: "hello-ally", env: None, r#as: Some("zagros"), edge_group: None, work: Some(&work_s), mock: Some(&fx), from: 1, to: 99, dry_run: false, table: false };
        let r = federate(&repo(), &o).unwrap();
        runenv::unset("GOAT_BIN");
        assert_eq!(r.exit, EXIT_OK);
        assert_eq!(r.checks.iter().filter(|c| c.name == "mock").count(), 14, "every phase is recorded, past the MANUAL ones");
        assert!(!sentinel.exists(), "a mock ran a real command");
        assert!(!work.exists(), "a mock created its working directory");
        let _ = std::fs::remove_dir_all(&root);
    }
}

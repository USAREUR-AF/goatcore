//! Rendering (C10): one flag selects a table for a person or versioned JSON/YAML
//! for a program. The JSON carries its own schema version (`goatbuilder.output/v1`)
//! so a consumer never scrapes prose.
//!
//! Color is a table-only affordance (`--color auto|always|never`): the `ok`/`FAIL`
//! glyph and the verdict word, nothing else, so a redirected or piped table stays
//! greppable. `auto` colors only a stdout that is a terminal; `CLICOLOR_FORCE` and
//! `NO_COLOR` (https://no-color.org) override, as goat's output does
//! (crates/goat-cli/src/output.rs).

use std::io::IsTerminal;

use crate::{Check, OutputFormat, Report};

/// `--color`'s three settings. `auto` is the default: a terminal gets color, a
/// pipe or file does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// Whether the table should carry ANSI color, from the flag and the environment.
/// `CLICOLOR_FORCE` forces it on even off a terminal; `NO_COLOR` forces it off;
/// otherwise `auto` follows whether stdout (where the table goes) is a terminal.
pub fn want_color(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
                true
            } else if std::env::var_os("NO_COLOR").is_some() {
                false
            } else {
                std::io::stdout().is_terminal()
            }
        }
    }
}

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn paint(on: bool, code: &str, s: &str) -> String {
    if on {
        format!("{code}{s}{RESET}")
    } else {
        s.to_string()
    }
}

pub fn render(report: &Report, format: OutputFormat, color: ColorChoice) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(report).unwrap_or_default(),
        OutputFormat::Yaml => serde_yaml::to_string(report).unwrap_or_default(),
        OutputFormat::Table => {
            let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
            let on = want_color(color);
            // history's rows are a timeline, not checks. A script still gets the rows.
            let page = match report.verb {
                "history" if terminal => history_page(report, on),
                "promote" if terminal => promote_page(report, on),
                "release" if terminal => release_page(report, on),
                "standup" if terminal => standup_page(report, on).or_else(|| plan_page(report, on)),
                "join-node" | "teardown" if terminal => plan_page(report, on),
                "pins" if terminal => pins_page(report, on).or_else(|| pins_wrote_page(report, on)),
                "fastlane" if terminal => fastlane_page(report, on),
                "diff" if terminal => crate::verbs::diff_page(report, on),
                _ => None,
            };
            page.unwrap_or_else(|| table(report, on, terminal))
        }
    }
}

/// A `phase` row whose detail is a `=== … ===` banner prints as a section
/// heading ("Phase 5 · core · netbird-mgmt"), not as a check row: it is a
/// title, and the banner's `===` and `[class]` read as noise to a person. The
/// row itself is unchanged, so -o json and every gate that reads it still see
/// the banner.
fn heading(c: &Check) -> Option<String> {
    if c.name != "phase" {
        return None;
    }
    let t = c.detail.trim().strip_prefix("===")?.strip_suffix("===")?.trim();
    // "Phase 5 [core] netbird-mgmt" -> "Phase 5 · core · netbird-mgmt"
    let t = match (t.find(" ["), t.find("] ")) {
        (Some(a), Some(b)) if a < b => format!("{} \u{b7} {} \u{b7} {}", &t[..a], &t[a + 2..b], &t[b + 2..]),
        _ => t.to_string(),
    };
    Some(t)
}

/// `history` as a table, one line per ledger row. `None` when a row isn't one this
/// page understands (a refusal, a fallback to the in-repo file): the plain table
/// then prints every row, so nothing a person needs is dropped.
fn history_page(report: &Report, color: bool) -> Option<String> {
    // A real ledger's row can end with (reversible:false) or (finished <when>).
    let row = regex::Regex::new(r"^\s*(\S+)\s+\[(\w+)\]\s+(\S+)(?:\s+\(.*\))?\s*$").ok()?;
    let mut heading = String::new();
    let mut machine = String::new();
    let mut rows: Vec<[String; 5]> = Vec::new();
    for c in &report.checks {
        if !c.ok {
            return None;
        }
        let d = c.detail.as_str();
        let t = d.trim();
        if let Some(rest) = t.strip_prefix("deploy history for ") {
            heading = rest.trim_end_matches(':').replace(" (newest first)", "");
        } else if let Some(rest) = t.strip_prefix("(ledger @ ").or_else(|| t.strip_prefix("(on-host ledger @ ")) {
            machine = rest.split([' ', ')']).next()?.to_string();
        } else if t == "(no rows)" {
            rows.push([machine.clone(), "no rows".into(), String::new(), String::new(), String::new()]);
        } else if let Some(action) = t.strip_prefix("action=") {
            // A real ledger adds prior= and operator= after the action.
            rows.last_mut()?[2] = action.split_whitespace().next()?.to_string();
        } else {
            let m = row.captures(d)?;
            let reference = &m[3];
            let digest = reference.split_once("@sha256:").map_or("", |(_, h)| h).chars().take(12).collect::<String>();
            rows.push([machine.clone(), m[1].to_string(), String::new(), crate::verbs::version_of(reference), digest]);
        }
    }
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let rows: Vec<Vec<String>> = rows.iter().map(|r| r.to_vec()).collect();
    // How much was read, and where the rest of each record is: the table has
    // no room for the prior version and the operator, and `-o json` does.
    let records = rows.iter().filter(|r| r[1] != "no rows").count();
    let machines = rows.iter().map(|r| r[0].as_str()).collect::<std::collections::BTreeSet<_>>().len();
    let read = plain(&format!("{records} record(s) on {machines} machine(s). Each record's prior version and operator: -o json."));
    let out = format!(
        "{} {heading}, newest first\n\n{}\n\n{}",
        bold("History:"),
        columns(&["MACHINE", "WHEN", "ACTION", "VERSION", "DIGEST"], &rows, color),
        crate::tutor::fill(&read, "", "").trim_end(),
    );
    Some(out)
}

/// Lay rows out in columns under bold titles, each as wide as its longest cell.
pub(crate) fn columns(titles: &[&str], rows: &[Vec<String>], color: bool) -> String {
    let widths: Vec<usize> = (0..titles.len()).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0).max(titles[i].chars().count())).collect();
    let line = |cells: Vec<&str>| {
        let mut l = String::new();
        for (cell, w) in cells.iter().zip(&widths) {
            l.push_str(&format!("{cell:<w$}  "));
        }
        l.trim_end().to_string()
    };
    let head = line(titles.to_vec());
    let mut out = if color { format!("\x1b[1m{head}\x1b[0m") } else { head };
    for r in rows {
        out.push('\n');
        out.push_str(&line(r.iter().map(String::as_str).collect()));
    }
    out
}

/// `promote status` as two tables. `None` for any other promote report, or a row
/// this page doesn't understand: the plain table then prints every row.
fn promote_page(report: &Report, color: bool) -> Option<String> {
    let record = regex::Regex::new(r"^\s*([^:\s]+): (\S+) opened (\S+) ref \S+\s+\((\S+)\)$").ok()?;
    let mut lines = report.checks.iter().map(|c| (c.name == "note" && c.ok).then_some(c.detail.as_str()));
    let first = lines.next()??;
    if first.split_whitespace().take(2).collect::<Vec<_>>() != ["component", "grade"] {
        return None;
    }
    let titles: Vec<String> = first.split_whitespace().map(str::to_uppercase).collect();
    let (mut grades, mut records, mut count) = (Vec::new(), Vec::new(), None);
    for l in lines {
        let l = l?;
        let t = l.trim();
        if t.is_empty() || t.chars().all(|c| c == '-') {
            continue;
        }
        if let Some(n) = t.strip_prefix("open records (").and_then(|r| r.strip_suffix("):")) {
            count = Some(n.to_string());
        } else if count.is_some() {
            let m = record.captures(l)?;
            let file = m[4].rsplit('/').next()?.to_string();
            records.push(vec![m[1].to_string(), m[2].to_string(), m[3].to_string(), file]);
        } else {
            let cells: Vec<String> = t.split_whitespace().map(String::from).collect();
            if cells.len() != titles.len() {
                return None;
            }
            grades.push(cells);
        }
    }
    let heads: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut out = columns(&heads, &grades, color);
    if let Some(n) = count {
        let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
        out.push_str(&format!("\n\n{} {n}, in ops/release/records/<component>/\n\n", bold("Open records:")));
        out.push_str(&columns(&["COMPONENT", "KIND", "OPENED", "RECORD"], &records, color));
    }
    Some(out)
}

/// The verdict word, coloured: what every table ends with.
fn verdict_words(report: &Report, color: bool) -> String {
    let verdict = match report.exit {
        // A read reports what is there. Green "ok" under a page that says
        // "0 ready, 11 blocked" reads as a verdict on the ladder.
        crate::EXIT_OK if report.verb == "promote" && report.checks.iter().any(|c| c.detail.starts_with("LADDER ")) => return "read".to_string(),
        crate::EXIT_OK => "ok",
        // rollback's exit 3 is the script's "refused as unsafe", not pending machines.
        crate::EXIT_NOT_CONVERGED if report.verb == "rollback" && report.checks.iter().any(|c| c.detail.starts_with("REFUSING")) => "refused as unsafe",
        // Nothing converges in a preflight: it reads what is declared.
        crate::EXIT_NOT_CONVERGED if report.verb == "preflight" => "not clean",
        crate::EXIT_NOT_CONVERGED => "not converged",
        _ => "refused",
    };
    paint(
        color,
        match report.exit {
            crate::EXIT_OK => GREEN,
            crate::EXIT_NOT_CONVERGED if verdict == "not converged" => YELLOW,
            _ => RED,
        },
        verdict,
    )
}

/// A row's capitalized topic and its text: the topic the detail already opens
/// with (`State: stateful`), else the last part of the row's name.
fn topic_of(name: &str, detail: &str) -> (String, String) {
    let cap = |t: &str| {
        let mut c = t.chars();
        c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect::<String>())
    };
    if let Some((head, rest)) = detail.split_once(": ") {
        if !head.contains(' ') && head.chars().all(|c| c.is_alphabetic() || c == '-') {
            return (cap(head), rest.to_string());
        }
    }
    let last = name.rsplit('/').next().unwrap_or(name);
    let rest = detail.strip_prefix(&format!("{last} ")).unwrap_or(detail);
    (cap(last), rest.to_string())
}

/// Whether this run asked for `-o json` or `-o yaml`. A verb that prints its own
/// page on a terminal checks this: `-o json` is the contract on any stream.
static MACHINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub fn set_machine_output(machine: bool) {
    let _ = MACHINE.set(machine);
}

pub fn machine_output() -> bool {
    MACHINE.get().copied().unwrap_or(false)
}

/// The checkout's declared owner (`repo: source:`), for the pages that name it.
static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_owner(owner: String) {
    let _ = OWNER.set(owner);
}

/// Image references and the digest line, in this checkout's name rather than ours.
fn own_names(text: &str, owner: &str) -> String {
    text.replace("{{ ghcr_org }}", &owner.to_lowercase()).replace(&format!("{owner}: sha256:"), "digest: sha256:")
}

/// Insider words as plain English, for a person's page only: `-o json` and a
/// pipe keep the words the gates compare (docs/design/goatbuilder-output.md rule 4).
fn plain(text: &str) -> String {
    let mut s = text.to_string();
    for (from, to) in [
        ("DRY: would ", "A real run will "),
        ("rollback: none (undeclared \u{2014} fail closed)", "no rollback declared"),
        ("none (undeclared \u{2014} fail closed)", "none declared"),
        ("stateful (undeclared \u{2014} fail closed)", "not declared, so treated as stateful"),
        ("true, topology not skew: ", ""),
        ("UNKNOWN: no declared ref", "no declared ref"),
        // What a kind means, rather than the field it is declared in. The kind
        // decides whether a rollback is a ref swap or a refusal.
        ("artifact_type=image reversible=true (declared/artifacts.yaml)", "a container image, which an earlier version can replace (declared/artifacts.yaml)"),
        ("artifact_type=image reversible=false (declared/artifacts.yaml)", "a container image declared not reversible (declared/artifacts.yaml)"),
        ("artifact_type=data (declared/artifacts.yaml)", "data, which an earlier version cannot undo (declared/artifacts.yaml)"),
        ("artifact_type=cert (declared/artifacts.yaml)", "a certificate, which is re-minted rather than reverted (declared/artifacts.yaml)"),
        ("artifact_type=forward-only (declared/artifacts.yaml)", "a change that only moves forward (declared/artifacts.yaml)"),
        // A dry run says so in words, not in a prefix a script would read.
        ("DRY: ", "A real run: "),
        ("an undeclared fact is a fail-closed gap, not a green", "a fact that nothing declares counts as a failure here, never as a pass"),
        ("Scope: single", "Scope: single component"),
        // The driver's own argument is not something a person types, so the
        // human page drops it. Dropping it takes its punctuation with it:
        // `<play>: -e target=<group>; declared in ...` left a bare `:;` behind
        // on every one of doctor's playbook rows.
        (": -e target=<group>", ""),
        (" -e target=<group>", ""),
        (" and fold it into fleet:", ""),
        ("--ack '<name>'", "--ack \"$(git config user.name)\""),
        (", with an operator ack:", ", run:"),
        ("a real run puts fleet ", "a real run puts "),
        ("no live rows are mapped for what ", "no live tests cover what "),
        ("evidence is the cheap suites only", "the evidence is quick tests only"),
        ("candidate tag ", "test version tag "),
        ("candidate ref ", "test version "),
        ("preprod candidate", "preprod test version"),
        ("open the candidate", "open a test version"),
        ("current fleet ref:", "current version:"),
        ("new fleet ref:", "new version:"),
        ("not in an open set", "not part of a release in progress"),
        ("would PRESERVE (absent: true \u{2014} topology, not skew): ", "would keep skipping on purpose: "),
        ("would clear deviations: <none>", "would clear goatnet overrides: none"),
        ("would clear deviations: ", "would clear goatnet overrides: "),
        ("rungs dev=", "stages dev="),
        ("1 day(s)", "1 day"),
        ("day(s)", "days"),
        ("fleet ref ", "current version "),
    ] {
        s = s.replace(from, to);
    }
    s = s.replacen(": DRY: a real run", ": a real run", 1);
    static GRADUATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let graduate = GRADUATE.get_or_init(|| regex::Regex::new(r"# graduate, fold into fleet:, deploy (\S+), verify every rung").unwrap());
    s = graduate.replace_all(&s, "# makes it the version every goatnet declares, deploys $1, checks every stage").into_owned();
    if let Some(owner) = OWNER.get() {
        s = own_names(&s, owner);
    }
    // `5 machine(s)` is how a script writes a count it cannot read. A page can:
    // one machine, five machines.
    static PLURAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // The count and the noun are not always neighbours: "0 earlier ref(s)".
    let plural = PLURAL.get_or_init(|| regex::Regex::new(r"\b(\d+)((?: [A-Za-z][A-Za-z-]*)*?) ([A-Za-z][A-Za-z-]*)\(s\)").unwrap());
    s = plural
        .replace_all(&s, |c: &regex::Captures| {
            let (n, between, word) = (&c[1], &c[2], &c[3]);
            let n = &format!("{n}{between}");
            if &c[1] == "1" {
                format!("{n} {word}")
            } else if let Some(stem) = word.strip_suffix('y') {
                format!("{n} {stem}ies")
            } else if word.ends_with('s') || word.ends_with("ch") || word.ends_with("sh") || word.ends_with('x') {
                format!("{n} {word}es")
            } else {
                format!("{n} {word}s")
            }
        })
        .to_string();
    static FLEET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let fleet = FLEET.get_or_init(|| regex::Regex::new(r"would set components\.\S+\.fleet = ").unwrap());
    s = fleet.replace_all(&s, "would set the fleet version to ").to_string();
    static DIGEST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let digest = DIGEST.get_or_init(|| regex::Regex::new(r"@sha256:<digest of [^>]*>").unwrap());
    s = digest.replace_all(&s, "").to_string();
    static VIA: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let via = VIA.get_or_init(|| regex::Regex::new(r"^groups \[([^\]]*)\] via (door|mesh).*$").unwrap());
    s = via
        .replace(&s, |c: &regex::Captures| format!("{} {}", &c[1], if &c[2] == "door" { "through the door" } else { "over the mesh" }))
        .to_string();
    if s.ends_with("\u{b7} single") {
        s.push_str(" component");
    }
    s.replace(" (ops/validation/flow-map.yaml) \u{2014} ", " (ops/validation/flow-map.yaml), so ").replace(" \u{2014} ", ": ")
}

/// A release dry run as tables with headers: what is being released, one row per
/// stage, and any secret that would stop it. `None` for anything else (a real run,
/// a refusal): the step-by-step page then prints every row.
fn release_page(report: &Report, color: bool) -> Option<String> {
    use crate::tutor::{fill, shell_wrap};
    let is_dry = |d: &str| d.starts_with("DRY:") || d.contains(": DRY: nothing was read");
    if !report.checks.iter().all(|c| c.ok) || !report.checks.iter().any(|c| is_dry(&c.detail)) {
        return None;
    }
    #[derive(Default)]
    struct Stage { name: String, goatnets: Vec<String>, hosts: Vec<String>, reach: Vec<String>, commands: usize, live: String }
    let via = regex::Regex::new(r"^groups \[([^\]]*)\] via (door|mesh)").ok()?;
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let mut version = String::new();
    let mut items: Vec<(String, String)> = Vec::new();
    let mut stages: Vec<Stage> = Vec::new();
    let mut unset: Vec<Vec<String>> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut later: Vec<String> = Vec::new();
    let mut stops: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut finally = String::new();
    let mut said = std::collections::HashSet::new();
    for c in &report.checks {
        let flat = c.detail.split_whitespace().collect::<Vec<_>>().join(" ");
        if c.name == "step" {
            section = flat.trim_start_matches("==> ").to_string();
            if let Some((stage, _)) = section.split_once(" rung:") {
                stages.push(Stage { name: stage.to_string(), ..Default::default() });
            } else if section.starts_with("every rung") {
                stages.push(Stage { name: "check all".to_string(), ..Default::default() });
            }
            if let Some(rest) = section.strip_prefix("deploy ") {
                version = rest.split(" to ").next().unwrap_or("").to_string();
            }
            continue;
        }
        let goatnet = c.name.split('/').next().unwrap_or("").replace('-', "_");
        if let Some((_, rest)) = flat.split_once("NOT SET: ") {
            let on = flat.split(" on ").nth(1).and_then(|r| r.split(' ').next()).unwrap_or("").replace('-', "_");
            for n in rest.split_whitespace().map(|w| w.trim_end_matches(';')).take_while(|w| w.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_' || ch == '|')) {
                if said.insert(format!("{on} {n}")) {
                    unset.push(vec![on.clone(), n.to_string()]);
                }
            }
            continue;
        }
        // What a real run refuses is the point of a dry run, whatever row it came in.
        if flat.contains("refuses without it") || flat.contains("served payloads UNRESOLVED") || flat.contains("need the console's Playwright toolchain") || (flat.starts_with("fix: ") && section.starts_with("live rows")) {
            stops.push(plain(flat.trim_start_matches("DRY: ")));
            continue;
        }
        if is_dry(&flat) {
            if let Some(s) = stages.last_mut() {
                s.commands += 1;
            }
            continue;
        }
        let last = c.name.rsplit('/').next().unwrap_or("");
        if let Some(m) = via.captures(&flat) {
            if let Some(s) = stages.last_mut() {
                if !s.goatnets.contains(&goatnet) {
                    s.goatnets.push(goatnet);
                }
                s.hosts.extend(m[1].split_whitespace().map(String::from));
                if !s.reach.iter().any(|r| r == &m[2]) {
                    s.reach.push(m[2].to_string());
                }
            }
        } else if flat.starts_with("no live rows are mapped") {
            if let Some(s) = stages.last_mut() {
                s.live = "none".to_string();
            }
        } else if section.starts_with("live rows") {
            if let Some(s) = stages.last_mut() {
                s.live = "yes".to_string();
            }
        } else if let Some(rest) = flat.strip_prefix("absent: true, topology not skew: ") {
            skipped.push(format!("{goatnet}: {rest}"));
        } else if c.name == "note" && section == "soak" {
            later.push(plain(&flat));
        } else if flat.contains(": DRY: a real run") {
            finally = plain(&flat);
        } else if c.name == "note" {
            if flat.starts_with("references (pointers") || flat.starts_with("OP_ACCOUNT=") || flat.starts_with("secrets: a real run") {
                continue;
            }
            for part in plain(&flat).split("; ") {
                let (item, value) = match part.split_once(' ') {
                    Some(("image", v)) => ("Image", v.to_string()),
                    Some(("stages", v)) => ("Stages", v.to_string()),
                    Some(("current", v)) => ("Current version", crate::verbs::version_of(v.trim_start_matches("version "))),
                    Some(("deploying", v)) => ("Deploying from", v.trim_start_matches("from ").to_string()),
                    Some(("deps", v)) => ("Dependencies", v.to_string()),
                    Some(("test", v)) if v.starts_with("version tag ") => ("Test version tag", v.trim_start_matches("version tag ").to_string()),
                    Some(("test", v)) => ("Test version", crate::verbs::version_of(v.trim_start_matches("version "))),
                    _ if part.contains(" holds admin on ") => ("Your access", part.to_string()),
                    _ => ("Note", part.to_string()),
                };
                items.push((item.to_string(), value));
            }
        } else if last == "playbook" && said.insert(flat.clone()) {
            items.push(("Playbook".to_string(), plain(&flat)));
        } else if last == "preflight" && said.insert(flat.clone()) {
            items.push(("Change type".to_string(), topic_of(&c.name, &plain(&flat)).1));
        }
    }
    let mut out = format!("{} {version} (dry run: nothing changes)\n\n", bold("Release:"));
    let w = items.iter().map(|(i, _)| i.chars().count()).max().unwrap_or(4).max(4);
    out.push_str(&bold(&format!("{:<w$}  DETAIL", "ITEM")));
    out.push('\n');
    let hang = " ".repeat(w + 2);
    for (item, value) in &items {
        out.push_str(fill(value, &format!("{item:<w$}  "), &hang).trim_end());
        out.push('\n');
    }
    let rows: Vec<Vec<String>> = stages
        .iter()
        .map(|s| vec![s.name.clone(), s.goatnets.join(" "), s.hosts.join(" "), s.reach.join(" "), s.commands.to_string(), if s.live.is_empty() { "-".to_string() } else { s.live.clone() }])
        .collect();
    out.push_str(&format!("\n{}\n", columns(&["STAGE", "GOATNET", "HOSTS", "REACH", "COMMANDS", "LIVE TESTS"], &rows, color)));
    if !skipped.is_empty() {
        out.push_str(&format!("\n{}\n", bold("Skipped on purpose:")));
        for s in &skipped {
            out.push_str(fill(s, "  ", "    ").trim_end());
            out.push('\n');
        }
    }
    if !unset.is_empty() {
        let table = columns(&["GOATNET", "SECRET NOT SET"], &unset, color);
        out.push_str(&format!("\n{table}\n"));
        let warn = "! A real run stops on these. The goatbuilder doctor command shows the fix.";
        out.push_str(&format!("{}\n", if color { format!("\x1b[1;33m{warn}\x1b[0m") } else { warn.to_string() }));
    }
    if !stops.is_empty() {
        out.push_str(&format!("\n{}\n", bold("A real run stops on these:")));
        for s in &stops {
            out.push_str(fill(s, "  ", "    ").trim_end());
            out.push('\n');
        }
    }
    if !later.is_empty() {
        out.push_str(&format!("\n{}\n", bold("Wait before prod")));
        for l in &later {
            match l.strip_prefix("goatbuilder ").and_then(|r| r.split_once(" # ")) {
                Some((cmd, why)) => {
                    out.push_str(&format!("\n{}\n\n", shell_wrap(&format!("goatbuilder {cmd}"), "    ").trim_end()));
                    out.push_str(fill(why, "  ", "  ").trim_end());
                    out.push('\n');
                }
                None => {
                    out.push_str(fill(l, "  ", "  ").trim_end());
                    out.push('\n');
                }
            }
        }
    }
    if !finally.is_empty() {
        out.push('\n');
        out.push_str(fill(&real_run_line(&finally), "", "").trim_end());
        out.push('\n');
    }
    out.push_str("\nCOMMANDS is how many commands each stage runs. Add -o json to list them.\n");
    // Passing a dry run says nothing about a real run these would stop.
    let end = if !stops.is_empty() {
        "Dry run finished. Nothing changed. Fix what is listed above before a real run."
    } else if !unset.is_empty() {
        "Dry run finished. Nothing changed. Set the secrets above before a real run."
    } else {
        "Dry run passed. Nothing changed."
    };
    Some(format!("{}\n\n{}", out.trim_end(), fill(end, "", "").trim_end()))
}

/// A `pins --check` that found a generated file out of date: which setting differs
/// and what to run, not a raw diff. `None` for anything else, or a change this page
/// can't read as one setting: the plain lines then print every row.
fn pins_page(report: &Report, color: bool) -> Option<String> {
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let short = |v: &str| {
        let v = v.trim().trim_matches('"');
        match v.split_once("@sha256:") {
            Some((_, h)) => h.chars().take(12).collect::<String>(),
            None => v.to_string(),
        }
    };
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut file = String::new();
    let mut removed: Option<(String, String)> = None;
    for c in report.checks.iter().filter(|c| c.name == "pins") {
        for line in c.detail.lines() {
            // A diff row is two spaces, then its marker: a space is context.
            let t = line.strip_prefix("  ").unwrap_or(line);
            if t.starts_with(' ') {
                continue;
            }
            if let Some(rest) = t.strip_prefix("FAIL: ") {
                file = rest.split(" is out of date").next()?.trim_start_matches("ops/ansible/group_vars/").to_string();
                if !rest.contains(" is out of date") {
                    return None;
                }
            } else if t.starts_with("--- ") || t.starts_with("+++ ") || t.starts_with("@@") {
                continue;
            } else if let Some(change) = t.strip_prefix('-') {
                removed = Some((change.split_once(": ")?.0.to_string(), change.split_once(": ")?.1.to_string()));
            } else if let Some(change) = t.strip_prefix('+') {
                let (key, value) = change.split_once(": ")?;
                let (was_key, was) = removed.take()?;
                if was_key != key {
                    return None;
                }
                rows.push(vec![file.clone(), key.to_string(), short(&was), short(value)]);
            }
        }
    }
    if rows.is_empty() || removed.is_some() {
        return None;
    }
    let mark = if color { "\x1b[31m\u{2717}\x1b[0m" } else { "\u{2717}" };
    let n = rows.len();
    let head = format!("{mark} {} {n} setting{} differ from the pins.", bold("Pins:"), if n == 1 { "" } else { "s" });
    let head = if n == 1 { head.replace("differ", "differs") } else { head };
    let table = columns(&["FILE", "SETTING", "FILE HAS", "PINS SAY"], &rows, color);
    let why = crate::tutor::fill("These files are generated from ops/pins/fleet-pins.yaml, so a difference means someone edited one by hand.", "", "");
    // The fix has to name the checkout that was checked, or it runs on another one.
    let args: Vec<String> = std::env::args().collect();
    let fix = pins_fix(&args);
    Some(format!("{head}\n\n{table}\n\n{}\n{} {fix}", why.trim_end(), bold("Fix:")))
}

/// `goatbuilder pins`, against the same checkout as the command that ran.
fn pins_fix(args: &[String]) -> String {
    match args.iter().position(|a| a == "-C" || a == "--repo").and_then(|i| args.get(i + 1)) {
        Some(repo) => format!("goatbuilder -C {repo} pins"),
        None => "goatbuilder pins".to_string(),
    }
}

/// `goatbuilder pins` regenerates every file, changed or not, so `wrote` says
/// nothing about what differs: say so, and how to see what did.
fn pins_wrote_page(report: &Report, color: bool) -> Option<String> {
    let rows: Vec<&str> = report.checks.iter().map(|c| c.detail.trim()).collect();
    if rows.is_empty() || !report.checks.iter().all(|c| c.ok && c.name == "pins") || !rows.iter().all(|d| d.starts_with("wrote ")) {
        return None;
    }
    let pins = rows.iter().filter(|d| d.contains("pins.yml")).count();
    let intents = rows.iter().filter(|d| d.contains("intent.json")).count();
    let tag = if color { "\x1b[1;32m\u{2713} Pins:\x1b[0m" } else { "\u{2713} Pins:" };
    Some(format!("{tag} regenerated {pins} pins files and {intents} intents from ops/pins/fleet-pins.yaml.\nIt rewrites every file, even when nothing differs. git status shows changes."))
}

/// One step a person does by hand, laid out as the tutor lays out a step: a mark,
/// its instruction, the command in bold, then any note. A plan reads nothing from
/// the system, so a mark here only ever means "still to do".
pub(crate) fn hand_step(n: usize, title: &str, command: Option<&str>, note: &str, color: bool) -> String {
    use crate::tutor::fill;
    // Numbered, never ticked: in the tutor a mark means a step the learner
    // still owes, and a plan owes nothing. These are phases of a real run.
    let first = format!("  {n}. ");
    let pad = " ".repeat(first.chars().count());
    let head = fill(&format!("{title}:"), &first, &pad);
    let head = head.trim_end();
    let mut out = if color { head.replacen(title, &format!("\x1b[1m{title}\x1b[0m"), 1) } else { head.to_string() };
    out.push('\n');
    if let Some(c) = command {
        let laid = fill(c, &pad, &format!("{pad}  "));
        let laid = laid.trim_end();
        out.push('\n');
        out.push_str(&if color { format!("\x1b[1m{laid}\x1b[0m") } else { laid.to_string() });
        out.push('\n');
    }
    if !note.is_empty() {
        out.push('\n');
        out.push_str(fill(note, &pad, &pad).trim_end());
        out.push('\n');
    }
    out
}

/// A manual phase's text as (command, note) in plain words.
fn manual_parts(text: &str) -> (Option<String>, String) {
    let t = text.trim();
    let t = t.strip_prefix("on the node: ").unwrap_or(t);
    let dash = regex::Regex::new(r"^((?:ssh|sudo) .*?)(?: \u{2014} | \(|\. |$)(.*)$").unwrap();
    let dashboard = regex::Regex::new(r"^in the NetBird dashboard of (\S+) \(or `goat admin` on it\): Setup Keys \u{2192} new key, auto_groups = the group a live mgmt host of \S+ holds; keep it for phase (\d+)").unwrap();
    if let Some(m) = dashboard.captures(t) {
        return (None, format!("In the NetBird dashboard of {0}, or with goat admin, open Setup Keys and make a new key. Give it the group a live mgmt host of {0} holds. Keep the key for phase {1}.", &m[1], &m[2]));
    }
    let Some(m) = dash.captures(t) else {
        return (None, plain(t));
    };
    let mut note = m[2].to_string();
    for (from, to) in [
        ("the address handed over may be 192.168.55.1: that is l4tbr0, the Jetson USB device-mode bridge on the box itself, reachable only over a direct USB-C cable. Find the real LAN address before anything else, then pass it as --address.", "Use the box's real LAN address, and pass it as --address. If you were given 192.168.55.1, that is only its USB-C bridge."),
        ("match the goatnet's declared PQC posture; never also pass --rosenpass-permissive=false). Gate: ", "Match the goatnet's declared PQC posture first: the flag printed here is an example, not the answer. Never also pass --rosenpass-permissive=false. It is done when "),
        ("`netbird status` shows Interface type: Kernel AND `sudo nft list tables` shows `table ip netbird`", "netbird status shows Interface type: Kernel, and sudo nft list tables shows table ip netbird."),
        (" \u{2014} Connected alone is an unfiltered node", " Connected alone is not enough."),
        (": Connected alone is an unfiltered node", " Connected alone is not enough."),
    ] {
        note = note.replace(from, to);
    }
    (Some(m[1].trim().to_string()), plain(&note))
}

/// A plan as a table (output standard, rule 3): one row per phase, then what a
/// person has to do by hand. `None` for anything but a plan of phases and commands;
/// `-o json` keeps every command.
fn plan_page(report: &Report, color: bool) -> Option<String> {
    use crate::tutor::fill;
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let (mut title, mut footer, mut verdict) = (String::new(), String::new(), String::new());
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut manual: Vec<(String, String)> = Vec::new();
    let mut needs: Vec<(String, String)> = Vec::new();
    let mut how: Vec<String> = Vec::new();
    // Each phase's command, as the run executes it (step 3 of the legibility work).
    let mut runs: Vec<(String, String, String)> = Vec::new();
    for c in &report.checks {
        if !c.ok {
            return None;
        }
        if let Some(h) = heading(c) {
            if let Some(v) = h.strip_prefix("Teardown verdict: ") {
                verdict = plain(v);
                continue;
            }
            let mut parts = h.splitn(3, " \u{b7} ");
            let first = parts.next()?;
            let phase = first.strip_prefix("Phase ").or_else(|| first.strip_prefix("Teardown of phase "))?.to_string();
            let kind = match parts.next()? {
                "T" => "tool",
                "A" => "ansible",
                "GATE" => "gate",
                "MANUAL" => "manual",
                other => other,
            };
            // The short form of the step: its first clause. -o json has the rest.
            let step = parts.next()?;
            let step = step.split(" \u{2014} ").next()?.split(" (").next()?.trim().replace("mint a setup key carrying", "make a setup key with").replace("enrol the node", "enroll the node").to_string();
            rows.push(vec![phase, kind.to_string(), step]);
            how.push(String::new());
            continue;
        }
        let flat = c.detail.split_whitespace().collect::<Vec<_>>().join(" ");
        match c.name.as_str() {
            "banner" => {
                // "site=a node=b class=c" reads as "site a, node b, class c".
                title = flat.split(" (DRY").next()?.split_whitespace().filter(|w| w.contains('=')).map(|kv| kv.replacen('=', " ", 1)).collect::<Vec<_>>().join(", ");
                if !flat.contains("DRY") {
                    return None;
                }
            }
            "footer" => footer = plain(&flat),
            "note" if flat == "needs: nothing" => {}
            "note" if flat.starts_with("needs: ") => needs.push((rows.last()?[0].clone(), flat.trim_start_matches("needs: ").to_string())),
            // A teardown's note opens with how the step comes down.
            "note" if report.verb == "teardown" => *how.last_mut()? = flat.split(':').next()?.to_string(),
            "note" => needs.push((rows.last()?[0].clone(), plain(&flat))),
            "cmd" => {
                let row = rows.last()?;
                if row[1] == "manual" {
                    manual.push((row[0].clone(), flat.clone()));
                } else {
                    runs.push((row[0].clone(), row[2].clone(), c.detail.trim().to_string()));
                }
            }
            _ => return None,
        }
    }
    if rows.is_empty() || title.is_empty() {
        return None;
    }
    let mut out = format!("{} {}, {title}\n", bold("Plan:"), report.verb);
    out.push_str("Nothing has run. This only shows the steps, so there is nothing to do now.\n\n");
    let wp = rows.iter().map(|r| r[0].chars().count()).max()?.max(5);
    let wk = rows.iter().map(|r| r[1].chars().count()).max()?.max(4);
    let wh = how.iter().map(|h| h.chars().count()).max().unwrap_or(0);
    let wh = if wh > 0 && wh <= 16 { wh.max(3) } else { 0 };
    let head = if wh > 0 { format!("{:<wp$}  {:<wk$}  {:<wh$}  STEP", "PHASE", "TYPE", "HOW") } else { format!("{:<wp$}  {:<wk$}  STEP", "PHASE", "TYPE") };
    out.push_str(&bold(&head));
    out.push('\n');
    let hang = " ".repeat(wp + wk + 4 + if wh > 0 { wh + 2 } else { 0 });
    for (r, h) in rows.iter().zip(&how) {
        let first = if wh > 0 { format!("{:<wp$}  {:<wk$}  {:<wh$}  ", r[0], r[1], h) } else { format!("{:<wp$}  {:<wk$}  ", r[0], r[1]) };
        out.push_str(fill(&r[2], &first, &hang).trim_end());
        out.push('\n');
    }
    if !runs.is_empty() {
        // Verbatim and unwrapped, so a line can be pasted to run one phase by hand.
        out.push_str(&format!("\n{}\n", bold("What each phase runs:")));
        for (n, step, cmd) in &runs {
            out.push_str(&format!("\n{n}  {step}\n    {cmd}\n"));
        }
    }
    if !needs.is_empty() {
        out.push_str(&format!("\n{}\n", bold("What each phase needs:")));
        let wm = needs.iter().map(|(n, _)| n.chars().count()).max()?;
        let hang = " ".repeat(wm + 2);
        for (n, what) in &needs {
            out.push_str(fill(what, &format!("{n:<wm$}  "), &hang).trim_end());
            out.push('\n');
        }
    }
    if !verdict.is_empty() {
        out.push('\n');
        out.push_str(fill(&format!("Result: {verdict}"), "", "  ").trim_end());
        out.push('\n');
    }
    if !manual.is_empty() {
        out.push_str(&format!("\n{}\n", bold("What a real run would leave to you:")));
        for (n, what) in &manual {
            let title = rows.iter().find(|r| &r[0] == n && r[1] == "manual").map_or("", |r| r[2].as_str());
            let (command, note) = manual_parts(what);
            let num: usize = n.parse().unwrap_or(0);
            out.push('\n');
            out.push_str(&hand_step(num, title, command.as_deref(), &note, color));
        }
    }
    out.push_str(&format!("\n{} -o json lists every row of the plan.\n", bold("To see more:")));
    if !footer.is_empty() {
        let footer = footer.replace("PLAN complete. ", "").replace("Remove --dry-run to execute (needs --address).", "Remove --dry-run and add --address.").replace("Manual steps (", "The run pauses at the steps you do yourself (").replace(") pause the run; resume with --from <next>.", "). After each one, continue with --from and the next phase number.");
        out.push_str(fill(&format!("{} {footer}", "To run it for real:"), "", "").trim_end().replacen("To run it for real:", &bold("To run it for real:"), 1).as_str());
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// A standup whose every phase deployed and converged, as one table. `None` for
/// anything else (a manual step, a gate, a failure): the page grouped by phase then
/// prints every row, so nothing is squeezed into a column.
fn standup_page(report: &Report, color: bool) -> Option<String> {
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let converged = regex::Regex::new(r"^converged on (\S+)").ok()?;
    let by = regex::Regex::new(r" by (\S+)$").ok()?;
    let measured = regex::Regex::new(r"^declared \u{2192} verified: (.+) in (.+?) \u{2014} every record names its declared ref$").ok()?;
    let (mut phase, mut class, mut component) = (String::new(), String::new(), String::new());
    let (mut rows, mut names, mut end): (Vec<Vec<String>>, Vec<String>, Option<String>) = (Vec::new(), Vec::new(), None);
    let mut title = String::new();
    for c in &report.checks {
        if !c.ok {
            return None;
        }
        if let Some(h) = heading(c) {
            let mut parts = h.split(" \u{b7} ");
            phase = parts.next()?.strip_prefix("Phase ")?.to_string();
            class = parts.next()?.to_string();
            component = parts.next()?.to_string();
            continue;
        }
        if c.name == "measured" {
            let m = measured.captures(c.detail.trim())?;
            end = Some(format!("{}, all running what main declares. Took {}.", &m[1], &m[2]));
            continue;
        }
        let parts: Vec<&str> = c.name.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        if title.is_empty() {
            title = format!("{}, site {}", parts[0], parts[1]);
        }
        if parts.last() == Some(&"publish") {
            if let Some(m) = by.captures(c.detail.trim()) {
                if !names.iter().any(|n| n == &m[1]) {
                    names.push(m[1].to_string());
                }
            }
            continue;
        }
        let m = converged.captures(c.detail.trim())?;
        rows.push(vec![phase.clone(), class.clone(), component.clone(), parts[parts.len() - 2].to_string(), crate::verbs::version_of(&m[1]), "converged".to_string()]);
    }
    let end = end?;
    if rows.is_empty() {
        return None;
    }
    let mut out = format!("{} {title}\n", bold("Standup:"));
    if !names.is_empty() {
        out.push_str(&format!("{} {}\n", bold("Acknowledged by:"), names.join(", ")));
    }
    out.push('\n');
    // The state reads green here as it does in `status`: the same word about the
    // same machine. Painted after the columns are laid out, and only where it
    // ends a row, so no colour code counts toward a width.
    let table = columns(&["PHASE", "CLASS", "COMPONENT", "MACHINE", "VERSION", "STATE"], &rows, color);
    let table = if color {
        table
            .lines()
            .map(|l| match l.strip_suffix("converged") {
                Some(head) => format!("{head}\x1b[32mconverged\x1b[0m"),
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        table
    };
    out.push_str(&table);
    out.push_str(&format!("\n\n{}", crate::tutor::fill(&end, "", "").trim_end()));
    Some(out)
}

/// An explanation as a person reads it: a bold title, then labelled lines inside
/// the page with the fix in yellow. The catalog's own text is what a pipe gets.
pub fn explanation_page(text: &str, color: bool) -> String {
    use crate::tutor::fill;
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let label = regex::Regex::new(r"^(what|why|fix|read|exit):\s*(.*)$").unwrap();
    let mut out = String::new();
    let mut in_fix = false;
    let mut in_field = false;
    const W: usize = 11; // "Exit code" and its gap
    let hang = " ".repeat(W);
    let field = |name: &str, value: &str, yellow: bool| {
        let first = format!("{name:<W$}");
        let laid = fill(value, &first, &hang);
        let laid = laid.trim_end();
        if laid.len() < first.len() {
            return format!("{}\n", bold(laid));
        }
        let (head, rest) = laid.split_at(first.len());
        let rest = if yellow && color { rest.lines().map(|l| format!("\x1b[33m{l}\x1b[0m")).collect::<Vec<_>>().join("\n") } else { rest.to_string() };
        format!("{}{rest}\n", bold(head))
    };
    let titles = format!("{:<W$}DETAIL", "FIELD");
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || (t.starts_with('[') && t.ends_with(']')) {
            continue;
        }
        if let Some((code, title)) = t.split_once(" \u{2014} ").filter(|(c, _)| !c.contains(' ')) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&bold(&format!("{code}: {title}")));
            out.push_str("\n\n");
            out.push_str(&bold(&titles));
            out.push('\n');
            in_fix = false;
            in_field = false;
        } else if let Some(m) = label.captures(t) {
            in_fix = &m[1] == "fix";
            in_field = true;
            let name = match &m[1] {
                "what" => "What",
                // A catalog entry lists the causes this error can have, which is
                // not a claim about which one happened here. "Why" read as that
                // claim: an HTTP failure was told the host was down.
                "why" => "Causes",
                "fix" => "Fix",
                "read" => "More",
                _ => "Exit code",
            };
            out.push_str(&field(name, &m[2], in_fix));
        } else if in_fix {
            // A step that is a command is laid out as one: broken only where a
            // shell reads the break, never mid-option, so it runs as printed.
            let laid = if crate::doctor::is_command(t) {
                crate::doctor::fix_lines_at(t, &hang, &hang).trim_end().to_string()
            } else {
                fill(t, &hang, &hang).trim_end().to_string()
            };
            out.push_str(&if color { format!("\x1b[33m{laid}\x1b[0m\n") } else { format!("{laid}\n") });
        } else if in_field {
            // A field with more than one line (a second cause, a second fix)
            // keeps its column: only the first line carries the label.
            out.push_str(fill(t, &hang, &hang).trim_end());
            out.push('\n');
        } else {
            out.push_str(fill(t, "", "  ").trim_end());
            out.push('\n');
        }
    }
    out
}

/// A refusal's first line: marked, without the program's name or debug quoting.
pub fn refusal_line(text: &str, color: bool) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = flat.replace("[\"", "").replace("\", \"", ", ").replace("\"]", "").replace(" \u{2014} ", ": ").replace("; known: ", ". Known: ").replace("; declared: ", ". Declared: ");
    let laid = crate::tutor::fill(&t, "\u{2717} ", "  ");
    let laid = laid.trim_end();
    if color { laid.replacen('\u{2717}', "\x1b[31m\u{2717}\x1b[0m", 1) } else { laid.to_string() }
}

/// A fastlane dry run as tables with headers: the versions, then what a real run
/// does. `None` for anything else (a real run, a refusal): the step-by-step page
/// then prints every row.
fn fastlane_page(report: &Report, color: bool) -> Option<String> {
    use crate::tutor::fill;
    if !report.checks.iter().all(|c| c.ok) || !report.checks.iter().any(|c| c.name == "step" && c.detail.contains("dry run")) {
        return None;
    }
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let mut component = String::new();
    let mut items: Vec<(&str, String)> = Vec::new();
    let mut steps: Vec<String> = Vec::new();
    let version = |v: &str| crate::verbs::version_of(plain(v).trim());
    for c in &report.checks {
        if c.name == "step" {
            if let Some(n) = c.detail.trim_start_matches("==> ").strip_prefix("component ") {
                component = n.to_string();
            }
            continue;
        }
        for line in c.detail.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if let Some(v) = flat.strip_prefix("current fleet ref:") {
                items.push(("Current version", version(v)));
            } else if let Some(v) = flat.strip_prefix("new fleet ref:") {
                items.push(("New version", version(v)));
            } else if let Some(d) = OWNER.get().and_then(|o| flat.strip_prefix(&format!("{o}: "))) {
                items.push(("Digest", d.to_string()));
            } else if flat == "not in an open set" {
                items.push(("Release in progress", "no".to_string()));
            } else if let Some(v) = flat.strip_prefix("ledger component:") {
                items.push(("Ledger", v.trim().to_string()));
            } else if flat.contains(".fleet = ") {
                let v = flat.split(".fleet = ").nth(1).unwrap_or("");
                steps.push(format!("Sets the fleet version to {}.", version(v)));
            } else if let Some(v) = flat.strip_prefix("would clear deviations: ") {
                steps.push(format!("Clears goatnet overrides: {}.", v.replace("<none>", "none")));
            } else if let Some(v) = flat.strip_prefix("would PRESERVE (absent: true \u{2014} topology, not skew): ") {
                steps.push(format!("Keeps skipping these on purpose: {v}."));
            } else if let Some(v) = flat.strip_prefix("would deploy via ") {
                steps.push(format!("Deploys with {v}."));
            } else {
                items.push(("Note", plain(&flat)));
            }
        }
    }
    let mut out = format!("{} {component} (dry run: nothing changes)\n\n", bold("Fastlane:"));
    let w = items.iter().map(|(i, _)| i.chars().count()).max().unwrap_or(4).max(4);
    out.push_str(&bold(&format!("{:<w$}  DETAIL", "ITEM")));
    out.push('\n');
    let hang = " ".repeat(w + 2);
    for (item, value) in &items {
        out.push_str(fill(value, &format!("{item:<w$}  "), &hang).trim_end());
        out.push('\n');
    }
    out.push_str(&format!("\n{}\n", bold("STEP  WHAT A REAL RUN DOES")));
    for (i, step) in steps.iter().enumerate() {
        out.push_str(fill(step, &format!("{:<4}  ", i + 1), "      ").trim_end());
        out.push('\n');
    }
    Some(format!("{}\n\nDry run passed. Nothing changed.", out.trim_end()))
}

/// "netvis: a real run puts <image> on a, b and c and reads every ledger" as a
/// sentence a person can follow.
fn real_run_line(line: &str) -> String {
    let parts = line.split_once(": a real run puts ").and_then(|(component, rest)| {
        let (image, rest) = rest.split_once(" on ")?;
        Some((component, image, rest.strip_suffix(" and reads every ledger")?))
    });
    match parts {
        Some((component, image, goatnets)) => format!("A real run puts {component} {} on {goatnets}, then checks that every machine recorded it.", crate::verbs::version_of(image)),
        None => line.to_string(),
    }
}

/// A step's title in plain words.
fn plain_step(title: &str) -> String {
    let t = plain(title)
        .replace(" rung:", " stage:")
        .replace("every rung", "every stage")
        .replace("graduate ", "promote ")
        .replace("Live rows", "Live tests")
        .replace("live rows", "live tests");
    match t.as_str() {
        "soak" | "Soak" => "Wait before prod".to_string(),
        "graduation-set membership" => "Release set".to_string(),
        "resolving image digest" => "Finding the image".to_string(),
        "dry run: stopping before any mutation" => "Dry run: stops here, nothing changes".to_string(),
        _ => t,
    }
}

/// A verb that reports its steps as it goes (`fastlane`, `release`): each step a
/// bold heading with its notes under it, and a refusal marked and in plain words.
fn narrative_body(report: &Report, color: bool) -> String {
    use crate::tutor::fill;
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    let mut out = String::new();
    let image = regex::Regex::new(r"\S*/[\w.-]+:([\w.-]+)@sha256:[0-9a-f\u{2026}]+").unwrap();
    // A dry run's commands, listed under the step that runs them, as they run.
    let mut dry: Vec<String> = Vec::new();
    // A release repeats each deploy's checks for every rung: say each thing once.
    let mut said = std::collections::HashSet::new();
    let flush = |dry: &mut Vec<String>, out: &mut String| {
        if !dry.is_empty() {
            // Right under a heading it is the first thing the step does.
            let under_heading = out.lines().last().is_some_and(|l| !l.starts_with(' ') && !l.starts_with('\u{2713}') && !l.starts_with('!'));
            let then = if under_heading { "A real run runs" } else { "Then a real run runs" };
            // These lines are the plan's own words for each command, not the
            // argv that runs: they are labelled so, until each verb builds them
            // from the command it runs (docs/design/goatbuilder-legibility.md, step 3).
            out.push_str(&format!("  {then}, as the plan describes it:\n"));
            for d in dry.drain(..) {
                out.push_str(&format!("    {d}\n"));
            }
        }
    };
    for c in &report.checks {
        let flat = c.detail.split_whitespace().collect::<Vec<_>>().join(" ");
        // A ledger left unread by a dry run is no command of its own to show.
        if c.ok && flat.contains(": DRY: nothing was read") {
            continue;
        }
        if c.ok && flat.starts_with("DRY:") && !flat.starts_with("DRY: would") {
            dry.push(c.detail.trim().trim_start_matches("DRY:").trim().to_string());
            continue;
        }
        // `Prep` already says these three, and `goatbuilder doctor` names this fix.
        let covered = ["/preflight/state", "/preflight/rollback", "/preflight/scope"].iter().any(|n| c.name.ends_with(n));
        if c.ok && (covered || flat.starts_with("references (pointers") || flat.starts_with("OP_ACCOUNT=") || flat == "secrets: a real run refuses here, before the tag and the pin") {
            continue;
        }
        // The names that aren't set: the words after `NOT SET:`, up to the prose.
        // Reading ledgers back templates nothing, so unset secrets stop nothing there.
        let unset = flat.split_once("NOT SET: ").filter(|_| !flat.contains("not needed for --verify-only")).map(|(_, rest)| {
            rest.split_whitespace().map(|w| w.trim_end_matches(';')).take_while(|w| w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '|')).collect::<Vec<_>>().join(" ")
        });
        // The dedupe key carries the row's place (its machine, or its topic), so
        // a release says each thing once while no machine's row is ever merged.
        let key_parts: Vec<&str> = c.name.split('/').collect();
        let key_machine = key_parts.len() >= 3 && ["converged", "pending", "unknown", "same", "DRIFT"].iter().any(|w| flat.starts_with(w));
        let key_topic = if key_machine { key_parts[key_parts.len() - 2].to_string() } else { topic_of(&c.name, &flat).0 };
        let once = format!("{key_topic}|{}", unset.clone().unwrap_or_else(|| topic_of(&c.name, &flat).1));
        if c.ok && c.name != "step" && (unset.is_some() || c.name != "note") && !said.insert(once) {
            continue;
        }
        flush(&mut dry, &mut out);
        match c.name.as_str() {
            "step" => {
                if !out.is_empty() {
                    out.push('\n');
                }
                let title = plain_step(flat.trim_start_matches("==> "));
                let mut chars = title.chars();
                let title = chars.next().map_or(String::new(), |f| f.to_uppercase().chain(chars).collect());
                out.push_str(&format!("{}\n", bold(&title)));
            }
            "note" if c.ok && unset.is_none() => {
                for line in c.detail.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    let text = plain(&line.split_whitespace().collect::<Vec<_>>().join(" "));
                    // A command to run later gets its own lines, so it can be pasted.
                    match text.strip_prefix("goatbuilder ").and_then(|rest| rest.split_once(" # ")) {
                        Some((cmd, why)) => {
                            out.push('\n');
                            out.push_str(crate::tutor::shell_wrap(&format!("goatbuilder {cmd}"), "    ").trim_end());
                            out.push_str("\n\n");
                            out.push_str(fill(why, "  ", "  ").trim_end());
                            out.push('\n');
                        }
                        None => {
                            out.push_str(fill(&text, "  ", "  ").trim_end());
                            out.push('\n');
                        }
                    }
                }
            }
            name if c.ok => {
                let name = if name == "note" { "secrets" } else { name };
                // A real run refuses on a secret that isn't set, so don't tick it.
                let flat = image.replace_all(&flat, "$1").to_string();
                let parts: Vec<&str> = name.split('/').collect();
                // A machine's own row is titled with the machine, as `status` names it.
                let on_machine = parts.len() >= 3 && ["converged", "pending", "unknown", "same", "DRIFT"].iter().any(|w| flat.starts_with(w));
                let (topic, text) = if on_machine {
                    (parts[parts.len() - 2].to_string(), flat.clone())
                } else if flat.starts_with("DRY: ") {
                    // "DRY" is not a topic: the row's own name is.
                    let (t, _) = topic_of(name, "");
                    (t, flat.clone())
                } else {
                    topic_of(name, &flat)
                };
                let topic = match topic.as_str() {
                    "Prep" => "Change type".to_string(),
                    "Transport" => "Reach".to_string(),
                    "Absent" | "Skipped" => "Skipped on purpose".to_string(),
                    _ => topic,
                };
                let text = plain(&text);
                // Stop at the names: `goatbuilder doctor` names the fix, once.
                let text = match (&unset, text.split_once(" templates ")) {
                    (Some(names), Some((subject, rest))) => {
                        let needs = rest.split_whitespace().next().unwrap_or("").trim_end_matches(';').to_string();
                        let list = names.split_whitespace().collect::<Vec<_>>();
                        let said = if list.len().to_string() == needs { format!("needs {needs} that aren't set:") } else { format!("needs {needs}, and {} aren't set:", list.len()) };
                        format!("{subject} {said}")
                    }
                    _ => text,
                };
                let warn = unset.is_some();
                // A tick is a thing that happened. A row that says what a real
                // run would do has not happened, so it reads as a statement: a
                // bullet, and the title in bold rather than green.
                let plan = !warn && (text.starts_with("A real run") || topic == "Plan");
                let mark = if warn {
                    "!"
                } else if plan {
                    "\u{b7}"
                } else {
                    "\u{2713}"
                };
                let line = format!("{mark} {topic}: {text}");
                let laid = fill(&line, "", "  ");
                let laid = laid.trim_end().to_string();
                // The mark and the topic read as the row's title, in its colour.
                let title = format!("{mark} {topic}:");
                let code = if warn {
                    "\x1b[1;33m"
                } else if plan {
                    "\x1b[1m"
                } else {
                    "\x1b[1;32m"
                };
                let laid = if color { laid.replacen(&title, &format!("{code}{title}\x1b[0m"), 1) } else { laid };
                out.push('\n');
                out.push_str(&laid);
                out.push('\n');
                if let Some(names) = unset.as_ref().filter(|_| warn) {
                    for n in names.split_whitespace() {
                        out.push_str(&format!("  {n}\n"));
                    }
                    out.push_str("  A real run stops here. The goatbuilder doctor command shows the fix.\n");
                }
            }
            _ => {
                let why = plain(flat.trim_start_matches("REFUSE \u{2014} "));
                // A failure names its place: the machine or row someone must fix.
                let place = match c.name.split_once('/') {
                    Some((_, rest)) => format!("{rest}: "),
                    None => String::new(),
                };
                let why = format!("{place}{why}");
                let why = why.as_str();
                let mark = if c.ok { "\u{2713}" } else { "\u{2717}" };
                let laid = fill(why, &format!("{mark} "), "  ");
                let laid = laid.trim_end();
                let code = if c.ok { "\x1b[32m" } else { "\x1b[31m" };
                let laid = if color { laid.replacen(mark, &format!("{code}{mark}\x1b[0m"), 1) } else { laid.to_string() };
                out.push_str(&format!("\n{laid}\n"));
                if let Some(fix) = &c.fix {
                    out.push_str(fill(&format!("Fix: {fix}"), "  ", "       ").trim_end());
                    out.push('\n');
                }
            }
        }
    }
    flush(&mut dry, &mut out);
    out.trim().to_string()
}

/// The steps, then the verdict every table ends with.
fn narrative(report: &Report, color: bool) -> String {
    format!("{}\n\n{}: {}", narrative_body(report, color), report.verb, verdict_words(report, color))
}

/// Rows that repeat one name are lines of text, not a table of checks: print
/// the lines, marking only what failed.
fn plain_lines(report: &Report, color: bool) -> String {
    let mut out = String::new();
    for c in &report.checks {
        if !c.ok {
            let mark = if color { "\x1b[31m\u{2717}\x1b[0m" } else { "\u{2717}" };
            out.push_str(&format!("{mark} "));
        }
        out.push_str(&c.detail);
        out.push('\n');
        if let Some(fix) = &c.fix {
            out.push_str(&format!("Fix: {fix}\n"));
        }
    }
    let n = report.checks.iter().filter(|c| !c.ok).count();
    let tail = if n == 0 { String::new() } else { format!(" ({n} failed)") };
    format!("{}\n\n{}: {}{tail}", out.trim_end(), report.verb, verdict_words(report, color))
}

/// Rows under phase headings: each marked, named by its machine or step, with a
/// version in place of a full image reference. -o json keeps the full rows.
fn phased(report: &Report, color: bool) -> String {
    use crate::tutor::fill;
    let image = regex::Regex::new(r"\S*/[\w.-]+:([\w.-]+)@sha256:[0-9a-f\u{2026}]+").unwrap();
    let bold = |t: &str| if color { format!("\x1b[1m{t}\x1b[0m") } else { t.to_string() };
    // A tick is a thing that happened; a dry run's row says what a real run
    // would do, so it reads as a statement (the same rule the steps follow).
    let mark = |ok: bool, plan: bool| match (ok, plan, color) {
        (true, true, _) => "\u{b7}",
        (true, false, true) => "\x1b[32m\u{2713}\x1b[0m",
        (true, false, false) => "\u{2713}",
        (false, _, true) => "\x1b[31m\u{2717}\x1b[0m",
        (false, _, false) => "\u{2717}",
    };
    let mut out = String::new();
    for c in &report.checks {
        if let Some(h) = heading(c) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(fill(&h, "", "  ").trim_end().replacen(&h, &bold(&h), 1).as_str());
            out.push('\n');
            continue;
        }
        let detail = plain(&image.replace_all(c.detail.trim(), "$1"));
        // A command prints as it runs: no word of it rewritten and no image
        // shortened, only wrapped (docs/design/goatbuilder-legibility.md, step 3).
        let cmd = c.detail.trim();
        match c.name.as_str() {
            // Verbatim and on one line: a wrap that splits inside quotes, or
            // collapses spaces, changes what a pasted line runs.
            "cmd" => {
                out.push_str(&format!("  {cmd}\n"));
            }
            "note" => {
                out.push_str(fill(&detail, "  ", "  ").trim_end());
                out.push('\n');
            }
            "banner" => {
                out.push_str(fill(&detail, "", "").trim_end());
                out.push('\n');
            }
            "measured" => {
                out.push_str(&format!("\n{}\n", fill(&format!("Measured: {detail}"), "", "  ").trim_end()));
            }
            name => {
                let mut parts = name.rsplit('/');
                let last = parts.next().unwrap_or(name);
                let short = match (last, parts.next()) {
                    ("publish", _) | (_, None) => last,
                    (_, Some(machine)) => machine,
                };
                let first = format!("  {} {short}  ", "\u{2713}");
                let laid = fill(&detail, &first, "      ");
                let plan = c.ok && detail.starts_with("A real run");
                let laid = laid.trim_end().replacen('\u{2713}', mark(c.ok, plan), 1);
                out.push_str(&laid);
                out.push('\n');
                if let Some(fix) = &c.fix {
                    out.push_str(fill(&format!("Fix: {fix}"), "      ", "           ").trim_end());
                    out.push('\n');
                }
            }
        }
    }
    format!("{}\n\n{}: {}", out.trim_end(), report.verb, verdict_words(report, color))
}

/// Rows that are a list of named checks can carry column titles. A narrative
/// (steps and notes, phases, or one name on every row) has no columns to title.
fn has_columns(report: &Report) -> bool {
    let first = report.checks.first().map(|c| c.name.as_str());
    let same_name = report.checks.len() > 1 && report.checks.iter().all(|c| Some(c.name.as_str()) == first);
    !same_name && !report.checks.iter().any(|c| matches!(c.name.as_str(), "step" | "note" | "phase"))
}

/// `titles` is on for a terminal only, so a script reading the table is untouched
/// until the gates that still parse it read `-o json` (output standard, rule 1).
fn table(report: &Report, color: bool, titles: bool) -> String {
    // A run that watched machines converge answers the same question as
    // `status`, so those rows print as its table, under whatever else it did.
    if titles && matches!(report.verb, "deploy" | "verify" | "rollback") {
        // Whatever the table does not take, these steps print: one predicate,
        // so a row the table skips (a machine that failed) is never also lost here.
        let rest: Vec<Check> = report.checks.iter().filter(|c| !crate::verbs::machine_row(c)).cloned().collect();
        if let Some(table) = crate::verbs::machines_page(report, color) {
            let head = if rest.is_empty() { String::new() } else { format!("{}\n\n", narrative_body(&Report::new(report.verb, rest, report.exit), color)) };
            return format!("{head}{table}\n\n{}: {}", report.verb, verdict_words(report, color));
        }
    }
    // Practice copy or real hosts, a deploy reads the same.
    let checks_page = matches!(report.verb, "deploy" | "verify" | "rollback") && report.checks.iter().any(|c| c.name.contains('/'));
    if titles && (checks_page || report.checks.iter().any(|c| c.name == "step")) {
        return narrative(report, color);
    }
    if titles && report.checks.iter().any(|c| heading(c).is_some()) {
        return phased(report, color);
    }
    let first = report.checks.first().map(|c| c.name.as_str());
    if titles && report.checks.len() > 1 && report.checks.iter().all(|c| Some(c.name.as_str()) == first) {
        return plain_lines(report, color);
    }
    let titles = titles && has_columns(report);
    let pad = if titles { "  " } else { "" };
    let width = report
        .checks
        .iter()
        .filter(|c| heading(c).is_none())
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .max(4)
        .max(if titles { "CHECK".len() } else { 0 });
    let mut out = String::new();
    if titles {
        let head = format!("{:<6}  {:<width$}  DETAIL", "STATUS", "CHECK", width = width);
        out.push_str(&if color { format!("\x1b[1m{head}\x1b[0m\n") } else { format!("{head}\n") });
    }
    for c in &report.checks {
        if let Some(h) = heading(c) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&if color { format!("\x1b[1m{h}\x1b[0m\n") } else { format!("{h}\n") });
            continue;
        }
        // A check that did not run is neither: its own word, in no colour, so
        // the eye does not read a column of green as a column of results.
        let glyph = if crate::doctor::not_checked(c) {
            "skip".to_string()
        } else if c.ok {
            paint(color, GREEN, "ok  ")
        } else {
            paint(color, RED, "FAIL")
        };
        // On a terminal a long detail wraps under its own column. Where the
        // names are so long that the column leaves no room to read in (the
        // `secrets:<goatnet>:<component>` rows of `--all`), the detail wraps
        // under its row instead: still 80 columns, and still one row one fact.
        let hang = " ".repeat(6 + 2 + width + 2);
        const ROOM: usize = 40;
        let room = 80usize.saturating_sub(hang.chars().count());
        if titles && room < ROOM {
            let under = " ".repeat(8);
            out.push_str(&format!("{}{pad}  {}\n", glyph, c.name));
            out.push_str(&format!("{}\n", crate::tutor::fill(&plain(&c.detail), &under, &under).trim_end()));
            if let Some(fix) = &c.fix {
                out.push_str(&crate::doctor::fix_lines_at(&plain(fix), &format!("{under}fix: "), &format!("{under}     ")));
            }
            continue;
        }
        let detail = if titles {
            crate::tutor::fill(&plain(&c.detail), &hang, &hang).trim_start().trim_end().to_string()
        } else {
            c.detail.clone()
        };
        out.push_str(&format!(
            "{}{pad}  {:<width$}  {}\n",
            glyph,
            c.name,
            detail,
            width = width
        ));
        if let Some(fix) = &c.fix {
            if titles {
                out.push_str(&crate::doctor::fix_lines_at(&plain(fix), &format!("{hang}fix: "), &format!("{hang}     ")));
            } else {
                out.push_str(&format!("{pad}      {:<width$}  fix: {}\n", "", fix, width = width));
            }
        }
    }
    let painted = verdict_words(report, color);
    // Headings are titles, not checks. Neither is the verb's own summary row,
    // which is the verdict this line is about ("preflight: ... NOT clean").
    let n = report.checks.iter().filter(|c| heading(c).is_none() && c.name != report.verb).count();
    if report.checks.iter().any(|c| heading(c).is_some()) {
        out.push('\n');
    }
    out.push_str(&format!("{}: {} ({} check{})", report.verb, painted, n, if n == 1 { "" } else { "s" }));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_names_the_checkouts_own_owner() {
        assert_eq!(own_names("ghcr.io/{{ ghcr_org }}/x", "AcmeCorp"), "ghcr.io/acmecorp/x");
        assert_eq!(own_names("AcmeCorp: sha256:aa", "AcmeCorp"), "digest: sha256:aa");
    }

    use super::*;
    use crate::{Check, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

    fn c(name: &str, ok: bool, fix: Option<&str>) -> Check {
        Check { name: name.into(), ok, detail: format!("{name} detail"), fix: fix.map(String::from) }
    }

    #[test]
    fn a_terminal_table_has_column_titles_and_a_script_does_not() {
        let r = Report::new("doctor", vec![c("config", true, None), c("checkout", false, Some("do x"))], EXIT_REFUSED);
        let on = table(&r, false, true);
        let lines: Vec<&str> = on.lines().collect();
        assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), ["STATUS", "CHECK", "DETAIL"]);
        let col = lines[0].find("CHECK").unwrap();
        assert_eq!(lines[1].find("config"), Some(col), "{on}");
        assert_eq!(lines[2].find("checkout"), Some(col), "{on}");
        assert!(!table(&r, false, false).contains("STATUS"));
        let long = Check { name: "preprod/portal/plan".into(), ok: true, detail: "word ".repeat(30).trim().to_string(), fix: None };
        let page = table(&Report::new("update", vec![long], EXIT_OK), false, true);
        assert!(page.lines().all(|l| l.chars().count() <= 80), "{page}");
        let at = page.lines().next().unwrap().find("DETAIL").unwrap();
        assert!(page.lines().nth(2).unwrap().starts_with(&" ".repeat(at)), "{page}");
    }

    #[test]
    fn history_is_a_table_and_falls_back_when_a_row_is_not_understood() {
        let h = |d: &str| Check { name: "history".into(), ok: true, detail: d.into(), fix: None };
        let digest = "3c".repeat(32);
        let mut rows = vec![
            h("deploy history for portal (newest first):"),
            h("  (ledger @ core-a)"),
            h(&format!("  2026-11-01T16:17:10Z  [image]  ghcr.io/o/portal:v1.4.0@sha256:{digest}")),
            h("      action=deploy"),
            h("  (ledger @ core-b)"),
            h("  (no rows)"),
        ];
        let page = history_page(&Report::new("history", rows.clone(), EXIT_OK), false).unwrap();
        let lines: Vec<&str> = page.lines().collect();
        assert_eq!(lines[0], "History: portal, newest first");
        assert_eq!(lines[2].split_whitespace().collect::<Vec<_>>(), ["MACHINE", "WHEN", "ACTION", "VERSION", "DIGEST"]);
        assert_eq!(lines[3].split_whitespace().collect::<Vec<_>>(), ["core-a", "2026-11-01T16:17:10Z", "deploy", "v1.4.0", "3c3c3c3c3c3c"]);
        assert!(lines[4].starts_with("core-b") && lines[4].contains("no rows"), "{page}");
        assert!(page.lines().all(|l| l.chars().count() <= 80), "{page}");
        rows.push(h("no reachable on-host ledger: falling back"));
        assert!(history_page(&Report::new("history", rows, EXIT_OK), false).is_none());
    }

    #[test]
    fn promote_status_is_two_tables_and_falls_back_on_anything_else() {
        let n = |d: &str| Check { name: "note".into(), ok: true, detail: d.into(), fix: None };
        let rows = vec![
            n("component  grade  dev  preprod  prod  "),
            n("-----------------------------"),
            n("portal  candidate  305e4d4663cb  305e4d4663cb  3292d9cad0ed"),
            n(""),
            n("open records (1):"),
            n("  portal: ladder opened 2026-10-31 ref \u{2026}943a  (ops/release/records/portal/20261031-305e4d4663cb.yaml)"),
        ];
        let page = promote_page(&Report::new("promote", rows, EXIT_OK), false).unwrap();
        let lines: Vec<&str> = page.lines().collect();
        assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), ["COMPONENT", "GRADE", "DEV", "PREPROD", "PROD"]);
        assert_eq!(lines[1].split_whitespace().count(), 5);
        assert!(lines.iter().any(|l| l.starts_with("Open records: 1,")), "{page}");
        assert!(page.lines().last().unwrap().ends_with("20261031-305e4d4663cb.yaml"), "{page}");
        assert!(!page.contains("note") && !page.contains("---"), "{page}");
        assert!(promote_page(&Report::new("promote", vec![n("something else")], EXIT_OK), false).is_none());
    }

    #[test]
    fn steps_read_as_headings_with_a_plain_refusal() {
        let row = |n: &str, ok, d: &str| Check { name: n.into(), ok, detail: d.into(), fix: None };
        let r = Report::new("fastlane", vec![
            row("step", true, "==> resolving image digest"),
            row("note", true, "not in an open set"),
            row("refused", false, "REFUSE \u{2014} tag sha-1 not found"),
        ], EXIT_REFUSED);
        let page = table(&r, false, true);
        assert_eq!(page, "Finding the image\n  not part of a release in progress\n\n\u{2717} tag sha-1 not found\n\nfastlane: refused");
        assert!(table(&r, false, false).contains("==> resolving"), "a script keeps the rows");
    }

    #[test]
    fn every_check_opens_with_a_capitalized_topic() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let r = Report::new("release", vec![
            row("step", "==> deploy"),
            row("zagros/preflight/state", "State: stateful"),
            row("zagros/preflight", "prep: stateful"),
            row("zagros/transport", "groups [a] via mesh"),
            row("zagros/netvis", "netvis on zagros: nothing was read"),
            row("zagros/netvis/secrets", "netvis on zagros templates 3; NOT SET: A B; put a reference line for each"),
        ], EXIT_OK);
        let page = table(&r, false, true);
        for want in ["\u{2713} Change type: stateful\n", "\u{2713} Reach: a over the mesh\n", "\u{2713} Netvis: on zagros: nothing was read\n", "! Secrets: netvis on zagros needs 3, and 2 aren't set:\n  A\n  B\n"] {
            assert!(page.contains(want), "{want}: {page}");
        }
        assert!(!page.contains("put a reference"), "{page}");
        let green = table(&r, true, true);
        assert!(green.contains("\x1b[1;32m\u{2713} Change type:\x1b[0m stateful"), "{green:?}");
        assert!(green.contains("\x1b[1;33m! Secrets:\x1b[0m"), "{green:?}");
    }

    #[test]
    fn a_release_says_each_thing_once() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let r = Report::new("release", vec![
            row("step", "==> preflight"),
            row("note", "secrets: netvis on zagros_dev templates 3; NOT SET: A B"),
            row("note", "references (pointers, no values) live in goat-credentials"),
            row("step", "==> deploy to dev"),
            row("dev/preflight/state", "State: stateful"),
            row("dev/preflight", "prep: stateful"),
            row("dev/netvis/secrets", "netvis on zagros-dev templates 3; NOT SET: A B; put a reference line"),
            row("dev/playbook", "netvis-deploy.yml"),
            row("dev/netvis", "netvis on zagros_dev: DRY: nothing was read; a real run reads"),
            row("step", "==> deploy to prod"),
            row("prod/preflight", "prep: stateful"),
            row("prod/netvis/secrets", "netvis on zagros-prod templates 3; NOT SET: C; put a reference line"),
            row("prod/playbook", "netvis-deploy.yml"),
        ], EXIT_OK);
        let page = table(&r, false, true);
        assert_eq!(page.matches("Change type:").count(), 1, "{page}");
        assert_eq!(page.matches("Playbook:").count(), 1, "{page}");
        assert_eq!(page.matches("\n  A\n").count(), 1, "{page}");
        assert!(page.contains("\n  C\n"), "a different goatnet's gap still prints: {page}");
        assert!(!page.contains("State:") && !page.contains("references (pointers"), "{page}");
        assert!(!page.contains("nothing was read"), "{page}");
    }

    #[test]
    fn a_persons_page_is_plain_english() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let r = Report::new("release", vec![
            row("step", "==> dev rung: zagros_dev runs v1"),
            row("step", "==> prod rung: graduate v1 and fold it into fleet:"),
            row("zagros/preflight", "prep: stateful \u{b7} rollback: none (undeclared \u{2014} fail closed) \u{b7} single"),
            row("zagros/transport", "groups [a b] via door (/x/door-open.sh zagros)"),
            row("zagros/skipped", "absent: true, topology not skew: delta_hosts"),
            row("step", "==> soak"),
            row("note", "v1 is the preprod candidate. After 1 day(s), with an operator ack:\n  goatbuilder release n --from prod --ack '<name>'  # graduate, fold into fleet:, deploy zagros_prod, verify every rung"),
        ], EXIT_OK);
        let page = table(&r, false, true);
        for gone in ["rung", "soak", "fail closed", "fold", "skew", "candidate", "<name>", "graduate", "\u{2014}"] {
            assert!(!page.to_lowercase().contains(gone), "{gone}: {page}");
        }
        for want in ["Dev stage:", "Prod stage: promote v1", "Change type: stateful \u{b7} no rollback declared \u{b7} single component", "Reach: a b through the door", "Skipped on purpose: delta_hosts", "Wait before prod", "--ack \"$(git config user.name)\""] {
            assert!(page.contains(want), "{want}: {page}");
        }
        assert!(table(&r, false, false).contains("rung"), "a script keeps the words the gates compare");
    }

    #[test]
    fn a_release_dry_run_names_what_a_real_run_refuses() {
        let row = |name: &str, detail: &str| Check { name: name.into(), ok: true, detail: detail.into(), fix: None };
        let page = |extra: Check| {
            let rows = vec![row("step", "==> dev rung: zagros_dev"), row("note", "DRY: goatbuilder pins /tmp/wt"), extra];
            release_page(&Report::new("release", rows, crate::EXIT_OK), false).unwrap()
        };
        for detail in [
            "DRY: CA key for zagros not at /k/ca.key.age \u{2014} a real run signs zagros's intent with it and refuses without it",
            "browser row(s) in [a] need the console's Playwright toolchain here; missing: /x/node_modules",
        ] {
            let p = page(row("note", detail));
            assert!(p.contains("A real run stops on these:") && !p.contains("Dry run passed"), "{p}");
        }
        assert!(page(row("note", "DRY: goatbuilder pins --check /tmp/wt")).contains("Dry run passed"));
    }

    #[test]
    fn a_release_dry_run_is_tables_and_a_refusal_is_not() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let mut rows = vec![
            row("step", "==> component netvis"),
            row("note", "image netvis; rungs dev=a preprod=b prod=c; fleet ref ghcr.io/o/netvis:sha-1@sha256:9f"),
            row("step", "==> preflight"),
            row("note", "secrets: netvis on zagros_dev templates 3 variable(s) from your environment; NOT SET: A_B C"),
            row("step", "==> dev rung: zagros_dev runs v1"),
            row("note", "DRY: git worktree add"),
            row("step", "==> deploy v1 to zagros_dev"),
            row("zagros-dev/preflight", "prep: stateful \u{b7} single"),
            row("zagros-dev/netvis/secrets", "netvis on zagros-dev templates 3; NOT SET: A_B C; put a reference line"),
            row("zagros-dev/transport", "groups [dev_hosts] via mesh (this laptop)"),
            row("zagros-dev/playbook", "netvis-deploy.yml -e target=<group>"),
            row("zagros-dev/dev_hosts/verify", "DRY: would read"),
            row("step", "==> live rows on zagros_dev"),
            row("note", "no live rows are mapped for what v1 changed"),
            row("step", "==> soak"),
            row("note", "v1 is the preprod candidate. After 1 day(s), with an operator ack:"),
            row("note", "goatbuilder release n --from prod --ack '<name>'  # graduate, fold into fleet:, deploy zagros_prod, verify every rung"),
        ];
        let page = table_page_for_test(&rows);
        for want in ["Release: v1 (dry run: nothing changes)", "ITEM", "Current version", "sha-1", "Change type", "STAGE", "GOATNET", "HOSTS", "REACH", "COMMANDS", "LIVE TESTS", "SECRET NOT SET", "Wait before prod", "Set the secrets above before a real run."] {
            assert!(page.contains(want), "{want}: {page}");
        }
        let stage = page.lines().find(|l| l.starts_with("dev ")).unwrap();
        assert_eq!(stage.split_whitespace().collect::<Vec<_>>(), ["dev", "zagros_dev", "dev_hosts", "mesh", "2", "none"]);
        assert_eq!(page.matches("A_B").count(), 1, "{page}");
        for gone in ["rung", "soak", "candidate", "fold", "<name>", "<group>"] {
            assert!(!page.to_lowercase().contains(gone), "{gone}: {page}");
        }
        rows.push(Check { name: "refused".into(), ok: false, detail: "no".into(), fix: None });
        assert!(release_page(&Report::new("release", rows, EXIT_REFUSED), false).is_none());
    }

    #[test]
    fn a_fastlane_dry_run_is_tables_without_would() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let mut rows = vec![
            row("step", "==> component netvis"),
            row("note", "current fleet ref: ghcr.io/{{ ghcr_org }}/netvis:sha-old@sha256:9f"),
            row("note", "ledger component:  netvis"),
            row("step", "==> graduation-set membership"),
            row("note", "not in an open set"),
            row("step", "==> resolving image digest"),
            row("note", "usareur-af: sha256:3c"),
            row("note", "new fleet ref: ghcr.io/{{ ghcr_org }}/netvis:sha-new@sha256:3c"),
            row("step", "==> dry run \u{2014} stopping before any mutation"),
            row("note", "would set components.netvis.fleet = ghcr.io/{{ ghcr_org }}/netvis:sha-new@sha256:3c"),
            row("note", "would clear deviations: <none>\n    would PRESERVE (absent: true \u{2014} topology, not skew): a, b"),
            row("note", "would deploy via netvis-deploy.yml to every goatnet member group"),
        ];
        let page = fastlane_page(&Report::new("fastlane", rows.clone(), EXIT_OK), false).unwrap();
        assert!(!page.to_lowercase().contains("would"), "{page}");
        for want in ["Fastlane: netvis (dry run: nothing changes)", "Current version", "sha-old", "New version", "STEP  WHAT A REAL RUN DOES", "\n1     Sets the fleet version to sha-new.", "\n2     Clears goatnet overrides: none.", "\n3     Keeps skipping these on purpose: a, b.", "\n4     Deploys with netvis-deploy.yml", "Dry run passed. Nothing changed."] {
            assert!(page.contains(want), "{want}: {page}");
        }
        rows.push(Check { name: "refused".into(), ok: false, detail: "no".into(), fix: None });
        assert!(fastlane_page(&Report::new("fastlane", rows, EXIT_REFUSED), false).is_none());
    }

    #[test]
    fn a_pins_mismatch_is_a_table_not_a_diff() {
        let row = |ok, d: &str| Check { name: "pins".into(), ok, detail: d.into(), fix: None };
        let rows = vec![
            row(false, "FAIL: ops/ansible/group_vars/zagros_b_hosts/pins.yml is out of date with ops/pins/fleet-pins.yaml"),
            row(true, "  --- ops/ansible/group_vars/zagros_b_hosts/pins.yml (committed)"),
            row(true, "  +++ ops/ansible/group_vars/zagros_b_hosts/pins.yml (rendered)"),
            row(true, "  @@ -38,7 +38,7 @@"),
            row(true, "       - \"/opt/{{ zenoh_stack }}/docker-compose.yml\""),
            row(true, "   # netbird-mgmt"),
            row(true, "  -netbird_mgmt_image: \"ghcr.io/o/netbird-mgmt@sha256:0d4d12dd727d7f200cef\""),
            row(true, "  +netbird_mgmt_image: \"ghcr.io/o/netbird-mgmt@sha256:ad4d12dd727d7f200cef\""),
            row(true, "Canon: docs/design/environment-parity.md"),
        ];
        let page = pins_page(&Report::new("pins", rows, EXIT_REFUSED), false).unwrap();
        assert!(page.starts_with("\u{2717} Pins: 1 setting differs from the pins."), "{page}");
        let line = page.lines().find(|l| l.starts_with("zagros_b_hosts")).unwrap();
        assert_eq!(line.split_whitespace().collect::<Vec<_>>(), ["zagros_b_hosts/pins.yml", "netbird_mgmt_image", "0d4d12dd727d", "ad4d12dd727d"]);
        assert!(page.contains("Fix: goatbuilder"), "{page}");
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pins_fix(&a(&["goatbuilder", "-C", "/ws/repo", "pins", "--check"])), "goatbuilder -C /ws/repo pins");
        assert_eq!(pins_fix(&a(&["goatbuilder", "pins", "--check"])), "goatbuilder pins");
        let wrote = |d: &str| Check { name: "pins".into(), ok: true, detail: d.into(), fix: None };
        let done = pins_wrote_page(&Report::new("pins", vec![wrote("wrote a/pins.yml"), wrote("wrote b/intent.json (serial 1)")], EXIT_OK), false).unwrap();
        assert!(done.starts_with("\u{2713} Pins: regenerated 1 pins files and 1 intents"), "{done}");
        assert!(!page.contains("@@") && !page.contains("Canon"), "{page}");
        let other = vec![row(true, "ok: fleet pins"), row(false, "FAIL: something else")];
        assert!(pins_page(&Report::new("pins", other, EXIT_REFUSED), false).is_none());
    }

    #[test]
    fn an_explanation_is_labelled_and_inside_the_page() {
        let raw = "  [code]\n  GB-002 \u{2014} unknown goatnet\n    what: the --env value is not a goatnet key the pins declare. The keys use underscores (zagros_dev), not hyphens.\n    why:  typed wrong\n    fix:  goatbuilder deploy --env zagros_dev\n          goatbuilder doctor --all\n    read: docs/design/goatbuilder.md\n    exit: 2\n";
        let page = explanation_page(raw, false);
        assert!(page.starts_with("GB-002: unknown goatnet\n\nFIELD      DETAIL\nWhat       the --env"), "{page}");
        assert!(page.contains("\nCauses     typed wrong\nFix        goatbuilder deploy --env zagros_dev\n           goatbuilder doctor --all\nMore       docs/design/goatbuilder.md\nExit code  2"), "{page}");
        assert!(!page.contains("[code]") && !page.contains('\u{2014}'), "{page}");
        assert!(page.lines().all(|l| l.chars().count() <= 80), "{page}");
        let coloured = explanation_page(raw, true);
        assert!(coloured.contains("\x1b[33mgoatbuilder deploy --env zagros_dev\x1b[0m"), "{coloured:?}");
        assert_eq!(refusal_line("unknown goatnet \"zagros-dev\"; known: [\"zagros\", \"zagros_dev\"]", false), "\u{2717} unknown goatnet \"zagros-dev\". Known: zagros, zagros_dev");
        // Two causes under one label: the second keeps the column (GB-050).
        let two = explanation_page("  GB-050 \u{2014} nothing is declared\n    why:  the key is right but nothing is pinned\n          every component is absent on it\n    exit: 2\n", false);
        assert!(two.contains("\nCauses     the key is right but nothing is pinned\n           every component is absent on it\n"), "{two:?}");
    }

    #[test]
    fn a_plan_is_a_table_with_the_manual_steps_spelled_out() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let mut rows = vec![
            row("banner", "join-node: site=s  node=n  class=b  (DRY RUN \u{2014} nothing executes)"),
            row("phase", "=== Phase 1 [MANUAL] reach the box on the SITE LAN ==="),
            row("cmd", "ssh goat@<address>"),
            row("phase", "=== Phase 2 [T] seed the SSH key (key auth + sudo) ==="),
            row("cmd", "'/long/path/seed-ssh-key.sh' goat@<address>"),
            row("phase", "=== Phase 5 [A] host baseline \u{2014} run detached ==="),
            row("cmd", "cd x && ansible-playbook y"),
            row("footer", "PLAN complete. Remove --dry-run to execute."),
        ];
        let page = plan_page(&Report::new("join-node", rows.clone(), EXIT_OK), false).unwrap();
        assert!(page.starts_with("Plan: join-node, site s, node n, class b\n"), "{page}");
        assert!(page.contains("PHASE  TYPE     STEP\n1      manual   reach the box on the SITE LAN\n2      tool     seed the SSH key\n5      ansible  host baseline\n"), "{page}");
        assert!(page.contains("Nothing has run. This only shows the steps"), "{page}");
        assert!(page.contains("What a real run would leave to you:\n\n  1. reach the box on the SITE LAN:\n\n     ssh goat@<address>\n"), "{page}");
        assert!(!page.contains('\u{2717}'), "a plan owes nothing, so it marks nothing: {page}");
        // Each phase's command, verbatim, under the table; the manual one is spelled out above.
        assert!(page.contains("What each phase runs:\n\n2  seed the SSH key\n    '/long/path/seed-ssh-key.sh' goat@<address>\n\n5  host baseline\n    cd x && ansible-playbook y\n"), "{page}");
        rows[0] = row("banner", "join-node: real run");
        assert!(plan_page(&Report::new("join-node", rows, EXIT_OK), false).is_none());
    }

    /// Real rows the practice copy doesn't produce, so a real run reads the same.
    #[test]
    fn real_rows_get_the_same_pages() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let d = "3c".repeat(32);
        let hist = Report::new("history", vec![
            row("history", "deploy history for netvis (newest first):"),
            row("history", "  (on-host ledger @ h1 \u{2014} source of truth)"),
            row("history", &format!("  2026-11-01T10:00:00Z  [image]  ghcr.io/o/netvis:v2@sha256:{d}  (finished 2026-11-01T10:05:00Z)")),
            row("history", "      action=deploy prior=abc operator=Sam"),
        ], EXIT_OK);
        let page = history_page(&hist, false).unwrap();
        let line = page.lines().find(|l| l.starts_with("h1")).unwrap();
        assert_eq!(line.split_whitespace().collect::<Vec<_>>(), ["h1", "2026-11-01T10:00:00Z", "deploy", "v2", "3c3c3c3c3c3c"]);

        let down = Report::new("teardown", vec![
            row("banner", "teardown: site=s  group=g  (DRY RUN \u{2014} nothing executes)"),
            row("phase", "=== Teardown of phase 14 [A] join the surfaces (relay) ==="),
            row("note", "designed: removing it is a change plus a redeploy"),
            row("phase", "=== Teardown verdict: refused: 15 step(s) cannot come down ==="),
        ], EXIT_OK);
        let page = plan_page(&down, false).unwrap();
        assert!(page.starts_with("Plan: teardown, site s, group g\n"), "{page}");
        assert!(page.contains("14     ansible  designed  join the surfaces"), "{page}");
        assert!(page.contains("Result: refused: 15 steps cannot come down"), "{page}");

        let up = Report::new("standup", vec![
            row("banner", "standup: site=s  group=g  profile=internet  obs=none \u{2014} no obs host  (DRY RUN \u{2014} nothing executes)"),
            row("phase", "=== Phase 1 [GATE] config preflight (fail-closed) ==="),
            row("note", "needs: a key"),
            row("cmd", "python3 x.py"),
        ], EXIT_OK);
        let page = plan_page(&up, false).unwrap();
        assert!(page.starts_with("Plan: standup, site s, group g, profile internet, obs none\n"), "{page}");
        assert!(page.contains("What each phase needs:\n1  a key"), "{page}");

        let dep = Report::new("deploy", vec![
            row("zagros-dev/preflight", "prep: stateful \u{b7} single"),
            row("zagros-dev/transport", "groups [h] via mesh (x)"),
            row("zagros-dev/h/deploy", "DRY: deploying x"),
            row("preprod/portal/plan", "DRY: would publish v1 as serial 1"),
            row("preprod/core-a/portal", &format!("converged on ghcr.io/o/portal:v2@sha256:{d} after 2 observations")),
        ], EXIT_OK);
        let page = table(&dep, false, true);
        assert!(page.contains("\u{2713} Change type: stateful"), "{page}");
        assert!(page.contains("\u{2713} Reach: h over the mesh"), "{page}");
        assert!(page.contains("real run runs, as the plan describes it:\n    deploying x\n"), "{page}");
        // Not a tick: nothing was published. A statement about a run that has
        // not happened reads as a statement.
        assert!(page.contains("\u{b7} Plan: A real run will publish v1"), "{page}");
        assert!(!page.contains("\u{2713} Plan:"), "{page}");
        assert!(page.lines().any(|l| l.contains("core-a") && l.contains("converged") && l.contains("v2")), "{page}");
        assert!(!page.contains("STATUS"), "{page}");
    }

    #[test]
    fn a_count_reads_as_a_count() {
        assert_eq!(plain("observe 5 machine(s): a b"), "observe 5 machines: a b");
        assert_eq!(plain("observe 1 machine(s)"), "observe 1 machine");
        assert_eq!(plain("2 goatnet(s), 1 component(s)"), "2 goatnets, 1 component");
        assert_eq!(plain("3 check(s) and 1 set(s)"), "3 checks and 1 set");
        assert_eq!(plain("4 entry(s)"), "4 entries");
        // The count and the noun are not always neighbours.
        assert_eq!(plain("only 0 earlier ref(s) in the history"), "only 0 earlier refs in the history");
        assert_eq!(plain("1 open graduation set(s)"), "1 open graduation set");
        // Not a count: left alone.
        assert_eq!(plain("machine(s) unnamed"), "machine(s) unnamed");
    }

    #[test]
    fn a_manual_step_has_its_command_apart_from_its_note() {
        let (c, n) = manual_parts("ssh goat@<the node's LAN address> \u{2014} the address handed over may be 192.168.55.1: that is l4tbr0, the Jetson USB device-mode bridge on the box itself, reachable only over a direct USB-C cable. Find the real LAN address before anything else, then pass it as --address.");
        assert_eq!(c.as_deref(), Some("ssh goat@<the node's LAN address>"));
        assert!(n.starts_with("Use the box's real LAN address") && !n.contains("l4tbr0"), "{n}");
        let (c, n) = manual_parts("in the NetBird dashboard of zagros-dev (or `goat admin` on it): Setup Keys \u{2192} new key, auto_groups = the group a live mgmt host of zagros-dev holds; keep it for phase 10");
        assert!(c.is_none() && n.contains("make a new key") && n.contains("Keep the key for phase 10."), "{n}");
        let (c, n) = manual_parts("on the node: sudo netbird up <the line phase 9 printed> --enable-rosenpass=false (match the goatnet's declared PQC posture; never also pass --rosenpass-permissive=false). Gate: `netbird status` shows Interface type: Kernel AND `sudo nft list tables` shows `table ip netbird`: Connected alone is an unfiltered node");
        assert_eq!(c.as_deref(), Some("sudo netbird up <the line phase 9 printed> --enable-rosenpass=false"));
        assert!(n.starts_with("Match the goatnet's declared PQC posture"), "the posture clause must survive: {n}");
        assert!(n.contains("It is done when netbird status") && n.contains("not enough"), "{n}");
        let page = hand_step(1, "reach the box", Some("ssh x"), "a note", true);
        assert!(page.contains("1. \x1b[1mreach the box\x1b[0m:") && page.contains("\x1b[1m     ssh x\x1b[0m"), "{page:?}");
    }

    #[test]
    fn a_standup_that_converged_is_one_table() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let d = "3c".repeat(32);
        let mut rows = vec![
            row("phase", "=== Phase 5 [core] portal ==="),
            row("g/a/core/portal/publish", &format!("serial 1: ghcr.io/o/portal:v1.4.0@sha256:{d} by contributora")),
            row("g/a/core/a-core-1/portal", &format!("converged on ghcr.io/o/portal:v1.4.0@sha256:{d} after 2 observations")),
            row("measured", "declared \u{2192} verified: 1 components on 1 machines in 2 ms \u{2014} every record names its declared ref"),
        ];
        let page = standup_page(&Report::new("standup", rows.clone(), EXIT_OK), false).unwrap();
        assert!(page.starts_with("Standup: g, site a\nAcknowledged by: contributora\n\nPHASE"), "{page}");
        // The same word about the same machine is the same colour as in `status`.
        let green = standup_page(&Report::new("standup", rows.clone(), EXIT_OK), true).unwrap();
        assert!(green.contains("\x1b[32mconverged\x1b[0m"), "{green:?}");
        let line = page.lines().find(|l| l.starts_with("5 ")).unwrap();
        assert_eq!(line.split_whitespace().collect::<Vec<_>>(), ["5", "core", "portal", "a-core-1", "v1.4.0", "converged"]);
        assert!(page.ends_with("1 components on 1 machines, all running what main declares. Took 2 ms."), "{page}");
        assert!(!page.contains("observation") && !page.contains("serial") && !page.contains("publish"), "{page}");
        // A manual step keeps the grouped page.
        rows.insert(1, row("cmd", "ssh somewhere"));
        assert!(standup_page(&Report::new("standup", rows.clone(), EXIT_OK), false).is_none());
        rows.remove(1);
        rows[2].ok = false;
        assert!(standup_page(&Report::new("standup", rows, EXIT_REFUSED), false).is_none());
    }

    #[test]
    fn a_real_run_reads_as_a_sentence() {
        assert_eq!(
            real_run_line("netvis: a real run puts ghcr.io/o/netvis:v1 on dev, zagros and prod and reads every ledger"),
            "A real run puts netvis v1 on dev, zagros and prod, then checks that every machine recorded it."
        );
    }

    fn table_page_for_test(rows: &[Check]) -> String {
        release_page(&Report::new("release", rows.to_vec(), EXIT_OK), false).unwrap()
    }

    #[test]
    fn machines_are_never_merged_and_a_failure_names_its_machine() {
        let row = |n: &str, ok: bool, d: &str| Check { name: n.into(), ok, detail: d.into(), fix: None };
        let d = "3c".repeat(32);
        let conv = format!("converged on ghcr.io/o/portal:v2@sha256:{d}");
        let r = Report::new("deploy", vec![
            row("dev/core-a/portal", true, &conv),
            row("dev/core-b/portal", true, &conv),
            row("dev/edge-a/portal", false, "unknown: record unreadable"),
        ], EXIT_NOT_CONVERGED);
        let page = table(&r, false, true);
        // The machines are a table now, and no row is merged away.
        assert!(page.lines().any(|l| l.contains("core-a") && l.contains("converged")), "{page}");
        assert!(page.lines().any(|l| l.contains("core-b") && l.contains("converged")), "a machine was merged away:\n{page}");
        assert!(page.contains("\u{2717} edge-a/portal: unknown: record unreadable"), "a failure must name its machine:\n{page}");
    }

    /// A long command with quotes, a dash and runs of spaces, as standup's
    /// phase 4 prints one, comes out byte for byte on one line, so the line
    /// pastes as the command the phase runs.
    #[test]
    fn a_phase_command_prints_verbatim_on_one_line() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let cmd = "if grep -qE 'name_snapshot_enabled:[[:space:]]*true' '/srv/repo/ops/ansible/group_vars/alpha_hosts/vars.yml' 2>/dev/null; then      python3 '/srv/repo/ops/dns/generate-mesh-hosts.py';      else echo 'name_snapshot_enabled not true for alpha_hosts \u{2014} skipping (ADR 1082)'; fi";
        let r = Report::new("standup", vec![row("phase", "=== Phase 4 [T] name-fallback snapshot (generate + sign) ==="), row("cmd", cmd)], EXIT_OK);
        let page = phased(&r, false);
        assert!(page.contains(&format!("\n  {cmd}\n")), "{page}");
    }

    #[test]
    fn a_dry_runs_commands_are_listed_as_they_run() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let r = Report::new("release", vec![row("step", "==> dev rung"), row("note", "DRY: git worktree add"), row("note", "DRY: go run")], EXIT_OK);
        let page = table(&r, false, true);
        assert!(page.contains("A real run runs, as the plan describes it:\n    git worktree add\n    go run\n"), "{page}");
    }

    #[test]
    fn rows_with_one_name_print_as_lines_marking_failures() {
        let row = |ok, d: &str| Check { name: "pins".into(), ok, detail: d.into(), fix: None };
        let r = Report::new("pins", vec![row(false, "pins.yml is out of date"), row(true, "  --- a")], EXIT_REFUSED);
        let page = table(&r, false, true);
        assert!(page.starts_with("\u{2717} pins.yml is out of date\n  --- a"), "{page}");
        assert!(!page.contains("ok    pins"), "{page}");
    }

    #[test]
    fn phases_group_rows_by_machine_with_versions() {
        let row = |n: &str, d: &str| Check { name: n.into(), ok: true, detail: d.into(), fix: None };
        let d = "3c".repeat(32);
        let r = Report::new("standup", vec![
            row("phase", "=== Phase 5 [core] portal ==="),
            row("g/a/core/portal/publish", &format!("serial 1: ghcr.io/o/portal:v1.4.0@sha256:{d}")),
            row("g/a/core/a-core-1/portal", &format!("converged on ghcr.io/o/portal:v1.4.0@sha256:{d}")),
        ], EXIT_OK);
        let page = table(&r, false, true);
        assert!(page.starts_with("Phase 5 \u{b7} core \u{b7} portal\n  \u{2713} publish  serial 1: v1.4.0\n  \u{2713} a-core-1  converged on v1.4.0"), "{page}");
        assert!(page.ends_with("standup: ok"), "{page}");
    }

    #[test]
    fn a_narrative_has_no_column_titles() {
        let steps = Report::new("fastlane", vec![c("note", true, None), c("phase", true, None)], EXIT_OK);
        assert!(!table(&steps, false, true).contains("STATUS"));
        let same = Report::new("history", vec![c("history", true, None), c("history", true, None)], EXIT_OK);
        assert!(!table(&same, false, true).contains("STATUS"));
    }

    #[test]
    fn json_carries_its_schema_version_verb_and_exit() {
        let r = Report::new("deploy", vec![c("a", true, None), c("b", false, Some("do x"))], EXIT_REFUSED);
        let v: serde_json::Value = serde_json::from_str(&render(&r, OutputFormat::Json, ColorChoice::Never)).unwrap();
        assert_eq!(v["schema"], "goatbuilder.output/v1");
        assert_eq!(v["verb"], "deploy");
        assert_eq!(v["ok"], false);
        assert_eq!(v["exit"], 1);
        assert_eq!(v["checks"][1]["fix"], "do x");
        assert!(v["checks"][0].get("fix").is_none(), "an absent fix is omitted, not null");
    }

    #[test]
    fn yaml_round_trips_the_same_document() {
        let r = Report::new("status", vec![c("x", true, None)], EXIT_NOT_CONVERGED);
        let v: serde_yaml::Value = serde_yaml::from_str(&render(&r, OutputFormat::Yaml, ColorChoice::Never)).unwrap();
        assert_eq!(v["schema"].as_str(), Some("goatbuilder.output/v1"));
        assert_eq!(v["exit"].as_i64(), Some(0));
    }

    #[test]
    fn the_table_ends_with_a_verdict_and_prints_each_fix_under_its_check() {
        let r = Report::new("deploy", vec![c("alpha", true, None), c("b", false, Some("run y"))], EXIT_REFUSED);
        let t = render(&r, OutputFormat::Table, ColorChoice::Never);
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines[0].starts_with("ok    alpha"), "{t}");
        assert!(lines[1].starts_with("FAIL  b    "), "names are padded to one width: {t}");
        assert!(lines[2].trim_start().starts_with("fix: run y"), "{t}");
        assert_eq!(*lines.last().unwrap(), "deploy: refused (2 checks)");
    }

    #[test]
    fn the_verdict_word_follows_the_exit_code() {
        let ok = Report::new("v", vec![c("a", true, None)], EXIT_REFUSED);
        assert!(render(&ok, OutputFormat::Table, ColorChoice::Never).ends_with("v: ok (1 check)"));
        let nc = Report::new("v", vec![c("a", false, None)], EXIT_NOT_CONVERGED);
        assert!(render(&nc, OutputFormat::Table, ColorChoice::Never).ends_with("v: not converged (1 check)"));
        let rf = Report::new("v", vec![c("a", false, None)], EXIT_REFUSED);
        assert!(render(&rf, OutputFormat::Table, ColorChoice::Never).ends_with("v: refused (1 check)"));
    }

    #[test]
    fn an_empty_report_still_renders_a_verdict() {
        let r = Report::new("v", vec![], EXIT_REFUSED);
        assert_eq!(render(&r, OutputFormat::Table, ColorChoice::Never), "v: ok (0 checks)");
    }

    #[test]
    fn color_wraps_only_the_glyph_and_the_verdict_and_never_appears_with_never() {
        let r = Report::new("deploy", vec![c("a", true, None), c("b", false, None)], EXIT_REFUSED);
        let plain = render(&r, OutputFormat::Table, ColorChoice::Never);
        assert!(!plain.contains('\x1b'), "never means no escapes: {plain:?}");

        let painted = render(&r, OutputFormat::Table, ColorChoice::Always);
        // The ok glyph is green, the FAIL glyph and the refused verdict are red.
        assert!(painted.contains("\x1b[32mok  \x1b[0m"), "{painted:?}");
        assert!(painted.contains("\x1b[31mFAIL\x1b[0m"), "{painted:?}");
        assert!(painted.contains("\x1b[31mrefused\x1b[0m"), "{painted:?}");
        // Detail text is never colored, so a colored table is still greppable.
        assert!(painted.contains("a detail") && painted.contains("b detail"));
        // The uncolored content is byte-identical once the escapes are stripped.
        let stripped: String = strip_ansi(&painted);
        assert_eq!(stripped, plain);
    }

    #[test]
    fn not_converged_is_yellow() {
        let r = Report::new("status", vec![c("a", false, None)], EXIT_NOT_CONVERGED);
        let painted = render(&r, OutputFormat::Table, ColorChoice::Always);
        assert!(painted.contains("\x1b[33mnot converged\x1b[0m"), "{painted:?}");
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}

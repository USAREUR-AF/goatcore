//! `goatbuilder explain <verb> [<phase>]`: a phase answered from the phase
//! catalog (ops/phases/catalog.yaml, crates/goat-errors/src/phases.rs) as
//! `explain <code>` answers an error from the error catalog
//! (docs/design/goatbuilder-legibility.md, step 6; gate row F4).
//!
//!   - `explain federate 8` or `explain federate mint`: that phase's entry, by
//!     number or id: what it does, why, what it changes, how to tell it
//!     worked, what it runs, the command by hand, what to do when it failed,
//!     and the document to read.
//!   - `explain federate`: the verb's phases, one line each.
//!   - A phase the verb does not have is refused (GB-262), naming the phases
//!     it has.
//!
//! `-o json` and `-o yaml` give the catalog's own fields. A first word that is
//! not a verb of the phase catalog is left to the error interpretation, as
//! before.

use goat_errors::phases::{phases, Phase};
use serde::Serialize;

use crate::OutputFormat;

/// The catalog code of a phase the verb does not have.
pub const NO_SUCH_PHASE: &str = "GB-262";

/// One line of a verb's phase list, as `-o json` gives it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Row<'a> {
    pub phase: String,
    pub id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub stage: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
}

#[derive(Serialize)]
struct List<'a> {
    verb: &'a str,
    phases: Vec<Row<'a>>,
}

/// What `explain <verb> [<phase>]` prints, or the refusal with its exit.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Printed on stdout; exit 0.
    Out(String),
    /// A refusal: the message, and the exit the catalog gives its code.
    Refused(String),
}

/// `input` names a verb of the phase catalog, and at most a phase: a second
/// word that is one of the verb's phases (by number or id), or all digits (a
/// phase number, refused when the verb has none by it). Any other second word
/// makes the input error text (`federate refused`), left to the
/// interpretation as before.
pub fn is_phase_query(input: &[String]) -> bool {
    let verb_ok = |v: &str| phases().verbs().contains(v);
    match input {
        [verb] => verb_ok(verb),
        [verb, phase] => {
            verb_ok(verb)
                && (goat_errors::phases::explain_phase(verb, phase).is_some()
                    || (!phase.is_empty() && phase.bytes().all(|b| b.is_ascii_digit())))
        }
        _ => false,
    }
}

/// `[text](target)` as `text (target)` for a terminal; a target that is an
/// anchor alone (`#x`) is in the phase's document, `read`.
fn terminal_links(text: &str, read: &str) -> String {
    let doc = read.split('#').next().unwrap_or(read);
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|close| {
            let target = &after[close + 2..];
            target
                .find(')')
                .map(|end| (&after[..close], &target[..end], close + 2 + end + 1))
        });
        match link {
            Some((label, target, used)) if !label.contains('[') && !target.contains(' ') => {
                out.push_str(&rest[..open]);
                let target = match target.strip_prefix('#') {
                    Some(anchor) => format!("{doc}#{anchor}"),
                    None => target.to_string(),
                };
                out.push_str(&format!("{label} ({target})"));
                rest = &after[used..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One phase's entry for a terminal: the catalog's text, its header naming the
/// phase's id and stage, and its links as text and target.
fn phase_text(p: &Phase) -> String {
    let body = goat_errors::phases::render_text(p);
    let rest = body.split_once('\n').map(|(_, r)| r).unwrap_or("");
    let stage = if p.stage.is_empty() {
        String::new()
    } else {
        format!(" ({})", p.stage)
    };
    let header = format!(
        "{} phase {} {}{stage} [{}]: {}",
        p.verb, p.phase, p.id, p.kind, p.title
    );
    format!("{header}\n{}", terminal_links(rest, &p.read))
}

fn rows(verb: &str) -> Vec<Row<'_>> {
    phases()
        .of(verb)
        .into_iter()
        .map(|p| Row {
            phase: p.phase.to_string(),
            id: &p.id,
            stage: &p.stage,
            kind: &p.kind,
            title: &p.title,
        })
        .collect()
}

fn list_text(verb: &str) -> String {
    let rows = rows(verb);
    let wid = rows.iter().map(|r| r.id.len()).max().unwrap_or(2);
    let wst = rows.iter().map(|r| r.stage.len()).max().unwrap_or(0);
    let wk = rows.iter().map(|r| r.kind.len()).max().unwrap_or(4);
    let mut s = format!(
        "{verb}: {} phases. goatbuilder explain {verb} <phase> answers for one, by number or id.\n\n",
        rows.len()
    );
    for r in &rows {
        // A step with no number (standup's image pull, its observability legs)
        // is named by its id, so its number column is left blank.
        let number = if r.phase.parse::<u32>().is_ok() {
            r.phase.as_str()
        } else {
            ""
        };
        s.push_str(
            format!(
                "{:>3}  {:<wid$}  {:<wst$}  {:<wk$}  {}",
                number, r.id, r.stage, r.kind, r.title
            )
            .trim_end(),
        );
        s.push('\n');
    }
    s
}

fn serialize<T: Serialize>(v: &T, format: OutputFormat) -> String {
    match format {
        OutputFormat::Yaml => serde_yaml::to_string(v).unwrap_or_default(),
        _ => format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default()),
    }
}

/// The answer for `verb` (a verb of the phase catalog) and `phase`.
pub fn answer(verb: &str, phase: Option<&str>, format: OutputFormat) -> Answer {
    let Some(phase) = phase else {
        return Answer::Out(match format {
            OutputFormat::Table => list_text(verb),
            _ => serialize(
                &List {
                    verb,
                    phases: rows(verb),
                },
                format,
            ),
        });
    };
    match goat_errors::phases::explain_phase(verb, phase) {
        Some(p) => Answer::Out(match format {
            OutputFormat::Table => phase_text(p),
            _ => serialize::<Phase>(p, format),
        }),
        None => {
            let known: Vec<String> = phases()
                .of(verb)
                .iter()
                .map(|p| format!("{} {}", p.phase, p.id))
                .collect();
            Answer::Refused(format!(
                "{verb} has no phase `{phase}`: its phases are {} (by number or id). goatbuilder explain {verb} lists them",
                known.join(", ")
            ))
        }
    }
}

/// The shape of a refusal under `-o json|yaml`, as main prints every coded
/// refusal (`goatbuilder.error/v1`).
#[derive(Serialize)]
struct ErrorOut<'a> {
    schema: &'static str,
    error: &'a str,
    code: &'a str,
    interpretation: &'a goat_errors::Interpretation,
}

/// `explain`'s phase half: `Some(exit)` when `input` asks about a phase (the
/// answer printed), `None` when it is for the error interpretation.
/// `--report` writes a report of an error the interpretation read, so it is
/// refused with a phase question rather than silently ignored.
pub fn dispatch(input: &[String], report: bool, format: OutputFormat) -> Option<i32> {
    if !is_phase_query(input) {
        return None;
    }
    if report {
        eprintln!(
            "goatbuilder: --report writes a report of an error explain interpreted; `explain {}` answers from the phase catalog and has none to write. Run it without --report",
            input.join(" ")
        );
        // Two requests that exclude each other: the command line's own code.
        eprintln!("  (GB-242 — goatbuilder explain GB-242)");
        return Some(2);
    }
    let exit = match answer(&input[0], input.get(1).map(String::as_str), format) {
        Answer::Out(text) => {
            print!("{text}");
            0
        }
        Answer::Refused(text) => {
            eprintln!("goatbuilder: {text}");
            match format {
                OutputFormat::Table => {
                    eprintln!("  ({NO_SUCH_PHASE} — goatbuilder explain {NO_SUCH_PHASE})")
                }
                _ => {
                    let interp = goat_errors::interpret_coded(NO_SUCH_PHASE, &text);
                    let doc = ErrorOut {
                        schema: "goatbuilder.error/v1",
                        error: &text,
                        code: NO_SUCH_PHASE,
                        interpretation: &interp,
                    };
                    print!("{}", serialize(&doc, format));
                }
            }
            goat_errors::explain(NO_SUCH_PHASE)
                .and_then(|e| e.exit)
                .unwrap_or(2)
        }
    };
    Some(exit)
}

/// What a failed phase's row carries (gate row F5; legibility step 7): the
/// error catalog's code, cause and fix when the phase's output names a known
/// error, else the phase catalog's own "if it failed" guidance; the log of
/// its output; and the command that runs that phase by hand.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailedPhase {
    pub verb: String,
    /// The phase's key in the phase catalog: a number (`12`) or a name
    /// (`image-pull`), as the catalog writes it.
    pub phase: goat_errors::phases::PhaseKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Other catalogued errors the output also names (a wrapper's words, an
    /// earlier warning), for the person reading.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub cause: String,
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    pub by_hand: String,
    pub read: String,
}

/// The output a failed phase's interpretation reads: at most this much of
/// its end, where the error that stopped it is.
pub const TAIL_BYTES: usize = 64 * 1024;

/// Interpret the output of `verb`'s failed `phase`, the key the runner recorded
/// it under: a phase number (`"12"`) or a step's name (`"image-pull"`,
/// `"observability-leg-host"`). `None` when the phase catalog has no such phase.
pub fn failed_phase(
    verb: &str,
    phase: &str,
    output: &str,
    log: Option<&str>,
) -> Option<FailedPhase> {
    let p = goat_errors::phases::explain_phase(verb, phase)?;
    let phase = p.phase.clone();
    // The end of the output, cut at a character boundary.
    let mut start = output.len().saturating_sub(TAIL_BYTES);
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let tail = &output[start..];
    // A code the output carries, or a pattern of the catalog it matches;
    // never a lexical guess. The one nearest the end of the output is what
    // stopped the phase: a later line, or, on one line, the innermost tool's
    // words that a wrapper quotes (ansible's UNREACHABLE carrying ssh's
    // "Permission denied").
    let mut hits: Vec<goat_errors::Hit> = goat_errors::interpret(tail)
        .hits
        .into_iter()
        .filter(|h| {
            matches!(
                h.layer,
                goat_errors::Layer::Code | goat_errors::Layer::Pattern
            )
        })
        .collect();
    hits.sort_by_key(|h| h.at);
    let hit = hits.pop();
    let also: Vec<String> = hits.iter().rev().map(|h| h.entry.code.clone()).collect();
    Some(match hit {
        Some(h) => FailedPhase {
            verb: verb.to_string(),
            phase,
            also,
            code: Some(h.entry.code.clone()),
            title: Some(h.entry.title.clone()),
            cause: h
                .entry
                .causes
                .first()
                .cloned()
                .unwrap_or_else(|| h.entry.what.clone()),
            fix: h.entry.fixes.join("; "),
            log: log.map(str::to_string),
            by_hand: p.by_hand.clone(),
            read: if h.entry.read.is_empty() {
                p.read.clone()
            } else {
                h.entry.read.clone()
            },
        },
        None => FailedPhase {
            verb: verb.to_string(),
            phase,
            also,
            code: None,
            title: None,
            cause: p.failed_because.clone(),
            fix: p.then.clone(),
            log: log.map(str::to_string),
            by_hand: p.by_hand.clone(),
            read: p.read.clone(),
        },
    })
}

impl FailedPhase {
    /// The row's text, after the phase's own failure line: what the output
    /// says went wrong, what to do, where the output is, and how to run this
    /// phase again by hand.
    pub fn detail(&self) -> String {
        let mut s = match (&self.code, &self.title) {
            (Some(c), Some(t)) => format!("{c}: {t}. Likely: {}. Fix: {}", self.cause, self.fix),
            _ => format!(
                "No known error in its output. This phase fails when {}; then {}",
                self.cause, self.fix
            ),
        };
        if let Some(l) = &self.log {
            s.push_str(&format!(". Its output: {l}"));
        }
        s.push_str(&format!(". By hand: {}. Read: {}", self.by_hand, self.read));
        if !self.also.is_empty() {
            s.push_str(&format!(". The output also names {}", self.also.join(", ")));
        }
        match &self.code {
            Some(c) => s.push_str(&format!(" (goatbuilder explain {c})")),
            None => s.push_str(&format!(
                " (goatbuilder explain {} {})",
                self.verb, self.phase
            )),
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn out(a: Answer) -> String {
        match a {
            Answer::Out(t) => t,
            Answer::Refused(r) => panic!("refused: {r}"),
        }
    }

    /// A verb of the phase catalog, and at most a phase, is a phase question;
    /// anything else stays with the error interpretation.
    /// A second word is a phase only when it resolves to one, or is all
    /// digits (a phase number, refused if the verb has no such phase);
    /// anything else is error text, interpreted as before.
    #[test]
    fn error_text_after_a_catalog_verb_is_still_interpreted() {
        assert!(!is_phase_query(&s(&["federate", "refused"])));
        assert!(!is_phase_query(&s(&["federate", "failed:"])));
        assert!(
            is_phase_query(&s(&["federate", "99"])),
            "a number is a phase question"
        );
        assert!(
            is_phase_query(&s(&["federate", "acceptance"])),
            "an id is a phase question"
        );
    }

    /// The terminal shows a markdown link as its text and its target; an
    /// anchor alone points into the phase's document.
    #[test]
    fn a_markdown_link_is_shown_as_text_and_target() {
        let read = "docs/operations/sovereign-federation-runbook.md#step-4";
        assert_eq!(
            terminal_links("inspect it ([phase 8 reruns](#phase-8-reruns)); then", read),
            "inspect it (phase 8 reruns (docs/operations/sovereign-federation-runbook.md#phase-8-reruns)); then"
        );
        assert_eq!(
            terminal_links("see [the design](docs/design/x.md#y) now", read),
            "see the design (docs/design/x.md#y) now"
        );
        assert_eq!(
            terminal_links("no link [here] or (there)", read),
            "no link [here] or (there)"
        );
        let t = out(answer("federate", Some("8"), OutputFormat::Table));
        assert!(
            !t.contains("]("),
            "a raw markdown link reached the terminal: {t}"
        );
    }

    #[test]
    fn only_a_catalog_verb_and_at_most_a_phase_is_a_phase_question() {
        assert!(is_phase_query(&s(&["federate"])));
        assert!(is_phase_query(&s(&["federate", "8"])));
        assert!(is_phase_query(&s(&["federate", "mint"])));
        assert!(!is_phase_query(&s(&["GB-074"])));
        assert!(!is_phase_query(&s(&["federate", "8", "extra"])));
        assert!(
            !is_phase_query(&s(&["deploy", "1"])),
            "deploy has no catalog entries yet"
        );
        assert!(!is_phase_query(&s(&[])));
    }

    /// The terminal answer for one phase is the catalog's entry, every
    /// answer it holds, pinned for phase 8.
    #[test]
    fn one_phase_on_a_terminal_is_its_catalog_entry() {
        let t = out(answer("federate", Some("8"), OutputFormat::Table));
        let p = goat_errors::phases::explain_phase("federate", "8").unwrap();
        assert!(
            t.starts_with(&format!(
                "federate phase 8 mint (plan) [{}]: {}\n",
                p.kind, p.title
            )),
            "{t}"
        );
        for (label, value) in [
            ("What it does", &p.what),
            ("Why it exists", &p.why),
            ("What it changes", &p.changes),
            ("How to tell it worked", &p.verify),
            ("It runs", &p.runs),
            ("By hand", &p.by_hand),
            ("Read", &p.read),
        ] {
            assert!(
                t.contains(&format!(
                    "\n{label}:\n  {}\n",
                    terminal_links(value.trim(), &p.read)
                )),
                "{label}\n{t}"
            );
        }
        // By id, the same answer.
        assert_eq!(out(answer("federate", Some(&p.id), OutputFormat::Table)), t);
    }

    /// `-o json` gives the catalog's own fields for the phase.
    #[test]
    fn one_phase_as_json_is_the_catalog_entry() {
        let j = out(answer("federate", Some("1"), OutputFormat::Json));
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["verb"], "federate");
        assert_eq!(v["phase"], 1);
        assert_eq!(v["id"], "preflight");
        for k in [
            "title", "what", "why", "changes", "verify", "runs", "by_hand", "read",
        ] {
            assert!(v[k].as_str().is_some_and(|x| !x.is_empty()), "{k}: {v}");
        }
        let y = out(answer("federate", Some("1"), OutputFormat::Yaml));
        let yv: serde_json::Value = serde_yaml::from_str(&y).unwrap();
        assert_eq!(yv, v);
    }

    /// No phase: the verb's phases, one line each, in order.
    #[test]
    fn no_phase_lists_the_verb_s_phases() {
        let t = out(answer("federate", None, OutputFormat::Table));
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(
            lines[0],
            "federate: 14 phases. goatbuilder explain federate <phase> answers for one, by number or id."
        );
        let body: Vec<&str> = lines[2..].to_vec();
        assert_eq!(body.len(), 14, "{t}");
        let first = goat_errors::phases::explain_phase("federate", "1").unwrap();
        assert!(
            body[0].trim_start().starts_with("1  preflight"),
            "{}",
            body[0]
        );
        assert!(body[0].ends_with(&first.title), "{}", body[0]);
        assert!(body[13].trim_start().starts_with("14"), "{}", body[13]);
        let j = out(answer("federate", None, OutputFormat::Json));
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["verb"], "federate");
        let ph = v["phases"].as_array().unwrap();
        assert_eq!(ph.len(), 14);
        assert_eq!(ph[0]["phase"], "1");
        assert_eq!(ph[0]["id"], "preflight");
        assert_eq!(ph[7]["id"], "mint");
    }

    /// A phase the verb does not have is refused, naming the ones it has, with
    /// a catalog code that exists.
    #[test]
    fn an_unknown_phase_is_refused_naming_the_phases() {
        for p in ["99", "nonesuch", "0"] {
            let Answer::Refused(r) = answer("federate", Some(p), OutputFormat::Table) else {
                panic!("{p} was answered");
            };
            assert!(r.contains(&format!("federate has no phase `{p}`")), "{r}");
            assert!(
                r.contains("1 preflight") && r.contains("8 mint") && r.contains("14 "),
                "{r}"
            );
        }
        let e = goat_errors::explain(NO_SUCH_PHASE).expect("the refusal's code is in the catalog");
        assert_eq!(e.tool, "goatbuilder");
    }

    /// F5: a phase whose output names a known error carries that error's code,
    /// cause and fix, the log of its output, and its command by hand.
    #[test]
    fn a_failed_phase_s_known_error_carries_its_code_and_fix() {
        let out = "PLAY [edge] ****\nTASK [goat-zenoh-edge : deploy] ****\nfatal: [edge-1]: UNREACHABLE! => {\"msg\": \"Failed to connect to the host via ssh: goat@192.0.2.10: Permission denied (publickey).\"}\n";
        let f = failed_phase(
            "federate",
            "12",
            out,
            Some("/var/lib/goat/runs/r1/logs/012-1.log"),
        )
        .unwrap();
        assert_eq!(f.code.as_deref(), Some("X-SSH-001"), "{f:?}");
        let e = goat_errors::explain("X-SSH-001").unwrap();
        assert_eq!(f.title.as_deref(), Some(e.title.as_str()));
        assert_eq!(f.cause, e.causes[0]);
        assert!(f.fix.contains(&e.fixes[0]), "{f:?}");
        assert_eq!(
            f.log.as_deref(),
            Some("/var/lib/goat/runs/r1/logs/012-1.log")
        );
        let p = goat_errors::phases::explain_phase("federate", "12").unwrap();
        assert_eq!(f.by_hand, p.by_hand);
        assert_eq!(
            f.also,
            ["X-ANSIBLE-001"],
            "the wrapper's words are named too"
        );
        let d = f.detail();
        for part in [
            "X-SSH-001",
            e.title.as_str(),
            "012-1.log",
            "By hand:",
            "goatbuilder explain X-SSH-001",
        ] {
            assert!(d.contains(part), "{part}\n{d}");
        }
    }

    /// F5 on phase 11: the peering-wg read-back that found the interface without
    /// the agreed peer and address is its own error (GB-271), not X-ANSIBLE-003's
    /// "a secret the play templates was empty". The fixture is the play's real
    /// output from a kill-and-resume run.
    #[test]
    fn the_peering_wg_interface_read_back_names_the_link_not_a_secret() {
        let out = include_str!("../tests/fixtures/failed-phase/peering-wg-interface-readback.log");
        let f = failed_phase("federate", "11", out, None).unwrap();
        assert_eq!(f.code.as_deref(), Some("GB-271"), "{f:?}");
        let e = goat_errors::explain("GB-271").unwrap();
        assert_eq!(f.cause, e.causes[0]);
        assert!(!f.cause.contains("secret"), "{f:?}");
        assert!(f.fix.contains("systemctl restart goat-peering-wg@"), "{f:?}");
        assert!(f.fix.contains("--from 11"), "{f:?}");
        let d = f.detail();
        assert!(d.contains("goatbuilder explain GB-271"), "{d}");
        assert!(!d.contains("GB-007"), "{d}");
    }

    /// Output that names no known error gets the phase catalog's own guidance,
    /// never a lexical guess.
    #[test]
    fn a_failed_phase_with_no_known_error_gets_the_phase_s_own_guidance() {
        let f = failed_phase("federate", "12", "something odd happened\nexit 2\n", None).unwrap();
        let p = goat_errors::phases::explain_phase("federate", "12").unwrap();
        assert_eq!(f.code, None, "{f:?}");
        assert_eq!(f.cause, p.failed_because);
        assert_eq!(f.fix, p.then);
        assert!(
            f.detail().contains("goatbuilder explain federate 12"),
            "{}",
            f.detail()
        );
        assert!(failed_phase("federate", "99", "", None).is_none());
    }

    /// A step is found by the key the runner recorded it under, a number or a
    /// name, and keeps the catalog's key: a phase number stays a number in
    /// JSON and in the row's text, as it was when phases were only numbers.
    #[test]
    fn a_failed_step_is_found_by_its_recorded_key_number_or_name() {
        let f = failed_phase("standup", "image-pull", "exit 2\n", None).unwrap();
        assert_eq!(
            f.phase,
            goat_errors::phases::PhaseKey::Name("image-pull".into())
        );
        assert!(
            f.detail()
                .ends_with("(goatbuilder explain standup image-pull)"),
            "{}",
            f.detail()
        );
        let f = failed_phase("federate", "12", "exit 2\n", None).unwrap();
        assert_eq!(f.phase, goat_errors::phases::PhaseKey::Number(12));
        assert_eq!(
            serde_json::to_value(&f).unwrap()["phase"],
            serde_json::json!(12)
        );
        assert!(
            f.detail().ends_with("(goatbuilder explain federate 12)"),
            "{}",
            f.detail()
        );
        assert!(failed_phase("standup", "no-such-step", "", None).is_none());
    }

    /// The code goatbuilder itself printed inside the phase is the answer, and
    /// only the end of a long output is read.
    #[test]
    fn a_code_in_the_phase_s_output_is_taken_and_only_the_tail_is_read() {
        let f = failed_phase(
            "federate",
            "8",
            "noise\ngoatbuilder: refused (GB-194)\n",
            None,
        )
        .unwrap();
        assert_eq!(f.code.as_deref(), Some("GB-194"), "{f:?}");
        let mut long = "Permission denied (publickey).\n".to_string();
        long.push_str(&"x".repeat(TAIL_BYTES + 10));
        let f = failed_phase("federate", "12", &long, None).unwrap();
        assert_eq!(
            f.code, None,
            "an error before the tail is not the one that stopped the phase: {f:?}"
        );
    }
}

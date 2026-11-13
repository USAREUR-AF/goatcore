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
        s.push_str(
            format!(
                "{:>3}  {:<wid$}  {:<wst$}  {:<wk$}  {}",
                r.phase, r.id, r.stage, r.kind, r.title
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
}

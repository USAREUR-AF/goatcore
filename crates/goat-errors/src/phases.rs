//! The phase catalog: every phase of every goatbuilder verb, what it does and
//! why, from ops/phases/catalog.yaml, compiled in at build time like the error
//! catalog (docs/design/goatbuilder-legibility.md, step 6). `explain <verb>
//! <phase>` answers from it, and the documents' phase tables are rendered from
//! the same file (ops/ci/check-phase-catalog.py).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The catalog source, embedded at build time. Re-bake by rebuilding.
pub const PHASES_YAML: &str = include_str!("../../../ops/phases/catalog.yaml");

/// The schema this loader reads.
pub const SCHEMA: &str = "goat-phases/v1";

/// A phase's number, or the title of a step that has none.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(untagged)]
pub enum PhaseKey {
    Number(u32),
    Name(String),
}

impl PhaseKey {
    /// A phase named on a command line: `12` is phase 12, anything else a name.
    pub fn parse(s: &str) -> PhaseKey {
        s.trim()
            .parse::<u32>()
            .map(PhaseKey::Number)
            .unwrap_or_else(|_| PhaseKey::Name(s.trim().to_string()))
    }
}

impl std::fmt::Display for PhaseKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PhaseKey::Number(n) => write!(f, "{n}"),
            PhaseKey::Name(s) => f.write_str(s),
        }
    }
}

/// One phase of one verb.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phase {
    pub verb: String,
    pub phase: PhaseKey,
    /// A short name for the phase, unique within the verb (`preflight`).
    #[serde(default)]
    pub id: String,
    /// The stage of the verb that runs it (`plan`, `deploy`, `test`), when it has stages.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stage: String,
    /// GATE, A (a play), T (a tool), CONFIRM, MANUAL.
    #[serde(default)]
    pub kind: String,
    pub title: String,
    /// What it does.
    pub what: String,
    /// Why it exists.
    pub why: String,
    /// What it changes, and where.
    pub changes: String,
    /// How to tell it worked.
    pub verify: String,
    /// The script, play or verb it runs.
    pub runs: String,
    /// The command that does the same by hand.
    pub by_hand: String,
    /// The document to read: a repository path, with an optional #anchor.
    pub read: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub failed_because: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub then: String,
}

#[derive(Debug, Deserialize)]
struct PhasesFile {
    schema: String,
    phases: Vec<Phase>,
}

/// The loaded phase catalog.
pub struct Phases {
    pub phases: Vec<Phase>,
}

impl Phases {
    /// Parse a phase catalog. Every entry needs every field `explain` answers
    /// from, and no verb has a phase twice.
    pub fn parse(yaml: &str) -> Result<Phases, String> {
        let file: PhasesFile =
            serde_yaml::from_str(yaml).map_err(|e| format!("phase catalog: {e}"))?;
        if file.schema != SCHEMA {
            return Err(format!(
                "phase catalog: schema {} is not {SCHEMA}",
                file.schema
            ));
        }
        let mut seen = BTreeSet::new();
        for p in &file.phases {
            let at = format!("phase catalog: {} phase {}", p.verb, p.phase);
            if !seen.insert((p.verb.clone(), p.phase.clone())) {
                return Err(format!("{at} appears twice"));
            }
            for (name, value) in [
                ("title", &p.title),
                ("what", &p.what),
                ("why", &p.why),
                ("changes", &p.changes),
                ("verify", &p.verify),
                ("runs", &p.runs),
                ("by_hand", &p.by_hand),
                ("read", &p.read),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("{at} has no `{name}`"));
                }
            }
        }
        Ok(Phases {
            phases: file.phases,
        })
    }

    /// The phase `key` of `verb`; a name matches a phase's `id` too.
    pub fn get(&self, verb: &str, key: &PhaseKey) -> Option<&Phase> {
        self.phases.iter().find(|p| {
            p.verb == verb && (&p.phase == key || matches!(key, PhaseKey::Name(n) if *n == p.id))
        })
    }

    /// Every phase of `verb`, in order.
    pub fn of(&self, verb: &str) -> Vec<&Phase> {
        let mut v: Vec<&Phase> = self.phases.iter().filter(|p| p.verb == verb).collect();
        v.sort_by(|a, b| a.phase.cmp(&b.phase));
        v
    }

    /// The verbs the catalog describes.
    pub fn verbs(&self) -> BTreeSet<&str> {
        self.phases.iter().map(|p| p.verb.as_str()).collect()
    }
}

/// The embedded phase catalog, parsed once. One that does not parse is a
/// build defect the CI gate refuses; here it is a panic with the reason.
pub fn phases() -> &'static Phases {
    static CAT: OnceLock<Phases> = OnceLock::new();
    CAT.get_or_init(|| Phases::parse(PHASES_YAML).expect("the embedded phase catalog must parse"))
}

/// `explain <verb> <phase>`: the entry, or `None` when the verb has no such phase.
pub fn explain_phase(verb: &str, phase: &str) -> Option<&'static Phase> {
    phases().get(verb, &PhaseKey::parse(phase))
}

/// The entry as `explain` prints it on a terminal.
pub fn render_text(p: &Phase) -> String {
    let mut s = format!("{} phase {} [{}]: {}\n", p.verb, p.phase, p.kind, p.title);
    for (label, value) in [
        ("What it does", &p.what),
        ("Why it exists", &p.why),
        ("What it changes", &p.changes),
        ("How to tell it worked", &p.verify),
        ("It runs", &p.runs),
        ("By hand", &p.by_hand),
        ("If it failed", &p.failed_because),
        ("Then", &p.then),
        ("Read", &p.read),
    ] {
        if !value.trim().is_empty() {
            s.push_str(&format!("\n{label}:\n  {}\n", value.trim()));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_catalog_parses_and_describes_federates_fourteen_phases() {
        let c = phases();
        let fed = c.of("federate");
        let numbers: Vec<PhaseKey> = fed.iter().map(|p| p.phase.clone()).collect();
        assert_eq!(numbers, (1..=14).map(PhaseKey::Number).collect::<Vec<_>>());
        for p in fed {
            assert!(!p.id.is_empty(), "federate phase {} has no id", p.phase);
        }
    }

    #[test]
    fn a_phase_is_found_by_number_or_by_id() {
        let by_number = explain_phase("federate", "8").unwrap();
        let by_id = explain_phase("federate", "mint").unwrap();
        assert_eq!(by_number, by_id);
        assert!(explain_phase("federate", "99").is_none());
        assert!(explain_phase("no-such-verb", "1").is_none());
    }

    #[test]
    fn every_read_path_exists_in_the_repository() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for p in &phases().phases {
            let path = p.read.split('#').next().unwrap_or("").trim();
            assert!(
                root.join(path).exists(),
                "{} phase {}: read {path} does not exist",
                p.verb,
                p.phase
            );
        }
    }

    #[test]
    fn an_entry_missing_a_field_or_given_twice_is_refused() {
        let one = "  - {verb: v, phase: 1, title: t, what: w, why: y, changes: c, verify: k, runs: r, by_hand: b, read: d}\n";
        assert!(Phases::parse(&format!("schema: goat-phases/v1\nphases:\n{one}")).is_ok());
        let twice = Phases::parse(&format!("schema: goat-phases/v1\nphases:\n{one}{one}"));
        assert!(twice.err().unwrap().contains("appears twice"));
        let no_why = one.replace("why: y, ", "why: '', ");
        let e = Phases::parse(&format!("schema: goat-phases/v1\nphases:\n{no_why}")).err();
        assert!(e.unwrap().contains("no `why`"));
        assert!(Phases::parse("schema: goat-phases/v0\nphases: []\n").is_err());
    }

    #[test]
    fn the_text_names_every_answer() {
        let t = render_text(explain_phase("federate", "1").unwrap());
        for label in [
            "What it does",
            "Why it exists",
            "What it changes",
            "How to tell it worked",
            "It runs",
            "By hand",
            "Read",
        ] {
            assert!(t.contains(label), "{label}\n{t}");
        }
    }
}

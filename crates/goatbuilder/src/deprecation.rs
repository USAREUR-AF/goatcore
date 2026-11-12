//! Deprecated commands and flags (ADR 1228, "Deprecation").
//!
//! A command or flag that is to be removed is named in
//! `ops/goatbuilder/compat/deprecations.yaml`, with its replacement and the
//! release that first warns. Until the major release that removes it, it keeps
//! working: a run that uses it is told so on stderr, with the replacement, and
//! a program reading `-o json` finds the same entries in the run's document.
//! The compatibility check (ops/ci/check-goatbuilder-compat.sh, row A3) lets a
//! command or flag of the last release be missing only when this list names it.

use clap::parser::ValueSource;
use clap::{ArgMatches, Command};
use serde::{Deserialize, Serialize};

/// The list this build ships, compiled in like the error catalog.
const SHIPPED: &str = include_str!("../../../ops/goatbuilder/compat/deprecations.yaml");

/// One deprecated command, or one flag of a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deprecation {
    /// The command, as the command reference heads it (`goatbuilder deploy`).
    pub command: String,
    /// The flag (`--env`); absent when the whole command is deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    /// What to use instead.
    pub replacement: String,
    /// The first release that warns (`goatbuilder/v0.6.0`).
    pub deprecated_in: String,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    deprecations: Vec<Deprecation>,
}

impl Deprecation {
    /// What a person reads: the deprecated form, since when, when it goes, and
    /// what to use. `version` is this binary's (`CARGO_PKG_VERSION`): before 1.0
    /// no removal release is promised; from 1.0 removal waits for the next major.
    pub fn warning(&self, version: &str) -> String {
        let what = match &self.flag {
            Some(f) => format!("{} {f}", self.command),
            None => self.command.clone(),
        };
        let major: u64 = version
            .split('.')
            .next()
            .and_then(|m| m.parse().ok())
            .unwrap_or(0);
        let removal = if major == 0 {
            "removed in a later release".to_string()
        } else {
            format!("removed no earlier than goatbuilder {}.0.0", major + 1)
        };
        format!(
            "warning: `{what}` is deprecated since {} and will be {removal}; use {} instead",
            self.deprecated_in, self.replacement
        )
    }
}

/// Read a deprecation list; refuses an entry the warning could not print.
pub fn parse(yaml: &str) -> Result<Vec<Deprecation>, String> {
    let file: File = serde_yaml::from_str(yaml).map_err(|e| format!("deprecations.yaml: {e}"))?;
    for (i, d) in file.deprecations.iter().enumerate() {
        let n = i + 1;
        if d.command != "goatbuilder" && !d.command.starts_with("goatbuilder ") {
            return Err(format!(
                "deprecations.yaml entry {n}: `command` must name a goatbuilder command"
            ));
        }
        if d.replacement.trim().is_empty() {
            return Err(format!(
                "deprecations.yaml entry {n}: `replacement` must name what to use instead"
            ));
        }
        let release = d.deprecated_in.strip_prefix("goatbuilder/v").unwrap_or("");
        if release.split('.').count() != 3
            || !release
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(format!(
                "deprecations.yaml entry {n}: `deprecated_in` must be goatbuilder/vX.Y.Z"
            ));
        }
        if let Some(f) = &d.flag {
            let name = f
                .strip_prefix("--")
                .or_else(|| f.strip_prefix('-'))
                .unwrap_or("");
            let whole = !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && (f.starts_with("--") || name.len() == 1);
            if !whole {
                return Err(format!(
                    "deprecations.yaml entry {n}: `flag` must be written as typed (--name or -n)"
                ));
            }
        }
    }
    Ok(file.deprecations)
}

/// The list this build ships.
pub fn shipped() -> Result<Vec<Deprecation>, String> {
    parse(SHIPPED)
}

/// The entries a parsed command line uses: a deprecated command on the path the
/// run took (or a command above it), or a deprecated flag given on the command
/// line (long or short form) to the command the entry names. Each entry once.
///
/// `argv` is the command line as typed (`std::env::args()`): a flag renamed
/// with its old name kept as a clap alias is only visible there, because
/// `ArgMatches` does not record which spelling was used.
pub fn used<'a>(
    deps: &'a [Deprecation],
    root: &Command,
    matches: &ArgMatches,
    argv: &[String],
) -> Vec<&'a Deprecation> {
    // (command path, the flags given to that command, as --long and -s forms)
    let mut levels: Vec<(String, Vec<String>)> = Vec::new();
    let mut chain: Vec<&Command> = Vec::new();
    let mut path = root.get_name().to_string();
    let mut cmd = root;
    let mut m = matches;
    loop {
        let mut given = Vec::new();
        for a in cmd.get_arguments() {
            if m.try_contains_id(a.get_id().as_str()).unwrap_or(false)
                && m.value_source(a.get_id().as_str()) == Some(ValueSource::CommandLine)
            {
                if let Some(l) = a.get_long() {
                    given.push(format!("--{l}"));
                }
                if let Some(s) = a.get_short() {
                    given.push(format!("-{s}"));
                }
            }
        }
        levels.push((path.clone(), given));
        chain.push(cmd);
        match m.subcommand() {
            Some((name, sub)) => match cmd.find_subcommand(name) {
                Some(c) => {
                    path = format!("{path} {name}");
                    cmd = c;
                    m = sub;
                }
                None => break,
            },
            None => break,
        }
    }
    for (level, spelling) in typed_flags(&chain, argv) {
        levels[level].1.push(spelling);
    }
    let ran = &levels.last().expect("the root is always a level").0;
    deps.iter()
        .filter(|d| match &d.flag {
            None => ran == &d.command || ran.starts_with(&format!("{} ", d.command)),
            Some(f) => levels
                .iter()
                .any(|(p, given)| p == &d.command && given.contains(f)),
        })
        .collect()
}

/// Every flag spelling typed on the command line, with the level (an index
/// into `chain`, the commands the run took) it was typed under. A flag's value
/// is skipped, so a value that looks like a flag is not taken for one.
fn typed_flags(chain: &[&Command], argv: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut level = 0;
    let mut tokens = argv.iter().skip(1);
    while let Some(t) = tokens.next() {
        if t == "--" {
            break;
        }
        if let Some(rest) = t.strip_prefix('-').filter(|r| !r.is_empty()) {
            let (spelling, inline_value) = match t.split_once('=') {
                Some((name, _)) => (name.to_string(), true),
                None => (t.clone(), false),
            };
            let long = rest.strip_prefix('-');
            let takes_value = chain[..=level].iter().rev().any(|c| {
                c.get_arguments().any(|a| {
                    let named = match long {
                        Some(l) => {
                            let l = l.split('=').next().unwrap_or(l);
                            a.get_long() == Some(l)
                                || a.get_all_aliases().into_iter().flatten().any(|x| x == l)
                        }
                        None => rest.len() == 1 && a.get_short() == rest.chars().next(),
                    };
                    named && a.get_action().takes_values()
                })
            });
            out.push((level, spelling));
            if takes_value && !inline_value {
                tokens.next();
            }
            continue;
        }
        // A word: the next command on the run's path, by name or alias.
        if let Some(next) = chain.get(level + 1) {
            if next.get_name() == t || next.get_all_aliases().any(|a| a == t) {
                level += 1;
            }
        }
    }
    out
}

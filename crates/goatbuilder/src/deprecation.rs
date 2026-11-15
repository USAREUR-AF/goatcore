//! Deprecated commands and flags (ADR 1228, "Deprecation").
//!
//! A command or flag that is to be removed is named in
//! `ops/goatbuilder/compat/deprecations.yaml`, with its replacement and the
//! release that first warns. Until the major release that removes it, it keeps
//! working: a run that uses it is told so on stderr, with the replacement
//! (`main.rs` calls [`in_use`]). The same entries, each with its warning
//! line, are the `deprecations` field of the run's `-o json` and `-o yaml`
//! document ([`record`], read by `Report::new`), absent when the run used none.
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
    /// The flag (`--env`), or a positional argument as the help names it
    /// (`<COMPONENT>`, only with `value`); absent when the whole command is
    /// deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    /// One value of the flag or argument; absent when the whole flag is
    /// deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
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
        let what = match (&self.flag, &self.value) {
            (Some(f), Some(v)) if f.starts_with('<') => format!("`{} {v}` ({f})", self.command),
            (Some(f), Some(v)) => format!("`{} {f} {v}`", self.command),
            (Some(f), None) => format!("`{} {f}`", self.command),
            (None, _) => format!("`{}`", self.command),
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
            "warning: {what} is deprecated since {} and will be {removal}; use {} instead",
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
        if let Some(v) = &d.value {
            if d.flag.is_none() || v.trim().is_empty() {
                return Err(format!(
                    "deprecations.yaml entry {n}: `value` deprecates one value of a flag or argument; it needs the `flag` (or `<ARG>`) it belongs to"
                ));
            }
        }
        if let Some(arg) = d.flag.as_deref().and_then(|f| f.strip_prefix('<')) {
            let whole = arg.strip_suffix('>').is_some_and(|a| {
                !a.is_empty()
                    && a.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            });
            if !whole || d.value.is_none() {
                return Err(format!(
                    "deprecations.yaml entry {n}: a positional argument (`<NAME>`) is deprecated one value at a time; name it as the help does, with `value`"
                ));
            }
        } else if let Some(f) = &d.flag {
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
    let mut parsed: Vec<&ArgMatches> = Vec::new();
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
        parsed.push(m);
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
        .filter(|d| match (&d.flag, &d.value) {
            (None, _) => ran == &d.command || ran.starts_with(&format!("{} ", d.command)),
            (Some(f), None) => levels
                .iter()
                .any(|(p, given)| p == &d.command && given.contains(f)),
            (Some(f), Some(v)) => levels.iter().enumerate().any(|(i, (p, given))| {
                p == &d.command
                    && (f.starts_with('<') || given.contains(f))
                    && given_values(chain[i], parsed[i], f).iter().any(|x| x == v)
            }),
        })
        .collect()
}

/// One deprecated item a run used, as its document lists it: the entry, and
/// the warning line the run printed on stderr for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Used {
    #[serde(flatten)]
    pub entry: Deprecation,
    pub warning: String,
}

/// The deprecated items a run's command line uses, each with its warning.
/// `version` is this binary's.
pub fn in_use(
    deps: &[Deprecation],
    root: &Command,
    matches: &ArgMatches,
    argv: &[String],
    version: &str,
) -> Vec<Used> {
    used(deps, root, matches, argv)
        .into_iter()
        .map(|d| Used {
            entry: d.clone(),
            warning: d.warning(version),
        })
        .collect()
}

/// The warnings a run prints on stderr: one line for each deprecated command,
/// flag or value its command line uses. `version` is this binary's.
#[allow(dead_code)] // the binary prints each [`Used`]'s warning; the tests read this
pub fn warnings(
    deps: &[Deprecation],
    root: &Command,
    matches: &ArgMatches,
    argv: &[String],
    version: &str,
) -> Vec<String> {
    in_use(deps, root, matches, argv, version)
        .into_iter()
        .map(|u| u.warning)
        .collect()
}

static RUN: std::sync::OnceLock<Vec<Used>> = std::sync::OnceLock::new();

/// Keep this run's items for its document; main calls it once, after it warns.
#[allow(dead_code)] // the module is also compiled alone by tests/deprecation.rs
pub fn record(used: Vec<Used>) {
    let _ = RUN.set(used);
}

/// This run's items, empty before [`record`] or when it used none.
#[allow(dead_code)]
pub fn recorded() -> Vec<Used> {
    RUN.get().cloned().unwrap_or_default()
}

/// The values given on the command line to the flag (`--env`, `-e`) or
/// positional argument (`<COMPONENT>`, by its value name or its id) `f` names.
fn given_values(cmd: &Command, m: &ArgMatches, f: &str) -> Vec<String> {
    let named = |a: &&clap::Arg| -> bool {
        if let Some(arg) = f.strip_prefix('<').and_then(|x| x.strip_suffix('>')) {
            a.is_positional()
                && (a
                    .get_value_names()
                    .into_iter()
                    .flatten()
                    .any(|n| n.as_str() == arg)
                    || a.get_id().as_str().to_uppercase() == arg.to_uppercase())
        } else if let Some(long) = f.strip_prefix("--") {
            a.get_long() == Some(long)
                || a.get_all_aliases().into_iter().flatten().any(|x| x == long)
        } else {
            let c = f.strip_prefix('-').and_then(|x| x.chars().next());
            c.is_some()
                && (a.get_short() == c
                    || a.get_all_short_aliases()
                        .into_iter()
                        .flatten()
                        .any(|x| Some(x) == c))
        }
    };
    let Some(a) = cmd.get_arguments().find(named) else {
        return Vec::new();
    };
    let id = a.get_id().as_str();
    if m.value_source(id) != Some(ValueSource::CommandLine) {
        return Vec::new();
    }
    match m.try_get_raw(id) {
        Ok(Some(vals)) => vals.map(|v| v.to_string_lossy().into_owned()).collect(),
        _ => Vec::new(),
    }
}

/// Every flag spelling typed on the command line, with the level (an index
/// into `chain`, the commands the run took) it was typed under. A flag's value
/// is skipped, so a value that looks like a flag is not taken for one.
fn typed_flags(chain: &[&Command], argv: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut level = 0;
    let mut tokens = argv.iter().skip(1).peekable();
    while let Some(t) = tokens.next() {
        if t == "--" {
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, _)) => (n, true),
                None => (long, false),
            };
            out.push((level, format!("--{name}")));
            let arg = find_arg(chain, level, |a| {
                a.get_long() == Some(name)
                    || a.get_all_aliases().into_iter().flatten().any(|x| x == name)
            });
            if let (Some(a), false) = (arg, inline) {
                skip_values(a, &mut tokens);
            }
            continue;
        }
        if let Some(cluster) = t.strip_prefix('-').filter(|r| !r.is_empty()) {
            // `-v`, `-Ex` (a value attached), `-vE x` (flags together, the last
            // taking the next word as its value).
            for (i, c) in cluster.char_indices() {
                out.push((level, format!("-{c}")));
                let arg = find_arg(chain, level, |a| {
                    a.get_short() == Some(c)
                        || a.get_all_short_aliases()
                            .into_iter()
                            .flatten()
                            .any(|x| x == c)
                });
                if let Some(a) = arg.filter(|a| a.get_action().takes_values()) {
                    if cluster[i + c.len_utf8()..].is_empty() {
                        skip_values(a, &mut tokens);
                    }
                    break;
                }
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

/// The argument a flag names, looked up from the command it was typed under
/// back to the root (a global flag is defined there).
fn find_arg<'c>(
    chain: &[&'c Command],
    level: usize,
    named: impl Fn(&clap::Arg) -> bool,
) -> Option<&'c clap::Arg> {
    chain[..=level]
        .iter()
        .rev()
        .find_map(|c| c.get_arguments().find(|a| named(a)))
}

/// Pass over the values a flag takes, as clap reads them: one, or for a flag
/// that takes several, every following word up to the next flag (every word,
/// for one that accepts values that look like flags), at most its maximum.
fn skip_values<'t>(
    a: &clap::Arg,
    tokens: &mut std::iter::Peekable<impl Iterator<Item = &'t String>>,
) {
    if !a.get_action().takes_values() {
        return;
    }
    let max = a.get_num_args().map(|r| r.max_values()).unwrap_or(1).max(1);
    let hyphens = a.is_allow_hyphen_values_set();
    let mut taken = 0;
    while taken < max {
        match tokens.peek() {
            Some(n) if n.as_str() == "--" => break,
            Some(n) if taken == 0 || hyphens || !n.starts_with('-') => {
                tokens.next();
                taken += 1;
            }
            _ => break,
        }
    }
}

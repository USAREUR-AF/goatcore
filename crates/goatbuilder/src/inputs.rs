//! Every value a run uses, with where it came from (contract clause C11, gate
//! row F2; docs/design/goatbuilder-legibility.md, "Inputs").
//!
//! One [`Input`] is one value a command line, a phase or a play reads: its name
//! (an environment variable, a flag or an Ansible variable), its value, and its
//! [`Source`]. A source is exactly one of:
//!
//! - a file and line: read from a file the deployment holds;
//! - derived from a declaration by a rule;
//! - an environment override: the operator set it, and it wins;
//! - a default, which belongs to no deployment;
//! - allocated: `goatbuilder federate allocate` chose it, and the allocation
//!   record holds it (or will, once allocate is run);
//! - an argument typed on the command line.
//!
//! A secret's value is never held here: [`Input::secret`] takes no value, and
//! an input whose name reads as a credential is made secret whatever its
//! constructor was given. A secret shows only its source.
//!
//! An input nothing gave a value to is [`Input::unset`]: it names where to
//! declare it and carries no source, so a check that asks every value for its
//! source fails on it.

// `goatbuilder inputs` and the run record read these; until main.rs wires the
// verb (W-L1's step 4), only the tests do.
#![allow(dead_code)]

use std::path::Path;

use serde::Serialize;

pub mod federate;
pub mod joinnode;
pub mod standup;

/// Where one value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Read from a file: the path (relative to the checkout when inside it),
    /// the line (1-based, when the key was found on one), and the key.
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        key: String,
    },
    /// Derived from a declaration (or another input) by a rule.
    Derived { from: String, rule: String },
    /// Set in the environment by the operator; it wins over every other source.
    Environment { var: String },
    /// A default that belongs to no deployment, and why it is that.
    Default { rule: String },
    /// Allocated: the record that holds it and the line there, or `line: None`
    /// with `recorded: false` when it was allocated afresh by this reading and
    /// is not in the record yet.
    Allocated {
        record: String,
        field: String,
        recorded: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
    },
    /// Typed on the command line.
    Argument { flag: String },
}

impl Source {
    pub fn file(path: impl Into<String>, line: Option<usize>, key: impl Into<String>) -> Source {
        Source::File {
            path: path.into(),
            line,
            key: key.into(),
        }
    }

    pub fn derived(from: impl Into<String>, rule: impl Into<String>) -> Source {
        Source::Derived {
            from: from.into(),
            rule: rule.into(),
        }
    }

    pub fn env(var: impl Into<String>) -> Source {
        Source::Environment { var: var.into() }
    }

    pub fn default(rule: impl Into<String>) -> Source {
        Source::Default { rule: rule.into() }
    }

    pub fn arg(flag: impl Into<String>) -> Source {
        Source::Argument { flag: flag.into() }
    }

    /// The one line a person reads.
    pub fn render(&self) -> String {
        match self {
            Source::File { path, line: Some(n), key } => format!("{path}:{n} {key}"),
            Source::File { path, line: None, key } => format!("{path} {key}"),
            Source::Derived { from, rule } => format!("derived from {from} by {rule}"),
            Source::Environment { var } => format!("environment ${var}"),
            Source::Default { rule } => format!("default: {rule}"),
            Source::Allocated { record, field, recorded: true, line: Some(n) } => {
                format!("allocated: {record}:{n} {field}")
            }
            Source::Allocated { record, field, recorded: true, line: None } => {
                format!("allocated: {record} {field}")
            }
            Source::Allocated { record, field, recorded: false, .. } => format!(
                "allocated: {field}, the first free value (not yet in {record}; `goatbuilder federate allocate` records it)"
            ),
            Source::Argument { flag } => format!("argument {flag}"),
        }
    }
}

/// One value a run uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    value: Option<String>,
    pub secret: bool,
    pub source: Option<Source>,
    /// For an input nothing gave a value: where to declare it.
    pub unset: Option<String>,
}

/// Whether a name reads as a credential: its value is then never held. A name
/// with a word like pass, secret or token reads as one, unless it ends by
/// naming where a value is kept (`token_env`, `secret_file`), not the value.
pub fn secret_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let names_a_place = words
        .last()
        .is_some_and(|w| ["env", "var", "variable", "name", "path", "file", "dir"].contains(w));
    let credential = words.iter().any(|w| {
        [
            "pass",
            "password",
            "passwd",
            "passphrase",
            "secret",
            "token",
            "credential",
            "credentials",
            "apikey",
            "enckey",
        ]
        .contains(w)
    }) || n.contains("private_key")
        || n.contains("api_key");
    credential && !names_a_place
}

impl Input {
    /// A value and its source. A credential-shaped name is held as a secret.
    pub fn new(name: impl Into<String>, value: impl Into<String>, source: Source) -> Input {
        let name = name.into();
        if secret_name(&name) {
            return Input::secret(name, source);
        }
        Input {
            name,
            value: Some(value.into()),
            secret: false,
            source: Some(source),
            unset: None,
        }
    }

    /// A secret: its source only.
    pub fn secret(name: impl Into<String>, source: Source) -> Input {
        Input {
            name: name.into(),
            value: None,
            secret: true,
            source: Some(source),
            unset: None,
        }
    }

    /// A variable a command reads from its environment when it runs
    /// (`$(printenv NAME)`, `$NAME`): its source is that variable, or `store`
    /// when the declared secrets store supplies it; `None` for `store` and
    /// nothing in the environment leaves it unset, with `hint` saying where to
    /// set it. A credential is never held, set or not.
    pub fn reads_env(
        name: impl Into<String>,
        secret: bool,
        now: Option<String>,
        store: Option<Source>,
        hint: impl Into<String>,
    ) -> Input {
        let name = name.into();
        let secret = secret || secret_name(&name);
        let (source, unset) = match (&now, store) {
            (Some(_), _) => (Source::env(name.clone()), None),
            (None, Some(s)) => (s, None),
            (None, None) => (Source::env(name.clone()), Some(hint.into())),
        };
        Input {
            value: if secret { None } else { now },
            name,
            secret,
            source: Some(source),
            unset,
        }
    }

    /// An input nothing gave a value, and where to declare it.
    pub fn unset(name: impl Into<String>, declare: impl Into<String>) -> Input {
        Input {
            name: name.into(),
            value: None,
            secret: false,
            source: None,
            unset: Some(declare.into()),
        }
    }

    /// The value, never a secret's.
    pub fn value(&self) -> Option<&str> {
        if self.secret {
            None
        } else {
            self.value.as_deref()
        }
    }

    /// One line: `NAME=value  <source>`.
    pub fn line(&self) -> String {
        match (&self.source, self.value(), self.secret) {
            (Some(s), _, true) => match &self.unset {
                Some(u) => format!("{} (secret) not set now  {} ({u})", self.name, s.render()),
                None => format!("{} (secret, not shown)  {}", self.name, s.render()),
            },
            (Some(s), Some(v), false) => format!("{}={v}  {}", self.name, s.render()),
            (Some(s), None, false) => match &self.unset {
                Some(u) => format!("{} not set now  {} ({u})", self.name, s.render()),
                None => format!("{}  {}", self.name, s.render()),
            },
            (None, _, _) => format!(
                "{} not set: {}",
                self.name,
                self.unset.as_deref().unwrap_or("nothing gave it a value")
            ),
        }
    }
}

/// The shape every run record's `inputs` and `goatbuilder inputs -o json` use:
/// `name`, `value` (null for a secret or an unset input), `secret`, `source`
/// (the rendered line; null when unset), `origin` (the structured source), and
/// `unset` (where to declare it) when nothing gave a value.
impl Serialize for Input {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("name", &self.name)?;
        m.serialize_entry("value", &self.value())?;
        m.serialize_entry("secret", &self.secret)?;
        m.serialize_entry("source", &self.source.as_ref().map(Source::render))?;
        m.serialize_entry("origin", &self.source)?;
        if let Some(u) = &self.unset {
            m.serialize_entry("unset", u)?;
        }
        m.end()
    }
}

/// The rows of `goatbuilder inputs` and of a dry run's input list: one per
/// input, `NAME=value  <source>`. An unset input is a row that is not ok,
/// with where to declare it as its fix.
pub fn rows(inputs: &[Input]) -> Vec<crate::Check> {
    inputs
        .iter()
        .map(|i| crate::Check {
            name: "input".into(),
            ok: i.source.is_some(),
            detail: i.line(),
            fix: i.unset.clone(),
        })
        .collect()
}

/// Add an input unless one of that name is already listed (the first source
/// to give a value is the one the run uses).
pub fn push(v: &mut Vec<Input>, i: Input) {
    if !v.iter().any(|x| x.name == i.name) {
        v.push(i);
    }
}

// --- where in a file ------------------------------------------------------------

/// One step of a YAML path: a mapping key or a sequence index.
#[derive(Debug, Clone, Copy)]
pub enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

/// The 1-based line of a YAML path in block-style YAML text, e.g.
/// `[Key("peerings"), Index(0), Key("with")]`. `None` when it is not written
/// there in block style (a flow mapping, an anchor, or absent). A reader of the
/// layout only, for naming a line; the value always comes from the YAML parser.
pub fn yaml_line(text: &str, path: &[Seg]) -> Option<usize> {
    // Each content line as (line number, indent of its content, content),
    // a `- ` item marker expanded into the indent of what follows it.
    let mut lines: Vec<(usize, usize, bool, &str)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim_start();
        if t.is_empty() || t.starts_with('#') || t == "---" {
            continue;
        }
        let mut indent = raw.len() - t.len();
        let mut rest = t;
        let mut item = false;
        if rest == "-" || rest.starts_with("- ") {
            item = true;
            let after = rest[1..].trim_start();
            indent += rest.len() - after.len();
            rest = after;
        }
        lines.push((i + 1, indent, item, rest));
    }
    let key_of = |c: &str| -> Option<String> {
        let (k, _) = c.split_once(':')?;
        let k = k.trim().trim_matches(|q| q == '"' || q == '\'');
        (!k.is_empty() && !k.contains(' ') || c.starts_with('"') || c.starts_with('\''))
            .then(|| k.to_string())
    };
    // The window of lines that hold the current node's children, and their indent.
    let (mut lo, mut hi) = (0usize, lines.len());
    let mut found = None;
    for (si, seg) in path.iter().enumerate() {
        let child = lines.get(lo..hi)?.first()?.1;
        let window = lo..hi;
        // A key: the child line at the window's indent that names it. An index:
        // the n-th `- ` item at that indent.
        let j = match seg {
            Seg::Key(k) => window
                .clone()
                .find(|&j| lines[j].1 == child && key_of(lines[j].3).as_deref() == Some(*k))?,
            Seg::Index(n) => window
                .clone()
                .filter(|&j| lines[j].2 && lines[j].1 == child)
                .nth(*n)?,
        };
        found = Some(lines[j].0);
        if si + 1 == path.len() {
            break;
        }
        // The node's children. A key's: the lines after it indented deeper. An
        // item's: its own line onwards, up to the next item or a shallower line.
        let ind = lines[j].1;
        let after = j + 1..hi;
        (lo, hi) = match seg {
            Seg::Key(_) => (
                j + 1,
                after.clone().find(|&x| lines[x].1 <= ind).unwrap_or(hi),
            ),
            Seg::Index(_) => (
                j,
                after
                    .clone()
                    .find(|&x| lines[x].1 < ind || (lines[x].1 == ind && lines[x].2))
                    .unwrap_or(hi),
            ),
        };
        if lo >= hi {
            return None;
        }
    }
    found
}

/// [`yaml_line`] of a top-level key.
pub fn yaml_key_line(text: &str, key: &str) -> Option<usize> {
    yaml_line(text, &[Seg::Key(key)])
}

/// The 1-based line of a host line `name ...` under `[group]` in an INI inventory.
pub fn ini_host_line(text: &str, group: &str, name: &str) -> Option<usize> {
    let mut inside = false;
    for (i, raw) in text.lines().enumerate() {
        let l = raw.trim();
        if let Some(g) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            inside = g == group;
            continue;
        }
        if inside && l.split_whitespace().next() == Some(name) {
            return Some(i + 1);
        }
    }
    None
}

/// A path as a person reads it: relative to `repo` when inside it.
pub fn rel(repo: &Path, p: &Path) -> String {
    p.strip_prefix(repo).unwrap_or(p).display().to_string()
}

/// The values of one command line by the conformance test's rule
/// (tests/legibility.rs, check b): the right side of every `-e name=value`
/// and `--flag=value`, the word after each option that takes a value (the
/// test's list and `extra`), the AWS profile `with-aws` is given, and the
/// value of a `NAME=value` before a command. `(true, NAME)` is a value read
/// from the environment (`$NAME`, `${NAME}`, `$(printenv NAME)`).
#[cfg(test)]
pub(crate) fn command_values(line: &str, extra: &[&str]) -> Vec<(bool, String)> {
    const FLAGS: &[&str] = &[
        "-l",
        "--limit",
        "-i",
        "--inventory",
        "--site",
        "--profile",
        "--tags",
        "--skip-tags",
        "--repo",
        "--address",
        "--node",
        "--class",
        "--env",
        "--component",
        "--out",
        "--with",
    ];
    let t =
        shlex::split(line).unwrap_or_else(|| line.split_whitespace().map(String::from).collect());
    let of = |v: &str| -> (bool, String) {
        // A word the shell ends with `;` (`... ini; fi`) is the value before it.
        let v = v.trim().trim_end_matches(';');
        // An Ansible lookup of the environment: `{{ lookup("env", "NAME") }}`.
        if let Some(n) = v
            .strip_prefix("{{ lookup(\"env\", \"")
            .and_then(|r| r.strip_suffix("\") }}"))
        {
            return (true, n.to_string());
        }
        let env = v
            .strip_prefix("$(printenv ")
            .and_then(|r| r.strip_suffix(')'))
            .or_else(|| v.strip_prefix("${").and_then(|r| r.strip_suffix('}')))
            .or_else(|| {
                v.strip_prefix('$')
                    .filter(|r| r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            });
        match env {
            Some(n) => (true, n.trim().to_string()),
            None => (false, v.to_string()),
        }
    };
    let takes = |w: &str| FLAGS.contains(&w) || extra.contains(&w);
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let w = &t[i];
        if w == "-e" && i + 1 < t.len() {
            if let Some((_, x)) = t[i + 1].split_once('=') {
                out.push(of(x));
            }
            i += 2;
            continue;
        }
        if w.starts_with("--") && w.contains('=') {
            out.push(of(w.split_once('=').unwrap().1));
        } else if takes(w) && i + 1 < t.len() {
            out.push(of(&t[i + 1]));
            i += 1;
        } else if w.ends_with("with-aws") {
            if let Some(p) = t.get(i + 1) {
                out.push(of(p));
            }
        } else if let Some((k, x)) = w.split_once('=') {
            if !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                // `NAME=$(command)` is made when the line runs: named by NAME.
                if x.starts_with("$(") && !x.starts_with("$(printenv ") {
                    out.push((true, k.to_string()));
                } else {
                    out.push(of(x));
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_holds_its_value() {
        let s = Input::secret("ALPHA_PG_PASS", Source::env("ALPHA_PG_PASS"));
        assert_eq!(s.value(), None);
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["value"], serde_json::Value::Null);
        assert_eq!(j["secret"], true);
        assert!(j["source"].as_str().unwrap().contains("ALPHA_PG_PASS"));
        // A credential-shaped name given a value is held as a secret anyway.
        let s = Input::new("NB_API_TOKEN", "hunter2", Source::env("NB_API_TOKEN"));
        assert!(s.secret);
        for n in [
            "ALPHA_PG_PASS",
            "CLOUDFLARE_API_TOKEN_ALPHA",
            "ALPHA_NB_ENCKEY",
            "relay_secret",
            "netbird_api_key",
        ] {
            assert!(secret_name(n), "{n}");
        }
        for n in [
            "token_env",
            "FED_SIGNING_KEY",
            "secret_file",
            "site",
            "FED_CA_KEY",
        ] {
            assert!(!secret_name(n), "{n}");
        }
        assert!(!serde_json::to_string(&s).unwrap().contains("hunter2"));
        assert!(!s.line().contains("hunter2"), "{}", s.line());
        assert!(!rows(&[s]).iter().any(|r| r.detail.contains("hunter2")));
    }

    #[test]
    fn every_kind_renders_one_line_and_serializes_its_origin() {
        let all = [
            Source::file(
                "ops/ansible/group_vars/xab_edge.yml",
                Some(3),
                "peering_wg_interface",
            ),
            Source::derived("goatbuilder.yaml peerings[0].with (xab)", "<with>_edge"),
            Source::env("FED_WORK"),
            Source::default("the driver's working-directory root"),
            Source::Allocated {
                record: "ops/goatbuilder/peering-values.yaml".into(),
                field: "wg_listen_port".into(),
                recorded: true,
                line: Some(7),
            },
            Source::Allocated {
                record: "ops/goatbuilder/peering-values.yaml".into(),
                field: "wg_listen_port".into(),
                recorded: false,
                line: None,
            },
            Source::arg("--work"),
        ];
        for s in all {
            let r = s.render();
            assert!(!r.is_empty() && !r.contains('\n'), "{r}");
            let j = serde_json::to_value(Input::new("X", "v", s)).unwrap();
            assert!(j["origin"]["kind"].is_string(), "{j}");
            assert_eq!(j["value"], "v");
        }
        let u = serde_json::to_value(Input::unset(
            "FED_CA_KEY",
            "group_vars/xab_edge: goat_federation_ca_key",
        ))
        .unwrap();
        assert_eq!(u["source"], serde_json::Value::Null);
        assert!(u["unset"]
            .as_str()
            .unwrap()
            .contains("goat_federation_ca_key"));
    }

    #[test]
    fn yaml_line_finds_keys_in_mappings_and_sequences() {
        let text = "\
schema: goatbuilder/v1
# a comment
environments:
  - name: dev
    converger: push-ansible
  - name: prod
    sites:
      - name: alpha
deploy:
  aws_profile: lab
peerings:
- with: xab
  environment: dev
  values:
    wg_listen_port: 51825
- with: xac
  environment: prod
";
        use Seg::*;
        assert_eq!(yaml_line(text, &[Key("schema")]), Some(1));
        assert_eq!(
            yaml_line(text, &[Key("deploy"), Key("aws_profile")]),
            Some(10)
        );
        assert_eq!(
            yaml_line(text, &[Key("environments"), Index(1), Key("name")]),
            Some(6)
        );
        assert_eq!(
            yaml_line(
                text,
                &[
                    Key("environments"),
                    Index(1),
                    Key("sites"),
                    Index(0),
                    Key("name")
                ]
            ),
            Some(8)
        );
        assert_eq!(
            yaml_line(text, &[Key("peerings"), Index(0), Key("with")]),
            Some(12)
        );
        assert_eq!(
            yaml_line(
                text,
                &[
                    Key("peerings"),
                    Index(0),
                    Key("values"),
                    Key("wg_listen_port")
                ]
            ),
            Some(15)
        );
        assert_eq!(
            yaml_line(text, &[Key("peerings"), Index(1), Key("environment")]),
            Some(17)
        );
        assert_eq!(
            yaml_line(text, &[Key("peerings"), Index(1), Key("values")]),
            None
        );
        assert_eq!(
            yaml_line(text, &[Key("environments"), Index(0), Key("sites")]),
            None
        );
        assert_eq!(
            yaml_line(text, &[Key("aws_profile")]),
            None,
            "only at the top level"
        );
    }

    #[test]
    fn ini_host_line_finds_a_host_in_its_group() {
        let t = "[a_edge]\nedge-1 ansible_host=192.0.2.10\n\n[a_core]\ncore-1 ansible_host=192.0.2.11\n";
        assert_eq!(ini_host_line(t, "a_core", "core-1"), Some(5));
        assert_eq!(ini_host_line(t, "a_edge", "core-1"), None);
    }
}

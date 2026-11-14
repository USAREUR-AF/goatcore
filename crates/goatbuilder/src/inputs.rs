//! Every value a run uses, with where it came from (contract clause C11, gate
//! row F2; docs/design/goatbuilder-legibility.md, "Run record").
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
//! - an argument typed on the command line;
//! - missing: nothing gave a value, and where to declare it.
//!
//! A secret's value is never held here: [`Input::secret`] takes no value, and
//! an input whose name reads as a credential is made secret whatever its
//! constructor was given. A secret shows only its source.
//!
//! An input nothing gave a value to is [`Input::unset`]: it is still listed,
//! never left out, with the source `missing` naming where to declare it and no
//! value, so a check that compares a command's literal values with the listed
//! ones fails on it. [`Input::sourced`] is false for it.
//!
//! The JSON entry is the run record's (docs/design/goatbuilder-legibility.md,
//! "Run record"): `name`, `value`, `secret`, `source` (one line, never empty),
//! `origin` (`kind` and that kind's fields). Beyond the design's fields,
//! `allocated` also carries `recorded` and `line`, and `missing` carries
//! `declare`; an input read from the environment that is not set now carries
//! `unset`, where to set it.

// Each verb hands its list to the run record (`record`) and prints it in its
// dry run (`rows`, and `section` on a plan page); `why` and the rest wait for
// `goatbuilder inputs`, which no change wires yet.
#![allow(dead_code)]

use std::path::Path;

use serde::Serialize;

pub mod deploy;
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
    /// Nothing gave a value: where to declare it.
    Missing { declare: String },
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
            Source::Missing { declare } => format!("missing: {declare}"),
        }
    }
}

/// One value a run uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    value: Option<String>,
    pub secret: bool,
    pub source: Source,
    /// For an input nothing gave a value: where to declare it.
    pub unset: Option<String>,
    /// Why a value is held back, when it is not a secret by name: an
    /// environment value that was to be a path and names no file.
    pub note: Option<String>,
}

/// Whether a name reads as a credential: its value is then never held. This
/// is a backstop: every credential a verb lists is flagged by its call site
/// ([`Input::secret`], [`Input::reads_env`] with `secret`), and the rule
/// catches a credential-shaped name that was not.
///
/// A name reads as one when a word of it is a credential word (pass, pwd, psk,
/// secret, token, ...), when it contains a credential phrase (password,
/// privatekey, masterkey, encryption_key, signing_key, ...), or when it ends in
/// `key` after a word that makes it a key's material (`bkp_key`, `hmac_key`).
/// It does not when it ends by naming where a value is kept (`token_env`,
/// `secret_file`, `ca_key_path`), not the value.
pub fn secret_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let names_a_place = words
        .last()
        .is_some_and(|w| ["env", "var", "variable", "name", "path", "file", "dir"].contains(w));
    const WORDS: &[&str] = &[
        "pass",
        "password",
        "passwd",
        "passphrase",
        "pwd",
        "psk",
        "secret",
        "secrets",
        "token",
        "credential",
        "credentials",
        "apikey",
        "enckey",
        "privatekey",
        "masterkey",
    ];
    const PHRASES: &[&str] = &[
        "password",
        "passwd",
        "private_key",
        "privatekey",
        "api_key",
        "masterkey",
        "master_key",
        "encryption_key",
        "signing_key",
        "preshared_key",
        "client_secret",
    ];
    const KEY_MATERIAL: &[&str] = &[
        "bkp",
        "backup",
        "hmac",
        "cipher",
        "crypt",
        "enc",
        "encryption",
        "master",
        "private",
        "signing",
        "preshared",
        "psk",
        "jwt",
        "api",
        "session",
        "secret",
    ];
    let key_material = words
        .iter()
        .enumerate()
        .any(|(i, w)| *w == "key" && i > 0 && KEY_MATERIAL.contains(&words[i - 1]));
    let credential = words.iter().any(|w| WORDS.contains(w))
        || PHRASES.iter().any(|p| n.contains(p))
        || key_material;
    credential && !names_a_place
}

/// Where each URL in `value` begins and ends: (index after its `://`, end),
/// a URL running to the start of the next URL's scheme, or the value's end.
fn url_spans(value: &str) -> Vec<(usize, usize)> {
    let marks: Vec<usize> = value.match_indices("://").map(|(i, _)| i).collect();
    let scheme_start = |i: usize| {
        value[..i]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || "+.-".contains(c)))
            .map(|j| j + 1)
            .unwrap_or(0)
    };
    marks
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            (
                i + 3,
                marks
                    .get(n + 1)
                    .map(|&k| scheme_start(k))
                    .unwrap_or(value.len()),
            )
        })
        .collect()
}

/// Every URL's userinfo, and a scheme-less DSN's, removed wherever a value
/// carries one. The whole userinfo goes, the user too, because a token is
/// often the user (`https://ghp_x@github.com/o/r`). A URL's userinfo runs to
/// the last `@` before the next URL's scheme (or the value's end), whatever
/// characters the password holds (`postgres://u:p,ss@h/db` becomes
/// `postgres://<redacted>@h/db`); a DSN word's (`u:p@ss@tcp(h:3306)/db`) to its
/// last `@`. A value with a `@` after its host loses more than it needs to;
/// that is the safe direction.
pub fn redact_userinfo(value: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (start, end) in url_spans(value) {
        out.push_str(&value[at..start]);
        match value[start..end].rfind('@') {
            Some(k) => {
                out.push_str("<redacted>");
                out.push_str(&value[start + k..end]);
            }
            None => out.push_str(&value[start..end]),
        }
        at = end;
    }
    out.push_str(&value[at..]);
    // A scheme-less DSN: a word whose part before its last `@` has a `:`.
    out.split_inclusive(char::is_whitespace)
        .map(|w| {
            let body = w.trim_end();
            match body.rfind('@') {
                Some(k)
                    if !body.contains("://")
                        && body[..k].contains(':')
                        && !body[..k].contains('/') =>
                {
                    format!("<redacted>{}", &w[k..])
                }
                _ => w.to_string(),
            }
        })
        .collect()
}

/// A URL's password dropped, its user kept: `ssh://u:p@h` becomes `ssh://u@h`.
fn drop_password(value: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (start, end) in url_spans(value) {
        out.push_str(&value[at..start]);
        let span = &value[start..end];
        match span.rfind('@') {
            Some(k) => {
                let user = span[..k].split(':').next().unwrap_or("");
                out.push_str(user);
                out.push_str(&span[k..]);
            }
            None => out.push_str(span),
        }
        at = end;
    }
    out.push_str(&value[at..]);
    out
}

impl Input {
    /// A value and its source. A credential-shaped name is held as a secret,
    /// and a URL's userinfo is never held ([`redact_userinfo`]).
    pub fn new(name: impl Into<String>, value: impl Into<String>, source: Source) -> Input {
        let name = name.into();
        let value = value.into();
        if secret_name(&name) {
            return Input::secret(name, source);
        }
        Input {
            name,
            value: Some(redact_userinfo(&value)),
            secret: false,
            source,
            unset: None,
            note: None,
        }
    }

    /// A path, as the call site that made it knows it to be: where something
    /// is kept, not the thing (`FED_SIGNING_KEY` holds the path to our signing
    /// key). It is shown only when it names an existing file, whatever gave
    /// it: anything else may be the thing itself (a key exported, or declared,
    /// in place of its path), and is held back as a secret, with the note that
    /// it is not a file. A declared path that names no file is one the run
    /// refuses anyway, so holding it back costs nothing.
    pub fn path(name: impl Into<String>, value: impl Into<String>, source: Source) -> Input {
        let name = name.into();
        let value = value.into();
        if !Path::new(&value).is_file() {
            let mut held = Input::secret(name.clone(), source);
            held.note = Some(format!(
                "not a file: {name} is to name a file, and its value is held back in case it is the secret itself"
            ));
            return held;
        }
        Input {
            name,
            value: Some(value),
            secret: false,
            source,
            unset: None,
            note: None,
        }
    }

    /// A host address the tool built from the inventory (`ssh://<user>@<host>`),
    /// as the call site that made it knows it to be: its user is an account
    /// name, and is kept; a password after it never is (`ssh://u:p@h` is held
    /// as `ssh://u@h`).
    pub fn address(name: impl Into<String>, value: impl Into<String>, source: Source) -> Input {
        let value = drop_password(&value.into());
        Input {
            name: name.into(),
            value: Some(value),
            secret: false,
            source,
            unset: None,
            note: None,
        }
    }

    /// A secret: its source only.
    pub fn secret(name: impl Into<String>, source: Source) -> Input {
        Input {
            name: name.into(),
            value: None,
            secret: true,
            source,
            unset: None,
            note: None,
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
            value: if secret {
                None
            } else {
                now.map(|v| redact_userinfo(&v))
            },
            name,
            secret,
            source,
            unset,
            note: None,
        }
    }

    /// An input nothing gave a value, and where to declare it.
    pub fn unset(name: impl Into<String>, declare: impl Into<String>) -> Input {
        let declare = declare.into();
        Input {
            name: name.into(),
            value: None,
            secret: false,
            source: Source::Missing {
                declare: declare.clone(),
            },
            unset: Some(declare),
            note: None,
        }
    }

    /// Whether something gave this input: any source but `missing`.
    pub fn sourced(&self) -> bool {
        !matches!(self.source, Source::Missing { .. })
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
        let s = self.sourced().then_some(&self.source);
        match (s, self.value(), self.secret) {
            (Some(s), _, true) => match (&self.unset, &self.note) {
                (Some(u), _) => format!("{} (secret) not set now  {} ({u})", self.name, s.render()),
                (None, Some(n)) => {
                    format!("{} (held back, not shown)  {} ({n})", self.name, s.render())
                }
                (None, None) => format!("{} (secret, not shown)  {}", self.name, s.render()),
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
/// `name`, `value` (null for a secret or an input with no value), `secret`,
/// `source` (the rendered line, never empty: `missing: <where>` when nothing
/// gave a value), `origin` (the structured source), and `unset` (where to
/// declare or set it) when the input has no value now.
impl Serialize for Input {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("name", &self.name)?;
        m.serialize_entry("value", &self.value())?;
        m.serialize_entry("secret", &self.secret)?;
        m.serialize_entry("source", &self.source.render())?;
        m.serialize_entry("origin", &self.source)?;
        if let Some(u) = &self.unset {
            m.serialize_entry("unset", u)?;
        }
        if let Some(n) = &self.note {
            m.serialize_entry("note", n)?;
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
            ok: i.sourced(),
            detail: i.line(),
            fix: i.unset.clone(),
        })
        .collect()
}

/// Hand the run's inputs to its run record (runner::set_inputs), in the shape
/// above: a secret by its source only.
pub fn record(inputs: &[Input]) {
    crate::runner::set_inputs(serde_json::to_value(inputs).unwrap_or_default());
}

/// The section a plan page prints from a dry run's input rows: a heading, then
/// each row's line (`NAME=value  <source>`, a secret by its source only).
pub fn section(lines: &[String], bold: &dyn Fn(&str) -> String) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n{}\n",
        bold("What it reads, and where each value comes from:")
    );
    for l in lines {
        out.push_str(crate::tutor::fill(l, "  ", "      ").trim_end());
        out.push('\n');
    }
    out
}

/// `goatbuilder why <value>`: the inputs a query names, each with the chain of
/// sources that gave it. A query matches an input by its name, else by its
/// value (a secret never matches by value). Each answer is the input's line,
/// then, for a value derived from other listed inputs (`from: "<name> (above)"`,
/// or `"<a> and <b> (above)"` for one derived from two), each one's answer,
/// indented, down to a source that names a file, the environment, a default, an
/// allocation, an argument or nothing.
pub fn why(inputs: &[Input], query: &str) -> Vec<String> {
    let by_name: Vec<&Input> = inputs.iter().filter(|i| i.name == query).collect();
    let hits = if by_name.is_empty() {
        inputs.iter().filter(|i| i.value() == Some(query)).collect()
    } else {
        by_name
    };
    let mut out = Vec::new();
    for i in hits {
        follow(inputs, i, 0, &mut vec![i.name.as_str()], &mut out);
    }
    out
}

/// One input's line at `depth`, then each listed input it was derived from, one
/// level deeper. `path` holds the names above it, so a cycle stops.
fn follow<'a>(
    inputs: &'a [Input],
    at: &'a Input,
    depth: usize,
    path: &mut Vec<&'a str>,
    out: &mut Vec<String>,
) {
    out.push(format!("{}{}", "  ".repeat(depth), at.line()));
    for name in parents(&at.source) {
        let Some(next) = inputs.iter().find(|x| x.name == name) else {
            continue;
        };
        if path.contains(&next.name.as_str()) {
            continue;
        }
        path.push(next.name.as_str());
        follow(inputs, next, depth + 1, path, out);
        path.pop();
    }
}

/// The listed inputs a derived source names: `<name> (above)`, or several
/// joined by ` and `. Empty for any other source.
fn parents(source: &Source) -> Vec<&str> {
    match source {
        Source::Derived { from, .. } => from
            .strip_suffix(" (above)")
            .map(|names| names.split(" and ").collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Add an input unless one of that name is already listed (the first source
/// to give a value is the one the run uses).
pub fn push(v: &mut Vec<Input>, i: Input) {
    if !v.iter().any(|x| x.name == i.name) {
        v.push(i);
    }
}

/// Where the project's AWS profile comes from, by settings::aws_profile's own
/// rule: `GOAT_AWS_PROFILE` when the operator set it, else [`deploy_setting`].
pub fn aws_profile_source(repo: &Path, env: &dyn Fn(&str) -> Option<String>) -> Source {
    if env("GOAT_AWS_PROFILE").is_some_and(|v| !v.is_empty()) {
        return Source::env("GOAT_AWS_PROFILE");
    }
    deploy_setting(
        repo,
        "aws_profile",
        "default, the profile the AWS tools themselves fall back to",
    )
}

/// `deploy.<key>` in goatbuilder.yaml at its line, only when it holds what
/// settings.rs reads there (a non-empty string); otherwise `default`, saying
/// so when the key is set to something that is not one (`""`, a number, `~`).
pub fn deploy_setting(repo: &Path, key: &str, default: &str) -> Source {
    if let Some(src) = setting_line(repo, &["deploy", key]) {
        return src;
    }
    let text = std::fs::read_to_string(repo.join(GOATBUILDER_YAML)).unwrap_or_default();
    match yaml_line(&text, &[Seg::Key("deploy"), Seg::Key(key)]) {
        Some(_) => Source::default(format!(
            "{default} (deploy.{key} is set, but not to a usable value)"
        )),
        None => Source::default(default.to_string()),
    }
}

/// The project's goatbuilder.yaml, from the checkout's root.
const GOATBUILDER_YAML: &str = "ops/goatbuilder/goatbuilder.yaml";

/// `<keys...>` in goatbuilder.yaml at its line, only when it holds what
/// settings.rs reads there: a non-empty string. `None` otherwise, so a caller
/// can fall back as settings.rs does.
pub fn setting_line(repo: &Path, keys: &[&str]) -> Option<Source> {
    let p = repo.join(GOATBUILDER_YAML);
    let text = std::fs::read_to_string(&p).ok()?;
    let path: Vec<Seg> = keys.iter().map(|k| Seg::Key(k)).collect();
    let n = yaml_line(&text, &path)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let usable = keys
        .iter()
        .try_fold(&doc, |v, k| v.get(*k))
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.is_empty());
    usable.then(|| Source::file(rel(repo, &p), Some(n), keys.join(".")))
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

/// Every input a derived source names (`<name> (above)`, each part of `<a> and <b>`)
/// that is not listed: `why` cannot follow it, so each verb's tests ask for none.
#[cfg(test)]
pub(crate) fn dangling(inputs: &[Input]) -> Vec<String> {
    inputs
        .iter()
        .flat_map(|i| parents(&i.source))
        .filter(|n| !inputs.iter().any(|x| x.name == *n))
        .map(String::from)
        .collect()
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

    /// Three leaks review-2 found in the backstop, each its own probe.
    #[test]
    fn an_address_keeps_its_user_and_drops_a_password() {
        let a = Input::address(
            "DOCKER_HOST",
            "ssh://u:p4ss@h.example.org",
            Source::env("FED_ROUTER_DOCKER_HOST"),
        );
        assert_eq!(a.value(), Some("ssh://u@h.example.org"));
        let a = Input::address("DOCKER_HOST", "ssh://goat@192.0.2.20", Source::env("X"));
        assert_eq!(a.value(), Some("ssh://goat@192.0.2.20"));
    }

    #[test]
    fn a_path_from_the_environment_is_shown_only_when_it_names_a_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("signing.key");
        std::fs::write(&f, "x").unwrap();
        let shown = Input::path(
            "FED_SIGNING_KEY",
            f.display().to_string(),
            Source::env("FED_SIGNING_KEY"),
        );
        assert_eq!(shown.value(), Some(f.display().to_string().as_str()));
        // The key itself, exported as the old script comment said.
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let held = Input::path("FED_SIGNING_KEY", key, Source::env("FED_SIGNING_KEY"));
        assert!(held.secret && held.value().is_none(), "{held:?}");
        assert!(!serde_json::to_string(&held).unwrap().contains(key));
        assert!(held.line().contains("not a file"), "{}", held.line());
        // Whatever gave it: a declared path is shown when it names a file, and a
        // key declared in its place is held back.
        let declared = Input::path(
            "FED_SIGNING_KEY",
            f.display().to_string(),
            Source::file(
                "ops/ansible/group_vars/xab_edge.yml",
                Some(3),
                "goat_federation_signing_key",
            ),
        );
        assert_eq!(declared.value(), Some(f.display().to_string().as_str()));
        let declared_key = Input::path(
            "FED_SIGNING_KEY",
            key,
            Source::file(
                "ops/ansible/group_vars/xab_edge.yml",
                Some(3),
                "goat_federation_signing_key",
            ),
        );
        assert!(
            declared_key.secret && declared_key.value().is_none() && declared_key.note.is_some(),
            "{declared_key:?}"
        );
        assert!(!serde_json::to_string(&declared_key).unwrap().contains(key));
    }

    #[test]
    fn a_password_holding_any_character_does_not_end_the_url() {
        for (value, gone) in [
            ("postgres://u:p,ss@h/db", "p,ss"),
            ("postgres://u:p;ss@h/db", "p;ss"),
            ("postgres://u:p\"ss@h/db", "p\"ss"),
            ("postgres://u:p'ss@h/db", "p'ss"),
            ("postgres://u:p ss@h/db", " ss@"),
            ("user:p@ss@h:22", "p@ss"),
        ] {
            let r = redact_userinfo(value);
            assert!(!r.contains(gone), "{value} -> {r}");
            assert!(r.contains("<redacted>"), "{value} -> {r}");
        }
        // Two URLs: each one's userinfo, up to the last '@' before the next scheme.
        assert_eq!(
            redact_userinfo("https://a:x@one.example.org/ https://b:y@two.example.org/"),
            "https://<redacted>@one.example.org/ https://<redacted>@two.example.org/"
        );
    }

    /// A deploy setting is sourced to its line only when the line holds a
    /// usable value; one set to something that is not (`""`, a number, `~`) is
    /// the default, and says why.
    #[test]
    fn a_deploy_setting_names_its_line_only_when_it_gives_the_value() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("ops/goatbuilder/goatbuilder.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        let none = |_: &str| None;
        std::fs::write(
            &cfg,
            "schema: goatbuilder/v1\ndeploy:\n  aws_profile: zagros\n",
        )
        .unwrap();
        assert_eq!(
            aws_profile_source(d.path(), &none),
            Source::file(
                "ops/goatbuilder/goatbuilder.yaml",
                Some(3),
                "deploy.aws_profile"
            )
        );
        for bad in ["\"\"", "123", "~"] {
            std::fs::write(
                &cfg,
                format!("schema: goatbuilder/v1\ndeploy:\n  aws_profile: {bad}\n"),
            )
            .unwrap();
            let src = aws_profile_source(d.path(), &none);
            assert!(
                matches!(&src, Source::Default { rule } if rule.contains("is set, but not to a usable value")),
                "{bad}: {src:?}"
            );
            // And the value settings.rs uses for it is the default, as the source says.
            assert_eq!(crate::settings::aws_profile(d.path()), "default", "{bad}");
        }
        std::fs::write(&cfg, "schema: goatbuilder/v1\n").unwrap();
        assert!(matches!(
            aws_profile_source(d.path(), &none),
            Source::Default { .. }
        ));
        let set = |k: &str| (k == "GOAT_AWS_PROFILE").then(|| "p".to_string());
        assert_eq!(
            aws_profile_source(d.path(), &set),
            Source::env("GOAT_AWS_PROFILE")
        );
    }

    #[test]
    fn why_answers_by_name_or_value_and_follows_what_a_value_was_derived_from() {
        let v = vec![
            Input::new(
                "join_founder_site",
                "zagros-a",
                Source::file(
                    "ops/ansible/group_vars/zagros_b_hosts/vars.yml",
                    Some(3),
                    "join_founder_site",
                ),
            ),
            Input::new(
                "founder_group",
                "zagros_a_hosts",
                Source::derived("join_founder_site (above)", "<founder, - as _>_hosts"),
            ),
            Input::new(
                "acl_site",
                "zagros-a",
                Source::derived(
                    "join_founder_site (above)",
                    "a joiner is governed by its founder's NetBird ACL",
                ),
            ),
            Input::secret("ZAGROS_B_PG_PASS", Source::env("ZAGROS_B_PG_PASS")),
            Input::unset("public_mgmt_fqdn", "declare public_mgmt_fqdn in vars.yml"),
        ];
        // By value: the derived group, then what it came from, indented.
        assert_eq!(
            why(&v, "zagros_a_hosts"),
            [
                "founder_group=zagros_a_hosts  derived from join_founder_site (above) by <founder, - as _>_hosts",
                "  join_founder_site=zagros-a  ops/ansible/group_vars/zagros_b_hosts/vars.yml:3 join_founder_site",
            ]
        );
        // A value two inputs share answers for both.
        assert_eq!(why(&v, "zagros-a").len(), 3);
        // By name, before value.
        assert_eq!(why(&v, "join_founder_site").len(), 1);
        // A secret answers by name with its source, never by value.
        assert_eq!(
            why(&v, "ZAGROS_B_PG_PASS"),
            ["ZAGROS_B_PG_PASS (secret, not shown)  environment $ZAGROS_B_PG_PASS"]
        );
        // Missing says where to declare it; an unknown query answers nothing.
        assert_eq!(
            why(&v, "public_mgmt_fqdn"),
            ["public_mgmt_fqdn not set: declare public_mgmt_fqdn in vars.yml"]
        );
        assert!(why(&v, "nowhere").is_empty());
        // A cycle stops.
        let c = vec![
            Input::new("a", "1", Source::derived("b (above)", "r")),
            Input::new("b", "2", Source::derived("a (above)", "r")),
        ];
        assert_eq!(why(&c, "a").len(), 2);
    }

    /// A source derived from several inputs (`a and b (above)`) names each;
    /// one that is not listed is dangling, though another part is.
    #[test]
    fn a_source_from_two_inputs_names_each_and_each_must_be_listed() {
        let v = vec![
            Input::new("a", "1", Source::file("f", Some(1), "a")),
            Input::new("t", "1@2", Source::derived("a and nosuch (above)", "r")),
        ];
        assert_eq!(dangling(&v), ["nosuch"]);
        // The listed parent is still followed.
        assert_eq!(why(&v, "t"), [v[1].line(), format!("  {}", v[0].line())]);
    }

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
            "PGPASSWORD",
            "MYSQL_PWD",
            "ALPHA_BKP_KEY",
            "wg_privatekey",
            "netbird_encryption_key",
            "vault_masterkey",
            "hmac_key",
            "jwt_signing_key",
            "preshared_key",
            "wg_psk",
            "FED_SIGNING_KEY",
            "OIDC_CLIENT_SECRET",
        ] {
            assert!(secret_name(n), "{n}");
        }
        for n in [
            "token_env",
            "secret_file",
            "site",
            "FED_CA_KEY",
            "ca_key_path",
            "key",
            "keyring",
            "FED_KEYRING",
            "ssh_user",
        ] {
            assert!(!secret_name(n), "{n}");
        }
        // A credential-shaped name is a secret whatever its value looks like: a
        // base64 key or a password may begin with '/'. Only the call site that
        // made a path says so (Input::path).
        for (name, value) in [
            (
                "wg_private_key",
                "/9xK3abcQmFzZTY0IGtleSBtYXRlcmlhbCBoZXJlYWI=",
            ),
            ("DB_PASSWORD", "/hunter2"),
            ("ALPHA_BKP_KEY", "/Zm9vYmFyYmF6"),
            ("FED_SIGNING_KEY", "/srv/fed/signing.key"),
        ] {
            let i = Input::new(name, value, Source::env(name));
            assert!(i.secret && i.value().is_none(), "{name}");
            assert!(
                !serde_json::to_string(&i).unwrap().contains(value),
                "{name}"
            );
        }
        // A path that names no file is held back, whatever gave it.
        let p = Input::path(
            "FED_SIGNING_KEY",
            "/srv/fed/signing.key",
            Source::file(
                "ops/ansible/group_vars/xab_edge.yml",
                Some(1),
                "goat_federation_signing_key",
            ),
        );
        assert!(p.secret && p.value().is_none() && p.note.is_some(), "{p:?}");
        let k = Input::new(
            "jwt_signing_key",
            "c2VjcmV0IGJ5dGVz",
            Source::env("jwt_signing_key"),
        );
        assert!(k.secret && k.value().is_none());
        // A URL's whole userinfo is never held, whatever the name.
        for (value, want) in [
            (
                "postgres://portal:hunter2@192.0.2.20:5432/registry",
                "postgres://<redacted>@192.0.2.20:5432/registry",
            ),
            // a token as the user
            (
                "https://ghp_0123456789abcdefTOKEN@github.com/o/r",
                "https://<redacted>@github.com/o/r",
            ),
            // a password holding '/', '#' or '?'
            ("postgres://u:pa/ss@h/db", "postgres://<redacted>@h/db"),
            ("postgres://u:pa#ss@h/db", "postgres://<redacted>@h/db"),
            (
                "postgres://u:pa?ss@h/db?sslmode=require",
                "postgres://<redacted>@h/db?sslmode=require",
            ),
            // several URLs in one value
            (
                "https://a:x@one.example.org/ https://b:y@two.example.org/",
                "https://<redacted>@one.example.org/ https://<redacted>@two.example.org/",
            ),
            (
                "redis://:p@a:6379,redis://:q@b:6379",
                "redis://<redacted>@a:6379,redis://<redacted>@b:6379",
            ),
            (
                "a=https://u:p@h1/x;b=https://v:q@h2/y",
                "a=https://<redacted>@h1/x;b=https://<redacted>@h2/y",
            ),
            // a scheme-less DSN
            ("u:p@tcp(h:3306)/db", "<redacted>@tcp(h:3306)/db"),
            (
                "dsn u:p@tcp(h:3306)/db done",
                "dsn <redacted>@tcp(h:3306)/db done",
            ),
        ] {
            let u = Input::new("registry", value, Source::env("X"));
            assert_eq!(u.value(), Some(want), "{value}");
            for gone in [
                "hunter2", "TOKEN", "pa/ss", "pa#ss", "pa?ss", ":x@", ":y@", ":p@", ":q@",
            ] {
                assert!(
                    !serde_json::to_string(&u).unwrap().contains(gone),
                    "{value}: {gone}"
                );
            }
        }
        let e = Input::reads_env(
            "DB_URL",
            false,
            Some("https://u:pw@example.org/x".into()),
            None,
            "",
        );
        assert_eq!(e.value(), Some("https://<redacted>@example.org/x"));
        // What carries no userinfo is left as it is; a host address the tool built
        // is said to be one by its call site.
        for plain in [
            "https://example.org/x",
            "user@example.org",
            "no-url",
            "/srv/fed/ca.pem",
            "a:b",
        ] {
            assert_eq!(redact_userinfo(plain), plain);
        }
        assert_eq!(
            redact_userinfo("ssh://goat@192.0.2.20"),
            "ssh://<redacted>@192.0.2.20"
        );
        let a = Input::address(
            "FED_ROUTER_DOCKER_HOST",
            "ssh://goat@192.0.2.20",
            Source::env("X"),
        );
        assert_eq!(a.value(), Some("ssh://goat@192.0.2.20"));
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
        assert!(u["source"].as_str().unwrap().starts_with("missing: "));
        assert_eq!(u["origin"]["kind"], "missing");
        assert_eq!(u["value"], serde_json::Value::Null);
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
  aws_profile: zagros
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

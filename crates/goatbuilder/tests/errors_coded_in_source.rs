//! Gate row C5, in the source: every error goatbuilder can return carries its
//! catalog code (docs/design/goatbuilder-v1-gate.md, row C5: "the drill logs
//! contain no error without a GB-nnn code").
//!
//! An error that leaves main.rs without a code prints `goatbuilder: <text>` and
//! nothing else whenever stderr is not a terminal and the explanation is off:
//! exactly what a drill log keeps. `tests/refusals_coded.rs` holds the
//! command-line corpus to its codes; this test holds the source.
//!
//! It reads every file under `src/` (test modules and `#[cfg(test)]` items
//! left out) for the places an error is made: `anyhow!(…)`, `bail!(…)`,
//! `ensure!(…)`, `.context(…)` and `.with_context(…)`, and a bare `?` on a
//! standard library or serde call (a file, a program, a JSON or YAML read) in a
//! function that returns anyhow's `Result`. A place is coded when its expression
//! calls `coded(`; a message that only names a `GB-nnn` code is not coded,
//! since the error is not `Coded` and takes the uncoded exit. A message that only meets one of the
//! catalog's `matches:` patterns is not: main.rs prints a code without the
//! explanation only for an error raised with `goat_errors::coded`, so a drill
//! log of a pattern-matched error carries none. Every other place is listed in
//! [`BASELINE`], one line each, as `<file> :: <kind> :: <message>`.
//!
//! The list only shrinks. A new uncoded place fails this test; a listed place
//! that is coded now, or gone, fails it too until its line is removed. Some
//! listed places may already be coded by the caller that wraps them; showing
//! that is part of removing the line, not a reason to keep it.
//!
//! `GOATBUILDER_UNCODED=write cargo test -p goatbuilder --test
//! errors_coded_in_source` rewrites the list from the source: for removing
//! lines after a fix, never for adding them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The list of uncoded places, relative to the crate.
const BASELINE: &str = "tests/errors-uncoded-baseline.txt";

/// One place an error is made.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    kind: &'static str,
    message: String,
}

impl Site {
    fn line(&self) -> String {
        format!("{} :: {} :: {}", self.file, self.kind, self.message)
    }
}

fn crate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The kinds of place an error is made, by the text that opens it.
const OPENERS: &[(&str, &str)] = &[
    ("anyhow::anyhow!(", "anyhow"),
    ("anyhow!(", "anyhow"),
    ("bail!(", "bail"),
    ("anyhow::ensure!(", "ensure"),
    ("ensure!(", "ensure"),
    (".with_context(", "context"),
    (".context(", "context"),
];

/// The text with every string literal's content blanked, so parentheses inside
/// a message do not count.
fn blank_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in s.chars() {
        if in_str {
            if escaped {
                escaped = false;
                out.push(' ');
            } else if c == '\\' {
                escaped = true;
                out.push(' ');
            } else if c == '"' {
                in_str = false;
                out.push('"');
            } else {
                out.push(' ');
            }
        } else {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
        }
    }
    out
}

/// The first string literal in `s`, its escapes kept as written.
fn first_literal(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let mut lit = String::new();
    let mut escaped = false;
    for c in s[start + 1..].chars() {
        if escaped {
            lit.push(c);
            escaped = false;
        } else if c == '\\' {
            lit.push(c);
            escaped = true;
        } else if c == '"' {
            return Some(lit);
        } else {
            lit.push(c);
        }
    }
    None
}

/// The lines of `text` that are not test code: each `#[cfg(test)]` item (a
/// module, a function, an impl) is left out up to the line that closes it at
/// its own indentation.
fn non_test_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim_start().starts_with("#[cfg(test)]") {
            let indent = &l[..l.len() - l.trim_start().len()];
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim_start().starts_with("#[") {
                j += 1;
            }
            if j < lines.len() && lines[j].trim_end().ends_with(';') {
                i = j + 1;
                continue;
            }
            let close = format!("{indent}}}");
            let mut k = j;
            while k < lines.len() && lines[k].trim_end() != close && lines[k].trim_end() != format!("{close};") {
                k += 1;
            }
            i = k + 1;
            continue;
        }
        keep.push(l);
        i += 1;
    }
    keep
}

/// A message on one line: a string continued with `\` at the end of a line,
/// and every run of whitespace, read as one space.
fn one_line(s: &str) -> String {
    s.replace("\\\n", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every uncoded place in one file's text.
fn uncoded_in(file: &str, text: &str) -> Vec<Site> {
    let lines = non_test_lines(text);
    let mut out = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut at = 0;
        while let Some((pos, opener, kind)) = OPENERS
            .iter()
            .filter_map(|(o, k)| line[at..].find(o).map(|p| (at + p, *o, *k)))
            .min_by_key(|(p, _, _)| *p)
        {
            // `anyhow!(` also matches inside `anyhow::anyhow!(`, and `ensure!(`
            // inside `anyhow::ensure!(`: take the longer.
            let start = pos;
            at = pos + opener.len();
            if (opener == "anyhow!(" || opener == "ensure!(") && line[..start].ends_with("anyhow::") {
                continue;
            }
            // The expression: from the opener to the line where its parenthesis closes.
            let mut expr = line[start..].to_string();
            let mut depth: i32 = 0;
            let mut m = n;
            loop {
                let seg = if m == n { &line[start..] } else { lines[m] };
                for c in blank_strings(seg).chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                if depth <= 0 || m + 1 >= lines.len() || m >= n + 20 {
                    break;
                }
                m += 1;
                expr.push('\n');
                expr.push_str(lines[m]);
            }
            // The rest of the statement can still code it (`.map_err(|e| coded(…))`).
            let mut tail = String::new();
            let mut t = m;
            while t + 1 < lines.len() && t < m + 3 && !lines[t].trim_end().ends_with(';') {
                t += 1;
                tail.push_str(lines[t]);
            }
            let whole = format!("{expr}\n{tail}");
            // Only a real `coded(…)` construction codes it: a message that names a
            // code is still an uncoded error, and exits as one.
            if whole.contains("coded(") {
                continue;
            }
            let message = match first_literal(&expr).map(|l| one_line(&l)) {
                Some(l) => l,
                None => {
                    let inner = expr[opener.len()..].split_whitespace().collect::<Vec<_>>().join(" ");
                    format!("<{}>", inner.chars().take(60).collect::<String>())
                }
            };
            out.push(Site { file: file.to_string(), kind, message });
        }
        // A bare `?` on a standard library or serde call passes the foreign error
        // up with no code. In a function that returns io::Result or Option the `?`
        // is that function's own business, and its caller is where it is coded.
        for m in bare_question().find_iter(line) {
            if !returns_anyhow(&lines, n) {
                continue;
            }
            let call = m.as_str().trim_end_matches('?').trim_end();
            let call = call.split_whitespace().collect::<Vec<_>>().join(" ");
            let message = format!("<{}>", call.chars().take(60).collect::<String>());
            out.push(Site { file: file.to_string(), kind: "question", message });
        }
        // A directory listing's entry is a `Result` of its own: `let e = e?;` in a
        // loop over `read_dir(…)` passes the system's error up with no code.
        if let Some(var) = read_dir_loop_var(line) {
            if returns_anyhow(&lines, n) {
                let bare = format!("let {var} = {var}?;");
                if lines.iter().skip(n + 1).take(3).any(|l| l.split_whitespace().collect::<String>() == bare.replace(' ', "")) {
                    out.push(Site { file: file.to_string(), kind: "question", message: format!("<read_dir entry {var}>") });
                }
            }
        }
    }
    out
}

/// The loop variable of a `for <var> in …read_dir(…)` line, if it is one.
fn read_dir_loop_var(line: &str) -> Option<&str> {
    let t = line.trim_start().strip_prefix("for ")?;
    let (var, rest) = t.split_once(" in ")?;
    (rest.contains("read_dir(") && var.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(var)
}

/// A standard library or serde call whose error a bare `?` passes up: a file
/// read, written, cloned or asked its type, a program started or waited for, a
/// JSON or YAML read or written.
fn bare_question() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(concat!(
            r"(?:\b(?:std::fs::\w+|fs::\w+|File::(?:open|create)|serde_json::\w+|serde_yaml::\w+)",
            r"\s*\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)",
            r"|\.(?:output|status|spawn|wait|try_clone|file_type|read_line|write_all)\((?:[^()]|\([^()]*\))*\))\s*\?",
        ))
        .unwrap()
    })
}

/// Whether the function around line `n` returns anyhow's `Result` (one type
/// argument), as goatbuilder's functions do; not `io::Result`, `Option` or a
/// `Result` with its own error type.
fn returns_anyhow(lines: &[&str], n: usize) -> bool {
    let Some(start) = (0..=n).rev().find(|&i| {
        let t = lines[i].trim_start();
        (t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ") || t.contains(" fn ")) && t.contains('(')
    }) else {
        return false;
    };
    // The signature may run over several lines, up to the body's brace.
    let mut sig = String::new();
    for l in lines.iter().skip(start).take(12) {
        sig.push_str(l);
        sig.push(' ');
        if l.contains('{') {
            break;
        }
    }
    let Some((_, ret)) = sig.split_once("->") else { return false };
    let ret = ret.trim();
    (ret.starts_with("Result<") || ret.starts_with("anyhow::Result<"))
        && !ret.contains("io::Result")
        && !ret.split_once('<').map(|(_, a)| a.contains(',')).unwrap_or(false)
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// Every uncoded place under `src/`.
fn uncoded() -> Vec<Site> {
    let root = crate_dir();
    let mut files = Vec::new();
    rs_files(&root.join("src"), &mut files);
    let mut out = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        // Test support compiled only into tests.
        if rel == "src/test_support.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        out.extend(uncoded_in(&rel, &text));
    }
    out.sort();
    out
}

fn counts(lines: impl Iterator<Item = String>) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for l in lines {
        *m.entry(l).or_insert(0) += 1;
    }
    m
}

fn read_baseline() -> Vec<String> {
    std::fs::read_to_string(crate_dir().join(BASELINE))
        .unwrap_or_default()
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_error_the_source_makes_is_coded_or_listed() {
    let found = uncoded();
    if std::env::var("GOATBUILDER_UNCODED").as_deref() == Ok("write") {
        let head = "# Places in crates/goatbuilder/src that make an error with no catalog code\n\
                    # (gate row C5; tests/errors_coded_in_source.rs). One line per place:\n\
                    # <file> :: <kind> :: <message>. The list only shrinks: code a place and\n\
                    # remove its line; never add one.\n";
        let body: String = found.iter().map(|s| format!("{}\n", s.line())).collect();
        std::fs::write(crate_dir().join(BASELINE), format!("{head}{body}")).unwrap();
        return;
    }
    let have = counts(found.iter().map(Site::line));
    let listed = counts(read_baseline().into_iter());
    let mut problems = Vec::new();
    for (line, n) in &have {
        let k = listed.get(line).copied().unwrap_or(0);
        if *n > k {
            problems.push(format!(
                "new uncoded error ({} more than listed): {line}\n    raise it with goat_errors::coded(\"GB-nnn\", …) and a catalog entry",
                n - k
            ));
        }
    }
    for (line, k) in &listed {
        let n = have.get(line).copied().unwrap_or(0);
        if *k > n {
            problems.push(format!(
                "listed but no longer uncoded ({} fewer): {line}\n    remove its line from crates/goatbuilder/{BASELINE}",
                k - n
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} problem(s) against crates/goatbuilder/{BASELINE}:\n{}\n",
        problems.len(),
        problems.join("\n")
    );
}

/// The scanner itself: an uncoded `anyhow!`, `ensure!` and `.context` are found, with
/// their messages; a coded one, and one in a test module, are not; one the
/// catalog would only match by its words is, and so is one whose message only
/// names a code: the error is not `Coded`, so it takes the uncoded exit. A bare
/// `?` on a log file's clone, a child's wait, a directory entry or its type is
/// found; the same call coded is not.
#[test]
fn the_scanner_finds_uncoded_errors_and_only_those() {
    let src = r#"
fn a() -> Result<()> {
    let t = std::fs::read_to_string(p).context("the plan file")?;
    if t.is_empty() {
        return Err(anyhow!("an empty plan: {}", p.display()));
    }
    Err(goat_errors::coded("GB-008", "needs --yes").into())
}

fn b() -> Result<()> {
    Err(anyhow::anyhow!(
        "a refusal with its code, GB-149, inside the message"
    ))
}

fn d() -> Result<()> {
    Err(anyhow!("federate is a mutation: pass --yes without a terminal (C10)"))
}

fn e(ok: bool) -> Result<()> {
    anyhow::ensure!(ok, "phase 1 wrote no results to {}", p.display());
    ensure!(ok, goat_errors::coded("GB-321", "phase 1 wrote no results"));
    Ok(())
}

fn f() -> Result<()> {
    let line = parse(argv).context(format!("goatbuilder federate {} (GB-242)", argv.join(" ")))?;
    Ok(())
}

fn g(p: &Path) -> Result<()> {
    let t = std::fs::read_to_string(p)?;
    let v: Value = serde_json::from_str(&t).map_err(|e| goat_errors::coded("GB-326", e.to_string()))?;
    Ok(())
}

fn h(p: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(p)?
}

fn i(log: &str, dir: &Path) -> Result<()> {
    let f = std::fs::File::create(log).map_err(|e| goat_errors::coded("GB-254", e.to_string()))?;
    let mut child = cmd.stdout(Stdio::from(f.try_clone()?)).spawn().map_err(|e| goat_errors::coded("GB-320", e.to_string()))?;
    let status = child.wait()?;
    let done = other.wait().map_err(|e| goat_errors::coded("GB-322", e.to_string()))?;
    for e in std::fs::read_dir(dir).map_err(|e| goat_errors::coded("GB-254", e.to_string()))? {
        let e = e?;
        if e.file_type()?.is_dir() {}
    }
    for e in std::fs::read_dir(dir).map_err(|e| goat_errors::coded("GB-254", e.to_string()))? {
        let e = e.map_err(|e| goat_errors::coded("GB-254", e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn c() { let _ = anyhow!("a test's own error"); }
}
"#;
    let got: Vec<String> = uncoded_in("src/x.rs", src).iter().map(Site::line).collect();
    assert_eq!(
        got,
        vec![
            "src/x.rs :: context :: the plan file".to_string(),
            "src/x.rs :: anyhow :: an empty plan: {}".to_string(),
            "src/x.rs :: anyhow :: a refusal with its code, GB-149, inside the message".to_string(),
            "src/x.rs :: anyhow :: federate is a mutation: pass --yes without a terminal (C10)".to_string(),
            "src/x.rs :: ensure :: phase 1 wrote no results to {}".to_string(),
            "src/x.rs :: context :: goatbuilder federate {} (GB-242)".to_string(),
            "src/x.rs :: question :: <std::fs::read_to_string(p)>".to_string(),
            "src/x.rs :: question :: <.try_clone()>".to_string(),
            "src/x.rs :: question :: <.wait()>".to_string(),
            "src/x.rs :: question :: <read_dir entry e>".to_string(),
            "src/x.rs :: question :: <.file_type()>".to_string(),
        ]
    );
}

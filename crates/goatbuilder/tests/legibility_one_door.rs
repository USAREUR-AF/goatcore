//! One door for commands (docs/design/goatbuilder-legibility.md, step 2): no code in
//! goatbuilder starts a process except through `src/runner.rs`, so every process is
//! framed, logged and recorded. This test fails on a `Command::new` anywhere else in
//! `src/`, outside a `#[cfg(test)]` module.
//!
//! The files that still start processes themselves are listed, with their count, in
//! `tests/legibility-one-door-allow.txt`, while their call sites move to the runner.
//! A count may only go down: a file over its count fails, a file under it fails until
//! the line is lowered (or removed at zero), and an unlisted file fails.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ALLOW: &str = "tests/legibility-one-door-allow.txt";

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(d: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `text` with the inside of every string and char literal blanked and every
/// comment removed, so neither a `//` in a string nor a brace or a call written
/// in one is read as code. Line breaks are kept.
fn code_only(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        // A line comment, to its line's end.
        if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // A block comment (not nested here; the crate has none nested).
        if ch == '/' && next == Some('*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                if c[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // A raw string: r"..." or r#"..."#, any number of #.
        if ch == 'r'
            && (next == Some('"') || next == Some('#'))
            && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_'))
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while c.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if c.get(j) == Some(&'"') {
                j += 1;
                loop {
                    if j >= c.len() {
                        break;
                    }
                    if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    if c[j] == '\n' {
                        out.push('\n');
                    }
                    j += 1;
                }
                out.push_str("\"\"");
                i = j;
                continue;
            }
        }
        // A string.
        if ch == '"' {
            let mut j = i + 1;
            while j < c.len() && c[j] != '"' {
                if c[j] == '\\' {
                    j += 1;
                }
                if c.get(j) == Some(&'\n') {
                    out.push('\n');
                }
                j += 1;
            }
            out.push_str("\"\"");
            i = j + 1;
            continue;
        }
        // A char literal ('x', '\n', '\''), not a lifetime ('a).
        if ch == '\'' {
            if next == Some('\\') {
                let mut j = i + 2;
                while j < c.len() && c[j] != '\'' {
                    j += 1;
                }
                out.push_str("' '");
                i = j + 1;
                continue;
            }
            if c.get(i + 2) == Some(&'\'') {
                out.push_str("' '");
                i += 3;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Constructions of std's `Command` in `text`, outside `#[cfg(test)]` items,
/// comments and strings: `Command::new(`, `std::process::Command::new(`, and
/// `X::new(` for every name the file gives it (`use … Command as X`,
/// `type X = Command;`).
fn spawns(text: &str) -> usize {
    let code = code_only(text);
    let mut names = vec!["Command".to_string()];
    let alias = regex::Regex::new(r"\bCommand\s+as\s+([A-Za-z_]\w*)").unwrap();
    let ty = regex::Regex::new(
        r"\btype\s+([A-Za-z_]\w*)\s*=\s*(?:std\s*::\s*process\s*::\s*)?Command\s*;",
    )
    .unwrap();
    names.extend(alias.captures_iter(&code).map(|c| c[1].to_string()));
    names.extend(ty.captures_iter(&code).map(|c| c[1].to_string()));
    let call = regex::Regex::new(&format!(
        r"(?:^|[^\w])(?:{})\s*::\s*new\s*\(",
        names.join("|")
    ))
    .unwrap();
    let mut n = 0;
    let mut skip_depth: Option<i64> = None;
    let mut depth: i64 = 0;
    let mut pending_cfg_test = false;
    for line in code.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        let opens = line.matches('{').count() as i64;
        let closes = line.matches('}').count() as i64;
        if pending_cfg_test {
            let brace = line.find('{');
            let semi = line.find(';');
            match (brace, semi) {
                // `#[cfg(test)] use …;`: the attributed item ends here, without a body.
                (None, Some(_)) => pending_cfg_test = false,
                (Some(b), Some(s)) if s < b => pending_cfg_test = false,
                (Some(_), _) => {
                    pending_cfg_test = false;
                    skip_depth = Some(depth);
                }
                (None, None) => {}
            }
        }
        if skip_depth.is_none() {
            n += call.find_iter(line).count();
        }
        depth += opens - closes;
        if let Some(d) = skip_depth {
            if depth <= d {
                skip_depth = None;
            }
        }
    }
    n
}

fn counts() -> BTreeMap<String, usize> {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    files
        .iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(src())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let n = spawns(&std::fs::read_to_string(p).unwrap());
            (n > 0 && rel != "runner.rs").then_some((rel, n))
        })
        .collect()
}

fn allowed() -> BTreeMap<String, usize> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ALLOW);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut w = line.split_whitespace();
        let (file, n) = (w.next().unwrap(), w.next().and_then(|n| n.parse().ok()));
        let n = n.unwrap_or_else(|| panic!("{ALLOW}:{}: want `<file under src/> <count>`", i + 1));
        assert!(
            out.insert(file.to_string(), n).is_none(),
            "{ALLOW}:{}: {file} listed twice",
            i + 1
        );
    }
    out
}

#[test]
fn every_process_goes_through_the_runner() {
    let (have, allow) = (counts(), allowed());
    let mut problems = Vec::new();
    for (file, n) in &have {
        match allow.get(file) {
            None => problems.push(format!(
                "src/{file} starts {n} process(es) itself: use crate::runner::Cmd"
            )),
            Some(a) if n > a => problems.push(format!(
                "src/{file} starts {n} process(es) itself, {a} allowed: use crate::runner::Cmd"
            )),
            Some(a) if n < a => problems.push(format!(
                "src/{file} is down to {n}: lower its line in {ALLOW} from {a}"
            )),
            _ => {}
        }
    }
    for (file, a) in &allow {
        if !have.contains_key(file) {
            problems.push(format!(
                "src/{file} starts no process itself now: remove its line ({a}) from {ALLOW}"
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

/// Each way the door has been got round counts (ci-watch's review of step 2a).
#[test]
fn the_count_sees_every_way_to_build_a_command() {
    // An alias of std's Command.
    assert_eq!(
        spawns("use std::process::Command as Spawn;\nfn a() { Spawn::new(\"x\"); }\n"),
        1
    );
    assert_eq!(
        spawns("use std::process::{Command as Run, Stdio};\nfn a() { Run::new(\"x\"); }\n"),
        1
    );
    assert_eq!(
        spawns("type Proc = std::process::Command;\nfn a() { Proc::new(\"x\"); }\n"),
        1
    );
    // `#[cfg(test)]` on an item with no body leaves the next item counted.
    assert_eq!(
        spawns("#[cfg(test)]\nuse std::fs;\nfn b() { Command::new(\"y\"); }\n"),
        1
    );
    // `//` inside a string is not a comment; a call in a string is not a call.
    assert_eq!(
        spawns("fn c() { let u = \"http://x\"; Command::new(\"z\"); }\n"),
        1
    );
    assert_eq!(
        spawns("fn d() { let t = \"Command::new(\"; let r = r#\"Command::new(\"#; }\n"),
        0
    );
    // A brace in a string or a char does not end a test module early.
    assert_eq!(spawns("#[cfg(test)]\nmod tests {\n    fn e() { let b = \"}\"; let c = '}'; Command::new(\"t\"); }\n}\nfn f() { Command::new(\"g\"); }\n"), 1);
}

#[test]
fn the_count_skips_test_modules_and_comments() {
    let text = "fn a() { Command::new(\"x\"); }\n// Command::new(\"y\")\n#[cfg(test)]\nmod tests {\n    fn b() { Command::new(\"z\"); }\n}\nfn c() { std::process::Command::new(\"w\"); }\n";
    assert_eq!(spawns(text), 2);
}

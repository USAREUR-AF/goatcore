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

/// `Command::new(` in `text`, outside `#[cfg(test)]` items and outside comments.
fn spawns(text: &str) -> usize {
    let mut n = 0;
    let mut skip_depth: Option<i64> = None;
    let mut depth: i64 = 0;
    let mut pending_cfg_test = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("");
        let trimmed = raw.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        let opens = line.matches('{').count() as i64;
        let closes = line.matches('}').count() as i64;
        if pending_cfg_test && opens > 0 {
            pending_cfg_test = false;
            skip_depth = Some(depth);
        }
        if skip_depth.is_none() {
            n += line.matches("Command::new(").count();
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

#[test]
fn the_count_skips_test_modules_and_comments() {
    let text = "fn a() { Command::new(\"x\"); }\n// Command::new(\"y\")\n#[cfg(test)]\nmod tests {\n    fn b() { Command::new(\"z\"); }\n}\nfn c() { std::process::Command::new(\"w\"); }\n";
    assert_eq!(spawns(text), 2);
}

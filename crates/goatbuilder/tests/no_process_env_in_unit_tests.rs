//! goatbuilder's tests share one process per test binary and run in parallel, so a
//! test that sets or removes a process environment variable changes what another
//! test reads at the same moment. A test passes the environment it needs instead:
//! an `_with`/`_in` variant of the function, or a value held for its own thread.
//! This test reads the source and fails on any `std::env::set_var` or
//! `std::env::remove_var` inside a `#[cfg(test)]` module of `src/`, whatever the
//! module is called, or anywhere in `tests/`, except the ones listed below with
//! the reason they remain.

use std::path::{Path, PathBuf};

/// (file, text on the line, why it remains)
const KNOWN: &[(&str, &str, &str)] = &[];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries {
        let p = e.unwrap().path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn writes_env(line: &str) -> bool {
    let l = line.trim_start();
    !l.starts_with("//") && (l.contains("env::set_var(") || l.contains("env::remove_var("))
}

/// The 1-based line ranges of every `#[cfg(test)]` module in `text`, whatever its
/// name: from the `mod <name> {` that follows the attribute to its closing brace.
fn test_modules(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" {
            // The item the attribute applies to: skip further attributes and blank lines.
            let mut j = i + 1;
            while j < lines.len() && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with("#[")) {
                j += 1;
            }
            let item = lines.get(j).map(|l| l.trim_start()).unwrap_or("");
            let is_mod = item.starts_with("mod ") || item.starts_with("pub mod ") || item.starts_with("pub(crate) mod ");
            if is_mod && item.trim_end().ends_with('{') {
                let mut depth = 0i32;
                let mut k = j;
                while k < lines.len() {
                    depth += lines[k].matches('{').count() as i32 - lines[k].matches('}').count() as i32;
                    if depth == 0 {
                        break;
                    }
                    k += 1;
                }
                out.push((j + 1, k + 1));
                i = k;
            }
        }
        i += 1;
    }
    out
}

/// Every unlisted write in one file: in `src/`, inside a test module; in `tests/`, anywhere.
fn scan(rel: &str, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let ranges: Vec<(usize, usize)> = if rel.starts_with("tests/") { vec![(1, lines.len())] } else { test_modules(text) };
    let mut found = Vec::new();
    for (from, to) in ranges {
        for n in from..=to.min(lines.len()) {
            let l = lines[n - 1];
            if writes_env(l) && !KNOWN.iter().any(|(file, needle, _)| rel == *file && l.contains(needle)) {
                found.push(format!("{rel}:{n}: {}", l.trim()));
            }
        }
    }
    found
}

#[test]
fn no_test_writes_the_process_environment() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let me = Path::new(file!()).file_name().unwrap().to_owned();
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    sources(&root.join("tests"), &mut files);
    files.sort();
    let mut found = Vec::new();
    for f in &files {
        if f.file_name() == Some(me.as_os_str()) {
            continue; // this file names the calls it looks for
        }
        let rel = f.strip_prefix(root).unwrap().display().to_string();
        found.extend(scan(&rel, &std::fs::read_to_string(f).unwrap()));
    }
    assert!(found.is_empty(), "a test writes the process environment, which tests running in parallel read; pass it in instead:\n{}", found.join("\n"));
}

/// The scan finds a write in a test module of any name, and in tests/, and leaves
/// product code and comments alone.
#[test]
fn the_scan_finds_a_write_in_a_test_module_of_any_name() {
    let planted = "fn product() {\n    std::env::set_var(\"A\", \"1\");\n}\n\n#[cfg(test)]\n#[allow(dead_code)]\nmod helpers {\n    fn h() {\n        // std::env::set_var(\"B\", \"2\");\n        if true {\n            std::env::remove_var(\"C\");\n        }\n    }\n}\n";
    let found = scan("src/x.rs", planted);
    assert_eq!(found, vec!["src/x.rs:11: std::env::remove_var(\"C\");".to_string()], "only the write inside the test module, not the comment or product code");
    let in_tests = scan("tests/x.rs", "fn t() {\n    std::env::set_var(\"D\", \"4\");\n}\n");
    assert_eq!(in_tests.len(), 1, "anywhere in tests/");
    assert_eq!(test_modules("#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n"), vec![(2, 5)]);
}

/// federate drives phases and, in process, the federation library's verbs, on
/// one thread while the rest of the process keeps running. A `set_var` there is
/// unsound in a multi-threaded process, so nothing under `src/federate/` writes
/// the process environment, product code included: a verb's values reach it as
/// values (runenv, and the library's `_with` hooks), never through the process.
#[test]
fn federate_never_writes_the_process_environment() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src/federate"), &mut files);
    files.sort();
    let mut found = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap().display().to_string();
        for (n, l) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            if writes_env(l) {
                found.push(format!("{rel}:{}: {}", n + 1, l.trim()));
            }
        }
    }
    assert!(found.is_empty(), "federate writes the process environment; pass the value in instead:\n{}", found.join("\n"));
}

/// The product code that may write the process environment: in `main`, before any
/// thread starts, where `set_var` is sound. Each line names the file, the text on
/// the line, and what reads the value.
const MAIN_ONLY: &[(&str, &str, &str)] = &[
    (
        "src/main.rs",
        "set_var(\"GOAT_DEPLOY_OFFLINE\"",
        "--offline: the plays and scripts every child runs read it",
    ),
    (
        "src/awake.rs",
        "set_var(MARKER",
        "the keep-awake marker a nested goatbuilder reads, so it holds no second assertion",
    ),
];

/// goatbuilder runs threads once a verb starts (the runner's output copies, survey),
/// and `set_var` while another thread runs is unsound. So outside `main`'s first
/// lines, product code passes a value to the child that reads it (`Cmd::env`,
/// run state) and never writes the process environment. `src/federate/` has its
/// own test (`federate_never_writes_the_process_environment`, with the in-process
/// init change) and is left to it.
#[test]
fn product_code_writes_the_process_environment_only_in_main_before_threads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    files.sort();
    let mut found = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap().display().to_string();
        if rel.starts_with("src/federate/") {
            continue;
        }
        for (n, l) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            if writes_env(l)
                && !MAIN_ONLY
                    .iter()
                    .any(|(file, needle, _)| rel == *file && l.contains(needle))
            {
                found.push(format!("{rel}:{}: {}", n + 1, l.trim()));
            }
        }
    }
    assert!(found.is_empty(), "product code writes the process environment while threads may run; pass the value to the child instead:\n{}", found.join("\n"));
    // Each allowed site is still there: an entry that matches nothing is removed.
    for (file, needle, _) in MAIN_ONLY {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            text.lines().any(|l| writes_env(l) && l.contains(needle)),
            "{file}: no `{needle}` left; remove it from MAIN_ONLY"
        );
    }
}

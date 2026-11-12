//! goatbuilder's unit tests share one process and run in parallel, so a test that
//! sets or removes a process environment variable changes what another test reads
//! at the same moment (a flake seen at 64 threads). A unit test passes the
//! environment it needs instead: an `_with`/`_in` variant of the function, or a
//! value for its own thread. This test reads the source and fails on any
//! `std::env::set_var` or `std::env::remove_var` inside a `#[cfg(test)]` module,
//! except the ones listed below with the reason they are safe.

use std::path::{Path, PathBuf};

/// (file, text on the line, why it is safe)
const KNOWN: &[(&str, &str, &str)] = &[
    ("federate/mod.rs", "remove_var(key)", "key is GOATBUILDER_TEST_VERB_ENV, a name no other code or test reads"),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_unit_test_writes_the_process_environment() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let Some(start) = text.find("#[cfg(test)]\nmod tests") else { continue };
        let rel = f.strip_prefix(&src).unwrap().display().to_string();
        for (n, line) in text[start..].lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with("//") || !(l.contains("std::env::set_var(") || l.contains("std::env::remove_var(")) {
                continue;
            }
            if KNOWN.iter().any(|(file, var, _)| rel == *file && l.contains(var)) {
                continue;
            }
            let at = text[..start].lines().count() + n + 1;
            found.push(format!("{rel}:{at}: {l}"));
        }
    }
    assert!(found.is_empty(), "a unit test writes the process environment, which tests running in parallel read; pass it in instead:\n{}", found.join("\n"));
}

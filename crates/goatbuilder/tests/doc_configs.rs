//! Every whole configuration example the documents ship validates with this build.
//!
//! A reader copies a ```yaml block that holds `schema: goatbuilder/v1` and runs
//! `goatbuilder validate` on it. This test does the same for every such block
//! under docs/: each is written as its own `goatbuilder.yaml` in a directory of
//! its own and run through the built binary, which must exit 0.
//!
//! A block meant to be refused says so on its first line, with the code it is
//! refused with: `# goatbuilder validate refuses this: GB-nnn`. The test then
//! requires a nonzero exit and that code in the output ([`judge`]).
//!
//! A block is read only when its fence is tagged ```yaml or ```yml and one of
//! its lines is exactly `schema: goatbuilder/v1`. So a fragment (no schema
//! line), a quoted `schema: "goatbuilder/v1"`, a schema line with a trailing
//! comment, and a block under an untagged ``` fence are all skipped: write a
//! whole example in the plain form for it to be checked.

#[path = "common/test_home.rs"]
mod test_home;

use std::path::{Path, PathBuf};

const SCHEMA_LINE: &str = "schema: goatbuilder/v1";
const REFUSED: &str = "# goatbuilder validate refuses this: ";

/// One ```yaml block: the document, the line its fence opens on, its text.
struct Block {
    doc: PathBuf,
    line: usize,
    text: String,
}

/// Every ```yaml or ```yml block in `text` that holds the schema line.
fn blocks_in(doc: &Path, text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (i, l) in text.lines().enumerate() {
        let t = l.trim_start();
        match &mut open {
            None if t.starts_with("```yaml") || t.starts_with("```yml") => {
                open = Some((i + 1, String::new()));
            }
            Some((line, body)) if t.starts_with("```") => {
                if body.lines().any(|b| b.trim() == SCHEMA_LINE) {
                    out.push(Block {
                        doc: doc.to_path_buf(),
                        line: *line,
                        text: std::mem::take(body),
                    });
                }
                open = None;
            }
            Some((_, body)) => {
                body.push_str(l);
                body.push('\n');
            }
            None => {}
        }
    }
    out
}

fn docs(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            docs(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// The verdict on one example: validate's exit code, its output, and the code
/// the block's first line says it is refused with, if any. Unmarked, it must
/// exit 0. Marked, it must exit nonzero and print that code.
fn judge(exit: Option<i32>, output: &str, mark: Option<&str>) -> Result<(), String> {
    let first = |n: usize| {
        output
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(n)
            .collect::<Vec<_>>()
            .join("\n    ")
    };
    match mark {
        None if exit != Some(0) => Err(format!(
            "does not validate (exit {exit:?}):\n    {}",
            first(6)
        )),
        Some(code) if exit == Some(0) || !output.contains(code) => Err(format!(
            "is marked as refused with {code}, but validate exited {exit:?}:\n    {}",
            first(4)
        )),
        _ => Ok(()),
    }
}

#[test]
fn every_whole_config_example_in_the_docs_validates() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    docs(&repo.join("docs"), &mut files);
    files.sort();
    let blocks: Vec<Block> = files
        .iter()
        .flat_map(|f| {
            blocks_in(
                f.strip_prefix(&repo).unwrap(),
                &std::fs::read_to_string(f).unwrap(),
            )
        })
        .collect();
    // The documents hold at least the four examples this test was written for:
    // an extractor that finds none would pass for nothing.
    assert!(
        blocks.len() >= 4,
        "only {} configuration examples found under docs/",
        blocks.len()
    );
    let home = test_home::path();
    let mut wrong = Vec::new();
    for (n, b) in blocks.iter().enumerate() {
        let dir = home.join(format!("block-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("goatbuilder.yaml"), &b.text).unwrap();
        let out = test_home::isolate(&mut std::process::Command::new(env!(
            "CARGO_BIN_EXE_goatbuilder"
        )))
        .current_dir(&dir)
        .args(["validate", "goatbuilder.yaml"])
        .env("GOAT_EXPLAIN", "0")
        .output()
        .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let at = format!("{}:{}", b.doc.display(), b.line);
        let mark = b
            .text
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix(REFUSED))
            .map(str::trim);
        if let Err(why) = judge(out.status.code(), &text, mark) {
            wrong.push(format!("{at} {why}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "configuration examples in the docs:\n  {}",
        wrong.join("\n  ")
    );
}

/// The extractor reads whole blocks, skips fragments, and reads a refused mark.
#[test]
fn the_extractor_reads_whole_blocks_and_skips_fragments() {
    let doc = "x\n```yaml\nschema: goatbuilder/v1\nrepo: {}\n```\n\n```yaml\ncomponents: {}\n```\n```yml\n# goatbuilder validate refuses this: GB-025\nschema: goatbuilder/v1\n```\n";
    let b = blocks_in(Path::new("d.md"), doc);
    assert_eq!(b.len(), 2, "two whole blocks, one fragment skipped");
    assert_eq!(b[0].line, 2);
    assert_eq!(b[0].text, "schema: goatbuilder/v1\nrepo: {}\n");
    assert!(b[1].text.starts_with(REFUSED));
}

/// The judgement, on each way a marked or unmarked example can come out.
#[test]
fn the_judgement_holds_a_refused_example_to_its_code() {
    let refused = "FAIL  config  GB-025: environments/0/name: \"X\" does not match\n";
    let other = "goatbuilder: goatbuilder.yaml is not valid YAML: ...\n  (GB-245 \u{2014} goatbuilder explain GB-245)\n";
    // Unmarked: valid passes, refused fails.
    assert!(judge(Some(0), "validate: ok", None).is_ok());
    assert!(judge(Some(1), refused, None).is_err());
    // Marked and refused with that code passes.
    assert!(judge(Some(1), refused, Some("GB-025")).is_ok());
    // Marked, but refused with another code, fails.
    let e = judge(Some(2), other, Some("GB-025")).unwrap_err();
    assert!(e.contains("marked as refused with GB-025"), "{e}");
    // Marked, but valid, fails.
    let e = judge(Some(0), "validate: ok", Some("GB-025")).unwrap_err();
    assert!(e.contains("exited Some(0)"), "{e}");
}

/// What the extractor does not read: a quoted schema, a trailing comment on the
/// schema line, and an untagged fence.
#[test]
fn the_extractor_skips_the_forms_it_does_not_read() {
    for doc in [
        "```yaml\nschema: \"goatbuilder/v1\"\n```\n",
        "```yaml\nschema: goatbuilder/v1   # the version\n```\n",
        "```\nschema: goatbuilder/v1\n```\n",
    ] {
        assert!(blocks_in(Path::new("d.md"), doc).is_empty(), "{doc}");
    }
}

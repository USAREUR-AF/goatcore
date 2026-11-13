//! Every whole configuration example the documents ship validates with this build.
//!
//! A reader copies a ```yaml block that holds `schema: goatbuilder/v1` and runs
//! `goatbuilder validate` on it. This test does the same for every such block
//! under docs/: each is written as its own `goatbuilder.yaml` in a directory of
//! its own and run through the built binary, which must exit 0.
//!
//! A block meant to be refused says so on its first line, with the code it is
//! refused with: `# goatbuilder validate refuses this: GB-nnn`. The test then
//! requires a nonzero exit and that code in the output. A fragment (a block
//! without the schema line) is not a whole configuration and is not read.

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
        let refused = b
            .text
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix(REFUSED))
            .map(str::trim);
        match refused {
            None if !out.status.success() => wrong.push(format!(
                "{at} does not validate (exit {:?}):\n    {}",
                out.status.code(),
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(6)
                    .collect::<Vec<_>>()
                    .join("\n    ")
            )),
            Some(code) if out.status.success() || !text.contains(code) => wrong.push(format!(
                "{at} is marked as refused with {code}, but validate exited {:?}:\n    {}",
                out.status.code(),
                text.lines().take(4).collect::<Vec<_>>().join("\n    ")
            )),
            _ => {}
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

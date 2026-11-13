//! The run record (docs/design/goatbuilder-legibility.md, step 4): one
//! `<state>/runs/<id>/run.json` for every run of a verb that drives hosts, and the
//! verbs that read them back: `goatbuilder runs`, `goatbuilder show <run>`,
//! `goatbuilder runs diff <a> <b>`.
//!
//! A record holds the tool's version, the tree's commit, the verb and its
//! arguments, each phase and each command (its line, argv, host, log, start, end,
//! exit), and the run's inputs with their sources. It never holds a secret value:
//! commands carry `$(printenv NAME)`, not values, and an input marked secret has no
//! value. Records are kept until the operator removes them.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::runner::Finished;
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

/// The record's schema; the version changes when a field changes meaning.
pub const SCHEMA: &str = "goatbuilder.run/v1";

#[derive(Serialize)]
struct Record<'a> {
    schema: &'static str,
    id: String,
    tool_version: &'static str,
    tree_commit: Option<&'a str>,
    verb: &'a str,
    args: &'a [String],
    started: &'a str,
    ended: String,
    exit: i32,
    /// The error that ended the run, as the tool printed it.
    error: Option<&'a str>,
    phases: &'a [crate::runner::PhaseEntry],
    commands: &'a [crate::runner::CommandEntry],
    inputs: &'a serde_json::Value,
}

/// Write the record of a run that ended with `exit` (and `error`, if it failed
/// with one). Returns where it was written.
pub fn write(run: &Finished, exit: i32, error: Option<&str>) -> Result<PathBuf> {
    let id = run
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A secret's value never reaches the file, whatever the verb handed over.
    let inputs = without_secret_values(&run.inputs);
    let rec = Record {
        schema: SCHEMA,
        id,
        tool_version: env!("CARGO_PKG_VERSION"),
        tree_commit: run.tree_commit.as_deref(),
        verb: &run.verb,
        args: &run.args,
        started: &run.started,
        ended: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        exit,
        error,
        phases: &run.phases,
        commands: &run.commands,
        inputs: &inputs,
    };
    std::fs::create_dir_all(&run.dir).with_context(|| format!("{}", run.dir.display()))?;
    let path = run.dir.join("run.json");
    let tmp = run.dir.join(".run.json.tmp");
    let text = serde_json::to_string_pretty(&rec)? + "\n";
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = o.open(&tmp).with_context(|| format!("{}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("{}", path.display()))?;
    Ok(path)
}

/// The checkout's commit, read from its git files rather than by starting git (a
/// process before the run begins would be outside the record). `None` when the
/// tree is not a git checkout or its HEAD cannot be resolved.
pub fn tree_commit(repo: &Path) -> Option<String> {
    let dot = repo.join(".git");
    let gitdir = if dot.is_dir() {
        dot
    } else {
        let text = std::fs::read_to_string(&dot).ok()?;
        let p = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if p.is_absolute() {
            p
        } else {
            repo.join(p)
        }
    };
    // A worktree's refs live in the common directory.
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|c| {
            let p = PathBuf::from(c.trim());
            if p.is_absolute() {
                p
            } else {
                gitdir.join(p)
            }
        });
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref:") else {
        return Some(head.to_string()).filter(|h| h.len() >= 40);
    };
    let r = r.trim();
    for dir in [Some(gitdir.clone()), common.clone()].into_iter().flatten() {
        if let Ok(c) = std::fs::read_to_string(dir.join(r)) {
            return Some(c.trim().to_string());
        }
        if let Ok(packed) = std::fs::read_to_string(dir.join("packed-refs")) {
            if let Some(c) = packed
                .lines()
                .find_map(|l| l.strip_suffix(r).map(|c| c.trim().to_string()))
            {
                return Some(c);
            }
        }
    }
    None
}

/// `inputs` with the `value` of every entry marked secret set to null.
fn without_secret_values(inputs: &serde_json::Value) -> serde_json::Value {
    let mut v = inputs.clone();
    if let Some(list) = v.as_array_mut() {
        for i in list.iter_mut() {
            if i["secret"] == true {
                if let Some(o) = i.as_object_mut() {
                    o.insert("value".into(), serde_json::Value::Null);
                }
            }
        }
    }
    v
}

/// Every record under `<state>/runs/`, oldest first (ids start with their UTC time).
fn all(state: &Path) -> Vec<(String, serde_json::Value)> {
    let Ok(rd) = std::fs::read_dir(state.join("runs")) else {
        return Vec::new();
    };
    let mut out: Vec<(String, serde_json::Value)> = rd
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(e.path().join("run.json")).ok()?;
            let v = serde_json::from_str(&text).unwrap_or_else(
                |err| serde_json::json!({ "id": id, "unreadable": err.to_string() }),
            );
            Some((id, v))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The run `which` names: an id, a unique prefix of one, or `last`.
fn find(state: &Path, which: &str) -> Result<(String, serde_json::Value)> {
    let runs = all(state);
    if which == "last" {
        return runs
            .into_iter()
            .last()
            .ok_or_else(|| anyhow!("no run is recorded under {}/runs", state.display()));
    }
    let hits: Vec<_> = runs
        .into_iter()
        .filter(|(id, _)| id.starts_with(which))
        .collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap()),
        0 => Err(anyhow!(
            "no run {which} under {}/runs; `goatbuilder runs` lists them",
            state.display()
        )),
        n => Err(anyhow!(
            "{n} runs start with {which}: {}; give more of the id",
            hits.iter()
                .map(|h| h.0.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn row(name: &str, detail: String) -> Check {
    Check {
        name: name.to_string(),
        ok: true,
        detail,
        fix: None,
    }
}

fn s<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn exit_of(v: &serde_json::Value) -> String {
    match (v["exit"].as_i64(), v["verdict"].as_str()) {
        (Some(e), _) => e.to_string(),
        (None, Some(why)) => format!("- ({why})"),
        (None, None) => "-".to_string(),
    }
}

/// `goatbuilder runs`: the recorded runs, newest first.
pub fn list(state: &Path, verb: Option<&str>, last: usize) -> Report {
    let runs = all(state);
    let mut rows: Vec<Check> = runs
        .iter()
        .rev()
        .filter(|(_, v)| verb.is_none_or(|want| s(v, "verb") == want))
        .take(last)
        .map(|(id, v)| {
            if let Some(e) = v["unreadable"].as_str() {
                return Check {
                    name: "run".into(),
                    ok: false,
                    detail: format!("{id}  unreadable: {e}"),
                    fix: None,
                };
            }
            let n = v["commands"].as_array().map_or(0, |a| a.len());
            row(
                "run",
                format!(
                    "{id}  {}  exit {}  {n} command(s)  {}",
                    s(v, "verb"),
                    exit_of(v),
                    v["args"]
                        .as_array()
                        .map(|a| a
                            .iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" "))
                        .unwrap_or_default()
                ),
            )
        })
        .collect();
    if rows.is_empty() {
        rows.push(row(
            "runs",
            format!("no run is recorded under {}/runs", state.display()),
        ));
    } else {
        rows.push(row(
            "runs",
            "goatbuilder show <id> prints one; goatbuilder runs diff <a> <b> compares two".into(),
        ));
    }
    Report::new("runs", rows, EXIT_REFUSED)
}

/// `goatbuilder show <run>`: one run, phase by phase, command by command, each with
/// its log. With `json`, the record itself.
pub fn show(state: &Path, which: &str, json: bool) -> Result<Report> {
    let (id, v) = find(state, which)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        let mut r = Report::new("show", Vec::new(), EXIT_REFUSED);
        r.printed = true;
        return Ok(r);
    }
    let mut rows = vec![row(
        "run",
        format!(
            "{id}: goatbuilder {} {}, exit {}, tool {}, tree {}",
            s(&v, "verb"),
            v["args"]
                .as_array()
                .map(|a| a
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" "))
                .unwrap_or_default(),
            exit_of(&v),
            s(&v, "tool_version"),
            v["tree_commit"].as_str().unwrap_or("unknown")
        ),
    )];
    rows.push(row(
        "time",
        format!("{} to {}", s(&v, "started"), s(&v, "ended")),
    ));
    if let Some(e) = v["error"].as_str() {
        rows.push(Check {
            name: "error".into(),
            ok: false,
            detail: e.to_string(),
            fix: None,
        });
    }
    let commands = v["commands"].as_array().cloned().unwrap_or_default();
    let mut printed_phase: Option<String> = None;
    for c in &commands {
        let ph = c["phase"].as_str().map(String::from);
        if ph != printed_phase {
            if let Some(id) = &ph {
                let entry = v["phases"]
                    .as_array()
                    .and_then(|a| a.iter().find(|p| p["id"].as_str() == Some(id.as_str())));
                let (name, purpose) = entry.map_or(("", ""), |p| (s(p, "name"), s(p, "purpose")));
                rows.push(row(
                    "phase",
                    format!("=== Phase {id} [{name}] {purpose} ==="),
                ));
            }
            printed_phase = ph;
        }
        let ok = c["exit"].as_i64() == Some(0);
        let log = c["log"]
            .as_str()
            .map(String::from)
            .or_else(|| c["log_note"].as_str().map(|n| format!("no log: {n}")))
            .unwrap_or_default();
        rows.push(Check {
            name: "cmd".into(),
            ok,
            detail: format!(
                "{}   (exit {}, {}s{}; {log})",
                s(c, "command"),
                exit_of(c),
                c["secs"].as_f64().unwrap_or(0.0),
                c["host"]
                    .as_str()
                    .map(|h| format!(", on {h}"))
                    .unwrap_or_default()
            ),
            fix: None,
        });
    }
    let inputs = v["inputs"].as_array().cloned().unwrap_or_default();
    for i in &inputs {
        let value = if i["secret"] == true {
            "(secret)".to_string()
        } else {
            i["value"]
                .as_str()
                .map_or("(unset)".to_string(), String::from)
        };
        rows.push(row(
            "input",
            format!(
                "{} = {value}  ({})",
                s(i, "name"),
                i["source"].as_str().unwrap_or("no source")
            ),
        ));
    }
    // The record is read; a failed run it describes is not a failure of `show`.
    let mut r = Report::new(
        "show",
        rows.into_iter()
            .map(|mut c| {
                c.ok = true;
                c
            })
            .collect(),
        EXIT_REFUSED,
    );
    r.exit = EXIT_OK;
    Ok(r)
}

/// `goatbuilder runs diff <a> <b>`: what changed between two runs, in their
/// arguments, inputs (value and source), commands (in order) and phase outcomes.
pub fn diff(state: &Path, a: &str, b: &str) -> Result<Report> {
    let (ia, va) = find(state, a)?;
    let (ib, vb) = find(state, b)?;
    let mut rows = vec![row(
        "runs",
        format!(
            "{ia} ({}) against {ib} ({})",
            s(&va, "verb"),
            s(&vb, "verb")
        ),
    )];
    for k in ["verb", "tool_version", "tree_commit"] {
        if va[k] != vb[k] {
            rows.push(row(k, format!("{} -> {}", va[k], vb[k])));
        }
    }
    if va["args"] != vb["args"] {
        rows.push(row("args", format!("{} -> {}", va["args"], vb["args"])));
    }
    if va["exit"] != vb["exit"] {
        rows.push(row("exit", format!("{} -> {}", exit_of(&va), exit_of(&vb))));
    }
    // Inputs, by name.
    let by_name = |v: &serde_json::Value| -> std::collections::BTreeMap<String, serde_json::Value> {
        v["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| Some((i["name"].as_str()?.to_string(), i.clone())))
            .collect()
    };
    let (na, nb) = (by_name(&va), by_name(&vb));
    let shown = |i: &serde_json::Value| {
        let value = if i["secret"] == true {
            "(secret)".to_string()
        } else {
            i["value"]
                .as_str()
                .map_or("(unset)".to_string(), String::from)
        };
        format!("{value} ({})", i["source"].as_str().unwrap_or("no source"))
    };
    for name in na
        .keys()
        .chain(nb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        match (na.get(name), nb.get(name)) {
            (Some(x), Some(y)) if x != y => rows.push(row(
                "input",
                format!("{name}: {} -> {}", shown(x), shown(y)),
            )),
            (Some(x), None) => rows.push(row("input", format!("{name}: {} -> (absent)", shown(x)))),
            (None, Some(y)) => rows.push(row("input", format!("{name}: (absent) -> {}", shown(y)))),
            _ => {}
        }
    }
    // Commands, as an ordered diff of their lines.
    let lines = |v: &serde_json::Value| -> Vec<String> {
        v["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                format!(
                    "[{}] {}",
                    c["phase"].as_str().unwrap_or("-"),
                    s(c, "command")
                )
            })
            .collect()
    };
    let (la, lb) = (lines(&va).join("\n") + "\n", lines(&vb).join("\n") + "\n");
    for change in similar::TextDiff::from_lines(&la, &lb).iter_all_changes() {
        let sign = match change.tag() {
            similar::ChangeTag::Delete => "-",
            similar::ChangeTag::Insert => "+",
            similar::ChangeTag::Equal => continue,
        };
        rows.push(row(
            "command",
            format!("{sign} {}", change.value().trim_end()),
        ));
    }
    // Phase outcomes, by id.
    let phases = |v: &serde_json::Value| -> std::collections::BTreeMap<String, String> {
        v["phases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p["id"].as_str()?.to_string(), exit_of(p))))
            .collect()
    };
    let (pa, pb) = (phases(&va), phases(&vb));
    for id in pa
        .keys()
        .chain(pb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (x, y) = (
            pa.get(id).map_or("(not run)", |s| s.as_str()),
            pb.get(id).map_or("(not run)", |s| s.as_str()),
        );
        if x != y {
            rows.push(row("phase", format!("phase {id}: exit {x} -> {y}")));
        }
    }
    if rows.len() == 1 {
        rows.push(row(
            "same",
            "the two runs ran the same commands with the same inputs and outcomes".into(),
        ));
    }
    Ok(Report::new("runs", rows, EXIT_REFUSED))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_with_no_exit_reads_as_its_verdict() {
        assert_eq!(exit_of(&serde_json::json!({ "exit": 3 })), "3");
        assert_eq!(exit_of(&serde_json::json!({ "exit": null, "verdict": "not checked: the deploy failed" })), "- (not checked: the deploy failed)");
        assert_eq!(exit_of(&serde_json::json!({ "exit": null })), "-");
    }
    use crate::runner::{CommandEntry, PhaseEntry};

    fn finished(dir: &Path, cmds: &[(&str, i32)], target: &str) -> Finished {
        Finished {
            dir: dir.to_path_buf(),
            verb: "standup".into(),
            args: vec!["standup".into(), "zagros".into()],
            tree_commit: Some("abc123".into()),
            started: "2026-11-12T00:00:00Z".into(),
            phases: vec![PhaseEntry {
                id: "5".into(),
                name: "mgmt-stack".into(),
                purpose: "p".into(),
                started: String::new(),
                ended: None,
                exit: Some(0),
                commands: cmds.len(),
                verdict: None,
            }],
            commands: cmds
                .iter()
                .map(|(c, e)| CommandEntry {
                    phase: Some("5".into()),
                    command: c.to_string(),
                    argv: vec![],
                    host: None,
                    log: Some("/l".into()),
                    log_note: None,
                    started: String::new(),
                    ended: None,
                    exit: Some(*e),
                    secs: Some(1.0),
                })
                .collect(),
            inputs: serde_json::json!([
                { "name": "target", "value": target, "secret": false, "source": "derived from --site by <site>_hosts" },
                { "name": "ZAGROS_PG_PASS", "value": null, "secret": true, "source": "environment" }
            ]),
        }
    }

    #[test]
    fn a_record_is_written_read_back_listed_shown_and_compared() {
        let t = tempfile::tempdir().unwrap();
        let state = t.path();
        let a = finished(
            &state.join("runs/20261112T000000Z-standup-1"),
            &[("one", 0), ("two", 0)],
            "zagros_hosts",
        );
        let b = finished(
            &state.join("runs/20261112T000100Z-standup-2"),
            &[("one", 0), ("three", 2)],
            "xaa_hosts",
        );
        let pa = write(&a, 0, None).unwrap();
        write(&b, 1, Some("phase 5 failed")).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pa).unwrap()).unwrap();
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["tool_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["tree_commit"], "abc123");
        assert_eq!(v["commands"].as_array().unwrap().len(), 2);
        assert_eq!(
            v["inputs"][1]["value"],
            serde_json::Value::Null,
            "a secret's value is never written"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&pa).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let l = list(state, None, 20);
        assert!(
            l.checks[0]
                .detail
                .starts_with("20261112T000100Z-standup-2  standup  exit 1"),
            "newest first: {:?}",
            l.checks
        );
        let sh = show(state, "20261112T000", false);
        assert!(sh.is_err(), "an ambiguous prefix is refused");
        let sh = show(state, "last", false).unwrap();
        assert!(sh
            .checks
            .iter()
            .any(|c| c.name == "error" && c.detail == "phase 5 failed"));
        assert!(sh
            .checks
            .iter()
            .any(|c| c.name == "input" && c.detail.starts_with("ZAGROS_PG_PASS = (secret)")));

        let d = diff(state, "20261112T000000Z", "20261112T000100Z").unwrap();
        let rows: Vec<String> = d
            .checks
            .iter()
            .map(|c| format!("{} {}", c.name, c.detail))
            .collect();
        assert!(rows.contains(&"exit 0 -> 1".to_string()), "{rows:?}");
        assert!(
            rows.iter()
                .any(|r| r.starts_with("input target: zagros_hosts")),
            "{rows:?}"
        );
        assert!(
            rows.contains(&"command - [5] two".to_string())
                && rows.contains(&"command + [5] three".to_string()),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("ZAGROS_PG_PASS")),
            "an unchanged secret input is not shown: {rows:?}"
        );
    }

    #[test]
    fn a_secret_handed_over_with_its_value_is_written_without_it() {
        let t = tempfile::tempdir().unwrap();
        let mut run = finished(
            &t.path().join("runs/20261112T000000Z-standup-1"),
            &[("one", 0)],
            "zagros_hosts",
        );
        run.inputs = serde_json::json!([
            { "name": "ZAGROS_PG_PASS", "value": "hunter2", "secret": true, "source": "environment" },
            { "name": "target", "value": "zagros_hosts", "secret": false, "source": "derived" }
        ]);
        let p = write(&run, 0, None).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("hunter2"), "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["inputs"][0]["value"], serde_json::Value::Null);
        assert_eq!(
            v["inputs"][1]["value"], "zagros_hosts",
            "a value not marked secret is kept"
        );
    }

    #[test]
    fn the_tree_commit_is_read_from_this_checkout() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let c = tree_commit(&repo).expect("this checkout has a HEAD");
        assert_eq!(c.len(), 40, "{c}");
        assert!(c.chars().all(|ch| ch.is_ascii_hexdigit()), "{c}");
        assert_eq!(tree_commit(Path::new("/nonexistent")), None);
    }

    #[test]
    fn no_runs_says_where_it_looked() {
        let t = tempfile::tempdir().unwrap();
        let l = list(t.path(), None, 20);
        assert!(l.checks[0].detail.starts_with("no run is recorded under"));
        assert!(show(t.path(), "last", false).is_err());
    }
}

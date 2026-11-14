//! `goatbuilder endstate <site>` — is the goatnet in its declared end state (ADR 1206)?
//!
//! What a site should run is derived from its own declarations, never a second
//! hand-kept list: the service catalog (`ops/goatbuilder/endstate/catalog.yaml`)
//! says what each service is, its containers, class and zero-trust edges; the
//! site's group_vars flags, the fleet pin table's `absent:` deviations and the
//! founder rule say whether the site runs it; `has_obs_host` and the inventory say
//! where, so a single-machine site runs the observability class on its one host.
//! Every present service is graded deployed, active, healthy and functional, and
//! each edge wired or broken: its served name resolves and its certificate
//! verifies, its OIDC client is registered in Hydra, its SPIRE entry exists. The
//! negative side is graded too: a container, Hydra client or SPIRE entry no
//! present service declares is an orphan.
//!
//! The script avenue runs the same file (ADR 1201): `ops/scripts/goatnet-endstate.sh`.
//! Reads only. Exit 0 in the declared end state; 3 when any grade fails or cannot
//! be read, or anything is orphaned (GB-068); 1 when it cannot run (GB-066).

use crate::runner::Cmd;
use crate::{Check, Report, EXIT_NOT_CONVERGED};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub struct Options<'a> {
    pub site: &'a str,
    /// Facts read earlier (or a fixture's), instead of reading the hosts.
    pub facts: Option<&'a Path>,
    /// The assay's rows (`goatbuilder assay --rows-out`), for the functional grade.
    pub assay_rows: Option<&'a Path>,
    /// Run the assay first and grade functional from its rows.
    pub run_assay: bool,
    pub strict: bool,
    /// A different repository root (a fixture tree).
    pub root: Option<&'a Path>,
    /// Table output streams the script's own lines; json/yaml carry the grades.
    pub stream: bool,
}

/// The Python that runs the script: `$GOATBUILDER_PYTHON`, else `python3` when it
/// has PyYAML, else Ansible's own interpreter (which always does, #2120).
pub(crate) fn python() -> String {
    if let Ok(p) = std::env::var("GOATBUILDER_PYTHON") {
        return p;
    }
    let has_yaml = |p: &str| Cmd::new(p).args(["-c", "import yaml"]).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if has_yaml("python3") {
        return "python3".into();
    }
    let ansible = Cmd::new("sh").args(["-c", "head -1 \"$(command -v ansible-playbook)\" 2>/dev/null"]).output();
    if let Ok(o) = ansible {
        let line = String::from_utf8_lossy(&o.stdout).trim().trim_start_matches("#!").trim().to_string();
        let interp = line.split_whitespace().last().unwrap_or("").to_string();
        if !interp.is_empty() && has_yaml(&interp) {
            return interp;
        }
    }
    "python3".into()
}

/// The command both avenues run.
pub fn command(repo: &Path, site: &str) -> Vec<String> {
    vec![
        python(),
        repo.join("ops/goatbuilder/endstate/endstate.py").display().to_string(),
        "grade".into(),
        site.into(),
    ]
}

pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    let root = o.root.map(absolute).unwrap_or_else(|| repo.to_path_buf());
    if !root.join(format!("ops/ansible/group_vars/{}_hosts", o.site.replace('-', "_"))).is_dir() {
        return Err(goat_errors::coded(
            "GB-066",
            format!("no goatnet site named `{}`: ops/ansible/group_vars/{}_hosts/ does not exist", o.site, o.site.replace('-', "_")),
        )
        .into());
    }
    let scratch = std::env::temp_dir().join(format!("goatbuilder-endstate-{}-{}", o.site, std::process::id()));
    let mut assay_rows: Option<PathBuf> = o.assay_rows.map(Path::to_path_buf);
    if assay_rows.is_none() && o.run_assay {
        let rows = scratch.with_extension("assay.jsonl");
        // The assay's own verdict is not this verb's: its rows feed the functional grade.
        if let Err(e) = crate::assay::run(
            repo,
            &crate::assay::Options { site: o.site, plan: false, only: None, vars: &[], json_out: Some(&rows), values: None, stream: false },
        ) {
            // The assay's own words can be a python traceback's worth of path:
            // wrapped, like everything else a person reads.
            eprintln!("{}", crate::tutor::fill(&format!("goatbuilder: the assay did not run, so the functional grade reads unknown: {e}"), "", "  ").trim_end());
        }
        if rows.is_file() {
            assay_rows = Some(rows);
        }
    }
    let grades = scratch.with_extension("json");
    let mut argv = command(repo, o.site);
    argv.extend(["--root".into(), root.display().to_string(), "--json-out".into(), grades.display().to_string()]);
    if let Some(f) = o.facts {
        argv.extend(["--facts".into(), absolute(f).display().to_string()]);
    }
    if let Some(a) = &assay_rows {
        argv.extend(["--assay".into(), a.display().to_string()]);
    }
    if o.strict {
        argv.push("--strict".into());
    }
    let mut cmd = Cmd::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(repo);
    // On a terminal this verb prints its own table from the grades.
    let page = o.stream && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let status = if page {
        cmd.stdout(Stdio::null()).status()
    } else if o.stream {
        cmd.status()
    } else {
        cmd.stdout(Stdio::from(std::io::stderr())).status()
    }
        .map_err(|e| goat_errors::coded("GB-066", format!("cannot run python3: {e}")))?;
    let text = std::fs::read_to_string(&grades).unwrap_or_default();
    let _ = std::fs::remove_file(&grades);
    let _ = std::fs::remove_file(scratch.with_extension("assay.jsonl"));
    if text.is_empty() || !matches!(status.code(), Some(0) | Some(3)) {
        // `Some(2)` is how Rust prints an Option, not how a person reads an
        // exit code -- and a process killed by a signal has none to print.
        let how = match status.code() {
            Some(c) => format!("exit {c}"),
            None => "it was killed".to_string(),
        };
        return Err(goat_errors::coded("GB-066", format!("the end state could not be graded ({how})")).into());
    }
    let g: serde_json::Value = serde_json::from_str(&text)?;
    let mut r = report(o.site, &g, !o.stream);
    if page {
        println!("{}", grades_page(&g, o.strict));
        let n = |k: &str| g["counts"][k].as_u64().unwrap_or(0);
        let orphans = g["orphans"].as_array().map_or(0, |a| a.len());
        println!("\nResult: {} passed, {} failed, {} unknown, {} with nothing to read, {orphans} orphans.", n("PASS"), n("FAIL"), n("UNKNOWN"), n("UNOBSERVED"));
        if r.exit != 0 {
            println!("endstate: not clean{}", if o.strict { ": --strict counts anything not proven" } else { "" });
        }
        r.printed = true; // the rows stay for the tutor's log
    }
    Ok(r)
}

fn absolute(p: &Path) -> PathBuf {
    std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf())
}

const GOOD: &[&str] = &["PASS", "N/A", "UNOBSERVED"];

/// The grades as a person reads them: one row per service, then each distinct
/// reason once. The script's own text stays what a pipe and the gate see.
fn grades_page(g: &serde_json::Value, strict: bool) -> String {
    use crate::tutor::fill;
    let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let cols = ["active", "deployed", "healthy", "functional"];
    // Presence and secrets carry no column, but their reasons still print.
    let graded = ["presence", "active", "deployed", "healthy", "functional", "secrets"];
    let mut rows = Vec::new();
    let mut reasons: std::collections::BTreeMap<(String, String), Vec<String>> = Default::default();
    for r in g["services"].as_array().into_iter().flatten() {
        if text(&r["presence"], "status") == "N/A" {
            continue;
        }
        let service = text(r, "service");
        let mut row = vec![service.clone()];
        for k in graded {
            let st = text(&r[k], "status");
            if st != "PASS" && st != "N/A" && !st.is_empty() {
                reasons.entry((st.clone(), format!("{k}: {}", text(&r[k], "why")))).or_default().push(service.clone());
            }
        }
        for k in cols {
            let st = text(&r[k], "status");
            row.push(if st.is_empty() { "-".into() } else { st });
        }
        let edges = r["edges"].as_array().cloned().unwrap_or_default();
        let good = edges.iter().filter(|e| text(e, "status") == "PASS").count();
        for e in edges.iter().filter(|e| text(e, "status") != "PASS" && text(e, "status") != "N/A") {
            reasons.entry((text(e, "status"), format!("{}: {}", text(e, "edge"), text(e, "why")))).or_default().push(service.clone());
        }
        row.push(format!("{good}/{}", edges.len()));
        rows.push(row);
    }
    let mut out = format!("End state: {} ({})\n\n", text(g, "site"), text(g, "shape"));
    out.push_str("This only reads. Nothing changed, and there is nothing to run.\n\n");
    out.push_str(&crate::output::columns(&["SERVICE", "RUNNING", "DEPLOYED", "HEALTHY", "WORKS", "CONNECTIONS"], &rows, false));
    let say = |why: &str| {
        why.replace("the service writes no deploy ledger", "it keeps no deploy record")
            .replace("no acceptance row exercises it", "no test covers it")
            .replace("no health signal: no docker healthcheck and no isitup row with a health path", "it has no health check")
            .replace("no deploy play is known for it (catalog `play:`), so its secrets cannot be derived", "no deploy playbook is known, so goatbuilder can't tell which secrets it needs")
    };
    let who = |names: &Vec<String>| format!(" ({})", names.join(", "));
    fn label(check: &str) -> &str {
        match check { "presence" => "Present", "active" => "Running", "deployed" => "Deployed", "healthy" => "Healthy", "functional" => "Works", "secrets" => "Secrets", other => other }
    }
    let unobserved: Vec<_> = reasons.iter().filter(|((s, _), _)| s == "UNOBSERVED").collect();
    if !unobserved.is_empty() {
        out.push_str(if strict {
            "\n\nUNOBSERVED means there was nothing to read. --strict counts that as not proven. Here's why:\n"
        } else {
            "\n\nUNOBSERVED means there was nothing to read, so it is not a failure. Here's why:\n"
        });
        for ((_, why), names) in &unobserved {
            let (check, reason) = why.split_once(": ").unwrap_or(("", why));
            out.push_str(fill(&format!("{}: {}{}", label(check), say(reason), who(names)), "  ", "    ").trim_end());
            out.push('\n');
        }
    }
    let failed: Vec<_> = reasons.iter().filter(|((s, _), _)| s != "UNOBSERVED").collect();
    if !failed.is_empty() {
        out.push_str("\n\nProblems:\n");
        for ((status, why), names) in &failed {
            let (check, reason) = why.split_once(": ").unwrap_or(("", why));
            out.push_str(fill(&format!("{status} {}: {}{}", label(check), say(reason), who(names)), "  ", "    ").trim_end());
            out.push('\n');
        }
    }
    for o in g["orphans"].as_array().into_iter().flatten() {
        out.push_str(fill(&format!("orphan {} {}: {}", text(o, "kind"), text(o, "name"), text(o, "why")), "  ", "    ").trim_end());
        out.push('\n');
    }
    if let Some(u) = g["unreachable"].as_array().filter(|u| !u.is_empty()) {
        let names: Vec<String> = u.iter().filter_map(|x| x.as_str().map(String::from)).collect();
        out.push_str(&format!("  unreachable: {}\n", names.join(", ")));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod page_tests {
    #[test]
    fn grades_are_one_row_per_service_and_each_reason_once() {
        let unobserved = serde_json::json!({"status": "UNOBSERVED", "why": "no signal"});
        let pass = serde_json::json!({"status": "PASS", "why": "fine"});
        let g = serde_json::json!({"site": "alpha", "shape": "single-host", "services": [
            {"service": "app", "presence": pass, "active": pass, "deployed": pass, "healthy": unobserved, "functional": pass, "edges": [{"edge": "e", "status": "PASS", "why": ""}]},
            {"service": "db", "presence": pass, "active": pass, "deployed": pass, "healthy": unobserved, "functional": pass, "edges": []},
            {"service": "gone", "presence": {"status": "N/A", "why": ""}},
            {"service": "ghost", "presence": {"status": "FAIL", "why": "declarations disagree"}, "edges": []},
        ], "orphans": [], "unreachable": []});
        let page = super::grades_page(&g, false);
        assert!(page.starts_with("End state: alpha (single-host)"), "{page}");
        let lines: Vec<&str> = page.lines().collect();
        assert!(lines[2].starts_with("This only reads."), "{page}");
        assert_eq!(lines[4].split_whitespace().collect::<Vec<_>>(), ["SERVICE", "RUNNING", "DEPLOYED", "HEALTHY", "WORKS", "CONNECTIONS"]);
        assert_eq!(lines[5].split_whitespace().collect::<Vec<_>>(), ["app", "PASS", "PASS", "UNOBSERVED", "PASS", "1/1"]);
        assert!(!page.contains("gone"), "{page}");
        assert_eq!(page.matches("no signal").count(), 1, "{page}");
        assert!(page.contains("UNOBSERVED means there was nothing to read"), "{page}");
        assert!(page.contains("Healthy: no signal (app, db)"), "{page}");
        assert!(!page.contains("Why each grade"), "{page}");
        assert!(page.contains("FAIL Present: declarations disagree (ghost)"), "a presence failure must print, named: {page}");
        let ghost = page.lines().find(|l| l.starts_with("ghost")).unwrap();
        assert_eq!(ghost.split_whitespace().collect::<Vec<_>>(), ["ghost", "-", "-", "-", "-", "0/0"], "{page}");
    }
}

/// One check per declared-present service (and per orphan) in json/yaml; the
/// verdict alone in table mode, where the script already printed every grade.
pub fn report(site: &str, g: &serde_json::Value, with_rows: bool) -> Report {
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mut checks = Vec::new();
    if with_rows {
        for r in g["services"].as_array().into_iter().flatten() {
            let presence = s(&r["presence"], "status");
            if presence == "N/A" {
                continue;
            }
            let mut bad = Vec::new();
            for k in ["presence", "active", "deployed", "healthy", "functional"] {
                let st = s(&r[k], "status");
                if !st.is_empty() && !GOOD.contains(&st.as_str()) {
                    bad.push(format!("{k} {st}: {}", s(&r[k], "why")));
                }
            }
            for e in r["edges"].as_array().into_iter().flatten() {
                let es = s(e, "status");
                if !GOOD.contains(&es.as_str()) {
                    bad.push(format!("{} {}: {}", s(e, "edge"), s(e, "status"), s(e, "why")));
                }
            }
            let name = s(r, "service");
            checks.push(Check {
                name: format!("service:{name}"),
                ok: bad.is_empty(),
                detail: if bad.is_empty() { "in its declared state".into() } else { bad.join("; ") },
                fix: (!bad.is_empty()).then(|| format!("goatbuilder endstate {site}   (then restore it: goatbuilder deploy --component {name} --env <goatnet>)")),
            });
        }
        for o in g["orphans"].as_array().into_iter().flatten() {
            checks.push(Check { name: format!("orphan:{}:{}", s(o, "kind"), s(o, "name")), ok: false, detail: s(o, "why"), fix: None });
        }
    }
    let c = &g["counts"];
    let n = |k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let orphans = g["orphans"].as_array().map(|a| a.len()).unwrap_or(0);
    let ok = g["exit"].as_i64() == Some(0);
    let detail = format!(
        "{} PASS, {} FAIL, {} UNKNOWN, {} UNOBSERVED, {} orphans ({})",
        n("PASS"),
        n("FAIL"),
        n("UNKNOWN"),
        n("UNOBSERVED"),
        orphans,
        s(g, "shape")
    );
    checks.push(Check {
        name: "endstate".into(),
        ok,
        detail: if ok { detail } else { format!("{detail} (GB-068)") },
        fix: (!ok).then(|| format!("goatbuilder endstate {site}   (each failing grade with its reason)")),
    });
    let mut report = Report::new("endstate", checks, EXIT_NOT_CONVERGED);
    if !ok {
        report.ok = false;
        report.exit = EXIT_NOT_CONVERGED;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/endstate").join(case)
    }

    fn grade(case: &str, site: &str) -> Report {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = fixture(case);
        let o = Options {
            site,
            facts: Some(&root.join("facts.json")),
            assay_rows: Some(&root.join("assay.jsonl")),
            run_assay: false,
            strict: false,
            root: Some(&root),
            stream: false,
        };
        run(&repo, &o).expect("graded")
    }

    #[test]
    fn a_single_machine_site_in_its_end_state_is_exit_0() {
        let r = grade("single-host", "alpha");
        assert!(r.ok, "{:?}", r.checks);
        assert_eq!(r.exit, crate::EXIT_OK);
        assert!(r.checks.iter().any(|c| c.name == "endstate" && c.detail.contains("single-host")));
    }

    #[test]
    fn declared_absent_services_are_not_graded_or_orphaned() {
        let r = grade("no-grafana-no-sandbox", "beta");
        assert!(r.ok, "{:?}", r.checks);
        assert!(!r.checks.iter().any(|c| c.name == "service:grafana" || c.name == "service:sandbox"));
    }

    #[test]
    fn orphans_broken_edges_and_contradictions_are_exit_3() {
        let r = grade("orphans-and-breaks", "gamma");
        assert_eq!(r.exit, EXIT_NOT_CONVERGED);
        let names: Vec<&str> = r.checks.iter().filter(|c| !c.ok).map(|c| c.name.as_str()).collect();
        for want in ["service:app", "service:core", "service:prometheus", "orphan:container:leftover", "orphan:oidc-client:stale-client", "orphan:spiffe-entry:/ghost"] {
            assert!(names.contains(&want), "{want} not in {names:?}");
        }
        let core = r.checks.iter().find(|c| c.name == "service:core").unwrap();
        assert!(core.detail.contains("not registered in Hydra"), "{}", core.detail);
    }

    #[test]
    fn a_phone_site_that_declares_no_audit_spool_is_in_its_end_state() {
        let r = grade("audit-spool-declared-absent", "delta");
        assert!(r.ok, "{:?}", r.checks);
        assert!(!r.checks.iter().any(|c| c.name == "service:audit-spool"));
    }

    #[test]
    fn an_audit_spool_declared_on_with_no_container_fails_active() {
        let r = grade("audit-spool-on-not-running", "epsilon");
        assert_eq!(r.exit, EXIT_NOT_CONVERGED);
        let spool = r.checks.iter().find(|c| c.name == "service:audit-spool").expect("graded");
        assert!(!spool.ok, "{}", spool.detail);
        assert!(spool.detail.contains("no container audit-subscriber"), "{}", spool.detail);
    }

    #[test]
    fn an_unknown_site_is_refused_before_anything_runs() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let o = Options { site: "no-such-site", facts: None, assay_rows: None, run_assay: false, strict: false, root: None, stream: false };
        let e = run(&repo, &o).unwrap_err().to_string();
        assert!(e.contains("GB-066") || e.contains("no goatnet site named"), "{e}");
    }
}

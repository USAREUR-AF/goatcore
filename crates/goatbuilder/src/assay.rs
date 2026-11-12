//! `goatbuilder assay <site>` — prove a stood-up goatnet was built correctly,
//! layer by layer: the acceptance manifest (`ops/validation/acceptance-tests.yaml`,
//! ADR 1115) run against the site's own values, with the goat CLI legs that read
//! the goatnet the way its operators do. It is the standup's acceptance phase as
//! a verb of its own, so a goatnet can be re-proven at any time after standup.
//!
//! The same two commands run here, in `goat-standup.sh` phase 13 and in this
//! binary's `standup` phase 13 (ADR 1201: both avenues, one assay). Reads only,
//! except the rows that round-trip a sample through the fabric, which store
//! nothing. Exit 0 when every applicable row PASSes; 3 when any row FAILs or
//! cannot answer (UNKNOWN), or none passed (GB-048); 1 when the assay cannot run
//! at all (GB-049).

use crate::runner::Cmd;
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_OK};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub struct Options<'a> {
    pub site: &'a str,
    pub plan: bool,
    pub only: Option<&'a str>,
    pub vars: &'a [String],
    pub json_out: Option<&'a Path>,
    /// A value set to use instead of deriving one from the site's group_vars
    /// (a fixture's, for a rehearsal).
    pub values: Option<&'a Path>,
    /// Table output streams the runner's own lines; json/yaml carry the rows.
    pub stream: bool,
}

/// The acceptance context, one definition for both entry points (LP-015): the
/// site's declared end-state model (ADR 1206), its values, and the runner over
/// every eligible row with both. Standup phase 13 runs [`Acceptance::phase_line`];
/// `assay` runs the same three itself, into private temp files, under the
/// goatnet's secrets store ([`under_store`]), so a credential row such as
/// `mgmt-api-answers` reads the same token either way.
pub struct Acceptance {
    pub model: Vec<String>,
    pub gen: Vec<String>,
    pub run: Vec<String>,
    endstate: String,
}

pub fn acceptance(repo: &Path, site: &str, values: &Path, endstate: &Path) -> Acceptance {
    let v = repo.join("ops/validation");
    let model = vec!["python3".into(), repo.join("ops/goatbuilder/endstate/endstate.py").display().to_string(), "model".into(), site.into()];
    let gen = vec!["python3".into(), v.join("generate-values.py").display().to_string(), site.into()];
    let run = vec![
        "python3".into(),
        v.join("run-acceptance.py").display().to_string(),
        "--env".into(),
        site.into(),
        "--values".into(),
        values.display().to_string(),
        "--endstate".into(),
        endstate.display().to_string(),
        "--all-eligible".into(),
        "--by-layer".into(),
    ];
    Acceptance { model, gen, run, endstate: endstate.display().to_string() }
}

impl Acceptance {
    /// The phase-13 line both standup drivers print and run (the parity gate diffs it
    /// against goat-standup.sh): the model into its file, the values, then the runner.
    pub fn phase_line(&self) -> String {
        let q = crate::standup::sh_quote;
        let sh = |a: &[String]| a.iter().map(|x| if x.contains('/') { q(x) } else { x.clone() }).collect::<Vec<_>>().join(" ");
        format!("{} > {} && {} && {}", sh(&self.model), q(&self.endstate), sh(&self.gen), sh(&self.run))
    }
}

/// Phase 13's own paths: the values file the generator writes in the checkout and the
/// model in the run's state directory.
pub fn phase13(repo: &Path, site: &str, state_dir: &str) -> Acceptance {
    acceptance(repo, site, &repo.join(format!("ops/validation/values-{site}.yaml")), &Path::new(state_dir).join(format!("endstate-{site}.json")))
}

/// A command run under the goatnet's declared secrets store: through
/// ops/scripts/with-secrets, the same resolution `goatbuilder secrets exec` and every
/// standup phase use. A value store whose file cannot be read refuses here, by name,
/// before anything is probed, so a missing store is never graded as a row.
pub fn under_store(repo: &Path, argv: &[String]) -> Result<Cmd> {
    let posture = repo.join("ops/secrets/posture.py");
    let kind = Cmd::new("python3").arg(&posture).arg("kind").output();
    if matches!(&kind, Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "value") {
        let probe = Cmd::new("python3").arg(&posture).args(["exec", "--", "true"]).current_dir(repo).output();
        if let Ok(o) = probe {
            if !o.status.success() {
                let why = String::from_utf8_lossy(&o.stderr).trim().to_string();
                return Err(goat_errors::coded("GB-049", format!("the goatnet's secrets store could not be read, so no credential row could answer: {why}")).into());
            }
        }
    }
    let mut c = Cmd::new(repo.join("ops/scripts/with-secrets"));
    c.args(argv).current_dir(repo);
    Ok(c)
}

pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    if let Some(values) = o.values {
        if !values.is_file() {
            return Err(goat_errors::coded("GB-049", format!("no value set at {}", values.display())).into());
        }
        // A rehearsal: the fixture's value set stands in for the goatnet, which has no
        // end state or store of its own to read.
        let values = std::env::current_dir().map(|d| d.join(values)).unwrap_or_else(|_| values.to_path_buf());
        let mut run = acceptance(repo, o.site, &values, Path::new("")).run;
        let i = run.iter().position(|a| a == "--endstate").expect("acceptance() passes --endstate");
        run.drain(i..i + 2);
        return finish(repo, o, run, false);
    }
    // Derived into private temp files: assay reads only, and must not write (or
    // overwrite a tracked) ops/validation/values-<site>.yaml in the checkout.
    let probe = acceptance(repo, o.site, Path::new(""), Path::new(""));
    let derived = derive_values(repo, o.site, &probe.gen)?;
    let model = derive(&probe.model, repo, &format!("endstate-{}", o.site), "json", "the site's declared end state");
    let model = match model {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&derived);
            return Err(e);
        }
    };
    let ctx = acceptance(repo, o.site, &derived, &model);
    let result = finish(repo, o, ctx.run, true);
    let _ = std::fs::remove_file(&derived);
    let _ = std::fs::remove_file(&model);
    result
}

/// Run one derivation command and keep its stdout in an owner-only temp file.
fn derive(argv: &[String], repo: &Path, stem: &str, ext: &str, what: &str) -> Result<PathBuf> {
    let g = Cmd::new(crate::endstate::python()).args(&argv[1..]).current_dir(repo).output();
    match g {
        Ok(out) if out.status.success() => {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            // A fresh name per call: two assays in one process (the tests, `endstate
            // --assay`) never share a file.
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("goatbuilder-assay-{stem}-{}-{n}.{ext}", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).with_context(|| format!("the temporary file for {what}"))?;
            f.write_all(&out.stdout).with_context(|| format!("the temporary file for {what}"))?;
            Ok(path)
        }
        Ok(out) => Err(goat_errors::coded("GB-049", format!("{what} could not be derived: {}", String::from_utf8_lossy(&out.stderr).trim())).into()),
        Err(e) => Err(goat_errors::coded("GB-049", format!("cannot run python3: {e}")).into()),
    }
}

fn derive_values(repo: &Path, site: &str, gen: &[String]) -> Result<PathBuf> {
    if !repo.join(format!("ops/ansible/group_vars/{}_hosts/vars.yml", site.replace('-', "_"))).is_file() {
        return Err(goat_errors::coded(
            "GB-049",
            format!("no goatnet named `{site}` here: ops/ansible/group_vars/{}_hosts/vars.yml does not exist", site.replace('-', "_")),
        )
        .into());
    }
    // The commands print as the standup spells them; they run under the Python that
    // has PyYAML (python3 when it does, else Ansible's own, #2120). The values name
    // the site's addresses: owner-only, and a fresh name.
    let mut argv = gen.to_vec();
    argv.push("--stdout".into());
    derive(&argv, repo, &format!("values-{site}"), "yaml", "the site's values")
}

fn finish(repo: &Path, o: &Options, mut run: Vec<String>, store: bool) -> Result<Report> {
    if o.plan {
        run.retain(|a| a != "--all-eligible" && a != "--by-layer");
        run.push("--plan".into());
    }
    if let Some(ids) = o.only {
        run.push("--only".into());
        run.push(ids.into());
    }
    for kv in o.vars {
        run.push("--var".into());
        run.push(kv.clone());
    }
    let scratch = std::env::temp_dir().join(format!("goatbuilder-assay-{}-{}.jsonl", o.site, std::process::id()));
    let jsonl: PathBuf = o.json_out.map(Path::to_path_buf).unwrap_or_else(|| scratch.clone());
    if !o.plan {
        run.push("--json".into());
        run.push(jsonl.display().to_string());
    }

    run[0] = crate::endstate::python();
    let mut cmd = if store && !o.plan {
        under_store(repo, &run)?
    } else {
        let mut c = Cmd::new(&run[0]);
        c.args(&run[1..]).current_dir(repo);
        c
    };
    // The runner's lines are the human report; in json/yaml mode they go to
    // stderr so stdout carries one document.
    //
    // A plan is read from the runner's own text, whichever surface asked for
    // it: a table on a terminal, the runner's lines through a pipe, and the
    // same plan as a document under `-o json` or `-o yaml`, which used to get
    // nothing at all (output standard, rule 1).
    if o.plan {
        let out = cmd.stderr(Stdio::inherit()).output().map_err(|e| goat_errors::coded("GB-049", format!("cannot run the acceptance runner: {e}")))?;
        let code = out.status.code().unwrap_or(1);
        if code != 0 {
            return Err(goat_errors::coded("GB-049", format!("the acceptance runner refused the plan (exit {code})")).into());
        }
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if !o.stream {
            return Ok(plan_report(&text, o.site).unwrap_or_else(|| Report::new("assay", vec![], EXIT_OK)));
        }
        if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            println!("{}", plan_page(&text, o.site).unwrap_or_else(|| text.trim_end().to_string()));
        } else {
            print!("{text}");
        }
        let mut r = Report::new("assay", vec![], EXIT_OK);
        r.printed = true;
        return Ok(r);
    }
    let status = if o.stream {
        cmd.status()
    } else {
        cmd.stdout(Stdio::from(std::io::stderr())).status()
    }
    .map_err(|e| goat_errors::coded("GB-049", format!("cannot run the acceptance runner: {e}")))?;


    // 0: no FAIL; 1: a FAIL; 2: nothing passed. Anything else is the runner
    // itself failing, not a verdict.
    match status.code() {
        Some(0..=2) => {}
        other => {
            return Err(goat_errors::coded("GB-049", format!("the acceptance runner did not finish (exit {other:?})")).into())
        }
    }
    let text = std::fs::read_to_string(&jsonl).unwrap_or_default();
    let _ = std::fs::remove_file(&scratch);
    let rows = parse(&text);
    let report = grade(o.site, &rows, !o.stream);
    Ok(report)
}

/// One verdict line of the runner's JSONL.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub status: String,
    pub evidence: String,
    pub layer: String,
}

pub fn parse(jsonl: &str) -> Vec<Row> {
    jsonl
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            Row { id: s("id"), status: s("status"), evidence: s("evidence"), layer: if s("layer").is_empty() { "other".into() } else { s("layer") } }
        })
        .collect()
}

/// The runner's plan as a table: one row per test, with what it expects in words.
/// `None` when a line isn't one this page understands: the raw plan then prints.
/// The runner's plan text, parsed: one row per check (area, name, what it
/// expects, how it is made), the count it printed, and the checks it says do
/// not apply here. `None` when the text is not a plan.
fn plan_parse(text: &str) -> Option<(Vec<Vec<String>>, String, String)> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut area, mut other) = (String::new(), String::new());
    let mut count = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("assay plan:") {
            count = rest.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        } else if let Some(rest) = t.strip_prefix("N/A here: ") {
            other = rest.to_string();
        } else if let Some(rest) = t.strip_prefix("prove ").map(|r| ("works", r)).or_else(|| t.strip_prefix("refuse ").map(|r| ("refuses", r))) {
            let mut w = rest.1.split_whitespace();
            rows.push(vec![area.clone(), w.next()?.to_string(), rest.0.to_string(), w.next().unwrap_or("").to_string()]);
        } else if t.ends_with(')') && t.contains(" (") {
            area = t.split(" (").next()?.to_string();
        } else {
            return None;
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some((rows, count, other))
}

/// The plan as a report: one check per planned check, so a program reads the
/// same plan the page shows. Nothing has run, so every row is ok and the
/// detail says what it would prove and how.
pub(crate) fn plan_report(text: &str, site: &str) -> Option<Report> {
    let (rows, count, other) = plan_parse(text)?;
    let applies = count.split(" applicable").next().unwrap_or("").trim().to_string();
    let mut checks: Vec<Check> = vec![Check {
        name: "plan".into(),
        ok: true,
        detail: format!("{applies} checks apply to {site}. Nothing has run, and no machine was touched."),
        fix: None,
    }];
    for r in &rows {
        checks.push(Check {
            name: format!("{}/{}", r[0], r[1]),
            ok: true,
            detail: format!("expects: {}; method: {}", r[2], r[3]),
            fix: None,
        });
    }
    if !other.is_empty() {
        checks.push(Check { name: "not-here".into(), ok: true, detail: format!("not for this goatnet: {other}"), fix: None });
    }
    Some(Report::new("assay", checks, EXIT_OK))
}

pub(crate) fn plan_page(text: &str, site: &str) -> Option<String> {
    let (rows, count, other) = plan_parse(text)?;
    let applies = count.split(" applicable").next().unwrap_or("").to_string();
    // Every other verb a person reads calls these checks, so this one does too.
    // How each check is made (container-health, dns-resolve) is in `-o json`:
    // it is the one column that does not fit beside the others at 80.
    let mut out = format!("Assay plan: {site}\n{applies} checks apply. Nothing has run, and no machine was touched.\n\n");
    let three: Vec<Vec<String>> = rows.iter().map(|r| r[..3].to_vec()).collect();
    out.push_str(&crate::output::columns(&["AREA", "CHECK", "EXPECT"], &three, std::io::IsTerminal::is_terminal(&std::io::stdout())));
    if !other.is_empty() {
        out.push_str("\n\n");
        out.push_str(crate::tutor::fill(&format!("Not for this goatnet: {other}"), "", "  ").trim_end());
    }
    // A plan that does not say how to run it is a page a reader stops on.
    out.push_str("\n\n");
    out.push_str(crate::tutor::fill(&format!("To run them, which reads every machine: goatbuilder assay {site}"), "", "  ").trim_end());
    out.push('\n');
    out.push_str(crate::tutor::fill("To run one: goatbuilder assay <site> --only <the check's name above>", "", "  ").trim_end());
    out.push('\n');
    out.push_str(crate::tutor::fill("How each check is made: goatbuilder -o json assay <site> --plan", "", "  ").trim_end());
    Some(out)
}

/// The verdict. Only PASS counts toward done (ADR 1115): a FAIL is the goatnet
/// failing, an UNKNOWN is a row nobody could answer, and both keep the exit at
/// 3. N/A and SKIPPED rows did not apply. `with_rows` carries every row as a
/// check (json/yaml); otherwise only the per-layer totals, since the runner
/// already printed the rows.
pub fn grade(site: &str, rows: &[Row], with_rows: bool) -> Report {
    let mut by_layer: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
    for r in rows {
        let e = by_layer.entry(r.layer.as_str()).or_default();
        match r.status.as_str() {
            "PASS" => e[0] += 1,
            "FAIL" => e[1] += 1,
            "UNKNOWN" => e[2] += 1,
            _ => {}
        }
    }
    let pass: usize = by_layer.values().map(|c| c[0]).sum();
    let fail: usize = by_layer.values().map(|c| c[1]).sum();
    let unknown: usize = by_layer.values().map(|c| c[2]).sum();
    let mut checks = Vec::new();
    if with_rows {
        for r in rows {
            let ok = matches!(r.status.as_str(), "PASS" | "N/A" | "SKIPPED");
            checks.push(Check {
                name: format!("{}:{}", r.layer, r.id),
                ok,
                detail: format!("{} {}", r.status, r.evidence),
                fix: (!ok).then(|| format!("goatbuilder assay {site} --only {}", r.id)),
            });
        }
    }
    let verdict_ok = fail == 0 && unknown == 0 && pass > 0;
    let detail = format!("{pass} PASS, {fail} FAIL, {unknown} UNKNOWN across {} layers", by_layer.len());
    checks.push(Check {
        name: "assay".into(),
        ok: verdict_ok,
        detail: if verdict_ok { detail } else { format!("{detail} (GB-048)") },
        fix: (!verdict_ok).then(|| "fix each FAIL; give each UNKNOWN a probe that can answer it (a goat context with a fabric identity: assay_probe_enabled and ops/scripts/create-assay-probe-identity.sh, or --var probe_context=<ctx> --var probe_via=<user@host>), then re-run".to_string()),
    });
    let mut report = Report::new("assay", checks, EXIT_NOT_CONVERGED);
    if !verdict_ok {
        report.ok = false;
        report.exit = EXIT_NOT_CONVERGED;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn row(id: &str, status: &str, layer: &str) -> String {
        serde_json::json!({"id": id, "status": status, "evidence": "e", "layer": layer}).to_string()
    }

    #[test]
    fn every_pass_is_exit_0() {
        let rows = parse(&[row("a", "PASS", "host"), row("b", "PASS", "fabric"), row("c", "N/A", "federation")].join("\n"));
        let r = grade("zagros", &rows, false);
        assert_eq!(r.exit, 0);
        assert!(r.ok);
        assert!(r.checks.last().unwrap().detail.starts_with("2 PASS, 0 FAIL, 0 UNKNOWN across 3 layers"));
    }

    #[test]
    fn a_fail_is_exit_3() {
        let rows = parse(&[row("a", "PASS", "host"), row("b", "FAIL", "fabric")].join("\n"));
        let r = grade("zagros", &rows, true);
        assert_eq!(r.exit, 3);
        let b = r.checks.iter().find(|c| c.name == "fabric:b").unwrap();
        assert!(!b.ok);
        assert_eq!(b.fix.as_deref(), Some("goatbuilder assay zagros --only b"));
    }

    #[test]
    fn an_unknown_is_not_green() {
        // A row nobody could answer is not a row that passed.
        let rows = parse(&[row("a", "PASS", "host"), row("b", "UNKNOWN", "fabric")].join("\n"));
        let r = grade("zagros", &rows, false);
        assert_eq!(r.exit, 3);
        assert!(r.checks.last().unwrap().detail.contains("GB-048"));
    }

    #[test]
    fn nothing_passed_is_not_green() {
        let rows = parse(&[row("a", "N/A", "host"), row("b", "SKIPPED", "fabric")].join("\n"));
        assert_eq!(grade("zagros", &rows, false).exit, 3);
        assert_eq!(grade("zagros", &[], false).exit, 3);
    }

    #[test]
    fn phase_13_is_one_acceptance_context_with_the_end_state() {
        // The line both standup drivers print; the parity gate diffs it against goat-standup.sh.
        assert_eq!(
            phase13(Path::new("/r"), "pasture", "/s").phase_line(),
            "python3 '/r/ops/goatbuilder/endstate/endstate.py' model pasture > '/s/endstate-pasture.json' && python3 '/r/ops/validation/generate-values.py' pasture && python3 '/r/ops/validation/run-acceptance.py' --env pasture --values '/r/ops/validation/values-pasture.yaml' --endstate '/s/endstate-pasture.json' --all-eligible --by-layer"
        );
        // The standalone assay runs the same runner arguments, with its own two files.
        let standalone = acceptance(Path::new("/r"), "pasture", Path::new("/t/values.yaml"), Path::new("/t/model.json")).run;
        let p13 = phase13(Path::new("/r"), "pasture", "/s").run;
        let shape = |a: &[String]| a.iter().map(|x| if x.starts_with("/t/") || x.starts_with("/s/") || x.ends_with("values-pasture.yaml") { "<file>".to_string() } else { x.clone() }).collect::<Vec<_>>();
        assert_eq!(shape(&standalone), shape(&p13));
        assert!(standalone.windows(2).any(|w| w == ["--endstate", "/t/model.json"]), "{standalone:?}");
    }

    /// A one-box goatnet in a temp repository: stub generators and a stub runner that
    /// grade two rows the way the real runner does. `prom-up` needs a service the
    /// end state declares absent (N/A with --endstate, FAIL without); `mgmt-api-answers`
    /// reads <SITE>_NB_API_TOKEN from its environment (PASS when set, UNKNOWN when not).
    /// The real with-secrets, posture.py and standup-phase-run resolve the store.
    fn lite_repo(token: Option<&str>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for rel in ["ops/scripts/with-secrets", "ops/scripts/standup-phase-run", "ops/secrets/posture.py"] {
            std::fs::create_dir_all(r.join(rel).parent().unwrap()).unwrap();
            std::fs::copy(src.join(rel), r.join(rel)).unwrap();
        }
        let write = |rel: &str, body: &str| {
            std::fs::create_dir_all(r.join(rel).parent().unwrap()).unwrap();
            std::fs::write(r.join(rel), body).unwrap();
        };
        write("ops/ansible/group_vars/cleanroom_hosts/vars.yml", "site_id: cleanroom\n");
        write("ops/goatbuilder/goatbuilder.yaml", "secrets:\n  store: env-file\n");
        write("ops/goatbuilder/endstate/endstate.py", "import json\nprint(json.dumps({'services': {'prometheus': {'present': False, 'assay': ['prometheus']}}}))\n");
        write("ops/validation/generate-values.py", "import sys, os\nbody = 'nb_api_token_env: CLEANROOM_NB_API_TOKEN\\n'\nif '--stdout' in sys.argv:\n    print(body, end='')\nelse:\n    open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'values-cleanroom.yaml'), 'w').write(body)\n");
        write("ops/validation/run-acceptance.py", r#"import json, os, sys
a = sys.argv[1:]
arg = lambda k: a[a.index(k) + 1] if k in a else None
here = os.path.dirname(os.path.abspath(__file__))
token = 'cleanroom'.upper() + '_NB_API_TOKEN'
absent = set()
if arg('--endstate'):
    for s in json.load(open(arg('--endstate')))['services'].values():
        if s.get('present') is False: absent.update(s.get('assay') or [])
rows = [
    {'id': 'prom-up', 'layer': 'obs', 'status': 'N/A' if 'prometheus' in absent else 'FAIL', 'evidence': ''},
    {'id': 'mgmt-api-answers', 'layer': 'mgmt-stack', 'status': 'PASS' if os.environ.get(token) else 'UNKNOWN', 'evidence': ''},
]
out = arg('--json') or os.path.join(here, 'phase13-rows.jsonl')
open(out, 'w').write(''.join(json.dumps(r) + '\n' for r in rows))
open(os.path.join(here, 'argv.log'), 'a').write(json.dumps(a) + '\n')
sys.exit(1 if any(r['status'] == 'FAIL' for r in rows) else 0)
"#);
        if let Some(t) = token {
            write(".envrc.local.d/secrets.env", &format!("CLEANROOM_NB_API_TOKEN={t}\n"));
        } else {
            write(".envrc.local.d/secrets.env", "OTHER=x\n");
        }
        d
    }

    fn statuses(jsonl: &str) -> BTreeMap<String, String> {
        parse(jsonl).into_iter().map(|r| (r.id, r.status)).collect()
    }

    fn standalone(r: &Path, rows: &Path) -> Result<Report> {
        run(r, &Options { site: "cleanroom", plan: false, only: None, vars: &[], json_out: Some(rows), values: None, stream: false })
    }

    /// LP-015: the standalone assay and phase 13 grade the same rows the same way: the
    /// end state makes the absent service's row N/A, and the stored token reaches the
    /// credential row without a `secrets exec` wrapper.
    #[test]
    fn standalone_assay_and_phase_13_grade_alike_under_the_store() {
        if std::env::var_os("GOAT_SECRET_STORE").is_some() || std::env::var_os("GOAT_SECRETS_RESOLVED").is_some() {
            return; // a store chosen in this shell would override the fixture's
        }
        let d = lite_repo(Some("fixture-token-value"));
        let r = d.path();
        let rows = r.join("standalone.jsonl");
        let rep = standalone(r, &rows).unwrap();
        let alone = statuses(&std::fs::read_to_string(&rows).unwrap());
        assert_eq!(alone.get("prom-up").map(String::as_str), Some("N/A"), "{alone:?}");
        assert_eq!(alone.get("mgmt-api-answers").map(String::as_str), Some("PASS"), "{alone:?}");
        assert_eq!(rep.exit, EXIT_OK, "{:?}", rep.checks);

        let state = tempfile::tempdir().unwrap();
        let line = phase13(r, "cleanroom", &state.path().display().to_string()).phase_line();
        let st = Command::new(r.join("ops/scripts/standup-phase-run")).arg(&line).current_dir(r).env_remove("GOAT_SECRET_ENV_FILE").status().unwrap();
        assert!(st.success());
        assert_eq!(statuses(&std::fs::read_to_string(r.join("ops/validation/phase13-rows.jsonl")).unwrap()), alone, "the same rows, the same verdicts");

        let argv: Vec<Vec<String>> = std::fs::read_to_string(r.join("ops/validation/argv.log")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let flags = |a: &[String]| a.iter().filter(|x| x.starts_with("--") && *x != "--json").cloned().collect::<Vec<_>>();
        assert_eq!(flags(&argv[0]), flags(&argv[1]), "{argv:?}");
    }

    /// The negative control: with the token absent from the store the row stays UNKNOWN
    /// and the assay exits non-zero; a store that cannot be read refuses before probing.
    #[test]
    fn without_the_token_the_credential_row_stays_unknown() {
        if std::env::var_os("GOAT_SECRET_STORE").is_some() || std::env::var_os("GOAT_SECRETS_RESOLVED").is_some() {
            return;
        }
        let d = lite_repo(None);
        let r = d.path();
        let rows = r.join("standalone.jsonl");
        let rep = standalone(r, &rows).unwrap();
        assert_eq!(statuses(&std::fs::read_to_string(&rows).unwrap()).get("mgmt-api-answers").map(String::as_str), Some("UNKNOWN"));
        assert_eq!(rep.exit, EXIT_NOT_CONVERGED);

        std::fs::remove_file(r.join(".envrc.local.d/secrets.env")).unwrap();
        let e = standalone(r, &rows).unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().unwrap().code, "GB-049");
        assert!(!r.join("ops/validation/argv.log").exists() || std::fs::read_to_string(r.join("ops/validation/argv.log")).unwrap().lines().count() == 1, "nothing was probed");
    }

    #[test]
    fn an_unknown_site_is_refused_before_anything_runs() {
        let dir = tempfile::tempdir().unwrap();
        let e = run(dir.path(), &Options { site: "nowhere", plan: false, only: None, vars: &[], json_out: None, values: None, stream: true }).unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().unwrap().code, "GB-049");
    }
}

#[cfg(test)]
mod plan_page_tests {
    #[test]
    fn a_plan_is_a_table_of_checks() {
        let raw = "assay plan: env=zagros_dev  2 applicable rows, 1 N/A\n\n  audit (1)\n    prove  audit-runs   container-health\n\n  mgmt-stack (1)\n    refuse mgmt-api-refuses http-ok\n\n  N/A here: other-row\n";
        let page = super::plan_page(raw, "zagros_dev").unwrap();
        assert!(page.starts_with("Assay plan: zagros_dev\n2 checks apply."), "{page}");
        assert!(page.contains("To run them, which reads every machine: goatbuilder assay zagros_dev"), "{page}");
        assert!(page.lines().all(|l| l.chars().count() <= 80), "{page}");
        let cols = |m: &str| page.lines().find(|l| l.contains(m)).unwrap().split_whitespace().map(String::from).collect::<Vec<_>>();
        // How a check is made is in -o json; the page has the three columns a
        // reader asks for, and they fit beside each other at 80.
        assert_eq!(cols("AREA"), ["AREA", "CHECK", "EXPECT"]);
        assert_eq!(cols("audit-runs"), ["audit", "audit-runs", "works"]);
        assert_eq!(cols("mgmt-api-refuses"), ["mgmt-stack", "mgmt-api-refuses", "refuses"]);
        assert!(!page.contains("container-health"), "the method is not on the page: {page}");
        assert!(page.contains("Not for this goatnet: other-row"), "{page}");
        assert!(super::plan_page("something else", "s").is_none());
    }
}

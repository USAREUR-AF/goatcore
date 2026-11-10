//! `goatbuilder federate status`, through the real binary against the
//! stand-in repository (federate_world): read-only, a table or
//! `goatbuilder.federate.status/v1`. Every test runs on both verb paths:
//! `goat federate` in a shell (`FED_VERBS=goat`) and goatbuilder's in-process
//! verbs (the default).

mod federate_world;

use std::path::Path;

use federate_world::{says, world_on, Verbs};

/// `name` as two tests, `name::goat` and `name::in_process`.
macro_rules! on_both_verb_paths {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            #[test]
            fn goat() {
                super::$name(super::Verbs::Goat)
            }
            #[test]
            fn in_process() {
                super::$name(super::Verbs::InProcess)
            }
        }
    )*};
}

on_both_verb_paths!(
    status_reads_the_journal_and_writes_nothing,
    status_reads_acceptance_evidence_current_stale_and_damaged,
    status_of_an_undeclared_peering_is_refused_and_a_run_keeps_its_own_refusal,
    work_without_a_partner_is_refused,
    a_partner_after_a_peering_name_is_refused,
);

/// Every file under `d` with its size and modification time.
fn listing(d: &Path) -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                walk(&p, out);
            }
            out.push(format!(
                "{} {} {:?}",
                p.display(),
                m.len(),
                m.modified().ok()
            ));
        }
    }
    let mut v = Vec::new();
    walk(d, &mut v);
    v.sort();
    v
}

fn sha256_file(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(std::fs::read(p).unwrap()))
}

fn evidence(work: &Path, agreement_sha: &str, verdict: &str) {
    let d = work.join("evidence/acceptance");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("result.json"),
        format!(
            r#"{{"schema":"goat-federation-acceptance/v1","run":{{"id":"r","started_at":"2026-11-09T10:00:00Z","ended_at":"2026-11-09T11:00:00Z"}},"agreement":{{"sha256":"{agreement_sha}"}},"counts":{{"PASS":3,"FAIL":0,"BLOCKED":0,"SKIP":0,"NOT APPLICABLE":0}},"verdict":"{verdict}"}}"#
        ),
    )
    .unwrap();
}

fn status_reads_the_journal_and_writes_nothing(verbs: Verbs) {
    let w = world_on(verbs);
    // Nothing run yet.
    let before = listing(&w.work);
    let (out, v) = w.run(&["status", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(v["schema"], "goatbuilder.federate.status/v1");
    assert_eq!(v["peerings"][0]["peering"], "hello-ally");
    assert_eq!(v["peerings"][0]["journal"], "none");
    assert_eq!(before, listing(&w.work), "read-only");

    // After a run that stops at phase 5.
    let (out, _) = w.run(&["hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    let before = listing(&w.work);
    let (out, v) = w.run(&["status", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    let p = &v["peerings"][0];
    assert_eq!(p["journal"], "ok");
    assert_eq!(p["last_run"]["outcome"], "stopped");
    assert_eq!(p["phases"].as_array().unwrap().last().unwrap()["phase"], 5);
    assert!(p["next"].as_str().unwrap().contains("--from 6"), "{p}");
    assert!(
        p["next"]
            .as_str()
            .unwrap()
            .contains(&format!("--work {}", w.work.display())),
        "{p}"
    );
    assert_eq!(before, listing(&w.work), "read-only");

    // The table and YAML, for a person and a program.
    let out = w
        .command_with(&["-o", "table"], &["status", "hello-ally"])
        .output()
        .unwrap();
    let page = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        page.contains("PEERING") && page.contains("hello-ally") && page.contains("stopped"),
        "{page}"
    );
    let out = w
        .command_with(&["-o", "yaml"], &["status", "hello-ally"])
        .output()
        .unwrap();
    let y = String::from_utf8_lossy(&out.stdout);
    assert!(y.contains("schema: goatbuilder.federate.status/v1"), "{y}");
}

fn status_reads_acceptance_evidence_current_stale_and_damaged(verbs: Verbs) {
    let w = world_on(verbs);
    w.write_keys();
    w.run(&["hello-ally"]);
    w.run(&["hello-ally", "--from", "6"]);
    w.run(&["hello-ally", "--from", "11"]);
    let ag = sha256_file(&w.work.join("agreement.json"));

    evidence(&w.work, &ag, "PASS");
    let (_, v) = w.run(&["status", "hello-ally"]);
    let a = &v["peerings"][0]["acceptance"];
    assert_eq!(
        (a["verdict"].as_str(), a["agreement"].as_str()),
        (Some("PASS"), Some("current")),
        "{v}"
    );
    assert!(
        v["peerings"][0]["next"]
            .as_str()
            .unwrap()
            .starts_with("nothing"),
        "{v}"
    );

    evidence(&w.work, "sha256:00", "PASS");
    let (_, v) = w.run(&["status", "hello-ally"]);
    assert_eq!(v["peerings"][0]["acceptance"]["agreement"], "stale");

    std::fs::write(w.work.join("evidence/acceptance/result.json"), "{cut short").unwrap();
    let (_, v) = w.run(&["status", "hello-ally"]);
    assert!(
        v["peerings"][0]["acceptance"]["verdict"]
            .as_str()
            .unwrap()
            .starts_with("unreadable"),
        "{v}"
    );
}

fn status_of_an_undeclared_peering_is_refused_and_a_run_keeps_its_own_refusal(verbs: Verbs) {
    let w = world_on(verbs);
    let (out, _) = w.run(&["status", "xzz"]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("GB-185") && text.contains("xzz"), "{text}");
    // A run of an undeclared peering is not GB-185.
    let (out, _) = w.run(&["xzz", "--dry-run"]);
    assert!(!out.status.success());
    assert!(!says(&out).contains("GB-185"), "{}", says(&out));
}

fn work_without_a_partner_is_refused(verbs: Verbs) {
    let w = world_on(verbs);
    let (out, _) = w.run(&["status", "--work", "/tmp/x"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(
        says(&out).contains("--work names one peering's folder"),
        "{}",
        says(&out)
    );
}

fn a_partner_after_a_peering_name_is_refused(verbs: Verbs) {
    let w = world_on(verbs);
    let (out, _) = w.run(&["hello-ally", "extra", "--dry-run"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(
        says(&out).contains("unexpected argument 'extra'"),
        "{}",
        says(&out)
    );
}

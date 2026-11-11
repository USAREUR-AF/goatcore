//! `goatbuilder federate deploy <partner>` and `test <partner>`, through the
//! real binary against the stand-in repository (federate_world).

mod federate_world;

use federate_world::{phases_run, rows, says, world, World};

/// Phases 1 to 9 done, and the plan written by `goatbuilder federate plan`.
/// Returns the plan's hash.
fn ready(w: &World) -> String {
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--to", "9"]);
    assert!(out.status.success(), "{}", says(&out));
    let (out, _) = w.run(&["plan", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    plan_hash(w)
}

fn plan_hash(w: &World) -> String {
    let plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(w.work.join("plan.json")).unwrap()).unwrap();
    assert_eq!(plan["verdict"], "READY", "{plan}");
    plan["hash"].as_str().unwrap().to_string()
}

/// No play ran (the live recheck may have read the hosts).
fn no_play(w: &World) {
    assert!(!w.log().contains("play "), "{}", w.log());
}

#[test]
fn deploy_needs_the_plan_approved_by_its_hash_and_never_by_yes() {
    let w = world();
    let hash = ready(&w);
    w.clear_log();
    let runs_before = w.state()["runs"].as_array().unwrap().len();
    let (out, _) = w.run(&["deploy", "hello-ally"]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-186") && text.contains(&hash),
        "names the hash to approve: {text}"
    );
    let (out, _) = w.run(&["deploy", "hello-ally", "--approve-plan", "sha256:00"]);
    assert!(says(&out).contains("GB-186"), "{}", says(&out));
    no_play(&w);
    assert_eq!(
        w.state()["runs"].as_array().unwrap().len(),
        runs_before,
        "nothing recorded"
    );
}

#[test]
fn deploy_runs_the_live_check_then_the_plays_and_records_what_it_holds() {
    let w = world();
    let hash = ready(&w);
    w.clear_log();
    // First deploy: the live preflight, then the by-hand admission stop.
    let (out, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10], "{}", says(&out));
    assert!(
        rows(&v, "approval").iter().any(|d| d.contains(&hash)),
        "{}",
        says(&out)
    );
    let st = w.state();
    let run = st["runs"].as_array().unwrap().last().unwrap();
    assert_eq!(
        (run["stage"].as_str(), run["plan"].as_str()),
        (Some("deploy"), Some(hash.as_str()))
    );
    assert!(st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "plan" && c["bound_to"] == hash.as_str() && c["how"] == "flag"));
    assert!(
        st["owned"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "udp port (tunnel)"
                && o["value"] == "51821"
                && o["scope"] == "hello-ally_edge"),
        "{}",
        st["owned"]
    );
    assert!(st["owned"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["status"] == "claimed"));

    // After the admission by hand: --from 11 declares it and deploys.
    w.clear_log();
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--from",
        "11",
        "--approve-plan",
        &hash,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [11, 12, 13], "{}", says(&out));
    let log = w.log();
    assert!(
        log.find("preflight").unwrap() < log.find("play ").unwrap(),
        "the live recheck runs before any play: {log}"
    );
    for play in [
        "peering-wg.yml",
        "peering-zenoh-edge.yml",
        "peering-spire.yml",
    ] {
        assert!(log.contains(play), "{play}: {log}");
    }
    assert!(
        !log.contains("goat federate"),
        "deploy renders nothing: {log}"
    );
    let st = w.state();
    assert!(
        st["owned"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["status"] == "deployed"),
        "{}",
        st["owned"]
    );
    assert_eq!(st["phases"]["10"]["outcome"], "passed");

    // A later deploy skips the declared admission while
    // its inputs hold, and runs the check and every play again.
    w.clear_log();
    let (out, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [11, 12, 13], "{}", says(&out));
    assert_eq!(rows(&v, "skipped").len(), 1);

    // test: acceptance alone.
    w.clear_log();
    let (out, v) = w.run(&["test", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [14]);
    assert_eq!(w.log(), "acceptance\n");
    assert_eq!(
        w.state()["runs"].as_array().unwrap().last().unwrap()["stage"],
        "test"
    );
}

#[test]
fn changed_inputs_or_a_changed_plan_need_a_new_plan() {
    let w = world();
    let hash = ready(&w);
    let frag = w.work.join("applied/ops/federation/acl/xab-fragment.json5");
    std::fs::write(&frag, "edited by hand\n").unwrap();
    w.clear_log();
    let (out, _) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-188") && text.contains("rendered changed since the plan"),
        "{text}"
    );
    no_play(&w);
    assert!(
        w.state()["owned"].as_array().unwrap().is_empty(),
        "nothing claimed"
    );

    // A plan edited by hand (here: to name another peering) no longer
    // matches its own hash, and the planner's reader refuses it.
    let other = std::fs::read_to_string(w.work.join("plan.json"))
        .unwrap()
        .replace("\"peering\": \"hello-ally\"", "\"peering\": \"xzz\"");
    std::fs::write(w.work.join("plan.json"), other).unwrap();
    let (out, _) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    assert!(says(&out).contains("GB-201"), "{}", says(&out));
}

#[test]
fn a_resource_another_peering_holds_on_the_same_host_is_refused() {
    let w = world();
    let hash = ready(&w);
    // Another peering's journal beside ours holds the same port on edge-1.
    let other = w.work.parent().unwrap().join("xzz");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("state.json"),
        r#"{"schema":"goatbuilder.federate.state/v1","peering":"xzz","updated":"","owned":[{"kind":"udp port (tunnel)","scope":"hello-ally_edge","value":"51821","status":"deployed","run":"r","at":""}]}"#,
    )
    .unwrap();
    let theirs = std::fs::read_to_string(other.join("state.json")).unwrap();
    w.clear_log();
    let (out, _) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-181") && text.contains("held by peering 'xzz'"),
        "{text}"
    );
    no_play(&w);
    assert_eq!(
        theirs,
        std::fs::read_to_string(other.join("state.json")).unwrap(),
        "the other peering's journal is untouched"
    );
}

#[test]
fn stage_words_need_a_partner_and_plan_flags_belong_to_deploy() {
    let w = world();
    let (out, _) = w.run(&["deploy"]);
    assert!(says(&out).contains("needs the partner"), "{}", says(&out));
    let (out, _) = w.run(&["hello-ally", "--approve-plan", "sha256:00", "--dry-run"]);
    assert!(
        says(&out).contains("belong to `goatbuilder federate deploy"),
        "{}",
        says(&out)
    );
}

#[test]
fn deploy_from_11_needs_the_admission_recorded_for_what_is_rendered_now() {
    let w = world();
    let hash = ready(&w);
    w.clear_log();
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--from",
        "11",
        "--approve-plan",
        &hash,
    ]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("GB-190"), "{text}");
    no_play(&w);
    let st = w.state();
    let last = st["runs"].as_array().unwrap().last().unwrap();
    assert_eq!(
        last["outcome"], "refused",
        "a refused deploy is journaled: {last}"
    );
    assert!(
        st["owned"].as_array().unwrap().is_empty(),
        "nothing claimed"
    );
}

#[test]
fn deploy_refuses_starts_and_flags_that_are_not_its_own() {
    let w = world();
    let hash = ready(&w);
    for args in [
        vec![
            "deploy",
            "hello-ally",
            "--from",
            "14",
            "--approve-plan",
            &hash,
        ],
        vec![
            "deploy",
            "hello-ally",
            "--from",
            "12",
            "--approve-plan",
            &hash,
        ],
        vec![
            "deploy",
            "hello-ally",
            "--to",
            "11",
            "--approve-plan",
            &hash,
        ],
        vec!["deploy", "hello-ally", "--resume", "--approve-plan", &hash],
        vec!["test", "hello-ally", "--resume"],
    ] {
        let (out, _) = w.run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", says(&out));
        assert!(says(&out).contains("GB-189"), "{args:?}: {}", says(&out));
    }
}

#[test]
fn a_dry_run_checks_a_given_hash_and_the_plans_inputs_and_runs_nothing() {
    let w = world();
    let hash = ready(&w);
    w.clear_log();
    let (out, v) = w.run(&["deploy", "hello-ally", "--dry-run", "--approve-plan", &hash]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10, 11, 12, 13]);
    no_play(&w);
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--dry-run",
        "--approve-plan",
        "sha256:00",
    ]);
    assert!(says(&out).contains("GB-186"), "{}", says(&out));
    std::fs::write(w.work.join("applied/extra.json5"), "x").unwrap();
    let (out, _) = w.run(&["deploy", "hello-ally", "--dry-run"]);
    assert!(
        says(&out).contains("GB-188") && says(&out).contains("rendered"),
        "{}",
        says(&out)
    );
}

#[test]
fn a_failed_deploy_keeps_its_claims_and_a_redeploy_keeps_deployed() {
    let w = world();
    let hash = ready(&w);
    w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    std::fs::write(&w.fail_12, "").unwrap();
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--from",
        "11",
        "--approve-plan",
        &hash,
    ]);
    assert!(!out.status.success());
    let fix = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "failed")
        .unwrap()["fix"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        fix.contains("goatbuilder federate deploy hello-ally") && fix.contains(&hash),
        "the fix carries the approval: {fix}"
    );
    let st = w.state();
    assert!(
        st["owned"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["status"] == "claimed"),
        "still reserved for this peering: {}",
        st["owned"]
    );
    std::fs::remove_file(&w.fail_12).unwrap();
    let (out, _) = w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(w.state()["owned"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["status"] == "deployed"));
    // Deploy again with the same plan, failing this time: deployed stays deployed.
    std::fs::write(&w.fail_12, "").unwrap();
    w.run(&["deploy", "hello-ally", "--approve-plan", &hash]);
    assert!(w.state()["owned"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["status"] == "deployed"));
}

#[test]
fn deploy_and_test_need_yes_without_a_terminal() {
    let w = world();
    let hash = ready(&w);
    let repo = w.repo.display().to_string();
    for stage in ["deploy", "test"] {
        let mut c = std::process::Command::new(federate_world::bin());
        for (k, v) in w.command(&[]).get_envs() {
            if let Some(v) = v {
                c.env(k, v);
            }
        }
        let out = c
            .args(if stage == "deploy" {
                vec![
                    "-C",
                    &repo,
                    "-o",
                    "json",
                    "federate",
                    stage,
                    "hello-ally",
                    "--approve-plan",
                    &hash,
                ]
            } else {
                vec!["-C", &repo, "-o", "json", "federate", stage, "hello-ally"]
            })
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let text = says(&out);
        assert!(!out.status.success(), "{stage}: {text}");
        assert!(
            text.contains("pass --yes without a terminal"),
            "{stage}: {text}"
        );
    }
}

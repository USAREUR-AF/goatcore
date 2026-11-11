//! Confirmation 2, phase 10 by the command, through the real binary against
//! the stand-in repository (federate_world): the summary and its digest, the
//! approval bound to it, the core's access control assembled, the dry run
//! and the restart, inside `deploy`.

mod federate_world;

use std::path::Path;

use federate_world::{phases_run, public_b64, rows, says, sha256_hex, world, World};

fn sha256_file(p: &Path) -> String {
    sha256_hex(&std::fs::read(p).unwrap())
}

/// Phases 1 to 9 with confirmation 1 by flag, and a plan; returns its hash.
fn ready(w: &World, confirm_digest: bool) -> String {
    let fp = sha256_hex(b"fake-ca-der");
    let key = public_b64("xab");
    // compose writes a fixed agreement in the stand-in, so its digest is known.
    let digest = sha256_file(&w.root.path().join("fake/agreement.json"));
    let mut args = vec![
        "hello-ally",
        "--to",
        "9",
        "--partner-key",
        &key,
        "--partner-ca-fingerprint",
        &fp,
    ];
    if confirm_digest {
        args.extend(["--agreement-digest", &digest]);
    }
    let (out, v) = w.run(&args);
    assert!(out.status.success(), "{}", says(&out));
    if confirm_digest {
        assert_eq!(
            phases_run(&v),
            [1, 2, 3, 4, 5, 6, 7, 8, 9],
            "{}",
            says(&out)
        );
    }
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

/// The digest to approve, from the stop's row.
fn admission_digest(v: &serde_json::Value) -> String {
    let d = rows(v, "stopped")
        .into_iter()
        .find(|d| d.contains("--approve-admission"))
        .expect("the stop names the digest")
        .to_string();
    d.split("--approve-admission ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn deploy_admits_the_edge_after_the_admission_is_approved_by_its_digest() {
    let w = world();
    let plan = ready(&w, true);
    w.clear_log();
    // Asked to admit, with nothing approved yet: the summary and its digest, and a stop.
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--review-in-pr",
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        rows(&v, "stopped").iter().any(|d| d.starts_with("GB-197")),
        "{}",
        says(&out)
    );
    let review = std::fs::read_to_string(w.work.join("admission/review.md")).unwrap();
    assert!(
        review.contains("fed.xab.xaa.01") && review.contains(&plan),
        "the summary names the plan: {review}"
    );
    let digest = review
        .split("--approve-admission ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(
        !w.root.path().join("core/config.json5").exists(),
        "nothing on the core changed"
    );

    // Approved by its digest: merged, assembled, dry run, restart, then 11 to 13.
    w.clear_log();
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10, 11, 12, 13], "{}", says(&out));
    let cfg = std::fs::read_to_string(w.root.path().join("core/config.json5")).unwrap();
    assert!(
        cfg.contains("fed-w-xab") && cfg.contains("fed.xab.xaa.01"),
        "{cfg}"
    );
    let log = w.log();
    let (dry, restart, play) = (
        log.find("dry-run").unwrap(),
        log.find("restart").unwrap(),
        log.find("peering-wg.yml").unwrap(),
    );
    assert!(
        dry < restart && restart < play,
        "dry run, then restart, then the transport: {log}"
    );
    assert!(
        !w.work.join("pki/partner-ca.pem").exists(),
        "their CA is the edge role's to install, not phase 10's"
    );
    let st = w.state();
    assert!(st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "admission"
            && c["bound_to"] == digest.as_str()
            && c["values"]["plan"] == plan.as_str()));
    assert_eq!(st["phases"]["10"]["outcome"], "passed");
}

#[test]
fn another_digest_or_an_unconfirmed_agreement_changes_nothing() {
    let w = world();
    // Keys confirmed at phase 5; phases 6 to 9 run without comparing the digest.
    let (fp, key) = (sha256_hex(b"fake-ca-der"), public_b64("xab"));
    let (out, _) = w.run(&[
        "hello-ally",
        "--to",
        "5",
        "--partner-key",
        &key,
        "--partner-ca-fingerprint",
        &fp,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--to", "9"]);
    assert!(out.status.success(), "{}", says(&out));
    let (out, _) = w.run(&["plan", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    let plan = plan_hash(&w);

    // Another digest.
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        "sha256:00",
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", says(&out));
    assert!(says(&out).contains("GB-194"), "{}", says(&out));

    // The right digest, but the agreement was never compared with the partner.
    let (_, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &plan, "--confirm"]);
    let digest = admission_digest(&v);
    w.clear_log();
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ]);
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-194") && text.contains("has not been confirmed with the partner"),
        "{text}"
    );
    assert!(
        !w.root.path().join("core/config.json5").exists(),
        "nothing on the core changed"
    );
    assert!(!w.log().contains("restart"), "{}", w.log());
}

#[test]
fn without_the_flags_phase_10_stops_as_before() {
    let w = world();
    let plan = ready(&w, true);
    let (out, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &plan]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10]);
    assert!(
        rows(&v, "note").iter().any(|d| d.contains("OPERATOR STEP")),
        "{}",
        says(&out)
    );
    assert!(!w.root.path().join("core/config.json5").exists());
}

#[test]
fn a_failed_dry_run_puts_every_file_back() {
    let w = world();
    let plan = ready(&w, true);
    let org_before = std::fs::read_to_string(w.root.path().join("core/org.json5")).unwrap();
    let (_, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &plan, "--confirm"]);
    let digest = admission_digest(&v);
    let out = w
        .command(&[
            "deploy",
            "hello-ally",
            "--approve-plan",
            &plan,
            "--approve-admission",
            &digest,
        ])
        .env("FED_CORE_DRY_RUN", "exit 3")
        .output()
        .unwrap();
    let text = says(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-195") && text.contains("every file placed is put back"),
        "{text}"
    );
    assert_eq!(
        org_before,
        std::fs::read_to_string(w.root.path().join("core/org.json5")).unwrap()
    );
    assert!(
        !w.root.path().join("core/config.json5").exists(),
        "a file that was not there is gone again"
    );
}

#[test]
fn a_failed_restart_keeps_the_first_backup_when_retried() {
    let w = world();
    let plan = ready(&w, true);
    let org_before = std::fs::read_to_string(w.root.path().join("core/org.json5")).unwrap();
    // The restart route is part of what is approved, so the digest is read
    // with the same settings it is approved under.
    let out = w
        .command(&["deploy", "hello-ally", "--approve-plan", &plan, "--confirm"])
        .env("FED_CORE_RESTART", "exit 4")
        .output()
        .unwrap();
    let digest = admission_digest(&serde_json::from_slice(&out.stdout).unwrap());
    let out = w
        .command(&[
            "deploy",
            "hello-ally",
            "--approve-plan",
            &plan,
            "--approve-admission",
            &digest,
        ])
        .env("FED_CORE_RESTART", "exit 4")
        .output()
        .unwrap();
    assert!(says(&out).contains("GB-196"), "{}", says(&out));
    // The original is in that run's backup folder.
    let backups: Vec<_> = walk(&w.work.join("admission"))
        .into_iter()
        .filter(|p| {
            p.to_string_lossy().contains("/backup/") && p.to_string_lossy().ends_with("org.json5")
        })
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(org_before, std::fs::read_to_string(&backups[0]).unwrap());
    // Retried: a new digest (the org fragment in place changed), a new backup folder.
    let (_, v) = w.run(&["deploy", "hello-ally", "--approve-plan", &plan, "--confirm"]);
    let digest2 = admission_digest(&v);
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest2,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(
        org_before,
        std::fs::read_to_string(&backups[0]).unwrap(),
        "the first backup is never overwritten"
    );
}

fn walk(d: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn keys_are_placed_for_the_owner_only_and_a_missing_key_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    let plan = ready(&w, true);
    let key = w.root.path().join("core/core-key.pem");
    let cert = w.root.path().join("core/core.pem");
    let run = |extra: &[&str]| {
        let mut args = vec!["deploy", "hello-ally", "--approve-plan", &plan];
        args.extend_from_slice(extra);
        let out = w
            .command(&args)
            .env("FED_CORE_CERT", &cert)
            .env("FED_CORE_KEY", &key)
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        (out, v)
    };
    let (_, v) = run(&["--confirm"]);
    let digest = admission_digest(&v);
    let (out, _) = run(&["--approve-admission", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(
        std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600,
        "a new key file is 0600"
    );
    let staged: Vec<_> = walk(&w.work.join("admission"))
        .into_iter()
        .filter(|p| p.ends_with("staged/core-key.pem"))
        .collect();
    assert!(
        staged.is_empty(),
        "the staged key copy is removed once placed: {staged:?}"
    );
    // A missing key that FED_CORE_KEY says to place: refused, nothing placed.
    let w2 = world();
    let plan2 = ready(&w2, true);
    std::fs::remove_file(w2.work.join("pki/core-key.pem")).unwrap();
    let out = w2
        .command(&[
            "deploy",
            "hello-ally",
            "--approve-plan",
            &plan2,
            "--approve-admission",
            "sha256:00",
        ])
        .env("FED_CORE_KEY", w2.root.path().join("core/core-key.pem"))
        .output()
        .unwrap();
    let text = says(&out);
    assert!(
        text.contains("GB-188") || text.contains("is missing or is not a PEM private key"),
        "{text}"
    );
    assert!(!w2.root.path().join("core/core-key.pem").exists());
}

#[test]
fn admission_flags_belong_to_deploy() {
    let w = world();
    let (out, _) = w.run(&[
        "hello-ally",
        "--approve-admission",
        "sha256:00",
        "--dry-run",
    ]);
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
}

/// The admission digest the review names (`--review-in-pr` writes it and stops).
fn reviewed_digest(w: &World, plan: &str) -> String {
    let (out, _) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        plan,
        "--review-in-pr",
    ]);
    assert!(out.status.success(), "{}", says(&out));
    std::fs::read_to_string(w.work.join("admission/review.md"))
        .unwrap()
        .split("--approve-admission ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// Admission that places their CA under pki/, a section the plan records.
fn ca_in_pki(w: &World) {
    let gv = w
        .repo
        .join("ops/ansible/group_vars/hello-ally_edge/peering.yml");
    let text = std::fs::read_to_string(&gv).unwrap();
    let text: String = text
        .lines()
        .map(|l| {
            if l.starts_with("goat_zenoh_edge_partner_ca_path:") {
                format!(
                    "goat_zenoh_edge_partner_ca_path: {}",
                    w.work.join("pki/partner-ca.pem").display()
                )
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&gv, text + "\n").unwrap();
}

/// Deploy's own phases change what the plan was made from: here the
/// admission places their CA in pki/. One approved plan carries deploy from
/// 10 through 13 in one run, with no second plan.
#[test]
fn deploy_carries_past_its_own_admission() {
    let w = world();
    ca_in_pki(&w);
    let plan = ready(&w, true);
    let digest = reviewed_digest(&w, &plan);
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10, 11, 12, 13], "{}", says(&out));
    assert!(
        w.work.join("pki/partner-ca.pem").exists(),
        "the admission changed pki/"
    );
}

/// What deploy's last phase left is the reference for the next run of the
/// same plan: a hand change after it is refused, and the same run passes
/// once the change is gone.
#[test]
fn a_change_after_deploys_last_phase_is_refused() {
    let w = world();
    ca_in_pki(&w);
    let plan = ready(&w, true);
    let digest = reviewed_digest(&w, &plan);
    let deploy = [
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ];
    std::fs::write(&w.fail_12, "").unwrap();
    let (out, _) = w.run(&deploy);
    assert!(!out.status.success(), "phase 12 fails: {}", says(&out));
    std::fs::remove_file(&w.fail_12).unwrap();

    let by_hand = w.work.join("pki/by-hand.pem");
    std::fs::write(&by_hand, "not deploy's\n").unwrap();
    let (out, _) = w.run(&deploy);
    let text = says(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("GB-188") && text.contains("pki"), "{text}");

    std::fs::remove_file(&by_hand).unwrap();
    let (out, v) = w.run(&deploy);
    assert!(out.status.success(), "{}", says(&out));
    assert!(phases_run(&v).ends_with(&[12, 13]), "{}", says(&out));
}

/// Run `cmd` with a terminal on stdin, stdout and stderr (a pseudo-terminal),
/// answering `yes` to each "Type yes" question. Returns everything printed.
fn at_a_terminal(cmd: &std::process::Command) -> String {
    const PTY: &str = r#"import os, pty, sys
pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], sys.argv[1:])
seen, asked = b"", 0
while True:
    try:
        d = os.read(fd, 65536)
    except OSError:
        break
    if not d:
        break
    seen += d
    while seen.count(b"Type yes") > asked:
        asked += 1
        os.write(fd, b"yes\n")
os.waitpid(pid, 0)
sys.stdout.buffer.write(seen)
"#;
    let mut py = std::process::Command::new("python3");
    py.arg("-c")
        .arg(PTY)
        .arg(cmd.get_program())
        .args(cmd.get_args());
    py.env_clear();
    for (k, v) in cmd.get_envs() {
        if let Some(v) = v {
            py.env(k, v);
        }
    }
    if let Some(d) = cmd.get_current_dir() {
        py.current_dir(d);
    }
    let out = py.output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// With a person at the terminal, the plain `goatbuilder federate <partner>`
/// does not stop at phase 10: it makes the plan, asks to approve it, asks to
/// approve what our core admits, and carries on through 10 to 14.
#[test]
fn the_plain_run_admits_at_its_prompt_and_carries_on() {
    let w = world();
    ready(&w, true);
    w.clear_log();
    let printed = at_a_terminal(&w.command_with(&["-o", "table"], &["hello-ally", "--from", "10"]));
    assert_eq!(
        printed.matches("Type yes").count(),
        2,
        "the plan, then the admission: {printed}"
    );
    let st = w.state();
    for n in ["10", "11", "12", "13", "14"] {
        assert_eq!(st["phases"][n]["outcome"], "passed", "phase {n}: {printed}");
    }
    assert!(w.work.join("plan.json").exists(), "the plan was made");
    let cfg = std::fs::read_to_string(w.root.path().join("core/config.json5")).unwrap();
    assert!(
        cfg.contains("fed.xab.xaa.01"),
        "admitted on our core: {cfg}"
    );
    assert!(w.log().contains("acceptance"), "{}", w.log());
}

/// Without a terminal the plain run still stops at phase 10, and the stop
/// names the commands that admit by the command.
#[test]
fn without_a_terminal_the_stop_at_10_names_plan_and_deploy() {
    let w = world();
    ready(&w, true);
    let (out, v) = w.run(&["hello-ally", "--from", "10"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10]);
    assert!(
        rows(&v, "note")
            .iter()
            .any(|d| d.contains("goatbuilder federate plan hello-ally")
                && d.contains("goatbuilder federate deploy hello-ally")),
        "{}",
        says(&out)
    );
}

/// Not every peering declares `hosts:`: `goatbuilder federate allocate`
/// renders this peering's groups into its own `peering-<edge group>.ini`
/// beside the hand-written `peering-edges.ini`, which holds another peering.
/// The planner reads both, as the plays do, so the host-allocation row is not
/// SKIP and the plan and deploy reach the admission.
#[test]
fn a_peering_in_its_own_inventory_file_is_planned_and_deployed() {
    let w = world();
    let inv = w.repo.join("ops/ansible/inventory");
    std::fs::write(
        inv.join("peering-edges.ini"),
        "[velmara_edge]\nedge-9 ansible_host=192.0.2.90\n\n[velmara_core]\ncore-9 ansible_host=192.0.2.91\n",
    )
    .unwrap();
    std::fs::write(
        inv.join("peering-hello-ally_edge.ini"),
        "[peering_edges:children]\nhello-ally_edge\n\n[hello-ally_edge]\nedge-1 ansible_host=192.0.2.10\n\n\
         [peering_cores:children]\nhello-ally_core\n\n[hello-ally_core]\ncore-1 ansible_host=192.0.2.11\n",
    )
    .unwrap();
    let plan = ready(&w, true);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(w.work.join("plan.json")).unwrap()).unwrap();
    let row = saved["readiness"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["check"] == "host-allocation")
        .cloned()
        .unwrap();
    assert_eq!(row["outcome"], "PASS", "{row}");
    let digest = reviewed_digest(&w, &plan);
    let (out, v) = w.run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10, 11, 12, 13], "{}", says(&out));
    assert_eq!(w.state()["phases"]["10"]["outcome"], "passed");
}

/// A core that keeps no committed slot list (no
/// `group_vars/<env>_hosts/zenoh-lazy-storage.yml`) needs no slot registry and
/// no FED_SLOT_CHECK=skip: the check is NOT APPLICABLE, as preflight judges
/// it. A core that keeps one still needs its registry named (GB-194).
#[test]
fn the_slot_check_is_not_applicable_without_lazy_slot_storage() {
    let w = world();
    let plan = ready(&w, true);
    let run = |extra: &[&str]| {
        let out = w
            .command(extra)
            .env_remove("FED_SLOT_CHECK")
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        (out, v)
    };
    // The slot list exists: the registry must be named, and nothing is placed.
    let anchors = w
        .repo
        .join("ops/ansible/group_vars/zagros_hosts/zenoh-lazy-storage.yml");
    std::fs::create_dir_all(anchors.parent().unwrap()).unwrap();
    std::fs::write(&anchors, "zenoh_lazy_storage_slots: []\n").unwrap();
    let (out, v) = run(&["deploy", "hello-ally", "--approve-plan", &plan, "--review-in-pr"]);
    assert!(
        rows(&v, "failed")
            .iter()
            .any(|d| d.starts_with("GB-194") && d.contains("FED_SLOT_REGISTRY_DB")),
        "{}",
        says(&out)
    );
    // No slot list: the summary and its digest, then the approved admission
    // with the slot check NOT APPLICABLE.
    std::fs::remove_file(&anchors).unwrap();
    let (out, _) = run(&["deploy", "hello-ally", "--approve-plan", &plan, "--review-in-pr"]);
    assert!(out.status.success(), "{}", says(&out));
    let review = std::fs::read_to_string(w.work.join("admission/review.md")).unwrap();
    let digest = review
        .split("--approve-admission ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let (out, v) = run(&[
        "deploy",
        "hello-ally",
        "--approve-plan",
        &plan,
        "--approve-admission",
        &digest,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        rows(&v, "slots").iter().any(|d| d.starts_with("NOT APPLICABLE")
            && d.contains("zagros_hosts/zenoh-lazy-storage.yml")),
        "{}",
        says(&out)
    );
    assert!(w.log().contains("restart"), "{}", w.log());
}

/// A core the goat-zenoh role deploys: its declarations in the stand-in
/// repository (the role's defaults, the fleet pin and its play, the core
/// router's inventory group and group_vars), and an `ansible` that reads and
/// writes a stand-in core host under `<root>/remote`. Returns that bin dir.
fn role_core(w: &World) -> std::path::PathBuf {
    let r = w.root.path();
    let a = w.repo.join("ops/ansible");
    federate_world::write(&a.join("roles/goat-zenoh/defaults/main.yml"), "zenoh_router_image: \"x\"\nzenoh_stack: \"{{ site_id }}-zenoh\"   # a shared role must not name a goatnet\ngoat_zenoh_acl_org_path: \"/etc/{{ zenoh_stack }}/acl-org-fragment.json5\"\nzenoh_sovereign_peerings: []\n");
    federate_world::write(&a.join("zenoh-deploy.yml"), "- hosts: \"{{ target | mandatory }}\"\n  roles: [goat-zenoh]\n");
    federate_world::write(&w.repo.join("ops/pins/fleet-pins.yaml"), "components:\n  zenoh-router:\n    var: zenoh_router_image\n    deploy_playbook: zenoh-deploy.yml\n");
    federate_world::write(&a.join("inventory/core-router.ini"), "[core_router]\nxaa-core ansible_host=192.0.2.11 ansible_user=goat\n");
    federate_world::write(&a.join("group_vars/core_router/zenoh.yml"), "zenoh_router_image: \"ghcr.io/example-org/zenoh@sha256:00\"\nsite_id: xaa\n");
    let org = std::fs::read_to_string(r.join("core/org.json5")).unwrap();
    federate_world::write(&r.join("remote/etc/xaa-zenoh/acl-org-fragment.json5"), &org);
    let bin = r.join("role-bin");
    federate_world::exe(
        &bin.join("ansible"),
        &format!(
            "#!/bin/sh\necho \"ansible $*\" >> '{log}'\nwhile [ $# -gt 0 ]; do case $1 in -m) m=$2; shift;; -a) a=$2; shift;; esac; shift; done\nsrc=$(echo \"$a\" | sed 's/.*src=\\([^ ]*\\).*/\\1/'); dest=$(echo \"$a\" | sed 's/.*dest=\\([^ ]*\\).*/\\1/')\ncase $m in *fetch) cp '{rem}'\"$src\" \"$dest\";; *copy) mkdir -p \"$(dirname '{rem}'\"$dest\")\"; cp \"$src\" '{rem}'\"$dest\";; esac\n",
            log = w.log.display(),
            rem = r.join("remote").display()
        ),
    );
    bin
}

#[test]
fn a_role_core_is_admitted_through_the_roles_play_and_its_fragment_on_the_host() {
    let w = world();
    let bin = role_core(&w);
    let plan = ready(&w, true);
    let role_run = |extra: &[&str]| {
        let mut c = w.command(extra);
        for k in ["FED_CORE_ACL_ORG", "FED_CORE_ACL_SEED", "FED_CORE_ACL_OUT", "FED_CORE_DRY_RUN", "FED_CORE_RESTART"] {
            c.env_remove(k);
        }
        c.env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()));
        let out = c.output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        (out, v)
    };
    let (out, _) = role_run(&["deploy", "hello-ally", "--approve-plan", &plan, "--review-in-pr"]);
    assert!(out.status.success(), "{}", says(&out));
    let review = std::fs::read_to_string(w.work.join("admission/review.md")).unwrap();
    assert!(
        review.contains("/etc/xaa-zenoh/acl-org-fragment.json5 on [core_router]")
            && review.contains("zenoh-deploy.yml on [core_router] in inventory/core-router.ini (site_id xaa)"),
        "{review}"
    );
    let remote = w.root.path().join("remote/etc/xaa-zenoh/acl-org-fragment.json5");
    assert!(!std::fs::read_to_string(&remote).unwrap().contains("fed-w-xab"), "nothing on the core changed before approval");
    let digest = review.split("--approve-admission ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    w.clear_log();
    let (out, v) = role_run(&["deploy", "hello-ally", "--approve-plan", &plan, "--approve-admission", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [10, 11, 12, 13], "{}", says(&out));
    let placed = std::fs::read_to_string(&remote).unwrap();
    assert!(placed.contains("fed-w-xab") && placed.contains("vendor-a") && placed.contains("fed-w-xac"), "merged on the host, every other block kept: {placed}");
    let log = w.log();
    let (fetch, copy, dry, play) = (
        log.find("ansible.builtin.fetch").unwrap(),
        log.find("ansible.builtin.copy").unwrap(),
        log.find("play --diff").unwrap(),
        log.find("play zenoh-deploy.yml").unwrap(),
    );
    assert!(fetch < copy && copy < dry && dry < play, "fetch, place, dry run, restart: {log}");
}

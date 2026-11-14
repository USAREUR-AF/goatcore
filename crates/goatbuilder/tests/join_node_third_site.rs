// Gate row H1: `join-node` runs phases 1 to 15 in order on a node that is not
// yet enrolled, at a goatnet that already has a joiner, for a third site that
// declares signal federation off (docs/design/phone-node.md, "What the run
// needed outside the tool", rows 1 to 3).
//
// The fixture `ops/goatbuilder/fixtures/join-node-third-site` is copied with
// the real gates phases 3, 7, 12 and 13 run (standup-config-preflight.py and
// signal-federation-check.py). Every program that would reach a host is a
// stand-in that records its arguments. The ansible stand-in refuses as the
// measured run did when a bootstrap pass waits for a service the next step
// starts: a site-bootstrap run before Ory is deployed, without
// `--skip-tags after-ory`, fails on the management service.
//
// The run starts with no --from. A manual phase (1, 8, 10) stops it with
// "re-run with --from <next>", and the test resumes exactly there, as the
// operator does; at the phase-10 stop it records the enrolled node's mesh
// address, which phase 12 checks.

//
// Gate row H2, on the same fixture: from phase 2 the run reaches phase 15
// with no operator step, the setup key minted where it is used, and no secret
// value in any argument list, run record, log or report
// (docs/design/goatbuilder.md, join-node part "(f)").

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Values the environment holds for a run. No test may find one of them in
/// an argument list, a run record, a log or a report.
const SETUP_KEY: &str = "SETUPKEY-SENTINEL-0001";
const NODE_PASS: &str = "NODEPASS-SENTINEL-0002";
const MINTED_KEY: &str = "MINTEDKEY-SENTINEL-0003";
const PG_PASS: &str = "PGPASS-SENTINEL-0004";

struct Site {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    checkout: PathBuf,
    /// Every argument list a stand-in was started with, one per line.
    log: PathBuf,
    /// Everything piped to the node over ssh.
    stdin_log: PathBuf,
    /// The first-login password, as the environment holds it.
    node_pass: String,
}

/// `password_only`: the node takes no key yet, so phase 2 seeds it with the
/// first-login password.
fn site_with(password_only: bool, node_pass: &str) -> Site {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let checkout = root.join("checkout");
    let r = repo_root();
    copy_tree(
        &r.join("ops/goatbuilder/fixtures/join-node-third-site"),
        &checkout,
    );
    std::fs::remove_file(checkout.join("README.md")).unwrap();
    // The real gates, the real mesh record and the real secrets injector.
    std::fs::create_dir_all(checkout.join("ops/scripts")).unwrap();
    // The end-state model, which says which health publishers the site declares.
    copy_tree(
        &r.join("ops/goatbuilder/endstate"),
        &checkout.join("ops/goatbuilder/endstate"),
    );
    std::fs::create_dir_all(checkout.join("ops/secrets")).unwrap();
    std::fs::create_dir_all(checkout.join("ops/federation/standup")).unwrap();
    for s in [
        "ops/scripts/standup-config-preflight.py",
        // Phase 3's clock check and the federation clock code it shares.
        "ops/scripts/join-node-clock-check.py",
        "ops/federation/standup/preflight_live.py",
        "ops/scripts/signal-federation-check.py",
        "ops/scripts/standup-mesh-record.py",
        "ops/scripts/inventory-hosts.py",
        "ops/scripts/with-secrets",
        "ops/scripts/seed-ssh-key.sh",
        "ops/secrets/posture.py",
        // Every health publisher's play: a play in the checkout is not a
        // reason to run it, the site's declaration is.
        "ops/ansible/fabric-health-deploy.yml",
        "ops/ansible/mesh-health-deploy.yml",
        "ops/ansible/presence-health-deploy.yml",
        "ops/ansible/time-health-deploy.yml",
        "ops/ansible/consistency-health-deploy.yml",
        "ops/ansible/fabric-crawler-deploy.yml",
    ] {
        std::fs::copy(r.join(s), checkout.join(s)).unwrap();
    }
    let log = root.join("argv.log");
    let stdin_log = root.join("stdin.log");
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    if password_only {
        std::fs::write(state.join("password-only"), "").unwrap();
    }
    let (l, i, st) = (log.display(), stdin_log.display(), state.display());
    // with-aws starts every play (`with-aws <profile> -- ansible-preflight.sh
    // <inventories> <play> ...`). The enrolment play stands in for the mint:
    // the key goes to a file on the node, never to its output, and the node's
    // mesh address to the file the run names.
    executable(
        &checkout.join("ops/scripts/with-aws"),
        &format!(
            "#!/bin/sh\nprintf 'play %s\\n' \"$*\" >> '{l}'\n\
             case \"$*\" in\n\
             *ory-deploy.yml*) touch '{st}/ory-up' ;;\n\
             *mesh-enroll-host.yml*)\n\
               out=''; for a; do case \"$a\" in mesh_ips_out=*) out=\"${{a#mesh_ips_out=}}\" ;; esac; done\n\
               [ -n \"$out\" ] || {{ echo 'mesh-enroll-host.yml: no mesh_ips_out'; exit 2; }}\n\
               printf '%s\\n' '{MINTED_KEY}' > '{st}/node-run-goat-phase8-setup-key'\n\
               printf '{{\"gamma-core-1\": \"100.127.0.3\"}}\\n' > \"$out\" ;;\n\
             *bootstrap*) case \"$*\" in *'--skip-tags after-ory'*) ;; *)\n\
               [ -f '{st}/ory-up' ] || {{ echo 'TASK [check the management service is up] fatal: netbird-mgmt is not up: it waits for Hydra, which ory-deploy.yml has not started'; exit 2; }} ;; esac ;;\n\
             esac\nexit 0\n"
        ),
    );
    // mint-site-certs.sh mints on this machine; the stand-in records it.
    executable(
        &checkout.join("ops/scripts/mint-site-certs.sh"),
        &format!("#!/bin/sh\nprintf 'mint %s\\n' \"$*\" >> '{l}'\n"),
    );
    executable(
        &checkout.join("ops/scripts/ansible-preflight.sh"),
        "#!/bin/sh\nexit 0\n",
    );
    // Phase 15's acceptance on the new node.
    executable(
        &checkout.join("ops/validation/generate-values.py"),
        &format!(
            "import sys\nopen('{l}', 'a').write('values ' + ' '.join(sys.argv[1:]) + '\\n')\n"
        ),
    );
    executable(&checkout.join("ops/validation/run-acceptance.py"), &format!("import sys\nopen('{l}', 'a').write('acceptance ' + ' '.join(sys.argv[1:]) + '\\n')\nprint('every applicable row PASS')\n"));
    // ssh reaches the node. It records its arguments, and what is piped to it
    // separately. A password-only node refuses key auth until the key is
    // pushed, and sudo -n until the sudoers entry is written.
    executable(
        &root.join("bin/ssh"),
        &format!(
            "#!/bin/sh\nprintf 'ssh %s\\n' \"$*\" >> '{l}'\n\
             case \"$*\" in *BatchMode=yes*)\n\
               if [ -f '{st}/password-only' ]; then\n\
                 case \"$*\" in *'sudo -n true'*) [ -f '{st}/sudo-seeded' ] || exit 1 ;; *) [ -f '{st}/key-seeded' ] || exit 255 ;; esac\n\
               fi ;;\n\
             esac\n\
             case \"$*\" in *'bash -s'*|*'sudo -S'*) cat >> '{i}' ;; esac\n\
             case \"$*\" in *'sudo -S'*) touch '{st}/sudo-seeded' ;; esac\n\
             exit 0\n"
        ),
    );
    // sshpass takes the password from SSHPASS (-e), then runs its command.
    executable(
        &root.join("bin/sshpass"),
        &format!(
            "#!/bin/sh\nprintf 'sshpass %s\\n' \"$*\" >> '{l}'\n\
             [ \"$1\" = -e ] && shift\n\
             case \"$1\" in ssh-copy-id) touch '{st}/key-seeded'; exit 0 ;; esac\n\
             exec \"$@\"\n"
        ),
    );
    Site {
        _tmp: tmp,
        root,
        checkout,
        log,
        stdin_log,
        node_pass: node_pass.to_string(),
    }
}

fn site() -> Site {
    site_with(false, NODE_PASS)
}

fn join_node(s: &Site, from: Option<&str>) -> (Output, serde_json::Value) {
    let mut args = vec![
        "-C".to_string(),
        s.checkout.display().to_string(),
        "--yes".into(),
        "-o".into(),
        "json".into(),
        "join-node".into(),
        "zagros".into(),
        "--site".into(),
        "gamma".into(),
        "--node".into(),
        "gamma-core-1".into(),
        "--class".into(),
        "core".into(),
        "--address".into(),
        "192.0.2.30".into(),
    ];
    if let Some(n) = from {
        args.extend(["--from".to_string(), n.to_string()]);
    }
    let path = format!(
        "{}:{}",
        s.root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(&args)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &s.root)
        .env("GOAT_SECRET_STORE", "env")
        .env("OWNER_EMAIL", "owner@example.org")
        .env("GAMMA_NB_SETUP_KEY", SETUP_KEY)
        .env("GAMMA_NODE_PASS", &s.node_pass)
        .env("GAMMA_PG_PASS", PG_PASS)
        .env("ZAGROS_SIGNAL_FEDERATION_TOKEN", "token-stand-in")
        .env("GOAT_NO_FUN", "1")
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

/// Every file under `dir`, read as text.
fn texts_under(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            texts_under(&p, out);
        } else if let Ok(bytes) = std::fs::read(&p) {
            out.push((p, String::from_utf8_lossy(&bytes).into_owned()));
        }
    }
}

/// No secret value in any argument list a stand-in recorded, in the run
/// records and logs the runs left, or in what the runs printed; the setup
/// key, which no phase needs, not even in what was piped to the node.
fn assert_no_secret_in_any_record(s: &Site, printed: &str) {
    let mut places: Vec<(PathBuf, String)> = vec![
        (
            s.log.clone(),
            std::fs::read_to_string(&s.log).unwrap_or_default(),
        ),
        (
            PathBuf::from("<what the runs printed>"),
            printed.to_string(),
        ),
    ];
    texts_under(&s.root.join(".goat"), &mut places);
    for (where_, text) in &places {
        for secret in [SETUP_KEY, NODE_PASS, MINTED_KEY, PG_PASS] {
            assert!(
                !text.contains(secret),
                "{secret} is in {}:\n{text}",
                where_.display()
            );
        }
    }
    let piped = std::fs::read_to_string(&s.stdin_log).unwrap_or_default();
    for secret in [SETUP_KEY, MINTED_KEY] {
        assert!(
            !piped.contains(secret),
            "{secret} was piped to the node:\n{piped}"
        );
    }
}

fn says(o: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The phases a run printed, in order.
fn phases(v: &serde_json::Value) -> Vec<u64> {
    v["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| c["name"] == "phase")
                .filter_map(|c| {
                    c["detail"]
                        .as_str()?
                        .strip_prefix("=== Phase ")?
                        .split_whitespace()
                        .next()?
                        .parse()
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `--from` a manual stop asks for, if the run stopped at one.
fn resume_at(v: &serde_json::Value) -> Option<String> {
    v["checks"].as_array()?.iter().find_map(|c| {
        let d = c["detail"].as_str()?;
        d.contains("OPERATOR STEP")
            .then(|| d.rsplit("--from ").next().unwrap().trim().to_string())
    })
}

#[test]
fn a_third_site_that_declares_federation_off_joins_from_phase_1_to_15() {
    let s = site();
    let mut from: Option<String> = None;
    let mut ran: Vec<u64> = Vec::new();
    for _ in 0..6 {
        if from.as_deref() == Some("11") {
            // The operator's step at phase 10: the node enrolled, and its mesh
            // address is recorded in the site's vars and the mesh inventory.
            for f in [
                "ops/ansible/group_vars/gamma_hosts/vars.yml",
                "ops/ansible/inventory/mgmt-hosts-mesh.ini",
            ] {
                let p = s.checkout.join(f);
                let t = std::fs::read_to_string(&p).unwrap();
                std::fs::write(&p, t.replace("TBD_GAMMA_MESH_IP_A", "100.127.0.3")).unwrap();
            }
        }
        let (out, v) = join_node(&s, from.as_deref());
        assert!(
            out.status.success(),
            "join-node {} stopped with a refusal after phases {ran:?} + {:?}:\n{}\nplays:\n{}",
            from.as_deref()
                .map(|n| format!("--from {n}"))
                .unwrap_or_else(|| "(no --from)".into()),
            phases(&v),
            says(&out),
            std::fs::read_to_string(&s.log).unwrap_or_default()
        );
        ran.extend(phases(&v));
        match resume_at(&v) {
            Some(next) => from = Some(next),
            None => break,
        }
    }
    let mut seen = ran.clone();
    seen.dedup();
    assert_eq!(
        seen,
        (1..=15).collect::<Vec<u64>>(),
        "every phase, once each, in order"
    );
    let log = std::fs::read_to_string(&s.log).unwrap();
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let ory = plays
        .iter()
        .position(|l| l.contains("ory-deploy.yml"))
        .expect("ory deployed");
    let pass1 = plays
        .iter()
        .position(|l| l.contains("participates_in_spock=false"))
        .expect("pass 1 ran");
    assert!(pass1 < ory, "pass 1 runs before Ory: {plays:#?}");
    assert!(
        log.lines().any(|l| l.starts_with("acceptance ")),
        "phase 15 ran acceptance:\n{log}"
    );
    assert!(
        !plays
            .iter()
            .any(|l| l.contains("signal-federation-transport.yml")),
        "federation declared off converges nothing: {plays:#?}"
    );
}

/// Gate row H2: with no operator step between phase 2 and phase 15. The run
/// stops only at phase 1, reaching the node; from 2 it runs to 15 in one go,
/// enrolling the node from a key minted where it is used and recording its
/// mesh address itself, and no secret is in any argument list.
#[test]
fn from_phase_2_a_node_joins_with_no_operator_step_and_no_secret_in_any_argv() {
    let s = site();
    let (out, v) = join_node(&s, None);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases(&v), vec![1], "only phase 1 is a person's step");
    assert_eq!(resume_at(&v).as_deref(), Some("2"), "{}", says(&out));
    let mut printed = says(&out);
    let (out, v) = join_node(&s, Some("2"));
    printed.push_str(&says(&out));
    assert!(
        out.status.success(),
        "{}\nargv:\n{}",
        says(&out),
        std::fs::read_to_string(&s.log).unwrap_or_default()
    );
    assert_eq!(
        resume_at(&v),
        None,
        "an operator step after phase 2: {}",
        says(&out)
    );
    let mut seen = phases(&v);
    seen.dedup();
    assert_eq!(seen, (2..=15).collect::<Vec<u64>>(), "{}", says(&out));
    // The node's mesh address is in the checkout, and the run says which files it wrote.
    let gv = std::fs::read_to_string(
        s.checkout
            .join("ops/ansible/group_vars/gamma_hosts/vars.yml"),
    )
    .unwrap();
    assert!(
        gv.contains("100.127.0.3") && !gv.contains("TBD_GAMMA_MESH_IP_A"),
        "{gv}"
    );
    let all = says(&out);
    for f in [
        "ops/ansible/group_vars/gamma_hosts/vars.yml",
        "ops/ansible/inventory/mgmt-hosts-mesh.ini",
    ] {
        assert!(
            all.contains(&format!("wrote {f}")),
            "the summary names {f}: {all}"
        );
    }
    let mut records = Vec::new();
    texts_under(&s.root.join(".goat/state/runs"), &mut records);
    let rec = records
        .iter()
        .filter(|(p, _)| p.ends_with("run.json"))
        .map(|(_, t)| serde_json::from_str::<serde_json::Value>(t).unwrap())
        .find(|r| {
            r["args"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "--from"))
        })
        .expect("a record of the run from phase 2");
    assert_eq!(
        rec["wrote"],
        serde_json::json!([
            "ops/ansible/group_vars/gamma_hosts/vars.yml",
            "ops/ansible/inventory/mgmt-hosts-mesh.ini"
        ]),
        "{rec}"
    );
    assert_no_secret_in_any_record(&s, &printed);
}

/// A node that takes only its first-login password: phase 2 seeds the key
/// with it, and the password reaches the node on stdin, never in an argument
/// list.
#[test]
fn a_password_only_node_is_seeded_with_no_password_in_any_argv() {
    let s = site_with(true, NODE_PASS);
    let (out, _) = join_node(&s, Some("2"));
    let argv = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(out.status.success(), "{}\nargv:\n{argv}", says(&out));
    assert!(
        argv.contains("sshpass -e ssh-copy-id"),
        "the key was seeded with the password:\n{argv}"
    );
    assert_no_secret_in_any_record(&s, &says(&out));
    assert!(
        std::fs::read_to_string(&s.stdin_log)
            .unwrap_or_default()
            .contains(NODE_PASS),
        "sudo took the password on stdin"
    );
}

/// A first-login password that is still a secret store's reference after the
/// injector ran is refused (GB-269), naming the variable, before it is sent
/// to the node.
#[test]
fn an_unresolved_reference_is_refused_before_it_reaches_the_node() {
    let s = site_with(true, "op://zagros/gamma-core-1/password");
    let (out, _) = join_node(&s, Some("2"));
    let all = says(&out);
    let argv = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!out.status.success(), "{all}");
    assert!(
        all.contains("GB-269") && all.contains("GAMMA_NODE_PASS"),
        "{all}"
    );
    assert!(
        !all.contains("op://zagros"),
        "the refusal never prints the reference: {all}"
    );
    assert!(
        !argv.contains("sshpass"),
        "nothing was sent with it:\n{argv}"
    );
    assert!(!argv.contains("op://"), "{argv}");
}

/// Gate row I3, the data router: a joining site's core node runs its own
/// Zenoh router, so the site's data fabric does not depend on reaching the
/// founder's. Phase 13 runs zenoh-deploy.yml for the site's core group,
/// limited to the node, after the Spock wiring and signal federation and
/// before phase 14.
#[test]
fn a_joiners_core_node_runs_its_data_router() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let router = plays
        .iter()
        .position(|l| l.contains("zenoh-deploy.yml"))
        .unwrap_or_else(|| panic!("no data router play ran for the joiner: {plays:#?}"));
    assert!(
        plays[router].contains("zenoh-deploy.yml -e target=gamma_hosts -l gamma-core-1"),
        "{}",
        plays[router]
    );
    let rewire = plays
        .iter()
        .rposition(|l| l.contains("spock-mesh-wire.yml --limit gamma-core-1"))
        .expect("the joiner's Spock wiring ran");
    assert!(
        rewire < router,
        "the router after the Spock wiring: {plays:#?}"
    );
    let acceptance = log
        .lines()
        .position(|l| l.starts_with("acceptance "))
        .unwrap();
    let router_line = log.lines().position(|l| l == plays[router]).unwrap();
    assert!(router_line < acceptance, "the router runs before phase 15");
}

/// Gate row I3, the certificates: a joining site's core node gets its router
/// and service leaves without a hand step. Phase 13 mints the site's leaves
/// from its manifest (mint-site-certs.sh, idempotent) and stages the service
/// leaves the manifest carries onto the node (stage-service-leaves.yml), both
/// before the data router, whose play reads the router leaf.
#[test]
fn a_joiners_core_node_mints_and_stages_its_leaves_before_the_router() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    let mint = lines
        .iter()
        .position(|l| *l == "mint gamma")
        .unwrap_or_else(|| panic!("the site's leaves were not minted:\n{log}"));
    let stage = lines
        .iter()
        .position(|l| l.contains("stage-service-leaves.yml"))
        .unwrap_or_else(|| panic!("no service leaves were staged:\n{log}"));
    let router = lines
        .iter()
        .position(|l| l.contains("zenoh-deploy.yml"))
        .unwrap();
    assert!(
        mint < stage && stage < router,
        "mint, stage, then the router:\n{log}"
    );
    let st = lines[stage];
    assert!(
        st.contains(
            "stage-service-leaves.yml -e target=gamma_hosts -l gamma-core-1 -e leaf_src_dir="
        ),
        "{st}"
    );
    assert!(
        st.contains("/.goat/state/gamma-leaves"),
        "the manifest's out_dir, else <state dir>/<site>-leaves: {st}"
    );
    assert!(
        st.contains(r#"{"leaf_only": ["fabric-health", "mesh-health"]}"#),
        "only the service leaves the manifest carries: {st}"
    );
}

/// Gate row I3, the health publishers: a joining site's core node runs the
/// publishers the site declares, as the end-state model reads its pin table
/// (gamma: fabric-health and mesh-health; time-health absent). Each one's
/// play runs for the site's core group, limited to the node, after the data
/// router; a publisher the site does not declare does not run.
#[test]
fn a_joiners_core_node_runs_the_health_publishers_the_site_declares() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let router = plays
        .iter()
        .position(|l| l.contains("zenoh-deploy.yml"))
        .unwrap();
    for p in ["fabric-health-deploy.yml", "mesh-health-deploy.yml"] {
        let at = plays
            .iter()
            .position(|l| l.contains(&format!("{p} -e target=gamma_hosts -l gamma-core-1")))
            .unwrap_or_else(|| panic!("{p} did not run for the joiner: {plays:#?}"));
        assert!(router < at, "{p} after the router: {plays:#?}");
    }
    assert!(
        !plays.iter().any(|l| l.contains("time-health-deploy.yml")),
        "a publisher the site does not declare does not run: {plays:#?}"
    );
}

/// Gate row I3, the front door: gamma declares signal federation off and no
/// ACME delivery, so it is a LAN site, and its core node gets the LAN routes
/// (lan-traefik-routes.yml) for the site's core group, limited to the node,
/// after the data router. No federation play sets up a door for it.
#[test]
fn a_joiners_core_node_gets_its_front_door_with_signal_federation_off() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let router = plays
        .iter()
        .position(|l| l.contains("zenoh-deploy.yml"))
        .unwrap();
    let door = plays
        .iter()
        .position(|l| {
            l.contains("lan-traefik-routes.yml -e target=gamma_hosts -e site_id=gamma -l gamma-core-1")
        })
        .unwrap_or_else(|| panic!("gamma's front door did not run: {plays:#?}"));
    assert!(router < door, "the door after the router: {plays:#?}");
    assert!(
        !plays.iter().any(|l| l.contains("traefik-acme.yml")),
        "a LAN site gets no ACME door: {plays:#?}"
    );
}

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

struct Site {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    checkout: PathBuf,
    log: PathBuf,
}

fn site() -> Site {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let checkout = root.join("checkout");
    let r = repo_root();
    copy_tree(
        &r.join("ops/goatbuilder/fixtures/join-node-third-site"),
        &checkout,
    );
    std::fs::remove_file(checkout.join("README.md")).unwrap();
    // The real gates.
    std::fs::create_dir_all(checkout.join("ops/scripts")).unwrap();
    for s in ["standup-config-preflight.py", "signal-federation-check.py"] {
        std::fs::copy(
            r.join("ops/scripts").join(s),
            checkout.join("ops/scripts").join(s),
        )
        .unwrap();
    }
    let log = root.join("argv.log");
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let (l, st) = (log.display(), state.display());
    // with-aws starts every play (`with-aws <profile> -- ansible-preflight.sh
    // <inventories> <play> ...`).
    executable(
        &checkout.join("ops/scripts/with-aws"),
        &format!(
            "#!/bin/sh\nprintf 'play %s\\n' \"$*\" >> '{l}'\n\
             case \"$*\" in\n\
             *ory-deploy.yml*) touch '{st}/ory-up' ;;\n\
             *bootstrap*) case \"$*\" in *'--skip-tags after-ory'*) ;; *)\n\
               [ -f '{st}/ory-up' ] || {{ echo 'TASK [check the management service is up] fatal: netbird-mgmt is not up: it waits for Hydra, which ory-deploy.yml has not started'; exit 2; }} ;; esac ;;\n\
             esac\nexit 0\n"
        ),
    );
    executable(
        &checkout.join("ops/scripts/ansible-preflight.sh"),
        "#!/bin/sh\nexit 0\n",
    );
    executable(
        &checkout.join("ops/scripts/seed-ssh-key.sh"),
        &format!("#!/bin/sh\nprintf 'seed %s\\n' \"$*\" >> '{l}'\n"),
    );
    // Phase 15's acceptance on the new node.
    executable(
        &checkout.join("ops/validation/generate-values.py"),
        &format!(
            "import sys\nopen('{l}', 'a').write('values ' + ' '.join(sys.argv[1:]) + '\\n')\n"
        ),
    );
    executable(&checkout.join("ops/validation/run-acceptance.py"), &format!("import sys\nopen('{l}', 'a').write('acceptance ' + ' '.join(sys.argv[1:]) + '\\n')\nprint('every applicable row PASS')\n"));
    // ssh reaches the node: it runs nothing there, and takes what is piped to it.
    executable(
        &root.join("bin/ssh"),
        &format!("#!/bin/sh\nprintf 'ssh %s\\n' \"$*\" >> '{l}'\nfor a; do [ \"$a\" = 'bash -s' ] && cat > /dev/null; done\nexit 0\n"),
    );
    Site {
        _tmp: tmp,
        root,
        checkout,
        log,
    }
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
        .env("OWNER_EMAIL", "owner@example.org")
        .env("GAMMA_NB_SETUP_KEY", "setup-key-stand-in")
        .env("ZAGROS_SIGNAL_FEDERATION_TOKEN", "token-stand-in")
        .env("GOAT_NO_FUN", "1")
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
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

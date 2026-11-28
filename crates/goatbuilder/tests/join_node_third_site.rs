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
    // The data router's play: a run is refused without it (GB-377). ansible is
    // a stand-in, so an empty one is enough.
    std::fs::write(checkout.join("ops/ansible/zenoh-deploy.yml"), "---\n").unwrap();
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
        // Phase 1: the node answers.
        "ops/scripts/join-node-reach.sh",
        "ops/secrets/posture.py",
        "ops/scripts/require-secret.sh",
        // Every health publisher's play: a play in the checkout is not a
        // reason to run it, the site's declaration is.
        "ops/ansible/fabric-health-deploy.yml",
        "ops/ansible/mesh-health-deploy.yml",
        "ops/ansible/presence-health-deploy.yml",
        "ops/ansible/time-health-deploy.yml",
        "ops/ansible/consistency-health-deploy.yml",
        "ops/ansible/fabric-crawler-deploy.yml",
        // The core type's site services (phase 13).
        "ops/ansible/decoy-deploy.yml",
        "ops/ansible/deadman-watch-deploy.yml",
        "ops/ansible/traefik-compression-metrics-deploy.yml",
        "ops/ansible/budget-notify-deploy.yml",
        "ops/ansible/names-collector-deploy.yml",
        "ops/ansible/oci-substrate-deploy.yml",
        "ops/ansible/host-net-publish-deploy.yml",
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
             *spock-seed-current-check.yml*) [ -f '{st}/seed-stale' ] && {{ echo 'GB-367: the founder changed identities after the seed (identities: founder 2 rows, joiner 1); re-seed before wiring Spock'; exit 2; }} ;;\n\
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
    // join-site-secrets.py takes the joiner's shared secrets from its
    // founder in phase 3; the stand-in records it.
    executable(
        &checkout.join("ops/scripts/join-site-secrets.py"),
        &format!("import sys\nopen('{l}', 'a').write('secrets ' + ' '.join(sys.argv[1:]) + '\\n')\n"),
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
             if [ -f '{st}/unreachable' ]; then echo 'ssh: connect to host 192.0.2.30 port 22: Connection timed out' >&2; exit 255; fi\n\
             if [ -f '{st}/silent' ]; then exit 255; fi\n\
             if [ -f '{st}/lan-down' ]; then case \"$*\" in *192.0.2.30*) echo 'ssh: connect to host 192.0.2.30 port 22: No route to host' >&2; exit 255 ;; esac; fi\n\
             if [ -f '{st}/loses-stderr' ]; then for a in \"$@\"; do case \"$a\" in UserKnownHostsFile=*) d=\"${{a#UserKnownHostsFile=}}\"; rm -f \"${{d%/*}}/stderr\" ;; esac; done; exit 255; fi\n\
             if [ -f '{st}/refusing-jump' ]; then echo 'goat@jump.example.org: Permission denied (publickey).' >&2; echo 'Connection closed by UNKNOWN port 65535' >&2; exit 255; fi\n\
             kh=; for a in \"$@\"; do case \"$a\" in UserKnownHostsFile=*) kh=\"${{a#UserKnownHostsFile=}}\" ;; esac; done\n\
             [ -z \"$kh\" ] || echo '192.0.2.30 ssh-ed25519 stand-in-host-key' >> \"$kh\"\n\
             case \"$*\" in *BatchMode=yes*)\n\
               if [ -f '{st}/password-only' ]; then\n\
                 case \"$*\" in *'sudo -n true'*) [ -f '{st}/sudo-seeded' ] || exit 1 ;; *) [ -f '{st}/key-seeded' ] || {{ echo 'goat@192.0.2.30: Permission denied (publickey,password).' >&2; exit 255; }} ;; esac\n\
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
    join_node_with(s, from, &[])
}

fn join_node_with(s: &Site, from: Option<&str>, extra: &[&str]) -> (Output, serde_json::Value) {
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
        extra
            .iter()
            .position(|a| *a == "--address")
            .and_then(|i| extra.get(i + 1))
            .map(|a| a.to_string())
            .unwrap_or_else(|| "192.0.2.30".into()),
    ];
    if let Some(n) = from {
        args.extend(["--from".to_string(), n.to_string()]);
    }
    let given = extra.iter().position(|a| *a == "--address");
    args.extend(
        extra
            .iter()
            .enumerate()
            .filter(|(i, _)| given.is_none_or(|g| *i != g && *i != g + 1))
            .map(|(_, a)| a.to_string()),
    );
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
        .env("GAMMA_NB_SETUP_KEY", SETUP_KEY)
        .env("GAMMA_NODE_PASS", &s.node_pass)
        .env("GAMMA_PG_PASS", PG_PASS)
        .env("ZAGROS_SIGNAL_FEDERATION_TOKEN", "token-stand-in")
        .env("ALPHA_NB_API_TOKEN", "api-token-stand-in")
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
    // One run, with no --from: phase 1 checks the node answers and goes on.
    let (out, v) = join_node(&s, None);
    assert!(
        out.status.success(),
        "join-node stopped after phases {:?}:\n{}\nplays:\n{}",
        phases(&v),
        says(&out),
        std::fs::read_to_string(&s.log).unwrap_or_default()
    );
    assert_eq!(resume_at(&v), None, "a stop for a person: {}", says(&out));
    let ran = phases(&v);
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

/// Gate row H2: with no operator step between phase 1 and phase 15. Phase 1
/// checks the node answers; the run goes on to 15 in one go,
/// enrolling the node from a key minted where it is used and recording its
/// mesh address itself, and no secret is in any argument list.
/// A joiner's account arrives from the founder (seed-from-peer, then Spock),
/// and site-bootstrap mints an account only on a founder, so phase 13 names no
/// bootstrap owner. An unattended run with no OWNER_EMAIL in its environment
/// (the helper sets none) goes through every bootstrap pass.
#[test]
fn a_joiner_passes_no_bootstrap_owner_and_needs_no_owner_email() {
    let s = site();
    let (out, v) = join_node(&s, None);
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(out.status.success(), "{}\nargv:\n{log}", says(&out));
    assert_eq!(phases(&v).last(), Some(&15), "{}", says(&out));
    let bootstraps: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("play ") && l.contains("bootstrap"))
        .collect();
    assert!(!bootstraps.is_empty(), "phase 13 ran the bootstrap passes:\n{log}");
    assert!(
        !log.contains("bootstrap_owner_email") && !log.contains("OWNER_EMAIL"),
        "a joiner passes no bootstrap owner:\n{log}"
    );
}

/// From phase 12 the node has a mesh address, and the plays reach it there
/// (the mesh inventory phase 10 filled in). Phase 14's own ssh does too: with
/// the node's LAN address unreachable after phase 12, phases 13 to 15 still
/// pass, and no ssh after phase 12 names the LAN address.
#[test]
fn after_phase_12_the_node_is_reached_at_its_mesh_address() {
    let s = site();
    let (out, v) = join_node_with(&s, None, &["--to", "12"]);
    assert!(out.status.success(), "phases 1 to 12: {}", says(&out));
    assert_eq!(phases(&v).last(), Some(&12), "{}", says(&out));
    let before = std::fs::read_to_string(&s.log).unwrap_or_default().lines().count();
    // The LAN path is gone: only the mesh reaches the node now.
    std::fs::write(s.root.join("state/lan-down"), "").unwrap();
    let (out, v) = join_node(&s, Some("13"));
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    let after: Vec<&str> = log.lines().skip(before).collect();
    assert!(
        out.status.success(),
        "with the LAN address unreachable, phases 13 to 15 stopped after {:?}:\n{}\nargv:\n{}",
        phases(&v),
        says(&out),
        after.join("\n")
    );
    assert_eq!(phases(&v).last(), Some(&15), "{}", says(&out));
    let ssh: Vec<&&str> = after.iter().filter(|l| l.starts_with("ssh ")).collect();
    assert!(
        ssh.iter().any(|l| l.contains("100.127.0.3")),
        "phase 14 reaches the node at its mesh address: {ssh:#?}"
    );
    assert!(
        !after.iter().any(|l| l.contains("192.0.2.30")),
        "nothing after phase 12 names the LAN address:\n{}",
        after.join("\n")
    );
}

/// Row 9: when the founder changed a seed table after the seed, phase 13
/// stops at its check before pass 3, with GB-367, and wires no Spock
/// subscription: the run never leaves a joiner subscribed over a seed that
/// lacks the founder's later writes.
#[test]
fn a_seed_the_founder_changed_since_stops_phase_13_before_spock_is_wired() {
    let s = site();
    std::fs::write(s.root.join("state/seed-stale"), "").unwrap();
    let (out, v) = join_node(&s, None);
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!out.status.success(), "{}\nargv:\n{log}", says(&out));
    assert_eq!(phases(&v).last(), Some(&13), "{}", says(&out));
    assert!(says(&out).contains("GB-367"), "{}", says(&out));
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let check = plays.iter().position(|l| l.contains("spock-seed-current-check.yml")).expect("the check ran");
    assert!(plays[..check].iter().any(|l| l.contains("seed-from-peer.yml")), "the seed ran first: {plays:#?}");
    assert_eq!(check + 1, plays.len(), "nothing ran after the check: {plays:#?}");
}

/// A joining site's own copy of the seed play that no longer matches the
/// generic play it was derived from is refused at phase 13's first step
/// (GB-362), before any phase-13 play changes the node: the real check script
/// runs against the fixture's copy.
#[test]
fn a_stale_site_copy_of_the_seed_play_stops_phase_13_before_any_play() {
    let s = site();
    let r = repo_root();
    std::fs::copy(r.join("ops/scripts/check-site-play-current.py"), s.checkout.join("ops/scripts/check-site-play-current.py")).unwrap();
    std::fs::copy(r.join("ops/ansible/site-seed-from-peer.yml"), s.checkout.join("ops/ansible/site-seed-from-peer.yml")).unwrap();
    // Derived from a generic play that has changed since: its digest is not the current one.
    std::fs::write(
        s.checkout.join("ops/ansible/gamma-seed-from-peer.yml"),
        format!("# derived-from: site-seed-from-peer.yml sha256:{}\n---\n", "0".repeat(64)),
    )
    .unwrap();
    let (out, v) = join_node(&s, None);
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!out.status.success(), "{}\nargv:\n{log}", says(&out));
    assert_eq!(phases(&v).last(), Some(&13), "{}", says(&out));
    assert!(says(&out).contains("GB-362"), "{}", says(&out));
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    assert!(
        !plays.iter().any(|l| l.contains("participates_in_spock") || l.contains("seed-from-peer.yml") || l.contains("ory-deploy.yml")),
        "no phase-13 play ran: {plays:#?}"
    );
}

/// Phase 15 runs the acceptance rows against the site's derived end state,
/// as standup's acceptance gate does: a row whose services the site declares
/// absent reads N/A, not FAIL, so an unattended run is not stopped by a row
/// that does not apply (ADR 1206).
#[test]
fn phase_15_runs_the_acceptance_rows_against_the_sites_end_state() {
    let s = site();
    let (out, v) = join_node(&s, None);
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(out.status.success(), "{}\nargv:\n{log}", says(&out));
    assert_eq!(phases(&v).last(), Some(&15), "{}", says(&out));
    let run = log
        .lines()
        .find(|l| l.starts_with("acceptance "))
        .unwrap_or_else(|| panic!("phase 15 ran the acceptance rows:\n{log}"));
    let words: Vec<&str> = run.split(' ').collect();
    let at = words
        .iter()
        .position(|w| *w == "--endstate")
        .unwrap_or_else(|| panic!("--endstate: {run}"));
    let model = std::fs::read_to_string(words[at + 1]).expect("the end-state model was written");
    let model: serde_json::Value = serde_json::from_str(&model).expect("the model is JSON");
    assert!(model["services"].is_object(), "{model}");
    assert!(words.contains(&"--all-eligible"), "{run}");
    the_joiners_gaps_are_the_admin_surfaces("gamma", &log);
}

#[test]
fn from_phase_1_a_node_joins_with_no_operator_step_and_no_secret_in_any_argv() {
    let s = site();
    let (out, v) = join_node(&s, None);
    let printed = says(&out);
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
    assert_eq!(seen, (1..=15).collect::<Vec<u64>>(), "{}", says(&out));
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
        .find(|r| r["args"].as_array().is_some_and(|a| a.iter().any(|x| x == "join-node")))
        .expect("a record of the run");
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
        plays[router].contains("zenoh-deploy.yml -e target=gamma_hosts -e zenoh_slot_fragment_bootstrap_if_unassigned=true -l gamma-core-1"),
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

/// Gate row I3: a joining site's core node runs its own mesh DNS replica, so
/// a site cut off from the founder still resolves the goatnet's names
/// (docs/adr/1235-per-site-mesh-dns-nameserver-groups.md). Phase 13 runs
/// goat-coredns.yml on the node after Ory, with the site's own nameserver
/// group (`gamma-mesh-dns`) and the goatnet's zone, and registers the group
/// through the founder's management API, before phase 14.
#[test]
fn a_joiner_core_node_runs_its_mesh_dns_replica() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let plays: Vec<&str> = log.lines().filter(|l| l.starts_with("play ")).collect();
    let dns = plays
        .iter()
        .position(|l| l.contains("goat-coredns.yml"))
        .unwrap_or_else(|| panic!("no mesh DNS play ran for the joiner: {plays:#?}"));
    let ory = plays
        .iter()
        .position(|l| l.contains("ory-deploy.yml"))
        .unwrap();
    assert!(ory < dns, "mesh DNS after Ory: {plays:#?}");
    let p = plays[dns];
    for want in [
        " -l gamma-core-1 ",
        "-e mesh_dns_zone_dir_name=alpha ",
        "-e netbird_nameserver_group_name=gamma-mesh-dns ",
        "-e netbird_api=https://zagros.example.org/api ",
        "-e netbird_api_token_envvar=ALPHA_NB_API_TOKEN",
    ] {
        assert!(p.contains(want), "{want:?} in {p}");
    }
    let acceptance = log
        .lines()
        .position(|l| l.starts_with("acceptance "))
        .unwrap();
    let dns_line = log.lines().position(|l| l == p).unwrap();
    assert!(dns_line < acceptance, "mesh DNS runs before phase 15");
}

/// The node's timings file: one row per step a run executed, with the columns
/// standup's file has (time written, phase, label, seconds, exit code, cause).
fn timing_rows(s: &Site) -> Option<Vec<Vec<String>>> {
    let f = s
        .root
        .join(".goat/state/join-node-gamma-gamma-core-1-timings.tsv");
    let text = std::fs::read_to_string(f).ok()?;
    Some(
        text.lines()
            .map(|l| l.split('\t').map(String::from).collect())
            .collect(),
    )
}

/// The plan's last line describes the run as it is: phase 1 checks that the
/// node answers and the run goes on to phase 15 with no operator step (H2).
/// It never calls phase 1 a manual step or sends a person to `--from 2`.
#[test]
fn a_dry_runs_footer_does_not_call_phase_1_a_manual_step() {
    let s = site();
    let (out, v) = join_node_with(&s, None, &["--dry-run"]);
    assert!(out.status.success(), "{}", says(&out));
    let footer = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "footer")
        .and_then(|c| c["detail"].as_str())
        .unwrap_or_else(|| panic!("a footer row: {}", says(&out)))
        .to_string();
    assert!(footer.starts_with("PLAN complete."), "{footer}");
    assert!(
        !footer.contains("manual"),
        "phase 1 is a check, not a manual step: {footer}"
    );
    assert!(
        !footer.contains("--from 2"),
        "a run does not pause after phase 1: {footer}"
    );
    assert!(
        footer.contains("GB-314") && footer.contains("--from 1"),
        "names the stop and how to resume: {footer}"
    );
}

/// The roles a joiner's core node runs, as the repository ships them: the data
/// router's (goat-zenoh) and every health publisher's defaults, which say how
/// each reads `zenoh_stack`.
fn with_role_defaults(s: &Site) {
    for role in [
        "goat-zenoh",
        "fabric-health",
        "mesh-health",
        "presence-health",
        "time-health",
        "consistency-health",
        "fabric-crawler",
    ] {
        let rel = format!("ops/ansible/roles/{role}/defaults");
        copy_tree(&repo_root().join(&rel), &s.checkout.join(&rel));
    }
}

/// The data router names its stack `<site_id>-zenoh` by its role default, and
/// a health publisher whose role defaults `zenoh_stack` to `goat-zenoh` would
/// look for a router and network of that name. A joiner whose group_vars do
/// not set `zenoh_stack` is refused before any phase runs (GB-361), naming the
/// publishers, both values and the line to add; set, every role reads it.
#[test]
fn a_joiner_whose_roles_disagree_on_zenoh_stack_is_refused_before_any_phase() {
    let s = site();
    with_role_defaults(&s);
    let (out, v) = join_node_with(&s, None, &["--dry-run"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(phases(&v).is_empty(), "no phase runs: {all}");
    assert!(all.contains("GB-361"), "{all}");
    assert!(all.contains("gamma-zenoh") && all.contains("goat-zenoh"), "both values: {all}");
    assert!(all.contains("zenoh_stack: gamma-zenoh"), "the line to add: {all}");
    assert!(all.contains("group_vars/gamma_hosts"), "where to add it: {all}");

    let vars = s.checkout.join("ops/ansible/group_vars/gamma_hosts/vars.yml");
    let mut text = std::fs::read_to_string(&vars).unwrap();
    text.push_str("zenoh_stack: gamma-zenoh\n");
    std::fs::write(&vars, text).unwrap();
    let (out, _) = join_node_with(&s, None, &["--dry-run"]);
    assert!(out.status.success(), "with zenoh_stack set, every role reads one value: {}", says(&out));
}

/// Like standup, a join-node run leaves a row per executed step in the state
/// directory, and a timing line in its summary; a dry run leaves no file.
#[test]
fn a_join_node_run_records_each_step_it_ran_and_a_dry_run_records_none() {
    let s = site();
    let (out, _) = join_node_with(&s, Some("2"), &["--dry-run"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(timing_rows(&s).is_none(), "a dry run writes no timings file");

    let (out, v) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let rows = timing_rows(&s).expect("a timings file in the state directory");
    let ran = phases(&v);
    assert_eq!(rows.len(), ran.len(), "one row per step run: {rows:?}");
    for (row, n) in rows.iter().zip(&ran) {
        assert_eq!(row.len(), 6, "{row:?}");
        assert_eq!(row[1], n.to_string(), "{row:?}");
        assert!(!row[2].is_empty() && row[3].parse::<u64>().is_ok(), "{row:?}");
        assert_eq!((row[4].as_str(), row[5].as_str()), ("0", "ok"), "{row:?}");
    }
    let checks = v["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "timings" && c["detail"].as_str().unwrap().starts_with("total ")),
        "{}",
        says(&out)
    );
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "time" && c["detail"].as_str().unwrap().starts_with("phase 2 took ")),
        "{}",
        says(&out)
    );
}

/// A failed step is recorded with its exit code, and nothing after it is.
#[test]
fn a_failed_join_node_step_is_recorded_with_its_exit_code() {
    let s = site_with(true, "op://zagros/gamma-core-1/password");
    let (out, _) = join_node(&s, Some("2"));
    assert!(!out.status.success(), "{}", says(&out));
    let rows = timing_rows(&s).expect("a timings file in the state directory");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0][1], "2", "{rows:?}");
    assert_ne!(rows[0][4], "0", "{rows:?}");
}


/// Phase 1 checks the node answers over SSH before anything is done on it
/// (GB-314). A node that does not answer stops the run at phase 1, with the
/// instruction for a person and `--from 1`, and nothing after it runs. A
/// node that answers but refuses the login (no key yet: phase 2 seeds it)
/// has answered: phase 1 cannot prove a login before phase 2.
#[test]
fn a_node_that_does_not_answer_stops_the_run_at_phase_1() {
    let s = site();
    std::fs::write(s.root.join("state/unreachable"), "").unwrap();
    let (out, v) = join_node(&s, None);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert_eq!(phases(&v), vec![1], "{all}");
    assert!(all.contains("GB-314") && all.contains("Connection timed out"), "{all}");
    assert!(all.contains("--address"), "the instruction names --address: {all}");
    assert!(all.contains("--from 1"), "{all}");
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!log.lines().any(|l| l.starts_with("play ") || l.starts_with("sshpass")), "something ran past phase 1:\n{log}");
}

/// When ssh fails and prints nothing, GB-314 says which case it was: ssh
/// printed nothing, or the file its stderr went to was gone before phase 1
/// read it. Either way the run stops at phase 1 and nothing runs after it.
#[test]
fn an_ssh_failure_with_no_words_names_its_case_at_phase_1() {
    for (flag, words) in [
        ("silent", "ssh exited 255 and printed nothing on stderr"),
        ("loses-stderr", "was gone before it was read"),
    ] {
        let s = site();
        std::fs::write(s.root.join("state").join(flag), "").unwrap();
        let (out, v) = join_node(&s, None);
        let all = says(&out);
        assert!(!out.status.success(), "{flag}: {all}");
        assert_eq!(phases(&v), vec![1], "{flag}: {all}");
        assert!(all.contains("GB-314") && all.contains(words), "{flag}: {all}");
        let log = std::fs::read_to_string(&s.log).unwrap_or_default();
        assert!(!log.lines().any(|l| l.starts_with("play ") || l.starts_with("sshpass")), "{flag}: something ran past phase 1:\n{log}");
    }
}

/// A jump host's "Permission denied" is not the node answering: the run
/// stops at phase 1. ssh says only that the jump host refused and the
/// connection closed; the node's own host key never arrived.
#[test]
fn a_jump_host_that_refuses_is_not_the_node_answering() {
    let s = site();
    std::fs::write(s.root.join("state/refusing-jump"), "").unwrap();
    let (out, v) = join_node(&s, None);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert_eq!(phases(&v), vec![1], "{all}");
    assert!(all.contains("GB-314"), "{all}");
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!log.lines().any(|l| l.starts_with("play ") || l.starts_with("sshpass")), "something ran past phase 1:\n{log}");
}

/// A Jetson's USB device-mode bridge address, 192.168.55.0/24 (l4tbr0), runs
/// sshd on the box itself and finishes a key exchange, but it is reachable
/// only over a direct USB-C cable, never from the site: phase 1 refuses it
/// before any connection, with GB-314 and the way to the real LAN address.
#[test]
fn the_jetson_usb_bridge_address_is_refused_at_phase_1() {
    for address in ["192.168.55.1", "192.168.55.100"] {
        let s = site();
        let (out, v) = join_node_with(&s, None, &["--address", address]);
        let all = says(&out);
        assert!(!out.status.success(), "{address}: {all}");
        assert_eq!(phases(&v), vec![1], "{address}: {all}");
        assert!(all.contains("GB-314") && all.contains("l4tbr0") && all.contains("--address"), "{address}: {all}");
        let log = std::fs::read_to_string(&s.log).unwrap_or_default();
        assert!(!log.lines().any(|l| l.starts_with("ssh ") && !l.starts_with("ssh -G")), "{address}: phase 1 connected:\n{log}");
        assert!(!log.lines().any(|l| l.starts_with("play ") || l.starts_with("sshpass")), "{address}: something ran past phase 1:\n{log}");
    }
    // An address beside it is not the bridge.
    let s = site();
    let (out, v) = join_node_with(&s, None, &["--address", "192.168.56.1"]);
    assert!(phases(&v).contains(&2), "{}", says(&out));
}

/// A node that takes only its first-login password answers, though it
/// refuses a key login before phase 2: the run goes on past phase 1.
#[test]
fn a_password_only_node_answers_at_phase_1() {
    let s = site_with(true, NODE_PASS);
    let (out, v) = join_node(&s, None);
    assert!(out.status.success(), "{}", says(&out));
    assert!(phases(&v).contains(&2), "{}", says(&out));
}

/// The joiner's end state, as phase 15 hands it to the rows: the four admin
/// surfaces are declared and not yet built at a second site (gate rows I1,
/// I4), nor is the site's own SPIRE server and agent (ADR 1164), and the
/// real runner's plan against that model reads their rows as gaps (SKIPPED
/// in a run, with the gap's words) and exits 0.
fn the_joiners_gaps_are_the_admin_surfaces(env: &str, log: &str) {
    let run = log
        .lines()
        .find(|l| l.starts_with("acceptance "))
        .unwrap_or_else(|| panic!("phase 15 ran no acceptance:\n{log}"));
    let words: Vec<&str> = run.split(' ').collect();
    let at = words.iter().position(|w| *w == "--endstate").unwrap_or_else(|| panic!("{run}"));
    let model = words[at + 1];
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(model).unwrap()).unwrap();
    let gaps: std::collections::BTreeSet<&str> = doc["services"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, s)| !s["gap"].is_null())
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(
        gaps,
        ["id-admin-console", "netbird-dashboard", "onboard-runner", "portal", "spire", "spire-agent"]
            .into_iter()
            .collect(),
        "{doc}"
    );
    let yaml = std::process::Command::new("python3").args(["-c", "import yaml"]).status().map(|s| s.success()).unwrap_or(false);
    if !yaml {
        eprintln!("skipped: the acceptance runner needs python3 with PyYAML");
        return;
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let values = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(values.path(), "{}\n").unwrap();
    let out = std::process::Command::new("python3")
        .arg(repo.join("ops/validation/run-acceptance.py"))
        .args(["--env", env, "--values"])
        .arg(values.path())
        .args(["--endstate", model, "--plan"])
        .output()
        .unwrap();
    let plan = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{plan}{}", String::from_utf8_lossy(&out.stderr));
    let line = plan
        .lines()
        .find(|l| l.contains("gaps here, declared and not yet built at this site:"))
        .unwrap_or_else(|| panic!("no gaps in the plan:\n{plan}"));
    for row in ["portal-", "onboard-runner-", "console-audit-path-is-a-file"] {
        assert!(line.contains(row), "{row}: {line}");
    }
}

/// Gate row I3, the declared set on the fixture: every service the end-state
/// model marks present on gamma's core node is either built by a play the
/// run ran, or named under the run's not-run heading. The admin surfaces are
/// the catalog's gaps (gate row I1, I4); gamma is a single-host site, so its
/// obs stack is declared on the core node, where no verb builds it yet
/// (`obs.yml` by hand), and each of the four is named with gate row I3. No
/// service is left out in silence.
#[test]
fn every_service_gamma_declares_is_built_or_named() {
    let s = site();
    let (out, v) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let model = Command::new("python3")
        .arg(s.checkout.join("ops/goatbuilder/endstate/endstate.py"))
        .args(["model", "gamma", "--root"])
        .arg(&s.checkout)
        .output()
        .unwrap();
    assert!(model.status.success(), "{}", says(&model));
    let model: serde_json::Value = serde_json::from_slice(&model.stdout).unwrap();
    let declared: Vec<&str> = model["services"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, sv)| {
            sv["present"] == true
                && sv["hosts"]
                    .as_array()
                    .is_some_and(|h| h.iter().any(|n| n == "gamma-core-1"))
        })
        .map(|(n, _)| n.as_str())
        .collect();
    let named: Vec<&str> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "not-run")
        .filter_map(|c| c["detail"].as_str())
        .collect();
    let log = std::fs::read_to_string(&s.log).unwrap();
    let ran = |play: &str| {
        log.lines()
            .any(|l| l.starts_with("play ") && l.contains(play) && l.contains("gamma-core-1"))
    };
    // What the run builds of the declared set, by the play that builds it.
    let built = [
        ("traefik", "lan-traefik-routes.yml"),
        ("netbird-mgmt", "bootstrap"),
        ("signal-server", "bootstrap"),
        ("pgbackrest", "bootstrap"),
        ("ory", "ory-deploy.yml"),
        ("kratos-selfservice-ui", "ory-deploy.yml"),
        ("mesh-coredns", "goat-coredns.yml"),
        ("fabric-health", "fabric-health-deploy.yml"),
        ("mesh-health", "mesh-health-deploy.yml"),
        // Declared at every site since the catalog says so (#968).
        ("zenoh-router", "zenoh-deploy.yml"),
        ("admit-audit", "bootstrap"),
    ];
    let line = |svc: &str| {
        named
            .iter()
            .find(|d| d.starts_with(&format!("{svc} \u{2014} ")))
            .copied()
    };
    for svc in &declared {
        match (built.iter().find(|(b, _)| b == svc), line(svc)) {
            (Some((_, play)), None) => assert!(ran(play), "{svc}: {play} did not run\n{log}"),
            (None, Some(_)) => {}
            (Some(_), Some(d)) => panic!("{svc} is built and named: {d}"),
            (None, None) => {
                panic!("{svc} is declared at gamma-core-1, not built and not named: {named:#?}")
            }
        }
    }
    for svc in [
        "netbird-dashboard",
        "portal",
        "id-admin-console",
        "onboard-runner",
    ] {
        let d = line(svc).unwrap_or_else(|| panic!("{svc} not named: {named:#?}"));
        assert!(d.ends_with("(gate row I1, I4)"), "{d}");
    }
    let obs = ["prometheus", "alertmanager", "grafana", "obs-exporters"];
    for svc in obs {
        assert_eq!(
            line(svc),
            Some(format!("{svc} \u{2014} declared at gamma-core-1, and no join-node step builds it yet (gate row I3)").as_str()),
            "{named:#?}"
        );
    }
    assert!(
        !named.iter().any(|d| d.ends_with("(gate row I3)")
            && !obs.iter().any(|o| d.starts_with(&format!("{o} \u{2014} ")))),
        "gamma declares nothing else no step builds: {named:#?}"
    );
    assert!(
        !named.iter().any(|d| d.contains("--class obs")),
        "a single-host site is never sent to an obs run: {named:#?}"
    );
}

/// Gate row I3, a joining site's secrets: phase 3 runs join-site-secrets.py
/// for gamma with its founder alpha, once, before phase 5's baseline play
/// changes the node.
#[test]
fn phase_3_takes_gammas_shared_secrets_from_alpha_before_the_node_changes() {
    let s = site();
    let (out, _) = join_node(&s, Some("2"));
    assert!(out.status.success(), "{}", says(&out));
    let log = std::fs::read_to_string(&s.log).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    let secrets: Vec<usize> = (0..lines.len()).filter(|&i| lines[i] == "secrets gamma --founder alpha").collect();
    assert_eq!(secrets.len(), 1, "{log}");
    let baseline = lines.iter().position(|l| l.starts_with("play ") && l.contains("mgmt-host-baseline.yml")).expect("phase 5 ran");
    assert!(secrets[0] < baseline, "{log}");
}

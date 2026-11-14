//! `goatbuilder join-node --dry-run` names what the joining site does not run
//! yet (docs/design/goatbuilder.md, `join-node` part (d)): under its own
//! heading, each gap the end-state model (`endstate.py model <site>`) names,
//! with the gate row that closes it, and each service the site turns off, with
//! its flag. Every site is declared to run every service (ADR 1165, gate group
//! I), so a service a joining site does not run yet is a gap, never a design.
//! A checkout whose model cannot be read says so in one line and lists nothing.
//!
//! The checkout is the standup fixture's declaration (`ops/goatbuilder/
//! fixtures/standup`) with its site `alpha` declared as a joiner, and this
//! repository's end-state model and catalog linked in, so the reasons are the
//! real model's. Nothing reaches a host.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The model needs Python with PyYAML, as `goatbuilder endstate` does.
fn python_with_yaml() -> bool {
    Command::new("python3")
        .args(["-c", "import yaml"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The standup fixture's declaration, with `alpha` a joiner of `zagros` that
/// turns isitup and the id console off. `with_model` links in this
/// repository's end-state model (script and catalog).
fn checkout(with_model: bool) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    let fixture = repo().join("ops/goatbuilder/fixtures/standup");
    std::fs::create_dir_all(r.join("ops/goatbuilder/declared")).unwrap();
    std::fs::copy(
        fixture.join("goatbuilder.yaml"),
        r.join("ops/goatbuilder/goatbuilder.yaml"),
    )
    .unwrap();
    std::fs::copy(
        fixture.join("converger.yaml"),
        r.join("ops/goatbuilder/converger.yaml"),
    )
    .unwrap();
    std::fs::copy(
        fixture.join("declared/pins.yaml"),
        r.join("ops/goatbuilder/declared/pins.yaml"),
    )
    .unwrap();
    let gv = r.join("ops/ansible/group_vars/alpha_hosts");
    std::fs::create_dir_all(&gv).unwrap();
    std::fs::write(
        gv.join("vars.yml"),
        "goatnet_founder: false\njoin_founder_site: zagros\nsite_enrollment_dns: netbird.example.org\nisitup_enabled: false\nid_admin_console_enabled: false\n",
    )
    .unwrap();
    // The site's leaf-cert manifest, which join-node's certificate steps read.
    std::fs::create_dir_all(r.join("ops/enrollment/manifests")).unwrap();
    std::fs::write(
        r.join("ops/enrollment/manifests/alpha-certs.yaml"),
        "site_id: alpha\nleaves:\n  - name: router\n    cn: goat-zenoh-router\n",
    )
    .unwrap();
    if with_model {
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            repo().join("ops/goatbuilder/endstate"),
            r.join("ops/goatbuilder/endstate"),
        )
        .unwrap();
    }
    t
}

fn dry_run(r: &Path, json: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    if json {
        c.args(["-o", "json"]);
    }
    c.args(["join-node", "-C"])
        .arg(r)
        .args([
            "dev",
            "--site",
            "alpha",
            "--node",
            "node-x",
            "--class",
            "core",
            "--dry-run",
        ])
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The gap text the catalog gives an admin surface at a joining site, and mesh DNS.
const ADMIN_GAP: &str = "declared, not yet built at this site: an admin surface cannot be turned on at a second site before its store is at the multi-writer schema and its write class is switched on there (gate row I1, I4)";
const MESH_DNS_GAP: &str = "declared, not yet built at this site: join-node does not deploy mesh DNS yet, and a site cut off from the others needs its own resolver (gate row I3)";

#[test]
fn a_joiners_dry_run_lists_what_it_does_not_run_with_the_models_reasons() {
    if !python_with_yaml() {
        eprintln!("skipped: the end-state model needs python3 with PyYAML (pip install pyyaml)");
        return;
    }
    let t = checkout(true);
    let o = dry_run(t.path(), true);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value =
        serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}\n{}", text(&o)));
    let rows: Vec<(String, String)> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["detail"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let heading = rows
        .iter()
        .position(|(n, _)| n == "not-run-heading")
        .unwrap_or_else(|| panic!("no heading for what the site does not run: {rows:#?}"));
    assert!(
        rows[heading].1.contains("does not run yet")
            && rows[heading].1.contains("gate row")
            && rows[heading].1.contains("endstate.py model alpha"),
        "{rows:#?}"
    );
    // After the phases, before the footer.
    let last_phase = rows.iter().rposition(|(n, _)| n == "phase").unwrap();
    assert!(heading > last_phase, "{rows:#?}");
    let listed: Vec<&str> = rows[heading + 1..]
        .iter()
        .filter(|(n, _)| n == "not-run")
        .map(|(_, d)| d.as_str())
        .collect();
    let line = |svc: &str| {
        *listed
            .iter()
            .find(|d| d.starts_with(&format!("{svc} \u{2014} ")))
            .unwrap_or_else(|| panic!("{svc} not listed: {listed:#?}"))
    };
    // Each admin surface, and mesh DNS, is a named gap with its gate row.
    assert_eq!(line("netbird-dashboard"), format!("netbird-dashboard \u{2014} {ADMIN_GAP}"));
    assert_eq!(line("id-admin-console"), format!("id-admin-console \u{2014} {ADMIN_GAP}"));
    assert_eq!(line("mesh-coredns"), format!("mesh-coredns \u{2014} {MESH_DNS_GAP}"));
    // Nothing reads as a design to leave a service out.
    for d in &listed {
        for wrong in ["founder-only", "single-homed", "by design", "\u{a7}D3"] {
            assert!(!d.contains(wrong), "{d}");
        }
    }
    // A service the site turns off reads its flag.
    assert_eq!(
        line("isitup"),
        "isitup \u{2014} `isitup_enabled` is false in group_vars"
    );
    // A service the model marks present is not listed.
    assert!(
        !listed.iter().any(|d| d.starts_with("postgres-spock ")),
        "{listed:#?}"
    );

    // The plain text carries the same heading and lines.
    let page = text(&dry_run(t.path(), false));
    assert!(page.contains("does not run yet"), "{page}");
    assert!(
        page.contains("isitup \u{2014} `isitup_enabled` is false in group_vars"),
        "{page}"
    );
}

#[test]
fn a_checkout_without_the_model_says_so_and_lists_nothing() {
    let t = checkout(false);
    let o = dry_run(t.path(), true);
    assert!(
        o.status.success(),
        "a plan still plans without the model: {}",
        text(&o)
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let rows: Vec<&serde_json::Value> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "not-run")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    let d = rows[0]["detail"].as_str().unwrap();
    assert!(
        d.starts_with("not read: ") && d.contains("ops/goatbuilder/endstate/endstate.py"),
        "{d}"
    );
}

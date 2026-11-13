//! `goatbuilder join-node --dry-run` names what the joining site does not run
//! (docs/design/goatbuilder.md, `join-node` part (d)): under its own heading,
//! every catalog service the end-state model (`endstate.py model <site>`) marks
//! absent here, each with the model's reason. An admin surface a joiner leaves
//! to the founder reads as single-homed under ADR 1084 §D3; a service the site
//! turns off reads its flag. A checkout whose model cannot be read says so in
//! one line and lists nothing.
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

const D3: &str = "single-homed at the founder (ADR 1084 \u{a7}D3); every site once ADR 1165 lifts it for this class";

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
        rows[heading].1.contains("does not run")
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
    // A founder-only admin surface, and one the site's flag keeps off.
    assert_eq!(line("netbird-dashboard"), format!("netbird-dashboard \u{2014} {D3} (declared by: founder-only service; this site joins a goatnet (goatnet_founder: false))"));
    assert_eq!(line("id-admin-console"), format!("id-admin-console \u{2014} {D3} (declared by: `id_admin_console_enabled` is false in group_vars)"));
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
    assert!(page.contains("does not run"), "{page}");
    assert!(
        page.contains(&format!(
            "isitup \u{2014} `isitup_enabled` is false in group_vars"
        )),
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

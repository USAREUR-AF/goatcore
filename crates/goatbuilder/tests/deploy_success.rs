//! `goatbuilder deploy` when it succeeds: the real binary, a stand-in checkout,
//! and a stubbed ansible that plays clean and answers with the ledger row the
//! play would write. No host is touched.
//!
//! Most deploy tests are refusals or dry runs; the C11 stand-ins accept exit 3,
//! because their recorder prints no play recap. These hold the success path to
//! what an operator reads: exit 0, the text and `-o json` naming what was
//! deployed where, the post-deploy verify reached and its verdict, and the run
//! record the run leaves.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The digest-pinned ref the standup fixture declares for portal on dev.
const PORTAL_REF: &str = "ghcr.io/example-org/portal:v1.4.0@sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";
const DIGEST: &str = "sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";
const HOST: &str = "dev-core-a";
const GROUP: &str = "dev_hosts";

/// `ops/scripts/with-aws <profile> -- <tool> <args...>`, as deploy runs every
/// ansible tool: the inventory lists one host in `dev_hosts`; the play
/// (`ansible-preflight.sh`) prints a clean recap and writes the host's ledger
/// row; the ledger read answers with that row once the play has run, and
/// `NO_LEDGER` before. Every call is logged to `$GB_STUB_DIR/calls`.
fn with_aws(row: &str) -> String {
    format!(
        r#"#!/bin/sh
d="${{GB_STUB_DIR:?}}"
printf '%s\n' "$*" >> "$d/calls"
shift
[ "$1" = "--" ] && shift
case "$1" in
  ansible-inventory)
    printf '%s\n' '{{"_meta":{{"hostvars":{{"{HOST}":{{}}}}}},"all":{{"children":["{GROUP}"]}},"{GROUP}":{{"hosts":["{HOST}"]}}}}' ;;
  ../scripts/ansible-preflight.sh)
    : > "$d/played"
    printf 'PLAY [portal] ***\n\nPLAY RECAP ***\n{HOST} : ok=3 changed=1 unreachable=0 failed=0 skipped=0\n' ;;
  ansible)
    if [ -f "$d/played" ]; then
      printf '%s | CHANGED | rc=0 | (stdout) %s\n' "{HOST}" '{row}'
    else
      printf '%s | CHANGED | rc=0 | (stdout) NO_LEDGER\n' "{HOST}"
    fi ;;
esac
"#
    )
}

struct World {
    root: tempfile::TempDir,
    repo: PathBuf,
}

/// A checkout with the standup fixture's declaration, one component (portal) on
/// one goatnet (dev, one group, one host), every ops/scripts entry a no-op stub
/// except with-aws (above) and served-payloads.py (the real one, which deploy's
/// verify consults), and a secrets posture that answers: no variable is
/// needed or available, so the transport phase's probe succeeds.
fn world(row: &str) -> World {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let repo = r.join("repo");
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = real.join("ops/goatbuilder/fixtures/standup");
    let w = |p: &str, t: &str| {
        let p = repo.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, t).unwrap();
        exec(&p);
    };
    w(
        "ops/goatbuilder/goatbuilder.yaml",
        &std::fs::read_to_string(fixture.join("goatbuilder.yaml")).unwrap(),
    );
    let declared = std::fs::read_to_string(fixture.join("declared/pins.yaml")).unwrap();
    assert!(
        declared.contains(PORTAL_REF),
        "the standup fixture no longer declares {PORTAL_REF}"
    );
    w("ops/goatbuilder/declared/pins.yaml", &declared);
    w(
        "ops/goatbuilder/converger.yaml",
        &std::fs::read_to_string(fixture.join("converger.yaml")).unwrap(),
    );
    for e in std::fs::read_dir(real.join("ops/scripts"))
        .unwrap()
        .flatten()
    {
        if e.file_type().unwrap().is_file() {
            w(
                &format!("ops/scripts/{}", e.file_name().to_string_lossy()),
                "#!/bin/sh\nexit 0\n",
            );
        }
    }
    w(
        "ops/scripts/served-payloads.py",
        &std::fs::read_to_string(real.join("ops/scripts/served-payloads.py")).unwrap(),
    );
    w("ops/scripts/with-aws", &with_aws(row));
    w(
        "ops/secrets/posture.py",
        "import sys\nsys.exit(0 if sys.argv[1:2] == [\"available\"] else 2)\n",
    );
    w(
        "ops/pins/fleet-pins.yaml",
        &format!("goatnets:\n  dev:\n    members: [{GROUP}]\ncomponents:\n  portal:\n    var: portal_image\n    fleet: \"{PORTAL_REF}\"\n"),
    );
    w(
        "ops/ansible/portal-deploy.yml",
        "- name: portal\n  hosts: \"{{ target }}\"\n  tasks: []\n",
    );
    w(
        "ops/ansible/inventory/mgmt-hosts.ini",
        &format!("[{GROUP}]\n{HOST} ansible_host=192.0.2.11\n"),
    );
    w(
        &format!("ops/ansible/group_vars/{GROUP}/pins.yml"),
        &format!("portal_image: \"{PORTAL_REF}\"\n"),
    );
    for d in ["home", "tmp", "stub", "bin"] {
        std::fs::create_dir_all(r.join(d)).unwrap();
    }
    // Deploy's transport phase runs `netbird status`. A stand-in answers it, so
    // the test neither needs netbird on the machine nor reads a real mesh.
    let netbird = r.join("bin/netbird");
    std::fs::write(
        &netbird,
        "#!/bin/sh\necho \"netbird $*\" >> \"$GB_STUB_DIR/calls\"\nexit 0\n",
    )
    .unwrap();
    exec(&netbird);
    World { root, repo }
}

fn exec(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The ledger row the stubbed play writes: rollback-ledger's row cut to its
/// component and the fields deploy reads (`ts`, `action`, `ref`), naming the
/// declared ref.
fn good_row() -> String {
    format!(
        r#"{{"ts": "2026-11-13T00:00:00Z", "component": "portal", "action": "deploy", "ref": "{PORTAL_REF}"}}"#
    )
}

impl World {
    fn deploy(&self, extra: &[&str]) -> Output {
        let r = self.root.path();
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(extra)
            .args(["deploy", "-C"])
            .arg(&self.repo)
            .args([
                "--env",
                "dev",
                "--component",
                "portal",
                "--ack",
                "A Person",
                "--yes",
            ])
            .env_clear()
            .env("PATH", {
                let mut p = std::ffi::OsString::from(r.join("bin"));
                p.push(":");
                p.push(std::env::var_os("PATH").unwrap_or_default());
                p
            })
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(r);
        c.output().unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }

    /// Every run.json under the run's home.
    fn records(&self) -> Vec<serde_json::Value> {
        let runs = self.root.path().join("home/.goat/state/runs");
        let Ok(rd) = std::fs::read_dir(runs) else {
            return Vec::new();
        };
        let mut v: Vec<serde_json::Value> = rd
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("run.json")).ok())
            .map(|t| serde_json::from_str(&t).unwrap())
            .collect();
        v.sort_by_key(|r| r["id"].as_str().unwrap_or("").to_string());
        v
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A deploy whose play runs clean and whose host's ledger then names the
/// declared ref exits 0, and says so: the ref deployed, the group, the recap,
/// the host converged on the digest by this run, and the verdict.
#[test]
fn a_deploy_that_converges_exits_0_and_names_what_it_deployed() {
    let w = world(&good_row());
    let out = w.deploy(&["-o", "table"]);
    let t = text(&out);
    assert_eq!(out.status.code(), Some(0), "{t}");
    // It ran the play on the group, then read the host's ledger.
    let calls = w.calls();
    // The transport phase reached the stand-in, not a netbird on this machine.
    assert!(
        calls.lines().any(|l| l == "netbird status"),
        "netbird status was not answered by the stand-in:\n{calls}"
    );
    assert!(
        calls.lines().any(|l| l.contains("ansible-preflight.sh")
            && l.contains("portal-deploy.yml")
            && l.contains(&format!("target={GROUP}"))),
        "no play on {GROUP}:\n{calls}"
    );
    assert!(
        calls
            .lines()
            .filter(|l| l.contains(" ansible ") && l.contains("deploy-history.jsonl"))
            .count()
            >= 2,
        "the ledger was not read before and after the play:\n{calls}"
    );
    for want in [
        format!("deploying {PORTAL_REF}"),
        format!("{HOST} : ok=3 changed=1 unreachable=0 failed=0"),
        format!("converged on {DIGEST}"),
        // The host had no ledger before the play, so this run wrote the row.
        "ledger row written by this run".to_string(),
        "portal on dev: every host's ledger names the declared ref".to_string(),
    ] {
        assert!(t.contains(&want), "the page does not say {want:?}:\n{t}");
    }
    // No command failed, the secrets posture probe included.
    assert!(!t.contains("NOT converged") && !t.contains("FAILED"), "{t}");
}

/// The same run under `-o json`: one report, exit 0, ok, every row ok, and rows
/// that name the deploy, the recap, the host's convergence and the verdict.
#[test]
fn a_deploy_that_converges_reports_it_in_json() {
    let w = world(&good_row());
    let out = w.deploy(&["-o", "json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON report: {e}\n{}", text(&out)));
    assert_eq!(v["verb"], "deploy", "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["exit"], 0, "{v}");
    let rows = v["checks"].as_array().unwrap();
    assert!(rows.iter().all(|c| c["ok"] == true), "a row is not ok: {v}");
    let row = |name: &str| {
        rows.iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no row {name}: {v}"))
    };
    assert!(row(&format!("dev/{GROUP}/deploy"))["detail"]
        .as_str()
        .unwrap()
        .contains(PORTAL_REF));
    assert!(row(&format!("dev/{GROUP}/recap"))["detail"]
        .as_str()
        .unwrap()
        .contains("failed=0"));
    let host = row(&format!("dev/{HOST}"))["detail"].as_str().unwrap();
    assert!(host.contains(&format!("converged on {DIGEST}")), "{host}");
    assert!(row("dev/portal")["detail"]
        .as_str()
        .unwrap()
        .contains("every host's ledger names the declared ref"));
}

/// The post-deploy verify is reached and decides the outcome: the same clean
/// play, but a ledger row that names another digest, exits 3 and names the
/// host NOT converged on that row. A deploy that skipped verify would pass
/// this play.
#[test]
fn the_verify_after_the_play_decides_the_outcome() {
    let other = good_row().replace(
        DIGEST,
        "sha256:4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d",
    );
    let w = world(&other);
    let out = w.deploy(&["-o", "table"]);
    let t = text(&out);
    assert_eq!(out.status.code(), Some(3), "{t}");
    assert!(
        t.contains(&format!("{HOST} : ok=3")),
        "the play did not run: {t}"
    );
    // "ledger tail" is the verdict for a finished deploy row that names
    // another digest; a row naming the declared digest but not finished gets
    // other words. The digest itself is not on the page: the tail shown is
    // cut to 140 characters, which ends inside the ref.
    assert!(
        t.lines().any(|l| l.contains(&format!("dev/{HOST}"))
            && l.contains("NOT converged — ledger tail")
            && l.contains(r#""action": "deploy""#)),
        "the host's row is not the other-digest verdict: {t}"
    );
    assert!(
        !t.contains("every host's ledger names the declared ref"),
        "{t}"
    );
}

/// The run leaves one record: verb deploy, the command line it was given (the
/// component and the goatnet), exit 0, and its commands under their phases:
/// the play on the group's target under `deploy:<group>`, and the ledger read
/// under `verify:<group>`, which ran (it carries commands and no skip verdict).
#[test]
fn a_deploy_that_converges_leaves_its_run_record() {
    let w = world(&good_row());
    let out = w.deploy(&["-o", "table"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let recs = w.records();
    assert_eq!(recs.len(), 1, "{recs:?}");
    let r = &recs[0];
    assert_eq!(r["verb"], "deploy", "{r}");
    assert_eq!(r["exit"], 0, "{r}");
    let args: Vec<&str> = r["args"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    for (flag, val) in [("--component", "portal"), ("--env", "dev")] {
        assert!(
            args.windows(2).any(|p| p == [flag, val]),
            "the record's args do not name {flag} {val}: {args:?}"
        );
    }
    let cmds = r["commands"].as_array().unwrap();
    let play = cmds
        .iter()
        .find(|c| {
            c["command"]
                .as_str()
                .is_some_and(|l| l.contains("ansible-preflight.sh"))
        })
        .unwrap_or_else(|| panic!("no play in the record: {r}"));
    assert_eq!(play["phase"], format!("deploy:{GROUP}"), "{play}");
    assert_eq!(play["exit"], 0, "{play}");
    let line = play["command"].as_str().unwrap();
    assert!(
        line.contains("portal-deploy.yml") && line.contains(&format!("target={GROUP}")),
        "{line}"
    );
    let phases = r["phases"].as_array().unwrap();
    // Every phase exited cleanly, the secrets probe's transport phase included.
    let transport = phases
        .iter()
        .find(|p| p["id"] == "transport")
        .unwrap_or_else(|| panic!("no transport phase: {r}"));
    assert_eq!(transport["exit"], 0, "{transport}");
    let verify = phases
        .iter()
        .find(|p| p["id"] == format!("verify:{GROUP}"))
        .unwrap_or_else(|| panic!("no verify phase: {r}"));
    assert!(
        verify["commands"].as_u64().unwrap_or(0) > 0 && verify["verdict"].is_null(),
        "verify did not run: {verify}"
    );
    assert!(
        cmds.iter().any(|c| c["phase"] == format!("verify:{GROUP}")
            && c["command"]
                .as_str()
                .is_some_and(|l| l.contains("deploy-history.jsonl"))),
        "the verify phase did not read the ledger: {r}"
    );
}

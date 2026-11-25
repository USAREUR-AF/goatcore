//! `fastlane --dry-run` resolves an image under the owner its fleet ref names,
//! on both avenues: the binary and the real ops/scripts/fastlane.sh, line for
//! line, on one checkout and one stand-in `gh` that answers for one owner only.

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::Path;
use std::process::Command;

const DIGEST: &str = "sha256:7777777777777777777777777777777777777777777777777777777777777777";

/// Both avenues' exit and lines for a component pinned at `fleet`, with a
/// registry that holds the package under `holder` alone.
fn both(fleet: &str, holder: &str) -> [(i32, String); 2] {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = r.join("repo");
    let put = |p: &str, text: &str| {
        let p = repo.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    for s in ["fastlane.sh", "project-setting.sh", "lib/keep-awake.sh"] {
        test_support::copied_executable(&real.join("ops/scripts").join(s), &repo.join("ops/scripts").join(s));
    }
    put("ops/release/graduate.py", &std::fs::read_to_string(real.join("ops/release/graduate.py")).unwrap());
    put("ops/goatbuilder/goatbuilder.yaml", &std::fs::read_to_string(real.join("ops/goatbuilder/fixtures/standup/goatbuilder.yaml")).unwrap());
    put("ops/ansible/widget-deploy.yml", "- name: w\n  hosts: all\n  tasks: []\n");
    put("ops/ansible/roles/widget/tasks/main.yml", "[]\n");
    put("ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros: {{members: [z_hosts]}}\ncomponents:\n  widget:\n    var: widget_image\n    fleet: \"{fleet}\"\n"));
    test_support::executable(
        &r.join("bin/gh"),
        &format!("#!/bin/sh\n[ \"$1\" = --version ] && exit 0\ncase \"$*\" in *\"/orgs/{holder}/packages/container/widget/\"*) echo {DIGEST}; exit 0 ;; esac\necho 'gh: Package not found. (HTTP 404)' >&2\nexit 1\n"),
    );
    std::fs::create_dir_all(r.join("home")).unwrap();
    for a in [&["init", "-q", "-b", "main"][..], &["add", "-A"], &["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "fixture"]] {
        assert!(Command::new("git").args(a).current_dir(&repo).env("GIT_CONFIG_GLOBAL", "/dev/null").status().unwrap().success());
    }
    let words = ["widget", "--ref", "sha-b7a1621", "--reason", "why", "--ack", "A Person", "--dry-run"];
    let run = |mut c: Command| {
        let o = c
            .env_clear()
            .env("PATH", format!("{}:{}", r.join("bin").display(), std::env::var("PATH").unwrap_or_default()))
            .env("HOME", r.join("home"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(&repo)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let text = match serde_json::from_str::<serde_json::Value>(&out) {
            Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{}\n", c["detail"].as_str().unwrap_or(""))).collect(),
            _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
        };
        let lines: Vec<String> = text
            .replace("\u{1b}[1m", "")
            .replace("\u{1b}[0m", "")
            .lines()
            .map(|l| l.trim().trim_start_matches("fastlane: ").to_string())
            .filter(|l| !l.is_empty())
            .collect();
        (o.status.code().unwrap_or(-1), lines.join("\n"))
    };
    let mut sh = Command::new("bash");
    sh.arg("ops/scripts/fastlane.sh").args(words);
    let mut gb = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    gb.args(["-o", "json", "-C"]).arg(&repo).arg("fastlane").args(words);
    [run(sh), run(gb)]
}

#[test]
fn an_image_is_read_and_pinned_under_the_owner_its_fleet_ref_names() {
    let old = format!("sha256:{}", "1".repeat(64));
    // The fixture's goatbuilder.yaml declares example-org.
    for (fleet, holder, rc, says) in [
        (format!("ghcr.io/other-org/widget:sha-0000000@{old}"), "other-org", 0, format!("would set components.widget.fleet = ghcr.io/other-org/widget:sha-b7a1621@{DIGEST}")),
        (format!("ghcr.io/other-org/widget@{old}"), "other-org", 0, format!("new fleet ref: ghcr.io/other-org/widget:sha-b7a1621@{DIGEST}")),
        // GitHub spells the owner; a registry path is lower case.
        (format!("ghcr.io/Other-Org/widget:sha-0000000@{old}"), "Other-Org", 0, format!("new fleet ref: ghcr.io/other-org/widget:sha-b7a1621@{DIGEST}")),
        (format!("ghcr.io/{{{{ ghcr_org }}}}/widget:sha-0000000@{old}"), "example-org", 0, format!("new fleet ref: ghcr.io/{{{{ ghcr_org }}}}/widget:sha-b7a1621@{DIGEST}")),
        // A package of that name under this checkout's owner is not the pinned one.
        (format!("ghcr.io/other-org/widget:sha-0000000@{old}"), "example-org", 1, "cannot read ghcr.io/other-org/widget".to_string()),
        (format!("ghcr.io/{{{{ ghcr_org }}}}/widget:sha-0000000@{old}"), "other-org", 1, "cannot read ghcr.io/example-org/widget".to_string()),
    ] {
        let [sh, gb] = both(&fleet, holder);
        assert_eq!(sh.0, rc, "{fleet}: {}", sh.1);
        assert!(sh.1.contains(&says), "{fleet}: no {says:?} in {}", sh.1);
        assert_eq!(sh, gb, "{fleet} held by {holder}");
    }
}

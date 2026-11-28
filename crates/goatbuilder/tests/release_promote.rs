//! `promote` and `fastlane` of a release component (gate J23) on both avenues:
//! the binary, and the real ops/release/graduate.py and ops/scripts/fastlane.sh,
//! each on its own copy of one checkout, line for line. The release is signed by
//! a stand-in Sigstore and read from a mirror directory. Nothing lands or
//! deploys here; ops/e2e/release-file.sh runs the whole fastlane.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "example-org/example-tool";

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(b))
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (f, t) = (e.path(), to.join(e.file_name()));
        match e.file_type().unwrap() {
            k if k.is_dir() => copy_tree(&f, &t),
            k if k.is_file() => test_support::copied_executable(&f, &t),
            _ => {}
        }
    }
}

struct World {
    root: tempfile::TempDir,
    pins: [String; 2],
}

/// Two copies of one checkout, `py` and `rs`: a three-rung ladder whose hop
/// requires runtime health, an image component and a release component.
fn world() -> World {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let s = sigstore::Sigstore::new();
    std::fs::write(r.join("trusted_root.json"), s.trusted_root()).unwrap();
    let pins = ["v1.0.0", "tool/v2.0.0"].map(|tag| {
        let at = r.join("mirror").join(REPO).join(tag);
        std::fs::create_dir_all(&at).unwrap();
        let file = format!("{tag} build\n");
        let sums = format!("{}  tool-linux-amd64\n", sha256(file.as_bytes()));
        std::fs::write(at.join("tool-linux-amd64"), file).unwrap();
        std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
        let w = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: tag.into() };
        std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &w)).unwrap();
        format!("{REPO}@{tag}@sha256:{}", sha256(sums.as_bytes()))
    });
    for side in ["py", "rs"] {
        let repo = r.join(side);
        let put = |p: &str, t: &str| {
            let p = repo.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        copy_tree(&real.join("ops/scripts"), &repo.join("ops/scripts"));
        put("ops/release/graduate.py", &std::fs::read_to_string(real.join("ops/release/graduate.py")).unwrap());
        put("ops/release/ladder.yaml", "rungs:\n- {name: dev, goatnet: zagros_dev, status: live, role: r}\n- {name: preprod, goatnet: zagros_pre, status: live, role: r}\n- {name: prod, goatnet: zagros, status: live, role: r}\nhops:\n- {from: dev, to: preprod, status: live, gates: [{id: candidate-record}]}\n- {from: preprod, to: prod, status: live, gates: [{id: candidate-record}, {id: runtime-health}, {id: operator-ack}]}\n");
        put("ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros_dev: {{members: [d_hosts]}}\n  zagros_pre: {{members: [p_hosts]}}\n  zagros: {{members: [z_hosts]}}\ncomponents:\n  svc:\n    var: svc_image\n    fleet: \"ghcr.io/example-org/svc:v1@sha256:{0}\"\n  tool:\n    var: tool_file\n    artifact_type: release\n    fleet: \"{1}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: tool-linux-amd64}}\n      signer: https://github.com/{REPO}/.github/workflows/_release-binary.yml\n    deviations:\n      zagros_dev:\n        value: \"{1}\"\n        reason: a trial\n      p_hosts:\n        absent: true\n        reason: none there\n", "1".repeat(64), pins[0]));
        put("ops/goatbuilder/goatbuilder.yaml", &std::fs::read_to_string(real.join("ops/goatbuilder/fixtures/standup/goatbuilder.yaml")).unwrap());
        put("ops/ansible/release-file-deploy.yml", "- name: r\n  hosts: \"{{ target }}\"\n  tasks: []\n");
        for a in [&["init", "-q", "-b", "main"][..], &["add", "-A"], &["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "fixture"]] {
            assert!(Command::new("git").args(a).current_dir(&repo).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_AUTHOR_DATE", "2026-11-01T00:00:00Z").env("GIT_COMMITTER_DATE", "2026-11-01T00:00:00Z").status().unwrap().success());
        }
    }
    std::fs::create_dir_all(r.join("home")).unwrap();
    World { root, pins }
}

impl World {
    fn run(&self, mut c: Command, side: &str) -> (i32, String) {
        let r = self.root.path();
        let o = c
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", r.join("home"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOATBUILDER_RELEASE_MIRROR", r.join("mirror"))
            .env("GOATBUILDER_UPDATE_TRUSTED_ROOT", r.join("trusted_root.json"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("COLUMNS", "100")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(r.join(side))
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let text = match serde_json::from_str::<serde_json::Value>(&out) {
            Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{}\n", c["detail"].as_str().unwrap_or(""))).collect(),
            _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
        };
        (o.status.code().unwrap_or(-1), text)
    }

    /// A promote step on each side's copy, held to one another; its exit and words.
    fn promote(&self, args: &[&str]) -> (i32, String) {
        let mut py = Command::new("python3");
        py.arg("ops/release/graduate.py").args(args);
        let mut rs = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        rs.args(["-o", "json", "-C"]).arg(self.root.path().join("rs")).args(["--yes", "promote"]).args(args);
        let (a, b) = (self.run(py, "py"), self.run(rs, "rs"));
        assert_eq!(a.0, b.0, "{args:?}\n{}\n{}", a.1, b.1);
        assert_eq!(norm(&a.1), norm(&b.1), "{args:?}");
        b
    }

    fn fastlane(&self, reference: &str) -> (i32, String) {
        let words = ["--ref", reference, "--reason", "why", "--ack", "A Person", "--dry-run"];
        let mut sh = Command::new("bash");
        sh.args(["ops/scripts/fastlane.sh", "tool"]).args(words);
        let mut gb = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        gb.args(["-o", "json", "-C"]).arg(self.root.path().join("rs")).args(["fastlane", "tool"]).args(words);
        let (a, b) = (self.run(sh, "py"), self.run(gb, "rs"));
        assert_eq!(a.0, b.0, "{}\n{}", a.1, b.1);
        assert_eq!(norm(&a.1), norm(&b.1));
        b
    }

    fn record(&self, side: &str) -> PathBuf {
        let d = self.root.path().join(side).join("ops/release/records/tool");
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.path()).filter(|p| !p.to_string_lossy().contains("hotfix")).collect();
        v.sort();
        v.pop().unwrap()
    }
}

fn norm(t: &str) -> String {
    t.lines()
        .map(|l| {
            let mut l = l.replace("\u{1b}[1m", "").replace("\u{1b}[0m", "").replace("python3 ops/release/graduate.py", "TOOL").replace("goatbuilder promote", "TOOL").replace("python3 ops/pins/generate-pins.py", "PINS").replace("goatbuilder pins", "PINS").replace("graduate.py", "TOOL").replace("`TOOL check`", "`check`").replace("`promote check`", "`check`").replace("`promote graduate`", "`TOOL graduate`");
            for p in ["goatbuilder: ", "fastlane: "] {
                l = l.strip_prefix(p).map(String::from).unwrap_or(l);
            }
            l.trim().to_string()
        })
        .filter(|l| !l.is_empty() && !l.starts_with("(GB-"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_release_is_opened_graduated_and_hotfixed_only_at_a_release_pin() {
    let w = world();
    let image = format!("ghcr.io/example-org/svc:v2@sha256:{}", "2".repeat(64));
    let new = w.pins[1].clone();
    for verb in ["open", "hotfix"] {
        let ack: &[&str] = if verb == "hotfix" { &["--ack", "A Person"] } else { &[] };
        let (rc, t) = w.promote(&[&[verb, "tool", "--ref", &image, "--reason", "x"], ack].concat());
        assert!(rc == 1 && t.contains("tool is a release component (artifact_type: release) and ghcr.io/example-org/svc"), "{t}");
        let (rc, t) = w.promote(&[&[verb, "svc", "--ref", &new, "--reason", "x"], ack].concat());
        assert!(rc == 1 && t.contains("is a release pin and svc is not a release component"), "{t}");
    }
    let (rc, t) = w.promote(&["open", "tool", "--ref", &new, "--reason", "the next release"]);
    assert!(rc == 0 && t.contains(&format!("value: {new}")), "{t}");
    for args in [&["status"][..], &["check"], &["review"]] {
        w.promote(args);
    }
    // No capture reads a file; the decision is an attributed override.
    let (rc, t) = w.promote(&["graduate", "tool", "--ack", "A Person"]);
    assert!(rc == 1 && t.contains("runtime-health: no capture reads a release component on preprod: it is a file, not a container"), "{t}");
    let (rc, t) = w.promote(&["graduate", "tool", "--ack", "A Person", "--override-runtime-health", "a command line tool"]);
    assert!(rc == 0 && t.contains(&format!("fleet: {new}")), "{t}");
    let rec = |s: &str| serde_yaml::from_str::<serde_yaml::Value>(&std::fs::read_to_string(w.record(s)).unwrap()).unwrap();
    assert_eq!(rec("py"), rec("rs"));
    assert_eq!(rec("rs")["state"], "graduated");
    assert_eq!(rec("rs")["runtime_health_override"]["preprod"]["by"], "A Person");
}

#[test]
fn check_refuses_an_open_record_whose_ref_is_not_the_components_kind() {
    let w = world();
    for side in ["py", "rs"] {
        let d = w.root.path().join(side).join("ops/release/records/tool");
        std::fs::create_dir_all(&d).unwrap();
        for (n, state) in [("20261101-aaaaaaaaaaaa", "candidate"), ("20261001-bbbbbbbbbbbb", "graduated")] {
            std::fs::write(d.join(format!("{n}.yaml")), format!("component: tool\nclass: ladder\ncandidate_ref: \"ghcr.io/example-org/svc:v2@sha256:{}\"\nprevious_fleet_ref: x\nopened: '2026-11-01'\nreason: r\nevidence: {{validation: v, preprod_convergence: c}}\nstate: {state}\ngraduated: null\noperator_ack: A\n", "2".repeat(64))).unwrap();
        }
    }
    let (rc, t) = w.promote(&["check"]);
    assert!(rc == 1 && t.contains("C2: ops/release/records/tool/20261101-aaaaaaaaaaaa.yaml tool is a release component"), "{t}");
    assert!(!t.contains("20261001-bbbbbbbbbbbb.yaml tool is a release component"), "a closed record is history: {t}");
}

#[test]
fn fastlane_verifies_the_release_and_says_what_it_would_move() {
    let w = world();
    let (rc, t) = w.fastlane(&w.pins[1]);
    assert_eq!(rc, 0, "{t}");
    for s in ["==> verifying the release", "release verified: tool-linux-amd64 (amd64) sha256", "would set components.tool.fleet = example-org/example-tool@tool/v2.0.0@sha256:", "would clear deviations: zagros_dev", "would PRESERVE (absent: true — topology, not skew): p_hosts", "would deploy via release-file-deploy.yml"] {
        assert!(t.contains(s), "no {s:?} in {t}");
    }
    let (rc, t) = w.fastlane(&w.pins[0]);
    assert!(rc == 1 && t.contains("that ref is already the fleet value"), "{t}");
    let (rc, t) = w.fastlane("v2.0.0");
    assert!(rc == 1 && t.contains("v2.0.0 is not a release pin"), "{t}");
    let (rc, t) = w.fastlane(&format!("{REPO}@tool/v2.0.0@sha256:{}", "e".repeat(64)));
    assert!(rc == 1 && t.contains("GB-381 refused before any host: tool's release") && t.contains("(check: digest). Nothing moved."), "{t}");
}

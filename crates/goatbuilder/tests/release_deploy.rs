//! A release component (gate J23) on both avenues: the real binary and the real
//! ops/scripts/deploy-component.sh, on one stand-in checkout, with a release the
//! stand-in Sigstore signed in a mirror directory. `with-aws`, which starts
//! every ansible tool, is a stand-in that records its calls and answers as a
//! host would. ops/e2e/release-file.sh does the same on a real container host.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

#[path = "common/signed_intent.rs"]
mod signed_intent;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REPO: &str = "example-org/example-tool";
const TAG: &str = "tool/v1.2.0";
const SIGNER: &str = "https://github.com/example-org/example-tool/.github/workflows/_release-binary.yml";
const PINS: &str = "ops/ansible/group_vars/test_hosts/pins.yml";

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
    repo: PathBuf,
    digest: String,
}

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let repo = r.join("repo");
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy_tree(&real.join("ops/scripts"), &repo.join("ops/scripts"));
        let w = |p: &str, t: &str| {
            let p = repo.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, t).unwrap();
        };
        // The signed release, in a mirror directory.
        let at = r.join("mirror").join(REPO).join(TAG);
        std::fs::create_dir_all(&at).unwrap();
        let sums = format!("{}  tool-linux-amd64\n{}  tool-linux-arm64\n", sha256(b"amd64 build\n"), sha256(b"arm64 build\n"));
        std::fs::write(at.join("tool-linux-amd64"), b"amd64 build\n").unwrap();
        std::fs::write(at.join("tool-linux-arm64"), b"arm64 build\n").unwrap();
        std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
        let s = sigstore::Sigstore::new();
        let signed = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &signed)).unwrap();
        std::fs::write(r.join("trusted_root.json"), s.trusted_root()).unwrap();
        let digest = sha256(sums.as_bytes());

        w("ops/release/ladder_gate.py", &std::fs::read_to_string(real.join("ops/release/ladder_gate.py")).unwrap());
        w("ops/secrets/posture.py", "import sys\nsys.exit(0 if sys.argv[1:2] == [\"available\"] else 2)\n");
        w("ops/ansible/release-file-deploy.yml", &std::fs::read_to_string(real.join("ops/ansible/release-file-deploy.yml")).unwrap());
        w("ops/ansible/inventory/mgmt-hosts.ini", "[test_hosts]\nhost-a ansible_host=192.0.2.11\n");
        // Deploy verifies the goatnet's signed intent before any host (GB-383);
        // the script avenue with the real generate-pins.py.
        copy_tree(&real.join("ops/consistency/python"), &repo.join("ops/consistency/python"));
        w("ops/pins/generate-pins.py", &std::fs::read_to_string(real.join("ops/pins/generate-pins.py")).unwrap());
        test_support::executable(&repo.join("ops/scripts/fresh-source.sh"), "#!/bin/sh\necho 1111111111111111111111111111111111111111\n");
        test_support::executable(
            &repo.join("ops/scripts/with-aws"),
            r#"#!/bin/sh
d="${GB_STUB_DIR:?}"
printf '%s\n' "$*" >> "$d/calls"
shift; [ "$1" = "--" ] && shift
case "$1" in
  ansible-inventory) printf '%s\n' '{"_meta":{"hostvars":{"host-a":{}}},"test_hosts":{"hosts":["host-a"]}}' ;;
  ../scripts/ansible-preflight.sh) : > "$d/played"; printf 'PLAY RECAP ***\nhost-a : ok=3 changed=1 unreachable=0 failed=0\n' ;;
  ansible)
    case "$*" in
      *GBFILE*) printf 'host-a | CHANGED | rc=0 | (stdout) GBFILE %s\n' "$(cat "$d/file")" ;;
      *) if [ -f "$d/played" ]; then printf 'host-a | CHANGED | rc=0 | (stdout) %s\n' "$(cat "$d/row")"; else printf 'host-a | CHANGED | rc=0 | (stdout) NO_LEDGER\n'; fi ;;
    esac ;;
esac
"#,
        );
        for d in ["home", "tmp", "stub"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        test_support::executable(&r.join("bin/netbird"), "#!/bin/sh\nexit 0\n");
        let w = World { root, repo, digest };
        w.pin(&w.pin_text());
        w.host("x86_64", &sha256(b"amd64 build\n"));
        w
    }

    fn pin_text(&self) -> String {
        format!("{REPO}@{TAG}@sha256:{}", self.digest)
    }

    fn pin(&self, pin: &str) {
        self.declare(pin, SIGNER);
    }

    fn declare(&self, pin: &str, signer: &str) {
        let put = |p: &str, t: String| {
            std::fs::create_dir_all(self.repo.join(p).parent().unwrap()).unwrap();
            std::fs::write(self.repo.join(p), t).unwrap();
        };
        put(
            "ops/pins/fleet-pins.yaml",
            format!("goatnets:\n  test_site:\n    members: [test_hosts]\ncomponents:\n  tool:\n    var: tool_file\n    artifact_type: release\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: tool-linux-amd64, arm64: tool-linux-arm64}}\n      signer: {signer}\n"),
        );
        put(PINS, format!("tool_file: \"{pin}\"\n"));
        put("../stub/row", format!("{{\"ts\": \"2026-11-24T00:00:00Z\", \"action\": \"deploy\", \"ref\": \"{pin}\"}}"));
        // A pin `goatbuilder pins` accepts is rendered and its intent signed, so
        // the release check is what decides; one it refuses leaves the intent
        // unsigned, as it would be.
        if pin.contains("@sha256:") {
            signed_intent::prepare(&self.repo, &["test_hosts"]);
            signed_intent::sign(&self.repo);
        }
    }

    fn host(&self, machine: &str, sha: &str) {
        let file = if machine == "none" { "none".to_string() } else { format!("{machine} {sha}  /opt/goat/releases/tool/{sha}/tool") };
        std::fs::write(self.root.path().join("stub/file"), file).unwrap();
    }

    fn env(&self, c: &mut Command) {
        let r = self.root.path();
        let mut path = std::ffi::OsString::from(r.join("bin"));
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        c.env_clear()
            .env("PATH", path)
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOATBUILDER_RELEASE_MIRROR", r.join("mirror"))
            .env("GOATBUILDER_UPDATE_TRUSTED_ROOT", r.join("trusted_root.json"))
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(&self.repo);
    }

    fn binary(&self, args: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "-C"]).arg(&self.repo).args(args);
        self.env(&mut c);
        c.output().unwrap()
    }

    fn script(&self, name: &str, args: &[&str]) -> Output {
        let mut c = Command::new("bash");
        c.arg(self.repo.join("ops/scripts").join(name)).arg("tool").args(args);
        self.env(&mut c);
        c.output().unwrap()
    }

    /// The same deploy on the binary, then on the script, each from a host that
    /// no play has run on.
    fn deploy(&self, extra: &[&str]) -> [Output; 2] {
        let gb = [&["deploy", "--env", "test_site", "--component", "tool", "--ack", "A Person", "--yes"], extra].concat();
        let sh = [&["--goatnet", "test_site"], extra].concat();
        let _ = std::fs::remove_file(self.root.path().join("stub/played"));
        let a = self.binary(&gb);
        let _ = std::fs::remove_file(self.root.path().join("stub/played"));
        [a, self.script("deploy-component.sh", &sh)]
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }

    fn vars(&self) -> PathBuf {
        self.root.path().join("home/.goat/state/releases/tool").join(&self.digest).join("deploy-vars.json")
    }
}

/// Every detail of the binary's report, or the script's lines.
fn text(o: &Output) -> String {
    let out = String::from_utf8_lossy(&o.stdout);
    match serde_json::from_str::<serde_json::Value>(&out) {
        Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{} {}\n", c["name"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""))).collect(),
        _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
    }
}

fn says(outs: &[Output], exits: [i32; 2], words: &[&str]) {
    for (o, exit) in outs.iter().zip(exits) {
        let t = text(o);
        assert_eq!(o.status.code(), Some(exit), "{t}");
        for w in words {
            assert!(t.contains(w), "no {w:?} in:\n{t}");
        }
    }
}

/// The refusal both avenues print, from its code to its check.
fn refusal(t: &str) -> String {
    let at = t.find("GB-381 ").unwrap_or_else(|| panic!("no GB-381 in:\n{t}"));
    let rest = &t[at..];
    rest[..rest.find(')').expect("the check") + 1].to_string()
}

#[test]
fn a_dry_run_fetches_and_verifies_and_reaches_no_host() {
    let w = World::new();
    let outs = w.deploy(&["--dry-run"]);
    says(
        &outs,
        [0, 0],
        &["release verified: tool-linux-amd64 (amd64) sha256", "signed by https://github.com/example-org/example-tool/.github/workflows/_release-binary.yml@refs/tags/tool/v1.2.0", "release-file-deploy.yml", "DRY: would read the file tool runs on each host of test_hosts and require the pin's"],
    );
    for o in &outs {
        assert!(text(o).contains(&format!("-e @{}", w.vars().display())), "{}", text(o));
    }
    assert_eq!(w.calls(), "", "a dry run reaches no host");
    let vars: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(w.vars()).unwrap()).unwrap();
    assert_eq!(vars["release_file_ref"], w.pin_text());
    assert_eq!(vars["release_file_files"]["arm64"]["sha256"], sha256(b"arm64 build\n"));
}

#[test]
fn a_release_that_does_not_verify_is_refused_before_any_host_in_the_same_words() {
    let good = World::new().pin_text();
    let other = format!("{REPO}@{TAG}@sha256:{}", "e".repeat(64));
    let wrong_signer = "https://github.com/example-org/example-tool/.github/workflows/other.yml";
    for (check, pin, signer, spoil) in [
        ("digest", Some(other.as_str()), SIGNER, ""),
        ("signature", None, wrong_signer, ""),
        ("listed", None, SIGNER, "tool-linux-arm64"),
        ("fetch", None, SIGNER, "SHA256SUMS.cosign-bundle"),
        ("pin", Some("example-org/example-tool@v1"), SIGNER, ""),
    ] {
        let w = World::new();
        assert_eq!(w.pin_text().len(), good.len());
        w.declare(pin.unwrap_or(&w.pin_text()), signer);
        let spoiled = w.root.path().join("mirror").join(REPO).join(TAG).join(spoil);
        match spoil {
            "" => {}
            "tool-linux-arm64" => std::fs::write(spoiled, b"another build\n").unwrap(),
            _ => std::fs::remove_file(spoiled).unwrap(),
        }
        let at = format!("(check: {check})");
        for dry in [false, true] {
            let outs = w.deploy(if dry { &["--dry-run"] } else { &[] });
            // A pin with no digest is refused first by the digest gate (GB-382),
            // and its intent cannot be signed (GB-383); a dry run names all three.
            if check == "pin" && !dry {
                says(&outs, [1, 1], &["GB-382 REFUSE — tool_file in group_vars/test_hosts/pins.yml is example-org/example-tool@v1"]);
                continue;
            }
            says(&outs, [1, 1], &["refused before any host: tool's release", &at]);
            assert_eq!(refusal(&text(&outs[0])), refusal(&text(&outs[1])), "{check}");
            if dry {
                let verdict = if check == "pin" { "(digest intent release)" } else { "(release)" };
                says(&outs, [1, 1], &["DRY: would read /var/lib/goat/tool/deploy-history.jsonl", &format!("DRY: a real run would refuse before any host {verdict}; nothing was read")]);
            }
        }
        assert_eq!(w.calls(), "", "{check}: no host");
    }
}

#[test]
fn a_deploy_passes_what_it_verified_and_holds_the_hosts_file_to_the_pin() {
    let w = World::new();
    let amd = sha256(b"amd64 build\n");
    says(&w.deploy(&[]), [0, 0], &[&format!("installed file tool-linux-amd64 is the pin's (sha256 {amd})"), &format!("converged on sha256:{}", w.digest)]);
    let plays: Vec<String> = w.calls().lines().filter(|l| l.contains("ansible-preflight.sh")).map(String::from).collect();
    assert_eq!(plays.len(), 2, "{plays:?}");
    for p in &plays {
        assert!(p.ends_with(&format!("release-file-deploy.yml -e target=test_hosts -e @{}", w.vars().display())), "{p}");
    }
    // Both architectures, another file, no file, an architecture the release has none for.
    w.host("aarch64", &sha256(b"arm64 build\n"));
    says(&w.deploy(&[]), [0, 0], &["installed file tool-linux-arm64 is the pin's"]);
    for (machine, sha, words) in [
        ("x86_64", "0".repeat(64), "but the installed file is sha256 0000"),
        ("aarch64", amd.clone(), "but the installed file is sha256"),
        ("riscv64", amd.clone(), "but the host is riscv64, and the release has no file for it"),
        ("none", String::new(), "but no file is installed"),
    ] {
        w.host(machine, &sha);
        says(&w.deploy(&[]), [3, 1], &["NOT converged — ledger on", words]);
    }
}

#[test]
fn verify_holds_the_file_and_says_unknown_when_it_cannot_verify_the_release() {
    let w = World::new();
    w.deploy(&[]);
    let verify = || [w.binary(&["verify", "tool", "--env", "test_site"]), w.script("deploy-component.sh", &["--goatnet", "test_site", "--verify-only"])];
    says(&verify(), [0, 0], &["installed file tool-linux-amd64 is the pin's"]);
    w.host("x86_64", &"0".repeat(64));
    says(&verify(), [3, 1], &["NOT converged — ledger on"]);
    // Neither kept nor in the mirror: the file is not compared, and not called converged.
    w.host("x86_64", &sha256(b"amd64 build\n"));
    std::fs::remove_dir_all(w.root.path().join("home/.goat/state/releases")).unwrap();
    std::fs::remove_dir_all(w.root.path().join("mirror")).unwrap();
    let outs = verify();
    says(&outs, [3, 1], &["unknown — ledger on", "its file was not compared: GB-381 tool's release", "(check: fetch)"]);
    for o in &outs {
        assert!(!text(o).contains("refused before any host"), "verify refuses nothing: {}", text(o));
    }
    // A verify dry run reads and fetches nothing.
    let before = w.calls();
    let o = w.binary(&["verify", "tool", "--env", "test_site", "--dry-run"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert_eq!(w.calls(), before);
}

#[test]
fn kept_files_verify_again_with_no_mirror() {
    let w = World::new();
    w.deploy(&["--dry-run"]);
    std::fs::remove_dir_all(w.root.path().join("mirror")).unwrap();
    says(&w.deploy(&["--dry-run"]), [0, 0], &["release verified:"]);
}

#[test]
fn a_rollback_runs_the_play_and_believes_only_a_new_rollback_row() {
    let w = World::new();
    w.deploy(&[]);
    let roll = || [w.binary(&["rollback", "tool", "1", "--site", "test-site", "--yes"]), w.script("rollback.sh", &["1", "--site", "test-site"])];
    // The host's newest row is still the deploy's: nothing was rolled back.
    says(&roll(), [3, 1], &["host-a: NOT rolled back", "NOT rolled back everywhere"]);
    // A rollback row that was there before this run is not this run's.
    std::fs::write(w.root.path().join("stub/row"), "{\"ts\": \"2026-11-24T01:00:00Z\", \"action\": \"rollback\", \"ref\": \"a\", \"prior_ref\": \"b\"}").unwrap();
    says(&roll(), [3, 1], &["this run wrote no ledger row; its newest is an earlier rollback (2026-11-24T01:00:00Z)"]);
    let plays: Vec<String> = w.calls().lines().filter(|l| l.contains("release_file_rollback=true")).map(String::from).collect();
    assert_eq!(plays.len(), 4, "{plays:?}");
    assert!(plays.iter().all(|p| p.ends_with("-e target=test_hosts -e release_file_component=tool -e release_file_ledger=tool -e release_file_rollback=true")), "{plays:?}");
    // No row before, a rollback row after: it moved.
    for i in 0..2 {
        std::fs::remove_file(w.root.path().join("stub/played")).unwrap();
        let o = if i == 0 { w.binary(&["rollback", "tool", "1", "--site", "test-site", "--yes"]) } else { w.script("rollback.sh", &["1"]) };
        let t = text(&o);
        assert_eq!(o.status.code(), Some(0), "{t}");
        assert!(t.contains("host-a: rolled back: b -> a"), "{t}");
    }
}

#[test]
fn a_rollback_of_more_than_one_step_or_a_saved_plan_of_one_is_refused_gb379() {
    let w = World::new();
    let outs = [w.binary(&["rollback", "tool", "2", "--site", "test-site", "--yes"]), w.script("rollback.sh", &["2", "--site", "test-site"])];
    says(&outs, [2, 1], &["tool is a release component (artifact_type: release): a host keeps the one file before the one in use, so it rolls back 1 step, not 2", "GB-379"]);
    let o = w.binary(&["plan", "rollback", "tool", "--site", "test-site", "--out", w.root.path().join("p.json").to_str().unwrap()]);
    assert!(text(&o).contains("GB-379"), "{}", text(&o));
    assert_eq!(w.calls(), "");
}

/// `release.bundle: attestation`: no signature over SHA256SUMS, an attestation of
/// each file. Both avenues verify it before any host and deploy it.
#[test]
fn an_attested_release_is_verified_by_each_files_attestation_on_both_avenues() {
    let w = World::new();
    let r = w.root.path();
    let at = r.join("mirror").join(REPO).join(TAG);
    std::fs::remove_file(at.join("SHA256SUMS.cosign-bundle")).unwrap();
    let s = sigstore::Sigstore::new();
    std::fs::write(r.join("trusted_root.json"), s.trusted_root()).unwrap();
    let signed = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
    let a: serde_json::Value = serde_json::from_str(&s.attestation_v03(&[("tool-linux-amd64", b"amd64 build\n"), ("tool-linux-arm64", b"arm64 build\n")], &signed)).unwrap();
    for f in ["tool-linux-amd64", "tool-linux-arm64"] {
        std::fs::write(at.join(format!("{f}.attestations.json")), serde_json::json!({"attestations": [{"bundle": a}]}).to_string()).unwrap();
    }
    let table = w.repo.join("ops/pins/fleet-pins.yaml");
    let t = std::fs::read_to_string(&table).unwrap().replace("      checksums: SHA256SUMS\n", "      checksums: SHA256SUMS\n      bundle: attestation\n");
    std::fs::write(&table, t).unwrap();
    signed_intent::prepare(&w.repo, &["test_hosts"]);
    signed_intent::sign(&w.repo);
    says(&w.deploy(&["--dry-run"]), [0, 0], &["release verified: tool-linux-amd64 (amd64) sha256", "@refs/tags/tool/v1.2.0"]);
    says(&w.deploy(&[]), [0, 0], &["installed file tool-linux-amd64 is the pin's"]);
    // An attestation of another build of the file is refused before any host, in the same words.
    let other: serde_json::Value = serde_json::from_str(&s.attestation_v03(&[("tool-linux-amd64", b"other build\n")], &signed)).unwrap();
    std::fs::write(at.join("tool-linux-amd64.attestations.json"), serde_json::json!({"attestations": [{"bundle": other}]}).to_string()).unwrap();
    std::fs::remove_dir_all(r.join("home/.goat/state/releases")).unwrap();
    let before = w.calls();
    let outs = w.deploy(&[]);
    says(&outs, [1, 1], &["refused before any host: tool's release", "(check: signature)"]);
    assert_eq!(refusal(&text(&outs[0])), refusal(&text(&outs[1])));
    assert_eq!(w.calls().lines().filter(|l| l.contains("ansible-preflight.sh")).count(), before.lines().filter(|l| l.contains("ansible-preflight.sh")).count());
}

/// The release's archive carries its play (`release.play`): both avenues verify
/// it, unpack its play and run it through release-archive-play.yml, rollback too,
/// and neither runs another play when that wrapper is missing.
#[test]
fn a_release_that_carries_its_play_runs_that_play_on_both_avenues() {
    let w = World::new();
    let r = w.root.path();
    let play = "- name: tool\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n";
    let at = r.join("mirror").join(REPO).join(TAG);
    let make = "import io,sys,tarfile\nt=tarfile.open(sys.argv[1],'w:gz')\nfor n,c in (('tool-1.2.0/',None),('tool-1.2.0/bin/tool','tool\\n'),('tool-1.2.0/deploy/play.yml',sys.argv[2])):\n i=tarfile.TarInfo(n.rstrip('/'))\n if c is None: i.type=tarfile.DIRTYPE; t.addfile(i)\n else: b=c.encode(); i.size=len(b); t.addfile(i,io.BytesIO(b))\nt.close()";
    assert!(Command::new("python3").args(["-I", "-c", make]).arg(at.join("tool.tar.gz")).arg(play).status().unwrap().success());
    let archive = sha256(&std::fs::read(at.join("tool.tar.gz")).unwrap());
    let sums = format!("{archive}  tool.tar.gz\n");
    std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
    let s = sigstore::Sigstore::new();
    let signed = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
    std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &signed)).unwrap();
    std::fs::write(r.join("trusted_root.json"), s.trusted_root()).unwrap();
    let pin = format!("{REPO}@{TAG}@sha256:{}", sha256(sums.as_bytes()));
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible/release-archive-play.yml");
    std::fs::write(w.repo.join("ops/ansible/release-archive-play.yml"), std::fs::read(&wrapper).unwrap()).unwrap();
    std::fs::write(
        w.repo.join("ops/pins/fleet-pins.yaml"),
        // Opt-in: only test_site lists it; test_other does not run it.
        format!("goatnets:\n  test_site:\n    members: [test_hosts]\n    opt_in: [tool]\n  test_other:\n    members: [other_hosts]\ncomponents:\n  tool:\n    opt_in: true\n    var: tool_file\n    artifact_type: release\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: tool.tar.gz, arm64: tool.tar.gz}}\n      play: deploy/play.yml\n      signer: {SIGNER}\n"),
    )
    .unwrap();
    std::fs::write(w.repo.join(PINS), format!("tool_file: \"{pin}\"\n")).unwrap();
    std::fs::write(r.join("stub/row"), format!("{{\"ts\": \"2026-11-24T00:00:00Z\", \"action\": \"deploy\", \"ref\": \"{pin}\"}}")).unwrap();
    signed_intent::prepare(&w.repo, &["test_hosts", "other_hosts"]);
    signed_intent::sign(&w.repo);
    w.host("x86_64", &archive);
    // The goatnet that did not opt in has no pin for it, and neither avenue deploys it there.
    assert!(!w.repo.join("ops/ansible/group_vars/other_hosts/pins.yml").exists() || !std::fs::read_to_string(w.repo.join("ops/ansible/group_vars/other_hosts/pins.yml")).unwrap().contains("tool_file"));
    let other = [
        w.binary(&["deploy", "--env", "test_other", "--component", "tool", "--ack", "A Person", "--yes", "--dry-run"]),
        w.script("deploy-component.sh", &["--goatnet", "test_other", "--dry-run"]),
    ];
    for o in &other {
        assert_ne!(o.status.code(), Some(0), "{}", text(o));
        assert!(text(o).contains("it is opt-in, and the goatnet does not list it under opt_in:"), "{}", text(o));
    }
    assert_eq!(w.calls(), "", "no host for a goatnet that did not opt in");

    says(&w.deploy(&[]), [0, 0], &["installed file tool.tar.gz is the pin's"]);
    let kept = r.join("home/.goat/state/releases/tool").join(sha256(sums.as_bytes()));
    let vars: Vec<PathBuf> = std::fs::read_dir(&kept).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("deploy-vars-")).collect();
    assert_eq!(vars.len(), 1, "one checkout, one file of variables: {vars:?}");
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&vars[0]).unwrap()).unwrap();
    let unpacked = PathBuf::from(doc["release_play"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(&unpacked).unwrap(), play);
    assert!(unpacked.to_string_lossy().contains(&format!("/release-plays/tool/{archive}-")), "{}", unpacked.display());
    let plays: Vec<String> = w.calls().lines().filter(|l| l.contains("ansible-preflight.sh")).map(String::from).collect();
    assert_eq!(plays.len(), 2, "{plays:?}");
    for p in &plays {
        assert!(p.ends_with(&format!("release-archive-play.yml -e target=test_hosts -e @{}", vars[0].display())), "{p}");
    }

    // A rollback runs the pin's own play, verified again, its path one value.
    std::fs::write(r.join("stub/row"), "{\"ts\": \"2026-11-24T01:00:00Z\", \"action\": \"rollback\", \"ref\": \"a\", \"prior_ref\": \"b\"}").unwrap();
    for i in 0..2 {
        let _ = std::fs::remove_file(r.join("stub/played"));
        let o = if i == 0 { w.binary(&["rollback", "tool", "1", "--site", "test-site", "--yes"]) } else { w.script("rollback.sh", &["1"]) };
        assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    }
    let rolls: Vec<String> = w.calls().lines().filter(|l| l.contains("release_file_rollback=true")).map(String::from).collect();
    assert_eq!(rolls.len(), 2, "{rolls:?}");
    for p in &rolls {
        assert!(p.contains("release-archive-play.yml -e target=test_hosts"), "{p}");
        let json = &p[p.find("release_file_rollback=true -e ").unwrap() + "release_file_rollback=true -e ".len()..];
        let v: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
        assert_eq!(v["release_play"].as_str(), unpacked.to_str(), "{p}");
    }

    // With no wrapper, neither avenue runs another play in its place.
    std::fs::remove_file(w.repo.join("ops/ansible/release-archive-play.yml")).unwrap();
    std::fs::write(w.repo.join("ops/ansible/tool-deploy.yml"), play).unwrap();
    let before = w.calls();
    says(&w.deploy(&[]), [2, 1], &["tool's play is in its release, and ops/ansible/release-archive-play.yml is not here"]);
    assert_eq!(w.calls().lines().filter(|l| l.contains("ansible-preflight.sh")).count(), before.lines().filter(|l| l.contains("ansible-preflight.sh")).count());
}

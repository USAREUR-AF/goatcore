//! Gate row B2, fourth slice: `goatbuilder federate test <partner> --bilateral`
//! runs phase 14 on a session both sides derive, with goatbuilder's own signer
//! and verifier (`goatbuilder acceptance-result sign|verify`, hidden) and the
//! peering's keyring. Without the flag, phase 14 is as it was.
//! docs/design/federation-acceptance.md §6.3.

mod federate_world;

use federate_world::{bin, says, world};

fn phase_14_command(out: &str) -> String {
    out.lines()
        .find(|l| l.contains("acceptance.sh"))
        .unwrap_or_else(|| panic!("no phase 14 command in: {out}"))
        .to_string()
}

#[test]
fn with_bilateral_phase_14_gets_goatbuilders_signer_verifier_and_the_keyring() {
    let w = world();
    let (out, _) = w.run(&["test", "hello-ally", "--bilateral", "--dry-run"]);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{}", says(&out));
    let cmd = phase_14_command(&text);
    assert!(cmd.contains("FED_ACCEPTANCE_ATTEMPT=1"), "{cmd}");
    assert!(cmd.contains("acceptance-result sign"), "{cmd}");
    assert!(cmd.contains("acceptance-result verify"), "{cmd}");
    assert!(
        cmd.contains(&format!("FED_KEYRING='{}/keys.txt'", w.work.display())),
        "{cmd}"
    );
}

#[test]
fn bilateral_takes_the_attempt_number() {
    let w = world();
    let (out, _) = w.run(&["test", "hello-ally", "--bilateral", "2", "--dry-run"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(phase_14_command(&String::from_utf8_lossy(&out.stdout))
        .contains("FED_ACCEPTANCE_ATTEMPT=2"));
}

#[test]
fn without_bilateral_phase_14_is_unchanged() {
    let w = world();
    let (out, _) = w.run(&["test", "hello-ally", "--dry-run"]);
    assert!(out.status.success(), "{}", says(&out));
    let cmd = phase_14_command(&String::from_utf8_lossy(&out.stdout));
    assert!(
        !cmd.contains("FED_ACCEPTANCE"),
        "phase 14 gains nothing without the flag: {cmd}"
    );
    assert!(!cmd.contains("FED_KEYRING"), "{cmd}");
}

#[test]
fn bilateral_belongs_to_federate_test_only() {
    let w = world();
    let (out, _) = w.run(&["hello-ally", "--bilateral", "--dry-run"]);
    assert_eq!(out.status.code(), Some(2), "{}", says(&out));
    assert!(
        says(&out).contains("--bilateral belongs to `goatbuilder federate test <partner>`"),
        "{}",
        says(&out)
    );
    assert!(says(&out).contains("GB-189"), "{}", says(&out));
}

/// The two commands phase 14 is given are real: signed by one side's seed,
/// verified by the other side under its keyring, through the binary itself.
#[test]
fn the_hidden_commands_sign_and_verify_through_the_binary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let keys = root.join("ops/federation/vectors/testkeys");
    let data = root.join("crates/goat-federation/src/testdata/acceptance");
    let dir = tempfile::tempdir().unwrap();
    let ring = dir.path().join("keys.txt");
    let line = |s: &str| {
        format!(
            "{s}={}",
            std::fs::read_to_string(keys.join(format!("{s}.public.b64")))
                .unwrap()
                .trim()
        )
    };
    std::fs::write(&ring, format!("{}\n{}\n", line("xaa"), line("xab"))).unwrap();
    let common = [
        "--agreement-sha256",
        "3d2bd7e59ac1d1cdb4fe6a7b4e3a85f0cba0ddd04a4ab1a63e64ab1a8b7b0f5d",
        "--session",
        "ses-xaa-xab-20261109-1",
        "--attempt",
        "1",
    ];
    let result = data.join("result-xab.json");
    let sign = std::process::Command::new(bin())
        .args(["acceptance-result", "sign"])
        .args(common)
        .args([
            "--by",
            "xab",
            "--for",
            "xaa",
            "--echoes",
            "6f1c2a9e4b7d03e5a8c1f2b3d4e5f607",
        ])
        .arg("--result")
        .arg(&result)
        .env_clear()
        .env("FED_SIGNING_KEY", keys.join("xab.seed.hex"))
        .output()
        .unwrap();
    assert!(
        sign.status.success(),
        "{}",
        String::from_utf8_lossy(&sign.stderr)
    );
    let committed = std::fs::read_to_string(data.join("signature-xab.json")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&sign.stdout).trim_end(),
        committed.trim_end()
    );
    let sig = dir.path().join("signature.json");
    std::fs::write(&sig, &sign.stdout).unwrap();
    let verify = |challenge: &str| {
        std::process::Command::new(bin())
            .args(["acceptance-result", "verify"])
            .args(common)
            .args(["--us", "xaa", "--them", "xab", "--our-challenge", challenge])
            .arg("--result")
            .arg(&result)
            .arg("--signature")
            .arg(&sig)
            .env_clear()
            .env("FED_KEYRING", &ring)
            .output()
            .unwrap()
    };
    let ok = verify("6f1c2a9e4b7d03e5a8c1f2b3d4e5f607");
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert_eq!(
        v["key_b64"],
        std::fs::read_to_string(keys.join("xab.public.b64"))
            .unwrap()
            .trim()
    );
    let stale = verify("0f1e2d3c4b5a69788796a5b4c3d2e1f0");
    assert_eq!(stale.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&stale.stderr).starts_with("GB-273: "),
        "{}",
        String::from_utf8_lossy(&stale.stderr)
    );
}

#[test]
fn the_hidden_commands_are_not_in_the_help() {
    let help = std::process::Command::new(bin())
        .arg("--help")
        .env_clear()
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&help.stdout).contains("acceptance-result"));
}

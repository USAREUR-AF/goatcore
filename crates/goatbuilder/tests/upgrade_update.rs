//! Gate row E3 (docs/design/goatbuilder-v1-gate.md, "Upgrading is tested"):
//! `goatbuilder update` moves a 0.x install to the build under test and back.
//!
//! An install is a copy of the released 0.5.1 binary in its own directory,
//! with its own HOME. A download surface served from this process on
//! 127.0.0.1 (version.json, the platform binary, its .sha256 and SHA256SUMS,
//! the layout the goat-get role publishes) is named by GOATBUILDER_UPDATE_URL.
//!   1. There: the surface serves the build under test. The released copy
//!      runs `update`; afterwards the installed file is the build under test,
//!      byte for byte, and runs.
//!   2. Back: the surface serves 0.5.1. The updated copy runs `update`; an
//!      older served version is refused as "nothing to do" and the file is
//!      untouched, then `update --force` installs 0.5.1 byte for byte.
//!
//! When the build under test is not newer than 0.5.1 by semver precedence
//! (main reports the released version until it is bumped), step 1 checks that
//! a plain `update` changes nothing, and then installs with `--force`. PATH is
//! /usr/bin:/bin, so no cosign answers: the signature check reports it was not
//! run, and the sha256 and SHA256SUMS checks still apply.
//!
//! The released binary comes from `$GOATBUILDER_UPGRADE_DIR`, as for E1.

#[path = "upgrade/released.rs"]
mod released;

mod federate_world;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

fn sha256(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// A surface on an ephemeral port serving `version` and `bin`; anything else
/// is a 404. Returns the base URL.
fn surface(version: &str, bin: Vec<u8>) -> String {
    let sum = sha256(&bin);
    surface_claiming(version, bin, &sum)
}

/// A surface that serves `bin` but names `sum` as its SHA-256, in the
/// `.sha256` and in SHA256SUMS: a binary that is not the one the surface
/// vouches for.
fn surface_claiming(version: &str, bin: Vec<u8>, sum: &str) -> String {
    let asset = released::asset();
    let routes: Vec<(String, Vec<u8>)> = vec![
        (
            "/goatbuilder/version.json".into(),
            format!("{{\"version\":\"{version}\",\"tag\":\"goatbuilder/v{version}\"}}")
                .into_bytes(),
        ),
        (format!("/goatbuilder/{asset}"), bin),
        (
            format!("/goatbuilder/{asset}.sha256"),
            format!("{sum}  {asset}\n").into_bytes(),
        ),
        (
            "/goatbuilder/SHA256SUMS".into(),
            format!("{sum}  {asset}\n").into_bytes(),
        ),
    ];
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, b)) => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            b.len()
                        );
                        let _ = s.write_all(b);
                    }
                    None => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                }
            });
        }
    });
    format!("http://{addr}/goatbuilder")
}

struct Install {
    _dir: tempfile::TempDir,
    exe: PathBuf,
    home: PathBuf,
}

fn install_of(bin: &Path) -> Install {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("bin/goatbuilder");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(bin, &exe).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Install {
        _dir: dir,
        exe,
        home,
    }
}

fn run(i: &Install, url: &str, args: &[&str]) -> Output {
    Command::new(&i.exe)
        .args(args)
        .env("GOATBUILDER_UPDATE_URL", url)
        .env("HOME", &i.home)
        .env("PATH", "/usr/bin:/bin")
        .env("GOAT_LANG", "en")
        .env("GOAT_NO_FUN", "1")
        .env_remove("GOAT_EXPLAIN")
        .current_dir(&i.home)
        .output()
        .unwrap()
}

fn says(o: &Output) -> String {
    format!(
        "exit {:?}\n{}{}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The version the installed binary reports (`goatbuilder 0.5.1` → `0.5.1`).
fn version_of(i: &Install) -> String {
    let o = run(i, "http://127.0.0.1:9/goatbuilder", &["--version"]);
    assert!(
        o.status.success(),
        "the installed binary does not run: {}",
        says(&o)
    );
    String::from_utf8_lossy(&o.stdout)
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_string()
}

/// Print a step and its whole output (shown with `--nocapture`, kept by CI's log).
fn step(what: &str, o: &Output) {
    println!("=== {what}\n{}", says(o));
}

fn installed(i: &Install) -> Vec<u8> {
    std::fs::read(&i.exe).unwrap()
}

#[test]
fn e3_update_moves_a_0_5_1_install_to_this_build_and_back() {
    let Some(old_bin) = released::binary("v0.5.1") else {
        return;
    };
    let old = std::fs::read(&old_bin).unwrap();
    let new = std::fs::read(env!("CARGO_BIN_EXE_goatbuilder")).unwrap();
    assert_ne!(old, new, "the build under test is the released binary");
    let i = install_of(&old_bin);
    let (old_v, new_v) = (version_of(&i), {
        let probe = install_of(Path::new(env!("CARGO_BIN_EXE_goatbuilder")));
        version_of(&probe)
    });
    assert_eq!(old_v, "0.5.1", "the released copy reports {old_v}");

    // 1. There.
    let there = surface(&new_v, new.clone());
    if !goat_selfupdate::is_outdated(&old_v, &new_v) {
        let o = run(&i, &there, &["update"]);
        step(
            "there: 0.5.1 runs `update`, served a build that is not newer",
            &o,
        );
        assert_eq!(o.status.code(), Some(0), "{}", says(&o));
        assert_eq!(
            installed(&i),
            old,
            "a plain update to a version that is not newer installed it"
        );
        let o = run(&i, &there, &["update", "--force"]);
        step("there: 0.5.1 runs `update --force`", &o);
        assert_eq!(
            o.status.code(),
            Some(0),
            "update --force to the build under test: {}",
            says(&o)
        );
    } else {
        let o = run(&i, &there, &["update"]);
        step(
            "there: 0.5.1 runs a plain `update`, served the newer build under test",
            &o,
        );
        assert_eq!(
            o.status.code(),
            Some(0),
            "update to the build under test: {}",
            says(&o)
        );
    }
    assert!(
        installed(&i) == new,
        "the install is not the build under test after the update"
    );
    assert_eq!(version_of(&i), new_v);

    // 2. Back: an older served version needs --force.
    let back = surface(&old_v, old.clone());
    if old_v != new_v {
        let o = run(&i, &back, &["update"]);
        step(
            "back: the build under test runs a plain `update`, served the older 0.5.1",
            &o,
        );
        assert_eq!(o.status.code(), Some(0), "{}", says(&o));
        assert!(
            String::from_utf8_lossy(&o.stdout).contains("--force")
                || String::from_utf8_lossy(&o.stderr).contains("--force"),
            "an older served version is not named as needing --force: {}",
            says(&o)
        );
        assert!(
            installed(&i) == new,
            "a plain update installed an older version"
        );
    }
    let o = run(&i, &back, &["update", "--force"]);
    step(
        "back: the build under test runs `update --force`, served 0.5.1",
        &o,
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "update --force back to 0.5.1: {}",
        says(&o)
    );
    assert!(
        installed(&i) == old,
        "the install is not 0.5.1 after going back"
    );
    assert_eq!(
        sha256(&installed(&i)),
        released::pin("v0.5.1", &released::asset()).unwrap()
    );
    assert_eq!(version_of(&i), "0.5.1");
}

/// After going back, a run journal that a later build wrote can name a phase
/// outcome 0.5.1 does not know (0.6 adds "waiting" and "incomplete" for phase
/// 14). Measured here: 0.5.1 refuses it with GB-182, for a run, a resume and
/// `status`, and leaves the file byte for byte as it was. It does not
/// overwrite the only account of what ran. The run is stranded on 0.5.1 until
/// the later build is installed again or the file is moved aside; that is the
/// case the rule "a reader accepts outcomes from a later version" is for.
#[test]
fn e3_back_on_0_5_1_a_journal_from_a_later_build_is_refused_and_kept() {
    let Some(old) = released::binary("v0.5.1") else {
        return;
    };
    let new = Path::new(federate_world::bin());
    let run_as = |bin: &Path, w: &federate_world::World, extra: &[&str]| -> Output {
        let c = w.command(extra);
        let mut o = Command::new(bin);
        o.args(c.get_args()).env_clear();
        for (k, v) in c.get_envs() {
            if let Some(v) = v {
                o.env(k, v);
            }
        }
        if let Some(d) = c.get_current_dir() {
            o.current_dir(d);
        }
        o.stdin(std::process::Stdio::null()).output().unwrap()
    };
    for word in ["waiting", "incomplete"] {
        let w = federate_world::world();
        run_as(new, &w, &["hello-ally"]);
        std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
        run_as(new, &w, &["hello-ally", "--from", "6"]);
        let o = run_as(new, &w, &["hello-ally", "--from", "11"]);
        assert!(o.status.success(), "{}", says(&o));
        // The journal as a later build leaves it: phase 14 `word`.
        let path = w.work.join("state.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["phases"]["14"]["outcome"] = serde_json::json!(word);
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let kept = std::fs::read(&path).unwrap();
        for args in [vec!["hello-ally", "--from", "14"], vec!["hello-ally"]] {
            let o = run_as(&old, &w, &args);
            assert_ne!(
                o.status.code(),
                Some(0),
                "0.5.1 {args:?} took a journal it cannot read: {}",
                says(&o)
            );
            let text = says(&o);
            assert!(
                text.contains("GB-182") && text.contains(&format!("unknown variant `{word}`")),
                "0.5.1 {args:?} on phase 14 {word}: {text}"
            );
        }
        let o = run_as(&old, &w, &["status", "hello-ally"]);
        assert!(
            says(&o).contains("GB-182"),
            "0.5.1 status on phase 14 {word}: {}",
            says(&o)
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            kept,
            "0.5.1 rewrote a journal it cannot read"
        );
    }
}

/// A surface whose binary is not the one its checksums vouch for: `update`
/// refuses it with a checksum mismatch, with and without `--force`, and the
/// install stays byte for byte the released 0.5.1.
#[test]
fn e3_a_binary_that_does_not_match_its_checksum_is_refused_and_the_install_kept() {
    let Some(old_bin) = released::binary("v0.5.1") else {
        return;
    };
    let old = std::fs::read(&old_bin).unwrap();
    let new = std::fs::read(env!("CARGO_BIN_EXE_goatbuilder")).unwrap();
    let i = install_of(&old_bin);
    // The served bytes are the build under test; the checksums name 0.5.1's.
    let tampered = surface_claiming("99.0.0", new, &sha256(&old));
    for args in [vec!["update"], vec!["update", "--force"]] {
        let o = run(&i, &tampered, &args);
        step(&format!("tampered: 0.5.1 runs `{}`", args.join(" ")), &o);
        assert_ne!(
            o.status.code(),
            Some(0),
            "{args:?} installed a binary that does not match its checksum: {}",
            says(&o)
        );
        assert!(
            says(&o).contains("checksum mismatch"),
            "{args:?}: {}",
            says(&o)
        );
        assert!(
            installed(&i) == old,
            "{args:?} changed the install although it refused"
        );
    }
    assert_eq!(version_of(&i), "0.5.1");
}

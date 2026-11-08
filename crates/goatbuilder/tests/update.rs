// `goatbuilder update` against a download surface served from this process:
// version.json, the platform binary and its .sha256 over plain HTTP on
// 127.0.0.1, named by GOATBUILDER_UPDATE_URL. Each case runs a COPY of the
// built binary, so the binary it replaces is the copy, and HOME is a temp dir,
// so the remembered surface is the test's own.
//
// Cases: an update applied (the copy then runs as the served release), already
// current, a checksum mismatch (refused, copy byte-for-byte intact), a .sha256
// that SHA256SUMS contradicts, the signature over SHA256SUMS verified or
// refused (a stand-in `cosign` on PATH), a stalled download (refused inside
// the stall bound, copy intact), an unreachable surface, and `--check`. PATH is
// /usr/bin:/bin unless a case puts its own cosign first, so a cosign installed
// on the machine running the tests is never what answers.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

#[derive(Clone)]
enum Body {
    Full(Vec<u8>),
    /// Headers promising `len` bytes, `sent` of them, then silence.
    Stall { len: usize, sent: Vec<u8> },
}

/// A surface on an ephemeral port: path → body; anything else is a 404.
fn serve(routes: Vec<(String, Body)>) -> String {
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
                match routes.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone()) {
                    Some(Body::Full(b)) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                        let _ = s.write_all(&b);
                    }
                    Some(Body::Stall { len, sent }) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
                        let _ = s.write_all(&sent);
                        let _ = s.flush();
                        std::thread::sleep(Duration::from_secs(90));
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    format!("http://{addr}/goatbuilder")
}

fn asset() -> String {
    let p = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-amd64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "linux-amd64",
    };
    format!("goatbuilder-{p}")
}

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(b))
}

/// The served "release": a script that answers --version as 9.9.9.
fn release() -> Vec<u8> {
    b"#!/bin/sh\necho 'goatbuilder 9.9.9'\n".to_vec()
}

fn surface(version: &str, bin: Body, sha: &str) -> String {
    surface_with_sums(version, bin, sha, sha)
}

/// `sha` in `<asset>.sha256`, `sums` for the asset's line in SHA256SUMS.
fn surface_with_sums(version: &str, bin: Body, sha: &str, sums: &str) -> String {
    serve(vec![
        ("/goatbuilder/version.json".into(), Body::Full(format!("{{\"version\":\"{version}\",\"tag\":\"goatbuilder/v{version}\"}}").into_bytes())),
        (format!("/goatbuilder/{}", asset()), bin),
        (format!("/goatbuilder/{}.sha256", asset()), Body::Full(format!("{sha}  {}\n", asset()).into_bytes())),
        ("/goatbuilder/SHA256SUMS".into(), Body::Full(format!("{}  goatbuilder-docs.tar.gz\n{sums}  {}\n", "0".repeat(64), asset()).into_bytes())),
        ("/goatbuilder/SHA256SUMS.cosign-bundle".into(), Body::Full(b"{\"bundle\":\"stand-in\"}".to_vec())),
    ])
}

/// A directory holding a stand-in `cosign` that exits `code`, for PATH.
fn fake_cosign(c: &Copy, code: i32) -> String {
    let dir = c.home.join("fakebin");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("cosign");
    std::fs::write(&p, format!("#!/bin/sh\necho \"stand-in cosign: $*\" >&2\nexit {code}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    format!("{}:/usr/bin:/bin", dir.display())
}

struct Copy {
    _dir: tempfile::TempDir,
    exe: PathBuf,
    home: PathBuf,
}

fn copy() -> Copy {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("bin/goatbuilder");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_goatbuilder"), &exe).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Copy { _dir: dir, exe, home }
}

fn run(c: &Copy, url: &str, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(&c.exe);
    cmd.args(args).env("GOATBUILDER_UPDATE_URL", url).env("HOME", &c.home).env("PATH", "/usr/bin:/bin").env("GOAT_LANG", "en").env_remove("GOAT_EXPLAIN").current_dir(c.home.as_path());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// A goatbuilder.yaml in the working directory naming the release signer.
fn signer_config(c: &Copy, release_source: &str) {
    let dir = c.home.join("ops/goatbuilder");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("goatbuilder.yaml"),
        format!(
            "schema: goatbuilder/v1\npreset: lite\nrepo:\n  source: example-org/declarations\n  image_namespace: ghcr.io/example-org\n  release_source: {release_source}\ncomponents:\n  portal:\n    declared_in: declared/pins.yaml\nenvironments:\n  - name: dev\n    converger: push-ansible\npromotion:\n  path:\n    - name: dev\n      kind: rung\n      environment: dev\n"
        ),
    )
    .unwrap();
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

#[test]
fn an_update_replaces_the_binary_with_the_verified_release() {
    let c = copy();
    let url = surface("9.9.9", Body::Full(release()), &sha256(&release()));
    let o = run(&c, &url, &["update"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("ok    checksum") && t.contains("ok    install") && t.contains("→ 9.9.9"), "{t}");
    assert!(t.contains("signature not checked: cosign is not on PATH"), "without cosign the row says so: {t}");
    assert_eq!(bytes(&c.exe), release());
    let v = Command::new(&c.exe).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), "goatbuilder 9.9.9");
    assert!(std::fs::read_dir(c.exe.parent().unwrap()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")), "no temp file is left");
    let remembered = std::fs::read_to_string(c.home.join(".goatbuilder/update-check.json")).unwrap();
    assert!(remembered.contains(&url) && remembered.contains("9.9.9"), "{remembered}");
}

#[test]
fn already_current_is_one_row_and_exit_zero() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface(env!("CARGO_PKG_VERSION"), Body::Full(release()), &sha256(&release()));
    let o = run(&c, &url, &["update"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("already up to date"), "{}", text(&o));
    assert_eq!(bytes(&c.exe), before, "nothing installed");
}

#[test]
fn a_checksum_mismatch_is_refused_and_the_binary_is_intact() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface("9.9.9", Body::Full(release()), &sha256(b"another build"));
    let o = run(&c, &url, &["update"], &[]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("checksum mismatch, refusing to install") && t.contains("GB-074"), "{t}");
    assert_eq!(bytes(&c.exe), before, "the installed binary is byte-for-byte the old one");
}

#[test]
fn a_stalled_download_is_refused_inside_the_stall_bound() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface("9.9.9", Body::Stall { len: 100_000, sent: vec![b'x'; 10] }, &sha256(&release()));
    let t0 = Instant::now();
    let o = run(&c, &url, &["update"], &[("GOATBUILDER_UPDATE_STALL_SECS", "1")]);
    let took = t0.elapsed();
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("stalled or broke") && t.contains("GB-073"), "{t}");
    assert!(took < Duration::from_secs(45), "a stall of 1 s ends the run long before the server's 90 s of silence: {took:?}");
    assert_eq!(bytes(&c.exe), before);
}

#[test]
fn check_reads_only_and_exits_three_when_behind() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface("9.9.9", Body::Full(release()), &sha256(&release()));
    let o = run(&c, &url, &["-o", "json", "update", "--check"], &[]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["verb"], "update");
    assert_eq!(v["checks"][1]["ok"], false);
    assert_eq!(v["checks"][1]["fix"], "goatbuilder update");
    assert_eq!(bytes(&c.exe), before);
}

#[test]
fn an_unreachable_surface_is_gb_073_and_changes_nothing() {
    let c = copy();
    let before = bytes(&c.exe);
    // A port that was free a moment ago: connection refused.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let o = run(&c, &format!("http://127.0.0.1:{port}/goatbuilder"), &["-o", "json", "update"], &[]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["code"], "GB-073");
    assert_eq!(bytes(&c.exe), before);
}

#[test]
fn a_sha256_that_sha256sums_contradicts_is_refused() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface_with_sums("9.9.9", Body::Full(release()), &sha256(&release()), &sha256(b"another build"));
    let o = run(&c, &url, &["update"], &[]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("SHA256SUMS says") && t.contains("GB-074"), "{t}");
    assert_eq!(bytes(&c.exe), before);
}

#[test]
fn a_verified_signature_installs_and_a_refused_one_does_not() {
    let c = copy();
    // The signer comes from the deployment's config, never from the code.
    signer_config(&c, "example-org/releases");
    let before = bytes(&c.exe);
    let url = surface("9.9.9", Body::Full(release()), &sha256(&release()));
    let refused = fake_cosign(&c, 1);
    let o = run(&c, &url, &["update"], &[("PATH", &refused)]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("signature refused") && t.contains("--certificate-identity-regexp") && t.contains("GB-074"), "{t}");
    assert_eq!(bytes(&c.exe), before, "a refused signature installs nothing");

    let verified = fake_cosign(&c, 0);
    let o = run(&c, &url, &["update"], &[("PATH", &verified)]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("ok    signature  SHA256SUMS signed by the example-org/releases release workflow"),
        "{}",
        text(&o)
    );
    assert_eq!(bytes(&c.exe), release());
}

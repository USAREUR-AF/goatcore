// `goatbuilder update` against a download surface served from this process:
// version.json, the platform binary and its .sha256 over plain HTTP on
// 127.0.0.1, named by GOATBUILDER_UPDATE_URL. Each case runs a COPY of the
// built binary, so the binary it replaces is the copy, and HOME is a temp dir,
// so the remembered surface is the test's own.
//
// Cases: an update applied (the copy then runs as the served release), already
// current, a checksum mismatch (refused, copy byte-for-byte intact), a .sha256
// that SHA256SUMS contradicts, the signature over SHA256SUMS verified in
// process or refused (another repository, another workflow, another version,
// a changed SHA256SUMS, none served), a stalled download (refused inside the
// stall bound, copy intact), an unreachable surface, and `--check`.
//
// The signature is a stand-in Sigstore's (tests/support/sigstore.rs): its
// trusted root is given to the copy through GOATBUILDER_UPDATE_TRUSTED_ROOT,
// and the release signer through a goatbuilder.yaml in the working directory.
// PATH is /usr/bin:/bin, and no cosign is needed.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

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

/// The stand-in Sigstore every case's surface is signed by, made once.
fn sigstore() -> &'static sigstore::Sigstore {
    static S: std::sync::OnceLock<sigstore::Sigstore> = std::sync::OnceLock::new();
    S.get_or_init(sigstore::Sigstore::new)
}

/// The release signer the copies are configured with (signer_config).
const SIGNER: &str = "example-org/releases";

/// How a surface's SHA256SUMS is signed.
#[derive(Clone)]
enum Signed {
    /// By the signer's release workflow at the served version's tag.
    AsRelease,
    /// By this workflow identity instead.
    As(sigstore::Workflow),
    /// A bundle over other bytes than the served SHA256SUMS.
    OverOther,
    /// No bundle is served.
    None,
}

/// `sha` in `<asset>.sha256`, `sums` for the asset's line in SHA256SUMS.
fn surface_with_sums(version: &str, bin: Body, sha: &str, sums: &str) -> String {
    surface_signed(version, bin, sha, sums, Signed::AsRelease)
}

fn surface_signed(version: &str, bin: Body, sha: &str, sums: &str, signed: Signed) -> String {
    let sums_text = format!("{}  goatbuilder-docs.tar.gz\n{sums}  {}\n", "0".repeat(64), asset());
    let release = sigstore::Workflow::release(SIGNER, version);
    let bundle = match signed {
        Signed::AsRelease => Some(sigstore().bundle(sums_text.as_bytes(), &release)),
        Signed::As(w) => Some(sigstore().bundle(sums_text.as_bytes(), &w)),
        Signed::OverOther => Some(sigstore().bundle(b"another SHA256SUMS", &release)),
        Signed::None => None,
    };
    let mut routes = vec![
        ("/goatbuilder/version.json".into(), Body::Full(format!("{{\"version\":\"{version}\",\"tag\":\"goatbuilder/v{version}\"}}").into_bytes())),
        (format!("/goatbuilder/{}", asset()), bin),
        (format!("/goatbuilder/{}.sha256", asset()), Body::Full(format!("{sha}  {}\n", asset()).into_bytes())),
        ("/goatbuilder/SHA256SUMS".into(), Body::Full(sums_text.into_bytes())),
    ];
    if let Some(b) = bundle {
        routes.push(("/goatbuilder/SHA256SUMS.cosign-bundle".into(), Body::Full(b.into_bytes())));
    }
    serve(routes)
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
    test_support::copied_executable(std::path::Path::new(env!("CARGO_BIN_EXE_goatbuilder")), &exe);
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("trusted_root.json"), sigstore().trusted_root()).unwrap();
    let c = Copy { _dir: dir, exe, home };
    // The signer comes from the deployment's config, never from the code.
    signer_config(&c, SIGNER);
    c
}

fn run(c: &Copy, url: &str, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(&c.exe);
    cmd.args(args).env("GOATBUILDER_UPDATE_URL", url).env("HOME", &c.home).env("PATH", "/usr/bin:/bin").env("GOAT_LANG", "en").env("GOATBUILDER_UPDATE_TRUSTED_ROOT", c.home.join("trusted_root.json")).env_remove("GOAT_EXPLAIN").current_dir(c.home.as_path());
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
    assert!(
        t.contains("ok    signature  SHA256SUMS signed by https://github.com/example-org/releases/.github/workflows/goatbuilder-release.yml@refs/tags/goatbuilder/v9.9.9"),
        "the signature is verified with no cosign on PATH: {t}"
    );
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

/// The signature over SHA256SUMS is verified in process: one by another
/// repository, another workflow, or for another version; one over other
/// bytes; or none served, is refused with GB-261, and nothing is installed.
/// The release's own installs. No cosign is on PATH.
#[test]
fn a_verified_signature_installs_and_a_refused_one_does_not() {
    let c = copy();
    let before = bytes(&c.exe);
    let w = |repo: &str, workflow: &str, version: &str| sigstore::Workflow {
        repo: repo.into(),
        workflow: workflow.into(),
        tag: format!("goatbuilder/v{version}"),
    };
    for (what, signed, says) in [
        ("another repository", Signed::As(w("example-org/forks", "goatbuilder-release.yml", "9.9.9")), "source repository"),
        ("goat's release workflow", Signed::As(w(SIGNER, "goat-release.yml", "9.9.9")), "Build Config URI"),
        ("another version", Signed::As(w(SIGNER, "goatbuilder-release.yml", "9.9.8")), "source ref"),
        ("other bytes", Signed::OverOther, "the Rekor entry names the hash"),
        ("no bundle", Signed::None, "the surface serves no signature for SHA256SUMS"),
    ] {
        let url = surface_signed("9.9.9", Body::Full(release()), &sha256(&release()), &sha256(&release()), signed);
        let o = run(&c, &url, &["update"], &[]);
        assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
        let t = text(&o);
        assert!(t.contains("GB-261") && t.contains(says), "{what}: {t}");
        assert_eq!(bytes(&c.exe), before, "{what}: a refused signature installs nothing");
    }

    let url = surface("9.9.9", Body::Full(release()), &sha256(&release()));
    let o = run(&c, &url, &["update"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ok    signature  SHA256SUMS signed by"), "{}", text(&o));
    assert_eq!(bytes(&c.exe), release());
}

/// Without the trusted root that holds the signing CA, the refusal names the
/// option to supply one.
#[test]
fn a_signature_from_an_untrusted_ca_names_the_trusted_root_option() {
    let c = copy();
    let before = bytes(&c.exe);
    let url = surface("9.9.9", Body::Full(release()), &sha256(&release()));
    let o = run(&c, &url, &["update"], &[("GOATBUILDER_UPDATE_TRUSTED_ROOT", "")]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("GB-261") && t.contains("GOATBUILDER_UPDATE_TRUSTED_ROOT"), "{t}");
    assert_eq!(bytes(&c.exe), before);
}

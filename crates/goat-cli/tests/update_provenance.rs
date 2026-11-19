//! `goat update` against a local download surface: what must vouch for a
//! binary before it replaces the installed one, and how long and how large a
//! download may be (GHSA-32cp-hjww-7vmj). Every surface here is a loopback
//! server in this process; the cosign on PATH is a stand-in script that
//! accepts exactly one SHA256SUMS signed by exactly one identity.
#![cfg(unix)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone)]
enum Body {
    Bytes(Vec<u8>),
    /// One byte every 200 ms, forever.
    Trickle,
    /// Declares this many bytes, then sends none.
    Declares(u64),
}

/// A loopback HTTP/1.0 surface serving `files` by path; anything else is 404.
fn surface(files: HashMap<String, Body>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let files = files.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                let mut r = BufReader::new(s.try_clone().unwrap());
                if r.read_line(&mut line).is_err() {
                    return;
                }
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                match files.get(&path) {
                    None => {
                        let _ = s.write_all(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    }
                    Some(Body::Bytes(b)) => {
                        let _ = s.write_all(
                            format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", b.len())
                                .as_bytes(),
                        );
                        let _ = s.write_all(b);
                    }
                    Some(Body::Declares(n)) => {
                        let _ = s.write_all(
                            format!("HTTP/1.0 200 OK\r\nContent-Length: {n}\r\n\r\n").as_bytes(),
                        );
                    }
                    Some(Body::Trickle) => {
                        let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
                        while s.write_all(b"x").and_then(|_| s.flush()).is_ok() {
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

fn asset() -> String {
    goat_selfupdate::asset_name("goat").expect("a platform the release builds")
}

fn sums_for(asset: &str, digest: &str) -> String {
    format!("{digest}  {asset}\n")
}

/// A release as the surface mirrors it: the binary, its .sha256, SHA256SUMS
/// and the signature bundle.
fn release(bin: &[u8]) -> HashMap<String, Body> {
    release_as(bin, NEW)
}

/// The same, with version.json advertising `version`.
fn release_as(bin: &[u8], version: &str) -> HashMap<String, Body> {
    let a = asset();
    let d = goat_selfupdate::sha256_hex(bin);
    HashMap::from([
        (
            "/version.json".to_string(),
            Body::Bytes(format!(r#"{{"version":"{version}"}}"#).into_bytes()),
        ),
        (format!("/{a}"), Body::Bytes(bin.to_vec())),
        (
            format!("/{a}.sha256"),
            Body::Bytes(format!("{d}  {a}\n").into_bytes()),
        ),
        (
            "/SHA256SUMS".to_string(),
            Body::Bytes(sums_for(&a, &d).into_bytes()),
        ),
        (
            "/SHA256SUMS.cosign-bundle".to_string(),
            Body::Bytes(b"{\"base64Signature\":\"stand-in\",\"cert\":\"stand-in\"}".to_vec()),
        ),
        (
            "/SHA256SUMS.sigstore.json".to_string(),
            Body::Bytes(
                b"{\"mediaType\":\"application/vnd.dev.sigstore.bundle.v0.3+json\"}".to_vec(),
            ),
        ),
    ])
}

const GOOD: &[u8] = b"#!/bin/sh\necho the signed release\n";
const FORGED: &[u8] = b"#!/bin/sh\necho forged by the surface\n";
const SIGNER: &str = "good-org/goatcore";
/// Newer than any goat this test builds.
const NEW: &str = "99.0.0";
/// Older than any goat this test builds.
const OLD: &str = "0.0.1";
/// The running binary's own version.
const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// The certificate SAN a genuine goat release signature carries: the release
/// workflow, run for the release tag of `version`.
fn release_id(repo: &str, version: &str) -> String {
    format!("https://github.com/{repo}/.github/workflows/_release-binary.yml@refs/tags/goat-cli-v{version}")
}

fn signed_sums(bin: &[u8]) -> String {
    sums_for(&asset(), &goat_selfupdate::sha256_hex(bin))
}

struct Env {
    dir: tempfile::TempDir,
    exe: PathBuf,
    ino: u64,
}

/// A copy of the goat binary under test (a hard link where possible) in its
/// own directory, so the one `goat update` replaces is this test's own. The
/// directory is under the target directory, on the binary's own filesystem, so
/// the link works and a debug binary is not copied once per test.
fn installed() -> Env {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let exe = dir.path().join("bin/goat");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    let src = env!("CARGO_BIN_EXE_goat");
    if std::fs::hard_link(src, &exe).is_err() {
        std::fs::copy(src, &exe).unwrap();
    }
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::create_dir_all(dir.path().join("path")).unwrap();
    let ino = std::fs::metadata(&exe).unwrap().ino();
    Env { dir, exe, ino }
}

impl Env {
    /// A cosign stand-in on this env's PATH: verify-blob succeeds only for
    /// `signed` (the exact SHA256SUMS bytes) under `signer`'s release identity.
    /// A cosign stand-in that behaves like cosign v3: `identity` is the
    /// certificate SAN its "signature" was made under, and `signed` the exact
    /// SHA256SUMS it covers. It verifies when `--certificate-identity` equals
    /// that SAN (or `--certificate-identity-regexp` matches it), the bundle is
    /// in the format the flags say (`--new-bundle-format`: a Sigstore bundle,
    /// which carries a mediaType; otherwise the legacy cosign bundle), and the
    /// blob is `signed`. Like cosign v3 it refuses `--trusted-root` without
    /// `--new-bundle-format`. Every call's arguments are appended to
    /// cosign.args; each success writes the format it verified to
    /// cosign.verified.
    fn cosign(&self, signed: &str, identity: &str) {
        self.cosign_as(signed, identity, false)
    }

    /// `old`: a cosign that predates the Sigstore bundle format
    /// (`unknown flag: --new-bundle-format`).
    fn cosign_as(&self, signed: &str, identity: &str, old: bool) {
        let sums = self.dir.path().join("signed-SHA256SUMS");
        std::fs::write(&sums, signed).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             {{ printf '%s\\n' \"$@\"; echo ==; }} >> '{args}'\n\
             newfmt=0; tr=''; bundle=''; mode=''; idv=''; last=''\n\
             while [ $# -gt 0 ]; do case \"$1\" in\n\
               verify-blob) ;;\n\
               --new-bundle-format) [ {old} = 1 ] && {{ echo 'Error: unknown flag: --new-bundle-format' >&2; exit 1; }}; newfmt=1 ;;\n\
               --bundle) bundle=$2; shift ;;\n\
               --certificate-identity) mode=exact; idv=$2; shift ;;\n\
               --certificate-identity-regexp) mode=re; idv=$2; shift ;;\n\
               --certificate-oidc-issuer) shift ;;\n\
               --trusted-root=*) tr=${{1#--trusted-root=}} ;;\n\
               --trusted-root) echo 'Error: --trusted-root takes =<path> here' >&2; exit 2 ;;\n\
               --offline) ;;\n\
               *) last=$1 ;;\n\
             esac; shift; done\n\
             [ -n \"$tr\" ] && [ $newfmt = 0 ] && {{ echo 'Error: --trusted-root only supported with --new-bundle-format' >&2; exit 1; }}\n\
             [ -s \"$bundle\" ] || {{ echo 'Error: empty bundle' >&2; exit 1; }}\n\
             if /usr/bin/grep -q mediaType \"$bundle\"; then fmt=new; else fmt=legacy; fi\n\
             [ $newfmt = 1 ] && [ $fmt = legacy ] && {{ echo 'Error: bundle: proto: unmarshal: unknown field base64Signature' >&2; exit 1; }}\n\
             [ $newfmt = 0 ] && [ $fmt = new ] && {{ echo 'Error: unmarshal legacy bundle: unknown field mediaType' >&2; exit 1; }}\n\
             case \"$mode\" in\n\
               exact) [ \"$idv\" = '{id}' ] ;;\n\
               re) printf '%s' '{id}' | /usr/bin/grep -Eq -- \"$idv\" ;;\n\
               *) false ;;\n\
             esac || {{ echo \"Error: none of the expected identities matched what was in the certificate, got subjects [{id}]\" >&2; exit 1; }}\n\
             /usr/bin/cmp -s \"$last\" '{sums}' || {{ echo 'Error: invalid signature when validating ASN.1 encoded signature' >&2; exit 1; }}\n\
             echo $fmt > '{verified}'\n\
             echo Verified OK >&2\n",
            args = self.dir.path().join("cosign.args").display(),
            verified = self.dir.path().join("cosign.verified").display(),
            sums = sums.display(),
            id = identity,
            old = u8::from(old),
        );
        let c = self.dir.path().join("path/cosign");
        std::fs::write(&c, script).unwrap();
        std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The bundle format the last successful cosign call verified.
    fn verified(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("cosign.verified"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn cosign_args(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("cosign.args")).unwrap_or_default()
    }

    fn command(&self, base: &str, extra: &[&str]) -> Command {
        let mut c = Command::new(&self.exe);
        c.arg("--profile-dir")
            .arg(self.dir.path().join("profile"))
            .arg("update")
            .args(extra)
            .env_clear()
            .env("HOME", self.dir.path())
            .env("PATH", self.dir.path().join("path"))
            .env("TMPDIR", self.dir.path())
            .env("GOAT_GET_ENDPOINT", base)
            .env("GOAT_NO_VERSION_CHECK", "1")
            .env("GOAT_UPDATE_SIGNER", SIGNER);
        c
    }

    fn update(&self, base: &str, extra: &[&str]) -> Output {
        self.command(base, extra).output().unwrap()
    }

    fn untouched(&self) -> bool {
        std::fs::metadata(&self.exe).unwrap().ino() == self.ino
    }

    fn now_holds(&self, want: &[u8]) -> bool {
        !self.untouched() && std::fs::read(&self.exe).unwrap() == want
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn a_signed_release_installs() {
    let e = installed();
    let files = release(GOOD);
    e.cosign(
        &sums_for(&asset(), &goat_selfupdate::sha256_hex(GOOD)),
        &release_id(SIGNER, NEW),
    );
    let o = e.update(&surface(files), &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.now_holds(GOOD), "{}", text(&o));
    assert!(
        text(&o).contains("signed by the good-org/goatcore release workflow"),
        "{}",
        text(&o)
    );
}

#[test]
fn the_surfaces_own_checksum_alone_is_refused() {
    // No cosign: only the server's own .sha256 vouches for the binary.
    let e = installed();
    let o = e.update(&surface(release(FORGED)), &["--force"]);
    assert!(
        !o.status.success(),
        "installed on a same-origin checksum alone: {}",
        text(&o)
    );
    assert!(e.untouched(), "{}", text(&o));
    assert!(text(&o).contains("cosign is not on PATH"), "{}", text(&o));
}

#[test]
fn allow_unsigned_installs_on_the_checksum_alone_and_says_so() {
    let e = installed();
    let o = e.update(&surface(release(GOOD)), &["--allow-unsigned"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.now_holds(GOOD));
    assert!(text(&o).contains("signature NOT checked"), "{}", text(&o));
}

#[test]
fn an_altered_binary_with_altered_checksums_is_refused_by_the_signature() {
    // The surface rewrites the binary, its .sha256 and SHA256SUMS together; the
    // signature is over the release's original SHA256SUMS.
    let e = installed();
    e.cosign(
        &sums_for(&asset(), &goat_selfupdate::sha256_hex(GOOD)),
        &release_id(SIGNER, NEW),
    );
    let o = e.update(&surface(release(FORGED)), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_release_signed_by_another_identity_is_refused() {
    let e = installed();
    let files = release(FORGED);
    // A genuine signature over exactly these SHA256SUMS, by the wrong repository.
    e.cosign(
        &sums_for(&asset(), &goat_selfupdate::sha256_hex(FORGED)),
        &release_id("evil-org/goatcore", NEW),
    );
    let o = e.update(&surface(files), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_checksum_that_disagrees_with_sha256sums_is_refused() {
    let e = installed();
    let a = asset();
    let mut files = release(GOOD);
    files.insert(format!("/{a}"), Body::Bytes(FORGED.to_vec()));
    files.insert(
        format!("/{a}.sha256"),
        Body::Bytes(format!("{}  {a}\n", goat_selfupdate::sha256_hex(FORGED)).into_bytes()),
    );
    e.cosign(
        &sums_for(&a, &goat_selfupdate::sha256_hex(GOOD)),
        &release_id(SIGNER, NEW),
    );
    let o = e.update(&surface(files), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(text(&o).contains("checksum disagreement"), "{}", text(&o));
}

#[test]
fn a_missing_signature_bundle_is_refused() {
    let e = installed();
    let mut files = release(FORGED);
    files.remove("/SHA256SUMS.cosign-bundle");
    files.remove("/SHA256SUMS.sigstore.json");
    e.cosign(
        &sums_for(&asset(), &goat_selfupdate::sha256_hex(FORGED)),
        &release_id(SIGNER, NEW),
    );
    let o = e.update(&surface(files), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("cannot read the release signature"),
        "{}",
        text(&o)
    );
}

#[test]
fn an_oversized_binary_is_refused_before_it_is_read() {
    let e = installed();
    let a = asset();
    let mut files = release(GOOD);
    files.insert(format!("/{a}"), Body::Declares(600 * 1024 * 1024));
    e.cosign(
        &sums_for(&a, &goat_selfupdate::sha256_hex(GOOD)),
        &release_id(SIGNER, NEW),
    );
    let o = e.update(&surface(files), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("larger than the 536870912-byte limit"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_trickling_download_is_cut_off_at_the_deadline() {
    let e = installed();
    let a = asset();
    let mut files = release(GOOD);
    files.insert(format!("/{a}"), Body::Trickle);
    e.cosign(
        &sums_for(&a, &goat_selfupdate::sha256_hex(GOOD)),
        &release_id(SIGNER, NEW),
    );
    let base = surface(files);
    let mut child = e
        .command(&base, &[])
        .env("GOAT_UPDATE_DEADLINE_SECS", "2")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // One byte every 200 ms never trips the 30 s stall timeout; only a total
    // deadline ends it.
    let t0 = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if t0.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let o = child.wait_with_output().unwrap();
    let status = status.unwrap_or_else(|| {
        panic!(
            "still downloading after 20 s: no total deadline\n{}",
            text(&o)
        )
    });
    assert!(!status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("did not finish within 2 s"),
        "{}",
        text(&o)
    );
}

#[test]
fn an_older_genuine_release_under_a_newer_version_json_is_refused() {
    // Rollback: the surface advertises 99.0.0 but serves the genuine, signed
    // 0.0.1 release. Its signature names goat-cli-v0.0.1, not v99.0.0.
    let e = installed();
    e.cosign(&signed_sums(FORGED), &release_id(SIGNER, OLD));
    let o = e.update(&surface(release_as(FORGED, NEW)), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
    assert!(
        text(&o).contains("@refs/tags/goat-cli-v99.0.0"),
        "{}",
        text(&o)
    );
}

#[test]
fn an_advertised_older_version_is_refused_as_a_downgrade() {
    // A genuine signed older release, honestly advertised: still a downgrade.
    let e = installed();
    e.cosign(&signed_sums(FORGED), &release_id(SIGNER, OLD));
    for extra in [&[][..], &["--force"][..], &["--allow-unsigned"][..]] {
        let o = e.update(&surface(release_as(FORGED, OLD)), extra);
        assert!(!o.status.success(), "{extra:?}: {}", text(&o));
        assert!(e.untouched(), "{extra:?}: {}", text(&o));
        assert!(
            text(&o).contains("refusing to downgrade"),
            "{extra:?}: {}",
            text(&o)
        );
    }
}

#[test]
fn allow_downgrade_installs_an_older_signed_release() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, OLD));
    let o = e.update(&surface(release_as(GOOD, OLD)), &["--allow-downgrade"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.now_holds(GOOD), "{}", text(&o));
    assert!(text(&o).contains("for goat-cli-v0.0.1"), "{}", text(&o));
}

#[test]
fn the_same_version_installs_only_with_force() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, RUNNING));
    let o = e.update(&surface(release_as(GOOD, RUNNING)), &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(text(&o).contains("already up to date"), "{}", text(&o));
    let o = e.update(&surface(release_as(GOOD, RUNNING)), &["--force"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.now_holds(GOOD), "{}", text(&o));
}

#[test]
fn a_signature_from_a_branch_a_pull_request_or_another_workflow_is_refused() {
    // All genuine signatures from this repository over exactly these bytes, made
    // outside a release: workflow_dispatch on a branch, a same-repo PR, another
    // workflow on the release tag, the release workflow on another product's tag.
    let repo = "https://github.com/good-org/goatcore/.github/workflows";
    for id in [
        format!("{repo}/_release-binary.yml@refs/heads/feature"),
        format!("{repo}/goat-release.yml@refs/heads/main"),
        format!("{repo}/wg-cp0-release.yml@refs/pull/7/merge"),
        format!("{repo}/_release-binary.yml@refs/pull/7/merge"),
        format!("{repo}/wg-cp0-release.yml@refs/tags/goat-cli-v{NEW}"),
        format!("{repo}/_release-binary.yml@refs/tags/goatbuilder/v{NEW}"),
        format!("{repo}/_release-binary.yml@refs/tags/goat-cli-v{NEW}-rc1"),
    ] {
        let e = installed();
        e.cosign(&signed_sums(FORGED), &id);
        let o = e.update(&surface(release(FORGED)), &[]);
        assert!(!o.status.success(), "{id}: {}", text(&o));
        assert!(e.untouched(), "{id}: {}", text(&o));
        assert!(
            text(&o).contains("release signature refused"),
            "{id}: {}",
            text(&o)
        );
    }
}

#[test]
fn a_version_json_that_is_not_a_release_version_is_refused() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    for v in ["99.0.0-rc1", "v99.0.0", "99.0", "99.0.0/../../x", "99.0.0 "] {
        let mut files = release(GOOD);
        files.insert(
            "/version.json".to_string(),
            Body::Bytes(serde_json::json!({ "version": v }).to_string().into_bytes()),
        );
        let o = e.update(&surface(files), &["--force", "--allow-downgrade"]);
        if v == "99.0.0 " {
            // version.json's value is trimmed before use: this one is 99.0.0.
            assert!(o.status.success(), "{v:?}: {}", text(&o));
            continue;
        }
        assert!(!o.status.success(), "{v:?}: {}", text(&o));
        assert!(e.untouched(), "{v:?}: {}", text(&o));
        assert!(
            text(&o).contains("not a release version"),
            "{v:?}: {}",
            text(&o)
        );
    }
}

#[test]
fn an_unreachable_trust_root_is_named_and_allow_unsigned_does_not_apply() {
    let e = installed();
    let c = e.dir.path().join("path/cosign");
    std::fs::write(
        &c,
        "#!/bin/sh\necho 'Error: getting trusted root: updating local metadata and targets: error updating to TUF remote mirror: Get \"https://tuf-repo-cdn.sigstore.dev/timestamp.json\": dial tcp: lookup tuf-repo-cdn.sigstore.dev: no such host' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o755)).unwrap();
    for extra in [&[][..], &["--allow-unsigned"][..]] {
        let o = e.update(&surface(release(GOOD)), extra);
        assert!(!o.status.success(), "{extra:?}: {}", text(&o));
        assert!(e.untouched(), "{extra:?}: {}", text(&o));
        let t = text(&o);
        assert!(t.contains("could not load the Sigstore trust root"), "{t}");
        assert!(t.contains("cosign initialize"), "{t}");
        assert!(!t.contains("release signature refused"), "{t}");
    }
}

#[test]
fn a_pinned_trust_root_is_used_offline() {
    let e = installed();
    let root = e.dir.path().join("trusted_root.json");
    std::fs::write(&root, "{\"mediaType\":\"stand-in\"}").unwrap();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e
        .command(&surface(release(GOOD)), &[])
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.now_holds(GOOD), "{}", text(&o));
    let args = std::fs::read_to_string(e.dir.path().join("cosign.args")).unwrap();
    let want = format!("--trusted-root={}\n--offline\n", root.display());
    assert!(args.contains(&want), "cosign was called with:\n{args}");
    assert!(text(&o).contains("offline against"), "{}", text(&o));
}

#[test]
fn a_pinned_trust_root_that_is_not_a_file_is_refused() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e
        .command(&surface(release(GOOD)), &["--allow-unsigned"])
        .env(
            "GOAT_SIGSTORE_TRUSTED_ROOT",
            e.dir.path().join("no-such-root.json"),
        )
        .output()
        .unwrap();
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    assert!(text(&o).contains("cannot be read"), "{}", text(&o));
}

#[test]
fn without_a_pinned_trust_root_cosign_is_not_told_to_go_offline() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e.update(&surface(release(GOOD)), &[]);
    assert!(o.status.success(), "{}", text(&o));
    let args = std::fs::read_to_string(e.dir.path().join("cosign.args")).unwrap();
    assert!(
        !args.contains("--offline") && !args.contains("--trusted-root"),
        "{args}"
    );
}

#[test]
fn a_certificate_named_like_a_tuf_failure_is_still_a_refused_signature() {
    // An attacker's repository named so its certificate subject mentions TUF
    // and the trust root: cosign's identity error carries that subject.
    let e = installed();
    e.cosign(
        &signed_sums(FORGED),
        "https://github.com/evil/tuf/.github/workflows/getting-trusted-root-tuf-repo-cdn.sigstore.dev.yml@refs/tags/goat-cli-v99.0.0",
    );
    let o = e.update(&surface(release(FORGED)), &[]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("release signature refused"), "{t}");
    assert!(!t.contains("trust root"), "{t}");
}

#[test]
fn the_trust_root_message_never_suggests_doing_without_cosign() {
    let e = installed();
    let c = e.dir.path().join("path/cosign");
    std::fs::write(
        &c,
        "#!/bin/sh\necho 'Error: getting trusted root: updating local metadata and targets: dial tcp: lookup tuf-repo-cdn.sigstore.dev: no such host' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = e.update(&surface(release(GOOD)), &[]);
    let t = text(&o);
    assert!(!o.status.success() && e.untouched(), "{t}");
    assert!(t.contains("could not load the Sigstore trust root"), "{t}");
    for hint in ["--allow-unsigned", "PATH", "uninstall", "remove cosign"] {
        assert!(
            !t.contains(hint),
            "trust-root message mentions {hint:?}: {t}"
        );
    }
}

#[test]
fn a_dry_run_checks_the_signature_and_downloads_nothing() {
    let e = installed();
    let a = asset();
    let mut files = release(GOOD);
    // Were the binary fetched, the trickle would hold the run past the test.
    files.insert(format!("/{a}"), Body::Trickle);
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e.update(&surface(files), &["--dry-run"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(e.untouched(), "{}", text(&o));
    let t = text(&o);
    assert!(
        t.contains("signed by the good-org/goatcore release workflow for goat-cli-v99.0.0"),
        "{t}"
    );
    assert!(t.contains("dry-run: would install 99.0.0"), "{t}");
}

#[test]
fn a_dry_run_refuses_what_a_real_run_would_refuse() {
    let e = installed();
    // A branch signature: a real run refuses it, so does the dry run.
    e.cosign(
        &signed_sums(FORGED),
        "https://github.com/good-org/goatcore/.github/workflows/_release-binary.yml@refs/heads/feature",
    );
    let o = e.update(&surface(release(FORGED)), &["--dry-run"]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
    assert!(!text(&o).contains("dry-run: would install"), "{}", text(&o));
    // No cosign at all: refused as on a real run.
    std::fs::remove_file(e.dir.path().join("path/cosign")).unwrap();
    let o = e.update(&surface(release(FORGED)), &["--dry-run"]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("cosign is not on PATH"), "{}", text(&o));
}

#[test]
fn a_pinned_trust_root_that_looks_like_a_flag_is_passed_as_one_argument() {
    let e = installed();
    let wd = e.dir.path().join("wd");
    std::fs::create_dir_all(&wd).unwrap();
    std::fs::write(wd.join("--offline"), "{\"mediaType\":\"stand-in\"}").unwrap();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e
        .command(&surface(release(GOOD)), &[])
        .current_dir(&wd)
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", "--offline")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    let args = std::fs::read_to_string(e.dir.path().join("cosign.args")).unwrap();
    assert!(
        args.lines().any(|l| l == "--trusted-root=--offline"),
        "cosign was called with:\n{args}"
    );
    assert!(!args.lines().any(|l| l == "--trusted-root"), "{args}");
}

#[test]
fn an_empty_or_invalid_pinned_trust_root_is_refused_before_cosign_runs() {
    for (name, body, why) in [
        ("empty.json", "", "is empty"),
        ("blank.json", " \n\t", "is empty"),
        ("garbage.json", "not json", "is not a JSON object"),
        ("array.json", "[1,2]", "is not a JSON object"),
    ] {
        let e = installed();
        let root = e.dir.path().join(name);
        std::fs::write(&root, body).unwrap();
        e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
        let o = e
            .command(&surface(release(GOOD)), &["--allow-unsigned"])
            .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
            .output()
            .unwrap();
        let t = text(&o);
        assert!(!o.status.success() && e.untouched(), "{name}: {t}");
        assert!(
            t.contains(why) && t.contains(&root.display().to_string()),
            "{name}: {t}"
        );
        assert!(
            !e.dir.path().join("cosign.args").exists(),
            "{name}: cosign ran"
        );
    }
}

#[test]
fn an_unreadable_pinned_trust_root_is_refused() {
    let e = installed();
    let root = e.dir.path().join("root.json");
    std::fs::write(&root, "{}").unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&root).is_ok() {
        return; // running as root: permissions do not stop a read
    }
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e
        .command(&surface(release(GOOD)), &[])
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
        .output()
        .unwrap();
    let t = text(&o);
    assert!(!o.status.success() && e.untouched(), "{t}");
    assert!(t.contains("cannot be read"), "{t}");
}

#[test]
fn a_pinned_trust_root_cosign_rejects_is_named_as_the_pinned_root() {
    let e = installed();
    let root = e.dir.path().join("trusted_root.json");
    std::fs::write(&root, "{\"mediaType\":\"stale\"}").unwrap();
    let c = e.dir.path().join("path/cosign");
    std::fs::write(
        &c,
        "#!/bin/sh\necho 'Error: getting trusted root: unmarshalling trusted root: proto: syntax error' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = e
        .command(&surface(release(GOOD)), &[])
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
        .output()
        .unwrap();
    let t = text(&o);
    assert!(!o.status.success() && e.untouched(), "{t}");
    assert!(t.contains("could not use the pinned trust root"), "{t}");
    assert!(t.contains(&root.display().to_string()), "{t}");
    assert!(!t.contains("unreachable"), "{t}");
    assert!(!t.contains("set GOAT_SIGSTORE_TRUSTED_ROOT"), "{t}");
    assert!(
        !t.contains("--allow-unsigned") && !t.contains("PATH"),
        "{t}"
    );
}

/// A release cut before the workflow wrote the Sigstore bundle format.
fn legacy_release(bin: &[u8]) -> HashMap<String, Body> {
    let mut f = release(bin);
    f.remove("/SHA256SUMS.sigstore.json");
    f
}

#[test]
fn the_sigstore_bundle_is_preferred_when_served() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e.update(&surface(release(GOOD)), &[]);
    assert!(o.status.success() && e.now_holds(GOOD), "{}", text(&o));
    assert_eq!(e.verified(), "new", "{}", e.cosign_args());
    assert!(
        e.cosign_args().contains("--new-bundle-format\n"),
        "{}",
        e.cosign_args()
    );
    assert!(
        text(&o).contains("SHA256SUMS.sigstore.json"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_legacy_only_release_verifies_online_against_the_same_exact_identity() {
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e.update(&surface(legacy_release(GOOD)), &[]);
    assert!(o.status.success() && e.now_holds(GOOD), "{}", text(&o));
    assert_eq!(e.verified(), "legacy");
    let args = e.cosign_args();
    assert!(!args.contains("--new-bundle-format"), "{args}");
    assert!(
        args.contains(&format!(
            "--certificate-identity\n{}\n",
            release_id(SIGNER, NEW)
        )),
        "{args}"
    );
    assert!(
        text(&o).contains("SHA256SUMS.cosign-bundle"),
        "{}",
        text(&o)
    );
}

#[test]
fn stripping_the_sigstore_bundle_verifies_nothing_less() {
    // The surface withholds SHA256SUMS.sigstore.json and serves a forged binary
    // under altered checksums: the legacy bundle still has to verify them.
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e.update(&surface(legacy_release(FORGED)), &[]);
    assert!(!o.status.success() && e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
    // Nor does a branch signature pass on the legacy path.
    e.cosign(
        &signed_sums(FORGED),
        "https://github.com/good-org/goatcore/.github/workflows/_release-binary.yml@refs/heads/feature",
    );
    let o = e.update(&surface(legacy_release(FORGED)), &[]);
    assert!(!o.status.success() && e.untouched(), "{}", text(&o));
}

#[test]
fn a_bad_sigstore_bundle_is_refused_and_not_traded_for_the_legacy_one() {
    // A served Sigstore bundle that fails is a refusal, not a cue to try the
    // legacy bundle (here a bundle in the wrong format under that name).
    let e = installed();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let mut files = release(GOOD);
    files.insert(
        "/SHA256SUMS.sigstore.json".to_string(),
        Body::Bytes(b"{\"base64Signature\":\"x\"}".to_vec()),
    );
    let o = e.update(&surface(files), &[]);
    assert!(!o.status.success() && e.untouched(), "{}", text(&o));
    assert!(
        text(&o).contains("release signature refused"),
        "{}",
        text(&o)
    );
    assert!(
        text(&o).contains("SHA256SUMS.sigstore.json"),
        "{}",
        text(&o)
    );
}

#[test]
fn offline_needs_a_release_in_the_sigstore_bundle_format() {
    let e = installed();
    let root = e.dir.path().join("trusted_root.json");
    std::fs::write(&root, "{\"mediaType\":\"stand-in\"}").unwrap();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    for extra in [&[][..], &["--allow-unsigned"][..]] {
        let o = e
            .command(&surface(legacy_release(GOOD)), extra)
            .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
            .output()
            .unwrap();
        let t = text(&o);
        assert!(!o.status.success() && e.untouched(), "{extra:?}: {t}");
        assert!(
            t.contains("offline verification needs a goat-cli release signed in the Sigstore bundle format (one that publishes SHA256SUMS.sigstore.json)"),
            "{t}"
        );
    }
    assert!(
        e.cosign_args().is_empty(),
        "cosign ran: {}",
        e.cosign_args()
    );
}

#[test]
fn offline_verification_uses_the_sigstore_bundle_and_the_pinned_root() {
    let e = installed();
    let root = e.dir.path().join("trusted_root.json");
    std::fs::write(&root, "{\"mediaType\":\"stand-in\"}").unwrap();
    e.cosign(&signed_sums(GOOD), &release_id(SIGNER, NEW));
    let o = e
        .command(&surface(release(GOOD)), &[])
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
        .output()
        .unwrap();
    assert!(o.status.success() && e.now_holds(GOOD), "{}", text(&o));
    assert_eq!(e.verified(), "new");
    let args = e.cosign_args();
    for want in [
        "--new-bundle-format\n".to_string(),
        format!("--trusted-root={}\n--offline\n", root.display()),
    ] {
        assert!(args.contains(&want), "{want:?} not in:\n{args}");
    }
}

#[test]
fn an_old_cosign_falls_back_to_the_legacy_bundle_online() {
    let e = installed();
    e.cosign_as(&signed_sums(GOOD), &release_id(SIGNER, NEW), true);
    let o = e.update(&surface(release(GOOD)), &[]);
    assert!(o.status.success() && e.now_holds(GOOD), "{}", text(&o));
    assert_eq!(e.verified(), "legacy");
    assert!(
        text(&o).contains("does not read the Sigstore bundle format"),
        "{}",
        text(&o)
    );
}

#[test]
fn an_old_cosign_cannot_verify_offline_and_says_so() {
    let e = installed();
    let root = e.dir.path().join("trusted_root.json");
    std::fs::write(&root, "{\"mediaType\":\"stand-in\"}").unwrap();
    e.cosign_as(&signed_sums(GOOD), &release_id(SIGNER, NEW), true);
    let o = e
        .command(&surface(release(GOOD)), &[])
        .env("GOAT_SIGSTORE_TRUSTED_ROOT", &root)
        .output()
        .unwrap();
    let t = text(&o);
    assert!(!o.status.success() && e.untouched(), "{t}");
    assert!(
        t.contains("does not support the Sigstore bundle format"),
        "{t}"
    );
    assert!(!t.contains("could not use the pinned trust root"), "{t}");
}

#[test]
fn cosign_v3s_tuf_download_failure_reads_as_the_trust_root() {
    let e = installed();
    let c = e.dir.path().join("path/cosign");
    std::fs::write(
        &c,
        "#!/bin/sh\necho 'Error: failed to download 13.root.json: Get \"https://tuf-repo-cdn.sigstore.dev/13.root.json\": dial tcp: lookup tuf-repo-cdn.sigstore.dev: no such host' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = e.update(&surface(release(GOOD)), &[]);
    let t = text(&o);
    assert!(!o.status.success() && e.untouched(), "{t}");
    assert!(t.contains("could not load the Sigstore trust root"), "{t}");
}

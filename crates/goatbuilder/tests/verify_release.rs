// `goatbuilder update verify-release <dir>`: verify a downloaded goatbuilder
// release without installing it, with update's own checks: the keyless
// signature over SHA256SUMS (update/signature.rs), and every other file of
// the release against SHA256SUMS. A stand-in Sigstore signs the releases
// (tests/support/sigstore.rs); its trusted root is given with --trusted-root.

#[path = "support/sigstore.rs"]
mod sigstore;

use std::path::Path;
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

const REPO: &str = "example-org/goatcore";
const VERSION: &str = "9.9.9";

fn sha256(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// A release directory as the release workflow writes it: two binaries, each
/// with its `.sha256`, SHA256SUMS over the binaries, and `bundle` (or none).
fn release(dir: &Path, sums_override: Option<&[u8]>, bundle: impl FnOnce(&[u8]) -> Option<String>) {
    std::fs::create_dir_all(dir).unwrap();
    let mut sums = String::new();
    for name in ["goatbuilder-linux-amd64", "goatbuilder-darwin-arm64"] {
        let bin = format!("the {name} build").into_bytes();
        std::fs::write(dir.join(name), &bin).unwrap();
        let d = sha256(&bin);
        std::fs::write(dir.join(format!("{name}.sha256")), format!("{d}  {name}\n")).unwrap();
        sums.push_str(&format!("{d}  {name}\n"));
    }
    let signed = sums.clone().into_bytes();
    std::fs::write(dir.join("SHA256SUMS"), sums_override.unwrap_or(&signed)).unwrap();
    if let Some(b) = bundle(&signed) {
        std::fs::write(dir.join("SHA256SUMS.cosign-bundle"), b).unwrap();
    }
}

/// Writes one stand-in release into the directory it is given.
type MakeRelease<'a> = Box<dyn Fn(&Path) + 'a>;

fn verify(dir: &Path, root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["update", "verify-release"])
        .arg(dir)
        .args(["--signer", REPO, "--version", VERSION, "--trusted-root"])
        .arg(root)
        .env("GOATBUILDER_NO_VERSION_CHECK", "1")
        .env_remove("GOATBUILDER_UPDATE_TRUSTED_ROOT")
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

#[test]
fn a_release_signed_by_its_release_workflow_verifies() {
    let s = sigstore::Sigstore::new();
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("trusted_root.json");
    std::fs::write(&root, s.trusted_root()).unwrap();
    let dir = t.path().join("release");
    release(&dir, None, |sums| {
        Some(s.bundle(sums, &sigstore::Workflow::release(REPO, VERSION)))
    });
    let o = verify(&dir, &root);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("goatbuilder-release.yml"), "{}", text(&o));
}

#[test]
fn a_release_that_does_not_verify_is_refused_naming_the_check() {
    let s = sigstore::Sigstore::new();
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("trusted_root.json");
    std::fs::write(&root, s.trusted_root()).unwrap();
    let good = |sums: &[u8]| Some(s.bundle(sums, &sigstore::Workflow::release(REPO, VERSION)));
    let other_workflow = sigstore::Workflow {
        repo: REPO.into(),
        workflow: "goat-release.yml".into(),
        tag: format!("goatbuilder/v{VERSION}"),
    };
    let cases: Vec<(&str, MakeRelease, &str)> = vec![
        (
            "SHA256SUMS changed after signing",
            Box::new(|d: &Path| release(d, Some(b"0000  goatbuilder-linux-amd64\n"), good)),
            "GB-261",
        ),
        (
            "another workflow identity",
            Box::new(|d: &Path| release(d, None, |sums| Some(s.bundle(sums, &other_workflow)))),
            "GB-261",
        ),
        (
            "a Rekor entry naming another sha256",
            Box::new(|d: &Path| {
                release(d, None, |_| {
                    Some(s.bundle(
                        b"another SHA256SUMS",
                        &sigstore::Workflow::release(REPO, VERSION),
                    ))
                })
            }),
            "GB-261",
        ),
        (
            "no bundle",
            Box::new(|d: &Path| release(d, None, |_| None)),
            "GB-261",
        ),
        (
            "a binary that SHA256SUMS does not list at its digest",
            Box::new(|d: &Path| {
                release(d, None, good);
                std::fs::write(d.join("goatbuilder-linux-amd64"), b"another build").unwrap();
            }),
            "GB-074",
        ),
        (
            "a .sha256 that disagrees with SHA256SUMS",
            Box::new(|d: &Path| {
                release(d, None, good);
                std::fs::write(
                    d.join("goatbuilder-darwin-arm64.sha256"),
                    format!("{}  goatbuilder-darwin-arm64\n", sha256(b"x")),
                )
                .unwrap();
            }),
            "GB-074",
        ),
        (
            "a file SHA256SUMS does not list",
            Box::new(|d: &Path| {
                release(d, None, good);
                std::fs::write(d.join("extra.bin"), b"not in the release").unwrap();
            }),
            "GB-074",
        ),
    ];
    for (what, make, code) in cases {
        let dir = t.path().join(what.replace(' ', "-"));
        make(&dir);
        let o = verify(&dir, &root);
        assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
        assert!(text(&o).contains(code), "{what}: {}", text(&o));
    }
}

#[test]
fn a_release_is_not_verified_without_a_trusted_root_that_holds_its_ca() {
    // The embedded public root does not hold the stand-in CA: refused.
    let s = sigstore::Sigstore::new();
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("release");
    release(&dir, None, |sums| {
        Some(s.bundle(sums, &sigstore::Workflow::release(REPO, VERSION)))
    });
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["update", "verify-release"])
        .arg(&dir)
        .args(["--signer", REPO, "--version", VERSION])
        .env("GOATBUILDER_NO_VERSION_CHECK", "1")
        .env_remove("GOATBUILDER_UPDATE_TRUSTED_ROOT")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("GB-261"), "{}", text(&o));
}

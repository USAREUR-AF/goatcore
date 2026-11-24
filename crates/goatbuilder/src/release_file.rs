//! A release component's file, fetched and verified on the operator's machine
//! (fleet-pins `artifact_type: release`, gate J23; docs/design/
//! goatbuilder-build-targets.md section 3.9). The host receives only a
//! verified file and needs no internet.
//!
//! A pin is `<owner>/<repo>@<tag>@sha256:<digest>`, the digest being the
//! release's checksums file's. Verifying one, in order, before anything is
//! written:
//!  1. the checksums file's SHA-256 is the pin's digest;
//!  2. its keyless signature, `<checksums>.cosign-bundle` beside it, verifies
//!     for the component's signer at the pin's tag (update/signature.rs,
//!     [`signature::verify_component`]);
//!  3. the architecture's file is listed in the checksums file, with its
//!     SHA-256.
//!
//! The verified files are kept under the state directory by the pin's digest,
//! where the `release-file` role copies the file from.
//!
//! Both deploy drivers call it through release_deploy.rs, which fetches every
//! architecture's file of a release and writes the variables the play takes.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::update::signature::{self, TrustRoot};

/// The catalog code for a release file that is not fetched or does not verify.
const REFUSED: &str = "GB-381";

/// The largest file read, as goatbuilder's own update reads.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// A release pin, split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pin {
    pub repo: String,
    pub tag: String,
    /// The checksums file's SHA-256, lowercase hex.
    pub digest: String,
}

impl Pin {
    /// A pin `goatbuilder pins` accepts (`pins::is_release_ref`), split.
    pub(crate) fn parse(reference: &str) -> Option<Pin> {
        if !crate::pins::is_release_ref(reference) {
            return None;
        }
        let (repo, rest) = reference.split_once('@')?;
        // A repository part of `.` or `..` would step out of a mirror directory.
        if repo.split('/').any(|seg| seg == "." || seg == "..") {
            return None;
        }
        let (tag, digest) = rest.rsplit_once("@sha256:")?;
        Some(Pin {
            repo: repo.into(),
            tag: tag.into(),
            digest: digest.into(),
        })
    }

    pub(crate) fn text(&self) -> String {
        format!("{}@{}@sha256:{}", self.repo, self.tag, self.digest)
    }
}

/// A component's `release:` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    pub checksums: String,
    /// Architecture to file name.
    pub files: BTreeMap<String, String>,
    pub signer: String,
}

impl Declared {
    /// From a component's `release:` mapping, as `goatbuilder pins` checked it.
    pub(crate) fn from_spec(release: &Value) -> Option<Declared> {
        let checksums = release.get("checksums")?.as_str()?.to_string();
        let signer = release.get("signer")?.as_str()?.to_string();
        let files = release
            .get("files")?
            .as_object()?
            .iter()
            .map(|(k, v)| v.as_str().map(|f| (k.clone(), f.to_string())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(Declared {
            checksums,
            files,
            signer,
        })
    }

    /// The checksums file's signature, beside it.
    pub(crate) fn bundle(&self) -> String {
        format!("{}.cosign-bundle", self.checksums)
    }
}

/// Where a release's files are read from.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// GitHub's release downloads: `<base>/<owner>/<repo>/releases/download/<tag>/<name>`.
    Web {
        base: String,
        connect: Duration,
        stall: Duration,
    },
    /// A directory holding `<owner>/<repo>/<tag>/<name>`: an operator's mirror.
    Dir(PathBuf),
    /// The directory an earlier run kept this pin's verified files in. They are
    /// verified again like any other.
    Kept(PathBuf),
}

impl Source {
    pub(crate) fn github() -> Source {
        Source::Web {
            base: "https://github.com".into(),
            connect: Duration::from_secs(20),
            stall: Duration::from_secs(60),
        }
    }

    fn get(&self, pin: &Pin, name: &str) -> std::result::Result<Vec<u8>, String> {
        match self {
            Source::Dir(_) | Source::Kept(_) => {
                let p = match self {
                    Source::Kept(d) => d.join(name),
                    Source::Dir(d) => d.join(&pin.repo).join(&pin.tag).join(name),
                    Source::Web { .. } => unreachable!("matched above"),
                };
                let meta = std::fs::symlink_metadata(&p)
                    .map_err(|e| format!("{} cannot be read: {e}", p.display()))?;
                if !meta.is_file() {
                    return Err(format!("{} is not a plain file", p.display()));
                }
                let mut out = Vec::new();
                std::fs::File::open(&p)
                    .and_then(|f| f.take(MAX_FILE).read_to_end(&mut out))
                    .map_err(|e| format!("{} cannot be read: {e}", p.display()))?;
                Ok(out)
            }
            Source::Web {
                base,
                connect,
                stall,
            } => {
                let url = format!("{base}/{}/releases/download/{}/{name}", pin.repo, pin.tag);
                let agent = ureq::AgentBuilder::new()
                    .timeout_connect(*connect)
                    .timeout_read(*stall)
                    .build();
                let resp = agent.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
                let mut out = Vec::new();
                resp.into_reader()
                    .take(MAX_FILE)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{url}: {e} after {} bytes", out.len()))?;
                Ok(out)
            }
        }
    }
}

/// A verified file, kept on the operator's machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fetched {
    pub path: PathBuf,
    pub name: String,
    /// The file's SHA-256, as the checksums file lists it and as it was read.
    pub sha256: String,
    /// Who signed the checksums file (the certificate's SAN).
    pub signed_by: String,
    pub log_index: i64,
}

pub(crate) fn refused(component: &str, pin: &Pin, check: &str, why: String) -> anyhow::Error {
    goat_errors::coded(
        REFUSED,
        format!(
            "refused before any host: {component}'s release {}: {why} (check: {check})",
            pin.text()
        ),
    )
    .into()
}

/// The digest the checksums file lists for `name`, in `sha256sum`'s format
/// (`<hex>  <name>` or `<hex> *<name>`).
fn listed(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hex, rest) = line.split_once(char::is_whitespace)?;
        let file = rest.trim_start().trim_start_matches('*');
        (file == name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    })
}

/// Fetch `component`'s file for `arch` at `pin`, verify it, and keep it under
/// `store`/<component>/<the pin's digest>/. Every refusal is GB-381, before
/// anything is written.
pub(crate) fn fetch(
    component: &str,
    pin: &Pin,
    declared: &Declared,
    arch: &str,
    source: &Source,
    root: &TrustRoot,
    store: &Path,
) -> Result<Fetched> {
    let bad = |check: &str, why: String| refused(component, pin, check, why);
    // `goatbuilder pins` holds the pin to this too; checked again here, before
    // anything is fetched, so no caller can install a release of a repository
    // the component's signer is not.
    if !crate::pins::pin_repository_is_signers(&pin.repo, &declared.signer) {
        return Err(bad(
            "repository",
            format!(
                "the pin's repository is {}, and release.signer is a workflow of {}: \
                 a release pin's repository must be its signer's repository",
                pin.repo,
                crate::pins::signer_repository(&declared.signer)
                    .unwrap_or_else(|| "no repository".into())
            ),
        ));
    }
    let Some(name) = declared.files.get(arch) else {
        return Err(bad(
            "arch",
            format!(
                "it declares no file for {arch} (it declares: {})",
                declared
                    .files
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };
    let sums = source
        .get(pin, &declared.checksums)
        .map_err(|e| bad("fetch", e))?;
    let got = goat_selfupdate::sha256_hex(&sums);
    if got != pin.digest {
        return Err(bad(
            "digest",
            format!(
                "{} is sha256 {got}, and the pin names {}",
                declared.checksums, pin.digest
            ),
        ));
    }
    let bundle = source.get(pin, &declared.bundle()).map_err(|e| {
        bad(
            "fetch",
            format!(
                "the release has no signature over {}: {e}",
                declared.checksums
            ),
        )
    })?;
    let expect = signature::Component {
        repo: &pin.repo,
        tag: &pin.tag,
        signer: &declared.signer,
    };
    let v = signature::verify_component(&sums, &String::from_utf8_lossy(&bundle), &expect, root)
        .map_err(|r| {
            bad(
                "signature",
                format!(
                    "{} does not verify for {} at {}: {r}",
                    declared.checksums, declared.signer, pin.tag
                ),
            )
        })?;
    let sums_text = String::from_utf8_lossy(&sums).to_string();
    let Some(want) = listed(&sums_text, name) else {
        return Err(bad(
            "listed",
            format!("{} does not list {name}", declared.checksums),
        ));
    };
    let file = source.get(pin, name).map_err(|e| bad("fetch", e))?;
    let sha = goat_selfupdate::sha256_hex(&file);
    if sha != want {
        return Err(bad(
            "listed",
            format!(
                "{name} is sha256 {sha}, and {} lists {want}",
                declared.checksums
            ),
        ));
    }
    let dir = kept_dir(store, component, pin);
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad("store", format!("{} cannot be made: {e}", dir.display())))?;
    for (n, bytes) in [
        (declared.checksums.as_str(), &sums),
        (declared.bundle().as_str(), &bundle),
        (name.as_str(), &file),
    ] {
        // Two runs may keep one release at once.
        let tmp = dir.join(format!(".{n}.{}.partial", std::process::id()));
        std::fs::write(&tmp, bytes)
            .and_then(|_| std::fs::rename(&tmp, dir.join(n)))
            .map_err(|e| {
                bad(
                    "store",
                    format!("{} cannot be written: {e}", dir.join(n).display()),
                )
            })?;
    }
    Ok(Fetched {
        path: dir.join(name),
        name: name.clone(),
        sha256: sha,
        signed_by: v.identity,
        log_index: v.log_index,
    })
}

/// Where a pin's verified files are kept.
pub(crate) fn kept_dir(store: &Path, component: &str, pin: &Pin) -> PathBuf {
    store.join(component).join(&pin.digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::signature::stand_in;

    const REPO: &str = "example-org/example-tool";
    const TAG: &str = "tool/v1.2.0";
    const SIGNER: &str =
        "https://github.com/example-org/example-tool/.github/workflows/_release-binary.yml";

    struct Release {
        _dir: tempfile::TempDir,
        mirror: PathBuf,
        store: PathBuf,
        root: TrustRoot,
        pin: Pin,
        declared: Declared,
        sigstore: stand_in::Sigstore,
    }

    /// A release of two files, signed by the stand-in Sigstore as SIGNER at TAG,
    /// in a mirror directory.
    fn release() -> Release {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join("mirror");
        let at = mirror.join(REPO).join(TAG);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("tool-linux-amd64"), b"amd64 build\n").unwrap();
        std::fs::write(at.join("tool-linux-arm64"), b"arm64 build\n").unwrap();
        let sums = format!(
            "{}  tool-linux-amd64\n{} *tool-linux-arm64\n",
            goat_selfupdate::sha256_hex(b"amd64 build\n"),
            goat_selfupdate::sha256_hex(b"arm64 build\n")
        );
        std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
        let sigstore = stand_in::Sigstore::new();
        let w = stand_in::Workflow {
            repo: REPO.into(),
            workflow: "release.yml".into(),
            tag: TAG.into(),
        };
        std::fs::write(
            at.join("SHA256SUMS.cosign-bundle"),
            sigstore.bundle(sums.as_bytes(), &w),
        )
        .unwrap();
        let root = TrustRoot::parse(&sigstore.trusted_root(), "stand-in").unwrap();
        let pin = Pin::parse(&format!(
            "{REPO}@{TAG}@sha256:{}",
            goat_selfupdate::sha256_hex(sums.as_bytes())
        ))
        .unwrap();
        let declared = Declared {
            checksums: "SHA256SUMS".into(),
            files: [
                ("amd64".to_string(), "tool-linux-amd64".to_string()),
                ("arm64".to_string(), "tool-linux-arm64".to_string()),
            ]
            .into(),
            signer: SIGNER.into(),
        };
        let store = dir.path().join("store");
        Release {
            _dir: dir,
            mirror,
            store,
            root,
            pin,
            declared,
            sigstore,
        }
    }

    fn code(e: &anyhow::Error) -> String {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_pin_splits_on_its_first_at_and_its_digest() {
        let p = Pin::parse(&format!("{REPO}@{TAG}@sha256:{}", "d".repeat(64))).unwrap();
        assert_eq!(
            (p.repo.as_str(), p.tag.as_str(), p.digest.len()),
            (REPO, TAG, 64)
        );
        assert_eq!(p.text(), format!("{REPO}@{TAG}@sha256:{}", "d".repeat(64)));
        assert!(Pin::parse("example-org/example-tool@v1").is_none());
        assert!(Pin::parse(&format!("example-org/..@v1@sha256:{}", "d".repeat(64))).is_none());
    }

    #[test]
    fn a_signed_listed_file_is_fetched_verified_and_kept_by_its_pin() {
        let r = release();
        let f = fetch(
            "tool",
            &r.pin,
            &r.declared,
            "arm64",
            &Source::Dir(r.mirror.clone()),
            &r.root,
            &r.store,
        )
        .unwrap();
        assert_eq!(
            f.path,
            r.store
                .join("tool")
                .join(&r.pin.digest)
                .join("tool-linux-arm64")
        );
        assert_eq!(std::fs::read(&f.path).unwrap(), b"arm64 build\n");
        assert_eq!(f.sha256, goat_selfupdate::sha256_hex(b"arm64 build\n"));
        assert_eq!(f.signed_by, format!("{SIGNER}@refs/tags/{TAG}"));
        assert!(r
            .store
            .join("tool")
            .join(&r.pin.digest)
            .join("SHA256SUMS.cosign-bundle")
            .is_file());
        // Kept, it verifies again with no mirror and no network.
        let kept = Source::Kept(kept_dir(&r.store, "tool", &r.pin));
        let again = fetch("tool", &r.pin, &r.declared, "arm64", &kept, &r.root, &r.store).unwrap();
        assert_eq!(again, f);
        std::fs::write(&f.path, b"changed on disk\n").unwrap();
        let e = fetch("tool", &r.pin, &r.declared, "arm64", &kept, &r.root, &r.store).unwrap_err();
        assert!(format!("{e:#}").contains("(check: listed)"), "{e:#}");
    }

    #[test]
    fn a_changed_digest_a_wrong_signer_or_an_unlisted_file_is_refused_before_anything_is_written() {
        let r = release();
        let src = Source::Dir(r.mirror.clone());
        let refusal = |pin: &Pin, d: &Declared, arch: &str| {
            let e = fetch("tool", pin, d, arch, &src, &r.root, &r.store).unwrap_err();
            assert_eq!(code(&e), "GB-381", "{e:#}");
            assert!(!r.store.exists(), "nothing written: {e:#}");
            format!("{e:#}")
        };
        let other = Pin {
            digest: "e".repeat(64),
            ..r.pin.clone()
        };
        assert!(refusal(&other, &r.declared, "amd64").contains("(check: digest)"));
        let wrong = Declared {
            signer: "https://github.com/example-org/example-tool/.github/workflows/other.yml"
                .into(),
            ..r.declared.clone()
        };
        assert!(refusal(&r.pin, &wrong, "amd64").contains("(check: signature)"));
        assert!(refusal(&r.pin, &r.declared, "riscv64").contains("(check: arch)"));
        let renamed = Declared {
            files: [("amd64".to_string(), "tool-linux-x86_64".to_string())].into(),
            ..r.declared.clone()
        };
        assert!(refusal(&r.pin, &renamed, "amd64").contains("does not list tool-linux-x86_64"));
        // The file changed after the checksums file was signed.
        std::fs::write(
            r.mirror.join(REPO).join(TAG).join("tool-linux-amd64"),
            b"another build\n",
        )
        .unwrap();
        assert!(refusal(&r.pin, &r.declared, "amd64").contains("(check: listed)"));
        // A release with no signature.
        std::fs::remove_file(
            r.mirror
                .join(REPO)
                .join(TAG)
                .join("SHA256SUMS.cosign-bundle"),
        )
        .unwrap();
        assert!(refusal(&r.pin, &r.declared, "arm64").contains("has no signature over SHA256SUMS"));
    }

    /// A release of another repository, signed by a run of that repository
    /// calling the declared signer's workflow at the pin's tag: its source
    /// repository is the pin's and its SAN is the declared signer's, and it is
    /// still refused, because the pin's repository is not the signer's.
    #[test]
    fn a_release_of_a_repository_other_than_the_signer_s_is_refused() {
        let r = release();
        const OTHER: &str = "other-org/example-tool";
        let at = r.mirror.join(OTHER).join(TAG);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("tool-linux-amd64"), b"other build\n").unwrap();
        let sums = format!(
            "{}  tool-linux-amd64\n",
            goat_selfupdate::sha256_hex(b"other build\n")
        );
        std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
        let w = stand_in::Workflow {
            repo: OTHER.into(),
            workflow: "release.yml".into(),
            tag: TAG.into(),
        };
        let leaf = stand_in::Leaf {
            san_repo: Some(REPO),
            ..Default::default()
        };
        std::fs::write(
            at.join("SHA256SUMS.cosign-bundle"),
            r.sigstore.bundle_with(sums.as_bytes(), &w, leaf),
        )
        .unwrap();
        let pin = Pin::parse(&format!(
            "{OTHER}@{TAG}@sha256:{}",
            goat_selfupdate::sha256_hex(sums.as_bytes())
        ))
        .unwrap();
        let e = fetch(
            "tool",
            &pin,
            &r.declared,
            "amd64",
            &Source::Dir(r.mirror.clone()),
            &r.root,
            &r.store,
        )
        .unwrap_err();
        assert_eq!(code(&e), "GB-381", "{e:#}");
        assert!(
            format!("{e:#}").contains("(check: repository)")
                && format!("{e:#}").contains("must be its signer's repository"),
            "{e:#}"
        );
        assert!(!r.store.exists(), "nothing written: {e:#}");
        // The same names in another case are the same repository.
        let upper = Pin {
            repo: "Example-Org/Example-Tool".into(),
            ..r.pin.clone()
        };
        let msg = fetch(
            "tool",
            &upper,
            &r.declared,
            "amd64",
            &Source::Dir(r.mirror.clone()),
            &r.root,
            &r.store,
        )
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap_or_default();
        assert!(!msg.contains("(check: repository)"), "{msg}");
    }

    #[test]
    fn a_checksums_file_signed_at_another_tag_is_refused() {
        let r = release();
        let at = r.mirror.join(REPO).join(TAG);
        let sums = std::fs::read(at.join("SHA256SUMS")).unwrap();
        let w = stand_in::Workflow {
            repo: REPO.into(),
            workflow: "release.yml".into(),
            tag: "tool/v1.1.0".into(),
        };
        std::fs::write(
            at.join("SHA256SUMS.cosign-bundle"),
            r.sigstore.bundle(&sums, &w),
        )
        .unwrap();
        let e = fetch(
            "tool",
            &r.pin,
            &r.declared,
            "amd64",
            &Source::Dir(r.mirror.clone()),
            &r.root,
            &r.store,
        )
        .unwrap_err();
        assert!(
            format!("{e:#}").contains("the source ref is refs/tags/tool/v1.1.0"),
            "{e:#}"
        );
    }
}

//! A release component's file, fetched and verified on the operator's machine
//! (fleet-pins `artifact_type: release`, gate J23; docs/design/
//! goatbuilder-build-targets.md section 3.9). The host receives only a
//! verified file and needs no internet.
//!
//! A pin is `<owner>/<repo>@<tag>@sha256:<digest>`, the digest being the
//! release's checksums file's. Verifying one, in order, before anything is
//! written:
//!  1. the checksums file's SHA-256 is the pin's digest;
//!  2. its keyless signature beside it (`<checksums>.cosign-bundle`, or
//!     `<checksums>.sigstore.json` when the pins say `bundle: sigstore`) verifies
//!     for the component's signer at the pin's tag (update/signature.rs,
//!     [`signature::verify_component`]);
//!  3. the architecture's file is listed in the checksums file, with its
//!     SHA-256;
//!  4. with `bundle: attestation`, in place of 2: the file's own GitHub
//!     artifact attestation (SLSA provenance) verifies for the signer at the
//!     pin's tag and names the file and its SHA-256
//!     ([`signature::verify_attestation`]). It is read from GitHub's
//!     attestations API by the file's SHA-256, or as `<file>.attestations.json`
//!     from a mirror or an earlier run's kept files.
//!
//! The verified files are kept under the state directory by the pin's digest,
//! where the `release-file` role copies the file from.
//!
//! Both deploy drivers call it through release_deploy.rs, which fetches every
//! architecture's file of a release and writes the variables the play takes.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::update::signature::{self, Format, TrustRoot};

/// The catalog code for a release file that is not fetched or does not verify.
const REFUSED: &str = "GB-381";

/// The largest file read, as goatbuilder's own update reads.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// The largest checksums file or signature read.
const MAX_SMALL: u64 = 1024 * 1024;
/// How long one of those may take in all.
const SMALL_DEADLINE: Duration = Duration::from_secs(120);

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
    /// Which signature the checksums file carries.
    pub format: Format,
    /// The play the archive carries, its path under the archive's top directory.
    pub play: Option<String>,
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
            format: match release.get("bundle") {
                None => Format::Cosign,
                Some(b) if b.as_str() == Some("cosign") => Format::Cosign,
                Some(b) if b.as_str() == Some("sigstore") => Format::Sigstore,
                Some(b) if b.as_str() == Some("attestation") => Format::Attestation,
                Some(_) => return None,
            },
            play: match release.get("play") {
                None => None,
                Some(p) => Some(p.as_str()?.to_string()),
            },
        })
    }

    /// The checksums file's signature, beside it; an attested release has none.
    pub(crate) fn bundle(&self) -> Option<String> {
        match self.format {
            Format::Cosign => Some(format!("{}.cosign-bundle", self.checksums)),
            Format::Sigstore => Some(format!("{}.sigstore.json", self.checksums)),
            Format::Attestation => None,
        }
    }

    /// Where a file's attestations are kept, or read from a mirror.
    pub(crate) fn attestations_of(name: &str) -> String {
        format!("{name}.attestations.json")
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

    /// `name` of the release, refused over `limit` bytes.
    fn get(&self, pin: &Pin, name: &str, limit: u64) -> std::result::Result<Vec<u8>, String> {
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
                    .and_then(|f| f.take(limit + 1).read_to_end(&mut out))
                    .map_err(|e| format!("{} cannot be read: {e}", p.display()))?;
                within(out, limit, &p.display().to_string())
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
                let mut req = agent.get(&url);
                if limit <= MAX_SMALL {
                    req = req.timeout(SMALL_DEADLINE);
                }
                let resp = req.call().map_err(|e| format!("{url}: {e}"))?;
                let mut out = Vec::new();
                resp.into_reader()
                    .take(limit + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{url}: {e} after {} bytes", out.len()))?;
                within(out, limit, &url)
            }
        }
    }
}

impl Source {
    /// The attestations of the file whose SHA-256 is `sha256`: GitHub's
    /// attestations API's answer for it, or the same kept beside the file.
    fn attestations(&self, pin: &Pin, name: &str, sha256: &str) -> std::result::Result<Vec<u8>, String> {
        match self {
            Source::Web { base, connect, stall } => {
                let api = if base == "https://github.com" { "https://api.github.com".to_string() } else { format!("{base}/api") };
                // Provenance only: another predicate is never read.
                let url = format!("{api}/repos/{}/attestations/sha256:{sha256}?predicate_type=provenance&per_page=30", pin.repo);
                let agent = ureq::AgentBuilder::new().timeout_connect(*connect).timeout_read(*stall).build();
                let resp = agent
                    .get(&url)
                    .set("Accept", "application/vnd.github+json")
                    .timeout(SMALL_DEADLINE)
                    .call()
                    .map_err(|e| format!("{url}: {e}"))?;
                let mut out = Vec::new();
                resp.into_reader().take(MAX_SMALL + 1).read_to_end(&mut out).map_err(|e| format!("{url}: {e} after {} bytes", out.len()))?;
                within(out, MAX_SMALL, &url)
            }
            _ => self.get(pin, &Declared::attestations_of(name), MAX_SMALL),
        }
    }
}

/// GitHub's attestations API's answer: each attestation's bundle as the exact
/// text sent, so the bundle reader judges those bytes (a repeated key, its size).
#[derive(serde::Deserialize)]
struct Answer {
    attestations: Vec<Item>,
}

#[derive(serde::Deserialize)]
struct Item {
    #[serde(default)]
    bundle: Option<Box<serde_json::value::RawValue>>,
}

/// The most attestations of one digest read, and the most whose cryptography is checked.
const MAX_LISTED: usize = 64;
const MAX_CHECKED: usize = 16;

/// Whether a bundle is worth the cryptography: it claims SLSA provenance in an
/// in-toto envelope. Nothing it claims is trusted here.
fn claims_provenance(bundle: &str) -> bool {
    use base64::Engine as _;
    let v: Value = serde_json::from_str(bundle).unwrap_or_default();
    let env = &v["dsseEnvelope"];
    env["payloadType"] == signature::attestation::PAYLOAD_TYPE
        && base64::engine::general_purpose::STANDARD
            .decode(env["payload"].as_str().unwrap_or_default())
            .ok()
            .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
            .is_some_and(|st| st["predicateType"] == signature::attestation::PREDICATE_TYPE)
}

/// The first attestation of `text` that verifies for `expect` as provenance of
/// the file; what was refused, counted by check, otherwise.
fn attested(text: &[u8], name: &str, sha256: &str, expect: &signature::Component, root: &TrustRoot) -> std::result::Result<signature::Attested, String> {
    let answer: Answer = serde_json::from_slice(text).map_err(|_| "the answer has no list of attestations it reads".to_string())?;
    let (mut other, mut checked) = (0usize, 0usize);
    let mut refused: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut first = String::new();
    for item in answer.attestations.iter().take(MAX_LISTED) {
        let Some(b) = &item.bundle else {
            other += 1;
            continue;
        };
        if !claims_provenance(b.get()) {
            other += 1;
            continue;
        }
        if checked == MAX_CHECKED {
            break;
        }
        checked += 1;
        match signature::verify_attestation(name, sha256, b.get(), expect, root) {
            Ok(got) => return Ok(got),
            Err(r) => {
                *refused.entry(r.check).or_default() += 1;
                if first.is_empty() {
                    first = r.to_string();
                }
            }
        }
    }
    let counts: Vec<String> = refused.iter().map(|(c, n)| format!("{n} at {c}")).collect();
    Err(format!(
        "of its {} attestations, {other} are not SLSA provenance and {checked} were checked{}{}",
        answer.attestations.len(),
        if counts.is_empty() { String::new() } else { format!(", refused {}", counts.join(", ")) },
        if first.is_empty() { String::new() } else { format!("; the first: {first}") }
    ))
}

fn within(out: Vec<u8>, limit: u64, what: &str) -> std::result::Result<Vec<u8>, String> {
    if out.len() as u64 > limit {
        return Err(format!("{what} is over {limit} bytes, the most read of it"));
    }
    Ok(out)
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
    if *name == declared.checksums || Some(name) == declared.bundle().as_ref() || name.ends_with(".attestations.json") || declared.checksums.ends_with(".attestations.json") {
        return Err(bad(
            "declaration",
            format!("{name} is its checksums file or that file's signature, not a file of the release"),
        ));
    }
    let sums = source
        .get(pin, &declared.checksums, MAX_SMALL)
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
    let expect = signature::Component {
        repo: &pin.repo,
        tag: &pin.tag,
        signer: &declared.signer,
    };
    // The checksums file's own signature, where it has one.
    let mut signed = None;
    if let Some(sig_name) = declared.bundle() {
        let bundle = source.get(pin, &sig_name, MAX_SMALL).map_err(|e| {
            bad(
                "fetch",
                format!(
                    "the release has no signature over {}: {e}",
                    declared.checksums
                ),
            )
        })?;
        let text = std::str::from_utf8(&bundle).map_err(|_| {
            bad("signature", format!("{sig_name} is not UTF-8 text"))
        })?;
        let v = signature::verify_component(&sums, text, declared.format, &expect, root).map_err(|r| {
            bad(
                "signature",
                format!(
                    "{} does not verify for {} at {}: {r}",
                    declared.checksums, declared.signer, pin.tag
                ),
            )
        })?;
        signed = Some((sig_name, bundle, v));
    }
    let sums_text = String::from_utf8_lossy(&sums).to_string();
    let Some(want) = listed(&sums_text, name) else {
        return Err(bad(
            "listed",
            format!("{} does not list {name}", declared.checksums),
        ));
    };
    let file = source.get(pin, name, MAX_FILE).map_err(|e| bad("fetch", e))?;
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
    // An attested release: the file's own attestation, in place of the checksums
    // file's signature.
    let (kept_sig, v) = match signed {
        Some((n, b, v)) => ((n, b), v),
        None => {
            let text = source.attestations(pin, name, &sha).map_err(|e| bad("fetch", format!("the attestations of {name} cannot be read: {e}")))?;
            let a = attested(&text, name, &sha, &expect, root).map_err(|why| {
                bad("signature", format!("{name} has no attestation that verifies for {} at {}: {why}", declared.signer, pin.tag))
            })?;
            ((Declared::attestations_of(name), text), a.verified)
        }
    };
    let dir = kept_dir(store, component, pin);
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad("store", format!("{} cannot be made: {e}", dir.display())))?;
    for (n, bytes) in [
        (declared.checksums.as_str(), &sums),
        (kept_sig.0.as_str(), &kept_sig.1),
        (name.as_str(), &file),
    ] {
        // Two runs may keep one release at once.
        let tmp = dir.join(format!(".{n}.{}.partial", std::process::id()));
        // Never through a link or a file that is already there.
        let _ = std::fs::remove_file(&tmp);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .and_then(|mut f| f.write_all(bytes))
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
            format: Format::Cosign,
            play: None,
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

    /// [`release`] attested: no signature over SHA256SUMS, an attestation of
    /// both files beside each, as GitHub's attestations API answers.
    fn attested_release() -> Release {
        let mut r = release();
        let at = r.mirror.join(REPO).join(TAG);
        std::fs::remove_file(at.join("SHA256SUMS.cosign-bundle")).unwrap();
        let w = stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        let a = r.sigstore.attestation_v03(&[("tool-linux-amd64", b"amd64 build\n"), ("tool-linux-arm64", b"arm64 build\n")], &w);
        for f in ["tool-linux-amd64", "tool-linux-arm64"] {
            let answer = serde_json::json!({"attestations": [{"bundle": serde_json::from_str::<Value>(&a).unwrap(), "repository_id": 1}]});
            std::fs::write(at.join(Declared::attestations_of(f)), answer.to_string()).unwrap();
        }
        r.declared.format = Format::Attestation;
        r
    }

    #[test]
    fn an_attested_file_is_fetched_verified_by_its_own_attestation_and_kept() {
        let r = attested_release();
        let src = Source::Dir(r.mirror.clone());
        let f = fetch("tool", &r.pin, &r.declared, "amd64", &src, &r.root, &r.store).unwrap();
        assert!(f.signed_by.ends_with("@refs/tags/tool/v1.2.0"), "{}", f.signed_by);
        let kept = kept_dir(&r.store, "tool", &r.pin);
        assert!(kept.join("tool-linux-amd64.attestations.json").is_file() && !kept.join("SHA256SUMS.cosign-bundle").exists());
        // Kept, it verifies again with no mirror.
        let again = fetch("tool", &r.pin, &r.declared, "amd64", &Source::Kept(kept.clone()), &r.root, &r.store).unwrap();
        assert_eq!(again.sha256, f.sha256);
        // A first attestation that is not one is passed over for one that is.
        let at = r.mirror.join(REPO).join(TAG);
        let p = at.join(Declared::attestations_of("tool-linux-arm64"));
        let mut answer: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let good = answer["attestations"][0].clone();
        answer["attestations"] = serde_json::json!([{"bundle": {"mediaType": "x"}}, good]);
        std::fs::write(&p, answer.to_string()).unwrap();
        fetch("tool", &r.pin, &r.declared, "arm64", &src, &r.root, &r.store).unwrap();
    }

    #[test]
    fn an_attested_file_without_its_own_good_attestation_is_refused_before_it_is_kept() {
        let check = |r: &Release, arch: &str| {
            let e = fetch("tool", &r.pin, &r.declared, arch, &Source::Dir(r.mirror.clone()), &r.root, &r.store).unwrap_err();
            assert_eq!(code(&e), REFUSED);
            assert!(!kept_dir(&r.store, "tool", &r.pin).join("tool-linux-amd64").exists());
            format!("{e:#}")
        };
        let at = |r: &Release| r.mirror.join(REPO).join(TAG);
        // None at all.
        let r = attested_release();
        std::fs::remove_file(at(&r).join("tool-linux-amd64.attestations.json")).unwrap();
        assert!(check(&r, "amd64").contains("(check: fetch)"));
        // One of another file, of another tag, or by another signer.
        for w in [
            stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: "tool/v1.1.0".into() },
            stand_in::Workflow { repo: "example-org/other".into(), workflow: "release.yml".into(), tag: TAG.into() },
        ] {
            let r = attested_release();
            let a = r.sigstore.attestation_v03(&[("tool-linux-amd64", b"amd64 build\n")], &w);
            std::fs::write(at(&r).join("tool-linux-amd64.attestations.json"), serde_json::json!({"attestations": [{"bundle": serde_json::from_str::<Value>(&a).unwrap()}]}).to_string()).unwrap();
            assert!(check(&r, "amd64").contains("(check: signature)"));
        }
        let r = attested_release();
        let a = r.sigstore.attestation_v03(&[("tool-linux-amd64", b"another build\n")], &stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() });
        std::fs::write(at(&r).join("tool-linux-amd64.attestations.json"), serde_json::json!({"attestations": [{"bundle": serde_json::from_str::<Value>(&a).unwrap()}]}).to_string()).unwrap();
        let e = check(&r, "amd64");
        assert!(e.contains("(check: signature)") && e.contains("is not the file's sha256"), "{e}");
        // An answer that is not one, and a signature over the checksums file in its place.
        let r = attested_release();
        std::fs::write(at(&r).join("tool-linux-amd64.attestations.json"), "[]").unwrap();
        assert!(check(&r, "amd64").contains("no list of attestations"));
        let r = attested_release();
        let sums = std::fs::read(at(&r).join("SHA256SUMS")).unwrap();
        let w = stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        std::fs::write(at(&r).join("tool-linux-amd64.attestations.json"), serde_json::json!({"attestations": [{"bundle": serde_json::from_str::<Value>(&r.sigstore.bundle_v03(&sums, &w)).unwrap()}]}).to_string()).unwrap();
        assert!(check(&r, "amd64").contains("(check: signature)"));
        // A file named as the attestations of another.
        let mut r = attested_release();
        r.declared.files.insert("amd64".into(), "tool-linux-arm64.attestations.json".into());
        assert!(check(&r, "amd64").contains("(check: declaration)"));
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
        // A declared file that is the checksums file or its signature.
        for n in ["SHA256SUMS", "SHA256SUMS.cosign-bundle"] {
            let own = Declared {
                files: [("amd64".to_string(), n.to_string())].into(),
                ..r.declared.clone()
            };
            assert!(refusal(&r.pin, &own, "amd64").contains("(check: declaration)"));
        }
        // A signature that is not text, then one too big to read.
        let sig = r.mirror.join(REPO).join(TAG).join("SHA256SUMS.cosign-bundle");
        let good = std::fs::read(&sig).unwrap();
        std::fs::write(&sig, [&good[..20], b"\xff\xff", &good[20..]].concat()).unwrap();
        assert!(refusal(&r.pin, &r.declared, "amd64").contains("is not UTF-8 text"));
        std::fs::write(&sig, vec![b' '; MAX_SMALL as usize + 1]).unwrap();
        assert!(refusal(&r.pin, &r.declared, "amd64").contains("the most read of it"));
        std::fs::write(&sig, good).unwrap();
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

    #[test]
    fn the_pins_choose_the_signature_format_and_neither_stands_in_for_the_other() {
        let r = release();
        let at = r.mirror.join(REPO).join(TAG);
        let src = Source::Dir(r.mirror.clone());
        let spec = |b: Value| {
            let mut v = serde_json::json!({"checksums": "SHA256SUMS", "files": {"amd64": "tool-linux-amd64"}, "signer": SIGNER});
            v["bundle"] = b;
            Declared::from_spec(&v)
        };
        let sigstore = spec("sigstore".into()).unwrap();
        assert_eq!(sigstore.bundle().as_deref(), Some("SHA256SUMS.sigstore.json"));
        assert_eq!(spec("cosign".into()).unwrap().bundle().as_deref(), Some("SHA256SUMS.cosign-bundle"));
        let attested = spec("attestation".into()).unwrap();
        assert_eq!((attested.format, attested.bundle()), (Format::Attestation, None));
        assert!(spec("other".into()).is_none() && spec(1.into()).is_none() && spec(Value::Null).is_none());
        // Only the older one is published.
        let e = fetch("tool", &r.pin, &sigstore, "amd64", &src, &r.root, &r.store).unwrap_err();
        assert!(format!("{e:#}").contains("has no signature over SHA256SUMS"), "{e:#}");
        // The older bundle under the newer name.
        std::fs::copy(at.join("SHA256SUMS.cosign-bundle"), at.join("SHA256SUMS.sigstore.json")).unwrap();
        let e = fetch("tool", &r.pin, &sigstore, "amd64", &src, &r.root, &r.store).unwrap_err();
        assert!(format!("{e:#}").contains("(check: signature)"), "{e:#}");
        assert!(!r.store.exists());
        // The newer one, alone.
        let w = stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        let sums = std::fs::read(at.join("SHA256SUMS")).unwrap();
        std::fs::write(at.join("SHA256SUMS.sigstore.json"), r.sigstore.bundle_v03(&sums, &w)).unwrap();
        std::fs::remove_file(at.join("SHA256SUMS.cosign-bundle")).unwrap();
        let f = fetch("tool", &r.pin, &sigstore, "amd64", &src, &r.root, &r.store).unwrap();
        assert_eq!(f.signed_by, format!("{SIGNER}@refs/tags/{TAG}"));
        let e = fetch("tool", &r.pin, &r.declared, "amd64", &src, &r.root, &r.store).unwrap_err();
        assert!(format!("{e:#}").contains("has no signature over SHA256SUMS"), "{e:#}");
    }
}

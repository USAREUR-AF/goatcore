//! Reading a Sigstore bundle, media type
//! `application/vnd.dev.sigstore.bundle.v0.3+json`, over a release's checksums
//! file (docs/design/goatbuilder-build-targets.md section 3.9, "A second
//! signature format: the acceptance list"). It reads the format and the
//! transparency log's inclusion proof; every other check is signature.rs's own,
//! the same code the older bundle goes through.
//!
//! Strict: the structures below name every field that may be there, and serde
//! refuses an unknown one, a duplicate one and one of the wrong type. A
//! timestamp authority's timestamp may be there and is not read: the only time
//! used is the log's signed entry timestamp, which is required.

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{hex, refuse, Entry, LogKey, Refusal};

/// The one media type read.
pub const MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";
/// The largest bundle read. A release's is about 10 KiB.
pub const MAX_BUNDLE: usize = 128 * 1024;
/// More proof hashes than a tree of 2^63 entries has.
const MAX_HASHES: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Bundle {
    media_type: String,
    verification_material: Material,
    /// Exactly one of these: a signature over a file, or a DSSE envelope.
    #[serde(default)]
    message_signature: Option<MessageSignature>,
    #[serde(default)]
    dsse_envelope: Option<Envelope>,
}

/// A DSSE envelope (in-toto's attestation framework): the payload, its type,
/// and one signature over their pre-authentication encoding.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope {
    payload: String,
    payload_type: String,
    signatures: Vec<EnvelopeSignature>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeSignature {
    sig: String,
    /// Not read: a keyless signature names its key by its certificate.
    #[serde(default)]
    #[allow(dead_code)]
    keyid: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Material {
    certificate: Raw,
    tlog_entries: Vec<LogEntry>,
    /// Not read.
    #[serde(default)]
    #[allow(dead_code)]
    timestamp_verification_data: Option<Timestamps>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Timestamps {
    #[serde(default)]
    #[allow(dead_code)]
    rfc3161_timestamps: Vec<SignedTimestamp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SignedTimestamp {
    #[allow(dead_code)]
    signed_timestamp: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Raw {
    raw_bytes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LogEntry {
    log_index: String,
    log_id: LogId,
    kind_version: KindVersion,
    integrated_time: String,
    inclusion_promise: Promise,
    inclusion_proof: Proof,
    canonicalized_body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LogId {
    key_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindVersion {
    kind: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Promise {
    signed_entry_timestamp: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Proof {
    log_index: String,
    root_hash: String,
    tree_size: String,
    hashes: Vec<String>,
    checkpoint: Checkpoint,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    envelope: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MessageSignature {
    message_digest: MessageDigest,
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageDigest {
    algorithm: String,
    digest: String,
}

/// The log's inclusion proof, as the bundle claims it.
pub(super) struct Inclusion {
    index: u64,
    size: u64,
    root: [u8; 32],
    hashes: Vec<[u8; 32]>,
    checkpoint: String,
}

fn b64(s: &str, what: &str) -> Result<Vec<u8>, Refusal> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| refuse("bundle", format!("the bundle's {what} is not base64: {e}")))
}

fn hash32(s: &str, what: &str) -> Result<[u8; 32], Refusal> {
    b64(s, what)?.try_into().map_err(|_| refuse("bundle", format!("the bundle's {what} is not 32 bytes")))
}

/// A protobuf JSON int64: a string of decimal digits, with no sign, space or
/// leading zero.
fn number(s: &str, what: &str) -> Result<u64, Refusal> {
    let plain = !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    s.parse::<u64>()
        .ok()
        .filter(|n| plain && *n <= i64::MAX as u64)
        .ok_or_else(|| refuse("bundle", format!("the bundle's {what} is not a whole number")))
}

/// At most 60 characters of a claim, escaped, for a refusal's message.
fn shown(s: &str) -> String {
    format!("{:?}", s.chars().take(60).collect::<String>())
}

/// serde reads an array where a structure is expected, its fields in order, and
/// reads null as a field left out. No other reader of this format does.
fn shape(v: &serde_json::Value) -> bool {
    let objects = |v: &serde_json::Value, names: &[&str]| v.is_object() && names.iter().all(|n| v[*n].is_object());
    let each = |v: &serde_json::Value, f: &dyn Fn(&serde_json::Value) -> bool| v.as_array().is_some_and(|a| a.iter().all(f));
    let material = &v["verificationMaterial"];
    let message = v.get("messageSignature").is_none_or(|m| objects(m, &["messageDigest"]));
    let envelope = v.get("dsseEnvelope").is_none_or(|e| e.is_object() && each(&e["signatures"], &|x| x.is_object()));
    objects(v, &["verificationMaterial"])
        && message
        && envelope
        && objects(material, &["certificate"])
        && each(&material["tlogEntries"], &|e| {
            objects(e, &["logId", "kindVersion", "inclusionPromise", "inclusionProof"])
                && objects(&e["inclusionProof"], &["checkpoint"])
        })
        && material.get("timestampVerificationData").is_none_or(|t| {
            t.is_object() && t.get("rfc3161Timestamps").is_none_or(|r| each(r, &|x| x.is_object()))
        })
}

/// A bundle that parses, its media type the one read. Nothing in it is trusted yet.
fn parse(bundle: &str) -> Result<Bundle, Refusal> {
    if bundle.len() > MAX_BUNDLE {
        return Err(refuse("bundle", format!("the bundle is {} bytes, and the most read is {MAX_BUNDLE}", bundle.len())));
    }
    // Where it does not parse, and not what it says there: that is the sender's text.
    let bad = |e: serde_json::Error| {
        let why = match e.classify() {
            serde_json::error::Category::Data => "a field is missing, unknown, repeated or of the wrong type",
            _ => "it is not one JSON document",
        };
        refuse("bundle", format!("it is not a {MEDIA_TYPE} bundle this reads: {why}, at line {} column {}", e.line(), e.column()))
    };
    let b: Bundle = serde_json::from_str(bundle).map_err(bad)?;
    if !shape(&serde_json::from_str(bundle).map_err(bad)?) {
        return Err(refuse("bundle", format!("it is not a {MEDIA_TYPE} bundle this reads: an array or null stands where an object must")));
    }
    if b.media_type != MEDIA_TYPE {
        return Err(refuse("bundle", format!("its media type is {}, not {MEDIA_TYPE}", shown(&b.media_type))));
    }
    Ok(b)
}

/// The one log entry, of `kind` 0.0.1, as an [`Entry`] signed by `sig`, and its proof.
fn entry_of(b: &Bundle, kind: &str, sig: Vec<u8>) -> Result<(Entry, Inclusion), Refusal> {
    let [e] = &b.verification_material.tlog_entries[..] else {
        return Err(refuse("entry", format!("the bundle has {} log entries, not exactly one", b.verification_material.tlog_entries.len())));
    };
    if e.kind_version.kind != kind || e.kind_version.version != "0.0.1" {
        return Err(refuse("entry", format!("the bundle's log entry is {} {}, not {kind} 0.0.1", shown(&e.kind_version.kind), shown(&e.kind_version.version))));
    }
    let p = &e.inclusion_proof;
    if p.hashes.len() > MAX_HASHES {
        return Err(refuse("inclusion", format!("the proof has {} hashes", p.hashes.len())));
    }
    let entry = Entry {
        sig,
        leaf: b64(&b.verification_material.certificate.raw_bytes, "certificate")?,
        body: b64(&e.canonicalized_body, "canonicalizedBody")?,
        time: number(&e.integrated_time, "integratedTime")? as i64,
        index: number(&e.log_index, "logIndex")? as i64,
        log_id: hex(&b64(&e.log_id.key_id, "logId")?),
        set: b64(&e.inclusion_promise.signed_entry_timestamp, "signedEntryTimestamp")?,
    };
    let proof = Inclusion {
        index: number(&p.log_index, "inclusionProof.logIndex")?,
        size: number(&p.tree_size, "inclusionProof.treeSize")?,
        root: hash32(&p.root_hash, "inclusionProof.rootHash")?,
        hashes: p.hashes.iter().map(|h| hash32(h, "inclusionProof.hashes")).collect::<Result<_, _>>()?,
        checkpoint: p.checkpoint.envelope.clone(),
    };
    Ok((entry, proof))
}

/// Read a bundle over `sums`: a message signature and a hashedrekord entry.
/// Nothing in it is trusted yet.
pub(super) fn read(sums: &[u8], bundle: &str) -> Result<(Entry, Inclusion), Refusal> {
    let b = parse(bundle)?;
    if b.dsse_envelope.is_some() {
        return Err(refuse("bundle", "it carries a DSSE envelope: a checksums file's bundle carries a message signature"));
    }
    let Some(m) = &b.message_signature else {
        return Err(refuse("bundle", "it carries no message signature"));
    };
    if m.message_digest.algorithm != "SHA2_256" {
        return Err(refuse("bundle", format!("its digest is {}, not SHA2_256", shown(&m.message_digest.algorithm))));
    }
    if b64(&m.message_digest.digest, "messageDigest")? != Sha256::digest(sums).as_slice() {
        return Err(refuse("signature", "the bundle states the digest of another file"));
    }
    let sig = b64(&m.signature, "signature")?;
    entry_of(&b, "hashedrekord", sig)
}

/// A DSSE envelope's payload and its type, as the bundle has them.
pub(super) struct Dsse {
    pub payload_type: String,
    pub payload: Vec<u8>,
}

/// Read an attestation bundle: a DSSE envelope with one signature and a dsse
/// entry. Nothing in it is trusted yet.
pub(super) fn read_dsse(bundle: &str) -> Result<(Entry, Inclusion, Dsse), Refusal> {
    let b = parse(bundle)?;
    if b.message_signature.is_some() {
        return Err(refuse("bundle", "it carries a message signature: an attestation's bundle carries a DSSE envelope"));
    }
    let Some(env) = &b.dsse_envelope else {
        return Err(refuse("bundle", "it carries no DSSE envelope"));
    };
    let [one] = &env.signatures[..] else {
        return Err(refuse("signature", format!("the envelope has {} signatures, not exactly one", env.signatures.len())));
    };
    let sig = b64(&one.sig, "dsseEnvelope.signatures.sig")?;
    let dsse = Dsse { payload_type: env.payload_type.clone(), payload: b64(&env.payload, "dsseEnvelope.payload")? };
    let (entry, proof) = entry_of(&b, "dsse", sig)?;
    Ok((entry, proof, dsse))
}

fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([1u8]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// RFC 9162 section 2.1.3.2: the root the proof makes from the entry.
fn root_of(body: &[u8], p: &Inclusion) -> Option<[u8; 32]> {
    if p.index >= p.size {
        return None;
    }
    let (mut f, mut s) = (p.index, p.size - 1);
    let mut r: [u8; 32] = {
        let mut h = Sha256::new();
        h.update([0u8]);
        h.update(body);
        h.finalize().into()
    };
    for h in &p.hashes {
        if s == 0 {
            return None;
        }
        if f & 1 == 1 || f == s {
            r = node(h, &r);
            while f & 1 == 0 && f != 0 {
                f >>= 1;
                s >>= 1;
            }
        } else {
            r = node(&r, h);
        }
        f >>= 1;
        s >>= 1;
    }
    (s == 0).then_some(r)
}

impl Inclusion {
    /// `body` is in the tree whose head the log signed: the proof leads from the
    /// entry's body to the checkpoint's root, and the checkpoint is signed by
    /// `key`, the log key that signed the entry's timestamp, under that log's
    /// own name.
    pub(super) fn check(&self, body: &[u8], key: &LogKey) -> Result<(), Refusal> {
        let bad = |why: String| refuse("inclusion", why);
        if root_of(body, self) != Some(self.root) {
            return Err(bad("the inclusion proof does not lead from the entry to the root it states".into()));
        }
        // A signed note: the text, an empty line, then the signature lines.
        let (text, sigs) = self.checkpoint.split_once("\n\n").ok_or_else(|| bad("the checkpoint is not a signed note".into()))?;
        let mut lines = text.split('\n');
        let (origin, size, root) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""), lines.next().unwrap_or(""));
        if number(size, "checkpoint size")? != self.size || hash32(root, "checkpoint root")? != self.root {
            return Err(bad("the checkpoint is for another tree than the proof's".into()));
        }
        // Rekor's origin is "<host> - <tree id>".
        let shard = origin.strip_prefix(key.name.as_str()).and_then(|r| r.strip_prefix(" - "));
        if key.name.is_empty() || !shard.is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())) {
            return Err(bad(format!("the checkpoint's origin is {}, not the log {:?}'s", shown(origin), key.name)));
        }
        // Another witness's line is not ours to judge; the log's own must hold.
        let signed = format!("{text}\n");
        let held = sigs.lines().any(|l| {
            let Some((by, sig)) = l.strip_prefix("\u{2014} ").and_then(|r| r.split_once(' ')) else {
                return false;
            };
            by == key.name && b64(sig, "checkpoint signature").is_ok_and(|raw| raw.len() > 4 && key.id.starts_with(&raw[..4]) && key.verifies(signed.as_bytes(), &raw[4..]))
        });
        if !held {
            return Err(bad("the checkpoint is not signed by the log's key".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The acceptance list's numbers are in the test names.
    use super::super::{stand_in, verify_component, Component, Format, TrustRoot, Verified};
    use super::*;
    use serde_json::{json, Value};

    const REAL_SUMS: &[u8] = include_bytes!("../../tests/fixtures/signature/runner-v0.8.0/SHA256SUMS");
    const REAL: &str = include_str!("../../tests/fixtures/signature/runner-v0.8.0/SHA256SUMS.sigstore.json");
    const REAL_REPO: &str = "USAREUR-AF/goat-processors";
    const REAL_SIGNER: &str = "https://github.com/USAREUR-AF/goat-processors/.github/workflows/runner-release.yml";
    const SUMS: &[u8] = b"a checksums file\n";
    const REPO: &str = "example-org/example-tool";
    const TAG: &str = "tool/v1.2.0";
    const SIGNER: &str = "https://github.com/example-org/example-tool/.github/workflows/_release-binary.yml";

    fn real(sums: &[u8], bundle: &str, tag: &str) -> Result<Verified, Refusal> {
        let root = TrustRoot::parse(super::super::EMBEDDED_ROOT, "embedded").unwrap();
        verify_component(sums, bundle, Format::Sigstore, &Component { repo: REAL_REPO, tag, signer: REAL_SIGNER }, &root)
    }

    fn workflow() -> stand_in::Workflow {
        stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() }
    }

    fn check(s: &stand_in::Sigstore, sums: &[u8], bundle: &str) -> Result<Verified, Refusal> {
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        verify_component(sums, bundle, Format::Sigstore, &Component { repo: REPO, tag: TAG, signer: SIGNER }, &root)
    }

    const PIPE: &str = include_str!("../../tests/fixtures/signature/xf-pipe-v0.1.5/goat-xf-pipe-zenoh_0.1.5_linux_amd64.tar.gz.sigstore.json");
    const PIPE_FILE: &str = "goat-xf-pipe-zenoh_0.1.5_linux_amd64.tar.gz";
    const PIPE_SHA: &str = "b9a154333523a503727eca50511c17ce6aa6bbb655d93b92fc4cd600c9939965";
    const PIPE_C: Component = Component {
        repo: "USAREUR-AF/goat-transformers",
        tag: "xf-pipe-v0.1.5",
        signer: "https://github.com/USAREUR-AF/goat-transformers/.github/workflows/release-pipe.yml",
    };
    const FB: &str = include_str!("../../tests/fixtures/signature/first-bite-v1.1.0/first-bite-1.1.0.intoto.jsonl");
    const FB_FILE: &str = "first-bite-1.1.0-x86_64-linux-gnu.tar.gz";
    const FB_SHA: &str = "668172332f3b9560ab79b91aa488aa841b352b501c28dee68e3585b6d64675ef";
    const FB_C: Component = Component {
        repo: "USAREUR-AF/first-bite",
        tag: "v1.1.0",
        signer: "https://github.com/USAREUR-AF/first-bite/.github/workflows/release.yml",
    };

    fn attested(name: &str, sha: &str, bundle: &str, c: &Component) -> Result<super::super::Attested, Refusal> {
        let root = TrustRoot::parse(super::super::EMBEDDED_ROOT, "embedded").unwrap();
        super::super::verify_attestation(name, sha, bundle, c, &root)
    }

    #[test]
    fn a1_published_github_attestations_of_two_repositories_are_accepted() {
        let a = attested(PIPE_FILE, PIPE_SHA, PIPE, &PIPE_C).unwrap();
        assert_eq!(a.verified.identity, format!("{}@refs/tags/xf-pipe-v0.1.5", PIPE_C.signer));
        let st: Value = serde_json::from_slice(&a.statement).unwrap();
        assert_eq!(st["subject"].as_array().unwrap().len(), 2, "the caller gets the whole statement it may check more of");
        let b = attested(FB_FILE, FB_SHA, FB.trim_end(), &FB_C).unwrap();
        assert_eq!(b.verified.identity, format!("{}@refs/tags/v1.1.0", FB_C.signer));
    }

    #[test]
    fn a2_an_attestation_of_another_file_tag_signer_or_kind_is_refused() {
        let other_sha = "5df5b60c1cd3c8f2297f6b053a5d3efe4ba35489ddba5667c64ba76095f6b55c";
        // The arm64 file is a subject too, but not under this name.
        assert_eq!(attested(PIPE_FILE, other_sha, PIPE, &PIPE_C).unwrap_err().check, "statement");
        assert_eq!(attested("goat-xf-pipe-zenoh_0.1.5_linux_arm64.tar.gz", PIPE_SHA, PIPE, &PIPE_C).unwrap_err().check, "statement");
        assert_eq!(attested(PIPE_FILE, PIPE_SHA, PIPE, &Component { tag: "xf-pipe-v0.1.4", ..PIPE_C }).unwrap_err().check, "identity");
        let other_workflow = Component { signer: "https://github.com/USAREUR-AF/goat-transformers/.github/workflows/other.yml", ..PIPE_C };
        assert_eq!(attested(PIPE_FILE, PIPE_SHA, PIPE, &other_workflow).unwrap_err().check, "identity");
        // Each format is read only where it is asked for.
        assert_eq!(real(REAL_SUMS, PIPE, "runner-v0.8.0").unwrap_err().check, "bundle");
        assert_eq!(attested("SHA256SUMS", &hex(&Sha256::digest(REAL_SUMS)), REAL, &PIPE_C).unwrap_err().check, "bundle");
    }

    #[test]
    fn a3_every_part_of_the_envelope_and_its_entry_is_held() {
        let good: Value = serde_json::from_str(PIPE).unwrap();
        let env = "/dsseEnvelope";
        let entry = "/verificationMaterial/tlogEntries/0";
        let payload = good.pointer(env).unwrap()["payload"].as_str().unwrap().to_string();
        let body: Value = serde_json::from_slice(&b64(good.pointer(entry).unwrap()["canonicalizedBody"].as_str().unwrap(), "x").unwrap()).unwrap();
        let enc = |v: &Value| base64::engine::general_purpose::STANDARD.encode(v.to_string());
        use base64::Engine as _;
        let st: Value = serde_json::from_slice(&b64(&payload, "x").unwrap()).unwrap();
        let mut forged = st.clone();
        forged["subject"][0]["digest"]["sha256"] = json!("0".repeat(64));
        for (want, f) in [
            ("signature", Box::new(|b: &mut Value| b.pointer_mut(env).unwrap()["payloadType"] = json!("application/json")) as Box<dyn Fn(&mut Value)>),
            ("entry", Box::new(move |b| b.pointer_mut(env).unwrap()["payload"] = json!(enc(&forged)))),
            ("signature", Box::new(|b| { let s = b.pointer(env).unwrap()["signatures"][0].clone(); b.pointer_mut(env).unwrap()["signatures"] = json!([s.clone(), s]); })),
            ("signature", Box::new(|b| b.pointer_mut(env).unwrap()["signatures"] = json!([]))),
            ("bundle", Box::new(|b| b.pointer_mut(env).unwrap()["extra"] = json!(1))),
            ("bundle", Box::new(|b| b.pointer_mut(env).unwrap()["signatures"][0]["extra"] = json!(1))),
            ("bundle", Box::new(|b| b["messageSignature"] = json!({"messageDigest": {"algorithm": "SHA2_256", "digest": ""}, "signature": ""}))),
            ("entry", Box::new(|b| b.pointer_mut(entry).unwrap()["kindVersion"] = json!({"kind": "hashedrekord", "version": "0.0.1"}))),
            ("entry", Box::new(|b| b.pointer_mut(entry).unwrap()["kindVersion"] = json!({"kind": "dsse", "version": "0.0.2"}))),
            ("timestamp", Box::new(|b| b.pointer_mut(entry).unwrap()["integratedTime"] = json!("1700000000"))),
            ("inclusion", Box::new(|b| b.pointer_mut(entry).unwrap()["inclusionProof"]["logIndex"] = json!("1"))),
        ] {
            let mut b = good.clone();
            f(&mut b);
            let e = attested(PIPE_FILE, PIPE_SHA, &b.to_string(), &PIPE_C).unwrap_err();
            assert_eq!(e.check, want, "{e}");
        }
        // The entry's body must name this payload and this one signature by this certificate;
        // a body changed is not the one the log signed.
        assert_eq!(body["kind"], "dsse");
        let mut b = good.clone();
        let mut changed = body.clone();
        changed["spec"]["payloadHash"]["value"] = json!("0".repeat(64));
        b.pointer_mut(entry).unwrap()["canonicalizedBody"] = json!(enc(&changed));
        assert_eq!(attested(PIPE_FILE, PIPE_SHA, &b.to_string(), &PIPE_C).unwrap_err().check, "timestamp");
    }

    #[test]
    fn a4_the_logs_entry_names_this_payload_this_one_signature_and_this_certificate() {
        let s = stand_in::Sigstore::new();
        let w = stand_in::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        let files: &[(&str, &[u8])] = &[("tool.tar.gz", b"a build\n")];
        let sha = hex(&Sha256::digest(b"a build\n"));
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        let c = Component { repo: REPO, tag: TAG, signer: SIGNER };
        let run = |b: &str| super::super::verify_attestation("tool.tar.gz", &sha, b, &c, &root);
        let a = run(&s.attestation_v03(files, &w)).unwrap();
        assert!(a.verified.identity.ends_with("@refs/tags/tool/v1.2.0"));
        for (body, words) in [
            (stand_in::DsseBody::Kind, "not dsse 0.0.1"),
            (stand_in::DsseBody::TwoSignatures, "exactly one signature"),
            (stand_in::DsseBody::OtherSignature, "another signature"),
            (stand_in::DsseBody::OtherVerifier, "another certificate"),
        ] {
            let e = run(&s.attestation_v03_with(files, &w, body)).unwrap_err();
            assert!(e.check == "entry" && e.detail.contains(words), "{words}: {e}");
        }
        // An attestation is never read as a checksums file's signature.
        assert_eq!(verify_component(b"x", &s.attestation_v03(files, &w), Format::Attestation, &c, &root).unwrap_err().check, "bundle");
    }

    #[test]
    fn item_9_a_published_bundle_that_cosign_verifies_is_accepted() {
        let v = real(REAL_SUMS, REAL, "runner-v0.8.0").unwrap();
        assert_eq!(v.identity, format!("{REAL_SIGNER}@refs/tags/runner-v0.8.0"));
    }

    #[test]
    fn item_9_a_bundle_for_another_file_tag_or_signer_is_refused() {
        assert_eq!(real(b"other\n", REAL, "runner-v0.8.0").unwrap_err().check, "signature");
        assert_eq!(real(REAL_SUMS, REAL, "runner-v0.7.0").unwrap_err().check, "identity");
        let s = stand_in::Sigstore::new();
        assert!(check(&s, SUMS, &s.bundle_v03(SUMS, &workflow())).is_ok());
        for w in [
            stand_in::Workflow { repo: "example-org/other".into(), ..workflow() },
            stand_in::Workflow { tag: "tool/v1.1.0".into(), ..workflow() },
        ] {
            assert_eq!(check(&s, SUMS, &s.bundle_v03(SUMS, &w)).unwrap_err().check, "identity");
        }
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        let other = Component { repo: REPO, tag: TAG, signer: "https://github.com/example-org/example-tool/.github/workflows/other.yml" };
        assert_eq!(verify_component(SUMS, &s.bundle_v03(SUMS, &workflow()), Format::Sigstore, &other, &root).unwrap_err().check, "identity");
    }

    #[test]
    fn items_3_5_and_6_a_bad_leaf_is_refused_as_in_the_older_bundle() {
        let s = stand_in::Sigstore::new();
        for (leaf, name) in [
            (stand_in::Leaf { expired: true, ..Default::default() }, "chain"),
            (stand_in::Leaf { ca: true, ..Default::default() }, "chain"),
            (stand_in::Leaf { unknown_critical: true, ..Default::default() }, "chain"),
            (stand_in::Leaf { no_code_signing: true, ..Default::default() }, "chain"),
            (stand_in::Leaf { issuer: Some("https://issuer.example.org"), ..Default::default() }, "identity"),
            (stand_in::Leaf { entry_of_another: true, ..Default::default() }, "entry"),
            (stand_in::Leaf { sct: stand_in::Sct::None, ..Default::default() }, "sct"),
            (stand_in::Leaf { sct: stand_in::Sct::UnknownLog, ..Default::default() }, "sct"),
            (stand_in::Leaf { sct: stand_in::Sct::OverOther, ..Default::default() }, "sct"),
        ] {
            assert_eq!(check(&s, SUMS, &s.bundle_v03_with(SUMS, &workflow(), leaf, stand_in::Checkpoint::Good)).unwrap_err().check, name);
        }
        // Another Sigstore's bundle, whole and well signed.
        let t = stand_in::Sigstore::new();
        assert!(check(&s, SUMS, &t.bundle_v03(SUMS, &workflow())).is_err());
    }

    #[test]
    fn item_5_a_checkpoint_the_log_did_not_sign_for_this_tree_is_refused() {
        use stand_in::Checkpoint as C;
        let s = stand_in::Sigstore::new();
        let with = |c| check(&s, SUMS, &s.bundle_v03_with(SUMS, &workflow(), stand_in::Leaf::default(), c));
        for c in [C::Good, C::Witnessed, C::Origin("rekor.example.org - 0")] {
            assert!(with(c).is_ok());
        }
        for (c, words) in [
            (C::OtherKey, "is not signed by the log's key"),
            (C::OtherName, "is not signed by the log's key"),
            (C::OtherHint, "is not signed by the log's key"),
            (C::OtherRoot, "for another tree"),
            (C::OtherSize, "for another tree"),
            (C::Origin("rekor.example.org"), "origin"),
            (C::Origin("rekor.example.org - "), "origin"),
            (C::Origin("rekor.example.org - x"), "origin"),
            (C::Origin("other.example.org - 1234"), "origin"),
            (C::Origin("rekor.example.org.other - 1234"), "origin"),
        ] {
            let e = with(c).unwrap_err();
            assert!(e.check == "inclusion" && e.detail.contains(words), "{words}: {e}");
        }
        let g: Value = serde_json::from_str(&s.bundle_v03(SUMS, &workflow())).unwrap();
        let mut b = g.clone();
        b["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["hashes"] = json!(vec![json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="); MAX_HASHES + 1]);
        assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "inclusion");
        // An index at or past the tree's size, and a tree of no entry, in the bundle itself.
        let proof = "/verificationMaterial/tlogEntries/0/inclusionProof";
        let size = g.pointer(proof).unwrap()["treeSize"].as_str().unwrap().to_string();
        let past = (size.parse::<u64>().unwrap() + 1).to_string();
        for (index, tree) in [(size.as_str(), size.as_str()), (past.as_str(), size.as_str()), ("0", "0")] {
            let mut b = g.clone();
            b.pointer_mut(proof).unwrap()["logIndex"] = json!(index);
            b.pointer_mut(proof).unwrap()["treeSize"] = json!(tree);
            assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "inclusion", "{index} of {tree}");
        }
    }

    #[test]
    fn item_8_the_trusted_roots_are_read_together_and_nothing_else() {
        let s = stand_in::Sigstore::new();
        let good = s.bundle_v03(SUMS, &workflow());
        let c = Component { repo: REPO, tag: TAG, signer: SIGNER };
        let verdict = |roots: &[String]| {
            let mut r = TrustRoot::parse(super::super::EMBEDDED_ROOT, "embedded").unwrap();
            roots.iter().for_each(|t| r.add(TrustRoot::parse(t, "operator").unwrap()));
            verify_component(SUMS, &good, Format::Sigstore, &c, &r)
        };
        let own = s.trusted_root();
        assert!(verdict(&[]).is_err());
        assert!(verdict(std::slice::from_ref(&own)).is_ok());
        // The same log key under an older name does not hide the right one.
        let old = own.replace("rekor.example.org", "old.example.org");
        assert_eq!(verdict(std::slice::from_ref(&old)).unwrap_err().check, "inclusion");
        assert!(verdict(&[old, own.clone()]).is_ok());
        for url in ["https://rekor.example.org/", "https://REKOR.example.org:443", "http://rekor.example.org"] {
            assert!(verdict(&[own.replace("https://rekor.example.org", url)]).is_ok(), "{url}");
        }
        // A log with an Ed25519 key signs no entry timestamps.
        let ed = stand_in::Sigstore::with_ed25519_log();
        assert_eq!(check(&ed, SUMS, &ed.bundle_v03(SUMS, &workflow())).unwrap_err().check, "timestamp");
        // A log key or a CA outside its window.
        for at in ["tlogs", "certificateAuthorities"] {
            let mut v: Value = serde_json::from_str(&own).unwrap();
            let w = if at == "tlogs" { &mut v[at][0]["publicKey"]["validFor"] } else { &mut v[at][0]["validFor"] };
            w["end"] = json!("2021-01-01T00:00:00Z");
            assert!(verdict(&[v.to_string()]).is_err(), "{at}");
        }
    }

    #[test]
    fn item_1_an_array_or_null_where_an_object_must_be_is_refused_and_not_echoed() {
        let s = stand_in::Sigstore::new();
        let mut good: Value = serde_json::from_str(&s.bundle_v03(SUMS, &workflow())).unwrap();
        good["verificationMaterial"]["timestampVerificationData"] = json!({"rfc3161Timestamps": [{"signedTimestamp": "AAAA"}]});
        assert!(check(&s, SUMS, &good.to_string()).is_ok());
        // Each structure's fields in the order this reader declares them.
        let order = ["mediaType", "verificationMaterial", "messageSignature", "certificate", "tlogEntries", "timestampVerificationData", "logIndex", "logId", "kindVersion", "integratedTime", "inclusionPromise", "inclusionProof", "rootHash", "treeSize", "hashes", "checkpoint", "canonicalizedBody", "kind", "version", "messageDigest", "algorithm", "digest", "signature", "rawBytes", "keyId", "signedEntryTimestamp", "envelope", "rfc3161Timestamps", "signedTimestamp"];
        let mut paths = Vec::new();
        objects(&good, String::new(), &mut paths);
        for p in &paths {
            let o = good.pointer(p).unwrap().as_object().unwrap();
            let fields: Vec<Value> = order.iter().filter_map(|k| o.get(*k).cloned()).collect();
            assert_eq!(fields.len(), o.len(), "{p}");
            for other in [json!(fields), json!(null)] {
                let mut b = good.clone();
                *b.pointer_mut(p).unwrap() = other;
                assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "bundle", "{p}");
            }
        }
        let mut b = good.clone();
        b["\u{1b}[2Jowned"] = json!(1);
        let e = check(&s, SUMS, &b.to_string()).unwrap_err();
        assert!(!e.detail.contains('\u{1b}') && !e.detail.contains("owned"), "{e}");
        let mut b = good.clone();
        b["mediaType"] = json!("\u{1b}".repeat(90_000));
        assert!(check(&s, SUMS, &b.to_string()).unwrap_err().detail.len() < 1000);
    }

    #[test]
    fn item_7_neither_format_is_read_as_the_other() {
        let s = stand_in::Sigstore::new();
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        let c = Component { repo: REPO, tag: TAG, signer: SIGNER };
        let (old, new) = (s.bundle(SUMS, &workflow()), s.bundle_v03(SUMS, &workflow()));
        assert!(verify_component(SUMS, &old, Format::Cosign, &c, &root).is_ok());
        assert_eq!(verify_component(SUMS, &old, Format::Sigstore, &c, &root).unwrap_err().check, "bundle");
        assert_eq!(verify_component(SUMS, &new, Format::Cosign, &c, &root).unwrap_err().check, "bundle");
    }

    /// Every path to a leaf of the JSON.
    fn leaves(v: &Value, at: String, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => o.iter().for_each(|(k, v)| leaves(v, format!("{at}/{k}"), out)),
            Value::Array(a) => a.iter().enumerate().for_each(|(i, v)| leaves(v, format!("{at}/{i}"), out)),
            _ => out.push(at),
        }
    }

    /// Every path to an object of the JSON.
    fn objects(v: &Value, at: String, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                o.iter().for_each(|(k, v)| objects(v, format!("{at}/{k}"), out));
                out.push(at);
            }
            Value::Array(a) => a.iter().enumerate().for_each(|(i, v)| objects(v, format!("{at}/{i}"), out)),
            _ => {}
        }
    }

    #[test]
    fn items_1_to_5_and_9_every_field_changed_removed_retyped_or_added_is_refused() {
        for (good, root) in [(REAL.to_string(), None), {
            let s = stand_in::Sigstore::new();
            (s.bundle_v03(SUMS, &workflow()), Some(s))
        }] {
            let refused = |b: &Value, what: &str| {
                let r = match &root {
                    None => real(REAL_SUMS, &b.to_string(), "runner-v0.8.0"),
                    Some(s) => check(s, SUMS, &b.to_string()),
                };
                assert!(r.is_err(), "accepted: {what}");
            };
            let g: Value = serde_json::from_str(&good).unwrap();
            let mut paths = Vec::new();
            leaves(&g, String::new(), &mut paths);
            for p in paths.iter().filter(|p| !p.contains("rfc3161Timestamps")) {
                let text = g.pointer(p).unwrap().as_str().unwrap().to_string();
                for i in [0, text.len() / 2, text.len().saturating_sub(3)] {
                    let mut c: Vec<char> = text.chars().collect();
                    let Some(x) = c.get_mut(i) else { continue };
                    *x = if *x == '1' { '2' } else { '1' };
                    let mut b = g.clone();
                    *b.pointer_mut(p).unwrap() = json!(c.iter().collect::<String>());
                    refused(&b, &format!("{p} at {i}"));
                }
                for other in [json!(""), json!(null), json!(1), json!({}), json!([])] {
                    let mut b = g.clone();
                    *b.pointer_mut(p).unwrap() = other;
                    refused(&b, &format!("{p} replaced"));
                }
                let (parent, key) = p.rsplit_once('/').unwrap();
                let mut b = g.clone();
                match b.pointer_mut(parent).unwrap() {
                    Value::Object(o) => drop(o.remove(key)),
                    Value::Array(a) => drop(a.remove(key.parse().unwrap())),
                    _ => unreachable!(),
                }
                refused(&b, &format!("{p} removed"));
            }
            let mut paths = Vec::new();
            objects(&g, String::new(), &mut paths);
            for p in &paths {
                let mut b = g.clone();
                b.pointer_mut(p).unwrap()["added"] = json!(1);
                refused(&b, &format!("an unknown field in {p:?}"));
            }
        }
    }

    #[test]
    fn item_1_a_duplicate_key_trailing_text_an_oversize_bundle_or_an_array_is_refused() {
        let s = stand_in::Sigstore::new();
        let good = s.bundle_v03(SUMS, &workflow());
        let dup = good.replacen("\"logIndex\":\"7\"", "\"logIndex\":\"7\",\"logIndex\":\"7\"", 1);
        assert_ne!(dup, good);
        let big = format!("{good}{}", " ".repeat(MAX_BUNDLE));
        for b in [dup, format!("{good}x"), big, String::new(), "[]".into()] {
            assert_eq!(check(&s, SUMS, &b).unwrap_err().check, "bundle");
        }
    }

    #[test]
    fn items_1_2_and_6_another_version_an_envelope_a_key_or_a_chain_is_refused() {
        let s = stand_in::Sigstore::new();
        let good: Value = serde_json::from_str(&s.bundle_v03(SUMS, &workflow())).unwrap();
        for m in ["application/vnd.dev.sigstore.bundle+json;version=0.3", "application/vnd.dev.sigstore.bundle+json;version=0.2", "application/vnd.dev.sigstore.bundle.v0.4+json"] {
            let mut b = good.clone();
            b["mediaType"] = json!(m);
            assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "bundle");
        }
        let mut dsse = good.clone();
        dsse.as_object_mut().unwrap().remove("messageSignature");
        dsse["dsseEnvelope"] = json!({"payload": "", "payloadType": "", "signatures": []});
        let mut both = good.clone();
        both["dsseEnvelope"] = dsse["dsseEnvelope"].clone();
        let mut key = good.clone();
        let cert = key["verificationMaterial"].as_object_mut().unwrap().remove("certificate").unwrap();
        key["verificationMaterial"]["publicKey"] = json!({"hint": "x"});
        let mut chain = good.clone();
        chain["verificationMaterial"]["x509CertificateChain"] = json!({"certificates": [cert]});
        for b in [dsse, both, key, chain] {
            assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "bundle");
        }
    }

    #[test]
    fn items_4_and_5_the_time_and_the_one_entry_are_the_logs() {
        let s = stand_in::Sigstore::new();
        let good: Value = serde_json::from_str(&s.bundle_v03(SUMS, &workflow())).unwrap();
        // A timestamp authority's timestamp changes nothing.
        let mut tsa = good.clone();
        tsa["verificationMaterial"]["timestampVerificationData"] = json!({"rfc3161Timestamps": [{"signedTimestamp": "AAAA"}]});
        assert_eq!(check(&s, SUMS, &tsa.to_string()), check(&s, SUMS, &good.to_string()));
        // It does not stand in for the log's.
        let e = "/verificationMaterial/tlogEntries/0";
        for k in ["inclusionPromise", "inclusionProof", "integratedTime"] {
            let mut b = tsa.clone();
            b.pointer_mut(e).unwrap().as_object_mut().unwrap().remove(k);
            assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "bundle");
        }
        let entry = good.pointer(e).unwrap().clone();
        for n in [vec![], vec![entry.clone(), entry]] {
            let mut b = good.clone();
            b["verificationMaterial"]["tlogEntries"] = json!(n);
            assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "entry");
        }
        // Another file's entry, and another file's proof.
        let other: Value = serde_json::from_str(&s.bundle_v03(b"other\n", &workflow())).unwrap();
        let mut b = good.clone();
        *b.pointer_mut(e).unwrap() = other.pointer(e).unwrap().clone();
        assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "entry");
        let mut b = good.clone();
        b.pointer_mut(e).unwrap()["inclusionProof"] = other.pointer(e).unwrap()["inclusionProof"].clone();
        assert_eq!(check(&s, SUMS, &b.to_string()).unwrap_err().check, "inclusion");
    }

    #[test]
    fn item_9_random_corruption_never_verifies_as_anything_else() {
        let s = stand_in::Sigstore::new();
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        let good = s.bundle_v03(SUMS, &workflow());
        let c = Component { repo: REPO, tag: TAG, signer: SIGNER };
        let want = verify_component(SUMS, &good, Format::Sigstore, &c, &root).unwrap();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..20_000 {
            let mut b = good.clone().into_bytes();
            for _ in 0..=next(3) {
                let i = next(b.len());
                match next(4) {
                    0 => b[i] = next(256) as u8,
                    1 => drop(b.remove(i)),
                    2 => b.insert(i, next(256) as u8),
                    _ => b.truncate(i.max(1)),
                }
            }
            if let Ok(v) = verify_component(SUMS, &String::from_utf8_lossy(&b), Format::Sigstore, &c, &root) {
                assert_eq!(v, want);
            }
        }
    }

    #[test]
    fn the_proof_is_rfc_9162s_for_every_tree_up_to_seventeen_leaves() {
        fn lh(i: u8) -> [u8; 32] {
            Sha256::new().chain_update([0, i]).finalize().into()
        }
        fn tree(l: &[u8]) -> [u8; 32] {
            match l {
                [i] => lh(*i),
                _ => {
                    let k = l.len().next_power_of_two() / 2;
                    node(&tree(&l[..k]), &tree(&l[k..]))
                }
            }
        }
        fn path(i: usize, l: &[u8]) -> Vec<[u8; 32]> {
            if l.len() == 1 {
                return vec![];
            }
            let k = l.len().next_power_of_two() / 2;
            let (mut p, other) = if i < k { (path(i, &l[..k]), tree(&l[k..])) } else { (path(i - k, &l[k..]), tree(&l[..k])) };
            p.push(other);
            p
        }
        let l: Vec<u8> = (0..17).collect();
        for n in 1..=l.len() {
            for i in 0..n {
                let mut p = Inclusion { index: i as u64, size: n as u64, root: tree(&l[..n]), hashes: path(i, &l[..n]), checkpoint: String::new() };
                assert_eq!(root_of(&[i as u8], &p), Some(p.root), "{i} of {n}");
                assert_ne!(root_of(&[99], &p), Some(p.root));
                p.hashes.push([0; 32]);
                assert_ne!(root_of(&[i as u8], &p), Some(p.root));
                p.hashes.pop();
                p.index = n as u64;
                assert_eq!(root_of(&[i as u8], &p), None);
            }
        }
    }
}

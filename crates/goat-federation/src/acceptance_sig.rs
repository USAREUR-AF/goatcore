// A partner's signed acceptance result — docs/design/federation-acceptance.md §6.1.
//
// Phase 14 on each side writes its acceptance result (`result.json`,
// `goat-federation-acceptance/v1`). For the command to complete the partner's
// half of acceptance (gate row B2), each side signs its result and the other
// verifies it under the key bound to that side at phase 5, the same Ed25519
// key that signs the side's federation package.
//
// One key, two kinds of message, so the two must never be confused. A result
// is not signed as JSON: the result file can hold `null` and fractions, which
// the package's canonical form refuses. What is signed is a fixed-format text
// STATEMENT whose first line is the domain, `goat-acceptance-result/v1`, and
// which names the agreement, the session, the attempt, both parties, the
// challenge echoed, the sha256 of the exact result bytes and the sha256 of the
// signing key. Everything else this key signs begins with another byte (§6.1
// lists them), and each signer refuses the other's input, so a signature over
// one kind can never verify as the other.
//
// Pure: no I/O, no clock, no environment. The seed and the verifying keys are
// parameters.

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::compose::KeyRing;
use crate::json::{canonical, parse_document, JVal};
use crate::package::decode_signature;

/// The first line of every acceptance-result statement, newline included.
pub const RESULT_DOMAIN: &str = "goat-acceptance-result/v1\n";

/// The schema of the detached signature document published beside a result.
pub const SIGNATURE_SCHEMA: &str = "goat-acceptance-signature/v1";

/// The partner's result does not verify (catalog GB-272).
pub const NOT_VERIFIED: &str = "GB-272";
/// The partner's result is for another agreement, session or attempt, names
/// other parties, or does not echo our challenge (catalog GB-273).
pub const NOT_THIS_SESSION: &str = "GB-273";

/// What one side signs, besides the result bytes themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    /// The agreement's sha256, 64 lowercase hex characters.
    pub agreement_sha256: String,
    /// `ses-` and then letters, digits and hyphens.
    pub session_id: String,
    /// The attempt number, from 1.
    pub attempt: i64,
    /// The signer's sovereign id.
    pub by: String,
    /// The partner's sovereign id: the side that verifies.
    pub for_: String,
    /// The partner's challenge, as the signer's run observed it.
    pub echoes: String,
}

/// What the verifier expects: its own view of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expect {
    pub agreement_sha256: String,
    pub session_id: String,
    pub attempt: i64,
    /// Our sovereign id.
    pub us: String,
    /// The partner's sovereign id: the signer.
    pub them: String,
    /// The challenge issued to us, which the partner's result must echo.
    pub our_challenge: String,
}

/// A refusal, with the catalog code it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub detail: String,
}

/// A verified result: what the evidence keeps beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The sha256 of the partner's key the signature verified under, hex.
    pub key_sha256: String,
    /// That key, the 32 raw bytes in standard base64, stored beside the
    /// evidence so it can be read back against the key bound when it was signed.
    pub key_b64: String,
    /// The sha256 of the result bytes, hex.
    pub result_sha256: String,
}

fn refuse(code: &'static str, detail: impl Into<String>) -> Refusal {
    Refusal {
        code,
        detail: detail.into(),
    }
}

/// The exact bytes Ed25519 signs for a result.
pub fn statement(f: &Fields, result_sha256: &str, key_sha256: &str) -> String {
    let _ = (f, result_sha256, key_sha256);
    unimplemented!("statement")
}

/// Sign a result: the detached signature document, in canonical form.
pub fn sign_result(seed: &[u8; 32], f: &Fields, result: &[u8]) -> Result<String, Refusal> {
    let _ = (seed, f, result);
    unimplemented!("sign_result")
}

/// Verify a partner's result against its detached signature document, the key
/// bound to the partner, and our own view of the session.
pub fn verify_result(
    result: &[u8],
    signature_doc: &[u8],
    keys: &KeyRing,
    expect: &Expect,
) -> Result<Verified, Refusal> {
    let _ = (
        result,
        signature_doc,
        keys,
        expect,
        canonical,
        parse_document,
    );
    let _: Option<JVal> = None;
    let _ = decode_signature;
    let _ = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(b""));
    let _ = refuse(NOT_VERIFIED, "");
    unimplemented!("verify_result")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::parse_package;

    const XAA_SEED: &str = include_str!("../../../ops/federation/vectors/testkeys/xaa.seed.hex");
    const XAB_SEED: &str = include_str!("../../../ops/federation/vectors/testkeys/xab.seed.hex");
    const XAA_PUB: &str = include_str!("../../../ops/federation/vectors/testkeys/xaa.public.b64");
    const XAB_PUB: &str = include_str!("../../../ops/federation/vectors/testkeys/xab.public.b64");
    const RESULT: &[u8] = include_bytes!("testdata/acceptance/result-xab.json");
    const SIGNATURE: &[u8] = include_bytes!("testdata/acceptance/signature-xab.json");

    fn seed(hex: &str) -> [u8; 32] {
        let h = hex.trim();
        let mut b = [0u8; 32];
        for (i, out) in b.iter_mut().enumerate() {
            *out = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap();
        }
        b
    }

    fn key(b64: &str) -> [u8; 32] {
        crate::package::decode_verifying_key(b64).unwrap()
    }

    fn ring() -> KeyRing {
        let mut k = KeyRing::default();
        k.insert("xaa", key(XAA_PUB));
        k.insert("xab", key(XAB_PUB));
        k
    }

    // The session the fixture belongs to: xab signs its result for xaa.
    const AGREEMENT: &str = "3d2bd7e59ac1d1cdb4fe6a7b4e3a85f0cba0ddd04a4ab1a63e64ab1a8b7b0f5d";
    const SESSION: &str = "ses-xaa-xab-20261109-1";
    const XAA_CHALLENGE: &str = "6f1c2a9e4b7d03e5a8c1f2b3d4e5f607";

    fn fields() -> Fields {
        Fields {
            agreement_sha256: AGREEMENT.into(),
            session_id: SESSION.into(),
            attempt: 1,
            by: "xab".into(),
            for_: "xaa".into(),
            echoes: XAA_CHALLENGE.into(),
        }
    }

    fn expect() -> Expect {
        Expect {
            agreement_sha256: AGREEMENT.into(),
            session_id: SESSION.into(),
            attempt: 1,
            us: "xaa".into(),
            them: "xab".into(),
            our_challenge: XAA_CHALLENGE.into(),
        }
    }

    fn signed(f: &Fields, seed_hex: &str, result: &[u8]) -> Vec<u8> {
        sign_result(&seed(seed_hex), f, result)
            .unwrap()
            .into_bytes()
    }

    fn code(r: Result<Verified, Refusal>) -> &'static str {
        r.expect_err("expected a refusal").code
    }

    // 1
    #[test]
    fn a_result_signed_by_the_bound_key_verifies() {
        let sig = signed(&fields(), XAB_SEED, RESULT);
        let v = verify_result(RESULT, &sig, &ring(), &expect()).unwrap();
        let want_key = hex(&Sha256::digest(key(XAB_PUB)));
        assert_eq!(v.key_sha256, want_key);
        assert_eq!(v.key_b64, XAB_PUB.trim());
        assert_eq!(v.result_sha256, hex(&Sha256::digest(RESULT)));
    }

    // 2
    #[test]
    fn one_changed_byte_in_the_result_or_the_signature_document_is_refused() {
        let sig = signed(&fields(), XAB_SEED, RESULT);
        let mut tampered = RESULT.to_vec();
        let at = tampered.iter().position(|&c| c == b'P').unwrap(); // a PASS becomes QASS
        tampered[at] = b'Q';
        assert_eq!(
            code(verify_result(&tampered, &sig, &ring(), &expect())),
            NOT_VERIFIED
        );

        // The signature document's own fields are under the signature too: a
        // forged result digest that matches the tampered bytes does not verify.
        let doc = String::from_utf8(sig).unwrap();
        let forged = doc.replace(
            &hex(&Sha256::digest(RESULT)),
            &hex(&Sha256::digest(&tampered)),
        );
        assert_ne!(doc, forged);
        assert_eq!(
            code(verify_result(
                &tampered,
                forged.as_bytes(),
                &ring(),
                &expect()
            )),
            NOT_VERIFIED
        );
    }

    // 3
    #[test]
    fn a_result_signed_by_another_partys_key_is_refused() {
        // xaa's seed signs a statement that says it is xab's.
        let sig = signed(&fields(), XAA_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_VERIFIED
        );
    }

    // 4
    #[test]
    fn a_package_signature_is_not_a_result_signature() {
        // xab's real package signature, made by xab's key over its package's
        // signing input, put in place of the result signature.
        let pkg = parse_package(include_bytes!(
            "../../../ops/federation/vectors/compose-full/package-xab.canonical.json"
        ))
        .unwrap();
        let pkg_sig = base64::engine::general_purpose::STANDARD.encode(pkg.signature_bytes);
        let doc = String::from_utf8(signed(&fields(), XAB_SEED, RESULT)).unwrap();
        let ours = signature_member(&doc);
        let swapped = doc.replace(&ours, &pkg_sig);
        assert_eq!(
            code(verify_result(
                RESULT,
                swapped.as_bytes(),
                &ring(),
                &expect()
            )),
            NOT_VERIFIED
        );

        // And the same key signing the statement WITHOUT its domain line does
        // not make a result signature either.
        let s = statement(
            &fields(),
            &hex(&Sha256::digest(RESULT)),
            &hex(&Sha256::digest(key(XAB_PUB))),
        );
        let bare = s.strip_prefix(RESULT_DOMAIN).unwrap();
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed(XAB_SEED));
        let bare_sig = base64::engine::general_purpose::STANDARD
            .encode(ed25519_dalek::Signer::sign(&sk, bare.as_bytes()).to_bytes());
        let swapped = doc.replace(&ours, &bare_sig);
        assert_eq!(
            code(verify_result(
                RESULT,
                swapped.as_bytes(),
                &ring(),
                &expect()
            )),
            NOT_VERIFIED
        );
    }

    // 5
    #[test]
    fn a_result_for_another_agreement_session_or_attempt_is_refused() {
        let mut other = fields();
        other.agreement_sha256 = "0".repeat(64);
        let sig = signed(&other, XAB_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );

        let mut other = fields();
        other.session_id = "ses-xaa-xab-20261109-2".into();
        let sig = signed(&other, XAB_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );

        let mut other = fields();
        other.attempt = 2;
        let sig = signed(&other, XAB_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );
    }

    // 6
    #[test]
    fn a_result_that_does_not_echo_our_challenge_is_refused() {
        let mut other = fields();
        other.echoes = "0123456789abcdef0123456789abcdef".into();
        let sig = signed(&other, XAB_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );
    }

    // 7
    #[test]
    fn a_result_by_us_or_for_another_partner_is_refused() {
        // Signed by xaa, for xab: our own result handed back to us.
        let mut ours = fields();
        ours.by = "xaa".into();
        ours.for_ = "xab".into();
        let sig = signed(&ours, XAA_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );

        // Signed by xab, for a third party.
        let mut elsewhere = fields();
        elsewhere.for_ = "xac".into();
        let sig = signed(&elsewhere, XAB_SEED, RESULT);
        assert_eq!(
            code(verify_result(RESULT, &sig, &ring(), &expect())),
            NOT_THIS_SESSION
        );
    }

    // 8
    #[test]
    fn a_missing_or_malformed_signature_document_is_refused_by_name() {
        let doc = String::from_utf8(signed(&fields(), XAB_SEED, RESULT)).unwrap();
        let sig = signature_member(&doc);
        for (what, bad) in [
            ("not JSON", "not json".to_string()),
            (
                "another schema",
                doc.replace(SIGNATURE_SCHEMA, "goat-acceptance-signature/v2"),
            ),
            ("a short signature", doc.replace(&sig, &sig[4..])),
            (
                "no signature",
                doc.replace(&format!(",\"signature\":\"{sig}\""), ""),
            ),
            ("no result digest", remove_member(&doc, "result_sha256")),
        ] {
            let r = verify_result(RESULT, bad.as_bytes(), &ring(), &expect());
            let e = r.expect_err(what);
            assert_eq!(e.code, NOT_VERIFIED, "{what}: {}", e.detail);
            assert!(
                !e.detail.is_empty(),
                "{what}: the refusal names what is wrong"
            );
        }
    }

    // 9
    #[test]
    fn the_statement_binds_the_exact_result_bytes_and_refuses_a_field_that_could_add_a_line() {
        let sig = signed(&fields(), XAB_SEED, RESULT);
        // The result is covered byte for byte, not as JSON: one more newline at
        // its end is another result.
        let mut more = RESULT.to_vec();
        more.push(b'\n');
        assert_eq!(
            code(verify_result(&more, &sig, &ring(), &expect())),
            NOT_VERIFIED
        );

        // A field cannot carry a newline into the statement.
        let mut f = fields();
        f.session_id = "ses-1\nresult sha256:00".into();
        assert!(sign_result(&seed(XAB_SEED), &f, RESULT).is_err());

        // The statement is exactly its documented lines.
        let s = statement(&fields(), "aa", "bb");
        assert_eq!(
            s,
            format!(
                "goat-acceptance-result/v1\nagreement sha256:{AGREEMENT}\nsession {SESSION}\n\
                 attempt 1\nby xab\nfor xaa\nechoes {XAA_CHALLENGE}\nresult sha256:aa\nkey sha256:bb\n"
            )
        );
    }

    // 10
    #[test]
    fn the_committed_signature_document_is_a_vector_that_recomputes() {
        // Ed25519 is deterministic: signing the fixture again gives the
        // committed document byte for byte, and the committed one verifies.
        let again = sign_result(&seed(XAB_SEED), &fields(), RESULT).unwrap();
        assert_eq!(
            again.as_bytes(),
            SIGNATURE.strip_suffix(b"\n").unwrap_or(SIGNATURE)
        );
        verify_result(RESULT, SIGNATURE, &ring(), &expect()).unwrap();
    }

    // 11 — the key that signs results also signs packages (phase 3). What it
    // signs for a package is the package's signing input, the canonical form
    // of a JSON object, so it always begins with `{`: it can never begin with
    // the result domain. Checked on the real path: every vector package's
    // signing input as `Package::signing_input` builds it, the input `init`
    // signs (the same function), the committed signing-input files, and the
    // conformance attestation line `verify` signs with the same key.
    #[test]
    fn what_phase_3_signs_for_a_package_can_never_begin_with_the_result_domain() {
        let vectors =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors");
        let mut seen = 0;
        for entry in std::fs::read_dir(&vectors).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(&dir).unwrap() {
                let p = f.unwrap().path();
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let bytes = std::fs::read(&p).unwrap();
                if name.ends_with(".signing-input.canonical.json") {
                    assert_eq!(bytes.first(), Some(&b'{'), "{}", p.display());
                    seen += 1;
                }
                if name.starts_with("package-")
                    && name.ends_with(".canonical.json")
                    && !name.contains("signing-input")
                {
                    if let Ok(pkg) = parse_package(&bytes) {
                        let input = pkg.signing_input();
                        assert!(input.starts_with('{'), "{}", p.display());
                        assert!(!input.starts_with(RESULT_DOMAIN));
                        seen += 1;
                    }
                }
            }
        }
        assert!(
            seen >= 10,
            "read only {seen} signing inputs: the vectors moved?"
        );
        let line = crate::render::attestation_line(&"a".repeat(64));
        assert!(line.starts_with("conformance ok for "));
        assert!(!line.starts_with(RESULT_DOMAIN));
        // And the package signer refuses the result domain outright, so no
        // caller can make it sign a statement.
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path().join("xab.seed.hex");
        std::fs::write(&k, XAB_SEED).unwrap();
        let s = statement(&fields(), "aa", "bb");
        assert!(crate::verbs::sign_with(&k, s.as_bytes()).is_err());
        // While a package's signing input still signs.
        assert!(crate::verbs::sign_with(&k, b"{\"a\":1}").is_ok());
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn signature_member(doc: &str) -> String {
        let v = parse_document(doc.as_bytes()).unwrap();
        v.get("signature")
            .and_then(JVal::as_str)
            .unwrap()
            .to_string()
    }

    fn remove_member(doc: &str, name: &str) -> String {
        canonical(&parse_document(doc.as_bytes()).unwrap().without_member(name))
    }
}

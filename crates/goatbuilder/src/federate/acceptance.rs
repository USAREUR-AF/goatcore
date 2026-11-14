//! goatbuilder's signer and verifier of a phase 14 acceptance result, on a
//! session both sides derive (docs/design/federation-acceptance.md §6.1, §6.3).
//!
//! acceptance.sh calls these through FED_ACCEPTANCE_SIGNER and
//! FED_ACCEPTANCE_VERIFIER with the arguments below, so the signature is made
//! and checked by the one implementation, `goat_federation::acceptance_sig`.
//! The signing key is ours, `secrets.federation.signing_key` (FED_SIGNING_KEY);
//! the verifying key is the partner's, read from the peering's keyring
//! (`<work>/keys.txt`, FED_KEYRING), which phase 5 writes after the key is
//! confirmed on the call. It is never taken from the result.
//!
//! A refusal prints `GB-272: …` or `GB-273: …` on stderr and exits 2, which is
//! how acceptance.sh tells a stale result (keep waiting) from a bad one.

// No command reaches this module until its two commands are wired; the
// allowance goes with that wiring.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use goat_federation::acceptance_sig::{Expect, Fields, Refusal, NOT_VERIFIED};

/// `sign`: the arguments acceptance.sh passes to FED_ACCEPTANCE_SIGNER.
#[derive(clap::Args, Debug, Clone)]
pub struct SignArgs {
    #[arg(long = "agreement-sha256")]
    pub agreement_sha256: String,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub attempt: i64,
    #[arg(long)]
    pub by: String,
    #[arg(long = "for")]
    pub for_: String,
    #[arg(long)]
    pub echoes: String,
    #[arg(long)]
    pub result: PathBuf,
    /// Our Ed25519 seed: 64 hex characters or 32 bytes base64.
    #[arg(long, env = "FED_SIGNING_KEY")]
    pub signing_key: Option<PathBuf>,
}

/// `verify`: the arguments acceptance.sh passes to FED_ACCEPTANCE_VERIFIER.
#[derive(clap::Args, Debug, Clone)]
pub struct VerifyArgs {
    #[arg(long = "agreement-sha256")]
    pub agreement_sha256: String,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub attempt: i64,
    #[arg(long)]
    pub us: String,
    #[arg(long)]
    pub them: String,
    #[arg(long = "our-challenge")]
    pub our_challenge: String,
    #[arg(long)]
    pub result: PathBuf,
    #[arg(long)]
    pub signature: PathBuf,
    /// The peering's keyring, `<sovereign id>=<base64 key>` per line.
    #[arg(long, env = "FED_KEYRING")]
    pub keyring: Option<PathBuf>,
}

fn refuse(detail: impl Into<String>) -> Refusal {
    Refusal {
        code: NOT_VERIFIED,
        detail: detail.into(),
    }
}

/// Our Ed25519 seed, 64 hex characters or 32 bytes base64, as FED_SIGNING_KEY
/// holds it for the package (confirm.rs reads it the same way).
fn read_seed(path: Option<&Path>) -> Result<[u8; 32], Refusal> {
    use base64::Engine as _;
    let Some(path) = path else {
        return Err(refuse(
            "no signing key: set FED_SIGNING_KEY, or secrets.federation.signing_key, to our Ed25519 seed",
        ));
    };
    let text =
        std::fs::read_to_string(path).map_err(|e| refuse(format!("{}: {e}", path.display())))?;
    let t = text.trim();
    let bytes: Option<Vec<u8>> = if t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit()) {
        (0..32)
            .map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).ok())
            .collect()
    } else {
        base64::engine::general_purpose::STANDARD.decode(t).ok()
    };
    bytes
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| {
            refuse(format!(
                "{}: not an Ed25519 seed (64 hex characters or 32 bytes base64)",
                path.display()
            ))
        })
}

/// The keys bound at phase 5, one `<sovereign id>=<base64 key>` per line, as
/// peering-standup.sh reads the same file for compose.
fn read_keyring(path: Option<&Path>) -> Result<goat_federation::compose::KeyRing, Refusal> {
    let Some(path) = path else {
        return Err(refuse(
            "no keyring: set FED_KEYRING to the peering's keys.txt, which phase 5 writes",
        ));
    };
    let text =
        std::fs::read_to_string(path).map_err(|e| refuse(format!("{}: {e}", path.display())))?;
    let mut ring = goat_federation::compose::KeyRing::default();
    for line in text.lines() {
        let l: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((sid, b64)) = l.split_once('=') else {
            return Err(refuse(format!(
                "{}: `{l}` is not `<sovereign id>=<base64 key>`",
                path.display()
            )));
        };
        let key = goat_federation::package::decode_verifying_key(b64)
            .map_err(|e| refuse(format!("{}: the key for `{sid}`: {e}", path.display())))?;
        ring.insert(sid, key);
    }
    Ok(ring)
}

/// Sign our result: the detached signature document.
pub fn sign(a: &SignArgs) -> Result<String, Refusal> {
    let seed = read_seed(a.signing_key.as_deref())?;
    let result =
        std::fs::read(&a.result).map_err(|e| refuse(format!("{}: {e}", a.result.display())))?;
    let f = Fields {
        agreement_sha256: a.agreement_sha256.clone(),
        session_id: a.session.clone(),
        attempt: a.attempt,
        by: a.by.clone(),
        for_: a.for_.clone(),
        echoes: a.echoes.clone(),
    };
    goat_federation::acceptance_sig::sign_result(&seed, &f, &result)
}

/// Verify the partner's result: `{key_b64, key_sha256, result_sha256}` as JSON.
pub fn verify(a: &VerifyArgs) -> Result<String, Refusal> {
    let keys = read_keyring(a.keyring.as_deref())?;
    let result =
        std::fs::read(&a.result).map_err(|e| refuse(format!("{}: {e}", a.result.display())))?;
    let doc = std::fs::read(&a.signature)
        .map_err(|e| refuse(format!("{}: {e}", a.signature.display())))?;
    let expect = Expect {
        agreement_sha256: a.agreement_sha256.clone(),
        session_id: a.session.clone(),
        attempt: a.attempt,
        us: a.us.clone(),
        them: a.them.clone(),
        our_challenge: a.our_challenge.clone(),
    };
    let v = goat_federation::acceptance_sig::verify_result(&result, &doc, &keys, &expect)?;
    Ok(serde_json::json!({
        "key_b64": v.key_b64,
        "key_sha256": v.key_sha256,
        "result_sha256": v.result_sha256,
    })
    .to_string())
}

/// Run one of the two and print as acceptance.sh reads it: the answer on
/// stdout and exit 0, or `<code>: <detail>` on stderr and exit 2.
pub fn print(answer: Result<String, Refusal>) -> i32 {
    match answer {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(r) => {
            eprintln!("{}: {}", r.code, r.detail);
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../goat-federation/src/testdata/acceptance"
    );
    const KEYS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../ops/federation/vectors/testkeys"
    );
    const AGREEMENT: &str = "3d2bd7e59ac1d1cdb4fe6a7b4e3a85f0cba0ddd04a4ab1a63e64ab1a8b7b0f5d";
    const SESSION: &str = "ses-xaa-xab-20261109-1";
    const XAA_CHALLENGE: &str = "6f1c2a9e4b7d03e5a8c1f2b3d4e5f607";

    fn key_line(sid: &str) -> String {
        let b64 = std::fs::read_to_string(format!("{KEYS}/{sid}.public.b64")).unwrap();
        format!("{sid}={}", b64.trim())
    }

    struct World {
        dir: tempfile::TempDir,
    }

    impl World {
        fn new(keyring: &[&str]) -> World {
            let dir = tempfile::tempdir().unwrap();
            let lines: Vec<String> = keyring.iter().map(|s| key_line(s)).collect();
            std::fs::write(dir.path().join("keys.txt"), lines.join("\n") + "\n").unwrap();
            std::fs::copy(
                format!("{DATA}/result-xab.json"),
                dir.path().join("result.json"),
            )
            .unwrap();
            World { dir }
        }
        fn p(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
        fn sign_args(&self) -> SignArgs {
            SignArgs {
                agreement_sha256: AGREEMENT.into(),
                session: SESSION.into(),
                attempt: 1,
                by: "xab".into(),
                for_: "xaa".into(),
                echoes: XAA_CHALLENGE.into(),
                result: self.p("result.json"),
                signing_key: Some(PathBuf::from(format!("{KEYS}/xab.seed.hex"))),
            }
        }
        fn verify_args(&self) -> VerifyArgs {
            VerifyArgs {
                agreement_sha256: AGREEMENT.into(),
                session: SESSION.into(),
                attempt: 1,
                us: "xaa".into(),
                them: "xab".into(),
                our_challenge: XAA_CHALLENGE.into(),
                result: self.p("result.json"),
                signature: self.p("signature.json"),
                keyring: Some(self.p("keys.txt")),
            }
        }
        fn signed(&self) {
            std::fs::write(self.p("signature.json"), sign(&self.sign_args()).unwrap()).unwrap();
        }
    }

    #[test]
    fn our_signature_is_the_committed_vector_and_verifies_under_the_peerings_keyring() {
        let w = World::new(&["xaa", "xab"]);
        let doc = sign(&w.sign_args()).unwrap();
        let committed = std::fs::read_to_string(format!("{DATA}/signature-xab.json")).unwrap();
        assert_eq!(doc, committed.trim_end());
        std::fs::write(w.p("signature.json"), &doc).unwrap();
        let out: serde_json::Value =
            serde_json::from_str(&verify(&w.verify_args()).unwrap()).unwrap();
        let pub_b64 = key_line("xab").split_once('=').unwrap().1.to_string();
        assert_eq!(out["key_b64"], pub_b64);
        assert!(out["key_sha256"].as_str().unwrap().len() == 64);
        assert!(out["result_sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn a_partner_with_no_key_in_the_keyring_is_refused() {
        let w = World::new(&["xaa"]);
        w.signed();
        let e = verify(&w.verify_args()).unwrap_err();
        assert_eq!(e.code, "GB-272", "{}", e.detail);
        assert!(e.detail.contains("xab"), "{}", e.detail);
    }

    #[test]
    fn a_changed_result_is_gb_272_and_a_stale_echo_is_gb_273() {
        let w = World::new(&["xaa", "xab"]);
        w.signed();
        let mut stale = w.verify_args();
        stale.our_challenge = "0f1e2d3c4b5a69788796a5b4c3d2e1f0".into();
        assert_eq!(verify(&stale).unwrap_err().code, "GB-273");
        let mut bytes = std::fs::read(w.p("result.json")).unwrap();
        bytes.push(b'\n');
        std::fs::write(w.p("result.json"), bytes).unwrap();
        assert_eq!(verify(&w.verify_args()).unwrap_err().code, "GB-272");
    }

    #[test]
    fn signing_without_a_readable_seed_is_refused_by_name() {
        let w = World::new(&["xaa", "xab"]);
        let mut a = w.sign_args();
        a.signing_key = None;
        let e = sign(&a).unwrap_err();
        assert!(e.detail.contains("FED_SIGNING_KEY"), "{}", e.detail);
        a.signing_key = Some(w.p("no-such-seed"));
        assert!(sign(&a).is_err());
        std::fs::write(w.p("short-seed"), "abcd").unwrap();
        a.signing_key = Some(w.p("short-seed"));
        assert!(sign(&a).unwrap_err().detail.contains("not an Ed25519 seed"));
    }

    /// The key comes from the keyring only: a result naming a key the keyring
    /// does not bind to the partner is refused, even though it names one.
    #[test]
    fn a_result_naming_a_key_outside_the_keyring_is_refused_gb_272() {
        let w = World::new(&["xaa", "xab"]);
        // Signed with xaa's seed, claiming to be xab's, naming xaa's key.
        let mut a = w.sign_args();
        a.signing_key = Some(PathBuf::from(format!("{KEYS}/xaa.seed.hex")));
        std::fs::write(w.p("signature.json"), sign(&a).unwrap()).unwrap();
        let e = verify(&w.verify_args()).unwrap_err();
        assert_eq!(e.code, "GB-272", "{}", e.detail);
        assert!(e.detail.contains("the key bound to `xab`"), "{}", e.detail);
    }

    /// Our key unset, unreadable, or still an unresolved secret reference (run
    /// outside ops/scripts/with-secrets) is refused with GB-142, as phase 3 does.
    #[test]
    fn an_unresolved_signing_key_is_refused_gb_142() {
        let w = World::new(&["xaa", "xab"]);
        let mut a = w.sign_args();
        for key in [
            None,
            Some(PathBuf::from("op://federation/signing-key")),
            Some(w.p("missing")),
        ] {
            a.signing_key = key.clone();
            let e = sign(&a).unwrap_err();
            assert_eq!(e.code, "GB-142", "{key:?}: {}", e.detail);
            assert!(e.detail.contains("signing key"), "{}", e.detail);
            assert!(e.detail.ends_with("(GB-142)"), "{}", e.detail);
        }
        a.signing_key = Some(PathBuf::from("op://federation/signing-key"));
        assert!(sign(&a).unwrap_err().detail.contains("with-secrets"));
    }

    #[test]
    fn a_refusal_is_printed_as_its_code_and_exits_2() {
        assert_eq!(print(Ok("{}".into())), 0);
        let r = Refusal {
            code: "GB-273",
            detail: "stale".into(),
        };
        assert_eq!(print(Err(r)), 2);
    }

    /// The flags acceptance.sh passes are exactly the ones these commands take,
    /// read from the runner itself, so the two cannot drift.
    #[test]
    fn the_runner_passes_exactly_these_flags() {
        use clap::CommandFactory as _;
        #[derive(clap::Parser)]
        struct S {
            #[command(flatten)]
            a: SignArgs,
        }
        #[derive(clap::Parser)]
        struct V {
            #[command(flatten)]
            a: VerifyArgs,
        }
        let runner = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ops/federation/standup/acceptance.sh"
        ))
        .unwrap();
        for (marker, cmd) in [
            ("$FED_ACCEPTANCE_SIGNER", S::command()),
            ("$FED_ACCEPTANCE_VERIFIER", V::command()),
        ] {
            let start = runner
                .find(&format!("{marker} --agreement-sha256"))
                .expect(marker);
            let mut call = String::new();
            for l in runner[start..].lines() {
                call.push_str(l);
                call.push(' ');
                if !l.trim_end().ends_with('\\') {
                    break;
                }
            }
            let passed: Vec<String> = call
                .split_whitespace()
                .filter(|w| w.starts_with("--"))
                .map(|w| w.trim_start_matches("--").to_string())
                .collect();
            let known: Vec<String> = cmd
                .get_arguments()
                .filter_map(|a| a.get_long())
                .map(String::from)
                .collect();
            for f in &passed {
                assert!(
                    known.contains(f),
                    "{marker} passes --{f}, which the command does not take"
                );
            }
            for a in cmd.get_arguments().filter(|a| a.is_required_set()) {
                let long = a.get_long().unwrap();
                assert!(
                    passed.iter().any(|f| f == long),
                    "{marker} does not pass the required --{long}"
                );
            }
        }
    }
}

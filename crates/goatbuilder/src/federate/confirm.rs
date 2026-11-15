//! Confirmation 1: the keys read aloud at phase 5, and the agreement digest
//! compared after phase 6.
//!
//! A key that arrives inside the package it verifies proves nothing, so the
//! partner's verifying key is bound by a person: read aloud on a call, then
//! typed or confirmed here, and written to the keyring by this command. The
//! partner's CA fingerprint is taken from their package and checked against
//! the CA certificate that arrived; the person confirms it is the one they
//! heard. After compose, both sides compare the agreement digest the same way.
//!
//! This runs only when asked for: `--confirm` (ask at a terminal), or the
//! values themselves (`--partner-key`, `--partner-ca-fingerprint`,
//! `--agreement-digest`, read aloud on the call). Without a terminal nothing
//! is taken as confirmed: every value comes by flag, a missing one stops the
//! run where it always stopped, and `--yes` confirms nothing. Every value
//! given is recorded in the journal with how it was given.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;

use super::prompt::{self, groups};
use super::run::{Paths, Surface};
use super::state::State;
use crate::Check;

/// What a confirmation step decided.
pub enum Verdict {
    /// Done; carry on.
    Continue,
    /// Waiting for a person; the run stops, exit 0, as a by-hand stop does.
    Stop(String),
    /// Failed; the run stops, exit 1.
    Fail(String),
}

pub struct Step {
    pub rows: Vec<Check>,
    pub verdict: Verdict,
}

fn ok(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

fn fail(code: &str, detail: impl Into<String>, fix: impl Into<String>) -> Step {
    let d = format!("{code}: {}", detail.into());
    Step {
        rows: vec![Check {
            name: "failed".into(),
            ok: false,
            detail: d.clone(),
            fix: Some(fix.into()),
        }],
        verdict: Verdict::Fail(d),
    }
}

fn stop(detail: impl Into<String>, fix: impl Into<String>) -> Step {
    let d = format!("GB-192: {}", detail.into());
    Step {
        rows: vec![Check {
            name: "stopped".into(),
            ok: true,
            detail: d.clone(),
            fix: Some(fix.into()),
        }],
        verdict: Verdict::Stop(d),
    }
}

fn field(v: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    cur.as_str().map(String::from)
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// `sha256:<hex>` over DER, for every certificate in a PEM file.
pub fn pem_fingerprints(pem: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pem);
    let mut out = Vec::new();
    let mut body: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if l == "-----BEGIN CERTIFICATE-----" {
            body = Some(String::new());
        } else if l == "-----END CERTIFICATE-----" {
            if let Some(b) = body.take() {
                if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(b.as_bytes()) {
                    out.push(sha256(&der));
                }
            }
        } else if let Some(b) = body.as_mut() {
            b.push_str(l);
        }
    }
    out
}

/// Our verifying key, base64, from the Ed25519 seed that signs our package
/// (FED_SIGNING_KEY: 64 hex characters or 32 bytes base64, as the directory
/// step reads it).
pub fn our_verifying_key(seed_path: &str) -> Result<String, String> {
    if seed_path.is_empty() {
        return Err("FED_SIGNING_KEY is not set".into());
    }
    let text = std::fs::read_to_string(seed_path).map_err(|e| format!("{seed_path}: {e}"))?;
    let t = text.trim();
    let not_seed =
        || format!("{seed_path}: not an Ed25519 seed (64 hex characters or 32 bytes base64)");
    let seed: Vec<u8> = if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        (0..32)
            .map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(t)
            .map_err(|_| not_seed())?
    };
    let seed: [u8; 32] = seed.try_into().map_err(|_| not_seed())?;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    Ok(base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().as_bytes()))
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A verifying key is 32 bytes, base64 (44 characters).
pub fn is_verifying_key(k: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(squeeze(k).as_bytes())
        .map(|b| b.len() == 32)
        .unwrap_or(false)
}

/// The keyring with `id=key` set for each pair, other lines kept as they were.
pub fn write_keyring(path: &Path, pairs: &[(&str, &str)]) -> std::io::Result<()> {
    let old = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| {
            let c = squeeze(l);
            !pairs.iter().any(|(id, _)| c.starts_with(&format!("{id}=")))
        })
        .map(String::from)
        .collect();
    for (id, key) in pairs {
        lines.push(format!("{id}={}", squeeze(key)));
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    std::fs::rename(tmp, path)
}

/// The values a person gives for confirmation 1, by flag or at a terminal.
#[derive(Default)]
pub struct Values {
    /// `--confirm`: ask at a terminal for what no flag gives.
    pub ask: bool,
    pub partner_key: Option<String>,
    pub partner_ca_fingerprint: Option<String>,
    pub agreement_digest: Option<String>,
    /// `--place-pointer-package <digest>`: place the package their single
    /// pointer named, the one with this digest (see `pointer`).
    pub pointer_package: Option<String>,
    /// `--accept-partner-package <digest>`: take the partner's new package,
    /// signed by the key already bound, the one with this digest (see
    /// `incoming`; ADR 1229).
    pub partner_package: Option<String>,
}

impl Values {
    /// Whether confirmation 1 was asked for at all.
    pub fn active(&self) -> bool {
        self.ask
            || self.partner_key.is_some()
            || self.partner_ca_fingerprint.is_some()
            || self.agreement_digest.is_some()
            || self.pointer_package.is_some()
            || self.partner_package.is_some()
    }
}

/// A sovereign id as it may appear in the keyring: plain characters only, so
/// a value from a partner's package can never write a second keyring line.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// Whether `package` is signed by `key` (base64): the check compose makes,
/// made here so a misheard key stops at phase 5 instead of at compose.
fn signed_by(package: &[u8], key: &str) -> Result<bool, String> {
    let pkg = goat_federation::package::parse_package(package).map_err(|f| format!("{f:?}"))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(squeeze(key).as_bytes())
        .map_err(|e| e.to_string())?;
    let raw: [u8; 32] = raw.try_into().map_err(|_| "not 32 bytes".to_string())?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&raw).map_err(|e| e.to_string())?;
    let sig = ed25519_dalek::Signature::from_bytes(&pkg.signature_bytes);
    Ok(ed25519_dalek::Verifier::verify(&vk, pkg.signing_input().as_bytes(), &sig).is_ok())
}

/// Phase 5, after the directory step: bind the partner's key and CA
/// fingerprint, write the keyring.
pub fn exchange(
    paths: &Paths,
    s: &mut Surface,
    st: &mut State,
    run: &str,
    rerun_directory: &dyn Fn() -> bool,
) -> Step {
    let interactive = s.values.ask && s.asker().interactive();
    let Ok(ours_bytes) = std::fs::read(&paths.ours) else {
        return fail(
            "GB-191",
            format!("our half is not at {}", paths.ours.display()),
            "run phase 3 again: goatbuilder federate <partner> --from 3",
        );
    };
    let ours: serde_json::Value = serde_json::from_slice(&ours_bytes).unwrap_or_default();
    let (Some(us), Some(our_fp)) = (
        field(&ours, &["sovereign_id"]),
        field(&ours, &["identity", "ca_fingerprint"]),
    ) else {
        return fail(
            "GB-191",
            format!(
                "{} has no sovereign_id or identity.ca_fingerprint",
                paths.ours.display()
            ),
            "run phase 3 again: goatbuilder federate <partner> --from 3",
        );
    };
    let seed = paths
        .signing
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let our_key = match our_verifying_key(&seed) {
        Ok(k) => k,
        Err(e) => {
            return fail(
                "GB-193",
                format!("our verifying key cannot be read: {e}"),
                "set FED_SIGNING_KEY to the Ed25519 seed that signed our package",
            )
        }
    };
    if !matches!(signed_by(&ours_bytes, &our_key), Ok(true)) {
        return fail(
            "GB-193",
            format!("FED_SIGNING_KEY is not the seed that signed our half ({}): the key we would read aloud does not verify it", paths.ours.display()),
            "set FED_SIGNING_KEY to the seed phase 3 signed with, or run phase 3 again with this one",
        );
    }
    let Ok(theirs_bytes) = std::fs::read(&paths.counterparty) else {
        return stop(
            format!("their half is not at {} yet", paths.counterparty.display()),
            "run phase 5 again after they publish theirs (the same command)",
        );
    };
    let theirs: serde_json::Value = serde_json::from_slice(&theirs_bytes).unwrap_or_default();
    let (Some(them), Some(their_fp)) = (
        field(&theirs, &["sovereign_id"]),
        field(&theirs, &["identity", "ca_fingerprint"]),
    ) else {
        return fail(
            "GB-191",
            format!(
                "{} has no sovereign_id or identity.ca_fingerprint",
                paths.counterparty.display()
            ),
            "ask the partner to publish their package again",
        );
    };
    if !plain_id(&them) || !plain_id(&us) || them == us {
        return fail("GB-191", format!("the sovereign ids in the two halves ('{us}', '{them}') must be two different plain ids"), "check both halves are the right packages");
    }
    if let Some(want) = crate::federate::runenv::var("FED_COUNTERPARTY_ID")
        .ok()
        .filter(|v| !v.is_empty())
    {
        if want != them {
            return fail("GB-191", format!("their half is from '{them}', and this peering's counterparty is '{want}' (FED_COUNTERPARTY_ID)"), "ask which package is theirs; do not carry on with another party's");
        }
    }
    // Their CA: in place, or saved by the directory as `.unconfirmed` until
    // their key is bound (per-counterparty mode); either way it must carry
    // their package's fingerprint.
    let unconfirmed_ca = paths.their_ca.with_file_name(format!(
        "{}.unconfirmed",
        paths
            .their_ca
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    let (ca_file, ca_unconfirmed) = if paths.their_ca.exists() {
        (paths.their_ca.clone(), false)
    } else {
        (unconfirmed_ca.clone(), true)
    };
    match std::fs::read(&ca_file) {
        Ok(pem) if pem_fingerprints(&pem).contains(&their_fp) => {}
        Ok(_) => return fail(
            "GB-191",
            format!(
                "no certificate in {} has their package's CA fingerprint {their_fp}",
                ca_file.display()
            ),
            "do not use it; ask the partner which CA is current and place that certificate there",
        ),
        Err(_) => {
            return stop(
                format!(
                    "their CA certificate is not at {} yet",
                    paths.their_ca.display()
                ),
                "run phase 5 again once it is there",
            )
        }
    }

    let mut rows = Vec::new();
    if interactive {
        s.asker().say(&format!(
            "\nPhase 5: read the keys aloud\n  On one call with {them}, read yours and listen to theirs. Each side reads\n  the other's back. A key that arrives inside their package proves nothing.\n\n  Yours ({us}):\n    verifying key:   {}\n    CA fingerprint:  {}\n  Theirs ({them}), from their package and the CA certificate that arrived:\n    CA fingerprint:  {}\n",
            groups(&our_key),
            groups(&our_fp),
            groups(&their_fp)
        ));
    } else {
        rows.push(ok(
            "read-aloud",
            format!(
                "read these to {them}: our verifying key {our_key}, our CA fingerprint {our_fp}"
            ),
        ));
    }

    // Their CA fingerprint: a flag's value must be theirs; at a terminal the
    // person says it is the one they heard.
    let fp_how = match s.values.partner_ca_fingerprint.clone() {
        Some(v) if prompt::same_value(&v, &their_fp) => "flag",
        Some(v) => {
            return fail(
                "GB-191",
                format!("--partner-ca-fingerprint {v} is not their package's CA fingerprint {their_fp}"),
                "check the value read aloud with the partner; do not carry on with a fingerprint that differs",
            )
        }
        None if interactive => {
            let a = s.asker().ask(&format!("Did {them} read this CA fingerprint aloud? Type yes, or press Enter to stop:"));
            if !prompt::is_yes(&a) {
                return stop("their CA fingerprint was not confirmed; nothing was written", "run the same command again when you are ready");
            }
            "terminal"
        }
        None => "",
    };

    // Their verifying key: given by flag, or typed as it is read aloud. It is
    // never offered from anything that arrived over the wire.
    let (their_key, key_how) = match s.values.partner_key.clone() {
        Some(k) => (squeeze(&k), "flag"),
        None if interactive => {
            match s.asker().ask(&format!("Type the verifying key {them} reads aloud (44 characters), or press Enter to stop:")).filter(|t| !t.is_empty()) {
                Some(k) => (squeeze(&k), "terminal"),
                None => return stop("their verifying key was not confirmed; nothing was written", "run the same command again when you are ready"),
            }
        }
        None => (String::new(), ""),
    };
    let mut missing = Vec::new();
    if key_how.is_empty() {
        missing.push("--partner-key");
    }
    if fp_how.is_empty() {
        missing.push("--partner-ca-fingerprint");
    }
    if !missing.is_empty() {
        let mut st = stop(
            format!(
                "waiting for the keys read aloud: pass {} (or --confirm at a terminal)",
                missing.join(" and ")
            ),
            "run the same command again with the values read aloud on the call",
        );
        st.rows.splice(0..0, rows);
        return st;
    }
    if !is_verifying_key(&their_key) {
        return fail(
            "GB-191",
            format!("'{their_key}' is not a verifying key (32 bytes, base64, 44 characters)"),
            "read it back with the partner group by group, then run phase 5 again",
        );
    }
    match signed_by(&theirs_bytes, &their_key) {
        Ok(true) => {}
        Ok(false) => {
            return fail(
                "GB-191",
                format!("their half ({}) is not signed by the key read aloud; nothing was written", paths.counterparty.display()),
                "read the key back with the partner group by group; if it is right, their package is not the one they signed",
            )
        }
        Err(e) => return fail("GB-191", format!("their half ({}) is not a package whose signature can be checked: {e}", paths.counterparty.display()), "ask the partner to publish their package again"),
    }
    if let Err(e) = write_keyring(&paths.keyring, &[(&us, &our_key), (&them, &their_key)]) {
        return fail(
            "GB-184",
            format!("cannot write the keyring {}: {e}", paths.keyring.display()),
            "check the keyring's folder is writable (FED_KEYRING or <work>/keys.txt)",
        );
    }
    // With their key bound, the directory step checks and places the CA and
    // bundle it held as `.unconfirmed`.
    if ca_unconfirmed || paths.work.join("theirs.spiffe-bundle.unconfirmed").exists() {
        let _ = rerun_directory();
        let ok_now = std::fs::read(&paths.their_ca)
            .map(|pem| pem_fingerprints(&pem).contains(&their_fp))
            .unwrap_or(false);
        if !ok_now {
            return fail(
                "GB-191",
                format!("the directory step did not place their CA at {} once their key was bound", paths.their_ca.display()),
                "run phase 5 again; the keyring now holds their key, so the directory step can check the CA",
            );
        }
        rows.push(ok(
            "directory",
            "the directory step placed their CA (and bundle) now that their key is bound",
        ));
    }
    let how = if key_how == "flag" && fp_how == "flag" {
        "flag"
    } else {
        "terminal"
    };
    // Recorded with what it was given for: their half and their CA as they
    // arrived (a new package is a new question on the next run).
    let ca_bytes = std::fs::read(&paths.their_ca).unwrap_or_default();
    let bound = super::state::digest(&[
        ("theirs", theirs_bytes.as_slice()),
        ("theirs-ca", ca_bytes.as_slice()),
    ]);
    st.confirm(
        run,
        5,
        "keys",
        how,
        &bound,
        BTreeMap::from([
            ("us".to_string(), us),
            ("our_key".to_string(), our_key),
            ("our_ca_fingerprint".to_string(), our_fp),
            ("partner".to_string(), them.clone()),
            ("partner_key".to_string(), their_key),
            ("partner_key_how".to_string(), key_how.to_string()),
            ("partner_ca_fingerprint".to_string(), their_fp),
            ("partner_ca_fingerprint_how".to_string(), fp_how.to_string()),
            ("partner_package".to_string(), sha256(&theirs_bytes)),
        ])
        .into_iter()
        .chain(issued(&theirs_bytes).map(|d| ("partner_package_issued".to_string(), d)))
        .collect(),
    );
    rows.push(ok("confirm", format!("keys confirmed ({how}): {them}'s verifying key verifies their half, and their CA fingerprint is the one read aloud; both keys written to {}", paths.keyring.display())));
    Step {
        rows,
        verdict: Verdict::Continue,
    }
}

/// The package the directory held back: their single pointer named it
/// because they host no package for us, so it may be the package of another
/// of their peerings (the directory step saved it at
/// `<work>/theirs.pointer.json`). It is held when this run's directory step
/// wrote it and nothing is placed, or when it is placed and was never
/// confirmed. `None` when nothing is held.
fn held_pointer(
    paths: &Paths,
    st: &State,
    since: Option<std::time::SystemTime>,
) -> Option<Vec<u8>> {
    let file = paths.work.join("theirs.pointer.json");
    let body = std::fs::read(&file).ok()?;
    match std::fs::read(&paths.counterparty) {
        Ok(placed) if placed == body => st
            .confirmed("pointer-package", 5, &sha256(&body))
            .is_none()
            .then_some(body),
        Ok(_) => None,
        Err(_) => {
            let fresh = match (since, std::fs::metadata(&file).and_then(|m| m.modified())) {
                (Some(t), Ok(m)) => m >= t,
                _ => true,
            };
            fresh.then_some(body)
        }
    }
}

/// Phase 5, after the directory step: when it held back a package their
/// pointer named, a person confirms it is the package for this peering (or
/// `--place-pointer-package <its digest>`), and `rerun` runs the directory
/// step again with FED_DIRECTORY_PLACE_POINTER_PACKAGE=1, which places it
/// and fetches the CA and bundle it vouches for. What it placed must be the
/// package confirmed. `None` when nothing is held.
pub fn pointer(
    paths: &Paths,
    s: &mut Surface,
    st: &mut State,
    run: &str,
    since: Option<std::time::SystemTime>,
    rerun: &dyn Fn() -> bool,
) -> Option<Step> {
    let body = held_pointer(paths, st, since)?;
    let digest = sha256(&body);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let them = field(&v, &["sovereign_id"]).unwrap_or_else(|| "(no sovereign_id)".into());
    let expires = field(&v, &["expires"]).unwrap_or_default();
    let interactive = s.values.ask && s.asker().interactive();
    let text = format!(
        "\nPhase 5: the directory held back a package\n  {them} hosts no package for us, so the directory fetched the package their\n  single pointer names. It may be the package of another of {them}'s peerings,\n  signed by the same key. Confirm on the call that it is the package for this one.\n    sovereign id:  {them}\n    expires:       {expires}\n    digest:        {}\n    saved at:      {}\n",
        groups(&digest),
        paths.work.join("theirs.pointer.json").display()
    );
    let how = match s.values.pointer_package.clone() {
        Some(d) if prompt::same_value(&d, &digest) => "flag",
        Some(d) => {
            return Some(fail(
                "GB-191",
                format!(
                "--place-pointer-package {d} is not the digest of the package held back ({digest})"
            ),
                "read the digest again with the partner; a different package arrived",
            ))
        }
        None if interactive => {
            s.asker().say(&text);
            if !prompt::is_yes(&s.asker().ask("Is this the package for this peering? Type yes to place it, or press Enter to stop:")) {
                return Some(stop("the package their pointer names was not confirmed; nothing was placed", "ask them to publish a package for us, or run again once it is confirmed on the call"));
            }
            "terminal"
        }
        None => {
            let mut step = stop(
                format!("the package their pointer names is held back: confirm on the call that it is the package for this peering, then pass --place-pointer-package {digest}"),
                "run the same command again with the flag, or ask them to publish a package for us",
            );
            step.rows.insert(0, ok("pointer-package", text));
            return Some(step);
        }
    };
    // Placed already (an earlier run placed it and never confirmed it):
    // the confirmation is all that was missing. Otherwise the directory step
    // places it now; what it placed must be what was confirmed, or it is
    // taken away again with the CA and bundle it brought.
    if std::fs::read(&paths.counterparty).ok().as_deref() != Some(body.as_slice()) {
        let brought = [
            paths.counterparty.clone(),
            paths.their_ca.clone(),
            paths.work.join("theirs.spiffe-bundle"),
        ];
        let before: Vec<bool> = brought.iter().map(|p| p.exists()).collect();
        let placed =
            rerun() && std::fs::read(&paths.counterparty).ok().as_deref() == Some(body.as_slice());
        if !placed {
            for (p, was) in brought.iter().zip(before) {
                if !was {
                    let _ = std::fs::remove_file(p);
                }
            }
            return Some(fail(
                "GB-191",
                format!("the directory step did not place the confirmed package {digest} at {}; what it placed instead was taken away", paths.counterparty.display()),
                "run phase 5 again; if their pointer now names another package, confirm that one",
            ));
        }
    }
    st.confirm(
        run,
        5,
        "pointer-package",
        how,
        &digest,
        BTreeMap::from([
            ("partner".to_string(), them.clone()),
            ("digest".to_string(), digest.clone()),
        ]),
    );
    Some(Step { rows: vec![ok("confirm", format!("the package {them}'s pointer names was confirmed for this peering ({how}) and placed"))], verdict: Verdict::Continue })
}

/// Where the directory step, or a person on the hand path, leaves a new
/// package from the partner that phase 5 has not placed yet (ADR 1229).
pub const INCOMING: &str = "theirs.incoming.json";

/// Beside `INCOMING` when the directory step took the package through the
/// partner's single pointer (no slot for this peering), so it may be the
/// package of another of their peerings.
pub const INCOMING_POINTER: &str = "theirs.incoming.json.pointer";

/// How many days past our UTC date an offered package's `issued` may be. One
/// further ahead would make every honest later package read as older (GB-308).
pub const ISSUED_AHEAD_DAYS: i64 = 7;

/// The code for a partner's package older than the one we hold (ADR 1229).
pub const OLDER_PACKAGE: &str = "GB-308";

/// A package's `issued` date (YYYY-MM-DD), as the parser checked it.
fn issued(package: &[u8]) -> Option<String> {
    goat_federation::package::parse_package(package)
        .ok()
        .map(|p| p.issued)
}

/// The key the keyring binds for `id`, as written by `write_keyring`.
fn keyring_key(path: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        squeeze(l)
            .strip_prefix(&format!("{id}="))
            .map(String::from)
            .filter(|k| !k.is_empty())
    })
}

/// The members that differ between two packages, as dotted paths
/// (`transport_offer.public_key`), the signature left out.
fn changed_members(a: &serde_json::Value, b: &serde_json::Value, at: &str, out: &mut Vec<String>) {
    match (a, b) {
        (serde_json::Value::Object(x), serde_json::Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                if at.is_empty() && k == "signature" {
                    continue;
                }
                let p = if at.is_empty() {
                    k.clone()
                } else {
                    format!("{at}.{k}")
                };
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => changed_members(u, v, &p, out),
                    _ => out.push(p),
                }
            }
        }
        _ if a != b => out.push(if at.is_empty() {
            "(the whole package)".into()
        } else {
            at.into()
        }),
        _ => {}
    }
}

/// `path` renamed to `<path>.<tag>-<UTC time>`, keeping any earlier copy.
fn set_aside(path: &Path, tag: &str) -> std::io::Result<std::path::PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut aside = path.with_file_name(format!("{name}.{tag}-{stamp}"));
    let mut n = 1;
    while aside.exists() {
        n += 1;
        aside = path.with_file_name(format!("{name}.{tag}-{stamp}-{n}"));
    }
    std::fs::rename(path, &aside)?;
    Ok(aside)
}

/// A copy of `path` at `<path>.<tag>-<UTC time>`, keeping any earlier copy;
/// `path` itself is left where it is.
fn copy_aside(path: &Path, tag: &str) -> std::io::Result<std::path::PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut aside = path.with_file_name(format!("{name}.{tag}-{stamp}"));
    let mut n = 1;
    while aside.exists() {
        n += 1;
        aside = path.with_file_name(format!("{name}.{tag}-{stamp}-{n}"));
    }
    // On disk before the new package is placed: after a crash the old one is
    // still somewhere, even if the rename landed and the copy had not. And its
    // name in the folder (POSIX: a file's fsync does not sync the directory
    // entry that names it). A copy that is not all on disk is removed, so no
    // stray copy is left beside a package that was never replaced.
    let kept = std::fs::copy(path, &aside)
        .and_then(|_| std::fs::File::open(&aside)?.sync_all())
        .and_then(|_| sync_dir(&aside));
    if let Err(e) = kept {
        let _ = std::fs::remove_file(&aside);
        return Err(e);
    }
    Ok(aside)
}

/// `body` written (0600) to a file beside `path` and synced to disk, ready to
/// be renamed over it: the bytes reach the disk before the rename makes them
/// the package.
fn write_staged(path: &Path, body: &[u8]) -> std::io::Result<std::path::PathBuf> {
    let tmp = path.with_extension("placing");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    std::io::Write::write_all(&mut f, body)?;
    f.sync_all()?;
    Ok(tmp)
}

/// Sync `path`'s folder, so a rename in it reaches the disk before anything is
/// journaled about it.
fn sync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// Write `body` to `path` (0600) through a file beside it, so a reader sees
/// the old package or the new one, never part of one. `hook` runs at each
/// stage (a test injects a failure there).
fn place(path: &Path, body: &[u8], hook: &mut dyn FnMut(Stage) -> std::io::Result<()>) -> Result<(), ReplaceError> {
    let before = hook(Stage::BeforePlace)
        .and_then(|_| write_staged(path, body))
        .and_then(|tmp| std::fs::rename(&tmp, path));
    if let Err(e) = before {
        let _ = std::fs::remove_file(path.with_extension("placing"));
        return Err(ReplaceError::BeforeRename(e));
    }
    hook(Stage::AfterRename)
        .and_then(|_| sync_dir(path))
        .map_err(ReplaceError::SyncAfterRename)
}

/// Replace the package at `path` with `body`, keeping the old one at
/// `<path>.<tag>-<UTC time>`. `hook` runs at each [`Stage`]: after the old one
/// is kept and before the new one is written, and after the rename (a test
/// injects a failure or a crash there).
///
/// The old package is COPIED aside and the new one is renamed over it, so at
/// every moment, a crash included, `path` holds the old package or the new
/// one, never neither. A failure before the rename removes the copy, and the
/// old package is where it was ([`ReplaceError::BeforeRename`]). A failure
/// after it keeps the copy, and the new package is in place
/// ([`ReplaceError::AfterRename`]).
fn replace_counterparty(
    path: &Path,
    body: &[u8],
    tag: &str,
    hook: &mut dyn FnMut(Stage) -> std::io::Result<()>,
) -> Result<std::path::PathBuf, ReplaceError> {
    let aside = copy_aside(path, tag).map_err(ReplaceError::CopyAside)?;
    let before = hook(Stage::BeforePlace)
        .and_then(|_| write_staged(path, body))
        .and_then(|tmp| std::fs::rename(&tmp, path));
    if let Err(e) = before {
        let _ = std::fs::remove_file(&aside);
        let _ = std::fs::remove_file(path.with_extension("placing"));
        return Err(ReplaceError::BeforeRename(e));
    }
    // The rename has landed: the new package is in place and the old one is
    // only in the aside copy, so the copy stays and the caller says so.
    if let Err(error) = hook(Stage::AfterRename).and_then(|_| sync_dir(path)) {
        return Err(ReplaceError::AfterRename { aside, error });
    }
    Ok(aside)
}

/// What a failed replacement of the partner's package says, by the state it
/// left (GB-191 when no package was held, else GB-184).
fn replace_failed(them: &str, paths: &Paths, held: &Path, e: ReplaceError) -> Step {
    match e {
        ReplaceError::CopyAside(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fail(
                "GB-191",
                format!("no package held at {}; nothing was placed", paths.counterparty.display()),
                format!("run phase 5 again with them, the keys read aloud: goatbuilder federate {them} --from 5 --partner-key <key> --partner-ca-fingerprint <fp>"),
            )
        }
        ReplaceError::CopyAside(e) => {
            fail(
                "GB-184",
                format!("cannot write the working folder: {} could not be copied aside ({e}); nothing was placed", paths.counterparty.display()),
                "check the working folder is writable, then run phase 5 again",
            )
        }
        // SyncAfterRename is place()'s alone; replace_counterparty never returns it.
        ReplaceError::BeforeRename(e) | ReplaceError::SyncAfterRename(e) => {
            fail(
                "GB-184",
                format!("cannot write the working folder: {them}'s new package could not be placed at {} ({e}); the old one is still in place, and no copy of it was kept", paths.counterparty.display()),
                "check the working folder is writable, then run phase 5 again",
            )
        }
        ReplaceError::AfterRename { aside, error } => {
            fail(
                "GB-184",
                format!("{them}'s new package is in place at {} but the folder could not be synced ({error}), so it was not recorded as confirmed; the old one is kept at {}", paths.counterparty.display(), aside.display()),
                format!("check the working folder's disk, then run phase 5 again: it asks again about {them}'s held incoming package ({}), the one now in place", held.display()),
            )
        }
    }
}

/// Where replace_counterparty is (a test injects a failure or a crash there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// The old package is copied aside; nothing is written yet.
    BeforePlace,
    /// The new package is renamed into place; the folder is not synced yet.
    AfterRename,
}

/// How replace_counterparty failed, by the state it left.
#[derive(Debug)]
pub(crate) enum ReplaceError {
    /// The old package could not be copied aside: nothing changed.
    CopyAside(std::io::Error),
    /// Before the rename: the old package is in place, the copy removed.
    BeforeRename(std::io::Error),
    /// After the rename: the NEW package is in place and the old one is kept
    /// at `aside`; the folder's sync failed, so the rename may not survive a
    /// crash.
    AfterRename { aside: std::path::PathBuf, error: std::io::Error },
    /// place() after the rename: the bytes given are in place, the folder's
    /// sync failed (no copy aside: place keeps none).
    SyncAfterRename(std::io::Error),
}

/// The package digest the last confirmation of this peering names, and its
/// `issued` date when it was readable: the keys confirmation of phase 5, a
/// partner-package confirmation (ADR 1229), or the migration record a build
/// writes for a peering confirmed before it recorded the package. The most
/// recent of them, in journal order. `None` before their key is bound.
pub fn confirmed_package(st: &State) -> Option<(String, Option<String>)> {
    let nonempty = |v: Option<&String>| v.filter(|d| !d.is_empty()).cloned();
    st.confirmations
        .iter()
        .rev()
        .filter(|c| c.phase == 5)
        .find_map(|c| match c.kind.as_str() {
            "keys" => nonempty(c.values.get("partner_package"))
                .map(|d| (d, nonempty(c.values.get("partner_package_issued")))),
            "partner-package" | MIGRATED => Some((c.bound_to.clone(), nonempty(c.values.get("issued"))))
                .filter(|(d, _)| !d.is_empty()),
            _ => None,
        })
}

/// The journal kind of the migration record.
pub const MIGRATED: &str = "partner-package-recorded";

/// The code for a held package that cannot be read when a peering confirmed
/// before this build is recorded (ADR 1229 migration).
pub const UNREADABLE_HELD: &str = "GB-310";

/// A peering whose phase 5 was confirmed by a build that did not record the
/// package: record the package held now as the confirmed one, journaled as a
/// migration, not as a person's confirmation. Stops (GB-310) when the held
/// package cannot be read. `None` when there is nothing to record.
pub fn migrate(paths: &Paths, st: &mut State, run: &str) -> Option<Step> {
    let keys = st.confirmations.iter().any(|c| c.kind == "keys" && c.phase == 5);
    if !keys || confirmed_package(st).is_some() {
        return None;
    }
    let them = st.peering.clone();
    let bytes = match std::fs::read(&paths.counterparty) {
        Ok(b) => b,
        Err(e) => {
            return Some(fail(
                UNREADABLE_HELD,
                format!(
                    "the package held for {them} cannot be read, so it cannot be recorded as confirmed ({}: {e}); nothing ran",
                    paths.counterparty.display()
                ),
                format!("put the package confirmed with {them} back at {}, or run phase 5 again with them: goatbuilder federate {them} --from 5", paths.counterparty.display()),
            ))
        }
    };
    let digest = sha256(&bytes);
    let when = issued(&bytes);
    let mut values = BTreeMap::from([
        ("partner".to_string(), them.clone()),
        ("digest".to_string(), digest.clone()),
    ]);
    if let Some(d) = &when {
        values.insert("issued".to_string(), d.clone());
    }
    st.record_migration(run, 5, MIGRATED, &digest, values);
    Some(Step {
        rows: vec![
            ok(
                "partner-package",
                format!(
                    "recorded the package held for {them} ({digest}{}) as the one confirmed: phase 5 was confirmed by a build that did not record it. A migration, not a confirmation; from now on a package older than it is refused and compose checks it",
                    when.map(|d| format!(", issued {d}")).unwrap_or_else(|| ", no readable issued date".into())
                ),
            ),
            // No person confirmed this package: its digest is read aloud at the next call.
            ok(
                "partner-package-digest",
                format!("the digest recorded without a person, {}: compare it with the partner at the next call", groups(&digest)),
            ),
        ],
        verdict: Verdict::Continue,
    })
}

/// Whether the last keys confirmation holds for the partner's package `bytes`:
/// the key the keyring binds and the package's CA fingerprint are the ones it
/// names. `None` when no keys confirmation is journaled (keys bound by hand).
fn partner_keys_hold(paths: &Paths, st: &State, bytes: &[u8]) -> Option<bool> {
    let keys = st.confirmations.iter().rev().find(|c| c.kind == "keys" && c.phase == 5)?;
    // With a keys confirmation journaled, a package that cannot be read for its
    // keys (not JSON, or a sovereign id that is not a plain id) does not hold:
    // fail closed.
    let Some(theirs) = serde_json::from_slice::<serde_json::Value>(bytes).ok() else {
        return Some(false);
    };
    let Some(them) = field(&theirs, &["sovereign_id"]).filter(|id| plain_id(id)) else {
        return Some(false);
    };
    let got = |k: &str| keys.values.get(k).filter(|v| !v.is_empty());
    // The key exactly (base64); the fingerprint as hex (same_value).
    let key_same = matches!((got("partner_key"), keyring_key(&paths.keyring, &them)), (Some(a), Some(b)) if a.trim() == b.trim());
    let fp_same = matches!((got("partner_ca_fingerprint"), field(&theirs, &["identity", "ca_fingerprint"])), (Some(a), Some(b)) if prompt::same_value(a, &b));
    Some(key_same && fp_same)
}

/// Before any phase after 5: the package on disk is the one the last
/// confirmation names (GB-191, naming phase 5), whatever phase the run
/// started from. `None` when it is, or before their key is bound.
pub fn compose_guard(paths: &Paths, st: &State, phase: u32) -> Option<Step> {
    let (want, _) = confirmed_package(st)?;
    let them = st.peering.clone();
    let fix = format!("run phase 5 again, which asks about a package that changed: goatbuilder federate {them} --from 5");
    match std::fs::read(&paths.counterparty) {
        Ok(b) if sha256(&b) == want => partner_keys_hold(paths, st, &b).filter(|held| !held).map(|_| {
            fail(
                "GB-191",
                format!(
                    "the partner's package at {} was confirmed at phase 5, but the keys confirmed there do not hold for it: its CA fingerprint, or the key the keyring binds, is not the one read aloud; phase {phase} did not run",
                    paths.counterparty.display()
                ),
                format!("confirm the keys again with them on the call: goatbuilder federate {them} --from 5 --partner-key <key> --partner-ca-fingerprint <fp>"),
            )
        }),
        Ok(b) => Some(fail(
            "GB-191",
            format!(
                "{} holds package {}, not the one confirmed at phase 5 ({want}); it changed after phase 5, so phase {phase} did not run",
                paths.counterparty.display(),
                sha256(&b)
            ),
            fix,
        )),
        Err(e) => Some(fail(
            "GB-191",
            format!(
                "{} cannot be read ({e}); the package confirmed at phase 5 is {want}, so phase {phase} did not run",
                paths.counterparty.display()
            ),
            fix,
        )),
    }
}

/// [`compose_guard`] for `goatbuilder federate compose` run as a verb: each
/// package given that lies in a peering's working folder (one with a run
/// journal), other than our own half there, must be the one confirmed at phase 5
/// in that journal, with the keys confirmed for it. A package outside any such
/// folder is composed as before. Refused with GB-191.
pub fn compose_verb_guard(packages: &[&Path]) -> anyhow::Result<()> {
    for p in packages {
        if p.file_name().and_then(|n| n.to_str()) == Some("ours.canonical.json") {
            continue;
        }
        let dir = match p.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        let Some(st) = State::read(dir)? else { continue };
        let mut paths = Paths::new(&dir.display().to_string());
        paths.counterparty = p.to_path_buf();
        if let Some(step) = compose_guard(&paths, &st, 6) {
            let detail = match &step.verdict {
                Verdict::Fail(d) | Verdict::Stop(d) => d.clone(),
                Verdict::Continue => String::new(),
            };
            let detail = detail.strip_prefix("GB-191: ").unwrap_or(&detail).to_string();
            return Err(goat_errors::coded("GB-191", format!("goatbuilder federate compose: {detail}")).into());
        }
    }
    Ok(())
}

/// Whether the agreement on disk is the one whose digest was compared with
/// the partner at phase 6 (journaled as the agreement confirmation).
pub fn agreement_unchanged(paths: &Paths, st: &State) -> bool {
    std::fs::read(&paths.agreement)
        .map(|b| st.confirmed("agreement", 6, &sha256(&b)).is_some())
        .unwrap_or(false)
}

/// After a new package from the partner is confirmed and placed (ADR 1229):
/// whether the keys confirmation journaled before still holds for it. True
/// when the last keys confirmation names the partner key the keyring binds,
/// the CA fingerprint the placed package carries, and our own verifying key
/// and CA fingerprint as they are now. Then confirmation 1 is not asked
/// again: the digest compared on the call was the new question.
pub fn keys_unchanged(paths: &Paths, st: &State) -> bool {
    let Some(keys) = st
        .confirmations
        .iter()
        .rev()
        .find(|c| c.kind == "keys" && c.phase == 5)
    else {
        return false;
    };
    let got = |k: &str| keys.values.get(k).filter(|v| !v.is_empty());
    let read = |p: &Path| -> Option<serde_json::Value> { serde_json::from_slice(&std::fs::read(p).ok()?).ok() };
    let (Some(theirs), Some(ours)) = (read(&paths.counterparty), read(&paths.ours)) else {
        return false;
    };
    let Some(them) = field(&theirs, &["sovereign_id"]).filter(|id| plain_id(id)) else {
        return false;
    };
    let seed = paths.signing.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    // Verifying keys are base64, where case is part of the value: compared exactly.
    // Fingerprints are hex: compared as same_value does (case and a sha256: prefix).
    let exact = [
        ("partner_key", keyring_key(&paths.keyring, &them)),
        ("our_key", our_verifying_key(&seed).ok()),
    ];
    let hex = [
        ("partner_ca_fingerprint", field(&theirs, &["identity", "ca_fingerprint"])),
        ("our_ca_fingerprint", field(&ours, &["identity", "ca_fingerprint"])),
    ];
    let keys_same = exact.iter().all(|(k, v)| matches!((got(k), v), (Some(a), Some(b)) if a.trim() == b.trim()));
    let fps_same = hex.iter().all(|(k, v)| matches!((got(k), v), (Some(a), Some(b)) if prompt::same_value(a, b)));
    // And their CA certificate on disk carries exactly the fingerprint confirmed,
    // as the normal path checks it: one certificate, that one.
    let ca_same = match (got("partner_ca_fingerprint"), std::fs::read(&paths.their_ca)) {
        (Some(fp), Ok(pem)) => {
            let fps = pem_fingerprints(&pem);
            fps.len() == 1 && prompt::same_value(&fps[0], fp)
        }
        _ => false,
    };
    keys_same && fps_same && ca_same
}

/// Phase 5, before the keys: a new package from the partner under the key
/// already bound (ADR 1229). It is new when the directory step, or a person
/// on the hand path, left a package at `<work>/theirs.incoming.json` that
/// differs from the one on disk, or when the package on disk is not the one
/// the last keys confirmation was given for and no confirmation names it (a
/// copy over the placed file by hand). A signature says who signed a
/// package, not which of two is current, so it is placed only once a person
/// confirms its digest, read on the call as the keys are: `yes` at a
/// terminal, or `--accept-partner-package <digest>`. The old package is kept
/// at `<work>/theirs.canonical.json.previous-<UTC time>`, and the
/// confirmation is journaled bound to the digest, so a later package is a
/// new question. A package issued before the one held is refused (a replay
/// is never asked about), even with its digest given. `None` when there is
/// none, or before their key is bound (the first exchange places theirs).
/// The partner this peering declares: FED_COUNTERPARTY_ID (goatbuilder.yaml
/// `peerings: with`, the value [`exchange`] checks their half against), else the
/// peering's own name.
fn declared_partner(st: &State) -> String {
    crate::federate::runenv::var("FED_COUNTERPARTY_ID")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| st.peering.clone())
}

pub fn incoming(paths: &Paths, s: &mut Surface, st: &mut State, run: &str) -> Option<Step> {
    let held = paths.work.join(INCOMING);
    let pointer_marker = paths.work.join(INCOMING_POINTER);
    // Every way the offered file goes (placed, refused, the same as held) takes
    // its marker with it.
    let drop_marker = || {
        let _ = std::fs::remove_file(paths.work.join(INCOMING_POINTER));
    };
    let placed = match std::fs::read(&paths.counterparty) {
        Ok(b) => b,
        // After their key is bound and a package confirmed, a package missing from
        // the place is a lost file, named as such (not a folder problem).
        Err(_) if confirmed_package(st).is_some() && held.exists() => {
            let them = st.peering.clone();
            return Some(fail(
                "GB-191",
                format!(
                    "no package held at {} (the one confirmed at phase 5 is {}); a new package is offered at {}, and nothing was placed",
                    paths.counterparty.display(),
                    confirmed_package(st).map(|(d, _)| d).unwrap_or_default(),
                    held.display()
                ),
                format!("run phase 5 again with them, the keys read aloud: goatbuilder federate {them} --from 5 --partner-key <key> --partner-ca-fingerprint <fp>"),
            ));
        }
        Err(_) => return None,
    };
    let placed_v: serde_json::Value = serde_json::from_slice(&placed).ok()?;
    let named = field(&placed_v, &["sovereign_id"]).filter(|id| plain_id(id))?;
    // The partner is the one the peering declares, never the one a file names: a
    // package that names another party is refused before its signature is checked.
    let them = declared_partner(st);
    if named != them {
        return Some(fail(
            "GB-191",
            format!(
                "{} names '{named}', and this peering's partner is '{them}'; nothing was placed",
                paths.counterparty.display()
            ),
            format!("put {them}'s package back at {}, or ask {them} to publish their current package again", paths.counterparty.display()),
        ));
    }
    let their_key = keyring_key(&paths.keyring, &them)?;
    let (body, by_hand) = match std::fs::read(&held) {
        Ok(b) if b == placed => {
            let _ = std::fs::remove_file(&held);
            drop_marker();
            return None;
        }
        Ok(b) => (b, false),
        Err(_) => {
            // The hand path's other door: a file copied over the placed one.
            let (bound_for, _) = confirmed_package(st)?;
            if bound_for == sha256(&placed) {
                return None;
            }
            (placed.clone(), true)
        }
    };
    let digest = sha256(&body);
    let new_v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    if !by_hand {
        if let Some(other) = field(&new_v, &["sovereign_id"]).filter(|id| id != &them) {
            drop_marker();
            let aside = match set_aside(&held, "refused") {
                Ok(p) => format!("it was set aside at {}", p.display()),
                Err(e) => format!("it could not be set aside ({e}); remove {}", held.display()),
            };
            return Some(fail(
                "GB-191",
                format!("the package offered as {them}'s new one names '{other}', and this peering's partner is '{them}'; {aside}; nothing was placed"),
                "do not use it; tell the partner, and ask them to publish their current package again",
            ));
        }
    }
    // The parser's refusal first: a package it refuses (an `issued` that is not
    // a date among them) is not checked for a signature, ordered or asked about.
    if let Err(f) = goat_federation::package::parse_package(&body) {
        let what = if by_hand {
            format!("{} is not a well-formed package ({f:?})", paths.counterparty.display())
        } else {
            drop_marker();
            match set_aside(&held, "refused") {
                Ok(p) => format!("the package offered as {them}'s new one is not a well-formed package ({f:?}); it was set aside at {}", p.display()),
                Err(e) => format!("the package offered as {them}'s new one is not a well-formed package ({f:?}), and it could not be set aside ({e}); remove {}", held.display()),
            }
        };
        return Some(fail(
            "GB-191",
            format!("{what}; nothing was placed"),
            "do not use it; tell the partner, and ask them to publish their current package again",
        ));
    }
    // An issued date far ahead of ours would make every honest later package
    // read as older (GB-308), locking the peering: refused as not well formed.
    if let Some(d) = issued(&body) {
        let today = chrono::Utc::now().date_naive();
        if let Ok(when) = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
            if (when - today).num_days() > ISSUED_AHEAD_DAYS {
                let what = if by_hand {
                    format!("{} is not a well-formed package: its issued date {d} is more than {ISSUED_AHEAD_DAYS} days after today's UTC date ({today})", paths.counterparty.display())
                } else {
                    drop_marker();
                    let aside = match set_aside(&held, "refused") {
                        Ok(p) => format!("it was set aside at {}", p.display()),
                        Err(e) => format!("it could not be set aside ({e}); remove {}", held.display()),
                    };
                    format!("the package offered as {them}'s new one is not a well-formed package: its issued date {d} is more than {ISSUED_AHEAD_DAYS} days after today's UTC date ({today}); {aside}")
                };
                return Some(fail(
                    "GB-191",
                    format!("{what}. A date that far ahead would make every later package read as older; nothing was placed"),
                    "ask the partner to publish their package again with today's date (and check the clock that signed it)",
                ));
            }
        }
    }
    let signed = matches!(signed_by(&body, &their_key), Ok(true));
    if !signed || field(&new_v, &["sovereign_id"]).as_deref() != Some(them.as_str()) {
        let what = if by_hand {
            format!("{} is not the package their last confirmation was for, and it is not {them}'s package signed by the key bound for them", paths.counterparty.display())
        } else {
            drop_marker();
            match set_aside(&held, "refused") {
                Ok(p) => format!("the package offered as {them}'s new one is not signed by the key bound for them; it was set aside at {}", p.display()),
                Err(e) => format!("the package offered as {them}'s new one is not signed by the key bound for them, and it could not be set aside ({e}); remove {}", held.display()),
            }
        };
        return Some(fail(
            "GB-191",
            format!("{what}; nothing was placed"),
            "do not use it; tell the partner, and ask them to publish their current package again",
        ));
    }
    // A package older than the one held is refused, never asked about:
    // asking again would turn a replay into a rollback approved by habit.
    // `issued` (YYYY-MM-DD, checked by the parser) is the only member that
    // orders a party's packages; same day, different digest is asked about.
    // The package held: the last one confirmed (or recorded by the migration),
    // else the one on disk.
    let record = confirmed_package(st);
    let held_issued = match &record {
        Some((_, d)) => d.clone(),
        None if by_hand => None,
        None => issued(&placed),
    };
    let new_issued = issued(&body);
    let held_digest = match &record {
        Some((d, _)) => d.clone(),
        None if by_hand => String::new(),
        None => sha256(&placed),
    };
    // Older, or not orderable against a known date: refused, never asked.
    let refusal = match (held_issued.as_deref(), new_issued.as_deref()) {
        (Some(was), Some(now)) if now < was => Some((format!("issued {now}"), format!("older than the one {}", if by_hand { "confirmed" } else { "we hold" }), was.to_string())),
        (Some(was), None) => Some(("with no readable issued date".to_string(), "so it cannot be ordered against the one we hold".to_string(), was.to_string())),
        _ => None,
    };
    if let Some((what_it_is, why, was)) = refusal {
        let what = if by_hand {
            format!("{} was replaced by hand with a package of {them}'s {what_it_is}, {why} ({was})", paths.counterparty.display())
        } else {
            drop_marker();
            let aside = match set_aside(&held, "refused") {
                Ok(p) => format!("it was set aside at {}", p.display()),
                Err(e) => format!("it could not be set aside ({e}); remove {}", held.display()),
            };
            format!("the package offered as {them}'s new one was {what_it_is}, {why} (issued {was}); {aside}")
        };
        return Some(fail(
            OLDER_PACKAGE,
            format!("{what}. A package that is older, or has no date to order it by, is never taken, even with its digest confirmed; nothing was placed"),
            "ask the partner which package is current; if they rolled back on purpose, they publish it again with a later issued date",
        ));
    }
    let mut changed = Vec::new();
    let old_v = if by_hand {
        serde_json::Value::Null
    } else {
        placed_v
    };
    if !by_hand {
        changed_members(&old_v, &new_v, "", &mut changed);
    }
    let changed_text = if by_hand {
        "not known: the package it replaced was copied over by hand".to_string()
    } else {
        changed.join(", ")
    };
    let interactive = s.values.ask && s.asker().interactive();
    // Two packages of one day are not ordered by anything in them: show
    // both digests, so the call settles which is current.
    let same_day = match (held_issued.as_deref(), new_issued.as_deref()) {
        (Some(was), Some(now)) if was == now && !held_digest.is_empty() => format!(
            "\n    both issued {now}: nothing in them says which is current.\n    the one held: {}",
            groups(&held_digest)
        ),
        (None, _) => format!(
            "\n    order unknown: the package held has no readable issued date{}",
            if held_digest.is_empty() { String::new() } else { format!("\n    the one held: {}", groups(&held_digest)) }
        ),
        _ => String::new(),
    };
    let via_pointer = !by_hand && pointer_marker.exists();
    let pointer_note = if via_pointer {
        format!("\n    it came through {them}'s single pointer, not a package they host for this peering:\n    it may be the package of another of {them}'s peerings. Confirm on the call that it is this one.")
    } else {
        String::new()
    };
    let text = format!(
        "\nPhase 5: {them} has a new package\n  It is signed by the key you bound for {them}, and it differs from the one\n  on disk. A signature does not say which of two packages is current, so\n  compare this digest with {them} on the call before it is used.\n    changed:  {changed_text}\n    digest:   {}\n    saved at: {}{same_day}{pointer_note}\n",
        groups(&digest),
        if by_hand { paths.counterparty.display() } else { held.display() }
    );
    let how = match s.values.partner_package.clone() {
        Some(d) if prompt::same_value(&d, &digest) => "flag",
        Some(d) => {
            return Some(fail(
                "GB-191",
                format!(
                "--accept-partner-package {d} is not the digest of {them}'s new package ({digest})"
            ),
                "read the digest again with the partner; a different package arrived",
            ))
        }
        None if interactive => {
            s.asker().say(&text);
            if !prompt::is_yes(&s.asker().ask(&format!("Did {them} read this digest on the call? Type yes to use the new package, or press Enter to stop:"))) {
                return Some(stop(
                    format!("{them}'s new package was not confirmed; nothing was placed"),
                    "run the same command again once the digest is compared on the call",
                ));
            }
            "terminal"
        }
        None => {
            let mut step = stop(
                format!("{them} has a new package signed by the key bound for them: compare its digest on the call, then pass --accept-partner-package {digest}"),
                "run the same command again with the flag",
            );
            step.rows.insert(0, ok("partner-package", text));
            return Some(step);
        }
    };
    let mut rows = Vec::new();
    if !by_hand {
        match replace_counterparty(&paths.counterparty, &body, "previous", &mut |_| Ok(())) {
            Ok(p) => rows.push(ok("partner-package", format!("the package it replaces is kept at {}", p.display()))),
            Err(e) => return Some(replace_failed(&them, paths, &held, e)),
        }
        let _ = std::fs::remove_file(&held);
        drop_marker();
    } else {
        // The bytes confirmed are the bytes placed, whatever was copied over
        // the file while the person was asked.
        match place(&paths.counterparty, &body, &mut |_| Ok(())) {
            Ok(()) => {}
            Err(ReplaceError::SyncAfterRename(e)) => {
                return Some(fail(
                    "GB-184",
                    format!("the package confirmed is in place at {} but the folder could not be synced ({e}), so it was not recorded as confirmed", paths.counterparty.display()),
                    "check the working folder's disk, then run phase 5 again: it asks about the package on disk",
                ));
            }
            Err(e) => {
                return Some(fail(
                    "GB-184",
                    format!("cannot write the working folder: the package confirmed could not be written to {} ({}); the file there is as it was", paths.counterparty.display(), match &e { ReplaceError::BeforeRename(e) | ReplaceError::CopyAside(e) | ReplaceError::SyncAfterRename(e) | ReplaceError::AfterRename { error: e, .. } => e }),
                    "check the working folder is writable, then run phase 5 again",
                ));
            }
        }
    }
    st.confirm(
        run,
        5,
        "partner-package",
        how,
        &digest,
        BTreeMap::from([
            ("partner".to_string(), them.clone()),
            ("digest".to_string(), digest.clone()),
            ("changed".to_string(), changed_text),
        ])
        .into_iter()
        .chain((!by_hand).then(|| ("previous".to_string(), sha256(&placed))))
        .chain(new_issued.clone().map(|d| ("issued".to_string(), d)))
        .collect(),
    );
    rows.push(ok(
        "confirm",
        format!("{them}'s new package {digest} was confirmed ({how}) and placed; the phases after 5 use it"),
    ));
    Some(Step {
        rows,
        verdict: Verdict::Continue,
    })
}

/// After compose (phase 6): the agreement digest, compared with the partner.
pub fn agreement(paths: &Paths, s: &mut Surface, st: &mut State, run: &str) -> Step {
    let bytes = match std::fs::read(&paths.agreement) {
        Ok(b) => b,
        Err(e) => {
            return fail(
                "GB-191",
                format!(
                    "compose passed but {} cannot be read: {e}",
                    paths.agreement.display()
                ),
                "run phase 6 again",
            )
        }
    };
    let digest = sha256(&bytes);
    // The same agreement, its digest already compared with the partner: not a
    // new question. A transport rotation recomposes it unchanged, since the
    // agreement carries no WireGuard key (ADR 1229; key-rotation design, K4).
    if s.values.agreement_digest.is_none() {
        if let Some(c) = st.confirmed("agreement", 6, &digest) {
            return Step {
                rows: vec![ok(
                    "confirm",
                    format!("agreement digest {digest} was confirmed with the partner before ({}, run {}): the agreement is unchanged, so it is not asked again", c.how, c.run),
                )],
                verdict: Verdict::Continue,
            };
        }
    }
    let interactive = s.values.ask && s.asker().interactive();
    let how = match s.values.agreement_digest.clone() {
        Some(v) if prompt::same_value(&v, &digest) => "flag",
        Some(v) => {
            return fail(
                "GB-191",
                format!("--agreement-digest {v} is not this agreement's digest {digest}"),
                "compare digests with the partner; `goat federate diff` names the field that differs",
            )
        }
        None if interactive => {
            s.asker().say(&format!("\nPhase 6: compare the agreement digest\n  Read it to the partner and listen to theirs:\n    {}\n", groups(&digest)));
            if !prompt::is_yes(&s.asker().ask("Is theirs the same? Type yes, or press Enter to stop:")) {
                return stop("the agreement digest was not confirmed", "compare digests with the partner, then run the same command again");
            }
            "terminal"
        }
        None => {
            return stop(
                format!("waiting for the agreement digest to be compared: read {digest} to the partner, then pass --agreement-digest (or --confirm at a terminal)"),
                "run the same command again with --agreement-digest",
            )
        }
    };
    st.confirm(
        run,
        6,
        "agreement",
        how,
        &digest,
        BTreeMap::from([("digest".to_string(), digest.clone())]),
    );
    Step {
        rows: vec![ok(
            "confirm",
            format!("agreement digest {digest} confirmed with the partner ({how})"),
        )],
        verdict: Verdict::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::federate::prompt::Scripted;

    struct World {
        _d: tempfile::TempDir,
        paths: Paths,
        their_key: String,
        their_fp: String,
    }

    fn vectors() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors")
    }

    fn seed_hex(party: &str) -> String {
        std::fs::read_to_string(vectors().join(format!("testkeys/{party}.seed.hex")))
            .unwrap()
            .trim()
            .to_string()
    }

    /// A conformance-vector package, with its CA fingerprint replaced, signed
    /// again with the committed test seed (worthless keys, public on purpose).
    pub fn resigned(party: &str, fp: &str) -> Vec<u8> {
        package(party, fp, None, party)
    }

    /// `party`'s conformance package with its CA fingerprint replaced and,
    /// when given, its `transport_offer.public_key` (a rotated WireGuard
    /// key), signed with `signer`'s committed test seed.
    pub fn package(party: &str, fp: &str, transport_key: Option<&str>, signer: &str) -> Vec<u8> {
        package_issued(party, fp, transport_key, signer, None)
    }

    /// As `package`, with its `issued` date replaced when given.
    pub fn package_issued(
        party: &str,
        fp: &str,
        transport_key: Option<&str>,
        signer: &str,
        issued: Option<&str>,
    ) -> Vec<u8> {
        use goat_federation::json::{canonical, parse_document, JVal};
        let bytes =
            std::fs::read(vectors().join(format!("compose-full/package-{party}.canonical.json")))
                .unwrap();
        let JVal::Obj(mut top) = parse_document(&bytes).unwrap().without_member("signature") else {
            panic!()
        };
        for (k, v) in top.iter_mut() {
            if k == "identity" {
                if let JVal::Obj(id) = v {
                    for (ik, iv) in id.iter_mut() {
                        if ik == "ca_fingerprint" {
                            *iv = JVal::Str(fp.into());
                        }
                    }
                }
            }
            if let (Some(date), "issued") = (issued, k.as_str()) {
                *v = JVal::Str(date.into());
            }
            if let (Some(key), "transport_offer", JVal::Obj(t)) =
                (transport_key, k.as_str(), &mut *v)
            {
                for (tk, tv) in t.iter_mut() {
                    if tk == "public_key" {
                        *tv = JVal::Str(key.into());
                    }
                }
            }
        }
        let unsigned = JVal::Obj(top.clone());
        let hex = seed_hex(signer);
        let seed: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
        let sig = ed25519_dalek::Signer::sign(&sk, canonical(&unsigned).as_bytes());
        top.push((
            "signature".into(),
            JVal::Str(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())),
        ));
        canonical(&JVal::Obj(top)).into_bytes()
    }

    fn public_b64(party: &str) -> String {
        std::fs::read_to_string(vectors().join(format!("testkeys/{party}.public.b64")))
            .unwrap()
            .trim()
            .to_string()
    }

    fn world() -> World {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        let mut paths = Paths::new(&w);
        let der = b"fake-ca-der";
        let their_fp = sha256(der);
        std::fs::write(
            &paths.their_ca,
            format!(
                "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
                base64::engine::general_purpose::STANDARD.encode(der)
            ),
        )
        .unwrap();
        std::fs::write(&paths.ours, resigned("xaa", &sha256(b"our-ca-der"))).unwrap();
        std::fs::write(&paths.counterparty, resigned("xab", &their_fp)).unwrap();
        std::fs::write(&paths.agreement, "{\"a\":1}").unwrap();
        let seed = d.path().join("seed");
        std::fs::write(&seed, seed_hex("xaa")).unwrap();
        paths.signing = Some(seed);
        World {
            _d: d,
            paths,
            their_key: public_b64("xab"),
            their_fp,
        }
    }

    fn surface(values: Values, answers: &[&str]) -> Surface {
        Surface {
            values,
            ask: Some(Box::new(Scripted::new(answers))),
            ..Default::default()
        }
    }

    fn code(step: &Step) -> String {
        match &step.verdict {
            Verdict::Fail(d) | Verdict::Stop(d) => d.split(':').next().unwrap_or("").to_string(),
            Verdict::Continue => "continue".into(),
        }
    }

    #[test]
    fn flags_bind_the_keys_and_write_the_keyring() {
        let w = world();
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = exchange(&w.paths, &mut s, &mut st, "r1", &|| true);
        assert_eq!(code(&step), "continue");
        let ring = std::fs::read_to_string(&w.paths.keyring).unwrap();
        assert!(
            ring.contains(&format!("xab={}", w.their_key)) && ring.contains("xaa="),
            "{ring}"
        );
        let c = st.confirmations.iter().find(|c| c.kind == "keys").unwrap();
        assert_eq!((c.how.as_str(), c.phase), ("flag", 5));
        assert_eq!(c.values["partner_key"], w.their_key);
    }

    #[test]
    fn a_value_that_differs_fails_and_a_missing_one_stops() {
        let w = world();
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some("sha256:00".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-191"
        );
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = exchange(&w.paths, &mut s, &mut st, "r1", &|| true);
        assert_eq!(
            code(&step),
            "GB-192",
            "no fingerprint, no terminal: the old stop"
        );
        assert!(step
            .rows
            .iter()
            .any(|r| r.detail.contains("--partner-ca-fingerprint")));
        assert!(!w.paths.keyring.exists(), "nothing written");
        let mut s = surface(
            Values {
                partner_key: Some("not-a-key".into()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-191"
        );
    }

    #[test]
    fn a_key_that_does_not_verify_their_half_fails_and_writes_nothing() {
        let w = world();
        let mut st = State::new("xab");
        // A real key, but another party's.
        let wrong = public_b64("xaa");
        let mut s = surface(
            Values {
                partner_key: Some(wrong),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = exchange(&w.paths, &mut s, &mut st, "r1", &|| true);
        assert_eq!(code(&step), "GB-191");
        assert!(step
            .rows
            .iter()
            .any(|r| r.detail.contains("not signed by the key read aloud")));
        assert!(!w.paths.keyring.exists());
    }

    #[test]
    fn a_seed_that_did_not_sign_our_half_fails() {
        let mut w = world();
        let other = w._d.path().join("other-seed");
        std::fs::write(&other, "22".repeat(32)).unwrap();
        w.paths.signing = Some(other);
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-193"
        );
    }

    #[test]
    fn a_sovereign_id_can_never_write_a_second_keyring_line() {
        assert!(plain_id("xab") && plain_id("x.a-b_1"));
        for bad in ["", "x\nxaa=k", "xab=k", "x b"] {
            assert!(!plain_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_unreadable_keyring_is_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("keys.txt");
        std::fs::create_dir_all(&p).unwrap();
        assert!(write_keyring(&p, &[("xaa", "k")]).is_err());
    }

    #[test]
    fn a_package_held_back_by_the_directory_is_placed_only_once_confirmed() {
        let w = world();
        let theirs = std::fs::read(&w.paths.counterparty).unwrap();
        std::fs::remove_file(&w.paths.counterparty).unwrap();
        std::fs::write(w.paths.work.join("theirs.pointer.json"), &theirs).unwrap();
        let digest = sha256(&theirs);
        let (cp, held) = (
            w.paths.counterparty.clone(),
            w.paths.work.join("theirs.pointer.json"),
        );
        let place = move || std::fs::copy(&held, &cp).is_ok();
        let mut st = State::new("xab");
        // No flag and no terminal: the stop names the digest; nothing placed.
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = pointer(&w.paths, &mut s, &mut st, "r1", None, &place).unwrap();
        assert_eq!(code(&step), "GB-192");
        assert!(step.rows.iter().any(|r| r.detail.contains(&digest)));
        assert!(!w.paths.counterparty.exists());
        // Another digest fails.
        let mut s = surface(
            Values {
                pointer_package: Some("sha256:00".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&pointer(&w.paths, &mut s, &mut st, "r1", None, &place).unwrap()),
            "GB-191"
        );
        // At a terminal, only yes; then it is placed, journaled, and no longer held.
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["y"],
        );
        assert_eq!(
            code(&pointer(&w.paths, &mut s, &mut st, "r1", None, &place).unwrap()),
            "GB-192"
        );
        let mut s = surface(
            Values {
                pointer_package: Some(digest.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&pointer(&w.paths, &mut s, &mut st, "r1", None, &place).unwrap()),
            "continue"
        );
        assert!(st.confirmed("pointer-package", 5, &digest).is_some());
        assert!(
            pointer(&w.paths, &mut s, &mut st, "r1", None, &place).is_none(),
            "placed: nothing held now"
        );
    }

    #[test]
    fn a_rerun_that_places_another_package_is_undone_and_still_held() {
        let w = world();
        let theirs = std::fs::read(&w.paths.counterparty).unwrap();
        std::fs::remove_file(&w.paths.counterparty).unwrap();
        std::fs::remove_file(&w.paths.their_ca).unwrap();
        std::fs::write(w.paths.work.join("theirs.pointer.json"), &theirs).unwrap();
        let digest = sha256(&theirs);
        let (cp, ca) = (w.paths.counterparty.clone(), w.paths.their_ca.clone());
        // The pointer now names another package: the directory places that.
        let other = move || {
            std::fs::write(&cp, b"{\"other\":1}").is_ok() && std::fs::write(&ca, b"x").is_ok()
        };
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                pointer_package: Some(digest.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = pointer(&w.paths, &mut s, &mut st, "r1", None, &other).unwrap();
        assert_eq!(code(&step), "GB-191");
        assert!(
            !w.paths.counterparty.exists() && !w.paths.their_ca.exists(),
            "what it placed is taken away"
        );
        assert!(st.confirmations.is_empty());
        // A package placed by an earlier run and never confirmed is still held.
        std::fs::write(&w.paths.counterparty, &theirs).unwrap();
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&pointer(&w.paths, &mut s, &mut st, "r1", None, &|| true).unwrap()),
            "GB-192"
        );
        // A pointer file older than this run's directory step is not offered.
        std::fs::remove_file(&w.paths.counterparty).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(pointer(&w.paths, &mut s, &mut st, "r1", Some(later), &|| true).is_none());
    }

    /// A transport key other than the vector's: the partner rotated it (K4).
    const ROTATED: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    /// The world after confirmation 1: their key in the keyring, and the
    /// keys journaled for the package on disk.
    fn bound() -> (World, State) {
        let w = world();
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "continue"
        );
        (w, st)
    }

    fn previous_copies(w: &World) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(&w.paths.work)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("theirs.canonical.json.previous-")
            })
            .collect()
    }

    #[test]
    fn the_keys_confirmation_records_the_package_it_was_given_for() {
        let (w, st) = bound();
        let theirs = std::fs::read(&w.paths.counterparty).unwrap();
        let c = st.confirmations.iter().find(|c| c.kind == "keys").unwrap();
        assert_eq!(c.values["partner_package"], sha256(&theirs));
    }

    #[test]
    fn a_new_package_under_the_bound_key_is_placed_only_once_confirmed() {
        let (w, mut st) = bound();
        let old = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        let incoming = w.paths.work.join(INCOMING);
        std::fs::write(&incoming, &new).unwrap();
        let digest = sha256(&new);
        // No flag and no terminal: the stop names the digest and what changed; nothing placed.
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-192");
        let text: String = step
            .rows
            .iter()
            .map(|r| r.detail.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(&digest) && text.contains("transport_offer.public_key"),
            "{text}"
        );
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), old);
        // Another digest fails; a "y" is not a yes.
        let mut s = surface(
            Values {
                partner_package: Some("sha256:00".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "GB-191");
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["y"],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "GB-192");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), old);
        assert!(st.confirmed("partner-package", 5, &digest).is_none());
        // At a terminal, the typed word: placed, the old one kept aside, journaled.
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["yes"],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
        assert!(!incoming.exists());
        let kept = previous_copies(&w);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), old);
        let c = st
            .confirmed("partner-package", 5, &digest)
            .expect("journaled");
        assert_eq!(c.how, "terminal");
        assert_eq!(c.values["previous"], sha256(&old));
        assert!(
            incoming_step_opt(&w, &mut s, &mut st).is_none(),
            "placed: nothing to ask"
        );
    }

    #[test]
    fn the_flag_places_a_new_package_whose_digest_it_names() {
        let (w, mut st) = bound();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(
            Values {
                partner_package: Some(sha256(&new)),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
        assert_eq!(
            st.confirmed("partner-package", 5, &sha256(&new))
                .unwrap()
                .how,
            "flag"
        );
    }

    #[test]
    fn a_package_not_signed_by_the_bound_key_is_refused_and_set_aside() {
        let (w, mut st) = bound();
        let old = std::fs::read(&w.paths.counterparty).unwrap();
        // xab's package, signed by another party's key.
        let forged = package("xab", &w.their_fp, Some(ROTATED), "xaa");
        let incoming = w.paths.work.join(INCOMING);
        std::fs::write(&incoming, &forged).unwrap();
        let mut s = surface(
            Values {
                partner_package: Some(sha256(&forged)),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "GB-191");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), old);
        assert!(
            !incoming.exists(),
            "set aside, so the next run does not ask again"
        );
        assert!(previous_copies(&w).is_empty());
    }

    #[test]
    fn the_package_already_on_disk_offered_again_is_not_a_question() {
        let (w, mut st) = bound();
        let incoming = w.paths.work.join(INCOMING);
        std::fs::copy(&w.paths.counterparty, &incoming).unwrap();
        let mut s = surface(Values::default(), &[]);
        assert!(incoming_step_opt(&w, &mut s, &mut st).is_none());
        assert!(!incoming.exists());
    }

    /// The code for a package older than the one held (the owner's rule on
    /// ADR 1229): refused, never asked about.
    const OLDER: &str = "GB-308";
    /// The vector packages are issued on 2026-09-05.
    const EARLIER: &str = "2026-09-01";
    const LATER: &str = "2026-09-20";

    fn flag(digest: &str) -> Values {
        Values {
            partner_package: Some(digest.into()),
            ..Default::default()
        }
    }

    /// A replay of an older package would become a rollback approved by
    /// habit, so it is refused even with its own digest given, and set aside.
    #[test]
    fn an_older_package_is_refused_even_with_its_digest() {
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let older = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(EARLIER));
        let incoming = w.paths.work.join(INCOMING);
        std::fs::write(&incoming, &older).unwrap();
        let mut s = surface(flag(&sha256(&older)), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), OLDER);
        assert!(
            matches!(step.verdict, Verdict::Fail(_)),
            "a refusal, not a stop"
        );
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), held);
        assert!(!incoming.exists(), "set aside, so it is not offered again");
        assert!(st
            .confirmed("partner-package", 5, &sha256(&older))
            .is_none());
        // A typed yes changes nothing either.
        std::fs::write(&incoming, &older).unwrap();
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["yes"],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), OLDER);
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), held);
    }

    #[test]
    fn a_newer_package_and_one_of_the_same_day_are_asked_about() {
        for issued in [LATER, "2026-09-05"] {
            let (w, mut st) = bound();
            let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(issued));
            std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
            let mut s = surface(
                Values {
                    partner_key: Some(w.their_key.clone()),
                    ..Default::default()
                },
                &[],
            );
            assert_eq!(
                code(&incoming_step(&w, &mut s, &mut st)),
                "GB-192",
                "{issued}"
            );
            let mut s = surface(flag(&sha256(&new)), &[]);
            assert_eq!(
                code(&incoming_step(&w, &mut s, &mut st)),
                "continue",
                "{issued}"
            );
            assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
        }
    }

    /// Two packages of one day are not ordered: the stop shows both digests.
    #[test]
    fn a_package_of_the_same_day_shows_both_digests() {
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some("2026-09-05"));
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(Values::default(), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-192");
        let text: String = step
            .rows
            .iter()
            .map(|r| r.detail.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(&groups(&sha256(&new))) && text.contains(&groups(&sha256(&held))),
            "{text}"
        );
        // A later package shows only its own.
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let step = incoming_step(&w, &mut s, &mut st);
        let text: String = step
            .rows
            .iter()
            .map(|r| r.detail.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains(&groups(&sha256(&held))), "{text}");
    }

    #[test]
    fn a_replayed_older_package_is_refused_after_a_newer_one_was_taken() {
        let (w, mut st) = bound();
        let old = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(flag(&sha256(&new)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        // The package it replaced, still signed by the same key, offered again.
        std::fs::write(w.paths.work.join(INCOMING), &old).unwrap();
        let mut s = surface(flag(&sha256(&old)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), OLDER);
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
    }

    #[test]
    fn an_older_package_copied_over_the_placed_one_by_hand_is_refused() {
        let (w, mut st) = bound();
        let older = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(EARLIER));
        std::fs::write(&w.paths.counterparty, &older).unwrap();
        let mut s = surface(flag(&sha256(&older)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), OLDER);
        assert!(st
            .confirmed("partner-package", 5, &sha256(&older))
            .is_none());
    }

    #[test]
    fn a_package_copied_over_the_placed_one_by_hand_is_asked_about() {
        let (w, mut st) = bound();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        std::fs::write(&w.paths.counterparty, &new).unwrap();
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-192");
        assert!(step.rows.iter().any(|r| r.detail.contains(&sha256(&new))));
        let mut s = surface(
            Values {
                partner_package: Some(sha256(&new)),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert!(st.confirmed("partner-package", 5, &sha256(&new)).is_some());
        assert!(incoming_step_opt(&w, &mut s, &mut st).is_none());
    }

    // ---- ADR 1229 fixes: the counterparty is never missing, the confirmed bytes
    // ---- are placed, older peerings are recorded, and compose checks the digest.

    /// Item 1: a failure, or a crash, between keeping the old package and placing
    /// the new one leaves the old package where it was, never no package.
    #[test]
    fn an_interrupted_replacement_leaves_the_old_package_in_place() {
        let (w, _st) = bound();
        let old = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        let r = replace_counterparty(&w.paths.counterparty, &new, "previous", &mut |at| {
            if at == Stage::BeforePlace { Err(std::io::Error::other("injected between the two steps")) } else { Ok(()) }
        });
        assert!(r.is_err());
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), old, "a failure leaves the old package in place");
        // A crash at the same point: the process dies there.
        let path = w.paths.counterparty.clone();
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = replace_counterparty(&path, &new, "previous", &mut |at| {
                if at == Stage::BeforePlace { panic!("crash between the two steps") } else { Ok(()) }
            });
        }));
        assert!(crashed.is_err());
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), old, "a crash leaves the old package in place");
        // And when nothing interrupts it: the new one is placed, the old one kept.
        replace_counterparty(&w.paths.counterparty, &new, "previous", &mut |_| Ok(())).unwrap();
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
        assert!(previous_copies(&w).iter().any(|p| std::fs::read(p).unwrap() == old));
    }

    /// Owner's read, defect 1: a failure AFTER the rename (the folder's sync)
    /// leaves the NEW package in place, so the old one must stay kept aside, and
    /// the error must say that state, not "nothing placed".
    #[test]
    fn a_failure_after_the_rename_keeps_the_old_package_aside_and_says_the_new_one_is_in_place() {
        let (w, _st) = bound();
        let old = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        let r = replace_counterparty(&w.paths.counterparty, &new, "previous", &mut |at| {
            if at == Stage::AfterRename { Err(std::io::Error::other("injected after the rename")) } else { Ok(()) }
        });
        match r {
            Err(ReplaceError::AfterRename { aside, .. }) => {
                assert_eq!(std::fs::read(&aside).unwrap(), old, "the old package is kept aside");
            }
            other => panic!("expected AfterRename, got {other:?}"),
        }
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new, "the new package is in place");
        assert!(previous_copies(&w).iter().any(|p| std::fs::read(p).unwrap() == old));
    }

    /// The by-hand placement (no copy aside): a failure after the rename is
    /// told apart from one before it, since the bytes given are then in place.
    /// What each failed replacement says is the state it left (the owner's
    /// read of bbedc23a: a message that tells the truth). Before the rename
    /// the old package is still in place and no copy is kept.
    #[test]
    fn each_failed_replacement_says_the_state_it_left() {
        let w = world();
        let held = w.paths.work.join(INCOMING);
        let say = |e| {
            let step = replace_failed("xab", &w.paths, &held, e);
            step.rows.iter().map(|r| format!("{} {}", r.detail, r.fix.clone().unwrap_or_default())).collect::<Vec<_>>().join("\n")
        };
        let before = say(ReplaceError::BeforeRename(std::io::Error::other("x")));
        assert!(before.contains("the old one is still in place"), "{before}");
        assert!(!before.contains("kept aside"), "{before}");
        let after = say(ReplaceError::AfterRename { aside: w.paths.work.join("theirs.canonical.json.previous-1"), error: std::io::Error::other("x") });
        assert!(after.contains("new package is in place") && after.contains("previous-1") && after.contains(INCOMING), "{after}");
        let copy = say(ReplaceError::CopyAside(std::io::Error::other("x")));
        assert!(copy.contains("nothing was placed"), "{copy}");
    }

    #[test]
    fn a_by_hand_placement_tells_a_failure_after_the_rename_from_one_before_it() {
        let w = world();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        let r = place(&w.paths.counterparty, &new, &mut |at| {
            if at == Stage::AfterRename { Err(std::io::Error::other("injected after the rename")) } else { Ok(()) }
        });
        assert!(matches!(r, Err(ReplaceError::SyncAfterRename(_))), "{r:?}");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new);
        let before = std::fs::read(&w.paths.counterparty).unwrap();
        let r = place(&w.paths.counterparty, b"other", &mut |at| {
            if at == Stage::BeforePlace { Err(std::io::Error::other("injected before")) } else { Ok(()) }
        });
        assert!(matches!(r, Err(ReplaceError::BeforeRename(_))), "{r:?}");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), before);
    }

    /// An asker that copies another file over the package while the person is
    /// being asked, then says yes.
    struct SwapsThenYes {
        path: std::path::PathBuf,
        with: Vec<u8>,
    }

    impl prompt::Ask for SwapsThenYes {
        fn interactive(&self) -> bool {
            true
        }
        fn say(&mut self, _text: &str) {}
        fn ask(&mut self, _question: &str) -> Option<String> {
            std::fs::write(&self.path, &self.with).unwrap();
            Some("yes".into())
        }
    }

    /// Item 2: on the hand path, the bytes confirmed are the bytes placed, even
    /// when another package is copied over the file while the person is asked.
    #[test]
    fn on_the_hand_path_the_confirmed_bytes_are_placed() {
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(&w.paths.counterparty, &new).unwrap();
        let mut s = Surface {
            values: Values {
                ask: true,
                ..Default::default()
            },
            ask: Some(Box::new(SwapsThenYes {
                path: w.paths.counterparty.clone(),
                with: held.clone(),
            })),
            ..Default::default()
        };
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), new, "the package confirmed is the one on disk");
        assert!(st.confirmed("partner-package", 5, &sha256(&new)).is_some());
    }

    /// A state as a build before ADR 1229 left it: the keys confirmed, the
    /// package they were given for not recorded.
    fn bound_before_1229() -> (World, State) {
        let (w, mut st) = bound();
        for c in st.confirmations.iter_mut() {
            c.values.remove("partner_package");
            c.values.remove("partner_package_issued");
        }
        (w, st)
    }

    /// Item 3: the package held for a peering confirmed before this build is
    /// recorded once, as a migration (not a person's confirmation), and from
    /// then on an older package copied over it is refused.
    #[test]
    fn a_peering_confirmed_before_1229_records_its_held_package_as_a_migration() {
        let (w, mut st) = bound_before_1229();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        assert!(confirmed_package(&st).is_none());
        let step = migrate(&w.paths, &mut st, "r2").expect("a migration is recorded");
        assert_eq!(code(&step), "continue");
        let (digest, issued) = confirmed_package(&st).expect("recorded");
        assert_eq!(digest, sha256(&held));
        assert_eq!(issued.as_deref(), Some("2026-09-05"));
        let c = st.confirmations.last().unwrap();
        assert_eq!(c.how, "migration", "journaled as a migration, not a person's confirmation");
        assert!(migrate(&w.paths, &mut st, "r3").is_none(), "recorded once");
        let older = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(EARLIER));
        std::fs::write(&w.paths.counterparty, &older).unwrap();
        let mut s = surface(flag(&sha256(&older)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), OLDER);
    }

    /// A migration records a package as confirmed without a person, so it prints
    /// the digest, in the groups read aloud, to compare with the partner at the
    /// next call.
    #[test]
    fn a_migration_prints_the_digest_to_compare_at_the_next_call() {
        let (w, mut st) = bound_before_1229();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let step = migrate(&w.paths, &mut st, "r2").expect("a migration is recorded");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains(&groups(&sha256(&held))), "{text}");
        assert!(text.contains("compare it with the partner at the next call"), "{text}");
    }

    #[test]
    fn a_held_package_that_cannot_be_read_stops_the_migration_with_gb_310() {
        let (w, mut st) = bound_before_1229();
        std::fs::remove_file(&w.paths.counterparty).unwrap();
        let step = migrate(&w.paths, &mut st, "r2").expect("a stop");
        assert_eq!(code(&step), UNREADABLE_HELD);
        assert!(matches!(step.verdict, Verdict::Fail(_)));
        assert!(confirmed_package(&st).is_none(), "nothing recorded");
    }

    #[test]
    fn a_migrated_package_with_no_readable_date_records_none_and_asks_order_unknown() {
        let (w, mut st) = bound_before_1229();
        // A held package whose issued date cannot be read.
        let mut held: serde_json::Value = serde_json::from_slice(&std::fs::read(&w.paths.counterparty).unwrap()).unwrap();
        held["issued"] = serde_json::Value::String("not a date".into());
        std::fs::write(&w.paths.counterparty, serde_json::to_vec(&held).unwrap()).unwrap();
        assert_eq!(code(&migrate(&w.paths, &mut st, "r2").unwrap()), "continue");
        let (_, issued) = confirmed_package(&st).unwrap();
        assert_eq!(issued, None);
        assert!(!st.confirmations.last().unwrap().values.values().any(|v| v.is_empty()), "never journals an empty value");
        // Item 4(b): a newer package, held date unknown: asked, saying the order is unknown.
        let new = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(Values::default(), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-192");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("order unknown: the package held has no readable issued date"), "{text}");
    }

    /// Item 4(a): an offered package whose issued date is missing or cannot be
    /// read, while the held one's is known, is refused, never asked about.
    #[test]
    fn an_offered_package_without_a_readable_date_is_refused_when_the_held_date_is_known() {
        for bad in ["2026-13-45", "yesterday"] {
            let (w, mut st) = bound();
            let held = std::fs::read(&w.paths.counterparty).unwrap();
            let offered = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(bad));
            std::fs::write(w.paths.work.join(INCOMING), &offered).unwrap();
            let mut s = surface(flag(&sha256(&offered)), &[]);
            let step = incoming_step(&w, &mut s, &mut st);
            // The parser refuses an issued that is not a date: that refusal
            // (GB-191, naming the package as not well formed) comes first.
            let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
            assert!(code(&step) == OLDER || (code(&step) == "GB-191" && text.contains("not a well-formed package")), "{bad}: {text}");
            assert!(matches!(step.verdict, Verdict::Fail(_)), "{bad}");
            assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), held, "{bad}");
        }
    }

    /// Item 5: before any phase after 5, the package on disk must be the one
    /// the last confirmation names.
    #[test]
    fn compose_refuses_a_package_that_is_not_the_one_confirmed_naming_phase_5() {
        let (w, st) = bound();
        assert!(compose_guard(&w.paths, &st, 6).is_none(), "the confirmed package passes");
        let other = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(&w.paths.counterparty, &other).unwrap();
        let step = compose_guard(&w.paths, &st, 6).expect("refused");
        assert_eq!(code(&step), "GB-191");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("phase 5") && text.contains(&sha256(&other)), "{text}");
        // Before their key is bound there is nothing to compare against.
        let w2 = world();
        assert!(compose_guard(&w2.paths, &State::new("xab"), 6).is_none());
    }

    /// Lab finding (window 6): after a new package is confirmed and placed,
    /// confirmation 1 is not asked again when the keys it confirmed are unchanged;
    /// it is when the new package carries another CA fingerprint, or the keyring
    /// binds another key.
    #[test]
    fn a_confirmed_new_package_with_unchanged_keys_does_not_ask_the_keys_again() {
        let (w, mut st) = bound();
        assert!(keys_unchanged(&w.paths, &st), "the keys confirmed for the package on disk");
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(flag(&sha256(&new)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert!(keys_unchanged(&w.paths, &st), "a transport rotation leaves the keys as confirmed");
        // Another CA fingerprint in the package: the keys are asked again.
        let other_fp = format!("sha256:{}", "ab".repeat(32));
        std::fs::write(&w.paths.counterparty, package("xab", &other_fp, Some(ROTATED), "xab")).unwrap();
        assert!(!keys_unchanged(&w.paths, &st));
        // Another key in the keyring: asked again.
        std::fs::write(&w.paths.counterparty, &new).unwrap();
        let ring = std::fs::read_to_string(&w.paths.keyring).unwrap();
        std::fs::write(&w.paths.keyring, ring.replace(&w.their_key, "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=")).unwrap();
        assert!(!keys_unchanged(&w.paths, &st));
        // No keys confirmation at all (keys bound by hand): asked.
        let (w2, mut st2) = bound();
        st2.confirmations.retain(|c| c.kind != "keys");
        assert!(!keys_unchanged(&w2.paths, &st2));
    }

    /// A new package with another CA fingerprint is confirmed and placed, and
    /// the run stops for the keys (exit 0). A resume past phase 5 must not
    /// compose it before that fingerprint is confirmed: the package matches the
    /// digest confirmed, but the keys confirmation does not hold for it.
    #[test]
    fn compose_refuses_a_placed_package_whose_keys_were_not_confirmed_again() {
        let (w, mut st) = bound();
        let other_fp = format!("sha256:{}", "ab".repeat(32));
        let new = package_issued("xab", &other_fp, Some(ROTATED), "xab", Some(LATER));
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(flag(&sha256(&new)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert!(!keys_unchanged(&w.paths, &st));
        let step = compose_guard(&w.paths, &st, 6).expect("refused until the keys are confirmed again");
        assert_eq!(code(&step), "GB-191");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("phase 5") && text.contains("CA fingerprint"), "{text}");
        // Keys bound by hand (no keys confirmation journaled): only the digest is checked.
        let (w2, mut st2) = bound();
        st2.confirmations.retain(|c| c.kind != "keys");
        st2.confirm("r", 5, "partner-package", "flag", &sha256(&std::fs::read(&w2.paths.counterparty).unwrap()), BTreeMap::new());
        assert!(compose_guard(&w2.paths, &st2, 6).is_none());
    }

    /// backout 1: an offered `issued` far in the future would lock the peering
    /// (every honest later package would read as older). One more than seven days
    /// past our UTC date is refused as not well formed; nothing is placed.
    #[test]
    fn an_offered_package_issued_far_in_the_future_is_refused() {
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let future = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some("2099-01-01"));
        std::fs::write(w.paths.work.join(INCOMING), &future).unwrap();
        let mut s = surface(flag(&sha256(&future)), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-191");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("2099-01-01") && text.contains("days"), "{text}");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), held);
        // Within the seven days: asked as usual.
        let (w, mut st) = bound();
        let soon = (chrono::Utc::now().date_naive() + chrono::Duration::days(3)).format("%Y-%m-%d").to_string();
        let near = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", Some(&soon));
        std::fs::write(w.paths.work.join(INCOMING), &near).unwrap();
        let mut s = surface(flag(&sha256(&near)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
    }

    /// A package of another party, `xac`, signed by `xac`'s own key, with its
    /// verifying key returned for the keyring.
    fn another_partys_package(w: &World, issued: Option<&str>) -> (Vec<u8>, String) {
        use goat_federation::json::{canonical, parse_document, JVal};
        let b = package_issued("xab", &w.their_fp, Some(ROTATED), "xab", issued);
        let JVal::Obj(mut top) = parse_document(&b).unwrap().without_member("signature") else {
            panic!()
        };
        for (k, v) in top.iter_mut() {
            if k == "sovereign_id" {
                *v = JVal::Str("xac".into());
            }
        }
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let sig = ed25519_dalek::Signer::sign(&sk, canonical(&JVal::Obj(top.clone())).as_bytes());
        top.push((
            "signature".into(),
            JVal::Str(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())),
        ));
        let key = base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().to_bytes());
        (canonical(&JVal::Obj(top)).into_bytes(), key)
    }

    fn with_xac_in_the_keyring(w: &World, key: &str) {
        let ring = std::fs::read_to_string(&w.paths.keyring).unwrap();
        std::fs::write(&w.paths.keyring, format!("{}xac={key}\n", if ring.ends_with('\n') { ring.clone() } else { format!("{ring}\n") })).unwrap();
    }

    /// Phase 5 checks the package names the peering's partner, on the hand path:
    /// a package of another party whose key is in the keyring, copied over the
    /// placed one, is refused, and nothing is confirmed.
    #[test]
    fn a_package_naming_another_party_is_refused_on_the_hand_path() {
        let (w, mut st) = bound();
        let (c, key) = another_partys_package(&w, None);
        with_xac_in_the_keyring(&w, &key);
        std::fs::write(&w.paths.counterparty, &c).unwrap();
        let before = serde_json::to_string(&st).unwrap();
        let mut s = surface(flag(&sha256(&c)), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-191");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("'xac'") && text.contains("'xab'"), "{text}");
        assert_eq!(serde_json::to_string(&st).unwrap(), before, "nothing journaled");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), c, "the placed file is left as it was");
    }

    /// The same on the directory path: a package of another party offered as
    /// the partner's new one is refused, and the held package stays in place.
    #[test]
    fn a_package_naming_another_party_is_refused_on_the_directory_path() {
        let (w, mut st) = bound();
        let held = std::fs::read(&w.paths.counterparty).unwrap();
        let (c, key) = another_partys_package(&w, Some(LATER));
        with_xac_in_the_keyring(&w, &key);
        std::fs::write(w.paths.work.join(INCOMING), &c).unwrap();
        let before = serde_json::to_string(&st).unwrap();
        let mut s = surface(flag(&sha256(&c)), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-191");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("'xac'") && text.contains("this peering's partner"), "{text}");
        assert_eq!(serde_json::to_string(&st).unwrap(), before, "nothing journaled");
        assert_eq!(std::fs::read(&w.paths.counterparty).unwrap(), held, "the held package is unchanged");
        assert!(!w.paths.work.join(INCOMING).exists(), "the offered package is set aside");
    }

    /// backout 2: with no package held (it was lost) and a new one offered, the
    /// step names the missing package and phase 5, not a writable working folder.
    #[test]
    fn a_missing_held_package_is_named_not_reported_as_a_folder_problem() {
        let (w, mut st) = bound();
        std::fs::remove_file(&w.paths.counterparty).unwrap();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        let mut s = surface(flag(&sha256(&new)), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(matches!(step.verdict, Verdict::Fail(_)), "{text}");
        assert!(text.contains("no package held at") && text.contains("phase 5"), "{text}");
        assert!(!text.contains("writable"), "{text}");
    }

    /// backout 3: a package that came through the partner's single pointer is
    /// asked about saying so: it may be the package of another of their peerings.
    #[test]
    fn a_package_through_their_pointer_is_asked_about_saying_so() {
        let (w, mut st) = bound();
        let new = package("xab", &w.their_fp, Some(ROTATED), "xab");
        std::fs::write(w.paths.work.join(INCOMING), &new).unwrap();
        std::fs::write(w.paths.work.join(INCOMING_POINTER), "").unwrap();
        let mut s = surface(Values::default(), &[]);
        let step = incoming_step(&w, &mut s, &mut st);
        assert_eq!(code(&step), "GB-192");
        let text: String = step.rows.iter().map(|r| r.detail.clone()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("pointer") && text.contains("another of"), "{text}");
        // Confirmed and placed: the marker goes with the saved copy.
        let mut s = surface(flag(&sha256(&new)), &[]);
        assert_eq!(code(&incoming_step(&w, &mut s, &mut st)), "continue");
        assert!(!w.paths.work.join(INCOMING_POINTER).exists());
    }

    fn pem_of(der: &[u8]) -> String {
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD.encode(der)
        )
    }

    /// backout 1: the skip path vouches for what the normal path checks: their CA
    /// certificate on disk carries exactly the fingerprint confirmed.
    #[test]
    fn keys_unchanged_requires_their_ca_file_to_carry_exactly_the_confirmed_fingerprint() {
        let (w, st) = bound();
        assert!(keys_unchanged(&w.paths, &st));
        let original = std::fs::read(&w.paths.their_ca).unwrap();
        std::fs::write(&w.paths.their_ca, pem_of(b"another-ca-der")).unwrap();
        assert!(!keys_unchanged(&w.paths, &st), "another certificate");
        let mut two = original.clone();
        two.extend_from_slice(pem_of(b"another-ca-der").as_bytes());
        std::fs::write(&w.paths.their_ca, two).unwrap();
        assert!(!keys_unchanged(&w.paths, &st), "the confirmed one and another");
        std::fs::remove_file(&w.paths.their_ca).unwrap();
        assert!(!keys_unchanged(&w.paths, &st), "none");
    }

    /// backout 2: verifying keys are base64, where case is part of the value:
    /// compared exactly, never case-folded.
    #[test]
    fn verifying_keys_are_compared_exactly() {
        let swap = |k: &str| -> String {
            k.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect()
        };
        for key in ["partner_key", "our_key"] {
            let (w, mut st) = bound();
            let c = st.confirmations.iter_mut().find(|c| c.kind == "keys").unwrap();
            let v = c.values[key].clone();
            assert_ne!(swap(&v), v);
            c.values.insert(key.to_string(), swap(&v));
            assert!(!keys_unchanged(&w.paths, &st), "{key} differs in case only");
            if key == "partner_key" {
                assert!(compose_guard(&w.paths, &st, 6).is_some(), "{key}: compose refuses it");
            }
        }
    }

    /// backout 3: with a keys confirmation journaled, a package compose cannot read
    /// for its keys (not JSON, or a sovereign id that is not a plain id) fails closed.
    #[test]
    fn compose_fails_closed_on_a_package_it_cannot_read_for_its_keys() {
        for garbage in [&b"not json"[..], &br#"{"sovereign_id":"xab
xac=x"}"#[..]] {
            let (w, mut st) = bound();
            std::fs::write(&w.paths.counterparty, garbage).unwrap();
            st.confirm("r", 5, "partner-package", "flag", &sha256(garbage), BTreeMap::new());
            let step = compose_guard(&w.paths, &st, 6).expect("fails closed");
            assert_eq!(code(&step), "GB-191");
        }
    }

    #[test]
    fn before_their_key_is_bound_nothing_is_a_new_package() {
        let w = world();
        let mut st = State::new("xab");
        std::fs::write(
            w.paths.work.join(INCOMING),
            package("xab", &w.their_fp, Some(ROTATED), "xab"),
        )
        .unwrap();
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["yes"],
        );
        assert!(incoming_step_opt(&w, &mut s, &mut st).is_none());
    }

    fn incoming_step_opt(w: &World, s: &mut Surface, st: &mut State) -> Option<Step> {
        incoming(&w.paths, s, st, "r2")
    }

    fn incoming_step(w: &World, s: &mut Surface, st: &mut State) -> Step {
        incoming_step_opt(w, s, st).expect("a new package is offered")
    }

    #[test]
    fn an_unconfirmed_ca_is_placed_by_the_directory_once_the_key_is_bound() {
        let w = world();
        let unconfirmed = w.paths.their_ca.with_file_name("theirs-ca.pem.unconfirmed");
        std::fs::rename(&w.paths.their_ca, &unconfirmed).unwrap();
        let (from, to) = (unconfirmed.clone(), w.paths.their_ca.clone());
        let place = move || std::fs::rename(&from, &to).is_ok();
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        let step = exchange(&w.paths, &mut s, &mut st, "r1", &place);
        assert_eq!(code(&step), "continue");
        assert!(w.paths.their_ca.exists() && !unconfirmed.exists());
        // A directory step that does not place it: refused.
        std::fs::rename(&w.paths.their_ca, &unconfirmed).unwrap();
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-191"
        );
    }

    #[test]
    fn a_ca_certificate_that_is_not_theirs_fails() {
        let w = world();
        std::fs::write(
            &w.paths.their_ca,
            "-----BEGIN CERTIFICATE-----\nb3RoZXI=\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                partner_key: Some(w.their_key.clone()),
                partner_ca_fingerprint: Some(w.their_fp.clone()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-191"
        );
    }

    #[test]
    fn at_a_terminal_only_typed_answers_confirm() {
        let w = world();
        let mut st = State::new("xab");
        let key = w.their_key.clone();
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["yes", &key],
        );
        let step = exchange(&w.paths, &mut s, &mut st, "r1", &|| true);
        assert_eq!(code(&step), "continue");
        assert_eq!(st.confirmations[0].how, "terminal");
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["y"],
        );
        assert_eq!(
            code(&exchange(&w.paths, &mut s, &mut st, "r1", &|| true)),
            "GB-192",
            "only the word yes"
        );
    }

    #[test]
    fn the_agreement_digest_is_compared_by_flag_or_at_a_terminal() {
        let w = world();
        let digest = sha256(b"{\"a\":1}");
        let mut st = State::new("xab");
        let mut s = surface(
            Values {
                agreement_digest: Some(groups(&digest).split(": ").nth(1).unwrap().into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(
            code(&agreement(&w.paths, &mut s, &mut st, "r1")),
            "continue"
        );
        assert!(st.confirmed("agreement", 6, &digest).is_some());
        let mut s = surface(
            Values {
                agreement_digest: Some("sha256:00".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&agreement(&w.paths, &mut s, &mut st, "r1")), "GB-191");
        // The same agreement, its digest confirmed before: not asked again (ADR 1229:
        // a transport rotation leaves the agreement as it was).
        let mut s = surface(
            Values {
                partner_key: Some("k".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&agreement(&w.paths, &mut s, &mut st, "r1")), "continue");
        // Another agreement: asked.
        std::fs::write(&w.paths.agreement, b"{\"a\":2}").unwrap();
        assert_eq!(code(&agreement(&w.paths, &mut s, &mut st, "r1")), "GB-192");
        std::fs::write(&w.paths.agreement, b"{\"a\":1}").unwrap();
        let mut s = surface(
            Values {
                ask: true,
                ..Default::default()
            },
            &["yes"],
        );
        assert_eq!(
            code(&agreement(&w.paths, &mut s, &mut st, "r1")),
            "continue"
        );
    }
}

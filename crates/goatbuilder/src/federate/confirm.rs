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
}

impl Values {
    /// Whether confirmation 1 was asked for at all.
    pub fn active(&self) -> bool {
        self.ask
            || self.partner_key.is_some()
            || self.partner_ca_fingerprint.is_some()
            || self.agreement_digest.is_some()
            || self.pointer_package.is_some()
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
    if let Some(want) = std::env::var("FED_COUNTERPARTY_ID")
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
        ]),
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
        }
        let unsigned = JVal::Obj(top.clone());
        let hex = seed_hex(party);
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
        let mut s = surface(
            Values {
                partner_key: Some("k".into()),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(code(&agreement(&w.paths, &mut s, &mut st, "r1")), "GB-192");
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

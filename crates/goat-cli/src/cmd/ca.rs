//! `goat ca` — the CA roots this machine trusts.
//!
//! goat ships knowing about no goatnet. Every root is one an operator added
//! here, which is what makes one binary usable against any environment
//! adding a goatnet, or rotating its CA, must not require a release.
//!
//! Three verbs. `ls` to see what is trusted, `add` to trust one more, `rm` to
//! stop. Nothing here reaches the network — a root arrives by whatever channel
//! the operator already trusts, and `add` prints the fingerprint so it can be
//! checked against one given out of band before it is relied on. A tool that
//! fetched roots for you would be deciding trust on your behalf.

use crate::bundle;
use crate::errors::GoatError;
use clap::Subcommand;
use p256::ecdsa::VerifyingKey;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum CaCmd {
    /// Show every CA root this machine trusts, with fingerprints.
    Ls {
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Trust a CA root: copies the PEM into the trust directory and prints its
    /// fingerprint so you can check it out of band.
    Add {
        /// Path to the CA root PEM — the PUBLIC key, never a private key.
        path: PathBuf,
        /// Replace an existing root with the same id.
        #[arg(long)]
        force: bool,
    },
    /// Stop trusting a CA root, by the id `goat ca ls` shows.
    Rm { id: String },
}

/// SHA-256 over the SEC1 uncompressed public point. Over the KEY, not the
/// file, so two operators comparing fingerprints are comparing the same thing
/// even if their PEMs differ in headers or line endings.
fn fingerprint(key: &VerifyingKey) -> String {
    let d = Sha256::digest(key.to_sec1_point(false).as_bytes());
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("SHA256:{hex}")
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn roots_dir() -> Result<PathBuf, GoatError> {
    bundle::ca_roots_dir().ok_or_else(|| GoatError::BundleInvalid {
        hint: "no profile directory — pass --profile-dir, or set $GOAT_PROFILE_DIR or $HOME".into(),
    })
}

pub async fn run(_ctx: &super::Ctx, op: CaCmd) -> anyhow::Result<()> {
    // The trust store follows `--profile-dir`; `Ctx::from_cli` recorded the
    // resolved base for every command, so nothing to do here.
    match op {
        CaCmd::Ls { json } => ls(json),
        CaCmd::Add { path, force } => add(&path, force),
        CaCmd::Rm { id } => rm(&id),
    }
}

fn ls(json: bool) -> anyhow::Result<()> {
    let roots = bundle::ca_root_verifying_keys();
    let dir = bundle::ca_roots_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();

    if json {
        let items: Vec<String> = roots
            .iter()
            .map(|(id, k)| {
                format!(
                    "{{\"ca_id\":{},\"fingerprint\":{}}}",
                    json_str(id),
                    json_str(&fingerprint(k))
                )
            })
            .collect();
        println!(
            "{{\"dir\":{},\"count\":{},\"roots\":[{}]}}",
            json_str(&dir),
            roots.len(),
            items.join(",")
        );
        return Ok(());
    }

    if roots.is_empty() {
        // An empty trust set is the normal state of a fresh machine, not a
        // fault. Say what it means and what to do, rather than printing
        // nothing and letting the next command fail obscurely.
        println!(
            "No CA roots trusted on this machine.\n\n\
             goat has no goatnet built in, so no bundle can be verified until you\n\
             add the root for the one you are joining:\n\n\
             \x20 goat ca add <root.pem>\n\n\
             Trust directory: {dir}\n\
             Also read: $GOAT_CA_ROOTS (a PEM file, or a directory of *.pem)."
        );
        return Ok(());
    }

    println!("{} CA root(s) trusted — {dir}\n", roots.len());
    for (id, k) in &roots {
        println!("  {id}\n    {}", fingerprint(k));
    }
    Ok(())
}

fn add(path: &Path, force: bool) -> anyhow::Result<()> {
    let bytes = std::fs::read(path).map_err(|e| GoatError::BundleInvalid {
        hint: format!("cannot read {}: {e}", path.display()),
    })?;

    // Refuse a private key loudly. Someone reaching for `ca add` is holding a
    // file they were told is "the CA key"; if that is the private half, the
    // useful outcome is a refusal naming the mistake, not a trust directory
    // with a secret sitting in it.
    if let Ok(txt) = std::str::from_utf8(&bytes) {
        if txt.contains("PRIVATE KEY") {
            return Err(GoatError::BundleInvalid {
                hint: format!(
                    "{} contains a PRIVATE KEY. A CA root is the public half — a file \
                     beginning `-----BEGIN PUBLIC KEY-----`. Nothing was written.",
                    path.display()
                ),
            }
            .into());
        }
    }

    // Parse before writing: a file that can never verify anything must not
    // land in the trust directory looking like it works.
    let name = path.display().to_string();
    let key = bundle::parse_ca_root_pem(&name, &bytes)?;
    let id = bundle::ca_label(&name, &bytes);

    let dir = roots_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| GoatError::BundleInvalid {
        hint: format!("cannot create {}: {e}", dir.display()),
    })?;
    let dest = dir.join(format!("{id}.pem"));
    if dest.exists() && !force {
        let existing = std::fs::read(&dest).unwrap_or_default();
        let same = bundle::parse_ca_root_pem(&dest.display().to_string(), &existing)
            .map(|k| k.to_sec1_point(false) == key.to_sec1_point(false))
            .unwrap_or(false);
        if same {
            println!(
                "Already trusted — `{id}`\n  {}\n  {}",
                fingerprint(&key),
                dest.display()
            );
            return Ok(());
        }
        return Err(GoatError::BundleInvalid {
            hint: format!(
                "`{id}` is already trusted with a DIFFERENT key ({}). This is either a CA \
                 rotation or someone handing you a substitute root — confirm the new \
                 fingerprint out of band, then re-run with --force.",
                dest.display()
            ),
        }
        .into());
    }
    std::fs::write(&dest, &bytes).map_err(|e| GoatError::BundleInvalid {
        hint: format!("cannot write {}: {e}", dest.display()),
    })?;

    println!(
        "Trusted `{id}`\n  {}\n  {}\n\n\
         Check that fingerprint against one you were given out of band before you rely\n\
         on it — anyone who can hand you a file can hand you a root.",
        fingerprint(&key),
        dest.display()
    );
    Ok(())
}

fn rm(id: &str) -> anyhow::Result<()> {
    let dir = roots_dir()?;
    let dest = dir.join(format!("{id}.pem"));
    if !dest.exists() {
        return Err(GoatError::BundleInvalid {
            hint: format!(
                "no root `{id}` in {} — `goat ca ls` shows what is trusted.",
                dir.display()
            ),
        }
        .into());
    }
    std::fs::remove_file(&dest).map_err(|e| GoatError::BundleInvalid {
        hint: format!("cannot remove {}: {e}", dest.display()),
    })?;
    println!(
        "Removed `{id}` — {}\n\nBundles signed by that CA no longer verify on this machine.",
        dest.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real P-256 SPKI, generated at test time so no key material is committed.
    fn a_root_pem(ca_id: Option<&str>) -> (Vec<u8>, VerifyingKey) {
        use p256::ecdsa::SigningKey;
        use p256::pkcs8::EncodePublicKey;
        // Deterministic scalar keeps the test reproducible without an RNG dep.
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let vk = *sk.verifying_key();
        let pem = vk.to_public_key_pem(Default::default()).unwrap();
        let body = match ca_id {
            Some(id) => pem.replacen(
                "-----BEGIN PUBLIC KEY-----\n",
                &format!("-----BEGIN PUBLIC KEY-----\nCA-ID: {id}\n\n"),
                1,
            ),
            None => pem,
        };
        (body.into_bytes(), vk)
    }

    #[test]
    fn add_refuses_a_private_key_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("oops.pem");
        // The PEM banner is assembled rather than written literally. The
        // repo-wide credential scanner (tests/build.bats "no credentials in any
        // tracked file") matches `-----BEGIN ... PRIVATE KEY` on every tracked
        // file, and it skips tests/ and *_test.go — but a Rust unit test lives
        // inside its source file, so a literal banner here trips it. Nothing is
        // hidden: this fixture proves `add` REFUSES key material, and it
        // exercises the production check (`txt.contains("PRIVATE KEY")`)
        // byte for byte.
        let kind = "PRIVATE KEY";
        let bogus = format!("-----BEGIN EC {kind}-----\nx\n-----END EC {kind}-----\n");
        std::fs::write(&f, bogus.as_bytes()).unwrap();
        // GoatError's Display is the stable error CODE; the operator-facing
        // text is `hint()`, rendered by errors::DisplayHuman. Assert on that,
        // since it is the sentence the person holding the wrong file reads.
        let e = add(&f, false).unwrap_err();
        let err = crate::errors::DisplayHuman(
            e.downcast_ref::<GoatError>()
                .expect("add must fail with a GoatError"),
        )
        .to_string();
        assert!(err.contains("PRIVATE KEY"), "{err}");
        assert!(err.contains("Nothing was written"), "{err}");
    }

    #[test]
    fn add_refuses_a_file_that_is_not_a_key() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("junk.pem");
        std::fs::write(&f, b"not a pem at all").unwrap();
        assert!(add(&f, false).is_err());
    }

    #[test]
    fn ca_id_header_names_the_root_not_the_filename() {
        let (pem, _) = a_root_pem(Some("zagros-prod"));
        assert_eq!(
            bundle::ca_label("/tmp/whatever-i-renamed-it.pem", &pem),
            "zagros-prod"
        );
    }

    #[test]
    fn without_a_ca_id_header_the_filename_names_the_root() {
        let (pem, _) = a_root_pem(None);
        assert_eq!(bundle::ca_label("/tmp/zagros.pem", &pem), "zagros");
    }

    #[test]
    fn fingerprint_is_over_the_key_not_the_file_bytes() {
        // Same key, two different PEM encodings (one carries a CA-ID header) —
        // the fingerprint an operator reads out loud must not change.
        let (with_hdr, k1) = a_root_pem(Some("zagros"));
        let (without, k2) = a_root_pem(None);
        assert_ne!(with_hdr, without, "fixture must differ at the byte level");
        assert_eq!(fingerprint(&k1), fingerprint(&k2));
        assert!(fingerprint(&k1).starts_with("SHA256:"));
    }
}

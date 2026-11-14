// verify: could we deliver this agreement? Reads local state; writes nothing.

use super::{
    emit_json, print_outcomes, read_file, sign_with, FedOutcome, Site, Verdict,
    EXIT_FED_NOT_CONFORMANT, EXIT_OK,
};
use crate::compose::digest_of;
use crate::render;
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// The composed agreement, as the bytes that were hashed.
    pub agreement: PathBuf,
    /// Which party we are. Read from the declaration file when omitted.
    #[arg(long = "as", value_name = "SOVEREIGN_ID")]
    pub as_party: Option<String>,
    /// Sign the attestation with this Ed25519 key — the same key that signed
    /// our package (§10). Without it the line is printed unsigned and says so.
    #[arg(long, value_name = "FILE")]
    pub signing_key: Option<PathBuf>,
    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct VerifyReport {
    verb: &'static str,
    verdict: Verdict,
    digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attestation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attestation_signature: Option<String>,
    outcomes: Vec<FedOutcome>,
}

pub fn run_verify(
    site: &dyn Fn() -> anyhow::Result<Site>,
    a: &VerifyArgs,
    out: &mut dyn Write,
) -> anyhow::Result<i32> {
    let bytes = read_file(&a.agreement)?;
    let digest = digest_of(&bytes);
    let mut outcomes = Vec::new();

    let ag = match render::parse_agreement(&bytes) {
        Ok(ag) => ag,
        Err(f) => {
            outcomes.push(FedOutcome::fail(
                "readable",
                format!("{} ({})", f.rule, f.spec),
                f.detail.clone(),
            ));
            finish_verify(out, a, Verdict::Fail, &digest, None, None, outcomes)?;
            return Ok(EXIT_FED_NOT_CONFORMANT);
        }
    };
    outcomes.push(FedOutcome::ok(
        "readable",
        format!("{digest} — the digest over these exact bytes"),
    ));

    let ours = match resolve_party(site, a.as_party.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            outcomes.push(FedOutcome::fail("party", e, "Pass --as <sovereign_id>."));
            finish_verify(out, a, Verdict::Fail, &digest, None, None, outcomes)?;
            return Ok(EXIT_FED_NOT_CONFORMANT);
        }
    };

    let today = chrono::Utc::now().date_naive();
    let mut failed = false;
    for c in render::verify_locally(&ag, &ours, today) {
        if c.ok {
            outcomes.push(FedOutcome::ok(c.name, c.detail));
        } else {
            failed = true;
            outcomes.push(FedOutcome::fail(c.name, c.detail, c.hint));
        }
    }

    if failed {
        outcomes.push(FedOutcome::fail(
            "conformance",
            "not attested",
            "Agreeing is not the same as being able to deliver. The attestation is withheld \
             rather than qualified: a conformance line with a caveat is one the counterparty \
             cannot act on.",
        ));
        finish_verify(out, a, Verdict::Fail, &digest, None, None, outcomes)?;
        return Ok(EXIT_FED_NOT_CONFORMANT);
    }

    let line = render::attestation_line(&digest);
    let sig = match &a.signing_key {
        Some(p) => Some(sign_with(p, line.as_bytes())?),
        None => None,
    };
    match &sig {
        Some(_) => outcomes.push(FedOutcome::ok(
            "conformance",
            format!("publish this, signed: {line}"),
        )),
        None => outcomes.push(FedOutcome::skip(
            "conformance",
            format!(
                "would publish `{line}`, but no --signing-key was given, so it is unsigned and \
                 attests nothing. §10 wants it signed with the same key that signed our package"
            ),
        )),
    }
    finish_verify(
        out,
        a,
        Verdict::Ok,
        &digest,
        Some(&line),
        sig.as_deref(),
        outcomes,
    )?;
    Ok(EXIT_OK)
}

fn finish_verify(
    out: &mut dyn Write,
    a: &VerifyArgs,
    verdict: Verdict,
    digest: &str,
    line: Option<&str>,
    sig: Option<&str>,
    outcomes: Vec<FedOutcome>,
) -> std::io::Result<()> {
    if a.json {
        emit_json(
            out,
            &VerifyReport {
                verb: "verify",
                verdict,
                digest: digest.to_string(),
                attestation: line.map(str::to_string),
                attestation_signature: sig.map(str::to_string),
                outcomes,
            },
        )?;
    } else {
        writeln!(out, "verify {}", a.agreement.display())?;
        print_outcomes(out, &outcomes)?;
        if let (Some(l), Some(s)) = (line, sig) {
            writeln!(out, "\n{l}\nsignature: {s}")?;
        }
    }
    Ok(())
}

/// Which party we are: `--as`, else the declaration's `sovereign_id`. The site
/// is resolved only when `--as` is absent.
pub(crate) fn resolve_party(
    site: &dyn Fn() -> anyhow::Result<Site>,
    explicit: Option<&str>,
) -> Result<String, String> {
    if let Some(s) = explicit {
        return Ok(s.to_string());
    }
    let path = site()
        .map_err(|e| format!("no --as, and no declaration to read it from: {e:#}"))?
        .declaration_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("no --as and no {} to read it from", path.display()))?;
    let d: super::init::Declaration =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(d.sovereign_id)
}

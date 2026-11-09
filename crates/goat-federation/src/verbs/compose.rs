// compose: two packages, one agreement, one digest. Pure.

use super::{
    emit_json, label, print_outcomes, read_file, FedOutcome, Verdict, EXIT_FED_MALFORMED,
    EXIT_FED_REFUSED, EXIT_OK,
};
use crate::compose::{compose, Composed, KeyRing};
use crate::json::JVal;
use crate::package::{decode_verifying_key, parse_package, Package};
use chrono::NaiveDate;
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct ComposeArgs {
    /// One party's package.
    pub first: PathBuf,
    /// The other party's package. Order does not matter: the agreement has no
    /// "us" and "them", so both sides compute the same bytes whichever way
    /// round they pass the files.
    pub second: PathBuf,

    /// A verifying key bound to a party, as `<sovereign_id>=<44 base64
    /// characters>`. Repeat for each party.
    ///
    /// This is the 32-byte RAW Ed25519 public key — not PEM, not DER, not
    /// OpenSSH. It must have reached you over a channel authenticated
    /// independently of the relationship you are establishing: a telephone
    /// call to a voice you recognise, a meeting, a courier. A call to a number
    /// the counterparty supplied over the unauthenticated channel authenticates
    /// nothing and merely feels like it does.
    #[arg(long = "key", value_name = "SID=BASE64")]
    pub keys: Vec<String>,

    /// A JSON file of bound keys, `{"<sovereign_id>": "<base64>"}`. Convenient
    /// for several partners; it establishes nothing that --key does not.
    #[arg(long, value_name = "FILE")]
    pub keyring: Option<PathBuf>,

    /// The evaluation date, `YYYY-MM-DD`. Defaults to today in UTC — never the
    /// local date, which would let two parties either side of local midnight
    /// reach different verdicts on the day a package falls due.
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub now: Option<String>,

    /// Write the agreement's canonical bytes here — no trailing newline, no
    /// byte-order mark, because that is the digest input.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// Print the agreement's canonical form on stdout.
    #[arg(long)]
    pub print: bool,

    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct ComposeReport<'a> {
    verb: &'static str,
    verdict: Verdict,
    now: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    /// The refusal rule that fired, when one did. A refusal produces no
    /// agreement and no digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<&'a str>,
    outcomes: Vec<FedOutcome>,
}

pub fn run_compose(a: &ComposeArgs, out: &mut dyn Write) -> anyhow::Result<i32> {
    let now = evaluation_date(a.now.as_deref())?;
    let keys = build_keyring(&a.keys, a.keyring.as_deref())?;

    let mut outcomes = Vec::new();
    let (p, q) = match (
        load_for_compose(&a.first, &mut outcomes),
        load_for_compose(&a.second, &mut outcomes),
    ) {
        (Some(p), Some(q)) => (p, q),
        _ => {
            report_compose(out, a, Verdict::Fail, &now, None, None, outcomes)?;
            return Ok(EXIT_FED_MALFORMED);
        }
    };

    match compose(&p, &q, now, &keys) {
        Composed::Refused(f) => {
            outcomes.push(FedOutcome::fail(
                "compose",
                format!("refused: {} ({})", f.rule, f.spec),
                f.detail.clone(),
            ));
            outcomes.push(FedOutcome::skip(
                "digest",
                "a refusal produces no agreement and no digest. This is an outcome, not an error \
                 condition: it is the mechanism working",
            ));
            report_compose(out, a, Verdict::Fail, &now, None, Some(f.rule), outcomes)?;
            Ok(EXIT_FED_REFUSED)
        }
        Composed::Agreement {
            value,
            canonical: c,
            digest,
        } => {
            outcomes.push(FedOutcome::ok(
                "signatures",
                "both packages verify under their bound keys",
            ));
            outcomes.push(FedOutcome::ok("compose", describe_agreement(&value)));
            outcomes.push(FedOutcome::ok(
                "digest",
                format!(
                    "{digest} — send this to the counterparty by any means at all. Altering it in \
                     transit causes a visible mismatch; causing a false MATCH would mean forging \
                     a signature, so the channel does not need to be trusted"
                ),
            ));
            if let Some(path) = &a.out {
                // The canonical form is bytes, not a text file. Writing it with
                // a trailing newline is the single commonest way to produce a
                // different digest over an identical agreement.
                std::fs::write(path, c.as_bytes())
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
                outcomes.push(FedOutcome::ok(
                    "write",
                    format!("{} — canonical bytes, no trailing newline", path.display()),
                ));
            }
            report_compose(out, a, Verdict::Ok, &now, Some(&digest), None, outcomes)?;
            if a.print {
                // stdout, unterminated, so `goat federate compose --print … |
                // shasum -a 256` gives the digest above.
                write!(out, "{c}")?;
                let _ = out.flush();
            }
            Ok(EXIT_OK)
        }
    }
}

fn report_compose(
    out: &mut dyn Write,
    a: &ComposeArgs,
    verdict: Verdict,
    now: &NaiveDate,
    digest: Option<&str>,
    rule: Option<&'static str>,
    outcomes: Vec<FedOutcome>,
) -> std::io::Result<()> {
    if a.json {
        emit_json(
            out,
            &ComposeReport {
                verb: "compose",
                verdict,
                now: now.to_string(),
                digest: digest.map(str::to_string),
                rule,
                outcomes,
            },
        )
    } else {
        writeln!(
            out,
            "compose {} + {} at now={now}",
            label(&a.first),
            label(&a.second)
        )?;
        print_outcomes(out, &outcomes)
    }
}

fn load_for_compose(path: &Path, outcomes: &mut Vec<FedOutcome>) -> Option<Package> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            outcomes.push(FedOutcome::fail(
                format!("read:{}", label(path)),
                format!("{e}"),
                "The two packages are read from files you name and nothing else.",
            ));
            return None;
        }
    };
    match parse_package(&bytes) {
        Ok(p) => Some(p),
        Err(f) => {
            // A malformed package is refused BEFORE composition begins, by the
            // structural rules of §3 and §4. Reporting it as a composition
            // refusal would be the right verdict from the wrong place.
            outcomes.push(FedOutcome::fail(
                format!("well-formed:{}", label(path)),
                format!(
                    "{} ({}) at `{}`",
                    f.rule,
                    f.spec,
                    if f.path.is_empty() {
                        "(top level)"
                    } else {
                        &f.path
                    }
                ),
                format!(
                    "{}. This is a property of one package, refused before composition begins — \
                     not one of composition's seven refusals. Run `goat federate lint` on it.",
                    f.detail
                ),
            ));
            None
        }
    }
}

fn describe_agreement(v: &JVal) -> String {
    let parties: Vec<&str> = v
        .get("parties")
        .and_then(JVal::as_arr)
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("sovereign_id").and_then(JVal::as_str))
                .collect()
        })
        .unwrap_or_default();
    let flows = v
        .get("flows")
        .and_then(JVal::as_arr)
        .map(<[JVal]>::len)
        .unwrap_or(0);
    let transport = match v
        .get("transport")
        .and_then(|t| t.get("kind"))
        .and_then(JVal::as_str)
    {
        Some(k) => k.to_string(),
        // Not an error, and it should not be read as one: the parties have
        // agreed data terms and no path (§8.1).
        None => "no transport — data terms agreed, no path".to_string(),
    };
    let expires = v.get("expires").and_then(JVal::as_str).unwrap_or("?");
    format!(
        "{} and {}, through {expires}; {transport}; {flows} flow{}",
        parties.first().unwrap_or(&"?"),
        parties.get(1).unwrap_or(&"?"),
        if flows == 1 { "" } else { "s" }
    )
}

/// `now` is the current date in UTC — never the local date of whoever happens
/// to be composing (§6). §4 took clock skew out of the packages by carrying
/// dates and no times; leaving `now` unstated would put it straight back one
/// layer down, in the evaluator.
fn evaluation_date(explicit: Option<&str>) -> anyhow::Result<NaiveDate> {
    match explicit {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("--now `{s}` is not a YYYY-MM-DD date")),
        None => Ok(chrono::Utc::now().date_naive()),
    }
}

/// The bound keys from `--key` flags and a `--keyring` file.
pub fn build_keyring(flags: &[String], file: Option<&Path>) -> anyhow::Result<KeyRing> {
    let mut ring = KeyRing::default();
    if let Some(path) = file {
        let bytes = read_file(path)?;
        let map: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes)
            .map_err(|e| {
                anyhow::anyhow!(
                    "{}: expected a JSON object of sovereign_id → base64 key: {e}",
                    path.display()
                )
            })?;
        for (sid, b64) in map {
            let key = decode_verifying_key(&b64)
                .map_err(|e| anyhow::anyhow!("{}: key for `{sid}`: {e}", path.display()))?;
            ring.insert(&sid, key);
        }
    }
    for flag in flags {
        let (sid, b64) = flag
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--key `{flag}` is not `<sovereign_id>=<base64>`"))?;
        let key =
            decode_verifying_key(b64).map_err(|e| anyhow::anyhow!("--key for `{sid}`: {e}"))?;
        ring.insert(sid, key);
    }
    Ok(ring)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_rejects_a_pem_public_key() {
        // The tell that the wrong thing is being transcribed (§5.1).
        let err = build_keyring(
            &["xaa=MCowBQYDK2VwAyEAVTFghAMPXAcjUb9cFBKCmt7XtJvaCCqw".into()],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("SubjectPublicKeyInfo"), "{err}");
    }
}

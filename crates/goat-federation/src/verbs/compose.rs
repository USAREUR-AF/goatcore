// compose: two packages, one agreement, one digest. Pure.

use super::{
    emit_json, label, print_outcomes, read_file, refusal, FedOutcome, Verdict, BAD_BOUND_KEY,
    BAD_NOW, EXIT_FED_MALFORMED, EXIT_FED_REFUSED, EXIT_OK, FILE_UNWRITABLE,
};
use crate::compose::{compose, compose_any, Composed, KeyRing};
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

    /// The package version this peering selected: v1 (the default) composes
    /// only a v1 pair, by v1's literal pin; v2 composes only a v2 pair. Both
    /// parties select the same one, so any other pair is refused.
    #[arg(long, value_enum, default_value_t = super::PackageVersion::V1)]
    pub package_version: super::PackageVersion,

    /// Print the agreement's canonical form on stdout.
    #[arg(long)]
    pub print: bool,

    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,

    /// A party the agreement must be between, by `sovereign_id`: once or
    /// twice (ours, the counterparty's). Every party named must be one of the
    /// two packages' parties, or compose refuses (GB-191) and writes nothing.
    /// A keyring can bind more parties than the two of one peering; this
    /// holds the agreement to the two it is for.
    #[arg(long = "party", value_name = "SOVEREIGN_ID")]
    pub parties: Vec<String>,
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
    check_parties(&a.parties)?;

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

    // The parties named: each must be one of the two packages' parties.
    let have = [p.sovereign_id.as_str(), q.sovereign_id.as_str()];
    if let Some(missing) = a.parties.iter().find(|w| !have.contains(&w.as_str())) {
        let other: Vec<&str> = have.iter().copied().filter(|h| !a.parties.iter().any(|w| w == h)).collect();
        outcomes.push(FedOutcome::fail(
            "parties",
            format!(
                "GB-191: the agreement is to be between {}, and the packages are {} and {}: {} is not one of them, and {} is not a party named",
                a.parties.join(" and "),
                have[0],
                have[1],
                missing,
                if other.is_empty() { "none".to_string() } else { other.join(" and ") }
            ),
            "Compose only the packages of the parties named. Ask which package is the counterparty's; do not compose another party's.",
        ));
        outcomes.push(FedOutcome::skip(
            "digest",
            "a refusal produces no agreement and no digest",
        ));
        report_compose(out, a, Verdict::Fail, &now, None, Some("parties"), outcomes)?;
        return Ok(EXIT_FED_REFUSED);
    }

    let composed = match a.package_version {
        super::PackageVersion::V1 => compose(&p, &q, now, &keys),
        super::PackageVersion::V2 => {
            // The selection is v2: a pair the router would compose as v1 is
            // still refused, naming the selection, once both are authenticated.
            let want = a.package_version.package_spec();
            match compose_any(&p, &q, now, &keys) {
                Composed::Agreement { .. } if p.spec != want || q.spec != want => {
                    let other = if p.spec != want { &p } else { &q };
                    Composed::Refused(crate::json::Finding::new(
                        "C_SPEC_PIN",
                        "§6 step 1",
                        format!("{}.spec", other.sovereign_id),
                        format!(
                            "`{}` declares spec `{}`, and this peering selects `{want}`; both \
                             parties select the same version before either emits it",
                            other.sovereign_id, other.spec
                        ),
                    ))
                }
                c => c,
            }
        }
    };
    match composed {
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
                    .map_err(|e| refusal(FILE_UNWRITABLE, format!("{}: {e}", path.display())))?;
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

/// `--party`: at most two, distinct, each a plain sovereign id.
fn check_parties(parties: &[String]) -> anyhow::Result<()> {
    if parties.len() > 2 {
        anyhow::bail!("--party names the two parties of one agreement at most, not {}", parties.len());
    }
    if parties.len() == 2 && parties[0] == parties[1] {
        anyhow::bail!("--party {} is given twice: an agreement is between two parties", parties[0]);
    }
    if let Some(bad) = parties.iter().find(|p| {
        p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    }) {
        anyhow::bail!("--party '{bad}' is not a sovereign id");
    }
    Ok(())
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
            .map_err(|_| refusal(BAD_NOW, format!("--now `{s}` is not a YYYY-MM-DD date"))),
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
                refusal(
                    BAD_BOUND_KEY,
                    format!(
                        "{}: expected a JSON object of sovereign_id → base64 key: {e}",
                        path.display()
                    ),
                )
            })?;
        for (sid, b64) in map {
            let key = decode_verifying_key(&b64).map_err(|e| {
                refusal(
                    BAD_BOUND_KEY,
                    format!("{}: key for `{sid}`: {e}", path.display()),
                )
            })?;
            ring.insert(&sid, key);
        }
    }
    for flag in flags {
        let (sid, b64) = flag.split_once('=').ok_or_else(|| {
            refusal(
                BAD_BOUND_KEY,
                format!("--key `{flag}` is not `<sovereign_id>=<base64>`"),
            )
        })?;
        let key = decode_verifying_key(b64)
            .map_err(|e| refusal(BAD_BOUND_KEY, format!("--key for `{sid}`: {e}")))?;
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

    use crate::json::{canonical, parse_document, JVal};

    fn vectors() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors")
    }

    fn text(p: &str) -> String {
        std::fs::read_to_string(vectors().join(p)).unwrap().trim().to_string()
    }

    /// `party`'s conformance package, given `sovereign_id` `as_id` and signed
    /// again with `party`'s committed test seed (worthless keys, public on purpose).
    fn package_as(party: &str, as_id: &str) -> Vec<u8> {
        let bytes = std::fs::read(vectors().join(format!("compose-full/package-{party}.canonical.json"))).unwrap();
        let JVal::Obj(mut top) = parse_document(&bytes).unwrap().without_member("signature") else {
            panic!()
        };
        for (k, v) in top.iter_mut() {
            if k == "sovereign_id" {
                *v = JVal::Str(as_id.into());
            }
        }
        let hex = text(&format!("testkeys/{party}.seed.hex"));
        let seed: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
        let sig = ed25519_dalek::Signer::sign(&sk, canonical(&JVal::Obj(top.clone())).as_bytes());
        use base64::Engine;
        top.push((
            "signature".into(),
            JVal::Str(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())),
        ));
        canonical(&JVal::Obj(top)).into_bytes()
    }

    struct Setup {
        _d: tempfile::TempDir,
        ours: PathBuf,
        theirs: PathBuf,
        out: PathBuf,
        keys: Vec<String>,
    }

    /// Our package (xaa) and a counterparty package that is `theirs_id`'s, with
    /// a key for each of xaa, xab and a third party xac bound, as in a keyring
    /// shared by several peerings.
    fn setup(theirs_id: &str) -> Setup {
        let d = tempfile::tempdir().unwrap();
        let ours = d.path().join("ours.json");
        let theirs = d.path().join("theirs.json");
        std::fs::write(&ours, package_as("xaa", "xaa")).unwrap();
        std::fs::write(&theirs, package_as("xab", theirs_id)).unwrap();
        let xab = text("testkeys/xab.public.b64");
        let keys = vec![
            format!("xaa={}", text("testkeys/xaa.public.b64")),
            format!("xab={xab}"),
            // The third party's package is signed by the key bound for it.
            format!("xac={xab}"),
        ];
        let out = d.path().join("agreement.json");
        Setup { _d: d, ours, theirs, out, keys }
    }

    fn args(s: &Setup, parties: &[&str]) -> ComposeArgs {
        ComposeArgs {
            first: s.ours.clone(),
            second: s.theirs.clone(),
            keys: s.keys.clone(),
            keyring: None,
            now: Some("2026-10-01".into()),
            out: Some(s.out.clone()),
            print: false,
            json: false,
            parties: parties.iter().map(|p| p.to_string()).collect(),
            package_version: crate::verbs::PackageVersion::V1,
        }
    }

    /// With the parties named, compose composes only between them: a genuine
    /// package of another party, whose key is bound, is refused (GB-191) and
    /// no agreement is written. Without them it composes any two whose keys
    /// it holds, as before.
    #[test]
    fn compose_takes_only_the_parties_it_is_given() {
        let s = setup("xab");
        let mut out = Vec::new();
        assert_eq!(run_compose(&args(&s, &["xaa", "xab"]), &mut out).unwrap(), EXIT_OK, "{}", String::from_utf8_lossy(&out));
        assert!(s.out.exists());

        let s = setup("xac");
        let mut out = Vec::new();
        assert_eq!(run_compose(&args(&s, &[]), &mut out).unwrap(), EXIT_OK, "without --party, as before: {}", String::from_utf8_lossy(&out));
        let s = setup("xac");
        let mut out = Vec::new();
        let rc = run_compose(&args(&s, &["xaa", "xab"]), &mut out).unwrap();
        let said = String::from_utf8_lossy(&out);
        assert_eq!(rc, EXIT_FED_REFUSED, "{said}");
        assert!(said.contains("GB-191") && said.contains("xac") && said.contains("xab"), "{said}");
        assert!(!s.out.exists(), "an agreement was written: {said}");
        // One party named is enough to pin the counterparty.
        let s = setup("xac");
        let mut out = Vec::new();
        assert_eq!(run_compose(&args(&s, &["xab"]), &mut out).unwrap(), EXIT_FED_REFUSED);
        assert!(!s.out.exists());
    }

    /// --party is a sovereign id, at most twice, and two that differ.
    #[test]
    fn compose_refuses_a_party_list_it_cannot_hold_to() {
        let s = setup("xab");
        for bad in [vec!["xaa", "xab", "xac"], vec!["xab", "xab"], vec!["x y"]] {
            let mut out = Vec::new();
            assert!(run_compose(&args(&s, &bad), &mut out).is_err(), "{bad:?}");
            assert!(!s.out.exists(), "{bad:?}");
        }
    }
}

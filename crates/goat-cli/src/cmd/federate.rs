// `goat federate` — the verbs an operator runs to federate with a partner.
//
// docs/design/federation-package-spec.md is the authority; this is one
// implementation of it, held to the byte-exact conformance vectors in
// ops/federation/vectors/ (which are normative: if this and a vector
// disagree, this is wrong).
//
// PRIVILEGE, and why it is arranged this way rather than merely documented.
// A partner has to be able to trust that handing us their package cannot
// touch anything, and "we promise" is not a property anyone can check. So:
//
//   lint / compose / diff   PURE. `goat_federation::{json,package,compose,diff}`
//                           read no file the operator did not name, open no
//                           socket, hold no credential and do not read the
//                           clock — `now` and the verifying keys arrive as
//                           arguments. They run in an empty container.
//   init / verify           read local state. Never write.
//   apply                   the only verb that writes, and only under
//                           --confirm.
//
// The reporting shape mirrors `goat doctor`: one structured outcome per
// check, a verdict, and a hint on failure. There is one such convention in
// this binary and this is it.

use crate::cmd::doctor::Verdict;
use crate::exit_codes;
use goat_federation::compose::{compose, digest_of, Composed, KeyRing};
use goat_federation::derive;
use goat_federation::diff::{diff, ByteCause, DiffOutcome, Side};
use goat_federation::json::{canonical, JVal};
use goat_federation::package::{decode_verifying_key, parse_package, warnings, Package};
use goat_federation::render;
use chrono::NaiveDate;
use clap::{Args, Subcommand};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum FederateCmd {
    /// Generate our half of a federation, read from the running system.
    ///
    /// Every capability in the package is MEASURED here, not typed: what
    /// transports are installed and configured, what image the router is
    /// actually running, what version the fabric is. A capability an operator
    /// types is a claim, and a wrong claim stays wrong until the link fails in
    /// front of a partner.
    ///
    /// Where a value cannot be derived this fails and names it. It never
    /// prompts and never fills one in — a silent default is how a link ends up
    /// with no rate limit.
    ///
    /// Exit 23 means something could not be derived; the list is the output.
    Init(InitArgs),

    /// Check one package: is it well-formed, and does it agree with itself?
    ///
    /// Runs the structural rules on a single file — required members, integers
    /// that are really integers, no duplicate member names, complete blocks,
    /// a signature of the right shape. It does NOT verify the signature: the
    /// verifying key is bound to a party over a channel authenticated
    /// independently of this link, so a lone package cannot be authenticated
    /// by looking at it. `compose` does that.
    ///
    /// Reads one file and nothing else. No network, no credentials.
    ///
    /// Exit 21 means the package is refused, and the finding names the rule.
    Lint(LintArgs),

    /// Combine two packages into one agreement and print its digest.
    ///
    /// Both parties compose independently and compare the digest. Equal
    /// digests prove both sides hold the identical agreement — that is the
    /// whole point of the format, and the comparison may travel by any channel
    /// at all, including one nobody trusts.
    ///
    /// A refusal is an outcome, not a failure: it is the mechanism working.
    /// Exit 20, with the rule that fired.
    ///
    /// Reads two files and the keys you pass. No network, no credentials, no
    /// host access — this runs in an empty container, which is what lets a
    /// partner trust it with their package.
    Compose(ComposeArgs),

    /// Two digests disagree? Run this. It names the field.
    ///
    /// This is the verb for the one loud failure this design has. Two parties
    /// compose the same pair, compare digests, and they differ — so each side
    /// prints its agreement and this diffs them, and the offending field names
    /// itself. Loud in the first hour instead of silent for three weeks.
    ///
    /// It also answers the case that reads as impossible: the digests differ
    /// and the two agreements are identical. There is then no field to show,
    /// and the instinct is to re-run everything. The answer is one measurement
    /// — a trailing newline, a byte-order mark, or an encoding difference —
    /// and this prints which of the three it is.
    ///
    /// Exit 22 when they differ, 0 when they match.
    Diff(DiffArgs),

    /// Could we actually deliver this agreement? Dry-run it here.
    ///
    /// Agreeing is not the same as being able to deliver. This runs the
    /// agreement through our own configuration generator WITHOUT applying it
    /// and asserts the output would be valid on this system, then prints the
    /// conformance attestation to publish: one line over the digest, signed
    /// with the same key that signed our package.
    ///
    /// Reads local state. Changes nothing.
    ///
    /// Exit 24 means we could not deliver it, and the failing check says why.
    Verify(VerifyArgs),

    /// Render the access rules and configuration this agreement implies.
    ///
    /// The only verb here that writes. The agreement is the sole input, so the
    /// release set we forward and the prefix the far side admits cannot drift
    /// apart by being typed twice.
    ///
    /// Prints what it would write and stops. Pass --confirm to write.
    Apply(ApplyArgs),
}

pub async fn run(ctx: &super::Ctx, cmd: FederateCmd) -> anyhow::Result<()> {
    match cmd {
        FederateCmd::Init(a) => run_init(ctx, a),
        FederateCmd::Lint(a) => run_lint(a),
        FederateCmd::Compose(a) => run_compose(a),
        FederateCmd::Diff(a) => run_diff(a),
        FederateCmd::Verify(a) => run_verify(ctx, a),
        FederateCmd::Apply(a) => run_apply(ctx, a),
    }
}

/// An exit code carried out through `anyhow` without a second error message —
/// every verb has already printed its own structured report by the time it
/// bails, and a bare `error:` line on top of it would say less than nothing.
#[derive(Debug)]
struct Exit(i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never rendered: main.rs reads the code off this type instead.
        write!(f, "federate exited {}", self.0)
    }
}
impl std::error::Error for Exit {}

/// `main.rs` asks this before falling back to EXIT_GENERIC.
pub fn exit_code_of(err: &anyhow::Error) -> Option<i32> {
    err.downcast_ref::<Exit>().map(|e| e.0)
}

fn bail_with(code: i32) -> anyhow::Result<()> {
    Err(anyhow::Error::new(Exit(code)))
}

// ---------------------------------------------------------------------------
// Shared reporting shape
// ---------------------------------------------------------------------------

/// One check's outcome. Same three fields `goat doctor` uses: a stable name, a
/// verdict, a one-line summary, and a hint that is non-empty exactly when the
/// verdict is Fail.
#[derive(Debug, Clone, Serialize)]
pub struct FedOutcome {
    pub check: String,
    pub verdict: Verdict,
    pub summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// Advice attached to a check that PASSED. `goat doctor`'s three verdicts
    /// are reused rather than forked — a warning is not a fourth verdict, it is
    /// an Ok with something worth saying — but it needs its own glyph, or a
    /// package that composes correctly renders identically to one that does not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub warn: bool,
    /// Pre-formatted text — a file template, or the exact bytes to sign —
    /// printed verbatim and never re-wrapped. Prose wraps; a template that has
    /// been wrapped is no longer a template, and bytes that have been wrapped
    /// are no longer the bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
}

impl FedOutcome {
    fn ok(check: impl Into<String>, summary: impl Into<String>) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Ok,
            summary: summary.into(),
            hint: String::new(),
            warn: false,
            block: None,
        }
    }
    fn fail(check: impl Into<String>, summary: impl Into<String>, hint: impl Into<String>) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Fail,
            summary: summary.into(),
            hint: hint.into(),
            warn: false,
            block: None,
        }
    }
    fn skip(check: impl Into<String>, summary: impl Into<String>) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Skip,
            summary: summary.into(),
            hint: String::new(),
            warn: false,
            block: None,
        }
    }
    /// A check that passed, with advice. The verdict stays Ok because it is.
    fn warn(check: impl Into<String>, summary: impl Into<String>, hint: impl Into<String>) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Ok,
            summary: summary.into(),
            hint: hint.into(),
            warn: true,
            block: None,
        }
    }
}

fn glyph(o: &FedOutcome) -> &'static str {
    match (o.verdict, o.warn) {
        (Verdict::Ok, true) => "[!]",
        (Verdict::Ok, false) => "[✓]",
        (Verdict::Skip, _) => "[⏭]",
        (Verdict::Fail, _) => "[✗]",
        (Verdict::Warn, _) => "[!]",
    }
}

fn print_outcome(o: &FedOutcome) {
    println!("{} {} — {}", glyph(o), o.check, o.summary);
    if !o.hint.is_empty() {
        for line in wrap(&o.hint, 88) {
            println!("     {line}");
        }
    }
    if let Some(b) = &o.block {
        println!();
        println!("{b}");
    }
}

/// Wrap prose to a width so a hint stays readable in a terminal at 03:00.
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn emit_json<T: Serialize>(v: &T) {
    match serde_json::to_string(v) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: could not serialise the report: {e}"),
    }
}

fn read_file(p: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))
}

fn label(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct LintArgs {
    /// The package to check.
    pub package: PathBuf,
    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct LintReport<'a> {
    verb: &'static str,
    file: String,
    verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<&'a str>,
    outcomes: Vec<FedOutcome>,
}

fn run_lint(a: LintArgs) -> anyhow::Result<()> {
    let bytes = read_file(&a.package)?;
    let mut outcomes = Vec::new();
    let mut rule = None;
    let verdict;

    match parse_package(&bytes) {
        Err(f) => {
            verdict = Verdict::Fail;
            rule = Some(f.rule);
            outcomes.push(FedOutcome::fail(
                "well-formed",
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
                f.detail.clone(),
            ));
        }
        Ok(pkg) => {
            verdict = Verdict::Ok;
            outcomes.push(FedOutcome::ok(
                "well-formed",
                format!(
                    "`{}`, issued {}, expires {} — every required member present and every number \
                     an integer",
                    pkg.sovereign_id, pkg.issued, pkg.expires
                ),
            ));
            outcomes.push(FedOutcome::ok("self-consistent", describe_blocks(&pkg)));
            outcomes.push(FedOutcome::ok(
                "signature-shape",
                "88 base64 characters in the standard alphabet, decoding to exactly 64 bytes",
            ));
            // Say what was NOT checked, every time, rather than implying a
            // completeness this verb does not have.
            outcomes.push(FedOutcome::skip(
                "signature-verify",
                "not checked here — the verifying key is bound to a party over a channel \
                 authenticated independently of this link (§5.1), never taken from the package. \
                 `goat federate compose` verifies it",
            ));
            // Advice, not a refusal. Folding it into the verdict would make a
            // clean package indistinguishable from a refused one at a glance.
            for w in warnings(&pkg, &bytes) {
                outcomes.push(FedOutcome::warn(
                    w.rule,
                    format!("advice ({})", w.spec),
                    w.detail,
                ));
            }
        }
    }

    // A warning does not change the verdict: the package is well-formed or it
    // is not, and folding advice into that answer would make a clean package
    // indistinguishable from a refused one at a glance.
    if a.json {
        emit_json(&LintReport {
            verb: "lint",
            file: a.package.display().to_string(),
            verdict,
            rule,
            outcomes,
        });
    } else {
        println!("package {}", a.package.display());
        for o in &outcomes {
            print_outcome(o);
        }
    }
    if verdict == Verdict::Fail {
        return bail_with(exit_codes::EXIT_FED_MALFORMED);
    }
    Ok(())
}

/// Presence is the declaration: what a package carries is what it is willing
/// to do (§3).
fn describe_blocks(pkg: &Package) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "fabric {} ({}), PQC {}",
        pkg.capabilities.fabric_version,
        if pkg.capabilities.fabric_fips_image {
            "validated build"
        } else {
            "unvalidated build"
        },
        if pkg.capabilities.pqc_capable {
            "capable"
        } else {
            "not capable"
        }
    ));
    match &pkg.transport_offer {
        Some(t) => parts.push(format!(
            "willing to connect ({}, MTU {}, {}, key {}…)",
            t.endpoint,
            t.measured_mtu,
            if t.listener { "listens" } else { "dials" },
            &t.public_key[..8.min(t.public_key.len())]
        )),
        None => parts.push("no transport offer".into()),
    }
    match &pkg.data_offer {
        Some(d) => {
            let send = d.release.as_ref().map(|r| format!("sends {r:?}"));
            let recv = d.accept.map(|a| {
                format!(
                    "accepts up to {} Hz / {} bytes / {} keys",
                    a.hz, a.bytes_per_message, a.distinct_keys
                )
            });
            let halves: Vec<String> = [send, recv].into_iter().flatten().collect();
            parts.push(halves.join(", "));
        }
        None => parts.push("no data offer".into()),
    }
    parts.join("; ")
}

// ---------------------------------------------------------------------------
// compose
// ---------------------------------------------------------------------------

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

fn run_compose(a: ComposeArgs) -> anyhow::Result<()> {
    let now = evaluation_date(a.now.as_deref())?;
    let keys = build_keyring(&a.keys, a.keyring.as_deref())?;

    let mut outcomes = Vec::new();
    let (p, q) = match (
        load_for_compose(&a.first, &mut outcomes),
        load_for_compose(&a.second, &mut outcomes),
    ) {
        (Some(p), Some(q)) => (p, q),
        _ => {
            report_compose(&a, Verdict::Fail, &now, None, None, outcomes);
            return bail_with(exit_codes::EXIT_FED_MALFORMED);
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
            report_compose(&a, Verdict::Fail, &now, None, Some(f.rule), outcomes);
            bail_with(exit_codes::EXIT_FED_REFUSED)
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
            report_compose(&a, Verdict::Ok, &now, Some(&digest), None, outcomes);
            if a.print {
                // stdout, unterminated, so `goat federate compose --print … |
                // shasum -a 256` gives the digest above.
                print!("{c}");
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
            }
            Ok(())
        }
    }
}

fn report_compose(
    a: &ComposeArgs,
    verdict: Verdict,
    now: &NaiveDate,
    digest: Option<&str>,
    rule: Option<&'static str>,
    outcomes: Vec<FedOutcome>,
) {
    if a.json {
        emit_json(&ComposeReport {
            verb: "compose",
            verdict,
            now: now.to_string(),
            digest: digest.map(str::to_string),
            rule,
            outcomes,
        });
    } else {
        println!(
            "compose {} + {} at now={now}",
            label(&a.first),
            label(&a.second)
        );
        for o in &outcomes {
            print_outcome(o);
        }
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

fn build_keyring(flags: &[String], file: Option<&Path>) -> anyhow::Result<KeyRing> {
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

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// The agreement we composed — the exact bytes we hashed.
    pub ours: PathBuf,
    /// The agreement they composed.
    pub theirs: PathBuf,
    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct DiffReport {
    verb: &'static str,
    verdict: Verdict,
    ours: String,
    theirs: String,
    /// `identical`, `bytes`, `fields`, or `unreadable` — what KIND of
    /// disagreement this is, which is the first thing to know.
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<DiffField>,
    outcomes: Vec<FedOutcome>,
}

#[derive(Serialize)]
struct DiffField {
    path: String,
    ours: String,
    theirs: String,
}

fn run_diff(a: DiffArgs) -> anyhow::Result<()> {
    let ours = read_file(&a.ours)?;
    let theirs = read_file(&a.theirs)?;
    // Fixed side names rather than filenames. Two agreements very often arrive
    // as two files with the SAME basename — one per directory, one per party —
    // and a report naming both sides `agreement.canonical.json` says nothing
    // about which is which, so the paths are printed once in the header and
    // every line below refers to a side.
    let (ours_label, theirs_label) = ("ours", "theirs");

    let mut outcomes = Vec::new();
    let mut fields = Vec::new();
    let mut digest = None;
    let kind;
    let verdict;

    match diff(&ours, &theirs, ours_label, theirs_label) {
        DiffOutcome::Identical { digest: d } => {
            kind = "identical";
            verdict = Verdict::Ok;
            outcomes.push(FedOutcome::ok(
                "digest",
                format!(
                    "{d} — equal. Both sides hold the identical agreement, and transcription \
                         error is now impossible"
                ),
            ));
            digest = Some(d);
        }
        DiffOutcome::Bytes(b) => {
            kind = "bytes";
            verdict = Verdict::Fail;
            let cause = match b.cause {
                ByteCause::TrailingNewline { .. } => "trailing-newline",
                ByteCause::ByteOrderMark { .. } => "byte-order-mark",
                ByteCause::EncodingDifference { .. } => "encoding-difference",
                ByteCause::NotCanonical { .. } => "not-canonical",
            };
            // The whole point of this branch: there is NO field to show, and
            // saying "no differences found" here would send someone to re-run
            // everything. Name the cause instead.
            outcomes.push(FedOutcome::fail(
                format!("bytes:{cause}"),
                b.conclusion.clone(),
                b.fix.clone(),
            ));
            outcomes.push(FedOutcome::skip(
                "fields",
                "no field differs — the two agreements are the same agreement. The disagreement \
                 is in the bytes that were hashed",
            ));
        }
        DiffOutcome::Fields(diffs) => {
            kind = "fields";
            verdict = Verdict::Fail;
            for d in &diffs {
                outcomes.push(FedOutcome::fail(
                    format!("field:{}", d.path),
                    format!("{ours_label} {} · {theirs_label} {}", d.left, d.right),
                    String::new(),
                ));
                fields.push(DiffField {
                    path: d.path.clone(),
                    ours: d.left.clone(),
                    theirs: d.right.clone(),
                });
            }
            outcomes.push(FedOutcome::skip(
                "cause",
                format!(
                    "{} field{}. Each is a value composed from the two packages, so the \
                     disagreement is upstream: compare the packages that produced these, not the \
                     agreements",
                    diffs.len(),
                    if diffs.len() == 1 {
                        " differs"
                    } else {
                        "s differ"
                    }
                ),
            ));
        }
        DiffOutcome::Unreadable { side, finding } => {
            kind = "unreadable";
            verdict = Verdict::Fail;
            let which = match side {
                Side::Left => ours_label,
                Side::Right => theirs_label,
            };
            outcomes.push(FedOutcome::fail(
                "readable",
                format!("{which}: {} ({})", finding.rule, finding.spec),
                finding.detail.clone(),
            ));
        }
    }

    if a.json {
        emit_json(&DiffReport {
            verb: "diff",
            verdict,
            ours: a.ours.display().to_string(),
            theirs: a.theirs.display().to_string(),
            kind,
            digest,
            fields,
            outcomes,
        });
    } else {
        println!(
            "diff\n  ours:   {}\n  theirs: {}",
            a.ours.display(),
            a.theirs.display()
        );
        for o in &outcomes {
            print_outcome(o);
        }
    }
    if verdict == Verdict::Fail {
        return bail_with(exit_codes::EXIT_FED_MISMATCH);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Also derive `transport_offer` — that we are willing to connect.
    ///
    /// Omitted, the package carries no transport offer, which is a complete
    /// and valid declaration: presence is the declaration, and a party willing
    /// to be recognised and nothing more says so by carrying neither optional
    /// block.
    #[arg(long)]
    pub with_transport: bool,

    /// Also derive `data_offer` — that we are willing to exchange.
    #[arg(long)]
    pub with_data: bool,

    /// Our Ed25519 signing key: 64 hex characters (the RFC 8032 seed) or 44
    /// base64 characters. Without it this prints the exact bytes to sign and
    /// stops — it will not invent a key.
    #[arg(long, value_name = "FILE")]
    pub signing_key: Option<PathBuf>,

    /// Write the package's canonical bytes here.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct InitReport {
    verb: &'static str,
    verdict: Verdict,
    /// Members that could not be derived. The list IS the output of a failed
    /// init: it is what has to be true before this package can exist.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    underived: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    outcomes: Vec<FedOutcome>,
}

/// The declared half: organisational identity and the link parameters two
/// parties agree rather than one party observes. Read from a file that is
/// reviewed the way production configuration is — never prompted for.
#[derive(Debug, serde::Deserialize)]
struct Declaration {
    sovereign_id: String,
    /// How long our half is offered for, in days from today.
    term_days: i64,
    cut_contact: String,
    #[serde(default)]
    transport: Option<DeclaredTransport>,
    #[serde(default)]
    data: Option<DeclaredData>,
}

#[derive(Debug, serde::Deserialize)]
struct DeclaredTransport {
    /// The federation interface. Its public key is READ from it, never set here.
    interface: String,
    endpoint: String,
    listener: bool,
    proposed_link_subnet: String,
    /// A measured path MTU. An interface MTU is not one and is never
    /// substituted for one — but it is cross-checked against the interface,
    /// because a path MTU larger than the interface carrying it cannot be right.
    measured_mtu: i64,
}

#[derive(Debug, serde::Deserialize)]
struct DeclaredData {
    #[serde(default)]
    release: Option<Vec<String>>,
    #[serde(default)]
    accept_hz: Option<i64>,
    #[serde(default)]
    accept_bytes_per_message: Option<i64>,
    #[serde(default)]
    accept_distinct_keys: Option<i64>,
}

const DECLARATION_TEMPLATE: &str = r#"# Our half of a federation: the part no probe of a running system can produce.
#
# `sovereign_id` is the organisation's name, from ISO 3166-1 alpha-3 lowercased
# (usa, isl, svk, svn), an established short name for a multinational body
# (nato), or a parent plus a local suffix (nato-jfcnp). It is NOT one of the
# organisation's environments: a sandbox and a production fabric share one
# sovereign_id, and putting an environment name here produces peering names that
# cannot survive that environment being retired.
#
# NOTHING in this file may name a capability. `goat federate init` measures
# capabilities.transports, capabilities.fabric_fips_image,
# capabilities.fabric_version, capabilities.pqc_capable and the transport public
# key from the running system, and refuses to read any of them from here — a
# capability typed by an operator is a claim, and a wrong claim stays wrong
# until the link fails in front of a partner.

sovereign_id = "xaa"
term_days    = 91
cut_contact  = "fed-ops@example"

# Optional. Present means "willing to connect"; --with-transport reads it.
# [transport]
# interface            = "wg-fed0"        # its public key is READ from it
# endpoint             = "_fed._udp.example"
# listener             = true
# proposed_link_subnet = "169.254.10.0/31"
# measured_mtu         = 1380             # from an actual path-MTU measurement

# Optional. Present means "willing to exchange"; --with-data reads it.
# Each half is complete once present, independently of the other: `release`
# alone is send-only, all three accept_* alone is receive-only.
# [data]
# release                  = ["release/weather/**"]
# accept_hz                = 100
# accept_bytes_per_message = 1048576
# accept_distinct_keys     = 5000
"#;

fn run_init(ctx: &super::Ctx, a: InitArgs) -> anyhow::Result<()> {
    let mut outcomes = Vec::new();
    let mut underived: Vec<String> = Vec::new();

    let decl_path = ctx.profile.context_dir().join(derive::DECLARATION_FILE);
    // Captured here and reused below instead of re-reading the same file a
    // second time — the second read used `.unwrap_or_default()`, so on ANY
    // failure of that redundant read (a TOCTOU remove between the two reads,
    // an unusual FS state) `decl_text` silently became empty, which switched
    // off the "measured-only field claimed" refusal check entirely rather
    // than erroring — a security-relevant check disabled by a swallowed
    // error, not a deliberate skip.
    let raw_decl_text = std::fs::read_to_string(&decl_path);
    let decl = match &raw_decl_text {
        Ok(s) => match toml::from_str::<Declaration>(s) {
            Ok(d) => d,
            Err(e) => {
                outcomes.push(FedOutcome::fail(
                    "declaration",
                    format!("{}: {e}", decl_path.display()),
                    "Fix the file. It is not prompted for and its values are not guessed at.",
                ));
                finish_init(
                    &a,
                    Verdict::Fail,
                    vec!["declaration".into()],
                    None,
                    outcomes,
                );
                return bail_with(exit_codes::EXIT_FED_UNDERIVED);
            }
        },
        Err(_) => {
            let mut o = FedOutcome::fail(
                "declaration",
                format!("no {} at {}", derive::DECLARATION_FILE, decl_path.display()),
                "Write it, review it the way you would any other production configuration, and \
                 re-run. It holds only what no probe can produce — the organisation's name, the \
                 term, the cut contact, and the link parameters two parties agree rather than \
                 one observes. A template follows.",
            );
            o.block = Some(DECLARATION_TEMPLATE.to_string());
            outcomes.push(o);
            finish_init(
                &a,
                Verdict::Fail,
                vec!["declaration".into()],
                None,
                outcomes,
            );
            return bail_with(exit_codes::EXIT_FED_UNDERIVED);
        }
    };

    // A declaration that names a measured member is an error rather than an
    // override. This is the rule the whole verb turns on, so it fires before
    // anything is read.
    // Reuse the read from above (the `decl` match already proved this file
    // is present and parses) rather than re-reading and silently defaulting
    // to empty on any failure of that redundant read.
    let decl_text = raw_decl_text.unwrap_or_default();
    let claimed: Vec<&str> = derive::MEASURED_ONLY
        .iter()
        .copied()
        .filter(|m| {
            let leaf = m.rsplit('.').next().unwrap_or(m);
            decl_text.lines().any(|l| {
                let l = l.trim();
                !l.starts_with('#') && l.starts_with(leaf) && l.contains('=')
            })
        })
        .collect();
    if !claimed.is_empty() {
        outcomes.push(FedOutcome::fail(
            "measured-not-claimed",
            format!("{} names {claimed:?}", decl_path.display()),
            "These are measured from the running system, never read from configuration. A \
             capability typed by an operator is a claim; a capability read from the running \
             system is a fact. Remove them.",
        ));
        finish_init(
            &a,
            Verdict::Fail,
            claimed.iter().map(|s| s.to_string()).collect(),
            None,
            outcomes,
        );
        return bail_with(exit_codes::EXIT_FED_UNDERIVED);
    }
    outcomes.push(FedOutcome::ok(
        "declaration",
        format!(
            "{} — sovereign_id `{}`, {} day term, cut contact recorded",
            decl_path.display(),
            decl.sovereign_id,
            decl.term_days
        ),
    ));

    // --- identity, from our own credentials ---
    let td = derive::trust_domain(&ctx.profile.cert_path());
    let cafp = derive::ca_fingerprint(&ctx.profile.ca_roots_path(), &ctx.profile.cert_path());
    // --- capabilities, measured ---
    let transports = derive::transports();
    let (fips, version) = derive::fabric_capabilities();
    let pqc = derive::pqc_capable();

    let mut members: Vec<(String, JVal)> = Vec::new();
    members.push((
        "spec".into(),
        JVal::Str(goat_federation::package::PACKAGE_SPEC.into()),
    ));
    members.push(("sovereign_id".into(), JVal::Str(decl.sovereign_id.clone())));

    let today = chrono::Utc::now().date_naive();
    members.push(("issued".into(), JVal::Str(today.to_string())));
    let expires = today + chrono::Duration::days(decl.term_days);
    members.push(("expires".into(), JVal::Str(expires.to_string())));

    let td_v = take(&td, &mut outcomes, &mut underived, JVal::Str);
    let cafp_v = take(&cafp, &mut outcomes, &mut underived, JVal::Str);
    if let (Some(t), Some(c)) = (td_v, cafp_v) {
        members.push((
            "identity".into(),
            JVal::Obj(vec![
                ("ca_fingerprint".into(), c),
                ("trust_domain".into(), t),
            ]),
        ));
    }

    let tr_v = take(
        &transports,
        &mut outcomes,
        &mut underived,
        |v: Vec<String>| JVal::Arr(v.into_iter().map(JVal::Str).collect()),
    );
    let fips_v = take(&fips, &mut outcomes, &mut underived, JVal::Bool);
    let ver_v = take(&version, &mut outcomes, &mut underived, JVal::Str);
    let pqc_v = take(&pqc, &mut outcomes, &mut underived, JVal::Bool);
    if let (Some(tr), Some(fi), Some(ve), Some(pq)) = (tr_v, fips_v, ver_v, pqc_v) {
        members.push((
            "capabilities".into(),
            JVal::Obj(vec![
                ("fabric_fips_image".into(), fi),
                ("fabric_version".into(), ve),
                ("pqc_capable".into(), pq),
                ("transports".into(), tr),
            ]),
        ));
    }

    if a.with_transport {
        match &decl.transport {
            None => {
                outcomes.push(FedOutcome::fail(
                    "transport_offer",
                    format!(
                        "--with-transport, but {} has no [transport] section",
                        decl_path.display()
                    ),
                    "A transport offer needs the endpoint, the link subnet and a measured path \
                     MTU, none of which a probe of one host produces. Declare them, or drop \
                     --with-transport: a package with no transport offer is complete and valid.",
                ));
                underived.push("transport_offer".into());
            }
            Some(t) => {
                let pk = derive::transport_public_key(&t.interface);
                match derive::interface_mtu(&t.interface) {
                    Some(mtu) => {
                        if t.measured_mtu > mtu {
                            outcomes.push(FedOutcome::fail(
                                "transport_offer.measured_mtu",
                                format!(
                                    "declared {} exceeds interface `{}` MTU {mtu}",
                                    t.measured_mtu, t.interface
                                ),
                                "A path MTU cannot be larger than the interface carrying it, so this \
                                 number was not measured across the path. It goes into the agreement \
                                 as min(both sides), and a wrong one there is a silently broken link.",
                            ));
                            underived.push("transport_offer.measured_mtu".into());
                        }
                    }
                    // `interface_mtu` reads `/sys/class/net/<if>/mtu` — absent on
                    // any non-Linux host, or if the interface doesn't exist, or
                    // on a permission-denied read. This used to just skip the
                    // whole cross-check with no trace, which is indistinguishable
                    // from "checked and it passed" in the outcome list.
                    None => outcomes.push(FedOutcome::skip(
                        "transport_offer.measured_mtu",
                        format!(
                            "could not read the MTU of interface `{}` (non-Linux host, missing \
                             interface, or unreadable /sys) — the declared measured_mtu was NOT \
                             cross-checked against it",
                            t.interface
                        ),
                    )),
                }
                if let Some(k) = take(&pk, &mut outcomes, &mut underived, JVal::Str) {
                    members.push((
                        "transport_offer".into(),
                        JVal::Obj(vec![
                            ("endpoint".into(), JVal::Str(t.endpoint.clone())),
                            ("listener".into(), JVal::Bool(t.listener)),
                            ("measured_mtu".into(), JVal::Int(t.measured_mtu)),
                            (
                                "proposed_link_subnet".into(),
                                JVal::Str(t.proposed_link_subnet.clone()),
                            ),
                            ("public_key".into(), k),
                        ]),
                    ));
                }
            }
        }
    } else {
        outcomes.push(FedOutcome::skip(
            "transport_offer",
            "absent — this half is not offering to connect. Presence is the declaration, so \
             nothing needs to be claimed",
        ));
    }

    if a.with_data {
        match &decl.data {
            None => {
                outcomes.push(FedOutcome::fail(
                    "data_offer",
                    format!(
                        "--with-data, but {} has no [data] section",
                        decl_path.display()
                    ),
                    "A data offer needs a release set, or all three accept limits, or both. A \
                     limit that is not stated is a limit that does not exist, so none of them is \
                     defaulted.",
                ));
                underived.push("data_offer".into());
            }
            Some(d) => match build_data_offer(d) {
                Ok(v) => {
                    members.push(("data_offer".into(), v));
                    members.push((
                        "cut".into(),
                        JVal::Obj(vec![(
                            "contact".into(),
                            JVal::Str(decl.cut_contact.clone()),
                        )]),
                    ));
                    outcomes.push(FedOutcome::ok("data_offer", describe_declared_data(d)));
                }
                Err(why) => {
                    outcomes.push(FedOutcome::fail(
                        "data_offer",
                        why,
                        "Fix the [data] section.",
                    ));
                    underived.push("data_offer".into());
                }
            },
        }
    } else {
        outcomes.push(FedOutcome::skip(
            "data_offer",
            "absent — this half is not offering to exchange",
        ));
    }

    if !underived.is_empty() {
        outcomes.push(FedOutcome::fail(
            "package",
            format!(
                "{} value{} could not be derived",
                underived.len(),
                if underived.len() == 1 { "" } else { "s" }
            ),
            "Nothing is prompted for and nothing is filled in. Each line above says what is \
             missing and where it would have come from; make it true on this host, or run this \
             on the host where it is.",
        ));
        finish_init(&a, Verdict::Fail, underived, None, outcomes);
        return bail_with(exit_codes::EXIT_FED_UNDERIVED);
    }

    // Sign, or hand over the exact bytes to sign. The one thing not done here
    // is invent a key.
    let unsigned = JVal::Obj(members);
    let signing_input = canonical(&unsigned);
    match &a.signing_key {
        None => {
            let mut o = FedOutcome::fail(
                "signature",
                "no --signing-key, so the package is incomplete",
                "The exact bytes to sign with Ed25519 follow — the canonical form with no \
                 `signature` member, and no trailing newline. Put the base64 of the 64-byte \
                 result in a top-level `signature` member.",
            );
            o.block = Some(signing_input.clone());
            outcomes.push(o);
            finish_init(&a, Verdict::Fail, vec!["signature".into()], None, outcomes);
            bail_with(exit_codes::EXIT_FED_UNDERIVED)
        }
        Some(path) => {
            let sig = sign_with(path, signing_input.as_bytes())?;
            let mut signed = match unsigned {
                JVal::Obj(m) => m,
                _ => unreachable!("built as an object"),
            };
            signed.push(("signature".into(), JVal::Str(sig)));
            let pkg = JVal::Obj(signed);
            let c = canonical(&pkg);
            outcomes.push(FedOutcome::ok(
                "signature",
                format!(
                    "Ed25519 over the canonical form ex-signature, from {}",
                    path.display()
                ),
            ));
            outcomes.push(FedOutcome::ok(
                "package",
                format!(
                    "`{}`, issued {today}, expires {expires} — {} bytes of canonical form",
                    decl.sovereign_id,
                    c.len()
                ),
            ));
            if let Some(out) = &a.out {
                std::fs::write(out, c.as_bytes())
                    .map_err(|e| anyhow::anyhow!("{}: {e}", out.display()))?;
                outcomes.push(FedOutcome::ok(
                    "write",
                    format!("{} — canonical bytes, no trailing newline", out.display()),
                ));
            }
            finish_init(&a, Verdict::Ok, Vec::new(), Some(c), outcomes);
            Ok(())
        }
    }
}

fn build_data_offer(d: &DeclaredData) -> Result<JVal, String> {
    let mut m: Vec<(String, JVal)> = Vec::new();
    let accepts = [
        ("accept_bytes_per_message", d.accept_bytes_per_message),
        ("accept_distinct_keys", d.accept_distinct_keys),
        ("accept_hz", d.accept_hz),
    ];
    let present = accepts.iter().filter(|(_, v)| v.is_some()).count();
    if present != 0 && present != 3 {
        let missing: Vec<&str> = accepts
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        return Err(format!(
            "the receive half is all three accept values or none; [data] is missing {missing:?}. A \
             limit that is not stated is a limit that does not exist"
        ));
    }
    for (name, v) in accepts {
        if let Some(v) = v {
            m.push((name.into(), JVal::Int(v)));
        }
    }
    match &d.release {
        Some(r) if r.is_empty() => {
            return Err(
                "`release = []` is not a send half that carries nothing — an empty array \
                        is not a declaration. Remove the key, or list what you will publish"
                    .into(),
            )
        }
        Some(r) => m.push((
            "release".into(),
            JVal::Arr(
                goat_federation::json::normalise_set(r)
                    .into_iter()
                    .map(JVal::Str)
                    .collect(),
            ),
        )),
        None => {}
    }
    if m.is_empty() {
        return Err(
            "[data] declares neither half. Give it a release set, or all three accept \
                    limits, or both"
                .into(),
        );
    }
    Ok(JVal::Obj(m))
}

fn describe_declared_data(d: &DeclaredData) -> String {
    match (&d.release, d.accept_hz) {
        (Some(r), Some(_)) => format!("sends {r:?}, and accepts within the declared limits"),
        (Some(r), None) => format!("send-only: {r:?}"),
        (None, Some(_)) => "receive-only: the three accept limits, and nothing released".into(),
        (None, None) => "declares neither half".into(),
    }
}

/// Move a derivation's value into the package, or record why it could not be.
fn take<T, F>(
    d: &derive::Derived<T>,
    outcomes: &mut Vec<FedOutcome>,
    underived: &mut Vec<String>,
    f: F,
) -> Option<JVal>
where
    T: Clone,
    F: FnOnce(T) -> JVal,
{
    match &d.outcome {
        Ok(v) => {
            outcomes.push(FedOutcome::ok(d.field, format!("read from {}", d.source)));
            Some(f(v.clone()))
        }
        Err(why) => {
            outcomes.push(FedOutcome::fail(
                d.field,
                format!("cannot derive from {}", d.source),
                why.clone(),
            ));
            underived.push(d.field.to_string());
            None
        }
    }
}

fn finish_init(
    a: &InitArgs,
    verdict: Verdict,
    underived: Vec<String>,
    package: Option<String>,
    outcomes: Vec<FedOutcome>,
) {
    if a.json {
        emit_json(&InitReport {
            verb: "init",
            verdict,
            underived,
            package,
            outcomes,
        });
    } else {
        println!("init — our half, read from the running system");
        for o in &outcomes {
            print_outcome(o);
        }
    }
}

/// The RFC 8032 seed (64 hex characters) or the same 32 bytes base64.
fn sign_with(path: &Path, message: &[u8]) -> anyhow::Result<String> {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};

    let text =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let text = text.trim();
    let seed: [u8; 32] = if text.len() == 64 && text.bytes().all(|c| c.is_ascii_hexdigit()) {
        let mut b = [0u8; 32];
        for i in 0..32 {
            b[i] = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).expect("hex checked");
        }
        b
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: expected an Ed25519 seed as 64 hex characters or 32 bytes base64",
                    path.display()
                )
            })?
    };
    let sk = SigningKey::from_bytes(&seed);
    Ok(base64::engine::general_purpose::STANDARD.encode(sk.sign(message).to_bytes()))
}

// ---------------------------------------------------------------------------
// verify
// ---------------------------------------------------------------------------

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

fn run_verify(ctx: &super::Ctx, a: VerifyArgs) -> anyhow::Result<()> {
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
            finish_verify(&a, Verdict::Fail, &digest, None, None, outcomes);
            return bail_with(exit_codes::EXIT_FED_NOT_CONFORMANT);
        }
    };
    outcomes.push(FedOutcome::ok(
        "readable",
        format!("{digest} — the digest over these exact bytes"),
    ));

    let ours = match resolve_party(ctx, a.as_party.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            outcomes.push(FedOutcome::fail("party", e, "Pass --as <sovereign_id>."));
            finish_verify(&a, Verdict::Fail, &digest, None, None, outcomes);
            return bail_with(exit_codes::EXIT_FED_NOT_CONFORMANT);
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
        finish_verify(&a, Verdict::Fail, &digest, None, None, outcomes);
        return bail_with(exit_codes::EXIT_FED_NOT_CONFORMANT);
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
        &a,
        Verdict::Ok,
        &digest,
        Some(&line),
        sig.as_deref(),
        outcomes,
    );
    Ok(())
}

fn finish_verify(
    a: &VerifyArgs,
    verdict: Verdict,
    digest: &str,
    line: Option<&str>,
    sig: Option<&str>,
    outcomes: Vec<FedOutcome>,
) {
    if a.json {
        emit_json(&VerifyReport {
            verb: "verify",
            verdict,
            digest: digest.to_string(),
            attestation: line.map(str::to_string),
            attestation_signature: sig.map(str::to_string),
            outcomes,
        });
    } else {
        println!("verify {}", a.agreement.display());
        for o in &outcomes {
            print_outcome(o);
        }
        if let (Some(l), Some(s)) = (line, sig) {
            println!("\n{l}\nsignature: {s}");
        }
    }
}

fn resolve_party(ctx: &super::Ctx, explicit: Option<&str>) -> Result<String, String> {
    if let Some(s) = explicit {
        return Ok(s.to_string());
    }
    let path = ctx.profile.context_dir().join(derive::DECLARATION_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("no --as and no {} to read it from", path.display()))?;
    let d: Declaration = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(d.sovereign_id)
}

// ---------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct ApplyArgs {
    /// The composed agreement.
    pub agreement: PathBuf,
    /// Which party we are. Read from the declaration file when omitted.
    #[arg(long = "as", value_name = "SOVEREIGN_ID")]
    pub as_party: Option<String>,
    /// Where to write. Files land under this directory, mirroring the repo
    /// paths they belong at.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// The ansible group the edge runs in. The generated group_vars land at
    /// `group_vars/<GROUP>/`, and `--limit <GROUP>` is what deploys them.
    ///
    /// Omitted, the path carries the literal `EDGE_HOST` for the operator to
    /// rename — which is a typed value in a flow whose whole purpose is that
    /// nothing is typed twice. Pass it.
    #[arg(long = "edge-group", value_name = "GROUP")]
    pub edge_group: Option<String>,
    /// Release-sets this partner holds an audience slot in, so the edge grants it
    /// a READ of `keys/<set>/**` and it can obtain a data key (ADR 1169
    /// decision 3). Repeatable.
    ///
    /// Omitted, no key rule is rendered and exchange is unsealed — which is the
    /// documented default, not a gap. Audience membership is ours to decide and
    /// is not carried in the agreement, so it is named here rather than derived.
    #[arg(long = "partner-audience", value_name = "RELEASE_SET")]
    pub partner_audience: Vec<String>,
    /// Write. Without this, the rendering is printed and nothing is touched.
    #[arg(long)]
    pub confirm: bool,
    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct ApplyReport {
    verb: &'static str,
    verdict: Verdict,
    digest: String,
    wrote: bool,
    files: Vec<String>,
    outcomes: Vec<FedOutcome>,
}

fn run_apply(ctx: &super::Ctx, a: ApplyArgs) -> anyhow::Result<()> {
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
            finish_apply(&a, Verdict::Fail, &digest, false, Vec::new(), outcomes);
            return bail_with(exit_codes::EXIT_FED_NOT_CONFORMANT);
        }
    };
    let ours = resolve_party(ctx, a.as_party.as_deref()).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Render only what we could actually deliver. Writing access rules from an
    // agreement that fails its own conformance check is how a link comes up
    // half-configured and looks deployed.
    let today = chrono::Utc::now().date_naive();
    let checks = render::verify_locally(&ag, &ours, today);
    if let Some(bad) = checks.iter().find(|c| !c.ok) {
        outcomes.push(FedOutcome::fail(
            bad.name,
            bad.detail.clone(),
            bad.hint.clone(),
        ));
        outcomes.push(FedOutcome::fail(
            "render",
            "nothing rendered",
            "Run `goat federate verify` and clear the failing check first. Rendering from an \
             agreement we cannot deliver produces configuration that looks applied and is not.",
        ));
        finish_apply(&a, Verdict::Fail, &digest, false, Vec::new(), outcomes);
        return bail_with(exit_codes::EXIT_FED_NOT_CONFORMANT);
    }

    let artifacts = render::render_all(
        &ag,
        &ours,
        &digest,
        a.edge_group.as_deref(),
        &a.partner_audience,
    );
    let mut files = Vec::new();
    for r in &artifacts {
        outcomes.push(FedOutcome::ok(r.path.clone(), r.note.clone()));
        files.push(r.path.clone());
    }

    let wrote = if a.confirm {
        let dir = a.out.clone().ok_or_else(|| {
            anyhow::anyhow!("--confirm needs --out <dir>: this verb will not choose where to write")
        })?;
        for r in &artifacts {
            // A parenthesised path is a statement, not a file: the trust anchor this
            // tool has no copy of, or the note that an agreement renders no partner
            // declaration. Writing one would leave a file named "(none) partner
            // declaration" in the output.
            if r.path.starts_with('(') {
                continue;
            }
            let target = dir.join(r.path.replace(['<', '>'], "_"));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, &r.body)
                .map_err(|e| anyhow::anyhow!("{}: {e}", target.display()))?;
        }
        let written = artifacts
            .iter()
            .filter(|r| !r.path.starts_with('('))
            .count();
        outcomes.push(FedOutcome::ok(
            "write",
            format!(
                "{written} file{} under {}",
                if written == 1 { "" } else { "s" },
                dir.display()
            ),
        ));
        true
    } else {
        outcomes.push(FedOutcome::skip(
            "write",
            "nothing written. Pass --confirm --out <dir> to write; this is the only verb here \
             that touches anything",
        ));
        false
    };

    finish_apply(&a, Verdict::Ok, &digest, wrote, files, outcomes);
    if !a.json && !wrote {
        for r in &artifacts {
            println!("\n--- {} ---\n{}", r.path, r.body);
        }
    }
    Ok(())
}

fn finish_apply(
    a: &ApplyArgs,
    verdict: Verdict,
    digest: &str,
    wrote: bool,
    files: Vec<String>,
    outcomes: Vec<FedOutcome>,
) {
    if a.json {
        emit_json(&ApplyReport {
            verb: "apply",
            verdict,
            digest: digest.to_string(),
            wrote,
            files,
            outcomes,
        });
    } else {
        println!("apply {} ({digest})", a.agreement.display());
        for o in &outcomes {
            print_outcome(o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_every_word() {
        let s = "one two three four five six seven eight nine ten";
        let joined = wrap(s, 12).join(" ");
        assert_eq!(joined, s);
    }

    fn declared(
        release: Option<Vec<&str>>,
        hz: Option<i64>,
        bpm: Option<i64>,
        keys: Option<i64>,
    ) -> DeclaredData {
        DeclaredData {
            release: release.map(|r| r.into_iter().map(str::to_string).collect()),
            accept_hz: hz,
            accept_bytes_per_message: bpm,
            accept_distinct_keys: keys,
        }
    }

    /// A limit that is not stated is a limit that does not exist, so a
    /// half-stated receive half is refused rather than completed.
    #[test]
    fn a_partial_receive_half_is_refused() {
        let err = build_data_offer(&declared(None, Some(100), Some(1024), None)).unwrap_err();
        assert!(err.contains("accept_distinct_keys"), "{err}");
    }

    /// One-way federation is a real case, not a degenerate one.
    #[test]
    fn each_half_is_complete_on_its_own() {
        let send = build_data_offer(&declared(
            Some(vec!["release/weather/**"]),
            None,
            None,
            None,
        ))
        .unwrap();
        assert!(send.get("release").is_some());
        assert!(send.get("accept_hz").is_none());

        let recv = build_data_offer(&declared(None, Some(100), Some(1024), Some(5))).unwrap();
        assert!(recv.get("release").is_none());
        assert!(recv.get("accept_hz").is_some());
    }

    /// An empty array is not a declaration — not a send half that happens to
    /// carry nothing.
    #[test]
    fn an_empty_release_is_refused_rather_than_read_as_send_only() {
        let err = build_data_offer(&declared(Some(vec![]), None, None, None)).unwrap_err();
        assert!(err.contains("not a send half"), "{err}");
    }

    #[test]
    fn a_data_offer_with_neither_half_is_refused() {
        let err = build_data_offer(&declared(None, None, None, None)).unwrap_err();
        assert!(err.contains("neither half"), "{err}");
    }

    /// The declared release set is normalised on the way in, so a package this
    /// tool signs never carries the unsorted form the signer is obliged to
    /// avoid.
    #[test]
    fn a_declared_release_set_is_normalised_before_signing() {
        let v = build_data_offer(&declared(
            Some(vec!["b/**", "a/**", "b/**"]),
            None,
            None,
            None,
        ))
        .unwrap();
        let items: Vec<&str> = v
            .get("release")
            .and_then(JVal::as_arr)
            .unwrap()
            .iter()
            .filter_map(JVal::as_str)
            .collect();
        assert_eq!(items, vec!["a/**", "b/**"]);
    }

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

// The federation verbs: init, lint, compose, diff, verify and apply.
//
// docs/design/federation-package-spec.md is the authority; this is one
// implementation of it, held to the byte-exact conformance vectors in
// ops/federation/vectors/ (which are normative: if this and a vector
// disagree, this is wrong).
//
// Every front end (`goat federate`, `goatbuilder federate`) runs these same
// functions with the same clap arguments, so the flags, the output and the
// exit codes cannot differ between them. A front end supplies only what this
// library cannot know: where its declaration, certificate and CA roots are
// (`Site`), and where the report goes (`out`).
//
// PRIVILEGE, and why it is arranged this way rather than merely documented.
// A partner has to be able to trust that handing us their package cannot
// touch anything, and "we promise" is not a property anyone can check. So:
//
//   lint / compose / diff   PURE. `crate::{json,package,compose,diff}` read no
//                           file the operator did not name, open no socket,
//                           hold no credential and do not read the clock —
//                           `now` and the verifying keys arrive as arguments.
//                           They run in an empty container.
//   init / verify           read local state. Never write.
//   apply                   the only verb that writes, and only under
//                           --confirm.
//
// The reporting shape mirrors `goat doctor`: one structured outcome per
// check, a verdict, and a hint on failure. There is one such convention and
// this is it.

use clap::Subcommand;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

mod apply;
mod compose;
mod diff;
mod init;
mod lint;
mod verify;

pub use apply::{run_apply, ApplyArgs};
pub use compose::{run_compose, ComposeArgs};
pub use diff::{run_diff, DiffArgs};
pub use init::{measure_here, measure_here_with, run_init, run_init_measured, InitArgs};
pub use lint::{run_lint, LintArgs};
pub use verify::{run_verify, VerifyArgs};

// Exit codes. Four of these are OUTCOMES rather than faults, and they carry
// distinct codes so a wrapper can tell them apart without parsing prose: a
// composition refusal is the mechanism working, and a malformed package is a
// different finding from a pair that refuses. Conflating them is the specific
// way this kind of tooling goes quietly wrong.
/// The verb succeeded.
pub const EXIT_OK: i32 = 0;
/// Composition refused — one of §6's seven rules fired. No agreement, no digest.
pub const EXIT_FED_REFUSED: i32 = 20;
/// One package is malformed by §3/§4 — refused before composition begins.
pub const EXIT_FED_MALFORMED: i32 = 21;
/// Two agreements disagree, by field or by bytes.
pub const EXIT_FED_MISMATCH: i32 = 22;
/// A value could not be derived from the running system, and is never guessed.
pub const EXIT_FED_UNDERIVED: i32 = 23;
/// The agreement would not be valid on our own systems — nothing attested.
pub const EXIT_FED_NOT_CONFORMANT: i32 = 24;

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

impl FederateCmd {
    /// The verb as it is typed: `init`, `lint`, `compose`, `diff`, `verify`, `apply`.
    pub fn name(&self) -> &'static str {
        match self {
            FederateCmd::Init(_) => "init",
            FederateCmd::Lint(_) => "lint",
            FederateCmd::Compose(_) => "compose",
            FederateCmd::Diff(_) => "diff",
            FederateCmd::Verify(_) => "verify",
            FederateCmd::Apply(_) => "apply",
        }
    }
}

/// The six verb names. A front end that also takes a partner name in the same
/// position refuses these as partner names.
pub const VERB_NAMES: &[&str] = &["init", "lint", "compose", "diff", "verify", "apply"];

/// Where this party's own local state is, which only the front end knows:
/// `goat` reads it from its profile's active context, `goatbuilder` from its
/// flags. `init` reads all three; `verify` and `apply` read the declaration
/// only when `--as` is not given. lint, compose and diff read none of it.
#[derive(Debug, Clone)]
pub struct Site {
    /// The directory holding the declaration, [`crate::derive::DECLARATION_FILE`].
    pub context_dir: PathBuf,
    /// Our mTLS certificate (PEM): the trust domain is read from it.
    pub cert_path: PathBuf,
    /// The CA roots (PEM): the CA fingerprint is the root that signed `cert_path`.
    pub ca_roots_path: PathBuf,
}

impl Site {
    /// The declaration file inside the context directory.
    pub fn declaration_path(&self) -> PathBuf {
        self.context_dir.join(crate::derive::DECLARATION_FILE)
    }
}

/// Run one verb. `site` is called only by a verb that reads local state, so a
/// pure verb runs where no site can be resolved. Returns the exit code
/// ([`EXIT_OK`] or one of the `EXIT_FED_*` outcomes) once the verb has written
/// its report to `out`; an `Err` is a fault the report could not describe (an
/// unreadable file, an unwritable output), and the caller prints it.
pub fn run(
    cmd: FederateCmd,
    site: &dyn Fn() -> anyhow::Result<Site>,
    out: &mut dyn Write,
) -> anyhow::Result<i32> {
    match cmd {
        FederateCmd::Init(a) => run_init(&site()?, &a, out),
        FederateCmd::Lint(a) => run_lint(&a, out),
        FederateCmd::Compose(a) => run_compose(&a, out),
        FederateCmd::Diff(a) => run_diff(&a, out),
        FederateCmd::Verify(a) => run_verify(site, &a, out),
        FederateCmd::Apply(a) => run_apply(site, &a, out),
    }
}

// ---------------------------------------------------------------------------
// Shared reporting shape
// ---------------------------------------------------------------------------

/// One check's verdict, serialised the way `goat doctor` serialises its own.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Skip,
    Fail,
}

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
    pub(crate) fn ok(check: impl Into<String>, summary: impl Into<String>) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Ok,
            summary: summary.into(),
            hint: String::new(),
            warn: false,
            block: None,
        }
    }
    pub(crate) fn fail(
        check: impl Into<String>,
        summary: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        FedOutcome {
            check: check.into(),
            verdict: Verdict::Fail,
            summary: summary.into(),
            hint: hint.into(),
            warn: false,
            block: None,
        }
    }
    pub(crate) fn skip(check: impl Into<String>, summary: impl Into<String>) -> Self {
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
    pub(crate) fn warn(
        check: impl Into<String>,
        summary: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
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

pub(crate) fn print_outcome(out: &mut dyn Write, o: &FedOutcome) -> std::io::Result<()> {
    writeln!(out, "{} {} — {}", glyph(o), o.check, o.summary)?;
    if !o.hint.is_empty() {
        for line in wrap(&o.hint, 88) {
            writeln!(out, "     {line}")?;
        }
    }
    if let Some(b) = &o.block {
        writeln!(out)?;
        writeln!(out, "{b}")?;
    }
    Ok(())
}

pub(crate) fn print_outcomes(out: &mut dyn Write, outcomes: &[FedOutcome]) -> std::io::Result<()> {
    for o in outcomes {
        print_outcome(out, o)?;
    }
    Ok(())
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

pub(crate) fn emit_json<T: Serialize>(out: &mut dyn Write, v: &T) -> std::io::Result<()> {
    match serde_json::to_string(v) {
        Ok(s) => writeln!(out, "{s}"),
        Err(e) => {
            eprintln!("error: could not serialise the report: {e}");
            Ok(())
        }
    }
}

pub(crate) fn read_file(p: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))
}

pub(crate) fn label(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The RFC 8032 seed (64 hex characters) or the same 32 bytes base64.
pub(crate) fn sign_with(path: &Path, message: &[u8]) -> anyhow::Result<String> {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};

    // This key also signs acceptance results, under their own domain line
    // (acceptance_sig.rs). A package's signing input is a JSON object and the
    // attestation line begins `conformance`, so neither begins with it; this
    // signer refuses the domain outright, so no caller can make it sign a
    // result statement.
    if message.starts_with(crate::acceptance_sig::RESULT_DOMAIN.as_bytes()) {
        anyhow::bail!("refusing to sign an acceptance-result statement with the package signer");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_every_word() {
        let s = "one two three four five six seven eight nine ten";
        let joined = wrap(s, 12).join(" ");
        assert_eq!(joined, s);
    }

    /// Every verb's name is in VERB_NAMES, so a front end that refuses them as
    /// partner names refuses all six.
    #[test]
    fn verb_names_cover_every_verb() {
        use clap::CommandFactory as _;
        #[derive(clap::Parser)]
        struct Cli {
            #[command(subcommand)]
            cmd: FederateCmd,
        }
        let names: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        assert_eq!(names, VERB_NAMES);
    }
}

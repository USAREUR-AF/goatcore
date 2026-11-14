// lint: one package, well-formed and self-consistent. Pure.

use super::{
    emit_json, print_outcomes, read_file, FedOutcome, Verdict, EXIT_FED_MALFORMED, EXIT_OK,
};
use crate::package::{parse_package, warnings, Package};
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

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

pub fn run_lint(a: &LintArgs, out: &mut dyn Write) -> anyhow::Result<i32> {
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
        emit_json(
            out,
            &LintReport {
                verb: "lint",
                file: a.package.display().to_string(),
                verdict,
                rule,
                outcomes,
            },
        )?;
    } else {
        writeln!(out, "package {}", a.package.display())?;
        print_outcomes(out, &outcomes)?;
    }
    if verdict == Verdict::Fail {
        return Ok(EXIT_FED_MALFORMED);
    }
    Ok(EXIT_OK)
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

// diff: two agreements whose digests disagree, and the field or byte that
// differs. Pure.

use super::{
    emit_json, print_outcomes, read_file, FedOutcome, Verdict, EXIT_FED_MISMATCH, EXIT_OK,
};
use crate::diff::{diff, ByteCause, DiffOutcome, Side};
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

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

pub fn run_diff(a: &DiffArgs, out: &mut dyn Write) -> anyhow::Result<i32> {
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
        emit_json(
            out,
            &DiffReport {
                verb: "diff",
                verdict,
                ours: a.ours.display().to_string(),
                theirs: a.theirs.display().to_string(),
                kind,
                digest,
                fields,
                outcomes,
            },
        )?;
    } else {
        writeln!(
            out,
            "diff\n  ours:   {}\n  theirs: {}",
            a.ours.display(),
            a.theirs.display()
        )?;
        print_outcomes(out, &outcomes)?;
    }
    if verdict == Verdict::Fail {
        return Ok(EXIT_FED_MISMATCH);
    }
    Ok(EXIT_OK)
}

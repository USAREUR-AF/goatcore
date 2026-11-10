// apply: render the access rules and configuration the agreement implies.
// The only verb that writes, and only under --confirm.

use super::verify::resolve_party;
use super::{
    emit_json, print_outcomes, read_file, FedOutcome, Site, Verdict, EXIT_FED_NOT_CONFORMANT,
    EXIT_OK,
};
use crate::compose::digest_of;
use crate::render;
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

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

pub fn run_apply(
    site: &dyn Fn() -> anyhow::Result<Site>,
    a: &ApplyArgs,
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
            finish_apply(out, a, Verdict::Fail, &digest, false, Vec::new(), outcomes)?;
            return Ok(EXIT_FED_NOT_CONFORMANT);
        }
    };
    let ours = resolve_party(site, a.as_party.as_deref()).map_err(|e| anyhow::anyhow!("{e}"))?;

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
        finish_apply(out, a, Verdict::Fail, &digest, false, Vec::new(), outcomes)?;
        return Ok(EXIT_FED_NOT_CONFORMANT);
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

    finish_apply(out, a, Verdict::Ok, &digest, wrote, files, outcomes)?;
    if !a.json && !wrote {
        for r in &artifacts {
            writeln!(out, "\n--- {} ---\n{}", r.path, r.body)?;
        }
    }
    Ok(EXIT_OK)
}

fn finish_apply(
    out: &mut dyn Write,
    a: &ApplyArgs,
    verdict: Verdict,
    digest: &str,
    wrote: bool,
    files: Vec<String>,
    outcomes: Vec<FedOutcome>,
) -> std::io::Result<()> {
    if a.json {
        emit_json(
            out,
            &ApplyReport {
                verb: "apply",
                verdict,
                digest: digest.to_string(),
                wrote,
                files,
                outcomes,
            },
        )
    } else {
        writeln!(out, "apply {} ({digest})", a.agreement.display())?;
        print_outcomes(out, &outcomes)
    }
}

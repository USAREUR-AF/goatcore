//! `goatbuilder federate rotate <partner> <key>`: replace one key of a running
//! peering and run the command again from the first phase it changes
//! (docs/design/federation-key-rotation.md §5).
//!
//! Built: `leaves`, the peering leaves (K3). Phase 8 runs the leaf helper with
//! `--rotate` (`ops/federation/standup/peering_leaf_material.py`). It mints
//! and validates the new pairs beside a copy of the old ones (mode 0600) and
//! commits the rotation by renaming that directory to
//! `<work>/pki/.rotation-pending`. It then installs the new pairs and removes
//! the directory, old pairs included. A run killed after the commit is
//! finished by the next one from the pairs already minted, or rolled back to
//! the old pairs; it never mints a third. The run is phases 8 to 14. Phase 10 stops, as in every run, for the review of what apply rendered,
//! now with the new leaves, and phase 12 installs them. Once phase 8 has
//! passed, the command that carries on is the plain resume
//! (`goatbuilder federate <partner> --from 11`), so following a stop or a
//! failure never mints another set. Nothing the partner holds changes, so
//! no `rotation.json` is written: phase 3 and our package are untouched.
//!
//! Built: `carrier`, the forwarder carrier keys (K5): phase 2 passes the
//! rotation's id to the plays it runs, and goat-peering-wg and
//! goat-peering-forwarder each move the carrier's key in place, once, to
//! `.previous-<id>/` and make a new one (their `tasks/carrier-key.yml`). The id
//! is kept in `<work>/.rotation-carrier` from the start of the rotation until
//! phase 2 passes, so a resumed rotation passes the same id and sets nothing
//! aside twice. The keys are exchanged within phase 2's own forwarder sequence
//! (forwarder.rs), so nothing the partner holds changes, and no `rotation.json`
//! is written. The run is phases 2 to 14.
//!
//! Built: `transport`, the peering's WireGuard key (K4), as a library. Before
//! anything runs, the rotation's id is kept in `<work>/.rotation-transport`
//! and written as the nonce of `<work>/rotation.json`, once (§7 item 1c).
//! Phase 2 passes the id to goat-peering-wg, which moves the key aside, once,
//! to `.previous-<id>/`, makes a new one and restarts the link onto it (its
//! `tasks/transport-key.yml` and `tasks/transport-live.yml`). Phase 3 hashes
//! `rotation.json`, so it measures the new key into a new package, and phase
//! 5 offers that package to the partner. The run is phases 2 to 14. The
//! partner receiving it is designed, not built (§3 K4, ADR 1229), so the
//! command refuses `transport` (below) and only the library runs it.
//!
//! Built: `ipsec-leaf` and `ipsec-ca`, the IPsec credentials of an
//! `ipsec-fips` peering (K6, §8). The run is phases 11 to 14. Phase 11's
//! `peering-edge.yml` gets the rotation's id and kind (and, for `ipsec-ca`, the
//! partner's read-back of our new CA, `--partner-holds-ipsec-ca`, GB-303), and
//! goat-peering-edge checks the leaf an operator staged, sets the live pair
//! aside once, switches, re-authenticates and rolls back (its
//! `tasks/rotate.yml`). The rotation's record, `<work>/.rotation-ipsec`, is
//! written before phase 11 and removed once it passed; a resume of another kind
//! or read-back is refused (GB-298). `--retire <id>` passes the retire step,
//! which removes the set-aside key. A peering whose agreement chose another
//! transport has no IPsec leaf (GB-189).
//!
//! Refused: `transport` and `signing` with GB-189, each naming what is not
//! built yet; `transport` also while the partner cannot receive our new
//! package (§5, ADR 1229), because the tunnel would stay down after our new
//! key. `ca` with GB-282: our CA's rotation needs an ADR superseding the
//! package specification's single CA fingerprint (§4; package-spec §5.1).

use std::path::Path;

use anyhow::Result;

use super::run::Surface;

/// The stage word: `goatbuilder federate rotate <partner> <key>`. A peering
/// can never be named this (config.rs RESERVED_PEERING_NAMES).
pub const VERB: &str = "rotate";
use super::{Options, Phase};

/// `goatbuilder federate rotate <partner> <key>`.
#[derive(clap::Args, Debug)]
pub struct RotateArgs {
    /// The partner whose peering's key is replaced (the peering name, as declared).
    pub partner: String,
    /// The key: leaves, carrier, ipsec-leaf, ipsec-ca, transport, signing or ca. This build rotates leaves, carrier, ipsec-leaf and ipsec-ca.
    pub key: String,
    /// ipsec-ca only: the sha256 fingerprint of our new IPsec CA, as the partner
    /// read it back after placing it beside their current one.
    #[arg(long = "partner-holds-ipsec-ca", value_name = "SHA256")]
    pub partner_holds_ipsec_ca: Option<String>,
    /// ipsec-leaf or ipsec-ca: run the retire step of this rotation id, which
    /// removes the set-aside key once both sides passed phase 14.
    #[arg(long, value_name = "ID")]
    pub retire: Option<String>,
    /// The environment the peering is declared on.
    #[arg(long)]
    pub env: Option<String>,
    /// Which party we are.
    #[arg(long = "as", value_name = "SOVEREIGN")]
    pub as_party: Option<String>,
    /// The ansible group the edge runs in (default <partner>_edge).
    #[arg(long)]
    pub edge_group: Option<String>,
    /// The working directory (default $FED_WORK, else /var/lib/goat-federation/<partner>).
    #[arg(long)]
    pub work: Option<String>,
    /// Start at this phase instead of the key's first; a start after that
    /// phase replaces nothing.
    #[arg(long, default_value_t = 1)]
    pub from: u32,
    /// Print the phases and commands; change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

impl RotateArgs {
    pub fn options<'a>(&'a self, mock: Option<&'a Path>, table: bool) -> Options<'a> {
        Options {
            with: &self.partner,
            env: self.env.as_deref(),
            r#as: self.as_party.as_deref(),
            edge_group: self.edge_group.as_deref(),
            work: self.work.as_deref(),
            mock,
            from: self.from,
            to: 99,
            dry_run: self.dry_run,
            table,
        }
    }
}

/// Whether this build rotates a key, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gate {
    Built,
    /// Refused with GB-189, with the reason.
    NotBuilt(&'static str),
    /// Refused with GB-282: a decision is needed first, not only code.
    NeedsAdr,
}

/// The keys `rotate` names, and whether this build rotates each. A key moves
/// to `Built` only together with what makes its rotation safe end to end: a
/// test below holds `transport` refused while the partner cannot receive our
/// new package.
pub const KEYS: &[(&str, Gate)] = &[
    ("leaves", Gate::Built),
    ("carrier", Gate::Built),
    ("ipsec-leaf", Gate::Built),
    ("ipsec-ca", Gate::Built),
    (
        "transport",
        Gate::NotBuilt(
            "the partner cannot yet receive our new package: under our unchanged signing key their phase 5 keeps the package on disk (§5, ADR 1229), so after our new WireGuard key the tunnel would stay down",
        ),
    ),
    (
        "signing",
        Gate::NotBuilt(
            "the stage that keeps the previous seed for the directory hand-over, records the rotation and runs from phase 3 is not built, and the partner has to bind our new key on the call (confirmation 1)",
        ),
    ),
    ("ca", Gate::NeedsAdr),
];

/// A rotation of keys phase 2 makes, under way: its id, and the file that
/// keeps it until phase 2 passes.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub id: String,
    pub file: std::path::PathBuf,
}

/// The file that keeps a carrier rotation's id in the working directory.
pub const CARRIER_FILE: &str = ".rotation-carrier";

/// The file that keeps a peering key rotation's id in the working directory.
pub const TRANSPORT_FILE: &str = ".rotation-transport";

/// The rotation record phase 3 reads (`run.rs`, `Paths::rotation`).
pub const RECORD_FILE: &str = "rotation.json";

/// The id of the carrier rotation under way in `work`, or a new one, written
/// to `<work>/.rotation-carrier` before anything runs.
pub fn carrier_rotation(work: &Path) -> Result<Pending> {
    pending(work, CARRIER_FILE, "carrier")
}

/// The id of the peering key rotation under way in `work`, or a new one,
/// written to `<work>/.rotation-transport` before anything runs; then
/// `<work>/rotation.json` names it, so phase 3 measures a new package. The
/// record is written only when it does not already name this rotation, so a
/// resumed rotation leaves its bytes, and phase 3's digest, as they were.
pub fn transport_rotation(work: &Path) -> Result<Pending> {
    let t = pending(work, TRANSPORT_FILE, "transport")?;
    let record = work.join(RECORD_FILE);
    let want = format!("{{\"key\":\"transport\",\"nonce\":\"{}\"}}\n", t.id);
    if std::fs::read_to_string(&record).ok().as_deref() != Some(want.as_str()) {
        std::fs::write(&record, want)?;
    }
    Ok(t)
}

/// The id kept in `<work>/<name>`, or a new `<prefix>-<UTC time>` written
/// there.
fn pending(work: &Path, name: &str, prefix: &str) -> Result<Pending> {
    let file = work.join(name);
    let kept = std::fs::read_to_string(&file)
        .ok()
        .map(|s| s.trim().to_string());
    let id = match kept.filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => {
            let id = format!("{prefix}-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
            std::fs::create_dir_all(work)?;
            std::fs::write(&file, format!("{id}\n"))?;
            id
        }
    };
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow::anyhow!(
            "{} holds '{id}', which is not a rotation id; remove the file to start the rotation again",
            file.display()
        ));
    }
    Ok(Pending { id, file })
}

/// `vars` before every `-e fed_work=` of phase 2's plays.
fn phase_2_vars(phases: &mut [Phase], vars: &str) {
    for p in phases.iter_mut().filter(|p| p.n == 2) {
        p.cmd = p
            .cmd
            .replace(" -e fed_work='", &format!("{vars} -e fed_work='"));
    }
}

/// Phase 2 passes the carrier rotation's id to every play it runs, before
/// each play's `-e fed_work=`: goat-peering-wg's for the edge's carrier, and
/// goat-peering-forwarder's, with the peering's entry name, for the
/// forwarder's. A phase 2 without a forwarder has no carrier, and the edge's
/// role ignores the id when it has no carrier.
pub(super) fn carrier(phases: &mut [Phase], c: &Pending, partner: &str) {
    let vars = format!(
        " -e peering_wg_carrier_rotation='{id}' -e peering_forwarder_carrier_rotation='{id}' -e peering_forwarder_carrier_rotation_name='{partner}'",
        id = c.id
    );
    phase_2_vars(phases, &vars);
}

/// Phase 2 passes the peering key rotation's id to every play it runs, before
/// each play's `-e fed_work=`. Only goat-peering-wg reads it; an edge whose
/// transport is not WireGuard has no such key and ignores it.
pub(super) fn transport(phases: &mut [Phase], t: &Pending) {
    phase_2_vars(
        phases,
        &format!(" -e peering_wg_transport_rotation='{}'", t.id),
    );
}

// `rotate <partner> transport` is refused until the partner can receive our
// new package (KEYS); this run is reached from the tests only.
#[cfg_attr(not(test), allow(dead_code))]
/// The run that rotates the peering key: phases 2 to 14, nothing skipped. A
/// start after phase 2 is kept and rotates nothing, and writes no record.
pub fn transport_run<'a>(o: &Options<'a>, work: &Path) -> Result<(Options<'a>, Surface)> {
    let from = o.from.max(2);
    let s = Surface {
        rotate_transport: if from <= 2 {
            Some(transport_rotation(work)?)
        } else {
            None
        },
        resume: false,
        ..Default::default()
    };
    Ok((Options { from, to: 14, ..*o }, s))
}

/// The run that rotates the carrier keys: phases 2 to 14, nothing skipped,
/// recorded as a `rotate` run. A start after phase 2 is kept and rotates
/// nothing: phase 2 is not in that run. Until phase 2 passes, the command
/// that carries on is `rotate <partner> carrier` again, which reads the same
/// id and so is the resume.
pub fn carrier_run<'a>(o: &Options<'a>, work: &Path) -> Result<(Options<'a>, Surface)> {
    let from = o.from.max(2);
    let s = Surface {
        stage: "rotate",
        again: (from <= 2).then(|| format!("goatbuilder federate {VERB} {} carrier", o.with)),
        rotate_carrier: if from <= 2 {
            Some(carrier_rotation(work)?)
        } else {
            None
        },
        resume: false,
        ..Default::default()
    };
    Ok((Options { from, to: 14, ..*o }, s))
}

/// The file that keeps an IPsec rotation's record until phase 11 passes.
pub const IPSEC_FILE: &str = ".rotation-ipsec";

/// An IPsec credential rotation (K6) for phase 11: its id, its kind (`leaf` or
/// `ca`), the partner's read-back of our new CA for `ca`, whether this run is
/// the retire step, and the record that keeps it (none for a retire).
#[derive(Debug, Clone, PartialEq)]
pub struct Ipsec {
    pub id: String,
    pub kind: &'static str,
    pub readback: Option<String>,
    pub retire: bool,
    pub file: Option<std::path::PathBuf>,
}

fn rotation_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A sha256 fingerprint as read aloud: 64 lowercase hex digits, with or
/// without `sha256:`.
fn fingerprint_ok(f: &str) -> bool {
    let h = f.strip_prefix("sha256:").unwrap_or(f);
    h.len() == 64
        && h.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// The transport the agreement in `work` chose, if it can be read.
fn agreed_transport(work: &Path) -> Option<String> {
    let t = std::fs::read_to_string(work.join("agreement.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    v.get("transport")?.get("kind")?.as_str().map(String::from)
}

/// The IPsec rotation for `key` (`ipsec-leaf` or `ipsec-ca`), refused before
/// anything runs when the flags or the peering do not fit, else the one under
/// way in `work` (same kind and read-back) or a new one, recorded first.
pub fn ipsec_rotation(
    work: &Path,
    partner: &str,
    key: &str,
    readback: Option<&str>,
    retire: Option<&str>,
) -> Result<Ipsec> {
    let kind = if key == "ipsec-ca" { "ca" } else { "leaf" };
    match agreed_transport(work).as_deref() {
        Some("ipsec-fips") => {}
        other => {
            return Err(goat_errors::coded(
                "GB-189",
                format!(
                    "`rotate {partner} {key}` rotates the IPsec leaf of an ipsec-fips peering, and this peering's agreement ({}) chose {}: its keys are rotated with leaves, carrier or transport",
                    work.join("agreement.json").display(),
                    other.unwrap_or("no transport this goatbuilder can read")
                ),
            )
            .into())
        }
    }
    if let Some(id) = retire {
        if !rotation_id_ok(id) {
            return Err(goat_errors::coded(
                "GB-189",
                format!("--retire '{id}' is not a rotation id (letters, digits, '-' and '_')"),
            )
            .into());
        }
        return Ok(Ipsec {
            id: id.to_string(),
            kind,
            readback: None,
            retire: true,
            file: None,
        });
    }
    let readback = match (kind, readback) {
        ("ca", Some(f)) if fingerprint_ok(f) => Some(f.to_string()),
        ("ca", got) => {
            return Err(goat_errors::coded(
                "GB-303",
                format!(
                    "`rotate {partner} ipsec-ca` switches to a leaf of our new CA only once the partner trusts it: pass --partner-holds-ipsec-ca <sha256>, the fingerprint the partner read back after placing our new CA ({}). Nothing ran",
                    match got { Some(f) => format!("'{f}' is not 64 hex digits"), None => "it was not given".to_string() }
                ),
            )
            .into())
        }
        (_, _) => None,
    };
    let file = work.join(IPSEC_FILE);
    if let Ok(t) = std::fs::read_to_string(&file) {
        let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
        let (k, id) = (
            v["kind"].as_str().unwrap_or(""),
            v["id"].as_str().unwrap_or(""),
        );
        let r = v["readback"].as_str().map(String::from);
        if k != kind || r != readback || !rotation_id_ok(id) {
            return Err(goat_errors::coded(
                "GB-298",
                format!(
                    "{} records the IPsec rotation '{id}' (ipsec-{k}{}), which is not the one asked for ({key}{}). Finish it with the command that started it, or remove the file to start again; goat-peering-edge keeps the set-aside pair in .previous-{id}/ on the edge. Nothing ran",
                    file.display(),
                    r.as_deref().map(|r| format!(", read-back {r}")).unwrap_or_default(),
                    readback.as_deref().map(|r| format!(", read-back {r}")).unwrap_or_default(),
                ),
            )
            .into());
        }
        return Ok(Ipsec {
            id: id.to_string(),
            kind,
            readback,
            retire: false,
            file: Some(file),
        });
    }
    let id = format!("ipsec-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
    let mut rec = serde_json::json!({"kind": kind, "id": id});
    if let Some(r) = &readback {
        rec["readback"] = serde_json::json!(r);
    }
    std::fs::create_dir_all(work)?;
    std::fs::write(&file, format!("{rec}\n"))?;
    Ok(Ipsec {
        id,
        kind,
        readback,
        retire: false,
        file: Some(file),
    })
}

/// Phase 11 passes the IPsec rotation to its play, before its `-e fed_work=`.
/// Only goat-peering-edge reads these values.
pub(super) fn ipsec(phases: &mut [Phase], i: &Ipsec) {
    let mut vars = format!(
        " -e peering_edge_rotation='{}' -e peering_edge_rotation_kind='{}'",
        i.id, i.kind
    );
    if let Some(r) = &i.readback {
        vars.push_str(&format!(" -e peering_edge_rotation_ca_readback='{r}'"));
    }
    if i.retire {
        vars.push_str(" -e peering_edge_rotation_retire='true'");
    }
    for p in phases.iter_mut().filter(|p| p.n == 11) {
        p.cmd = p
            .cmd
            .replace(" -e fed_work='", &format!("{vars} -e fed_work='"));
    }
}

/// The run that rotates the IPsec leaf: phases 11 to 14, nothing skipped,
/// recorded as a `rotate` run. A start after phase 11 is kept and rotates
/// nothing: phase 11 is not in that run.
pub fn ipsec_run<'a>(o: &Options<'a>, i: Ipsec, again: String) -> (Options<'a>, Surface) {
    let from = o.from.max(11);
    let s = Surface {
        stage: "rotate",
        again: (from <= 11).then_some(again),
        rotate_ipsec: (from <= 11).then_some(i),
        resume: false,
        ..Default::default()
    };
    (Options { from, to: 14, ..*o }, s)
}

/// Phase 8 replaces every leaf: `--rotate` on the leaf helper, and nothing
/// else in the plan changes.
pub(super) fn leaves(phases: &mut [Phase]) {
    for p in phases.iter_mut().filter(|p| p.n == 8) {
        p.cmd.push_str(" --rotate");
    }
}

/// The run that rotates the leaves: phases 8 to 14, nothing skipped, recorded
/// as a `rotate` run. A start after phase 8 (`--from 9` or later) is kept and
/// mints nothing: phase 8 is not in that run.
pub fn leaves_run<'a>(o: &Options<'a>) -> (Options<'a>, Surface) {
    let from = o.from.max(8);
    let s = Surface {
        stage: "rotate",
        again: (from <= 8).then(|| format!("goatbuilder federate {VERB} {} leaves", o.with)),
        rotate_leaves: from <= 8,
        resume: false,
        ..Default::default()
    };
    (Options { from, to: 14, ..*o }, s)
}

/// `goatbuilder federate rotate <partner> <key>`. `readback` and `retire` are
/// `--partner-holds-ipsec-ca` and `--retire`, which only the IPsec keys take.
pub fn rotate(
    repo: &Path,
    o: &Options,
    key: &str,
    readback: Option<&str>,
    retire: Option<&str>,
) -> Result<crate::Report> {
    refused(o.with, key)?;
    if readback.is_some() && key != "ipsec-ca" {
        return Err(goat_errors::coded("GB-189", format!("--partner-holds-ipsec-ca belongs to `goatbuilder federate rotate {} ipsec-ca`, not to `rotate {} {key}`", o.with, o.with)).into());
    }
    if retire.is_some() && !key.starts_with("ipsec-") {
        return Err(goat_errors::coded("GB-189", format!("--retire belongs to `goatbuilder federate rotate {} ipsec-leaf|ipsec-ca`, not to `rotate {} {key}`", o.with, o.with)).into());
    }
    match key {
        "ipsec-leaf" | "ipsec-ca" => {
            let work = super::work_dir(o);
            let i = ipsec_rotation(Path::new(&work), o.with, key, readback, retire)?;
            let mut again = format!("goatbuilder federate {VERB} {} {key}", o.with);
            if let Some(r) = &i.readback {
                again.push_str(&format!(" --partner-holds-ipsec-ca {r}"));
            }
            if i.retire {
                again.push_str(&format!(" --retire {}", i.id));
            }
            let (o, mut s) = ipsec_run(o, i, again);
            super::federate_with(repo, &o, &mut s)
        }
        "leaves" => {
            let (o, mut s) = leaves_run(o);
            super::federate_with(repo, &o, &mut s)
        }
        "carrier" => {
            let work = super::work_dir(o);
            let (o, mut s) = carrier_run(o, Path::new(&work))?;
            super::federate_with(repo, &o, &mut s)
        }
        // A key marked Built without its run here is a mistake in this file.
        k => Err(anyhow::anyhow!(
            "`rotate {} {k}` is marked built but has no run",
            o.with
        )),
    }
}

/// The refusal for a key this build does not rotate, before anything runs.
pub fn refused(partner: &str, key: &str) -> Result<()> {
    let built = || {
        KEYS.iter()
            .filter(|(_, g)| *g == Gate::Built)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match KEYS.iter().find(|(k, _)| *k == key) {
        Some((_, Gate::Built)) => Ok(()),
        Some((k, Gate::NotBuilt(why))) => Err(goat_errors::coded(
            "GB-189",
            format!(
                "`rotate {partner} {k}` is designed, not built (docs/design/federation-key-rotation.md §5): {why}. This goatbuilder rotates {}",
                built()
            ),
        )
        .into()),
        Some((k, Gate::NeedsAdr)) => Err(goat_errors::coded(
            "GB-282",
            format!(
                "`rotate {partner} {k}` needs a decision first: rotating our CA needs both CAs trusted for an overlap, and every check accepts the one CA fingerprint the package specification carries (federation-package-spec.md §5.1). An ADR superseding that section comes before any code (docs/design/federation-key-rotation.md §4). This goatbuilder rotates {}",
                built()
            ),
        )
        .into()),
        None => Err(goat_errors::coded(
            "GB-189",
            format!(
                "`rotate` takes one of {}, not '{key}'",
                KEYS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
            ),
        )
        .into()),
    }
}

/// After phase `n` of a run passed. Once a rotate run's phase 8 has minted the
/// new leaves, the run carries on as the plain one: no `rotate` command is
/// offered again (it would mint another set), and the default resume is.
pub(super) fn after_phase(s: &mut Surface, n: u32) {
    if s.rotate_leaves && n == 8 {
        s.rotate_leaves = false;
        s.again = None;
    }
    // Phase 2 made and exchanged the new carrier keys, or made the new
    // peering key and put it on the interface: that rotation's phase 2 is
    // done, and a later run passes no id. `rotation.json` stays: phase 3
    // hashes it from now on (§7 item 1c).
    // Phase 11 passed the IPsec rotation to the edge, which switched (or
    // retired): its record is done, and a later run passes nothing.
    if n == 11 {
        if let Some(i) = s.rotate_ipsec.take() {
            s.again = None;
            if let Some(f) = i.file {
                let _ = std::fs::remove_file(f);
            }
        }
    }
    if n == 2 {
        if s.rotate_carrier.is_some() {
            s.again = None;
        }
        for p in [s.rotate_carrier.take(), s.rotate_transport.take()]
            .into_iter()
            .flatten()
        {
            let _ = std::fs::remove_file(&p.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: &anyhow::Error) -> &str {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.as_str())
            .unwrap_or_default()
    }

    /// Wiring `transport` before the partner can receive our new package
    /// would leave the tunnel down: it stays refused, and says why, until §5
    /// (ADR 1229) is built. Moving it to Built means changing this test.
    #[test]
    fn transport_is_refused_while_the_partner_cannot_receive_our_new_package() {
        let e = refused("xab", "transport").unwrap_err();
        assert_eq!(code(&e), "GB-189");
        let m = e.to_string();
        assert!(
            m.contains("the partner cannot yet receive our new package"),
            "{m}"
        );
        assert!(m.contains("ADR 1229"), "{m}");
        assert!(m.contains("the tunnel would stay down"), "{m}");
    }

    /// `signing` stays refused until its stage and the partner's re-binding
    /// are both in place.
    #[test]
    fn signing_is_refused_and_says_what_is_missing() {
        let e = refused("xab", "signing").unwrap_err();
        assert_eq!(code(&e), "GB-189");
        let m = e.to_string();
        assert!(
            m.contains("previous seed") && m.contains("confirmation 1"),
            "{m}"
        );
    }

    /// `ca` needs an ADR, not only code: GB-282 names it.
    #[test]
    fn our_ca_is_refused_until_the_adr_superseding_one_ca_fingerprint() {
        let e = refused("xab", "ca").unwrap_err();
        assert_eq!(code(&e), "GB-282");
        let m = e.to_string();
        assert!(
            m.contains("§5.1") && m.contains("ADR") && m.contains("§4"),
            "{m}"
        );
    }

    #[test]
    fn only_the_leaves_and_carrier_are_built_and_nothing_runs_for_a_refused_key() {
        assert_eq!(
            KEYS.iter()
                .filter(|(_, g)| *g == Gate::Built)
                .map(|(k, _)| *k)
                .collect::<Vec<_>>(),
            ["leaves", "carrier", "ipsec-leaf", "ipsec-ca"]
        );
        assert!(refused("xab", "leaves").is_ok());
        assert!(refused("xab", "carrier").is_ok());
    }

    fn ipsec_work(kind: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("agreement.json"),
            format!(r#"{{"transport":{{"kind":"{kind}"}}}}"#),
        )
        .unwrap();
        d
    }
    const FP: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A new IPsec rotation is recorded before anything runs; a second call
    /// resumes it with the same id; another kind or read-back is GB-298.
    #[test]
    fn an_ipsec_rotation_is_recorded_once_and_resumed_by_its_kind_and_read_back() {
        let d = ipsec_work("ipsec-fips");
        let a = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None).unwrap();
        assert!(a.id.starts_with("ipsec-") && a.kind == "ca" && a.readback.as_deref() == Some(FP));
        let rec = std::fs::read_to_string(d.path().join(IPSEC_FILE)).unwrap();
        assert!(rec.contains(&a.id) && rec.contains(FP), "{rec}");
        let b = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None).unwrap();
        assert_eq!(a.id, b.id, "a resume passes the same id");
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None).unwrap_err();
        assert_eq!(code(&e), "GB-298");
        let other = format!("sha256:{}", "f".repeat(64));
        let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(&other), None).unwrap_err();
        assert_eq!(code(&e), "GB-298");
        assert!(e.to_string().contains(&a.id), "{e}");
    }

    /// `ipsec-ca` needs a 64-hex-digit read-back (GB-303), and no record is written without one.
    #[test]
    fn an_ipsec_ca_rotation_needs_a_well_formed_read_back() {
        let d = ipsec_work("ipsec-fips");
        for bad in [None, Some("abc"), Some("sha256:XYZ"), Some(&FP[..70])] {
            let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", bad, None).unwrap_err();
            assert_eq!(code(&e), "GB-303", "{bad:?}");
        }
        assert!(!d.path().join(IPSEC_FILE).exists());
        assert!(
            ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(&FP[7..]), None).is_ok(),
            "bare hex is accepted"
        );
    }

    /// Only an ipsec-fips peering has an IPsec leaf.
    #[test]
    fn an_ipsec_rotation_of_another_transport_is_refused() {
        for kind in ["wireguard", "quic"] {
            let d = ipsec_work(kind);
            let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains(kind), "{e}");
        }
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            code(&ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None).unwrap_err()),
            "GB-189"
        );
    }

    /// The retire step names its rotation, writes no record, and refuses a malformed id.
    #[test]
    fn the_retire_step_names_its_rotation_and_writes_no_record() {
        let d = ipsec_work("ipsec-fips");
        let r = ipsec_rotation(d.path(), "xab", "ipsec-ca", None, Some("ipsec-1")).unwrap();
        assert!(r.retire && r.id == "ipsec-1" && r.file.is_none());
        assert!(!d.path().join(IPSEC_FILE).exists());
        let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", None, Some("x; rm -rf /")).unwrap_err();
        assert_eq!(code(&e), "GB-189");
    }

    /// Phase 11 alone gets the values, before its `-e fed_work=`; after phase 11
    /// passed the record is removed and nothing is offered again.
    #[test]
    fn phase_11_gets_the_rotation_and_its_record_goes_once_phase_11_passed() {
        let d = ipsec_work("ipsec-fips");
        let i = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None).unwrap();
        let mut ph = vec![
            phase(11, "play -e fed_work='/w'"),
            phase(12, "play -e fed_work='/w'"),
        ];
        ipsec(&mut ph, &i);
        assert!(ph[0].cmd.contains(&format!("-e peering_edge_rotation='{}' -e peering_edge_rotation_kind='ca' -e peering_edge_rotation_ca_readback='{FP}' -e fed_work='/w'", i.id)), "{}", ph[0].cmd);
        assert_eq!(ph[1].cmd, "play -e fed_work='/w'");
        let (o, mut s) = ipsec_run(&opts(1), i, "again".into());
        assert_eq!((o.from, o.to), (11, 14));
        after_phase(&mut s, 11);
        assert!(s.rotate_ipsec.is_none() && s.again.is_none());
        assert!(!d.path().join(IPSEC_FILE).exists());
        let (o, s) = ipsec_run(
            &opts(12),
            ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None).unwrap(),
            "again".into(),
        );
        assert!(
            o.from == 12 && s.rotate_ipsec.is_none(),
            "a start after phase 11 rotates nothing"
        );
    }

    fn opts(from: u32) -> Options<'static> {
        Options {
            with: "xab",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some("/nonexistent/w"),
            mock: None,
            from,
            to: 99,
            dry_run: true,
            table: false,
        }
    }

    fn phase(n: u32, cmd: &str) -> Phase {
        Phase {
            n,
            kind: "A",
            label: format!("phase {n}"),
            cmd: cmd.to_string(),
            verb: None,
        }
    }

    /// Phase 2's every play gets the rotation's id and the peering's name,
    /// before its `-e fed_work=`; no other phase changes.
    #[test]
    fn a_carrier_rotation_passes_its_id_to_every_play_of_phase_2_only() {
        let play = |p: &str| {
            format!("cd 'r/ops/ansible' && ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit 'xab_edge' -e fed_edge_group='xab_edge' -e fed_work='/w' -e fed_counterparty='/w/theirs.canonical.json' {p}")
        };
        let two = format!(
            "({}) && ({}) && ({})",
            play("peering-prepare.yml"),
            play("peering-forwarder.yml"),
            play("peering-prepare.yml")
        );
        let mut ps = vec![phase(2, &two), phase(11, &play("peering-wg.yml"))];
        let c = Pending {
            id: "carrier-1".into(),
            file: "/w/.rotation-carrier".into(),
        };
        carrier(&mut ps, &c, "xab");
        let want = " -e peering_wg_carrier_rotation='carrier-1' -e peering_forwarder_carrier_rotation='carrier-1' -e peering_forwarder_carrier_rotation_name='xab' -e fed_work='/w'";
        assert_eq!(ps[0].cmd.matches(want).count(), 3, "{}", ps[0].cmd);
        assert_eq!(ps[1].cmd, play("peering-wg.yml"), "phase 11 is unchanged");
    }

    /// The id is written once, before anything runs, and a resumed rotation
    /// reads the same one; phase 2 passing removes it.
    #[test]
    fn a_carrier_rotation_keeps_one_id_until_phase_2_passes() {
        let d = tempfile::tempdir().unwrap();
        let a = carrier_rotation(d.path()).unwrap();
        assert!(a.id.starts_with("carrier-"), "{a:?}");
        assert_eq!(
            std::fs::read_to_string(d.path().join(CARRIER_FILE))
                .unwrap()
                .trim(),
            a.id
        );
        assert_eq!(
            carrier_rotation(d.path()).unwrap(),
            a,
            "a resumed rotation reads the same id"
        );
        let (o, mut s) = carrier_run(&opts(1), d.path()).unwrap();
        assert_eq!((o.from, o.to), (2, 14));
        assert!(!s.resume);
        assert_eq!(s.stage, "rotate");
        assert_eq!(
            s.again.as_deref(),
            Some("goatbuilder federate rotate xab carrier")
        );
        assert_eq!(s.rotate_carrier.as_ref(), Some(&a));
        after_phase(&mut s, 1);
        assert!(d.path().join(CARRIER_FILE).exists(), "phase 1 leaves it");
        after_phase(&mut s, 2);
        assert!(s.rotate_carrier.is_none());
        assert!(
            s.again.is_none(),
            "after phase 2 the plain resume carries on"
        );
        assert!(
            !d.path().join(CARRIER_FILE).exists(),
            "phase 2 passing removes it"
        );
        std::fs::write(d.path().join(CARRIER_FILE), "../x\n").unwrap();
        assert!(carrier_rotation(d.path())
            .unwrap_err()
            .to_string()
            .contains("is not a rotation id"));
    }

    /// A start after phase 2 rotates nothing and writes nothing.
    #[test]
    fn a_carrier_rotation_from_after_phase_2_rotates_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (o, s) = carrier_run(&opts(5), d.path()).unwrap();
        assert_eq!(o.from, 5);
        assert!(s.rotate_carrier.is_none());
        assert!(s.again.is_none());
        assert!(!d.path().join(CARRIER_FILE).exists());
    }

    /// Phase 2's every play gets the peering key rotation's id before its
    /// `-e fed_work=`; no other phase changes.
    #[test]
    fn a_transport_rotation_passes_its_id_to_every_play_of_phase_2_only() {
        let play = |p: &str| format!("ansible-playbook -e fed_work='/w' {p}");
        let two = format!(
            "({}) && ({})",
            play("peering-prepare.yml"),
            play("peering-prepare.yml")
        );
        let mut ps = vec![
            phase(2, &two),
            phase(3, "goat-federation init"),
            phase(11, &play("peering-wg.yml")),
        ];
        let t = Pending {
            id: "transport-1".into(),
            file: "/w/.rotation-transport".into(),
        };
        transport(&mut ps, &t);
        let want = " -e peering_wg_transport_rotation='transport-1' -e fed_work='/w'";
        assert_eq!(ps[0].cmd.matches(want).count(), 2, "{}", ps[0].cmd);
        assert_eq!(ps[1].cmd, "goat-federation init");
        assert_eq!(ps[2].cmd, play("peering-wg.yml"), "phase 11 is unchanged");
    }

    /// The record phase 3 reads is written once, before anything runs, with
    /// the rotation's id as its nonce. A resumed rotation keeps both, phase 2
    /// passing ends the rotation and keeps the record, and the next rotation
    /// writes a new record in its place (§7 item 1c).
    #[test]
    fn a_transport_rotation_writes_the_record_phase_3_reads_once() {
        let d = tempfile::tempdir().unwrap();
        let record = d.path().join(RECORD_FILE);
        let (o, mut s) = transport_run(&opts(1), d.path()).unwrap();
        assert_eq!((o.from, o.to), (2, 14));
        assert!(!s.resume);
        let t = s.rotate_transport.clone().expect("a rotation under way");
        assert!(t.id.starts_with("transport-"), "{t:?}");
        let want = format!("{{\"key\":\"transport\",\"nonce\":\"{}\"}}\n", t.id);
        assert_eq!(std::fs::read_to_string(&record).unwrap(), want);
        let (_, s2) = transport_run(&opts(1), d.path()).unwrap();
        assert_eq!(
            s2.rotate_transport.as_ref(),
            Some(&t),
            "a resumed rotation keeps its id"
        );
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            want,
            "and its record"
        );
        after_phase(&mut s, 2);
        assert!(s.rotate_transport.is_none());
        assert!(
            !d.path().join(TRANSPORT_FILE).exists(),
            "phase 2 passing ends the rotation"
        );
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            want,
            "and keeps the record"
        );
        // A start after phase 2 rotates nothing and leaves the record alone.
        let (o, s3) = transport_run(&opts(3), d.path()).unwrap();
        assert_eq!(o.from, 3);
        assert!(s3.rotate_transport.is_none());
        assert_eq!(std::fs::read_to_string(&record).unwrap(), want);
        // The next rotation writes a new record in its place.
        std::fs::write(d.path().join(TRANSPORT_FILE), "transport-2\n").unwrap();
        let (_, s4) = transport_run(&opts(1), d.path()).unwrap();
        assert_eq!(s4.rotate_transport.unwrap().id, "transport-2");
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            "{\"key\":\"transport\",\"nonce\":\"transport-2\"}\n"
        );
    }

    #[test]
    fn only_the_leaves_carrier_and_ipsec_keys_rotate_and_every_other_key_says_it_is_not_built() {
        let o = opts(1);
        for k in ["transport", "signing"] {
            let e = rotate(Path::new("/nonexistent"), &o, k, None, None).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains("designed, not built"), "{k}: {e}");
            assert!(
                e.to_string()
                    .contains("This goatbuilder rotates leaves, carrier, ipsec-leaf, ipsec-ca"),
                "{k}: {e}"
            );
        }
        assert_eq!(
            code(&rotate(Path::new("/nonexistent"), &o, "ca", None, None).unwrap_err()),
            "GB-282"
        );
        let e = rotate(Path::new("/nonexistent"), &o, "everything", None, None).unwrap_err();
        assert_eq!(code(&e), "GB-189");
        assert!(
            e.to_string()
                .contains("leaves, carrier, ipsec-leaf, ipsec-ca, transport, signing, ca"),
            "{e}"
        );
    }
}

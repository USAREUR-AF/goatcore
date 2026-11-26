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
//! Built: `transport`, the peering's WireGuard key (K4). Before
//! anything runs, the rotation's id is kept in `<work>/.rotation-transport`
//! and written as the nonce of `<work>/rotation.json`, once (§7 item 1c).
//! Phase 2 passes the id to goat-peering-wg, which moves the key aside, once,
//! to `.previous-<id>/`, makes a new one and restarts the link onto it (its
//! `tasks/transport-key.yml` and `tasks/transport-live.yml`). Phase 3 hashes
//! `rotation.json`, so it measures the new key into a new package, and phase
//! 5 offers that package to the partner. The run is phases 2 to 14. The
//! partner's phase 5 takes it once both sides compare its digest on the call
//! (`--accept-partner-package`, ADR 1229); until their phase 11 runs, the
//! tunnel is down (§3 K4).
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
//! Refused: `signing` with GB-189, naming what is not built yet. `ca` with
//! GB-282: our CA's rotation is decided (ADR 1242: the next CA announced in
//! package v2, §4) and not built yet.

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
    /// The key: leaves, carrier, ipsec-leaf, ipsec-ca, transport, signing or ca. This build rotates leaves, carrier, ipsec-leaf, ipsec-ca and transport.
    pub key: String,
    /// ipsec-ca only: the partner's attestation that it placed our new IPsec CA
    /// beside its current one: the sha256 fingerprint it read back from the file
    /// it placed. goatbuilder checks the fingerprint is our staged CA's; it
    /// cannot check the partner's edge.
    #[arg(long = "partner-holds-ipsec-ca", value_name = "SHA256")]
    pub partner_holds_ipsec_ca: Option<String>,
    /// ipsec-leaf or ipsec-ca: run the retire step of this rotation id, which
    /// removes the set-aside key. Only once phase 14 passed here after the
    /// switch (check the partner's on the call), and only while no other
    /// rotation is under way.
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
    /// Refused with GB-282: decided (ADR 1242), not built yet.
    NeedsAdr,
}

/// The keys `rotate` names, and whether this build rotates each. A key moves
/// to `Built` only together with what makes its rotation safe end to end:
/// `transport` moved once the partner's phase 5 could take our new package
/// (ADR 1229, `confirm::incoming`).
pub const KEYS: &[(&str, Gate)] = &[
    ("leaves", Gate::Built),
    ("carrier", Gate::Built),
    ("ipsec-leaf", Gate::Built),
    ("ipsec-ca", Gate::Built),
    ("transport", Gate::Built),
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
        std::fs::write(&record, want).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(&record)).display())))?;
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
            std::fs::create_dir_all(work).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(work)).display())))?;
            std::fs::write(&file, format!("{id}\n")).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", std::path::Path::new(&(&file)).display())))?;
            id
        }
    };
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(goat_errors::coded(
            "GB-321",
            format!(
                "{} holds '{id}', which is not a rotation id; remove the file to start the rotation again",
                file.display()
            ),
        )
        .into());
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
pub(super) fn transport(phases: &mut [Phase], t: &Pending) -> Result<()> {
    inject(
        phases,
        2,
        &format!(" -e peering_wg_transport_rotation='{}'", t.id),
        "the peering key rotation",
    )
}

/// The run that rotates the peering key: phases 2 to 14, nothing skipped,
/// recorded as a `rotate` run. A start after phase 2 is kept and rotates
/// nothing, and writes no record. Until phase 2 passes, the command that
/// carries on is `rotate <partner> transport` again, which reads the same id.
/// The tunnel is down from phase 2 until the partner's phase 11 runs with
/// our new package (§3 K4): the partner runs `goatbuilder federate <us>
/// --from 5 --accept-partner-package <digest>`, the digest compared on the
/// call (ADR 1229).
pub fn transport_run<'a>(o: &Options<'a>, work: &Path) -> Result<(Options<'a>, Surface)> {
    let from = o.from.max(2);
    let s = Surface {
        stage: "rotate",
        again: (from <= 2).then(|| format!("goatbuilder federate {VERB} {} transport", o.with)),
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

/// The IPsec rotations of this peering, as this side ran them: each one's id,
/// kind, read-back, and when its phase 11 switched, its phase 14 passed after
/// the switch, and its retire step ran. The retire step is allowed only for a
/// rotation whose phase 14 passed here.
pub const IPSEC_LEDGER: &str = "ipsec-rotations.json";

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

/// A sha256 fingerprint as read aloud (64 hex digits, either case, with or
/// without `sha256:`), in one form: `sha256:` and lowercase. `None` when it is
/// not one.
fn fingerprint(f: &str) -> Option<String> {
    let t = f.trim();
    let h = t
        .strip_prefix("sha256:")
        .or_else(|| t.strip_prefix("SHA256:"))
        .unwrap_or(t)
        .to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("sha256:{h}"))
}

/// The ledger of this peering's IPsec rotations (`IPSEC_LEDGER`): a list of
/// objects with `id`, `kind`, `readback` (for `ca`, the partner's attestation),
/// and the times `started`, `switched` (phase 11 passed), `accepted` (phase 14
/// passed after the switch) and `retired`.
fn ledger_read(work: &Path) -> Result<Vec<serde_json::Value>> {
    let f = work.join(IPSEC_LEDGER);
    match std::fs::read_to_string(&f) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(goat_errors::coded(
            "GB-298",
            format!("{} cannot be read: {e}. Nothing ran", f.display()),
        )
        .into()),
        Ok(t) => serde_json::from_str::<serde_json::Value>(&t)
            .ok()
            .and_then(|v| v.get("rotations").and_then(|r| r.as_array()).cloned())
            .ok_or_else(|| {
                goat_errors::coded(
                    "GB-298",
                    format!(
                        "{} is not a ledger of IPsec rotations; it was left as it is. Nothing ran",
                        f.display()
                    ),
                )
                .into()
            }),
    }
}

fn ledger_write(work: &Path, rows: &[serde_json::Value]) -> Result<()> {
    let f = work.join(IPSEC_LEDGER);
    let tmp = f.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        format!("{}\n", serde_json::json!({ "rotations": rows })),
    )
    .map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &f).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", f.display())))?;
    Ok(())
}

/// Set `field` to now on the ledger rows `pick` chooses; best effort, as the
/// phase it records has already passed.
fn ledger_mark(
    work: &Path,
    field: &str,
    pick: &dyn Fn(&serde_json::Value) -> bool,
) -> Option<String> {
    let warn = |e: String| {
        Some(format!(
            "{} was not updated ({field}): {e}. Until it is, a new IPsec rotation does not wait for this one's retire, and --retire does not see it; check the working folder and record it by hand",
            work.join(IPSEC_LEDGER).display()
        ))
    };
    let mut rows = match ledger_read(work) {
        Ok(r) => r,
        Err(e) => return warn(e.to_string()),
    };
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut changed = false;
    for r in rows.iter_mut().filter(|r| pick(r)) {
        r[field] = serde_json::json!(now);
        changed = true;
    }
    if changed {
        if let Err(e) = ledger_write(work, &rows) {
            return warn(e.to_string());
        }
    }
    None
}

fn has(r: &serde_json::Value, field: &str) -> bool {
    r.get(field)
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.is_empty())
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
    phase_11_in_run: bool,
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
    let file = work.join(IPSEC_FILE);
    let ledger = ledger_read(work)?;
    if let Some(id) = retire {
        if !rotation_id_ok(id) {
            return Err(goat_errors::coded(
                "GB-189",
                format!("--retire '{id}' is not a rotation id (letters, digits, '-' and '_')"),
            )
            .into());
        }
        if readback.is_some() {
            return Err(goat_errors::coded(
                "GB-189",
                format!("--retire and --partner-holds-ipsec-ca do not go together: the retire step of '{id}' moves no CA and takes no attestation. Nothing ran"),
            )
            .into());
        }
        if file.exists() {
            return Err(goat_errors::coded(
                "GB-298",
                format!("{} records an IPsec rotation under way, so '{id}' is not retired: finish that rotation first (the command that started it), then retire. Nothing ran", file.display()),
            )
            .into());
        }
        let row = ledger.iter().find(|r| r["id"].as_str() == Some(id));
        let Some(row) =
            row.filter(|r| has(r, "switched") && has(r, "accepted") && !has(r, "retired"))
        else {
            return Err(goat_errors::coded(
                "GB-309",
                format!(
                    "the retire step finds no confirmed rotation to retire: {} shows no IPsec rotation '{id}' whose phase 11 switched and whose phase 14 passed here after the switch{}. Retire only an id the rotation printed, once phase 14 passed here after the switch, and the partner says theirs did. Nothing ran",
                    work.join(IPSEC_LEDGER).display(),
                    match row { Some(r) if has(r, "retired") => " (it is already retired)", Some(_) => " (phase 14 has not passed here since its switch)", None => "" }
                ),
            )
            .into());
        };
        let rkind = if row["kind"].as_str() == Some("ca") {
            "ca"
        } else {
            "leaf"
        };
        if rkind != kind {
            return Err(goat_errors::coded(
                "GB-298",
                format!("'{id}' was an ipsec-{rkind} rotation, not {key}: retire it as `rotate {partner} ipsec-{rkind} --retire {id}`. Nothing ran"),
            )
            .into());
        }
        return Ok(Ipsec {
            id: id.to_string(),
            kind: rkind,
            readback: None,
            retire: true,
            file: None,
        });
    }
    let readback = match (kind, readback) {
        ("ca", Some(f)) if fingerprint(f).is_some() => fingerprint(f),
        ("ca", got) => {
            return Err(goat_errors::coded(
                "GB-303",
                format!(
                    "`rotate {partner} ipsec-ca` switches to a leaf of our new CA only with the partner's attestation that it placed that CA: pass --partner-holds-ipsec-ca <sha256>, the fingerprint the partner read back from the file it placed ({}). Nothing ran",
                    match got { Some(f) => format!("'{f}' is not 64 hex digits"), None => "it was not given".to_string() }
                ),
            )
            .into())
        }
        (_, _) => None,
    };
    if let Ok(t) = std::fs::read_to_string(&file) {
        let v: Option<serde_json::Value> = serde_json::from_str(&t).ok();
        let (k, id, r) = match &v {
            Some(v) => (
                v["kind"].as_str().unwrap_or(""),
                v["id"].as_str().unwrap_or(""),
                v["readback"].as_str().and_then(fingerprint),
            ),
            None => ("", "", None),
        };
        if !matches!(k, "leaf" | "ca") || !rotation_id_ok(id) {
            return Err(goat_errors::coded(
                "GB-298",
                format!(
                    "{} is not a rotation record (it is empty, not JSON, or has no kind and id), so the rotation under way is not known. Read it, and the edge's /etc/swanctl/.previous-*/ directories, before removing it. Nothing ran",
                    file.display()
                ),
            )
            .into());
        }
        if k != kind || r != readback {
            return Err(goat_errors::coded(
                "GB-298",
                format!(
                    "{} records the IPsec rotation '{id}' (ipsec-{k}{}), which is not the one asked for ({key}{}). Finish it with the command that started it. Removing the file starts a new rotation id; the edge keeps .previous-{id}/ as it is (after a GB-302 rollback, with the old pair live), and nothing removes it yet. Nothing ran",
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
    if let Some(open) = ledger
        .iter()
        .find(|r| has(r, "switched") && !has(r, "retired"))
    {
        let (oid, ok) = (
            open["id"].as_str().unwrap_or(""),
            open["kind"].as_str().unwrap_or("leaf"),
        );
        return Err(goat_errors::coded(
            "GB-298",
            format!(
                "the IPsec rotation '{oid}' switched and is not retired, so its set-aside pair is still on the edge: retire it first, once phase 14 passed here after the switch and the partner says theirs did: goatbuilder federate {VERB} {partner} ipsec-{ok} --retire {oid}. Nothing ran"
            ),
        )
        .into());
    }
    let id = format!("ipsec-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
    // A run that leaves phase 11 out rotates nothing, so it records nothing.
    if !phase_11_in_run {
        return Ok(Ipsec {
            id,
            kind,
            readback,
            retire: false,
            file: None,
        });
    }
    let mut rec = serde_json::json!({"kind": kind, "id": id});
    if let Some(r) = &readback {
        rec["readback"] = serde_json::json!(r);
    }
    std::fs::create_dir_all(work).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", work.display())))?;
    std::fs::write(&file, format!("{rec}\n")).map_err(|e| goat_errors::coded("GB-254", format!("{}: {e}", file.display())))?;
    let mut rows = ledger;
    let mut row = serde_json::json!({
        "id": id,
        "kind": kind,
        "started": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    if let Some(r) = &readback {
        row["readback"] = serde_json::json!(r);
    }
    rows.push(row);
    ledger_write(work, &rows)?;
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
pub(super) fn ipsec(phases: &mut [Phase], i: &Ipsec) -> Result<()> {
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
    inject(phases, 11, &vars, "the IPsec rotation")
}

/// Put `vars` before every `-e fed_work=` of phase `n`, and refuse when there
/// is none: a rotation whose values reach no play would run the phase as a
/// plain one and report the rotation done.
fn inject(phases: &mut [Phase], n: u32, vars: &str, what: &str) -> Result<()> {
    let mut made = 0;
    for p in phases.iter_mut().filter(|p| p.n == n) {
        made += p.cmd.matches(" -e fed_work='").count();
        p.cmd = p
            .cmd
            .replace(" -e fed_work='", &format!("{vars} -e fed_work='"));
    }
    if made == 0 {
        return Err(goat_errors::coded(
            "GB-328",
            format!("phase {n} has no play to pass {what} to (no `-e fed_work=` in its command); nothing ran"),
        )
        .into());
    }
    Ok(())
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
            let i = ipsec_rotation(
                Path::new(&work),
                o.with,
                key,
                readback,
                retire,
                o.from <= 11,
            )?;
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
            no_carrier_refused(repo, o)?;
            let work = super::work_dir(o);
            let (o, mut s) = carrier_run(o, Path::new(&work))?;
            super::federate_with(repo, &o, &mut s)
        }
        "transport" => {
            let work = super::work_dir(o);
            let (o, mut s) = transport_run(o, Path::new(&work))?;
            super::federate_with(repo, &o, &mut s)
        }
        // A key marked Built without its run here is a mistake in this file.
        k => Err(goat_errors::coded(
            "GB-322",
            format!("`rotate {} {k}` is marked built but has no run", o.with),
        )
        .into()),
    }
}

/// The carrier is the forwarder's (docs/design/peering-forwarder.md §2): a
/// peering no forwarder relays has none, and its edge's role ignores a carrier
/// rotation's id, so the run would pass having changed no key. Refused before
/// anything runs, and before the id is kept. A peering has a forwarder when
/// goatbuilder.yaml declares one, or when its edge group's hand-written
/// group_vars turn on `peering_wg_via_forwarder` (a hand-run deployment's
/// declaration, the role's own switch for the carrier), read in the group the
/// run deploys to ([`super::run_edge_group`]). An undeclared peering
/// is left to the run's own refusal (GB-149).
fn no_carrier_refused(repo: &Path, o: &Options) -> Result<()> {
    let cfg = super::config::load(&super::config_path(repo, o))?;
    let Some(p) = super::declaration::peering(&cfg, o.with, o.env)? else {
        return Ok(());
    };
    if p.forwarder.is_some() {
        // goatbuilder applies this forwarder: phase 2 runs its play with the
        // rotation's id after copying the edge's new carrier key
        // (forwarder::splice), so the forwarder takes the new key itself.
        return Ok(());
    }
    let edge_group = super::run_edge_group(o);
    if !super::peering_files::via_forwarder(repo, &edge_group) {
        return Err(goat_errors::coded(
            "GB-317",
            format!(
                "`rotate {w} carrier`: peering {w} on {env} declares no forwarder, neither in goatbuilder.yaml nor as `peering_wg_via_forwarder: true` in its edge group's group_vars, so its edge has no carrier to rotate (the carrier is the link to a peering forwarder, docs/design/peering-forwarder.md). Nothing ran (GB-317). Rotate its peering key with `rotate {w} transport` once that is built, or its leaves with `rotate {w} leaves`",
                w = o.with,
                env = p.environment
            ),
        )
        .into());
    }
    // A forwarder declared by hand: its entry, with the edge's carrier public
    // key, is in the operator's own group_vars, which a rotation cannot
    // change, and the edge's new key only exists once phase 2 has made it on
    // the edge. Rotating would leave the carrier down, so nothing runs.
    Err(goat_errors::coded(
        "GB-312",
        format!(
            "`rotate {w} carrier`: peering {w} on {env} is relayed by a forwarder declared by hand (`peering_wg_via_forwarder: true` in {edge_group}'s group_vars; goatbuilder.yaml does not declare it). Nothing ran (GB-312). The forwarder holds the edge's carrier public key in peering_forwarder_peerings[name={w}].edge_carrier_public_key, in its hand-written group_vars, which this rotation cannot change; the new value would be the edge's new carrier public key, which phase 2 makes on the edge, so it cannot be known before the rotation runs; and peering-forwarder.yml is the play that applies it. A rotation would leave the carrier down. To rotate this carrier, declare the forwarder on the peering in goatbuilder.yaml (`forwarder:` with its endpoint), run `goatbuilder federate allocate`, review and commit what it writes, and run `goatbuilder federate rotate {w} carrier` again: phase 2 then gives the forwarder the edge's new key itself",
            w = o.with,
            env = p.environment
        ),
    )
    .into())
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
                "`rotate {partner} {k}` is decided, not built: rotating our CA needs both CAs trusted for an overlap, and ADR 1242 decides it (our next CA announced in package v2, where every check of v1 accepts the one CA fingerprint federation-package-spec.md §5.1 binds), but this goatbuilder does not build that overlap yet (docs/design/federation-key-rotation.md §4). This goatbuilder rotates {}",
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
pub(super) fn after_phase(s: &mut Surface, n: u32, work: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    // Phase 14 passed: every IPsec rotation that switched and has not been
    // accepted since is accepted now, which is what allows its retire step.
    // Only a ledger that exists is touched: a peering with no IPsec rotation
    // has none.
    if n == 14 && work.join(IPSEC_LEDGER).exists() {
        warnings.extend(ledger_mark(work, "accepted", &|r| {
            has(r, "switched") && !has(r, "accepted")
        }));
    }
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
            let field = if i.retire { "retired" } else { "switched" };
            warnings.extend(ledger_mark(work, field, &|r| {
                r["id"].as_str() == Some(i.id.as_str())
            }));
            if let Some(f) = i.file {
                let _ = std::fs::remove_file(f);
            }
        }
    }
    if n == 2 {
        if s.rotate_carrier.is_some() || s.rotate_transport.is_some() {
            s.again = None;
        }
        for p in [s.rotate_carrier.take(), s.rotate_transport.take()]
            .into_iter()
            .flatten()
        {
            let _ = std::fs::remove_file(&p.file);
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: &anyhow::Error) -> &str {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.as_str())
            .unwrap_or_default()
    }

    /// `transport` is rotated now that the partner's phase 5 takes our new
    /// package (ADR 1229, `confirm::incoming`).
    #[test]
    fn transport_is_rotated_now_that_the_partner_can_receive_our_new_package() {
        assert!(refused("xab", "transport").is_ok());
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

    /// `ca` is decided (ADR 1242) and not built: GB-282 says so, and never
    /// that the decision is still to be made.
    #[test]
    fn our_ca_is_refused_as_decided_and_not_built() {
        let e = refused("xab", "ca").unwrap_err();
        assert_eq!(code(&e), "GB-282");
        let m = e.to_string();
        assert!(
            m.contains("is decided, not built") && m.contains("ADR 1242") && m.contains("§4"),
            "{m}"
        );
        assert!(!m.contains("needs a decision") && !m.contains("comes before any code"), "{m}");
    }

    #[test]
    fn only_the_leaves_carrier_transport_and_ipsec_keys_are_built_and_nothing_runs_for_a_refused_key() {
        assert_eq!(
            KEYS.iter()
                .filter(|(_, g)| *g == Gate::Built)
                .map(|(k, _)| *k)
                .collect::<Vec<_>>(),
            ["leaves", "carrier", "ipsec-leaf", "ipsec-ca", "transport"]
        );
        assert!(refused("xab", "leaves").is_ok());
        assert!(refused("xab", "carrier").is_ok());
        assert!(refused("xab", "transport").is_ok());
        assert!(refused("xab", "ipsec-leaf").is_ok());
        assert!(refused("xab", "ipsec-ca").is_ok());
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
        let a = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None, true).unwrap();
        assert!(a.id.starts_with("ipsec-") && a.kind == "ca" && a.readback.as_deref() == Some(FP));
        let rec = std::fs::read_to_string(d.path().join(IPSEC_FILE)).unwrap();
        assert!(rec.contains(&a.id) && rec.contains(FP), "{rec}");
        let b = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None, true).unwrap();
        assert_eq!(a.id, b.id, "a resume passes the same id");
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err();
        assert_eq!(code(&e), "GB-298");
        let other = format!("sha256:{}", "f".repeat(64));
        let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(&other), None, true).unwrap_err();
        assert_eq!(code(&e), "GB-298");
        assert!(e.to_string().contains(&a.id), "{e}");
    }

    /// `ipsec-ca` needs a 64-hex-digit read-back (GB-303), and no record is written without one.
    #[test]
    fn an_ipsec_ca_rotation_needs_a_well_formed_read_back() {
        let d = ipsec_work("ipsec-fips");
        for bad in [None, Some("abc"), Some("sha256:XYZ"), Some(&FP[..70])] {
            let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", bad, None, true).unwrap_err();
            assert_eq!(code(&e), "GB-303", "{bad:?}");
        }
        assert!(!d.path().join(IPSEC_FILE).exists());
        assert!(
            ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(&FP[7..]), None, true).is_ok(),
            "bare hex is accepted"
        );
    }

    /// Only an ipsec-fips peering has an IPsec leaf.
    #[test]
    fn an_ipsec_rotation_of_another_transport_is_refused() {
        for kind in ["wireguard", "quic"] {
            let d = ipsec_work(kind);
            let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains(kind), "{e}");
        }
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            code(&ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err()),
            "GB-189"
        );
    }

    /// A rotation that switched at phase 11 and passed phase 14 afterwards,
    /// in `d`'s ledger.
    fn accepted(d: &Path, key: &str, readback: Option<&str>) -> String {
        let i = ipsec_rotation(d, "xab", key, readback, None, true).unwrap();
        let (_, mut s) = ipsec_run(&opts(11), i.clone(), "again".into());
        after_phase(&mut s, 11, d);
        after_phase(&mut s, 14, d);
        i.id
    }

    /// The retire step names its rotation, writes no record, refuses a
    /// malformed id, and takes its kind from the ledger.
    #[test]
    fn the_retire_step_names_its_rotation_and_writes_no_record() {
        let d = ipsec_work("ipsec-fips");
        let id = accepted(d.path(), "ipsec-ca", Some(FP));
        let r = ipsec_rotation(d.path(), "xab", "ipsec-ca", None, Some(&id), true).unwrap();
        assert!(r.retire && r.id == id && r.file.is_none() && r.kind == "ca");
        assert!(!d.path().join(IPSEC_FILE).exists());
        let e = ipsec_rotation(d.path(), "xab", "ipsec-ca", None, Some("x; rm -rf /"), true)
            .unwrap_err();
        assert_eq!(code(&e), "GB-189");
        // The kind is the retired rotation's, not the key's.
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&id), true).unwrap_err();
        assert_eq!(code(&e), "GB-298", "{e}");
    }

    /// Item 4: no retire while a rotation is under way; only for an id whose
    /// phase 14 passed here; never with a read-back.
    #[test]
    fn retire_needs_a_rotation_this_side_saw_pass_phase_14() {
        let d = ipsec_work("ipsec-fips");
        // Nothing in the ledger: refused.
        let e =
            ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some("ipsec-1"), true).unwrap_err();
        assert_eq!(code(&e), "GB-309", "{e}");
        // Under way (the record is there): refused.
        let i = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap();
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&i.id), true).unwrap_err();
        assert_eq!(code(&e), "GB-298", "{e}");
        // Switched at phase 11, phase 14 not passed yet: refused.
        let (_, mut s) = ipsec_run(&opts(11), i.clone(), "again".into());
        after_phase(&mut s, 11, d.path());
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&i.id), true).unwrap_err();
        assert_eq!(code(&e), "GB-309", "{e}");
        // Phase 14 passed: allowed.
        after_phase(&mut s, 14, d.path());
        assert!(ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&i.id), true).is_ok());
        // Never together with a read-back.
        let e =
            ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), Some(&i.id), true).unwrap_err();
        assert_eq!(code(&e), "GB-189", "{e}");
        // Once retired, a second retire of the same id is refused.
        let r = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&i.id), true).unwrap();
        let (_, mut s) = ipsec_run(&opts(11), r, "again".into());
        after_phase(&mut s, 11, d.path());
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some(&i.id), true).unwrap_err();
        assert_eq!(code(&e), "GB-309", "{e}");
    }

    /// Item 5: a new rotation is refused while an earlier one switched and was
    /// not retired, naming the retire command.
    #[test]
    fn a_new_rotation_waits_for_the_last_one_to_be_retired() {
        let d = ipsec_work("ipsec-fips");
        let id = accepted(d.path(), "ipsec-leaf", None);
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err();
        assert_eq!(code(&e), "GB-298", "{e}");
        assert!(e.to_string().contains(&format!("--retire {id}")), "{e}");
        assert!(!d.path().join(IPSEC_FILE).exists(), "nothing recorded");
    }

    /// The ledger's "accepted" is this side's phase 14: every message and the
    /// help say "phase 14 passed here after the switch"; "both sides" is advice.
    #[test]
    fn the_retire_wording_says_whose_phase_14_it_saw() {
        const HERE: &str = "phase 14 passed here after the switch";
        let d = ipsec_work("ipsec-fips");
        let e =
            ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, Some("ipsec-1"), true).unwrap_err();
        assert!(e.to_string().contains(HERE), "{e}");
        accepted(d.path(), "ipsec-leaf", None);
        let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err();
        assert!(e.to_string().contains(HERE), "{e}");
        use clap::CommandFactory;
        #[derive(clap::Parser)]
        struct T {
            #[command(flatten)]
            r: RotateArgs,
        }
        let help = T::command().render_long_help().to_string();
        assert!(help.contains(HERE), "{help}");
    }

    /// A ledger mark that cannot be written is a warning row, so a second
    /// rotation cannot stack on one the ledger never saw switch.
    #[test]
    fn a_ledger_that_cannot_be_written_is_a_warning() {
        let d = ipsec_work("ipsec-fips");
        let i = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap();
        let (_, mut s) = ipsec_run(&opts(11), i, "again".into());
        std::fs::remove_file(d.path().join(IPSEC_LEDGER)).unwrap();
        std::fs::create_dir(d.path().join(IPSEC_LEDGER)).unwrap();
        let w = after_phase(&mut s, 11, d.path());
        assert!(
            w.iter()
                .any(|m| m.contains(IPSEC_LEDGER) && m.contains("switched")),
            "{w:?}"
        );
        assert!(after_phase(&mut s, 3, d.path()).is_empty());
    }

    /// Item 5: the read-back is compared as a fingerprint: `sha256:` or not,
    /// upper or lower case.
    #[test]
    fn the_read_back_is_compared_as_a_fingerprint() {
        let d = ipsec_work("ipsec-fips");
        let a = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None, true).unwrap();
        let upper = FP[7..].to_uppercase();
        let b = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(&upper), None, true).unwrap();
        assert_eq!(
            a.id, b.id,
            "the same read-back, written another way, resumes"
        );
        assert_eq!(b.readback.as_deref(), Some(FP), "passed on in one form");
    }

    /// Item 5: an empty or corrupt record is refused with its own message.
    #[test]
    fn an_empty_or_corrupt_record_is_named() {
        for body in ["", "{not json", "{\"kind\":\"leaf\"}"] {
            let d = ipsec_work("ipsec-fips");
            std::fs::write(d.path().join(IPSEC_FILE), body).unwrap();
            let e = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap_err();
            assert_eq!(code(&e), "GB-298", "{body:?}");
            let m = e.to_string();
            assert!(
                m.contains(IPSEC_FILE) && m.contains("not a rotation record"),
                "{body:?}: {m}"
            );
        }
    }

    /// Item 6: a run that leaves phase 11 out writes no record.
    #[test]
    fn a_run_without_phase_11_records_nothing() {
        let d = ipsec_work("ipsec-fips");
        let i = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, false).unwrap();
        assert!(i.file.is_none());
        assert!(!d.path().join(IPSEC_FILE).exists());
        assert!(!d.path().join(IPSEC_LEDGER).exists());
    }

    /// Item 7: a phase with nowhere to put the values is refused, never passed over.
    #[test]
    fn an_injection_that_finds_no_place_is_refused() {
        let d = ipsec_work("ipsec-fips");
        let i = ipsec_rotation(d.path(), "xab", "ipsec-leaf", None, None, true).unwrap();
        // A rotation's values that reach no play are refused: GB-328, with
        // the federate verb's refused exit (1).
        let mut ph = vec![phase(11, "play peering-edge.yml")];
        assert_eq!(code(&ipsec(&mut ph, &i).unwrap_err()), "GB-328");
        let mut ph = vec![phase(12, "play -e fed_work='/w'")];
        assert_eq!(
            code(&ipsec(&mut ph, &i).unwrap_err()),
            "GB-328",
            "no phase 11 at all"
        );
        let t = Pending {
            id: "transport-1".into(),
            file: "/w/.rotation-transport".into(),
        };
        let mut ps = vec![phase(2, "play peering-prepare.yml")];
        assert_eq!(code(&transport(&mut ps, &t).unwrap_err()), "GB-328");
    }

    /// Item 8: the read-back is an attestation, and the help says so; the
    /// ledger records it.
    #[test]
    fn the_read_back_is_called_an_attestation_and_recorded() {
        use clap::{Args, CommandFactory};
        #[derive(clap::Parser)]
        struct T {
            #[command(flatten)]
            r: RotateArgs,
        }
        let _ = <RotateArgs as Args>::augment_args;
        let help = T::command().render_long_help().to_string();
        assert!(!help.contains("only once the partner trusts it"), "{help}");
        assert!(help.contains("attestation"), "{help}");
        let d = ipsec_work("ipsec-fips");
        accepted(d.path(), "ipsec-ca", Some(FP));
        let l = std::fs::read_to_string(d.path().join(IPSEC_LEDGER)).unwrap();
        assert!(l.contains(FP), "{l}");
    }

    /// Phase 11 alone gets the values, before its `-e fed_work=`; after phase 11
    /// passed the record is removed and nothing is offered again.
    #[test]
    fn phase_11_gets_the_rotation_and_its_record_goes_once_phase_11_passed() {
        let d = ipsec_work("ipsec-fips");
        let i = ipsec_rotation(d.path(), "xab", "ipsec-ca", Some(FP), None, true).unwrap();
        let mut ph = vec![
            phase(11, "play -e fed_work='/w'"),
            phase(12, "play -e fed_work='/w'"),
        ];
        ipsec(&mut ph, &i).unwrap();
        assert!(ph[0].cmd.contains(&format!("-e peering_edge_rotation='{}' -e peering_edge_rotation_kind='ca' -e peering_edge_rotation_ca_readback='{FP}' -e fed_work='/w'", i.id)), "{}", ph[0].cmd);
        assert_eq!(ph[1].cmd, "play -e fed_work='/w'");
        let (o, mut s) = ipsec_run(&opts(1), i, "again".into());
        assert_eq!((o.from, o.to), (11, 14));
        after_phase(&mut s, 11, d.path());
        assert!(s.rotate_ipsec.is_none() && s.again.is_none());
        assert!(!d.path().join(IPSEC_FILE).exists());
        // A new rotation waits for that one's retire, so a fresh peering here.
        let d2 = ipsec_work("ipsec-fips");
        let (o, s) = ipsec_run(
            &opts(12),
            ipsec_rotation(d2.path(), "xab", "ipsec-leaf", None, None, false).unwrap(),
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
        after_phase(&mut s, 1, Path::new("/nonexistent/w"));
        assert!(d.path().join(CARRIER_FILE).exists(), "phase 1 leaves it");
        after_phase(&mut s, 2, Path::new("/nonexistent/w"));
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
        transport(&mut ps, &t).unwrap();
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
        after_phase(&mut s, 2, Path::new("/nonexistent/w"));
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
    fn only_signing_and_ca_are_not_built_and_every_other_key_rotates() {
        let o = opts(1);
        let e = rotate(Path::new("/nonexistent"), &o, "signing", None, None).unwrap_err();
        assert_eq!(code(&e), "GB-189");
        assert!(e.to_string().contains("designed, not built"), "{e}");
        assert!(
            e.to_string()
                .contains("This goatbuilder rotates leaves, carrier, ipsec-leaf, ipsec-ca, transport"),
            "{e}"
        );
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

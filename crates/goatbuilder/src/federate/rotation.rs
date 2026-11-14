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
    /// The key: leaves, carrier, transport, signing or ca. This build rotates leaves and carrier.
    pub key: String,
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

/// `goatbuilder federate rotate <partner> <key>`.
pub fn rotate(repo: &Path, o: &Options, key: &str) -> Result<crate::Report> {
    refused(o.with, key)?;
    match key {
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
            ["leaves", "carrier"]
        );
        assert!(refused("xab", "leaves").is_ok());
        assert!(refused("xab", "carrier").is_ok());
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
    fn only_the_leaves_and_carrier_rotate_and_every_other_key_says_it_is_not_built() {
        let o = opts(1);
        for k in ["transport", "signing"] {
            let e = rotate(Path::new("/nonexistent"), &o, k).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains("designed, not built"), "{k}: {e}");
            assert!(
                e.to_string()
                    .contains("This goatbuilder rotates leaves, carrier"),
                "{k}: {e}"
            );
        }
        assert_eq!(
            code(&rotate(Path::new("/nonexistent"), &o, "ca").unwrap_err()),
            "GB-282"
        );
        let e = rotate(Path::new("/nonexistent"), &o, "everything").unwrap_err();
        assert_eq!(code(&e), "GB-189");
        assert!(
            e.to_string()
                .contains("leaves, carrier, transport, signing, ca"),
            "{e}"
        );
    }
}

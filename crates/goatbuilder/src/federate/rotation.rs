//! `goatbuilder federate rotate <partner> <key>`: replace one key of a running
//! peering and run the command again from the first phase it changes
//! (docs/design/federation-key-rotation.md §5).
//!
//! Built: `leaves`, the peering leaves (K3). Phase 8 runs the leaf helper with
//! `--rotate`, which sets the old pairs aside under `.previous-<random>/` and
//! keeps a `.rotation-pending` journal while it replaces them
//! (`ops/federation/standup/peering_leaf_material.py`); the run is phases 8 to
//! 14. Phase 10 stops, as in every run, for the review of what apply rendered,
//! now with the new leaves, and phase 12 installs them. Once phase 8 has
//! passed, the command that carries on is the plain resume
//! (`goatbuilder federate <partner> --from 11`), so following a stop or a
//! failure never mints another set. Nothing the partner holds changes, so
//! no `rotation.json` is written: phase 3 and our package are untouched.
//!
//! Built: `carrier`, the forwarder carrier keys (K5), as a library: phase 2
//! passes the rotation's id to the plays it runs, and goat-peering-wg and
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
//! partner receiving it is designed, not built (§3 K4, ADR 1229).
//!
//! Designed, not built: `signing` and `ca`.

use std::path::Path;

use anyhow::Result;

use super::run::Surface;

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
/// The stage word: `goatbuilder federate rotate <partner> <key>`. A peering
/// can never be named this (config.rs RESERVED_PEERING_NAMES).
pub const VERB: &str = "rotate";
use super::{Options, Phase};

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
/// The keys `rotate` names, and whether this build rotates each.
pub const KEYS: &[(&str, bool)] = &[
    ("leaves", true),
    ("carrier", true),
    ("transport", true),
    ("signing", false),
    ("ca", false),
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

// Called from main.rs once its `rotate` stage word is wired (one line).
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

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
/// The run that rotates the carrier keys: phases 2 to 14, nothing skipped. A
/// start after phase 2 is kept and rotates nothing: phase 2 is not in that run.
pub fn carrier_run<'a>(o: &Options<'a>, work: &Path) -> Result<(Options<'a>, Surface)> {
    let from = o.from.max(2);
    let s = Surface {
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

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
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

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
/// `goatbuilder federate rotate <partner> <key>`.
pub fn rotate(repo: &Path, o: &Options, key: &str) -> Result<crate::Report> {
    match KEYS.iter().find(|(k, _)| *k == key) {
        Some(("leaves", _)) => {
            let (o, mut s) = leaves_run(o);
            super::federate_with(repo, &o, &mut s)
        }
        Some(("carrier", _)) => {
            let work = super::work_dir(o);
            let (o, mut s) = carrier_run(o, Path::new(&work))?;
            super::federate_with(repo, &o, &mut s)
        }
        Some(("transport", _)) => {
            let work = super::work_dir(o);
            let (o, mut s) = transport_run(o, Path::new(&work))?;
            super::federate_with(repo, &o, &mut s)
        }
        Some((k, _)) => Err(goat_errors::coded(
            "GB-189",
            format!(
                "`rotate {} {k}` is designed, not built (docs/design/federation-key-rotation.md §5): this goatbuilder rotates {}",
                o.with,
                KEYS.iter().filter(|(_, b)| *b).map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
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
        assert_eq!(s.rotate_carrier.as_ref(), Some(&a));
        after_phase(&mut s, 1);
        assert!(d.path().join(CARRIER_FILE).exists(), "phase 1 leaves it");
        after_phase(&mut s, 2);
        assert!(s.rotate_carrier.is_none());
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
    fn only_the_leaves_the_carrier_and_the_transport_rotate_and_every_other_key_says_it_is_not_built(
    ) {
        let o = opts(1);
        for k in ["signing", "ca"] {
            let e = rotate(Path::new("/nonexistent"), &o, k).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains("designed, not built"), "{k}: {e}");
            assert!(
                e.to_string().contains("rotates leaves, carrier, transport"),
                "{k}: {e}"
            );
        }
        let e = rotate(Path::new("/nonexistent"), &o, "everything").unwrap_err();
        assert_eq!(code(&e), "GB-189");
        assert!(
            e.to_string()
                .contains("leaves, carrier, transport, signing, ca"),
            "{e}"
        );
    }
}

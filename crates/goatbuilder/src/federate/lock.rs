//! Two locks keep runs from stepping on each other.
//!
//! The per-peering lock, `<work>/.lock`, is held for the whole of a real run,
//! so two runs of one peering never interleave phases or overwrite each
//! other's journal. A second run is refused (GB-180) and changes nothing.
//!
//! The host-level allocation check: before a run takes a resource on a shared
//! host (a tunnel interface, a port, a link subnet, a certificate folder), it
//! holds `<root>/.allocation.lock` (`root` is the folder that holds every
//! peering's working folder on this machine, `FED_WORK_ROOT` or the parent of
//! `--work`), reads every other peering's journal there, and refuses (GB-181)
//! when one of them holds the same resource on the same host. When nothing
//! collides it records the resources as `claimed` in this peering's journal
//! before the lock is let go, so a second invocation that starts a moment
//! later sees them. A resource is only ever claimed by the peering that names
//! it; nothing here removes or changes another peering's record.
//!
//! The boundary is the work root: peerings whose working folders sit under
//! different roots do not see each other, so keep every peering of one host
//! under one root (the default, /var/lib/goat-federation, does). The journals
//! say what this machine's runs hold. Whether a port is free on
//! the host itself is measured live by preflight just before a change.
//!
//! `claim` has its first caller in `goatbuilder federate deploy`; until then
//! only the tests call it.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::state::State;

/// A working folder or lock that cannot be made or taken (GB-184).
pub fn io_error(what: impl std::fmt::Display, e: impl std::fmt::Display) -> anyhow::Error {
    goat_errors::coded("GB-184", format!("{what}: {e}")).into()
}

pub const PEERING_LOCK: &str = ".lock";
#[cfg_attr(not(test), allow(dead_code))]
pub const HOST_LOCK: &str = ".allocation.lock";

/// The per-peering lock, released when dropped.
pub struct PeeringLock {
    _file: File,
}

impl PeeringLock {
    /// Take `<work>/.lock`, or refuse (GB-180) when another run holds it.
    pub fn take(work: &Path, peering: &str) -> Result<PeeringLock> {
        std::fs::create_dir_all(work).map_err(|e| {
            io_error(
                format!("cannot create the working folder {}", work.display()),
                e,
            )
        })?;
        let p = work.join(PEERING_LOCK);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&p)
            .map_err(|e| io_error(format!("cannot open the peering lock {}", p.display()), e))?;
        // A lock that reads as held by this very process is a child it
        // forked a moment ago that has not exec'd yet (it shares the open
        // file); wait a moment for that, never for another process.
        let mut tries = 0;
        let mut taken = file.try_lock();
        while matches!(taken, Err(std::fs::TryLockError::WouldBlock))
            && tries < 50
            && std::fs::read_to_string(&p).unwrap_or_default().trim()
                == format!("pid {}", std::process::id())
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
            tries += 1;
            taken = file.try_lock();
        }
        match taken {
            Ok(()) => {
                let _ = file.set_len(0);
                let _ = (&file).write_all(format!("pid {}\n", std::process::id()).as_bytes());
                Ok(PeeringLock { _file: file })
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                let holder = std::fs::read_to_string(&p).unwrap_or_default();
                Err(goat_errors::coded(
                    "GB-180",
                    format!(
                        "another run of peering '{peering}' holds {} ({}); one run per peering at a time",
                        p.display(),
                        holder.trim()
                    ),
                )
                .into())
            }
            Err(std::fs::TryLockError::Error(e)) => Err(io_error(
                format!("cannot take the peering lock {}", p.display()),
                e,
            )),
        }
    }
}

/// A resource a run is about to take on a shared host. `scope` is the host
/// the resource lives on, as the inventory resolves it (not a group name:
/// two peerings' groups can name the same host).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    pub kind: String,
    pub scope: String,
    pub value: String,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Want {
    pub fn new(kind: &str, scope: &str, value: &str) -> Self {
        Want {
            kind: kind.into(),
            scope: scope.into(),
            value: value.into(),
        }
    }
}

/// The host-level lock, held from the check until this peering's claims are
/// saved; released when dropped.
#[cfg_attr(not(test), allow(dead_code))]
pub struct HostGuard {
    _file: File,
}

/// The folder that holds every peering's working folder on this machine.
#[cfg_attr(not(test), allow(dead_code))]
pub fn work_root(work: &Path) -> PathBuf {
    match std::env::var("FED_WORK_ROOT")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(r) => PathBuf::from(r),
        None => work
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Check `wants` against every other peering's journal under `root`, and on
/// no collision record them as `claimed` in `state` and save it. Refuses
/// (GB-181) on a collision, and on another journal it cannot read, because
/// then it cannot tell. Returns the guard; drop it once the caller is done
/// with the host-level view.
#[cfg_attr(not(test), allow(dead_code))]
pub fn claim(
    root: &Path,
    work: &Path,
    state: &mut State,
    run: &str,
    wants: &[Want],
) -> Result<HostGuard> {
    std::fs::create_dir_all(root)
        .map_err(|e| io_error(format!("cannot create the work root {}", root.display()), e))?;
    let p = root.join(HOST_LOCK);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p)
        .map_err(|e| {
            io_error(
                format!("cannot open the allocation lock {}", p.display()),
                e,
            )
        })?;
    // Held only for the length of a read of the journals, so wait for it.
    file.lock().map_err(|e| {
        io_error(
            format!("cannot take the allocation lock {}", p.display()),
            e,
        )
    })?;
    let guard = HostGuard { _file: file };

    let mut clashes = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| io_error(format!("cannot read the work root {}", root.display()), e))?
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.is_dir() && !same_dir(d, work))
        .collect();
    entries.sort();
    for dir in entries {
        let other = match State::read(&dir) {
            Ok(Some(s)) => s,
            Ok(None) => continue,
            Err(e) => {
                return Err(goat_errors::coded(
                    "GB-181",
                    format!(
                        "cannot tell whether peering '{}' may take its resources: {e:#}",
                        state.peering
                    ),
                )
                .into())
            }
        };
        if other.peering == state.peering {
            // The same peering from another folder takes another `.lock`, so
            // the per-peering lock cannot keep the two apart: refuse.
            return Err(goat_errors::coded(
                "GB-181",
                format!(
                    "peering '{}' also has a run journal in {}; one working folder per peering, or two runs could take the same resources",
                    state.peering,
                    dir.display()
                ),
            )
            .into());
        }
        for w in wants {
            if other
                .owned
                .iter()
                .any(|o| o.held() && o.same(&w.kind, &w.scope, &w.value))
            {
                clashes.push(format!(
                    "{} {} on {} is held by peering '{}' ({})",
                    w.kind,
                    w.value,
                    w.scope,
                    other.peering,
                    dir.display()
                ));
            }
        }
    }
    if !clashes.is_empty() {
        return Err(goat_errors::coded(
            "GB-181",
            format!(
                "peering '{}' cannot take what another peering on the same host holds: {}",
                state.peering,
                clashes.join("; ")
            ),
        )
        .into());
    }
    for w in wants {
        // A resource this peering already has deployed stays `deployed`.
        let deployed = state
            .owned
            .iter()
            .any(|o| o.same(&w.kind, &w.scope, &w.value) && o.status == "deployed");
        state.own(
            run,
            &w.kind,
            &w.scope,
            &w.value,
            if deployed { "deployed" } else { "claimed" },
        );
    }
    state.save(work)?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: &anyhow::Error) -> String {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_second_run_of_the_same_peering_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let held = PeeringLock::take(d.path(), "xab").unwrap();
        let e = PeeringLock::take(d.path(), "xab")
            .err()
            .expect("a second lock must be refused");
        assert_eq!(code(&e), "GB-180", "{e}");
        drop(held);
        PeeringLock::take(d.path(), "xab").expect("free again once the first run ends");
    }

    #[test]
    fn a_resource_another_peering_holds_is_refused_and_nothing_is_claimed() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("xab"), root.path().join("xac"));
        let mut sa = State::new("xab");
        let g = claim(
            root.path(),
            &a,
            &mut sa,
            "r1",
            &[Want::new("udp-port", "edge-1", "51821")],
        )
        .unwrap();
        drop(g);
        assert_eq!(State::load(&a, "xab").unwrap().owned[0].status, "claimed");

        let mut sb = State::new("xac");
        let e = claim(
            root.path(),
            &b,
            &mut sb,
            "r2",
            &[
                Want::new("udp-port", "edge-1", "51822"),
                Want::new("udp-port", "edge-1", "51821"),
            ],
        )
        .err()
        .expect("the same port on the same host is refused");
        assert_eq!(code(&e), "GB-181");
        assert!(e.to_string().contains("held by peering 'xab'"), "{e}");
        assert!(sb.owned.is_empty(), "a refused claim records nothing");
        assert!(!b.join("state.json").exists(), "and writes nothing");
        // The first peering's journal is untouched.
        assert_eq!(State::load(&a, "xab").unwrap().owned.len(), 1);

        // The same port on another host, or a different port, is fine.
        claim(
            root.path(),
            &b,
            &mut sb,
            "r2",
            &[
                Want::new("udp-port", "edge-2", "51821"),
                Want::new("udp-port", "edge-1", "51822"),
            ],
        )
        .unwrap();
        assert_eq!(State::load(&b, "xac").unwrap().owned.len(), 2);
    }

    #[test]
    fn a_rerun_of_the_same_peering_reclaims_its_own_and_a_released_resource_is_free() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("xab"), root.path().join("xac"));
        let mut sa = State::new("xab");
        let want = [Want::new("wg-interface", "edge-1", "wg-fed1")];
        drop(claim(root.path(), &a, &mut sa, "r1", &want).unwrap());
        drop(
            claim(root.path(), &a, &mut sa, "r2", &want)
                .expect("a peering never collides with itself"),
        );
        assert_eq!(sa.owned.len(), 1);

        sa.own("r3", "wg-interface", "edge-1", "wg-fed1", "released");
        sa.save(&a).unwrap();
        let mut sb = State::new("xac");
        claim(root.path(), &b, &mut sb, "r4", &want).expect("a released resource can be taken");
    }

    #[test]
    fn the_work_root_is_the_folder_above_the_working_folder() {
        if std::env::var_os("FED_WORK_ROOT").is_none() {
            assert_eq!(
                work_root(Path::new("/var/lib/goat-federation/xab")),
                Path::new("/var/lib/goat-federation")
            );
        }
    }

    #[test]
    fn the_same_peering_in_another_folder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let mut a = State::new("xab");
        drop(
            claim(
                root.path(),
                &root.path().join("xab"),
                &mut a,
                "r1",
                &[Want::new("udp-port", "edge-1", "1")],
            )
            .unwrap(),
        );
        let mut b = State::new("xab");
        let e = claim(
            root.path(),
            &root.path().join("xab-again"),
            &mut b,
            "r2",
            &[Want::new("udp-port", "edge-1", "2")],
        )
        .err()
        .expect("two folders for one peering are refused");
        assert_eq!(code(&e), "GB-181");
        assert!(
            e.to_string().contains("one working folder per peering"),
            "{e}"
        );
    }

    #[test]
    fn a_journal_that_cannot_be_read_stops_the_claim() {
        let root = tempfile::tempdir().unwrap();
        let other = root.path().join("xad");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("state.json"), "{broken").unwrap();
        let mut s = State::new("xab");
        let e = claim(
            root.path(),
            &root.path().join("xab"),
            &mut s,
            "r1",
            &[Want::new("udp-port", "edge-1", "1")],
        )
        .err()
        .expect("an unreadable neighbour is not proof of no collision");
        assert_eq!(code(&e), "GB-181");
    }
}

//! `goatbuilder federate rotate <partner> <key>`: replace one key of a running
//! peering and run the command again from the first phase it changes
//! (docs/design/federation-key-rotation.md §5).
//!
//! Built: `leaves`, the peering leaves (K3). Phase 8 runs the leaf helper with
//! `--rotate`, which sets the old pairs aside under `.previous-<random>/` and
//! keeps a `.rotation-pending` journal while it replaces them
//! (`ops/federation/standup/peering_leaf_material.py`); the run is phases 8 to
//! 14, phase 10 stops for a new approval because its inputs include the new
//! leaves, and phase 12 installs them. Nothing the partner holds changes, so
//! no `rotation.json` is written: phase 3 and our package are untouched.
//!
//! Designed, not built: `carrier`, `transport`, `signing` and `ca`.

use std::path::Path;

use anyhow::Result;

use super::run::Surface;
use super::{Options, Phase};

// Called from main.rs once its `rotate` stage word is wired (one line).
#[cfg_attr(not(test), allow(dead_code))]
/// The keys `rotate` names, and whether this build rotates each.
pub const KEYS: &[(&str, bool)] = &[
    ("leaves", true),
    ("carrier", false),
    ("transport", false),
    ("signing", false),
    ("ca", false),
];

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
/// as a `rotate` run, and carried on with the same command.
pub fn leaves_run<'a>(o: &Options<'a>) -> (Options<'a>, Surface) {
    let s = Surface {
        stage: "rotate",
        again: Some(format!("goatbuilder federate rotate {} leaves", o.with)),
        rotate_leaves: true,
        resume: false,
        ..Default::default()
    };
    (
        Options {
            from: 8,
            to: 14,
            ..*o
        },
        s,
    )
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
        Some((k, _)) => Err(goat_errors::coded(
            "GB-189",
            format!("`rotate {} {k}` is designed, not built (docs/design/federation-key-rotation.md §5): this goatbuilder rotates the leaves only", o.with),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: &anyhow::Error) -> &str {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.as_str())
            .unwrap_or_default()
    }

    #[test]
    fn only_the_leaves_rotate_and_every_other_key_says_it_is_not_built() {
        let o = Options {
            with: "xab",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some("/nonexistent/w"),
            mock: None,
            from: 1,
            to: 99,
            dry_run: true,
            table: false,
        };
        for (k, built) in KEYS.iter().filter(|(_, b)| !b) {
            assert!(!built);
            let e = rotate(Path::new("/nonexistent"), &o, k).unwrap_err();
            assert_eq!(code(&e), "GB-189");
            assert!(e.to_string().contains("designed, not built"), "{k}: {e}");
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

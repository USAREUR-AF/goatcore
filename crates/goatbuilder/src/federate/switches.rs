//! Who writes a peering's deployment halves (`peerings[].deployment`).
//!
//! Declared `rendered` or `hand`: as declared. Absent: `rendered`, unless the
//! peering already runs from hand-written files, which `allocate` never takes
//! over silently (the rule that it never moves a running peering). Such a
//! peering stays on its hand files and `allocate` says so, naming why; it is
//! opted in by declaring the switch `rendered`. A peering whose half
//! `allocate` already generated stays rendered.

use std::path::Path;

use super::alloc::PeeringValues;
use crate::config::Peering;

/// Whether one switch renders, and, when an absent switch was left on hand
/// files, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub rendered: bool,
    pub left_by_hand: Option<String>,
}

fn choose(declared: Option<&str>, hand_reason: impl FnOnce() -> Option<String>) -> Choice {
    match declared {
        Some("rendered") => Choice {
            rendered: true,
            left_by_hand: None,
        },
        Some(_) => Choice {
            rendered: false,
            left_by_hand: None,
        },
        None => match hand_reason() {
            Some(why) => Choice {
                rendered: false,
                left_by_hand: Some(why),
            },
            None => Choice {
                rendered: true,
                left_by_hand: None,
            },
        },
    }
}

/// Whether `rel` is a file `allocate` generated: a peering it already renders
/// stays rendered, whatever hand files appear beside it later.
fn generated(repo: &Path, rel: &str) -> bool {
    std::fs::read_to_string(repo.join(rel))
        .map(|t| t.starts_with(super::peering_files::MARKER))
        .unwrap_or(false)
}

/// `peerings[].deployment`.
pub fn deployment(repo: &Path, p: &Peering, v: &PeeringValues) -> Choice {
    choose(p.deployment.as_deref(), || {
        let edge_half = format!(
            "ops/ansible/group_vars/{}/peering-deployment.yml",
            v.edge_group
        );
        if generated(repo, &edge_half) {
            return None;
        }
        super::peering_files::hand_run_reason(repo, &v.edge_group)
    })
}

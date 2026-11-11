//! Phases 7, 8, 9 and 14 always name our party. Without `--as` it is read
//! from the declaration, as the help says, else from our half; mint and
//! acceptance otherwise take the agreement's first party, which is wrong for
//! the party that sorts second. Through the real binary against the stand-in
//! repository (federate_world).

mod federate_world;

use federate_world::{rows, says, world, World};

/// The commands of phases 7, 8, 9 and 14 in a dry run.
fn later_commands(w: &World, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["hello-ally", "--dry-run"];
    args.extend_from_slice(extra);
    let (out, v) = w.run(&args);
    assert!(out.status.success(), "{}", says(&out));
    let cmds: Vec<String> = rows(&v, "cmd").iter().map(|s| s.to_string()).collect();
    let pick = |needle: &str| {
        cmds.iter()
            .find(|c| c.contains(needle))
            .unwrap_or_else(|| panic!("no command with {needle}: {cmds:?}"))
            .clone()
    };
    vec![
        pick("federate verify"),
        pick("mint-peering-leaves.sh"),
        pick("federate apply"),
        pick("acceptance.sh"),
    ]
}

fn our_half(w: &World, id: &str) {
    std::fs::write(
        w.work.join("ours.canonical.json"),
        format!(r#"{{"sovereign_id":"{id}"}}"#),
    )
    .unwrap();
}

fn declaration(w: &World, id: &str) {
    let d = w.profile.join("contexts/default");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("federation.toml"),
        format!("sovereign_id = \"{id}\"\nterm_days = 30\ncut_contact = \"fed-ops@example.org\"\n"),
    )
    .unwrap();
}

/// Each party, first or second in the agreement, is named in every later
/// phase, read from our half when there is no declaration.
#[test]
fn both_parties_are_named_from_our_half() {
    for id in ["xaa", "xab"] {
        let w = world();
        our_half(&w, id);
        for c in later_commands(&w, &[]) {
            assert!(c.contains(&format!("--as '{id}'")), "{id}: {c}");
        }
    }
}

/// The declaration names us first; our half only when there is none.
#[test]
fn the_declaration_names_us_before_our_half() {
    let w = world();
    our_half(&w, "xaa");
    declaration(&w, "xab");
    for c in later_commands(&w, &[]) {
        assert!(c.contains("--as 'xab'"), "{c}");
    }
}

/// `--as` given on the command line is used as given.
#[test]
fn as_given_wins() {
    let w = world();
    declaration(&w, "xab");
    for c in later_commands(&w, &["--as", "xaa"]) {
        assert!(c.contains("--as 'xaa'") && !c.contains("xab'"), "{c}");
    }
}

/// With nothing that names us, the commands carry no --as, as before.
#[test]
fn nothing_names_us_and_nothing_is_guessed() {
    let w = world();
    for c in later_commands(&w, &[]) {
        assert!(!c.contains("--as"), "{c}");
    }
    our_half(&w, "Not An Id");
    for c in later_commands(&w, &[]) {
        assert!(!c.contains("--as"), "{c}");
    }
}

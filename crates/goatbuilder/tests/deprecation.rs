// A deprecated command or flag still works and says what replaces it
// (ADR 1228, "Deprecation"; ops/goatbuilder/compat/deprecations.yaml).
//
// The module is compiled here on its own, so its behaviour is proven before
// the binary calls it.
#[path = "../src/deprecation.rs"]
mod deprecation;

use clap::{Arg, ArgAction, Command};
use deprecation::{parse, used, Deprecation};

fn tree() -> Command {
    Command::new("goatbuilder")
        .arg(Arg::new("output").long("output").short('o').global(true))
        .subcommand(
            Command::new("deploy")
                .arg(Arg::new("env").long("env").short('e'))
                .arg(Arg::new("environment").long("environment"))
                .arg(
                    Arg::new("dry_run")
                        .long("dry-run")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("promote").subcommand(Command::new("graduate").arg(Arg::new("component"))),
        )
}

const LIST: &str = r#"
deprecations:
  - command: goatbuilder deploy
    flag: --env
    replacement: --environment
    deprecated_in: goatbuilder/v0.6.0
  - command: goatbuilder promote graduate
    replacement: goatbuilder promote hop
    deprecated_in: goatbuilder/v0.6.0
"#;

fn run(argv: &[&str]) -> Vec<Deprecation> {
    let deps = parse(LIST).unwrap();
    let m = tree().try_get_matches_from(argv).unwrap();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    used(&deps, &tree(), &m, &argv)
        .into_iter()
        .cloned()
        .collect()
}

#[test]
fn a_deprecated_flag_used_is_reported_with_its_replacement() {
    let u = run(&["goatbuilder", "deploy", "--env", "dev"]);
    assert_eq!(u.len(), 1, "{u:?}");
    assert_eq!(u[0].flag.as_deref(), Some("--env"));
    let w = u[0].warning("0.6.0");
    assert!(
        w.starts_with("warning: `goatbuilder deploy --env` is deprecated since goatbuilder/v0.6.0"),
        "{w}"
    );
    assert!(w.contains("use --environment instead"), "{w}");
}

#[test]
fn the_short_form_of_a_deprecated_flag_is_reported_too() {
    assert_eq!(run(&["goatbuilder", "deploy", "-e", "dev"]).len(), 1);
}

#[test]
fn a_deprecated_flag_not_used_is_not_reported() {
    assert!(run(&["goatbuilder", "deploy", "--environment", "dev", "--dry-run"]).is_empty());
}

#[test]
fn a_deprecated_command_used_is_reported() {
    let u = run(&["goatbuilder", "-o", "json", "promote", "graduate", "portal"]);
    assert_eq!(u.len(), 1, "{u:?}");
    assert!(u[0].flag.is_none());
    assert!(
        u[0].warning("1.2.0")
            .contains("removed no earlier than goatbuilder 2.0.0"),
        "{}",
        u[0].warning("1.2.0")
    );
}

#[test]
fn before_1_0_the_warning_names_no_removal_release() {
    let u = run(&["goatbuilder", "promote", "graduate", "portal"]);
    assert!(
        u[0].warning("0.6.0").contains("removed in a later release"),
        "{}",
        u[0].warning("0.6.0")
    );
}

#[test]
fn another_command_with_the_same_flag_is_not_reported() {
    let deps = parse("deprecations:\n  - {command: goatbuilder promote, flag: --env, replacement: --x, deprecated_in: goatbuilder/v0.6.0}\n").unwrap();
    let m = tree()
        .try_get_matches_from(["goatbuilder", "deploy", "--env", "dev"])
        .unwrap();
    let argv: Vec<String> = ["goatbuilder", "deploy", "--env", "dev"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(used(&deps, &tree(), &m, &argv).is_empty());
}

#[test]
fn the_json_form_names_the_command_the_flag_and_the_replacement() {
    let u = run(&["goatbuilder", "deploy", "--env", "dev"]);
    let j = serde_json::to_value(&u[0]).unwrap();
    assert_eq!(j["command"], "goatbuilder deploy");
    assert_eq!(j["flag"], "--env");
    assert_eq!(j["replacement"], "--environment");
    assert_eq!(j["deprecated_in"], "goatbuilder/v0.6.0");
}

#[test]
fn the_shipped_list_parses() {
    deprecation::shipped().unwrap();
}

#[test]
fn an_entry_without_a_replacement_is_refused() {
    assert!(parse(
        "deprecations:\n  - {command: goatbuilder deploy, deprecated_in: goatbuilder/v0.6.0}\n"
    )
    .is_err());
}

/// The usual way to rename a flag keeps the old name as a clap alias of the new
/// Arg. ArgMatches does not say which spelling was typed, so the command line
/// itself is read: the deprecated spelling, under the command the entry names.
fn aliased() -> Command {
    Command::new("goatbuilder")
        .arg(Arg::new("output").long("output").short('o').global(true))
        .subcommand(
            Command::new("deploy")
                .arg(Arg::new("environment").long("environment").alias("env"))
                .arg(
                    Arg::new("component")
                        .long("component")
                        .allow_hyphen_values(true),
                ),
        )
        .subcommand(Command::new("status").arg(Arg::new("env").long("env")))
}

fn run_aliased(argv: &[&str]) -> Vec<Deprecation> {
    let deps = parse(
        "deprecations:\n  - {command: goatbuilder deploy, flag: --env, replacement: --environment, deprecated_in: goatbuilder/v0.6.0}\n",
    )
    .unwrap();
    let m = aliased().try_get_matches_from(argv).unwrap();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    used(&deps, &aliased(), &m, &argv)
        .into_iter()
        .cloned()
        .collect()
}

#[test]
fn a_deprecated_flag_kept_as_an_alias_is_reported() {
    assert_eq!(
        run_aliased(&["goatbuilder", "deploy", "--env", "dev"]).len(),
        1
    );
    assert_eq!(
        run_aliased(&["goatbuilder", "-o", "json", "deploy", "--env=dev"]).len(),
        1
    );
}

#[test]
fn the_new_spelling_of_an_aliased_flag_is_not_reported() {
    assert!(run_aliased(&["goatbuilder", "deploy", "--environment", "dev"]).is_empty());
}

#[test]
fn a_value_that_looks_like_the_deprecated_flag_is_not_reported() {
    assert!(run_aliased(&["goatbuilder", "deploy", "--component", "--env"]).is_empty());
}

#[test]
fn the_same_spelling_under_another_command_is_not_reported() {
    assert!(run_aliased(&["goatbuilder", "status", "--env", "dev"]).is_empty());
}

#[test]
fn a_command_or_flag_that_is_not_a_whole_name_is_refused() {
    for bad in [
        "deprecations:\n  - {command: goatbuilderx, replacement: x, deprecated_in: goatbuilder/v0.6.0}\n",
        "deprecations:\n  - {command: goatbuilder deploy, flag: '-', replacement: x, deprecated_in: goatbuilder/v0.6.0}\n",
        "deprecations:\n  - {command: goatbuilder deploy, flag: '--', replacement: x, deprecated_in: goatbuilder/v0.6.0}\n",
        "deprecations:\n  - {command: goatbuilder deploy, flag: '--env x', replacement: x, deprecated_in: goatbuilder/v0.6.0}\n",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
    assert!(parse("deprecations:\n  - {command: goatbuilder, flag: -o, replacement: --output, deprecated_in: goatbuilder/v0.6.0}\n").is_ok());
}

/// A short alias with its value attached (`-Ex`), and a flag that takes several
/// values (`--tag a b`), read the way clap reads them.
fn shorts() -> Command {
    Command::new("goatbuilder").subcommand(
        Command::new("deploy")
            .arg(
                Arg::new("environment")
                    .long("environment")
                    .short('e')
                    .short_alias('E'),
            )
            .arg(
                Arg::new("tag")
                    .long("tag")
                    .num_args(1..)
                    .allow_hyphen_values(true),
            )
            .arg(Arg::new("verbose").short('v').action(ArgAction::SetTrue))
            .arg(Arg::new("old").long("old").action(ArgAction::SetTrue)),
    )
}

fn run_shorts(list: &str, argv: &[&str]) -> Vec<Deprecation> {
    let deps = parse(list).unwrap();
    let m = shorts().try_get_matches_from(argv).unwrap();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    used(&deps, &shorts(), &m, &argv)
        .into_iter()
        .cloned()
        .collect()
}

const SHORT_E: &str = "deprecations:\n  - {command: goatbuilder deploy, flag: -E, replacement: -e, deprecated_in: goatbuilder/v0.6.0}\n";
const OLD: &str = "deprecations:\n  - {command: goatbuilder deploy, flag: --old, replacement: --new, deprecated_in: goatbuilder/v0.6.0}\n";

#[test]
fn a_short_alias_with_its_value_attached_is_reported() {
    assert_eq!(
        run_shorts(SHORT_E, &["goatbuilder", "deploy", "-Edev"]).len(),
        1
    );
    assert_eq!(
        run_shorts(SHORT_E, &["goatbuilder", "deploy", "-E", "dev"]).len(),
        1
    );
    assert_eq!(
        run_shorts(SHORT_E, &["goatbuilder", "deploy", "-vEdev"]).len(),
        1
    );
    assert!(run_shorts(SHORT_E, &["goatbuilder", "deploy", "-edev"]).is_empty());
}

#[test]
fn every_value_of_a_many_valued_flag_is_skipped() {
    // `--tag` takes several values, and they may look like flags: `--old` here is
    // a value of `--tag`, not the deprecated flag.
    let m = shorts()
        .try_get_matches_from(["goatbuilder", "deploy", "--tag", "a", "--old"])
        .unwrap();
    assert_eq!(
        m.subcommand_matches("deploy")
            .unwrap()
            .get_many::<String>("tag")
            .unwrap()
            .count(),
        2,
        "clap reads --old as the second value of --tag"
    );
    assert!(run_shorts(OLD, &["goatbuilder", "deploy", "--tag", "a", "--old"]).is_empty());
    assert_eq!(
        run_shorts(OLD, &["goatbuilder", "deploy", "--old", "--tag", "a", "b"]).len(),
        1
    );
}

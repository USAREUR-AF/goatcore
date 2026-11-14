//! `goatbuilder docs` — the reference entry point and the anti-drift command
//! reference generator, as goat has (crates/goat-cli/src/cmd/docs.rs). Bare
//! `goatbuilder docs` points at the reference chapters; `goatbuilder docs
//! --markdown` emits the full command reference as Markdown, walked from the
//! live clap command tree, so it can NEVER drift from `goatbuilder --help`.
//!
//! The committed copy is `docs/reference/goatbuilder/command-reference.md`, and
//! a CI gate regenerates it and refuses a diff (`.github/workflows/goatbuilder-pr.yml`),
//! so a new verb that is not re-rendered fails the PR.

use std::fmt::Write as _;

/// The committed reference, relative to the repository root. The CI gate and a
/// test both name it here so there is one spelling.
pub const REFERENCE_PATH: &str = "docs/reference/goatbuilder/command-reference.md";

pub fn run(markdown: bool) -> anyhow::Result<()> {
    if !markdown {
        println!("goatbuilder docs — the build/release command-line reference.");
        println!();
        println!("  In the repo:  docs/reference/goatbuilder/   (start at README.md)");
        println!("  Reference:    {REFERENCE_PATH}   (or `goatbuilder docs --markdown`)");
        return Ok(());
    }
    // Render from the ACTIVE-LOCALE command tree: `select_language` ran in main
    // before dispatch, so with no `--lang` this stays English-canonical (the
    // committed reference the CI gate diffs); `--lang fr docs --markdown` emits
    // the French reference from the same already-translated help catalogs.
    let out = reference();
    // Write directly, not `print!`, so `goatbuilder docs --markdown | head`
    // closing the pipe early exits cleanly instead of panicking.
    use std::io::Write;
    if let Err(e) = std::io::stdout().write_all(out.as_bytes()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(e.into());
    }
    Ok(())
}

/// The full command reference as Markdown, from the live (localized) clap tree.
pub fn reference() -> String {
    use clap::CommandFactory;
    let cmd = crate::i18n::localize_command(crate::Cli::command());
    let mut out = String::new();
    out.push_str("# goatbuilder — command reference\n\n");
    out.push_str(
        "_Generated from the `goatbuilder` command tree — always in sync with what the binary does. Regenerate with `goatbuilder docs --markdown`._\n\n",
    );
    render(&cmd, "goatbuilder", &mut out, None);
    out
}

/// Recursively emit one command (and its visible subcommands) as Markdown.
/// Hidden commands/flags (the easter eggs, `_complete`) are skipped; a global
/// flag is documented once, on the command that declares it (the root's on the
/// root, `federate`'s measure flags on `federate`), not repeated per command.
fn render(cmd: &clap::Command, path: &str, out: &mut String, parent: Option<&clap::Command>) {
    let _ = writeln!(out, "## `{path}`\n");

    if let Some(about) = cmd.get_long_about().or_else(|| cmd.get_about()) {
        let _ = writeln!(out, "{}\n", about.to_string().trim());
    }

    let args: Vec<&clap::Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        // A global flag the parent carries is the parent's: documented there.
        .filter(|a| !a.is_global_set() || parent.is_none_or(|p| p.get_arguments().all(|pa| pa.get_id() != a.get_id())))
        .collect();

    let positionals: Vec<&&clap::Arg> = args.iter().filter(|a| a.is_positional()).collect();
    let flags: Vec<&&clap::Arg> = args.iter().filter(|a| !a.is_positional()).collect();

    if !positionals.is_empty() {
        let _ = writeln!(out, "**Arguments**\n");
        let _ = writeln!(out, "| Argument | Description |");
        let _ = writeln!(out, "|---|---|");
        for a in &positionals {
            let name = a.get_id().as_str().to_uppercase();
            let help = a.get_help().map(|s| s.to_string()).unwrap_or_default();
            let _ = writeln!(out, "| `<{name}>` | {} |", oneline(&help));
        }
        out.push('\n');
    }

    if !flags.is_empty() {
        let _ = writeln!(out, "**Options**\n");
        let _ = writeln!(out, "| Flag | Description | Default |");
        let _ = writeln!(out, "|---|---|---|");
        for a in &flags {
            let mut disp = String::new();
            if let Some(long) = a.get_long() {
                disp.push_str(&format!("--{long}"));
            }
            if let Some(short) = a.get_short() {
                if !disp.is_empty() {
                    disp.push_str(", ");
                }
                disp.push_str(&format!("-{short}"));
            }
            if a.get_action().takes_values() {
                let _ = write!(disp, " <{}>", a.get_id().as_str().to_uppercase());
            }
            let help = a.get_help().map(|s| s.to_string()).unwrap_or_default();
            let defs: Vec<String> = a
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy().to_string())
                .collect();
            let def = if defs.is_empty() { String::new() } else { format!("`{}`", defs.join(", ")) };
            let _ = writeln!(out, "| `{disp}` | {} | {def} |", oneline(&help));
        }
        out.push('\n');
    }

    let subs: Vec<&clap::Command> = cmd.get_subcommands().filter(|c| !c.is_hide_set()).collect();

    if !subs.is_empty() {
        let _ = writeln!(out, "**Subcommands**\n");
        for c in &subs {
            let about = c.get_about().map(|s| oneline(&s.to_string())).unwrap_or_default();
            let _ = writeln!(out, "- [`{} {}`](#{}) — {about}", path, c.get_name(), anchor(path, c.get_name()));
        }
        out.push('\n');
    }

    for c in &subs {
        render(c, &format!("{path} {}", c.get_name()), out, Some(cmd));
    }
}

/// Collapse a help string to a single Markdown-table-safe line.
fn oneline(s: &str) -> String {
    s.replace('\n', " ").replace('|', "\\|").trim().to_string()
}

/// mdBook-style heading anchor for an intra-page link.
fn anchor(path: &str, name: &str) -> String {
    format!("{path} {name}").to_lowercase().replace(' ', "-").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_covers_visible_verbs_and_the_new_parity_surfaces() {
        let out = reference();
        assert!(out.contains("## `goatbuilder`"));
        assert!(out.contains("## `goatbuilder doctor`"));
        assert!(out.contains("## `goatbuilder deploy`"));
        // The parity verbs this track adds.
        assert!(out.contains("## `goatbuilder completion`"));
        assert!(out.contains("## `goatbuilder docs`"));
        // The global flags are documented (once, on the root).
        assert!(out.contains("--color"));
        assert!(out.contains("--verbose"));
        assert!(out.contains("--timeout"));
    }

    /// A global flag is documented on the command that declares it, once: the
    /// root's on the root, and federate's measure flags (declared global on
    /// `federate`, so every federate verb takes them) on `federate`.
    #[test]
    fn a_global_flag_is_documented_where_it_is_declared() {
        let out = reference();
        let section = |heading: &str| -> String {
            let start = out.find(&format!("## `{heading}`\n")).unwrap_or_else(|| panic!("no section {heading}"));
            let rest = &out[start + 1..];
            rest[..rest.find("\n## `").unwrap_or(rest.len())].to_string()
        };
        let federate = section("goatbuilder federate");
        for flag in ["--side", "--half-out", "--edge-half", "--over-ssh", "--router-container"] {
            assert!(federate.contains(&format!("`{flag} ")), "{flag} is not documented on goatbuilder federate");
            assert_eq!(out.matches(&format!("| `{flag} ")).count(), 1, "{flag} is documented more than once");
        }
        for flag in ["--color", "--verbose", "--timeout"] {
            assert_eq!(out.matches(&format!("| `{flag}")).count(), 1, "{flag} is documented more than once");
        }
    }

    #[test]
    fn hidden_surfaces_never_appear() {
        let out = reference();
        // No heading for the hidden helper or the eggs (the string `goatbuilder
        // _complete` still appears in the `completion` help, which is correct —
        // it tells an operator how to wire the hook).
        assert!(!out.contains("## `goatbuilder _complete`"), "the dynamic-candidate helper is hidden");
        assert!(!out.contains("## `goatbuilder make`"), "the eggs are hidden");
        assert!(!out.contains("## `goatbuilder argan`"));
    }

    #[test]
    fn a_global_flag_is_documented_once_not_per_subcommand() {
        let out = reference();
        assert_eq!(out.matches("`--color <COLOR>`").count(), 1, "global flag repeated");
    }

    // The committed reference must be byte-identical to the live render. This
    // is the drift gate's Rust half: `cargo test` catches a stale reference even
    // off CI. Regenerate with `goatbuilder docs --markdown > <REFERENCE_PATH>`.
    #[test]
    fn the_committed_reference_is_in_sync_with_the_command_tree() {
        // CARGO_MANIFEST_DIR is crates/goatbuilder; the reference is repo-root-relative.
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = repo.join(REFERENCE_PATH);
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(
            committed,
            reference(),
            "{REFERENCE_PATH} is stale — regenerate: goatbuilder docs --markdown > {REFERENCE_PATH}"
        );
    }
}

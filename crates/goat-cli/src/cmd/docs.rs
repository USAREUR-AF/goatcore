// `goat docs` — the docs entry point + the anti-drift reference generator
// (docs/design/goat-cli-hosted-docs.md). Bare `goat docs` points at the
// hosted `docs.nova` site; `goat docs --markdown` emits the full command
// reference as Markdown, walked from the live clap command tree so it can
// NEVER drift from `goat --help`. The hosted-docs build renders this as
// its reference chapter from the same tagged binary it serves.

use clap::Args;
use std::fmt::Write as _;

#[derive(Args, Debug)]
pub struct DocsArgs {
    /// Emit the full command reference as Markdown (for the hosted docs
    /// site or offline reading). Generated from the live command tree, so
    /// it always matches `goat --help`.
    #[arg(long)]
    pub markdown: bool,
}

pub fn run(args: DocsArgs) -> anyhow::Result<()> {
    use clap::CommandFactory;
    if !args.markdown {
        println!("goat docs — browsable, searchable docs for the goat CLI.");
        println!();
        println!("  On the mesh:  https://docs.<site>.<your-zone>/");
        println!("  Offline:      goat docs --markdown    # the full command reference");
        return Ok(());
    }
    // Render from the ACTIVE-LOCALE command tree (ADR 1024): `select_language`
    // ran in `main` before dispatch, so `localize_command` swaps the root
    // description, global flags, and each subcommand one-liner to the active
    // language's catalog. With no `--lang` the loader is English, so
    // `goat docs --markdown` stays English-canonical; `goat --lang fr docs
    // --markdown` emits the French reference — reusing the already-translated
    // help catalogs so the hosted docs site gets a per-language reference for
    // free (docs/design/goat-cli-hosted-docs.md "localized reference").
    let cmd = crate::i18n::localize_command(crate::Cli::command());
    let mut out = String::new();
    out.push_str("# goat — command reference\n\n");
    out.push_str(
        "_Generated from the `goat` command tree — always in sync with what the binary does._\n\n",
    );
    render(&cmd, "goat", &mut out, true);
    // `print!` panics on a write failure (it has no error-return path) —
    // the common failure here is `goat docs --markdown | head` closing the
    // pipe early (BrokenPipe), which is a normal, expected shell action, not
    // a bug. Write directly so that case exits cleanly instead of panicking
    // with a Rust backtrace over what the user did on purpose.
    use std::io::Write;
    if let Err(e) = std::io::stdout().write_all(out.as_bytes()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(e.into());
    }
    Ok(())
}

/// Recursively emit one command (and its visible subcommands) as Markdown.
/// Hidden commands/flags (the easter eggs, `_complete`) are skipped;
/// global flags are documented once on the root, not repeated per command.
fn render(cmd: &clap::Command, path: &str, out: &mut String, is_root: bool) {
    let _ = writeln!(out, "## `{path}`\n");

    if let Some(about) = cmd.get_long_about().or_else(|| cmd.get_about()) {
        let _ = writeln!(out, "{}\n", oneblock(&about.to_string()));
    }

    let args: Vec<&clap::Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        // Global flags are documented once, on the root.
        .filter(|a| is_root || !a.is_global_set())
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
            let def = if defs.is_empty() {
                String::new()
            } else {
                format!("`{}`", defs.join(", "))
            };
            let _ = writeln!(out, "| `{disp}` | {} | {def} |", oneline(&help));
        }
        out.push('\n');
    }

    let subs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .collect();

    if !subs.is_empty() {
        let _ = writeln!(out, "**Subcommands**\n");
        for c in &subs {
            let about = c
                .get_about()
                .map(|s| oneline(&s.to_string()))
                .unwrap_or_default();
            let _ = writeln!(out, "- [`{} {}`](#{}) — {about}", path, c.get_name(), anchor(path, c.get_name()));
        }
        out.push('\n');
    }

    for c in &subs {
        render(c, &format!("{path} {}", c.get_name()), out, false);
    }
}

/// Collapse a help string to a single Markdown-table-safe line.
fn oneline(s: &str) -> String {
    s.replace('\n', " ").replace('|', "\\|").trim().to_string()
}

/// Normalize a multi-line about block: keep paragraphs, drop hard-wrapped
/// single newlines so it reflows in the rendered page.
fn oneblock(s: &str) -> String {
    s.trim().to_string()
}

/// mdBook-style heading anchor for an intra-page link.
fn anchor(path: &str, name: &str) -> String {
    format!("{path} {name}")
        .to_lowercase()
        .replace(' ', "-")
        .replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn markdown_reference_covers_visible_commands_and_hides_eggs() {
        let cmd = crate::Cli::command();
        let mut out = String::new();
        render(&cmd, "goat", &mut out, true);
        // Documents the real surfaces.
        assert!(out.contains("## `goat`"));
        assert!(out.contains("## `goat hello`"));
        assert!(out.contains("## `goat doctor`"));
        assert!(out.contains("## `goat tutor`"));
        // The global flags are documented (once, on the root).
        assert!(out.contains("--lang"));
        // Hidden easter eggs + machine surfaces never appear.
        assert!(!out.contains("## `goat goat`"));
        assert!(!out.contains("xyzzy"));
        assert!(!out.contains("brew"));
        assert!(!out.contains("_complete"));
    }

    #[test]
    fn global_flags_documented_once_not_per_subcommand() {
        let cmd = crate::Cli::command();
        let mut out = String::new();
        render(&cmd, "goat", &mut out, true);
        // `--lang` is global — it must appear exactly once (on the root),
        // not repeated under every subcommand's Options table.
        assert_eq!(out.matches("`--lang <LANG>`").count(), 1, "global flag repeated");
    }
}

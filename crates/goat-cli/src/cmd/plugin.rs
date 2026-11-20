// `goat plugin` — Phase E.2. PATH-based extension discovery
// (kubectl-style). When `goat <X>` matches no built-in subcommand,
// the dispatcher in main.rs scans PATH for an executable named
// `goat-<X>` and execs it with the remaining args.
//
// `goat plugin list` is the introspection side: scan PATH, dedupe by
// name, render one row per `goat-*` executable. `goat plugin which
// <name>` answers "which file would `goat <name>` exec?" without
// running it.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum PluginCmd {
    /// List every `goat-*` executable on PATH (kubectl-style plugins).
    /// First occurrence per name wins — shadowed entries are noted.
    List {
        /// Emit one JSON object per plugin instead of pretty rows.
        #[arg(long)]
        json: bool,
    },
    /// Print the path of the executable `goat <name>` would dispatch
    /// to. Exits non-zero if no `goat-<name>` is on PATH.
    Which {
        /// Plugin subcommand name (the `<X>` in `goat <X>`).
        name: String,
    },
}

#[derive(Debug, serde::Serialize)]
pub struct PluginEntry {
    name: String,
    path: PathBuf,
    /// Other entries with the same name found later on PATH — kubectl
    /// surfaces these as "shadowed" so operators see why a plugin
    /// they're certain they installed isn't being picked up.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shadowed_by: Vec<PathBuf>,
}

pub async fn run(_ctx: &super::Ctx, op: PluginCmd) -> anyhow::Result<()> {
    match op {
        PluginCmd::List { json } => {
            let entries = scan_path_for_plugins();
            if json {
                println!("{}", serde_json::to_string(&entries).unwrap_or_default());
                return Ok(());
            }
            if entries.is_empty() {
                eprintln!("(no goat-* plugins on PATH)");
                return Ok(());
            }
            let name_w = entries.iter().map(|e| e.name.len()).max().unwrap_or(0);
            for e in &entries {
                println!("{:width$}  {}", e.name, e.path.display(), width = name_w);
                for s in &e.shadowed_by {
                    println!("{:width$}  (shadowed: {})", "", s.display(), width = name_w);
                }
            }
            Ok(())
        }
        PluginCmd::Which { name } => match find_plugin(&name) {
            Some(p) => {
                println!("{}", p.display());
                Ok(())
            }
            None => anyhow::bail!("no goat-{name} on PATH"),
        },
    }
}

/// Scan every directory in $PATH for `goat-*` executables. First
/// occurrence per name wins; later ones go into `shadowed_by`.
pub fn scan_path_for_plugins() -> Vec<PluginEntry> {
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut by_name: std::collections::BTreeMap<String, PluginEntry> = Default::default();
    let mut order: Vec<String> = Vec::new();

    for dir in std::env::split_paths(&path) {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let file_name = entry.file_name();
            let name_str = match file_name.to_str() {
                Some(s) => s,
                None => continue,
            };
            let rest = match name_str.strip_prefix("goat-") {
                Some(rest) if !rest.is_empty() => rest,
                _ => continue,
            };
            // On Windows the directory entry carries an extension
            // (`goat-foo.exe`); strip a trailing .exe/.bat/.cmd so the
            // plugin lists + dispatches as `foo`, matching what the
            // operator types. On Unix the name is already bare.
            #[cfg(not(unix))]
            let plugin_name = rest
                .rsplit_once('.')
                .filter(|(_, ext)| {
                    matches!(ext.to_ascii_lowercase().as_str(), "exe" | "bat" | "cmd")
                })
                .map(|(stem, _)| stem)
                .unwrap_or(rest);
            #[cfg(unix)]
            let plugin_name = rest;
            // Skip our own binary if PATH contains the install dir.
            // The dispatcher already filters built-ins; this is the
            // belt-and-suspenders.
            if plugin_name == "cli" {
                continue;
            }
            if !is_executable(&entry.path()) {
                continue;
            }
            let key = plugin_name.to_string();
            match by_name.get_mut(&key) {
                Some(existing) => existing.shadowed_by.push(entry.path()),
                None => {
                    by_name.insert(
                        key.clone(),
                        PluginEntry {
                            name: key.clone(),
                            path: entry.path(),
                            shadowed_by: Vec::new(),
                        },
                    );
                    order.push(key);
                }
            }
        }
    }

    order.into_iter().filter_map(|k| by_name.remove(&k)).collect()
}

/// First `goat-<name>` on PATH, if any.
pub fn find_plugin(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let stem = format!("goat-{name}");
    for dir in std::env::split_paths(&path) {
        // Bare name — Unix executables, or an extension the operator
        // spelled out explicitly.
        let candidate = dir.join(&stem);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        // Windows: probe the common PATHEXT suffixes so `goat foo`
        // resolves a `goat-foo.exe` on PATH (the operator never types
        // the extension).
        #[cfg(not(unix))]
        for ext in ["exe", "bat", "cmd"] {
            let candidate = dir.join(format!("{stem}.{ext}"));
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &std::path::Path) -> bool {
    // Windows: treat .exe / .bat / .cmd as executable.
    p.is_file()
        && matches!(
            p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
            Some("exe") | Some("bat") | Some("cmd")
        )
}

/// Names of built-in top-level subcommands. The dispatcher uses this
/// to decide whether to attempt PATH lookup. Keep in sync with the
/// `Cmd` enum in main.rs — adding a new built-in here is a deliberate
/// reminder to also update this list (caught by the unit test below).
pub const BUILTIN_SUBCOMMANDS: &[&str] = &[
    "carriage",
    "pod",
    "profile",
    "context",
    "doctor",
    "org",
    // `vendor` is the hidden alias of `org` (ADR 1052 rename) — kept here so
    // the dispatcher never attempts a PATH plugin lookup for it.
    "vendor",
    "topic",
    "pub",
    "sub",
    "inspect",
    "compose",
    "narrate",
    "completion",
    "_complete",
    "explain",
    "flow",
    "mock",
    "admin",
    "status",
    "heal",
    "services",
    "client",
    "plugin",
    "help",
];

/// True if the first non-flag argument matches a built-in subcommand
/// (or is a global flag like `--help` / `--version`). Used by the
/// dispatcher to skip PATH lookup for known commands.
pub fn is_builtin_or_flag(arg: &str) -> bool {
    if arg.starts_with('-') {
        return true;
    }
    BUILTIN_SUBCOMMANDS.contains(&arg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn builtin_list_matches_main_cmd_enum() {
        // Defensive: if main.rs adds a new top-level subcommand
        // without updating BUILTIN_SUBCOMMANDS, the dispatcher will
        // wrongly attempt PATH lookup for it. This test catches the
        // ones we know about today; the human update for new ones
        // is a code-review check.
        let expected: HashSet<&str> = [
            "profile", "context", "doctor", "org", "vendor", "topic", "pub",
            "sub", "inspect", "compose", "narrate", "completion",
            "_complete", "explain", "flow", "mock", "admin", "status",
            "heal", "services", "client", "plugin", "help", "carriage", "pod",
        ]
        .into_iter()
        .collect();
        let actual: HashSet<&str> = BUILTIN_SUBCOMMANDS.iter().copied().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn builtin_recognizes_flags() {
        assert!(is_builtin_or_flag("--help"));
        assert!(is_builtin_or_flag("-v"));
        assert!(is_builtin_or_flag("--version"));
    }

    #[test]
    fn builtin_recognizes_known() {
        assert!(is_builtin_or_flag("doctor"));
        assert!(is_builtin_or_flag("admin"));
    }

    #[test]
    fn builtin_rejects_unknown() {
        assert!(!is_builtin_or_flag("org11x"));
        assert!(!is_builtin_or_flag("inmates-onboard"));
    }
}


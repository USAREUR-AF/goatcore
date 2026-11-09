//! `goatbuilder federate init|lint|compose|diff|verify|apply`: the six
//! federation verbs, run in process from the `goat-federation` library, with
//! the same arguments, output and exit codes as `goat federate <verb>`
//! (`goat_federation::verbs`).
//!
//! What goatbuilder supplies that the library cannot know is where this
//! party's own state is: the declaration (`federation.toml`), the mTLS
//! certificate and the CA roots. goatbuilder has no profile of its own, so
//! these come from, in order:
//!
//!   1. `--context-dir DIR` (or `$GOAT_CONTEXT_DIR`): the directory that holds
//!      `federation.toml`, `mtls.cert.pem` and `ca-roots.pem`;
//!   2. otherwise the goat context directory, found the way `goat` finds it:
//!      `$GOAT_PROFILE_DIR` (else `~/.goat`), then `contexts/<name>` where the
//!      name is `$GOAT_CONTEXT`, else `current_context` in `config.toml`, else
//!      `default`;
//!
//! and `--cert FILE` / `--ca-roots FILE` override the two files inside it.
//! Only `init` reads all three; `verify` and `apply` read the declaration only
//! when `--as` is absent; lint, compose and diff read none of it. When none of
//! these resolves, the verb stops with GB-100 and names what to supply.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use goat_federation::verbs::{self, FederateCmd, Site};

/// Where this party's own state is, for the verbs that read it.
#[derive(Args, Debug, Clone, Default)]
pub struct SiteArgs {
    /// The six verbs: the directory holding federation.toml, mtls.cert.pem and
    /// ca-roots.pem (default: $GOAT_CONTEXT_DIR, else the goat context
    /// directory, found the way goat finds it).
    #[arg(long, value_name = "DIR", global = true)]
    pub context_dir: Option<PathBuf>,
    /// The six verbs: our mTLS certificate (default: mtls.cert.pem in the
    /// context directory).
    #[arg(long, value_name = "FILE", global = true)]
    pub cert: Option<PathBuf>,
    /// The six verbs: the CA roots (default: ca-roots.pem in the context
    /// directory).
    #[arg(long, value_name = "FILE", global = true)]
    pub ca_roots: Option<PathBuf>,
}

impl SiteArgs {
    pub fn is_empty(&self) -> bool {
        self.context_dir.is_none() && self.cert.is_none() && self.ca_roots.is_none()
    }
}

/// The file names inside a goat context directory (crates/goat-cli/src/profile.rs).
const CERT_FILE: &str = "mtls.cert.pem";
const CA_ROOTS_FILE: &str = "ca-roots.pem";

/// The site the verbs read, from the flags, else the goat context directory.
pub fn resolve_site(a: &SiteArgs) -> Result<Site> {
    let explicit = a.context_dir.clone().filter(|p| !p.as_os_str().is_empty());
    let context_dir =
        match explicit.or_else(|| non_empty_env("GOAT_CONTEXT_DIR").map(PathBuf::from)) {
            Some(d) => d,
            None => goat_context_dir()?,
        };
    Ok(Site {
        cert_path: a
            .cert
            .clone()
            .unwrap_or_else(|| context_dir.join(CERT_FILE)),
        ca_roots_path: a
            .ca_roots
            .clone()
            .unwrap_or_else(|| context_dir.join(CA_ROOTS_FILE)),
        context_dir,
    })
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The goat context directory, resolved the way `goat` resolves it
/// (crates/goat-cli/src/profile.rs `Profile::resolve`).
fn goat_context_dir() -> Result<PathBuf> {
    let profile =
        match non_empty_env("GOAT_PROFILE_DIR") {
            Some(p) => PathBuf::from(p),
            None => match non_empty_env("HOME") {
                Some(h) => Path::new(&h).join(".goat"),
                None => return Err(goat_errors::coded(
                    "GB-100",
                    "no context directory for the federation verbs: pass --context-dir <dir> (the \
                     directory holding federation.toml, mtls.cert.pem and ca-roots.pem), or set \
                     GOAT_CONTEXT_DIR, or GOAT_PROFILE_DIR for a goat profile",
                )
                .into()),
            },
        };
    let name = non_empty_env("GOAT_CONTEXT")
        .or_else(|| current_context(&profile.join("config.toml")))
        .unwrap_or_else(|| "default".to_string());
    Ok(profile.join("contexts").join(name))
}

/// `current_context = "<name>"` from a goat `config.toml`, when it says one.
fn current_context(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("current_context")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Run one verb, its report on `out`. Returns the verb's exit code.
pub fn run_to(cmd: FederateCmd, site: &SiteArgs, out: &mut dyn Write) -> Result<i32> {
    let resolve = || resolve_site(site);
    verbs::run(cmd, &resolve, out)
}

/// Run one verb, its report on stdout.
pub fn run(cmd: FederateCmd, site: &SiteArgs) -> Result<i32> {
    let mut out = std::io::stdout().lock();
    run_to(cmd, site, &mut out)
}

/// `goatbuilder federate <verb>` from the command line: run it, and turn a
/// fault into goatbuilder's refusal line. The exit code is the verb's own
/// (0, or 20 to 24 as `goat federate` exits); a fault exits 1, as it does in
/// goat, and a coded refusal (GB-100) exits with its catalog code.
pub fn exit_of(cmd: FederateCmd, site: &SiteArgs) -> i32 {
    match run(cmd, site) {
        Ok(code) => code,
        Err(e) => fault_exit(&e),
    }
}

/// Print a verb's fault as goatbuilder's refusal line and return its exit
/// code: a coded refusal (GB-100) exits with its catalog code, anything else 1.
pub fn fault_exit(e: &anyhow::Error) -> i32 {
    eprintln!("goatbuilder: {e:#}");
    match e.downcast_ref::<goat_errors::Coded>() {
        Some(c) => {
            eprintln!("  ({} — goatbuilder explain {})", c.code, c.code);
            goat_errors::explain(&c.code)
                .and_then(|x| x.exit)
                .unwrap_or(2)
        }
        None => 1,
    }
}

/// One verb's command line after `goatbuilder federate`, as the tests write it.
#[cfg(test)]
#[derive(clap::Parser, Debug)]
#[command(name = "goatbuilder federate", no_binary_name = true)]
struct VerbLine {
    #[command(flatten)]
    site: SiteArgs,
    #[command(subcommand)]
    cmd: FederateCmd,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use clap::Parser;

    fn vectors() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors")
    }

    fn line(words: &[&str]) -> VerbLine {
        VerbLine::try_parse_from(words.iter().map(|s| s.to_string())).unwrap()
    }

    fn capture(words: &[&str]) -> (i32, String) {
        let l = line(words);
        let mut out = Vec::new();
        let code = run_to(l.cmd, &l.site, &mut out).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    fn keys() -> Vec<String> {
        ["xaa", "xab"]
            .iter()
            .map(|p| {
                let k = std::fs::read_to_string(vectors().join(format!("testkeys/{p}.public.b64")))
                    .unwrap();
                format!("{p}={}", k.trim())
            })
            .collect()
    }

    fn now() -> String {
        std::fs::read_to_string(vectors().join("now.txt"))
            .unwrap()
            .trim()
            .to_string()
    }

    /// The two packages of a vector pair (a refusal pair may name its second
    /// package differently, as `refuse-same-sovereign-id` does).
    fn pair(dir: &Path) -> Vec<String> {
        let mut p: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.starts_with("package-")
                    && n.ends_with(".canonical.json")
                    && !n.contains("signing-input")
            })
            .map(|p| p.display().to_string())
            .collect();
        p.sort();
        assert_eq!(p.len(), 2, "{}: {p:?}", dir.display());
        p
    }

    fn compose_words<'a>(
        dir: &'a str,
        keys: &'a [String],
        now: &'a str,
        extra: &[&'a str],
    ) -> Vec<String> {
        let mut w = vec!["compose".to_string()];
        w.extend(pair(&vectors().join(dir)));
        w.extend(["--now".to_string(), now.to_string()]);
        for k in keys {
            w.extend(["--key".to_string(), k.clone()]);
        }
        w.extend(extra.iter().map(|s| s.to_string()));
        w
    }

    /// Every composing vector composes to its normative digest, in process.
    #[test]
    fn compose_reaches_every_vectors_digest() {
        let (keys, now) = (keys(), now());
        let mut seen = 0;
        for e in std::fs::read_dir(vectors()).unwrap() {
            let dir = e.unwrap().file_name().to_string_lossy().into_owned();
            if !dir.starts_with("compose-") {
                continue;
            }
            let want =
                std::fs::read_to_string(vectors().join(&dir).join("agreement.digest.txt")).unwrap();
            let w = compose_words(&dir, &keys, &now, &["--json"]);
            let (code, out) = capture(&w.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(code, verbs::EXIT_OK, "{dir}: {out}");
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["digest"].as_str().unwrap(), want.trim(), "{dir}");
            seen += 1;
        }
        assert!(seen >= 10, "found only {seen} composing vectors");
    }

    /// Every refusing vector refuses with exit 20, and names the rule.
    #[test]
    fn compose_refuses_every_refusal_vector_with_exit_20() {
        let (keys, now) = (keys(), now());
        let mut seen = 0;
        for e in std::fs::read_dir(vectors()).unwrap() {
            let dir = e.unwrap().file_name().to_string_lossy().into_owned();
            if !dir.starts_with("refuse-") {
                continue;
            }
            let w = compose_words(&dir, &keys, &now, &["--json"]);
            let (code, out) = capture(&w.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(code, verbs::EXIT_FED_REFUSED, "{dir}: {out}");
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert!(
                v["rule"].as_str().is_some_and(|r| !r.is_empty()),
                "{dir}: {out}"
            );
            seen += 1;
        }
        assert!(seen >= 9, "found only {seen} refusal vectors");
    }

    /// lint passes each composing vector's packages and refuses every malformed
    /// one with exit 21.
    #[test]
    fn lint_passes_good_packages_and_refuses_malformed_ones() {
        let v = vectors();
        let good = v
            .join("compose-full/package-xaa.canonical.json")
            .display()
            .to_string();
        let (code, out) = capture(&["lint", &good]);
        assert_eq!(code, verbs::EXIT_OK, "{out}");
        assert!(
            out.starts_with("package ") && out.contains("[✓] well-formed"),
            "{out}"
        );
        let mut seen = 0;
        for e in std::fs::read_dir(v.join("malformed")).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let s = p.display().to_string();
            let (code, out) = capture(&["lint", "--json", &s]);
            assert_eq!(code, verbs::EXIT_FED_MALFORMED, "{s}: {out}");
            seen += 1;
        }
        assert!(seen >= 10, "found only {seen} malformed packages");
    }

    /// diff: identical agreements match (0); different ones name fields (22);
    /// a pretty-printed copy of the same agreement differs in bytes only.
    #[test]
    fn diff_names_fields_and_bytes() {
        let v = vectors();
        let full = v
            .join("compose-full/agreement.canonical.json")
            .display()
            .to_string();
        let noflows = v
            .join("compose-no-flows/agreement.canonical.json")
            .display()
            .to_string();
        let pretty = v
            .join("compose-full/agreement.pretty.json")
            .display()
            .to_string();
        assert_eq!(capture(&["diff", &full, &full]).0, verbs::EXIT_OK);
        let (code, out) = capture(&["diff", "--json", &full, &noflows]);
        assert_eq!(code, verbs::EXIT_FED_MISMATCH, "{out}");
        assert!(out.contains(r#""kind":"fields""#), "{out}");
        let (code, out) = capture(&["diff", "--json", &full, &pretty]);
        assert_eq!(code, verbs::EXIT_FED_MISMATCH, "{out}");
        assert!(out.contains(r#""kind":"bytes""#), "{out}");
    }

    /// The site comes from --context-dir, and --cert/--ca-roots override the
    /// two files in it.
    #[test]
    fn the_site_comes_from_the_flags() {
        let s = resolve_site(&SiteArgs {
            context_dir: Some("/ctx".into()),
            cert: None,
            ca_roots: Some("/ca.pem".into()),
        })
        .unwrap();
        assert_eq!(s.context_dir, PathBuf::from("/ctx"));
        assert_eq!(s.cert_path, PathBuf::from("/ctx/mtls.cert.pem"));
        assert_eq!(s.ca_roots_path, PathBuf::from("/ca.pem"));
        assert_eq!(s.declaration_path(), PathBuf::from("/ctx/federation.toml"));
    }

    /// init with no declaration in the context directory says where it looked,
    /// prints the template and exits 23, as goat does.
    #[test]
    fn init_without_a_declaration_names_the_file_and_exits_23() {
        let d = tempfile::tempdir().unwrap();
        let ctx = d.path().display().to_string();
        let (code, out) = capture(&["init", "--context-dir", &ctx]);
        assert_eq!(code, verbs::EXIT_FED_UNDERIVED, "{out}");
        assert!(
            out.contains(&format!("no federation.toml at {ctx}/federation.toml")),
            "{out}"
        );
        assert!(
            out.contains("sovereign_id = \"xaa\""),
            "the template follows: {out}"
        );
    }

    /// verify with --as reads no site at all, so it runs where none resolves.
    #[test]
    fn verify_with_as_reads_no_site() {
        let full = vectors()
            .join("compose-full/agreement.canonical.json")
            .display()
            .to_string();
        let l = line(&["verify", &full, "--as", "xaa", "--json"]);
        let mut out = Vec::new();
        let unreachable = || -> Result<Site> { Err(anyhow!("no site here")) };
        let code = verbs::run(l.cmd, &unreachable, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("no site here"), "{text}");
        assert!(
            code == verbs::EXIT_OK || code == verbs::EXIT_FED_NOT_CONFORMANT,
            "{text}"
        );
    }
}

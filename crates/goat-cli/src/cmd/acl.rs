// Evaluate a deployment's ACL declaration with the same evaluator as netvis
// and CI. This command needs no network connection or credentials. Nothing of
// any deployment is compiled in: the declaration is always read from files the
// deployment supplies (see `declaration`), and with none found the command
// stops with E_ACL_DECLARATION_MISSING.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use panoscope_core::aclcheck::{Declaration, Report, Verdict};

use crate::errors::GoatError;
use crate::profile::Profile;

/// The two declaration files, by name. A directory that supplies a
/// declaration holds both.
pub(crate) const TAXONOMY_FILE: &str = "netbird_acl_taxonomy.yml";
pub(crate) const MATRIX_FILE: &str = "netbird_acl_matrix.yml";
/// Where a checkout or deployment overlay keeps them.
const CHECKOUT_DIR: &str = "ops/ansible";
/// Names a directory holding both files.
pub(crate) const DECLARATION_DIR_ENV: &str = "GOAT_ACL_DECLARATION_DIR";
/// The profile.toml key naming a directory holding both files.
pub(crate) const CONTEXT_KEY: &str = "acl_declaration_dir";
/// The directory inside a context that holds both files when the context's
/// profile names none (`goat context refresh` fills it from get.<site>/acl/).
pub(crate) const CONTEXT_SUBDIR: &str = "acl";

#[derive(Subcommand, Debug)]
pub enum AclCmd {
    /// Check the deployment's ACL declaration. Violations and undetermined
    /// results fail. The declaration is read from --declaration, else
    /// GOAT_ACL_DECLARATION_DIR, else the active context (its
    /// `acl_declaration_dir`, or its `acl/` directory), else the checkout the
    /// working directory is in.
    Test(AclTestArgs),
}

pub async fn run(ctx: &super::Ctx, cmd: AclCmd) -> anyhow::Result<()> {
    match cmd {
        AclCmd::Test(args) => run_test(&ctx.profile, args).await,
    }
}

#[derive(Args, Debug)]
pub struct AclTestArgs {
    /// Scope to check. Omit to check every declared scope.
    pub scope: Option<String>,
    /// Directory holding netbird_acl_taxonomy.yml and netbird_acl_matrix.yml.
    #[arg(long, value_name = "DIR")]
    pub declaration: Option<PathBuf>,
    /// Emit the full report as JSON.
    #[arg(long)]
    pub json: bool,
    /// Include satisfied assertions in the text report.
    #[arg(long)]
    pub verbose: bool,
    /// Accept a check in which no assertion exists. Without it, a scope (or a
    /// whole declaration) that declares no assertions fails: nothing was
    /// checked, and an empty check is not a pass.
    #[arg(long)]
    pub none_expected: bool,
}

pub async fn run_test(profile: &Profile, args: AclTestArgs) -> anyhow::Result<()> {
    // The source is named on stderr, apart from JSON and from the verdict.
    let decl = declaration(profile, args.declaration.as_deref(), "acl test")?;
    let checked = check(&decl, args.scope.as_deref())?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&checked.reports)?);
        // JSON stays the report array; the coverage line goes to stderr.
        if !checked.unasserted.is_empty() {
            eprintln!(
                "acl test: not checked — no assertions declared: {}",
                checked.unasserted.join(", ")
            );
        }
    } else {
        for r in &checked.reports {
            print_report(r, args.verbose);
        }
        // Include unchecked scopes so the report shows its coverage.
        if !checked.unasserted.is_empty() {
            println!(
                "\nnot checked — no assertions declared: {}",
                checked.unasserted.join(", ")
            );
        }
        if checked.reports.is_empty() && args.none_expected {
            println!("Nothing was checked, and --none-expected says none were expected.");
        }
    }

    verdict(&checked, args.scope.as_deref(), args.none_expected)
}

/// What one run evaluated: a report per scope that carries assertions, and
/// the scopes that carry none.
struct Checked {
    reports: Vec<Report>,
    unasserted: Vec<String>,
}

/// Evaluate `scope`, or every declared scope when it is `None`.
fn check(decl: &Declaration, scope: Option<&str>) -> anyhow::Result<Checked> {
    let scopes = match scope {
        Some(s) => vec![s.to_string()],
        None => decl.scopes(),
    };
    let mut checked = Checked {
        reports: Vec::new(),
        unasserted: Vec::new(),
    };
    for site in &scopes {
        // Reject an unknown scope before checking whether it has assertions.
        let model = decl.model(site)?;
        let tests = decl.tests(site);
        if tests.is_empty() {
            checked.unasserted.push(site.clone());
            continue;
        }
        checked
            .reports
            .push(panoscope_core::aclcheck::evaluate(&model, &tests));
    }
    Ok(checked)
}

/// The exit decision. A run that evaluated no assertion fails unless the
/// caller said none were expected: `all()` over an empty set is true, and a
/// goatnet whose declaration carries no assertions would otherwise pass every
/// gate that reads this verb. Undetermined results fail as well as explicit
/// violations.
fn verdict(checked: &Checked, scope: Option<&str>, none_expected: bool) -> anyhow::Result<()> {
    if checked.reports.is_empty() && !none_expected {
        let what = match scope {
            Some(s) => format!("scope `{s}` declares no ACL assertions"),
            None => "no scope in the declaration carries an ACL assertion".to_string(),
        };
        anyhow::bail!(
            "{what}, so nothing was checked, and an empty check is not a pass. \
             Declare them under `netbird_acl_tests:` in ops/warrant/rule-intents/netbird-acl.yaml \
             and regenerate with ops/warrant/generate.py, or pass --none-expected if none are meant to exist"
        );
    }
    if checked.reports.iter().all(|r| r.is_clean()) {
        Ok(())
    } else {
        anyhow::bail!("ACL assertions did not hold against the declaration")
    }
}

/// Where a declaration was found, for the note on stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    Flag(PathBuf),
    Env(PathBuf),
    Context(PathBuf),
    Checkout(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Flag(d) => write!(f, "--declaration {}", d.display()),
            Source::Env(d) => write!(f, "{DECLARATION_DIR_ENV}={}", d.display()),
            Source::Context(d) => write!(f, "the context's {}", d.display()),
            Source::Checkout(d) => write!(f, "the checkout's {}", d.display()),
        }
    }
}

/// The ACL declaration that governs here, first match wins:
/// 1. `flag` (`--declaration <dir>`);
/// 2. the directory `GOAT_ACL_DECLARATION_DIR` names;
/// 3. the active context: the directory its profile.toml names as
///    `acl_declaration_dir`, else its own `acl/` directory when that holds
///    both files;
/// 4. `ops/ansible/` of the checkout or deployment overlay the working
///    directory is in.
///
/// A directory named by 1–3 must hold both files, or the command fails rather
/// than check other rules. Nothing found is E_ACL_DECLARATION_MISSING.
pub(crate) fn declaration(
    profile: &Profile,
    flag: Option<&Path>,
    verb: &str,
) -> anyhow::Result<Declaration> {
    let cwd = std::env::current_dir().ok();
    let env = std::env::var_os(DECLARATION_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let source = locate(flag, env, Some(profile), cwd.as_deref())?;
    let dir = match &source {
        Source::Flag(d) | Source::Env(d) | Source::Context(d) | Source::Checkout(d) => d,
    };
    let (tax, mat) = read_dir(dir, &source)?;
    eprintln!("{verb}: reading the ACL declaration from {source}");
    Ok(Declaration::parse(&tax, &mat)?)
}

/// Pick the declaration's directory without reading the files.
pub(crate) fn locate(
    flag: Option<&Path>,
    env: Option<PathBuf>,
    profile: Option<&Profile>,
    cwd: Option<&Path>,
) -> anyhow::Result<Source> {
    if let Some(d) = flag {
        return Ok(Source::Flag(d.to_path_buf()));
    }
    if let Some(d) = env {
        return Ok(Source::Env(d));
    }
    if let Some(p) = profile {
        if let Some(d) = context_dir(p) {
            return Ok(Source::Context(d));
        }
    }
    if let Some(d) = cwd.and_then(checkout_dir) {
        return Ok(Source::Checkout(d));
    }
    Err(GoatError::AclDeclarationMissing {
        hint: format!(
            "supply the deployment's {TAXONOMY_FILE} and {MATRIX_FILE}: pass --declaration <dir>, \
             set {DECLARATION_DIR_ENV}=<dir>, set `{CONTEXT_KEY} = \"<dir>\"` in the context's \
             profile.toml, put both files in the context's {CONTEXT_SUBDIR}/ directory \
             (`goat context refresh` fetches them when the goatnet serves them), or run from a \
             checkout that carries them under {CHECKOUT_DIR}/"
        ),
    }
    .into())
}

/// The context's declaration directory: its profile's `acl_declaration_dir`,
/// else its `acl/` directory when both files are there.
fn context_dir(profile: &Profile) -> Option<PathBuf> {
    let body = std::fs::read_to_string(profile.profile_toml_path()).ok();
    if let Some(d) = body
        .as_deref()
        .and_then(|b| crate::profile::extract_toml_field(b, CONTEXT_KEY))
        .filter(|v| !v.trim().is_empty())
    {
        return Some(PathBuf::from(d));
    }
    let d = profile.context_dir().join(CONTEXT_SUBDIR);
    holds_both(&d).then_some(d)
}

/// `ops/ansible/` of the checkout `start` is inside, when it holds both files.
fn checkout_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|a| a.join(CHECKOUT_DIR))
        .find(|d| holds_both(d))
}

fn holds_both(dir: &Path) -> bool {
    dir.join(TAXONOMY_FILE).is_file() && dir.join(MATRIX_FILE).is_file()
}

/// Both files from the chosen directory. Either one unreadable is an error.
fn read_dir(dir: &Path, source: &Source) -> anyhow::Result<(String, String)> {
    let read = |name: &str| {
        let path = dir.join(name);
        std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "the ACL declaration from {source} is incomplete: {} cannot be read: {e}",
                path.display()
            )
        })
    };
    Ok((read(TAXONOMY_FILE)?, read(MATRIX_FILE)?))
}

fn print_report(r: &Report, verbose: bool) {
    let head = match r.verdict {
        Verdict::Satisfied => format!("{}: all {} assertions hold", r.site, r.satisfied),
        Verdict::Baselined => format!(
            "{}: {} hold, {} are recorded known violations — nothing new is broken",
            r.site, r.satisfied, r.baselined
        ),
        Verdict::Violated => format!("{}: {} VIOLATED", r.site, r.violated),
        Verdict::Undetermined => format!(
            "{}: {} UNDETERMINED — an unknown is not a pass, so the whole scope is capped",
            r.site, r.undetermined
        ),
    };
    println!("\n{head}");
    println!("  checked against the DECLARATION, {}", r.quantifier);

    for name in &r.baseline_now_passing {
        println!(
            "  ratchet: `{name}` was baselined and now passes — delete the baseline, \
             or it becomes permission"
        );
    }

    for o in &r.outcomes {
        let show = verbose || !matches!(o.verdict, Verdict::Satisfied);
        if !show {
            continue;
        }
        let tag = match o.verdict {
            Verdict::Satisfied => "ok        ",
            Verdict::Baselined => "recorded  ",
            Verdict::Violated => "VIOLATED  ",
            Verdict::Undetermined => "UNDECIDED ",
        };
        println!("  {tag}[{}] {}", o.check, o.name);
        if let Some(reason) = o.reason {
            println!(
                "              {} ({})",
                reason.explain(),
                if reason.is_declaration_fault() {
                    "your declaration"
                } else {
                    "an unread source"
                }
            );
        }
        if let Some(b) = &o.baseline {
            println!(
                "              recorded since {} — {}",
                b.violated_since,
                b.why.trim()
            );
        }
        for c in &o.counterexamples {
            println!(
                "              {} -> {}:{} ({})",
                c.src, c.dst, c.port, c.proto
            );
            if let Some(g) = &c.granted_by {
                println!("                granted by rule `{g}`");
            }
            // Include a nearby rule to help diagnose the missing permission.
            if let Some(n) = &c.nearest {
                println!("                nearest:  {n}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic declaration with assertions, under tests/fixtures/acl.
    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acl")
    }

    fn fixture() -> (String, String) {
        let d = fixture_dir();
        (
            std::fs::read_to_string(d.join(TAXONOMY_FILE)).expect("fixture taxonomy"),
            std::fs::read_to_string(d.join(MATRIX_FILE)).expect("fixture matrix"),
        )
    }

    fn decl(drop_tests: bool) -> Declaration {
        let (tax, mat) = fixture();
        let mat = if drop_tests {
            mat.split("netbird_acl_tests:").next().unwrap().to_string()
        } else {
            mat
        };
        Declaration::parse(&tax, &mat).expect("fixture declaration")
    }

    fn profile_in(dir: &Path) -> Profile {
        Profile {
            dir: dir.to_path_buf(),
            context: "t".into(),
        }
    }

    #[tokio::test]
    async fn it_runs_with_no_network_and_no_credentials() {
        let home = tempfile::tempdir().expect("temp dir");
        run_test(
            &profile_in(home.path()),
            AclTestArgs {
                scope: None,
                declaration: Some(fixture_dir()),
                json: true,
                verbose: false,
                none_expected: true,
            },
        )
        .await
        .expect("the fixture's assertions hold");
    }

    #[tokio::test]
    async fn an_unknown_scope_says_so_rather_than_passing_silently() {
        let home = tempfile::tempdir().expect("temp dir");
        let e = run_test(
            &profile_in(home.path()),
            AclTestArgs {
                scope: Some("nosuchsite".into()),
                declaration: Some(fixture_dir()),
                json: false,
                verbose: false,
                none_expected: true,
            },
        )
        .await;
        assert!(
            e.is_err(),
            "an unknown scope must not exit 0, even with --none-expected"
        );
    }

    #[test]
    fn a_scope_with_no_assertions_fails_and_says_why() {
        let d = decl(true);
        let checked = check(&d, Some("lone")).expect("lone is declared");
        assert!(checked.reports.is_empty());
        assert_eq!(checked.unasserted, vec!["lone".to_string()]);
        let msg = format!(
            "{:#}",
            verdict(&checked, Some("lone"), false).expect_err("empty is not a pass")
        );
        assert!(
            msg.contains("scope `lone` declares no ACL assertions"),
            "{msg}"
        );
        assert!(msg.contains("--none-expected"), "the way out is named: {msg}");
    }

    #[test]
    fn a_declaration_with_no_assertions_anywhere_fails() {
        let d = decl(true);
        let checked = check(&d, None).expect("both scopes declared");
        assert_eq!(
            checked.unasserted,
            vec!["lone".to_string(), "quiet".to_string()]
        );
        let msg = format!(
            "{:#}",
            verdict(&checked, None, false).expect_err("empty is not a pass")
        );
        assert!(
            msg.contains("no scope in the declaration carries an ACL assertion"),
            "{msg}"
        );
    }

    #[test]
    fn none_expected_turns_an_empty_check_into_a_reported_pass() {
        let d = decl(true);
        let checked = check(&d, Some("lone")).expect("lone is declared");
        verdict(&checked, Some("lone"), true).expect("the caller said none were expected");
    }

    #[test]
    fn a_scope_with_holding_assertions_passes_without_the_flag() {
        let d = decl(false);
        let checked = check(&d, Some("lone")).expect("lone is declared");
        assert_eq!(checked.reports.len(), 1);
        verdict(&checked, Some("lone"), false).expect("both assertions hold");
    }

    #[test]
    fn an_unasserted_scope_beside_an_asserted_one_is_reported_not_hidden() {
        let d = decl(false);
        let checked = check(&d, None).expect("both scopes declared");
        assert_eq!(checked.reports.len(), 1);
        assert_eq!(checked.unasserted, vec!["quiet".to_string()]);
        verdict(&checked, None, false).expect("the asserted scope holds");
    }

    #[test]
    fn none_expected_never_excuses_a_violation() {
        let (tax, mat) = fixture();
        let broken = mat.replace("deny: [\"INFRA:22\"]", "deny: [\"INFRA:443\"]");
        let d = Declaration::parse(&tax, &broken).expect("fixture declaration");
        let checked = check(&d, Some("lone")).expect("lone is declared");
        assert!(verdict(&checked, Some("lone"), true).is_err());
    }

    fn write_both(dir: &Path) {
        let (tax, mat) = fixture();
        std::fs::create_dir_all(dir).expect("dir");
        std::fs::write(dir.join(TAXONOMY_FILE), tax).expect("taxonomy");
        std::fs::write(dir.join(MATRIX_FILE), mat).expect("matrix");
    }

    #[test]
    fn the_flag_wins_then_the_env_then_the_context_then_the_checkout() {
        let home = tempfile::tempdir().expect("temp dir");
        let p = profile_in(home.path());
        let ctx_acl = p.context_dir().join(CONTEXT_SUBDIR);
        write_both(&ctx_acl);
        let tree = tempfile::tempdir().expect("temp dir");
        write_both(&tree.path().join(CHECKOUT_DIR));
        let cwd = tree.path().join("docs");
        let flag = PathBuf::from("/flag");
        let env = PathBuf::from("/env");

        let got = |f: Option<&Path>, e: Option<PathBuf>, prof: Option<&Profile>| {
            locate(f, e, prof, Some(&cwd)).expect("found")
        };
        assert_eq!(
            got(Some(&flag), Some(env.clone()), Some(&p)),
            Source::Flag(flag.clone())
        );
        assert_eq!(got(None, Some(env.clone()), Some(&p)), Source::Env(env));
        assert_eq!(got(None, None, Some(&p)), Source::Context(ctx_acl));
        assert_eq!(
            got(None, None, None),
            Source::Checkout(tree.path().join(CHECKOUT_DIR))
        );
    }

    #[test]
    fn the_contexts_named_directory_beats_its_acl_directory() {
        let home = tempfile::tempdir().expect("temp dir");
        let p = profile_in(home.path());
        std::fs::create_dir_all(p.context_dir()).expect("context dir");
        std::fs::write(
            p.profile_toml_path(),
            format!("{CONTEXT_KEY} = \"/somewhere\"\n"),
        )
        .expect("profile");
        write_both(&p.context_dir().join(CONTEXT_SUBDIR));
        assert_eq!(
            locate(None, None, Some(&p), None).expect("found"),
            Source::Context(PathBuf::from("/somewhere"))
        );
    }

    #[test]
    fn nothing_found_is_the_catalog_error_naming_every_place() {
        let home = tempfile::tempdir().expect("temp dir");
        let p = profile_in(home.path());
        let plain = tempfile::tempdir().expect("temp dir");
        let e = locate(None, None, Some(&p), Some(plain.path())).expect_err("nothing supplied");
        let g = e.downcast_ref::<GoatError>().expect("a goat error");
        assert_eq!(g.code(), "E_ACL_DECLARATION_MISSING");
        let hint = g.hint();
        for want in ["--declaration", DECLARATION_DIR_ENV, CONTEXT_KEY, "goat context refresh"] {
            assert!(hint.contains(want), "{want} missing from: {hint}");
        }
    }

    #[test]
    fn a_named_directory_missing_a_file_is_an_error() {
        let d = tempfile::tempdir().expect("temp dir");
        std::fs::write(d.path().join(TAXONOMY_FILE), "x").expect("taxonomy");
        let src = Source::Flag(d.path().to_path_buf());
        let msg = format!("{:#}", read_dir(d.path(), &src).expect_err("matrix missing"));
        assert!(msg.contains(MATRIX_FILE), "{msg}");
    }

    #[test]
    fn a_checkout_is_found_from_anywhere_inside_it_and_a_plain_folder_is_not() {
        let tree = tempfile::tempdir().expect("temp dir");
        write_both(&tree.path().join(CHECKOUT_DIR));
        let nested = tree.path().join("crates/x/src");
        std::fs::create_dir_all(&nested).expect("nested");
        assert_eq!(
            checkout_dir(&nested),
            Some(tree.path().join(CHECKOUT_DIR))
        );
        let plain = tempfile::tempdir().expect("temp dir");
        assert_eq!(checkout_dir(plain.path()), None);
    }
}

// Evaluate the ACL declaration with the same evaluator as netvis and CI.
// This command needs no network connection, profile, or credentials. The
// declaration is a deployment's own: GOAT_ACL_DECLARATION_DIR, else the
// checkout or overlay the working directory is in, else the copy compiled in.

use std::path::Path;

use clap::{Args, Subcommand};
use panoscope_core::aclcheck::{Declaration, Report, Verdict};

// Keep these includes in the CLI. Other production consumers of panoscope-core
// do not package the ACL files and must be able to build without them.
const TAXONOMY_YAML: &str = include_str!("../../../../ops/ansible/netbird_acl_taxonomy.yml");
const MATRIX_YAML: &str = include_str!("../../../../ops/ansible/netbird_acl_matrix.yml");

/// The compiled-in declaration, keyed by its path in a goat checkout.
const DECLARATION: [(&str, &str); 2] = [
    ("ops/ansible/netbird_acl_taxonomy.yml", TAXONOMY_YAML),
    ("ops/ansible/netbird_acl_matrix.yml", MATRIX_YAML),
];

#[derive(Subcommand, Debug)]
pub enum AclCmd {
    /// Check the embedded declaration. Violations and undetermined results fail.
    Test(AclTestArgs),
}

pub async fn run(cmd: AclCmd) -> anyhow::Result<()> {
    match cmd {
        AclCmd::Test(args) => run_test(args).await,
    }
}

#[derive(Args, Debug)]
pub struct AclTestArgs {
    /// Scope to check. Omit to check every declared scope.
    pub scope: Option<String>,
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

pub async fn run_test(args: AclTestArgs) -> anyhow::Result<()> {
    // Inside a checkout, the declaration on disk is the one that governs: a
    // goatnet rendered after this binary was built (a learner's goatlite) is in
    // the files and not in the embedded copy, which answered "no site named"
    // for every such goatnet. Outside a checkout, the embedded copy.
    // The note stays on stderr, apart from JSON and from the verdict.
    let decl = declaration("acl test: checking the declaration in this checkout")?;
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

/// Names a directory that holds a deployment's `netbird_acl_taxonomy.yml` and
/// `netbird_acl_matrix.yml`. A deployment whose declaration is not the one
/// compiled in sets it, so the CLI checks that deployment's rules from any
/// working directory.
pub(crate) const DECLARATION_DIR_ENV: &str = "GOAT_ACL_DECLARATION_DIR";

/// The ACL declaration that governs here, first match wins:
/// 1. the directory `GOAT_ACL_DECLARATION_DIR` names (both files must be
///    readable there, or the command fails rather than check other rules);
/// 2. the files of the goat checkout, or of a deployment overlay carrying
///    `ops/ansible/netbird_acl_taxonomy.yml`, that the working directory is in;
/// 3. the copy compiled in.
///
/// `note` goes to stderr when files on disk are used, so the source is never
/// silent.
pub(crate) fn declaration(note: &str) -> anyhow::Result<Declaration> {
    if let Some(dir) = std::env::var_os(DECLARATION_DIR_ENV).filter(|v| !v.is_empty()) {
        let dir = std::path::PathBuf::from(dir);
        let (tax, mat) = dir_declaration(&dir)?;
        eprintln!("{note} ({DECLARATION_DIR_ENV}={})", dir.display());
        return Ok(Declaration::parse(&tax, &mat)?);
    }
    let on_disk = std::env::current_dir()
        .ok()
        .and_then(|cwd| checkout_declaration(&cwd));
    Ok(match &on_disk {
        Some((tax, mat)) => {
            eprintln!("{note}");
            Declaration::parse(tax, mat)?
        }
        None => Declaration::parse(TAXONOMY_YAML, MATRIX_YAML)?,
    })
}

/// Both declaration files from a directory named explicitly. Either one
/// missing is an error: the caller asked for this deployment's rules.
fn dir_declaration(dir: &Path) -> anyhow::Result<(String, String)> {
    let read = |name: &str| {
        let path = dir.join(name);
        std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "{DECLARATION_DIR_ENV} names {}, but {} cannot be read: {e}",
                dir.display(),
                path.display()
            )
        })
    };
    Ok((
        read("netbird_acl_taxonomy.yml")?,
        read("netbird_acl_matrix.yml")?,
    ))
}

/// Both declaration files read from the checkout `start` is inside, if any:
/// a goat source checkout (it carries `crates/goat-cli`) or a deployment
/// overlay that carries its own taxonomy under `ops/ansible/`.
fn checkout_declaration(start: &Path) -> Option<(String, String)> {
    let root = start.ancestors().find(|dir| {
        dir.join("crates/goat-cli/Cargo.toml").is_file() || dir.join(DECLARATION[0].0).is_file()
    })?;
    let tax = std::fs::read_to_string(root.join(DECLARATION[0].0)).ok()?;
    let mat = std::fs::read_to_string(root.join(DECLARATION[1].0)).ok()?;
    Some((tax, mat))
}

// Missing or unreadable declarations differ from the embedded copy too. Use the
// crate manifest to find the source root even when both YAML files are missing.
#[allow(dead_code)]
fn changed_declaration_files(start: &Path) -> Vec<&'static str> {
    let Some(root) = start
        .ancestors()
        .find(|dir| dir.join("crates/goat-cli/Cargo.toml").is_file())
    else {
        return Vec::new();
    };
    DECLARATION
        .iter()
        .filter(|(path, built)| {
            std::fs::read_to_string(root.join(path))
                .map_or(true, |on_disk| !declarations_match(&on_disk, built))
        })
        .map(|(path, _)| *path)
        .collect()
}

fn declarations_match(on_disk: &str, built: &str) -> bool {
    on_disk.replace("\r\n", "\n") == built.replace("\r\n", "\n")
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

    #[tokio::test]
    async fn it_runs_with_no_context_no_network_and_no_credentials() {
        // This checkout's declaration may carry no assertions at all; the flag
        // keeps this test about the verb running, not about the declaration.
        run_test(AclTestArgs {
            scope: None,
            json: true,
            verbose: false,
            none_expected: true,
        })
        .await
        .expect("the declared assertions must hold");
    }

    #[tokio::test]
    async fn an_unknown_scope_says_so_rather_than_passing_silently() {
        let e = run_test(AclTestArgs {
            scope: Some("nosuchsite".into()),
            json: false,
            verbose: false,
            none_expected: true,
        })
        .await;
        assert!(
            e.is_err(),
            "an unknown scope must not exit 0, even with --none-expected"
        );
    }

    const TAX: &str = "netbird_acl_taxonomy:\n  lone:\n    site_prefix: LONE\n    groups:\n    - suffix: INFRA\n    - suffix: DEVELOPER\n  quiet:\n    site_prefix: QUIET\n    groups:\n    - suffix: INFRA\n";
    const RULES: &str = "netbird_acl_matrix:\n  lone:\n    rules:\n    - name: dev to infra\n      sources: [DEVELOPER]\n      destinations: [INFRA]\n      protocol: tcp\n      ports: [\"443\"]\n      action: accept\n  quiet:\n    rules: []\n";

    fn decl(tests: &str) -> Declaration {
        Declaration::parse(TAX, &format!("{RULES}{tests}")).expect("fixture declaration")
    }

    const LONE_TESTS: &str = "netbird_acl_tests:\n  lone:\n  - name: a developer reaches the box on 443\n    src: DEVELOPER\n    proto: tcp\n    accept: [\"INFRA:443\"]\n  - name: a developer has no SSH to the box\n    src: DEVELOPER\n    proto: tcp\n    deny: [\"INFRA:22\"]\n";

    #[test]
    fn a_scope_with_no_assertions_fails_and_says_why() {
        let d = decl("");
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
        assert!(
            msg.contains("--none-expected"),
            "the way out is named: {msg}"
        );
    }

    #[test]
    fn a_declaration_with_no_assertions_anywhere_fails() {
        let d = decl("");
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
        let d = decl("");
        let checked = check(&d, Some("lone")).expect("lone is declared");
        verdict(&checked, Some("lone"), true).expect("the caller said none were expected");
    }

    #[test]
    fn a_scope_with_holding_assertions_passes_without_the_flag() {
        let d = decl(LONE_TESTS);
        let checked = check(&d, Some("lone")).expect("lone is declared");
        assert_eq!(checked.reports.len(), 1);
        verdict(&checked, Some("lone"), false).expect("both assertions hold");
    }

    #[test]
    fn an_unasserted_scope_beside_an_asserted_one_is_reported_not_hidden() {
        let d = decl(LONE_TESTS);
        let checked = check(&d, None).expect("both scopes declared");
        assert_eq!(checked.reports.len(), 1);
        assert_eq!(checked.unasserted, vec!["quiet".to_string()]);
        verdict(&checked, None, false).expect("the asserted scope holds");
    }

    #[test]
    fn none_expected_never_excuses_a_violation() {
        let broken = LONE_TESTS.replace("deny: [\"INFRA:22\"]", "deny: [\"INFRA:443\"]");
        let d = decl(&broken);
        let checked = check(&d, Some("lone")).expect("lone is declared");
        assert!(verdict(&checked, Some("lone"), true).is_err());
    }

    // The fixture needs the crate manifest used to locate a source checkout.
    fn checkout(taxonomy: &str, matrix: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let crate_dir = dir.path().join("crates/goat-cli");
        std::fs::create_dir_all(&crate_dir).expect("crate directory");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"goat-cli\"\n",
        )
        .expect("crate manifest");
        let ansible = dir.path().join("ops/ansible");
        std::fs::create_dir_all(&ansible).expect("ops/ansible");
        std::fs::write(ansible.join("netbird_acl_taxonomy.yml"), taxonomy).expect("taxonomy");
        std::fs::write(ansible.join("netbird_acl_matrix.yml"), matrix).expect("matrix");
        dir
    }

    #[test]
    fn an_edited_declaration_is_named_from_anywhere_in_the_checkout() {
        let dir = checkout(TAXONOMY_YAML, &format!("{MATRIX_YAML}\n# edited\n"));
        let nested = dir.path().join("crates/goat-cli");
        std::fs::create_dir_all(&nested).expect("nested");
        assert_eq!(
            changed_declaration_files(&nested),
            vec!["ops/ansible/netbird_acl_matrix.yml"]
        );
    }

    #[test]
    fn a_matching_checkout_and_a_plain_folder_are_quiet() {
        let same = checkout(TAXONOMY_YAML, MATRIX_YAML);
        assert!(
            changed_declaration_files(same.path()).is_empty(),
            "matching checkout"
        );
        let plain = tempfile::tempdir().expect("temp dir");
        assert!(
            changed_declaration_files(plain.path()).is_empty(),
            "not a checkout"
        );
    }

    #[test]
    fn the_checkouts_declaration_is_the_one_read() {
        let edited = format!("{MATRIX_YAML}\n# edited\n");
        let dir = checkout(TAXONOMY_YAML, &edited);
        let nested = dir.path().join("crates/goat-cli");
        let (tax, mat) = checkout_declaration(&nested).expect("inside a checkout");
        assert_eq!(tax, TAXONOMY_YAML);
        assert_eq!(mat, edited);
        let plain = tempfile::tempdir().expect("temp dir");
        assert!(
            checkout_declaration(plain.path()).is_none(),
            "not a checkout"
        );
    }

    #[test]
    fn an_overlay_without_the_cli_crate_is_read_too() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ansible = dir.path().join("ops/ansible");
        std::fs::create_dir_all(&ansible).expect("ops/ansible");
        std::fs::write(ansible.join("netbird_acl_taxonomy.yml"), TAX).expect("taxonomy");
        std::fs::write(ansible.join("netbird_acl_matrix.yml"), RULES).expect("matrix");
        let nested = dir.path().join("docs/deep");
        std::fs::create_dir_all(&nested).expect("nested");
        let (tax, mat) = checkout_declaration(&nested).expect("inside an overlay");
        assert_eq!(tax, TAX);
        assert_eq!(mat, RULES);
    }

    #[test]
    fn a_named_directory_is_read_and_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("netbird_acl_taxonomy.yml"), TAX).expect("taxonomy");
        let msg = format!(
            "{:#}",
            dir_declaration(dir.path()).expect_err("matrix missing")
        );
        assert!(msg.contains("netbird_acl_matrix.yml"), "{msg}");
        std::fs::write(dir.path().join("netbird_acl_matrix.yml"), RULES).expect("matrix");
        let (tax, mat) = dir_declaration(dir.path()).expect("both present");
        assert_eq!((tax.as_str(), mat.as_str()), (TAX, RULES));
    }

    #[test]
    fn line_endings_do_not_change_the_declaration() {
        let crlf = checkout(
            &TAXONOMY_YAML.replace("\r\n", "\n").replace("\n", "\r\n"),
            &MATRIX_YAML.replace("\r\n", "\n").replace("\n", "\r\n"),
        );
        assert!(changed_declaration_files(crlf.path()).is_empty());
        assert!(declarations_match("key: value\n", "key: value\r\n"));
        assert!(declarations_match("key: value\r\n", "key: value\n"));
        assert!(!declarations_match("key: edited\r\n", "key: value\n"));
    }

    #[test]
    fn missing_or_unreadable_declarations_are_reported() {
        let dir = checkout(TAXONOMY_YAML, MATRIX_YAML);
        let taxonomy = dir.path().join(DECLARATION[0].0);
        let matrix = dir.path().join(DECLARATION[1].0);
        std::fs::remove_file(&matrix).expect("remove matrix");
        assert_eq!(
            changed_declaration_files(dir.path()),
            vec![DECLARATION[1].0]
        );
        std::fs::remove_file(&taxonomy).expect("remove taxonomy");
        assert_eq!(
            changed_declaration_files(dir.path()),
            vec![DECLARATION[0].0, DECLARATION[1].0]
        );
        std::fs::create_dir(&matrix).expect("unreadable matrix directory");
        assert_eq!(
            changed_declaration_files(dir.path()),
            vec![DECLARATION[0].0, DECLARATION[1].0]
        );
    }
}

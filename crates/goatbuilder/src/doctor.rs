//! `doctor` — what a run needs, checked before anything is cut, each failure with
//! its recovery command. The checks are the ones the first microscope release
//! candidate lacked: the Python the pins need, a current `main` checkout (C3),
//! the transport each environment will use (the mesh first, a door only when not
//! meshed), and — F-587's class — that every component's deploy playbook
//! resolves, through any alias, to a play with a definite target mode.

use crate::runner::Cmd;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::config::{self, Component};
use crate::{Check, Report, EXIT_REFUSED};

pub fn run(repo: &Path, only_env: Option<&str>, frozen: bool, credentials: bool, browser_tests: bool) -> Result<Report> {
    let mut checks = Vec::new();

    // 1. The config itself.
    let cfg_path = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let lite_path = repo.join("goatlite.yaml");
    let cfg = match config::load(&cfg_path) {
        // A goatlite checkout with no config of its own (the open-source export):
        // goatlite.yaml declares the goatnet, and row `lite` below judges that file.
        Err(_) if !cfg_path.is_file() && lite_path.is_file() => {
            checks.push(ok("config", format!("{} is absent: goatlite.yaml declares this checkout's goatnet", rel(repo, &cfg_path))));
            None
        }
        Ok(c) => {
            checks.push(ok(
                "config",
                format!(
                    "{} validates ({}, preset {})",
                    rel(repo, &cfg_path),
                    c.schema,
                    c.preset.as_deref().unwrap_or("full")
                ),
            ));
            if let Some(r) = &c.repo {
                checks.push(ok(
                    "repo",
                    format!(
                        "source {}, images under {}, declarations signed by {}",
                        r.source,
                        r.image_namespace,
                        r.signing.as_deref().unwrap_or("intent-sign")
                    ),
                ));
            }
            if let Some(l) = &c.lite {
                checks.push(ok("lite", lite_summary(l, c.secrets.as_ref().map(|s| s.store.as_str()))));
            }
            Some(c)
        }
        Err(e) => {
            checks.push(refused(
                "config",
                format!("{e:#}"),
                "fix the config, then re-run `goatbuilder validate`",
            ));
            None
        }
    };

    // 1b. A goatlite (ADR 1213): the learner's goatlite.yaml at the checkout root,
    // when there is one. Its row names every decision the preset made, and its
    // environment (direct, no door) is the one the AWS rows below are judged by.
    let lite_cfg = if lite_path.is_file() {
        match config::load(&lite_path) {
            Ok(c) if c.preset.as_deref() == Some("lite") && c.lite.is_some() => {
                checks.push(ok("lite", lite_summary(c.lite.as_ref().expect("checked"), c.secrets.as_ref().map(|s| s.store.as_str()))));
                Some(c)
            }
            Ok(_) => {
                checks.push(ok("lite", "goatlite.yaml is not a lite config (no preset: lite with a lite: block); ignored".to_string()));
                None
            }
            Err(e) => {
                checks.push(refused("lite", format!("{e:#}"), "fix goatlite.yaml, then re-run `goatbuilder validate goatlite.yaml`"));
                None
            }
        }
    } else {
        None
    };

    // 2. Tools on the path. The AWS rows are skipped, as declared, when nothing
    // in the run reaches AWS (goatlite.md §5 row 1): GOAT_AWS_MODE=none, or every
    // declared environment is reached directly and no door is ssm.
    let aws_skip = aws_rows_skipped(lite_cfg.as_ref().or(cfg.as_ref()), std::env::var("GOAT_AWS_MODE").ok().as_deref());
    // A goatlite cuts no release (ADR 1213): the rows that only a release reads
    // (gh, a candidate's digest, the live rows' goat contexts) are skipped, as
    // declared, so the lite doctor grades only what the standup needs (#148).
    let lite = lite_cfg.is_some();
    for (tool, why, fix) in [
        ("git", "the source of record", "install git"),
        (
            "gh",
            "release tags, pull requests and checks",
            "install the GitHub CLI and run `gh auth login`",
        ),
        (
            "ansible-playbook",
            "the push converger",
            "pip install ansible (or pipx)",
        ),
        (
            "aws-vault",
            "every ansible call runs under the AWS profile the checkout declares (deploy.aws_profile); the door needs it too",
            "brew install aws-vault, then aws-vault add <your AWS profile>",
        ),
        (
            "go",
            "signs the regenerated intents (ops/enrollment/cmd/intent-sign) on release and fastlane",
            "brew install go",
        ),
        (
            "ssh",
            "reaches the hosts (mesh or door) and reads their ledgers",
            "ships with the OS",
        ),
    ] {
        if tool == "gh" && lite {
            checks.push(release_row_skipped("tool:gh", "release tags, pull requests and checks"));
            continue;
        }
        if tool == "aws-vault" {
            if let Some(reason) = aws_skip.as_deref() {
                checks.push(ok("tool:aws-vault", format!("skipped: declared: {reason}")));
                continue;
            }
        }
        if tool == "ansible-playbook" && which(tool) {
            checks.push(ansible_row(Cmd::new(tool).arg("--version").output()));
            continue;
        }
        if which(tool) {
            checks.push(ok(&format!("tool:{tool}"), why.to_string()));
        } else {
            checks.push(refused(
                &format!("tool:{tool}"),
                format!("{tool} is not on PATH ({why})"),
                fix,
            ));
        }
    }
    // A candidate's digest: ops/scripts/image-digest uses whichever of crane,
    // skopeo or docker is on PATH (ADR 1204).
    match ["crane", "skopeo", "docker"].iter().find(|t| which(t)) {
        _ if lite => checks.push(release_row_skipped("tool:image-digest", "a candidate's digest")),
        Some(t) => checks.push(ok("tool:image-digest", format!("{t} reads a candidate's digest from the registry (release)"))),
        None => checks.push(refused("tool:image-digest", "none of crane, skopeo or docker is on PATH (release reads a candidate's digest with one)".to_string(), "brew install crane (or skopeo, or Docker)")),
    }
    if which("cosign") {
        checks.push(ok(
            "tool:cosign",
            "verifies signed releases (C2)".to_string(),
        ));
    } else {
        checks.push(ok(
            "tool:cosign",
            "not on PATH: needed only for a cosign-signed publication".to_string(),
        ));
    }
    // A long run from a Mac holds it awake with caffeinate (#2122); without it a
    // run stalls whenever the Mac idles into sleep and the mesh drops.
    if cfg!(target_os = "macos") {
        if which("caffeinate") {
            checks.push(ok("tool:caffeinate", "holds the Mac awake for the life of a long verb (#2122)".to_string()));
        } else {
            checks.push(ok("tool:caffeinate", "not on PATH: a long run stalls if the Mac idles into sleep; keep it awake by hand".to_string()));
        }
    }
    match python_has(&["yaml", "jsonschema"]) {
        Ok(true) => checks.push(ok(
            "python:deps",
            "PyYAML + jsonschema present (the pins and intents render)".to_string(),
        )),
        Ok(false) => checks.push(refused(
            "python:deps",
            "PyYAML or jsonschema missing".to_string(),
            "pip install pyyaml jsonschema (or pipx/venv)",
        )),
        Err(e) => checks.push(refused(
            "python:deps",
            format!("python3 not runnable: {e}"),
            "install python3",
        )),
    }

    // 2b. The environment's own op:// references. `op run` validates EVERY
    // variable whose value starts with op://, not only the env-file, and
    // refuses the whole run on one with fewer than vault/item/field — so a
    // stray export from a shell profile stops a deploy before any host.
    checks.push(op_refs_check(std::env::vars()));

    // 2c. The live acceptance rows run the console's Playwright suite from
    // this laptop (ops/scripts/onboard-intake-smoke.sh); without its
    // toolchain a release reads as failed on the goatnet when it failed here.
    // Only a release runs them, so a missing toolchain is a note unless the run
    // asks for it (--browser-tests).
    checks.push(if lite_cfg.is_some() { live_rows_skipped() } else { live_rows_check(repo, browser_tests) });
    if lite_cfg.is_some() {
        checks.push(image_token_row(std::env::var("GHCR_READ_TOKEN").ok().filter(|v| !v.is_empty()).is_some() || crate::secrets::available(repo).contains("GHCR_READ_TOKEN")));
    }

    // 2d. The sessions and tools a release leans on and only discovers
    // mid-run: the 1Password session, the aws-vault credential, crane (rc.sh
    // resolves digests with it and discards its error), and the goat
    // contexts the live rows mint invites from.
    checks.push(op_session_check(repo, credentials));
    checks.extend(aws_vault_check(repo, aws_skip.as_deref(), credentials));
    if lite {
        checks.push(release_row_skipped("tool:crane", "a candidate's digest"));
        checks.push(release_row_skipped("tool:goat-contexts", "the goat contexts the live rows mint invites from"));
    } else {
        checks.push(crane_check());
        checks.extend(goat_contexts_check(repo));
    }

    // 3. The checkout: on main/master, and current with origin (C3); or an
    // assembled tree, which its .goat-assembled marker names.
    checks.push(checkout_check(repo, frozen));

    // 3b. The declared secrets posture and whether its store can serve (ADR 1204).
    let secrets = crate::secrets::doctor_rows(repo, credentials);
    checks.extend(if lite { secrets.into_iter().map(lite_secrets_row).collect() } else { secrets });

    if let Some(cfg) = cfg {
        // 4. Each environment's transport, as the resolver would choose it.
        for env in cfg
            .environments
            .iter()
            .filter(|e| only_env.map(|o| o == e.name).unwrap_or(true))
        {
            let mut detail = match (env.converger.as_str(), env.transport.as_deref()) {
                ("pull-agent", _) => "pull-agent: no transport: the agent reaches out".to_string(),
                (_, Some("ssm")) => "push over the mesh when this machine is on the environment's profile; otherwise its SSM door (bootstrap/break-glass grade: ~20-minute idle timeout: not for a long play)".to_string(),
                (_, Some("direct")) | (_, None) => "push over the mesh (direct, SSH with the inventory)".to_string(),
                (_, Some(other)) => format!("transport '{other}' is not one this binary knows"),
            };
            if let Some(inv) = env.inventory.as_deref() {
                detail.push_str(&format!("; inventory {inv}"));
            }
            for site in &env.sites {
                let mut s = format!(
                    "; site {} ({}",
                    site.name,
                    site.role.as_deref().unwrap_or("founder")
                );
                if let Some(j) = site.joins.as_deref() {
                    s.push_str(&format!(" joins {j}"));
                }
                if let Some(c) = site.converger.as_deref() {
                    s.push_str(&format!(", converger {c}"));
                }
                let overrides: Vec<String> = site
                    .classes
                    .iter()
                    .filter_map(|(k, c)| c.converger.as_ref().map(|v| format!("{k}: {v}")))
                    .collect();
                if !overrides.is_empty() {
                    s.push_str(&format!(", class overrides [{}]", overrides.join(", ")));
                }
                s.push(')');
                detail.push_str(&s);
            }
            checks.push(ok(&format!("env:{}", env.name), detail));
            if env.converger != "pull-agent" {
                checks.push(reach_check(repo, env));
            }
        }

        // 5. The promotion path, stage by stage.
        let path: Vec<String> = cfg
            .promotion
            .path
            .iter()
            .map(|s| format!("{} ({})", s.name, s.kind))
            .collect();
        checks.push(ok("promotion", path.join(" → ")));

        // 6. Every component's deploy playbook resolves to a definite target mode.
        for (name, comp) in &cfg.components {
            checks.push(playbook_check(repo, name, comp));
        }

        // 7. The secrets each component's deploy templates from this environment,
        //    named before any host is touched (the class the first external
        //    deployer hit: one assert on the host per run).
        // What this machine can supply does not change between components, and
        // reading it means a posture call and a walk of .envrc.local.d: once.
        let mut have = crate::secrets::available(repo);
        have.extend(names_on_this_machine(repo));
        for env in cfg
            .environments
            .iter()
            .filter(|e| only_env.map(|o| o == e.name).unwrap_or(true))
            .filter(|e| e.converger != "pull-agent" && !e.groups.is_empty())
        {
            for (name, comp) in &cfg.components {
                if let Some(c) = secrets_check(repo, env, name, comp, &have) {
                    checks.push(c);
                }
            }
        }
    }

    Ok(Report::new("doctor", checks, EXIT_REFUSED))
}

fn checkout_check(repo: &Path, frozen: bool) -> Check {
    // An assembled tree (goatcore at the overlay's goatcore.ref with the overlay
    // copied over it) is on a detached HEAD by construction, and only its marker
    // says so. Without the marker a detached HEAD is the F-586 case below.
    match crate::fresh::marker(repo) {
        Ok(Some(m)) => return assembled_check(repo, &m),
        Ok(None) => {}
        Err(e) => {
            return refused(
                "checkout",
                format!("{e:#}: this tree carries {} but it does not say what the tree was built from", crate::fresh::MARKER),
                "re-assemble the tree from the overlay (its assemble step writes the marker)",
            )
        }
    }
    let branch = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
    if branch != "main" && branch != "master" {
        return refused(
            "checkout",
            format!("{} is on '{}': releases run from a main checkout (rc.sh deploys from the invoking checkout; F-586)", repo.display(), if branch.is_empty() { "a detached HEAD" } else { &branch }),
            "git checkout main && git pull --ff-only origin main",
        );
    }
    // A fetch is a read; it changes no file in the working tree.
    let _ = git(repo, &["fetch", "--quiet", "origin", &branch]);
    let behind = git(
        repo,
        &["rev-list", "--count", &format!("HEAD..origin/{branch}")],
    )
    .ok()
    .and_then(|s| s.trim().parse::<u64>().ok())
    .unwrap_or(0);
    if behind > 0 {
        let fix = format!("git pull --ff-only origin {branch}");
        let detail = format!(
            "local {branch} is {behind} commit(s) behind origin/{branch}{}",
            if frozen {
                " (--frozen: refusing rather than fast-forwarding)"
            } else {
                ""
            }
        );
        return refused("checkout", detail, &fix);
    }
    ok(
        "checkout",
        format!("on {branch}, current with origin/{branch}"),
    )
}

/// The checkout row for an assembled tree: what the marker says it was built
/// from. The marker is trusted as rc.sh and `release` trust it. Its currency
/// against the overlay is judged by the deploy's own fresh-source check
/// (ops/scripts/lib/source_of_record.py), not here; the refusals below are the
/// ones that check, `release` or rc.sh would make from the marker alone.
fn assembled_check(repo: &Path, m: &crate::fresh::Marker) -> Check {
    const REASSEMBLE: &str = "re-assemble the tree from the overlay (its assemble step writes the marker)";
    if m.goatcore_ref == "local" {
        return refused(
            "checkout",
            format!("{} was assembled from local directories (goatcore_ref=local), which no deploy accepts", repo.display()),
            "re-assemble the tree from the pinned refs",
        );
    }
    // source_of_record.py refuses it at deploy time (GB-064): no commit records
    // what the tree deploys.
    if m.overlay_commit.ends_with("-dirty") {
        return refused(
            "checkout",
            format!(
                "{} was assembled from uncommitted overlay changes (overlay_commit={}), so no commit records what this tree deploys",
                repo.display(),
                m.overlay_commit
            ),
            &format!("commit the overlay change, then {REASSEMBLE}"),
        );
    }
    let head = git(repo, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_default();
    if head != m.goatcore_ref {
        return refused(
            "checkout",
            format!(
                "{} carries {} naming goatcore {}, but its HEAD is {}: the tree is not what the marker says it was built from",
                repo.display(),
                crate::fresh::MARKER,
                m.goatcore_ref,
                if head.is_empty() { "unreadable" } else { &head }
            ),
            REASSEMBLE,
        );
    }
    let built = format!(
        "assembled tree ({}): built from {} (overlay commit {}); its detached HEAD is by construction, and the deploy's fresh-source check judges it against the overlay (C3)",
        crate::fresh::MARKER,
        m.describe(),
        m.overlay_commit
    );
    // release and rc.sh refuse (GB-009) a marker that names no overlay checkout
    // with an assemble.sh. A deploy does not: its preflight finds the overlay
    // through GOAT_OVERLAY_DIR. So this is a note, not a failure.
    let ovp = m.overlay_path.clone().unwrap_or_default();
    if ovp.is_empty() || !Path::new(&ovp).join("assemble.sh").is_file() {
        return Check {
            name: "checkout".to_string(),
            ok: true,
            detail: format!(
                "{built}; but the marker names no overlay checkout with an assemble.sh (overlay_path={}), so `release` and rc.sh refuse this tree (GB-009); a deploy finds the overlay through GOAT_OVERLAY_DIR",
                if ovp.is_empty() { "<none>" } else { &ovp }
            ),
            fix: Some(format!("{REASSEMBLE} so it records overlay_path=, or set GOAT_OVERLAY_DIR=<overlay checkout> for a deploy")),
        };
    }
    ok("checkout", built)
}

/// Who can deploy to a push environment, read from the files that decide it:
/// everyone on its mesh (a row for the environment in the mesh inventory), or
/// everyone holding the provider credentials (an `ssm` door in the door
/// registry). An environment with neither — or whose only door is an address
/// on one operator's own network — is deployable from one machine only.
/// Tailscale is the bad-day path (ADR 1114) and never a transport.
fn reach_check(repo: &Path, env: &config::Environment) -> Check {
    let inventory = repo.join(
        env.inventory
            .as_deref()
            .unwrap_or("ops/ansible/inventory/mgmt-hosts-mesh.ini"),
    );
    let doors = repo.join("ops/ansible/inventory/doors.yaml");
    let name = format!("env:{}:reach", env.name);
    let group = format!("[{}_hosts]", env.name);
    let (inv, reg) = (rel(repo, &inventory), rel(repo, &doors));
    match reach(&inventory, &doors, &env.name) {
        Ok(r) => match (r.mesh_row, r.door.as_deref()) {
            (true, Some("ssm")) => ok(
                &name,
                format!("{group} in {inv} and an ssm door in {reg}: deployable over the mesh or through the door"),
            ),
            (true, None) => ok(
                &name,
                format!("{group} in {inv}; no door in {reg}: deployable only from a machine on the environment's profile"),
            ),
            (false, Some("ssm")) => ok(
                &name,
                format!("no {group} in {inv}; an ssm door in {reg}: deployable through the door only"),
            ),
            // A LAN goatnet's declared transport (ADR 1213): the box is reached at
            // its inventory address by every deployer on the LAN; there is no door.
            (true, Some("direct")) => ok(
                &name,
                format!("{group} in {inv} and transport direct in {reg}: reached at its inventory address, no door"),
            ),
            (false, Some("direct")) => ok(
                &name,
                format!("transport direct in {reg}: reached at its inventory address, no door (no {group} in {inv} — the inventory this environment declares is what reaches it)"),
            ),
            (_, Some(other)) => refused(
                &name,
                format!("door type '{other}' in {reg} is not a deployer path: an address on one operator's own network reaches it from one machine (Tailscale is the bad-day path, ADR 1114)"),
                "declare an ssm door through the environment's cloud mesh peer in ops/ansible/inventory/doors.yaml, then ops/scripts/generate-door-inventory.py --all",
            ),
            (false, None) if env.transport.as_deref() == Some("direct") => refused(
                &name,
                format!("transport direct declared, but no {group} in {inv} and no entry in {reg}: nothing names the box's address yet"),
                "add the box to the inventory group (ansible_host=<lan address>) and a `type: direct` entry in ops/ansible/inventory/doors.yaml — a goatlite standup renders both from goatlite.yaml in phase 1",
            ),
            (false, None) => refused(
                &name,
                format!("no {group} in {inv} and no door in {reg}: no deployer's machine can reach it"),
                "add the environment's row to the mesh inventory (ansible_host=<mesh ip>) and declare its door in ops/ansible/inventory/doors.yaml",
            ),
        },
        Err(e) => refused(
            &name,
            format!("{e:#}"),
            "fix the file named, then re-run doctor",
        ),
    }
}

/// The same list, each name once, in reading order.
fn distinct(mut v: Vec<&str>) -> Vec<&str> {
    v.sort_unstable();
    v.dedup();
    v
}

/// The environment variables a component's deploy templates for an environment:
/// the `lookup('env', NAME)` bindings in the environment's group_vars, kept to
/// the variables the component's play and role actually reference. `None` when
/// the component has no playbook here or needs nothing; otherwise one check,
/// refused with every unset name and the one command that resolves them.
fn secrets_check(repo: &Path, env: &config::Environment, name: &str, comp: &Component, have: &std::collections::HashSet<String>) -> Option<Check> {
    let ansible = repo.join("ops/ansible");
    let ledger = comp.ledger.as_deref().unwrap_or(name);
    let start = [ansible.join(format!("{ledger}-deploy.yml")), ansible.join(format!("{name}-deploy.yml"))]
        .into_iter()
        .find(|p| p.is_file())?;
    let needed = secrets_needed(repo, &env.groups, name, ledger, &start);
    if needed.is_empty() {
        return None;
    }
    // Secrets load for one command at a time, so "not in this environment" is the
    // normal state. The question here is whether this machine can supply them,
    // which the caller read once for the whole run.
    // Two declarations can template the same variable (a role and its play, a
    // group and its host). It is one variable to supply, so it is counted and
    // named once, and the count is a count of variables.
    let all = distinct(needed.values().map(|s| s.as_str()).collect());
    let missing = distinct(needed.values().filter(|e| !satisfied(have, e)).map(|e| e.as_str()).collect());
    let check_name = format!("secrets:{}:{name}", env.name);
    if missing.is_empty() {
        Some(ok(&check_name, format!("{} variable(s) on this machine under the secrets posture: {}", all.len(), all.join(", "))))
    } else {
        Some(refused(
            &check_name,
            format!("{} of {} not on this machine: {}", missing.len(), all.len(), missing.join(", ")),
            &crate::secrets::hint(repo, &missing.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        ))
    }
}

/// The variable names this machine's local secret files declare. Only the name
/// left of `=` is read: a value or a reference never leaves the file.
fn names_on_this_machine(repo: &Path) -> std::collections::HashSet<String> {
    // The value must be real: `NAME=` (empty) and `NAME="${NAME:-$OTHER}"`
    // (a fallback that may itself be unset) declare nothing this machine holds.
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=["']?[^"'$\s]"#).unwrap());
    let mut files = vec![repo.join(".envrc")];
    let mut dirs = vec![repo.join(".envrc.local.d")];
    // The store in force, which is what the checkout declares unless the shell
    // overrode it (ADR 1204). Reading only the override meant a checkout that
    // declares openbao had its own refs directory skipped, and doctor then
    // reported secrets missing that were sitting right there.
    if crate::secrets::store(repo) == "openbao" {
        dirs.push(repo.join(".envrc.local.d/openbao"));
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "refs" || x == "envrc")));
        }
    }
    let mut names = std::collections::HashSet::new();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(f) {
            names.extend(text.lines().filter_map(|l| re.captures(l).map(|c| c[1].to_string())));
        }
    }
    names
}

/// ansible variable → ENV NAME, for the lookups the play (through one alias
/// hop) and the roles named `name` / `ledger` reference. Mirrors
/// ops/scripts/deploy-secrets-needed.py, which the driver runs.
pub fn secrets_needed(repo: &Path, groups: &[String], name: &str, ledger: &str, play: &Path) -> BTreeMap<String, String> {
    fn words_of(text: &str, into: &mut std::collections::HashSet<String>) {
        let mut cur = String::new();
        for ch in text.chars() {
            if ch.is_ascii_lowercase() || ch == '_' || (ch.is_ascii_digit() && !cur.is_empty()) {
                cur.push(ch);
            } else {
                if cur.len() > 1 {
                    into.insert(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
        if cur.len() > 1 {
            into.insert(cur);
        }
    }
    let mut words = std::collections::HashSet::new();
    let mut texts = Vec::new();
    if let Ok(t) = std::fs::read_to_string(play) {
        if let Some(i) = t.find("import_playbook:") {
            let imp = t[i + "import_playbook:".len()..].lines().next().unwrap_or("").trim().to_string();
            if let Ok(t2) = std::fs::read_to_string(play.parent().unwrap_or(repo).join(imp)) {
                texts.push(t2);
            }
        }
        texts.push(t);
    }
    let mut roles = vec![name.to_string()];
    if ledger != name {
        roles.push(ledger.to_string());
    }
    for r in roles {
        let dir = repo.join("ops/ansible/roles").join(r);
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Ok(t) = std::fs::read_to_string(&p) {
                        texts.push(t);
                    }
                }
            }
        }
    }
    for t in &texts {
        words_of(t, &mut words);
    }
    let mut out = BTreeMap::new();
    for g in groups {
        let dir = repo.join("ops/ansible/group_vars").join(g);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "yml" || x == "yaml").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            let Ok(t) = std::fs::read_to_string(&f) else { continue };
            for line in t.lines() {
                // A top-level key only, as the script reads it (`^name:`): an indented key is a
                // field inside a structure (a relay-health target can carry a `token:` lookup
                // of another goatnet's bearer), and the word `token` is in every role.
                let Some((var, rest)) = line.split_once(':') else { continue };
                if var.is_empty() || !var.chars().next().unwrap().is_ascii_lowercase() || var.contains(' ') {
                    continue;
                }
                let Some(i) = rest.find("lookup(") else { continue };
                let args = &rest[i + 7..];
                if !args.trim_start().starts_with("'env'") && !args.trim_start().starts_with("\"env\"") {
                    continue;
                }
                let Some(j) = args.find(',') else { continue };
                let envname: String = args[j + 1..].trim_start().trim_start_matches(['\'', '"']).chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
                if !envname.is_empty() && words.contains(var) {
                    out.insert(var.to_string(), envname);
                }
            }
        }
    }
    let mut roles = vec![name.to_string()];
    if ledger != name {
        roles.push(ledger.to_string());
    }
    for r in play_roles(play, 0) {
        if !roles.contains(&r) {
            roles.push(r);
        }
    }
    let roles = role_closure(repo, &roles);
    let play_text = std::fs::read_to_string(play).unwrap_or_default();
    for (var, alts) in role_default_bindings(repo, &roles, groups, &play_text) {
        out.entry(var).or_insert_with(|| alts.join("|"));
    }
    // The lookups the tasks make themselves (set_fact, an assert, a copy's content),
    // the asserts of a later phase included: a name already found above stays as found.
    let found: std::collections::HashSet<String> = out.values().flat_map(|a| a.split('|').map(String::from)).collect();
    for (var, alts) in task_bindings(repo, play, &roles, &group_scalars(repo, groups)) {
        if !out.contains_key(&var) && !alts.iter().any(|a| found.contains(a)) {
            out.insert(var, alts.join("|"));
        }
    }
    // An outbound channel whose empty value turns it off is never a need
    // (ops/secrets/off-when-empty.txt; pluggable-dependencies.md row 16).
    let off = off_when_empty(repo);
    out.retain(|_, alts| !alts.split('|').all(|a| off.contains(a)));
    out
}

/// The names in ops/secrets/off-when-empty.txt (first word of each non-comment
/// line). Mirrors `off_when_empty` in ops/scripts/deploy-secrets-needed.py.
pub fn off_when_empty(repo: &Path) -> std::collections::HashSet<String> {
    std::fs::read_to_string(repo.join("ops/secrets/off-when-empty.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('#').next().and_then(|w| w.split_whitespace().next()).map(String::from))
        .collect()
}

/// True when any one of `alts` (`A|B|C`, as [`secrets_needed`] renders a role
/// default that reads the first of several names) is available.
pub fn satisfied(have: &std::collections::HashSet<String>, alts: &str) -> bool {
    alts.split('|').any(|e| have.contains(e))
}

/// The scalar top-level values of the groups' vars files (site_id and the like).
pub(crate) fn group_scalars(repo: &Path, groups: &[String]) -> BTreeMap<String, String> {
    let re = regex::Regex::new(r"^([a-z][a-z0-9_]*):\s*(\S.*)$").unwrap();
    let mut out = BTreeMap::new();
    for g in groups {
        let dir = repo.join("ops/ansible/group_vars").join(g);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "yml" || x == "yaml").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            let Ok(t) = std::fs::read_to_string(&f) else { continue };
            for line in t.lines() {
                if let Some(c) = re.captures(line) {
                    let v = c[2].split(" #").next().unwrap_or("").trim().trim_matches(|ch| ch == '\'' || ch == '"').to_string();
                    out.entry(c[1].to_string()).or_insert(v);
                }
            }
        }
    }
    out
}

/// For a role-default value: the names any one of which satisfies it, or `None`
/// when it is not a need (a literal fallback, a shell name, or a name computed
/// from something unresolvable). Mirrors ops/scripts/deploy-secrets-needed.py.
///
/// `rvals` are the role's own scalar values, so a lookup whose argument is a variable
/// (`lookup('env', portal_db_password_env)`) resolves through it: the group's value
/// first, then the role's, holding a literal name or `{{ site_id … }}_SUFFIX`.
pub fn binding_alternatives(expr: &str, vars: &BTreeMap<String, String>, rvals: &BTreeMap<String, String>) -> Option<Vec<String>> {
    const NOT_SECRETS: [&str; 7] = ["HOME", "USER", "LOGNAME", "PATH", "PWD", "SHELL", "TMPDIR"];
    if regex::Regex::new(r#"default\(\s*['"]"#).unwrap().is_match(expr) {
        return None;
    }
    let lookup = regex::Regex::new(r#"lookup\(\s*['"]env['"]\s*,\s*"#).unwrap();
    // A literal name closes the lookup: `'ICAM_OIDC_' ~ (p.id …)` is computed, not a name.
    let literal = regex::Regex::new(r#"^['"]([A-Z][A-Z0-9_]*)['"]\s*\)"#).unwrap();
    let computed = regex::Regex::new(r#"^\(\s*(site_id_upper|site_id)\b(.*?)\)\s*[+~]\s*['"]([A-Z0-9_]+)['"]"#).unwrap();
    let variable = regex::Regex::new(r#"^([a-z][a-z0-9_]*)\s*\)"#).unwrap();
    let mut names = Vec::new();
    for m in lookup.find_iter(expr) {
        let rest = &expr[m.end()..];
        let n = if let Some(c) = literal.captures(rest) {
            c[1].to_string()
        } else if let Some(c) = variable.captures(rest) {
            name_in(&c[1], vars, rvals, 0)?
        } else {
            let c = computed.captures(rest)?;
            site_name(&c[1], &c[2], &c[3], vars)?
        };
        names.push(n);
    }
    if names.is_empty() || names.iter().any(|n| NOT_SECRETS.contains(&n.as_str())) {
        return None;
    }
    Some(names)
}

/// A computed name: site_id (or site_id_upper) through the filters, then the suffix.
fn site_name(base: &str, filters: &str, suffix: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let fallback = regex::Regex::new(r#"default\(\s*['"]([A-Z0-9_]+)['"]"#).unwrap();
    let val = if base == "site_id_upper" {
        vars.get("site_id_upper").cloned().filter(|v| !v.is_empty()).or_else(|| fallback.captures(filters).map(|d| d[1].to_string()))?
    } else {
        let sid = vars.get("site_id")?.to_uppercase();
        if filters.contains("replace") { sid.replace('-', "_") } else { sid }
    };
    (!val.contains('{')).then(|| format!("{val}{suffix}"))
}

/// The environment name variable `var` holds: the group's value first, then the
/// roles'; a literal name, `{{ site_id … }}_SUFFIX`, or one more variable
/// `{{ other_var | default('NAME') }}` (audit-subscriber's root-credential names
/// default to the minio role's `minio_root_user_envvar`). Mirrors `_name_in`.
fn name_in(var: &str, vars: &BTreeMap<String, String>, rvals: &BTreeMap<String, String>, depth: u8) -> Option<String> {
    let name_only = regex::Regex::new(r#"^[A-Z][A-Z0-9_]*$"#).unwrap();
    let templated = regex::Regex::new(r#"^\{\{\s*(site_id_upper|site_id)\b([^}]*)\}\}([A-Z0-9_]+)$"#).unwrap();
    let hop = regex::Regex::new(r#"^\{\{\s*([a-z][a-z0-9_]*)\s*(?:\|\s*default\(\s*['"]([A-Z0-9_]*)['"]\s*(?:,\s*true\s*)?\)\s*)?\}\}$"#).unwrap();
    let val = vars.get(var).filter(|v| !v.is_empty()).or_else(|| rvals.get(var)).cloned().unwrap_or_default();
    if name_only.is_match(&val) {
        return Some(val);
    }
    if let Some(t) = templated.captures(&val) {
        return site_name(&t[1], &t[2], &t[3], vars);
    }
    let t = hop.captures(&val).filter(|_| depth < 3)?;
    name_in(&t[1], vars, rvals, depth + 1).or_else(|| t.get(2).map(|m| m.as_str().to_string()).filter(|s| !s.is_empty()))
}

// --- env lookups in the tasks a deploy runs ---------------------------------------
// A role can read a secret in a task, not in its defaults: audit-subscriber resolves
// its S3 keys and the MinIO root pair with set_fact, then asserts them, the root pair
// in an included lifecycle.yml that runs after the container is recreated. So the
// tasks the play runs are walked too, `when:` conditions carried down. Mirrors the
// task walk in ops/scripts/deploy-secrets-needed.py.

const TASK_NOT_NEEDS: [&str; 18] = [
    "name", "when", "changed_when", "failed_when", "until", "msg", "fail_msg", "success_msg", "tags", "notify", "register", "loop",
    "with_items", "with_dict", "loop_control", "no_log", "become", "delegate_to",
];

fn load_yaml(path: &Path) -> serde_yaml::Value {
    std::fs::read_to_string(path).ok().and_then(|t| serde_yaml::from_str(&t).ok()).unwrap_or(serde_yaml::Value::Null)
}

fn scalar_text(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Bool(b) => Some(if *b { "True".into() } else { "False".into() }),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A variable's scalar value (group first, then the roles'), or None when unknown or templated.
fn var_value(var: &str, vars: &BTreeMap<String, String>, rvals: &BTreeMap<String, String>) -> Option<String> {
    let v = vars.get(var).or_else(|| rvals.get(var))?;
    (!v.contains('{')).then(|| v.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
}

/// True / false for the condition shapes the roles use on a variable; None when it
/// cannot be decided (the task then counts). Mirrors `eval_when`.
pub fn eval_when(cond: &str, vars: &BTreeMap<String, String>, rvals: &BTreeMap<String, String>) -> Option<bool> {
    let mut c = cond.trim();
    while c.starts_with('(') && c.ends_with(')') && c.matches('(').count() == 1 {
        c = c[1..c.len() - 1].trim();
    }
    if c.contains(" or ") {
        return None;
    }
    if c.contains(" and ") {
        let parts: Vec<Option<bool>> = c.split(" and ").map(|x| eval_when(x, vars, rvals)).collect();
        if parts.contains(&Some(false)) {
            return Some(false);
        }
        return if parts.contains(&None) { None } else { Some(true) };
    }
    if let Some(rest) = c.strip_prefix("not ") {
        return eval_when(rest, vars, rvals).map(|b| !b);
    }
    let re = regex::Regex::new(r#"^\(?\s*([a-z_][a-z0-9_]*)\s*(?:\|\s*default\([^)]*\)\s*)?(\|\s*(bool|length|trim)\s*)?\)?\s*(?:(==|!=|>=|>|<=|<)\s*(\S+))?$"#).unwrap();
    let m = re.captures(c)?;
    let val = var_value(&m[1], vars, rvals)?;
    let filt = m.get(3).map(|x| x.as_str());
    let op = m.get(4).map(|x| x.as_str());
    let rhs = m.get(5).map(|x| x.as_str()).unwrap_or("");
    if filt == Some("length") {
        let left = val.chars().count() as i64;
        let Some(op) = op else { return Some(left > 0) };
        let n: i64 = rhs.parse().ok()?;
        return Some(match op {
            "==" => left == n,
            "!=" => left != n,
            ">" => left > n,
            ">=" => left >= n,
            "<" => left < n,
            _ => left <= n,
        });
    }
    match op {
        Some("==") => return Some(val == rhs.trim_matches(|c| c == '\'' || c == '"')),
        Some("!=") => return Some(val != rhs.trim_matches(|c| c == '\'' || c == '"')),
        Some(_) => return None,
        None => {}
    }
    match val.to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" | "" => Some(false),
        _ => (filt != Some("bool")).then_some(true),
    }
}

fn conds_of(task: &serde_yaml::Value) -> Vec<serde_yaml::Value> {
    match task.get("when") {
        None | Some(serde_yaml::Value::Null) => vec![],
        Some(serde_yaml::Value::Sequence(s)) => s.clone(),
        Some(v) => vec![v.clone()],
    }
}

/// (nearest key, string) for every string leaf, skipping keys that never make a need.
fn strings_of(node: &serde_yaml::Value, key: Option<&str>, out: &mut Vec<(Option<String>, String)>) {
    match node {
        serde_yaml::Value::String(s) => out.push((key.map(String::from), s.clone())),
        serde_yaml::Value::Mapping(m) => {
            for (k, v) in m {
                let ks = k.as_str();
                if ks.map(|x| TASK_NOT_NEEDS.contains(&x) || x == "listen").unwrap_or(false) {
                    continue;
                }
                strings_of(v, ks.or(key), out);
            }
        }
        serde_yaml::Value::Sequence(s) => s.iter().for_each(|v| strings_of(v, key, out)),
        _ => {}
    }
}

struct TaskWalk<'a> {
    repo: &'a Path,
    vars: &'a BTreeMap<String, String>,
    rvals: BTreeMap<String, String>,
    out: BTreeMap<String, Vec<String>>,
    seen: std::collections::HashSet<(PathBuf, String)>,
    stack: Vec<String>,
}

impl TaskWalk<'_> {
    fn live(&self, conds: &[serde_yaml::Value]) -> bool {
        conds.iter().all(|c| match c {
            serde_yaml::Value::Bool(b) => *b,
            other => {
                let t = scalar_text(other).unwrap_or_default();
                !t.contains("lookup(") && eval_when(&t, self.vars, &self.rvals) != Some(false)
            }
        })
    }

    fn tasks(&mut self, items: Option<&serde_yaml::Value>, base: &Path, role: &str, conds: &[serde_yaml::Value]) {
        let Some(serde_yaml::Value::Sequence(items)) = items else { return };
        for t in items {
            if !t.is_mapping() {
                continue;
            }
            let mut here = conds.to_vec();
            here.extend(conds_of(t));
            let never = match t.get("tags") {
                Some(serde_yaml::Value::String(s)) => s.contains("never"),
                Some(serde_yaml::Value::Sequence(s)) => s.iter().any(|x| x.as_str() == Some("never")),
                _ => false,
            };
            if !self.live(&here) || never {
                continue;
            }
            if ["block", "rescue", "always"].iter().any(|k| t.get(*k).is_some()) {
                for k in ["block", "rescue", "always"] {
                    self.tasks(t.get(k), base, role, &here);
                }
                continue;
            }
            let inc = ["include_tasks", "import_tasks", "ansible.builtin.include_tasks", "ansible.builtin.import_tasks"].iter().find_map(|k| t.get(*k));
            if let Some(inc) = inc {
                let f = inc.get("file").and_then(|v| v.as_str()).or_else(|| inc.as_str());
                if let Some(f) = f.filter(|f| !f.contains('{')) {
                    self.file(&base.join(f), base, role, &here);
                }
                continue; // an include's vars: are parameters; the included file decides
            }
            let rinc = ["include_role", "import_role", "ansible.builtin.include_role", "ansible.builtin.import_role"].iter().find_map(|k| t.get(*k));
            if let Some(rinc) = rinc {
                if let Some(n) = rinc.get("name").and_then(|v| v.as_str()).filter(|n| !n.contains('{')) {
                    let entry = rinc.get("tasks_from").and_then(|v| v.as_str()).unwrap_or("main").to_string();
                    self.role(n, &here, &entry);
                }
                continue;
            }
            self.task(t, role);
        }
    }

    fn task(&mut self, t: &serde_yaml::Value, role: &str) {
        let ident = regex::Regex::new(r"^[a-z][a-z0-9_]*$").unwrap();
        let soft = regex::Regex::new(r"(?s)\bif\b.*\belse\b").unwrap();
        let mut facts: std::collections::HashSet<String> = std::collections::HashSet::new();
        for k in ["set_fact", "ansible.builtin.set_fact", "vars"] {
            if let Some(serde_yaml::Value::Mapping(m)) = t.get(k) {
                facts.extend(m.keys().filter_map(|x| x.as_str().map(String::from)));
            }
        }
        let mut leaves = Vec::new();
        strings_of(t, None, &mut leaves);
        for (key, text) in leaves {
            if !text.contains("lookup(") || soft.is_match(&text) {
                continue;
            }
            let Some(alts) = binding_alternatives(&text, self.vars, &self.rvals) else { continue };
            let var = match key {
                Some(k) if facts.contains(&k) && ident.is_match(&k) => k,
                _ => format!("{role}:{}", alts[0]),
            };
            self.out.entry(var).or_insert(alts);
        }
    }

    fn file(&mut self, path: &Path, base: &Path, role: &str, conds: &[serde_yaml::Value]) {
        let sig = (path.to_path_buf(), format!("{conds:?}"));
        if self.seen.contains(&sig) || self.seen.len() > 500 {
            return;
        }
        self.seen.insert(sig);
        let doc = load_yaml(path);
        self.tasks(Some(&doc), base, role, conds);
    }

    fn role(&mut self, name: &str, conds: &[serde_yaml::Value], entry: &str) {
        let rdir = self.repo.join("ops/ansible/roles").join(name);
        if !rdir.is_dir() || self.stack.iter().any(|r| r == name) {
            return;
        }
        self.stack.push(name.to_string());
        let meta = load_yaml(&rdir.join("meta/main.yml"));
        if let Some(serde_yaml::Value::Sequence(deps)) = meta.get("dependencies") {
            for dep in deps.clone() {
                let dname = dep.as_str().or_else(|| dep.get("role").and_then(|v| v.as_str())).map(String::from);
                if let Some(d) = dname.filter(|d| !d.contains('{')) {
                    let mut c = conds.to_vec();
                    c.extend(conds_of(&dep));
                    self.role(&d, &c, "main");
                }
            }
        }
        let tdir = rdir.join("tasks");
        for ext in ["yml", "yaml"] {
            let f = tdir.join(format!("{entry}.{ext}"));
            if f.is_file() {
                self.file(&f, &tdir, name, conds);
                break;
            }
        }
        self.stack.pop();
    }

    fn play(&mut self, path: &Path, depth: u8) {
        let doc = load_yaml(path);
        let base = path.parent().unwrap_or(self.repo).to_path_buf();
        let Some(plays) = doc.as_sequence() else { return };
        for p in plays {
            if !p.is_mapping() {
                continue;
            }
            if let Some(imp) = p.get("import_playbook").and_then(|v| v.as_str()) {
                let imp = base.join(imp);
                if imp.is_file() && imp != path && depth < 3 {
                    self.play(&imp, depth + 1);
                }
                continue;
            }
            let pc = conds_of(p);
            self.tasks(p.get("pre_tasks"), &base, "play", &pc);
            if let Some(serde_yaml::Value::Sequence(roles)) = p.get("roles") {
                for r in roles {
                    let n = r.as_str().or_else(|| r.get("role").and_then(|v| v.as_str())).or_else(|| r.get("name").and_then(|v| v.as_str()));
                    if let Some(n) = n.filter(|n| !n.contains('{')) {
                        let mut c = pc.clone();
                        c.extend(conds_of(r));
                        self.role(n, &c, "main");
                    }
                }
            }
            self.tasks(p.get("tasks"), &base, "play", &pc);
            self.tasks(p.get("post_tasks"), &base, "play", &pc);
        }
    }
}

/// The literal role names a play's `roles:` lists (through import_playbook).
fn play_roles(play: &Path, depth: u8) -> Vec<String> {
    let mut out = Vec::new();
    let doc = load_yaml(play);
    for p in doc.as_sequence().into_iter().flatten() {
        if let Some(imp) = p.get("import_playbook").and_then(|v| v.as_str()) {
            let imp = play.parent().unwrap_or(Path::new(".")).join(imp);
            if imp.is_file() && imp != play && depth < 3 {
                out.extend(play_roles(&imp, depth + 1));
            }
        }
        if let Some(serde_yaml::Value::Sequence(roles)) = p.get("roles") {
            for r in roles {
                let n = r.as_str().or_else(|| r.get("role").and_then(|v| v.as_str())).or_else(|| r.get("name").and_then(|v| v.as_str()));
                if let Some(n) = n.filter(|n| !n.contains('{')) {
                    out.push(n.to_string());
                }
            }
        }
    }
    out
}

/// The scalar top-level values of the roles' defaults and vars, first role first.
fn role_scalars(repo: &Path, roles: &[String]) -> BTreeMap<String, String> {
    let scalar = regex::Regex::new(r"^([a-z][a-z0-9_]*):\s*(\S.*)$").unwrap();
    let mut out = BTreeMap::new();
    for r in roles {
        for sub in ["defaults", "vars"] {
            let Ok(rd) = std::fs::read_dir(repo.join("ops/ansible/roles").join(r).join(sub)) else { continue };
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "yml" || x == "yaml").unwrap_or(false)).collect();
            files.sort();
            for f in files {
                let Ok(t) = std::fs::read_to_string(&f) else { continue };
                for line in t.lines() {
                    if let Some(c) = scalar.captures(line) {
                        let v = c[2].split(" #").next().unwrap_or("").trim().trim_matches(|ch| ch == '\'' || ch == '"').to_string();
                        out.entry(c[1].to_string()).or_insert(v);
                    }
                }
            }
        }
    }
    out
}

/// ansible var (or `<role>:<NAME>` for a lookup no variable holds) -> names, for the
/// env lookups in the tasks the play runs on this goatnet. Mirrors `task_bindings`.
fn task_bindings(repo: &Path, play: &Path, roles: &[String], vars: &BTreeMap<String, String>) -> BTreeMap<String, Vec<String>> {
    let mut w = TaskWalk { repo, vars, rvals: role_scalars(repo, roles), out: BTreeMap::new(), seen: Default::default(), stack: vec![] };
    w.play(play, 0);
    w.out
}

/// `roles` and every role they pull in: include_role / import_role by a literal
/// name in their tasks and handlers, and meta dependencies. Mirrors `role_closure`
/// in ops/scripts/deploy-secrets-needed.py.
fn role_closure(repo: &Path, roles: &[String]) -> Vec<String> {
    let included = regex::Regex::new(r#"(?m)(?:include_role|import_role):\s*\n\s+name:\s*['"]?([A-Za-z0-9_.-]+)['"]?\s*$"#).unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut queue: std::collections::VecDeque<String> = roles.iter().cloned().collect();
    while let Some(r) = queue.pop_front() {
        if seen.contains(&r) {
            continue;
        }
        seen.push(r.clone());
        let rdir = repo.join("ops/ansible/roles").join(&r);
        if !rdir.is_dir() {
            continue;
        }
        for sub in ["tasks", "handlers"] {
            let mut stack = vec![rdir.join(sub)];
            let mut found: Vec<PathBuf> = Vec::new();
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().map(|x| x == "yml" || x == "yaml").unwrap_or(false) {
                        found.push(p);
                    }
                }
            }
            found.sort();
            for p in found {
                let Ok(t) = std::fs::read_to_string(&p) else { continue };
                queue.extend(included.captures_iter(&t).map(|c| c[1].to_string()));
            }
        }
        if let Ok(t) = std::fs::read_to_string(rdir.join("meta/main.yml")) {
            let doc: serde_yaml::Value = serde_yaml::from_str(&t).unwrap_or_default();
            if let Some(deps) = doc.get("dependencies").and_then(|d| d.as_sequence()) {
                for dep in deps {
                    let name = dep.as_str().or_else(|| dep.get("role").and_then(|v| v.as_str()));
                    if let Some(n) = name.filter(|n| !n.contains('{')) {
                        queue.push_back(n.to_string());
                    }
                }
            }
        }
    }
    seen
}

/// ansible var -> names, for the env lookups in the defaults and vars of `roles`
/// that no member group overrides and that the play or the role's own tasks and
/// templates reference (goat-get's GOAT_CLI_RELEASE_TOKEN lives there).
fn role_default_bindings(repo: &Path, roles: &[String], groups: &[String], play_text: &str) -> BTreeMap<String, Vec<String>> {
    let word = regex::Regex::new(r"\b[a-z][a-z0-9_]+\b").unwrap();
    let binding = regex::Regex::new(r#"^([a-z][a-z0-9_]*):\s*["'](.*)["']\s*(#.*)?$"#).unwrap();
    let or_names = regex::Regex::new(r"#\s*or:\s*([A-Z][A-Z0-9_]*(?:\s*,\s*[A-Z][A-Z0-9_]*)*)").unwrap();
    // `# when: <var>=<value>`: needed only when the group's value (else the role's) is that.
    let when = regex::Regex::new(r"#.*?\bwhen:\s*([a-z][a-z0-9_]*)\s*=\s*([A-Za-z0-9_.-]+)").unwrap();
    let scalar = regex::Regex::new(r"^([a-z][a-z0-9_]*):\s*(\S.*)$").unwrap();
    let vars = group_scalars(repo, groups);
    let mut out = BTreeMap::new();
    for r in roles {
        let rdir = repo.join("ops/ansible/roles").join(r);
        let mut used: std::collections::HashSet<String> = word.find_iter(play_text).map(|m| m.as_str().to_string()).collect();
        let mut stack = vec![rdir.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    let n = p.file_name().and_then(|x| x.to_str()).unwrap_or("");
                    if n != "defaults" && n != "vars" {
                        stack.push(p);
                    }
                } else if let Ok(t) = std::fs::read_to_string(&p) {
                    used.extend(word.find_iter(&t).map(|m| m.as_str().to_string()));
                }
            }
        }
        let mut lines: Vec<String> = Vec::new();
        for sub in ["defaults", "vars"] {
            let Ok(rd) = std::fs::read_dir(rdir.join(sub)) else { continue };
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "yml" || x == "yaml").unwrap_or(false)).collect();
            files.sort();
            for f in files {
                let Ok(t) = std::fs::read_to_string(&f) else { continue };
                lines.extend(t.lines().map(str::to_string));
            }
        }
        let mut rvals: BTreeMap<String, String> = BTreeMap::new();
        for line in &lines {
            if let Some(c) = scalar.captures(line) {
                let v = c[2].split(" #").next().unwrap_or("").trim().trim_matches(|ch| ch == '\'' || ch == '"').to_string();
                rvals.entry(c[1].to_string()).or_insert(v);
            }
        }
        for line in &lines {
            let Some(c) = binding.captures(line) else { continue };
            let (var, expr) = (&c[1], &c[2]);
            if !expr.contains("lookup(") || vars.contains_key(var) || !used.contains(var) {
                continue;
            }
            let comment = c.get(3).map(|m| m.as_str()).unwrap_or("");
            if let Some(w) = when.captures(comment) {
                let actual = vars.get(&w[1]).filter(|v| !v.is_empty()).or_else(|| rvals.get(&w[1]));
                if actual.map(String::as_str) != Some(&w[2]) {
                    continue;
                }
            }
            if let Some(mut alts) = binding_alternatives(expr, &vars, &rvals) {
                if let Some(o) = or_names.captures(comment) {
                    for n in o[1].split(',').map(str::trim) {
                        if !alts.iter().any(|a| a == n) {
                            alts.push(n.to_string());
                        }
                    }
                }
                out.insert(var.to_string(), alts);
            }
        }
    }
    out
}

/// What the mesh inventory and the door registry say about an environment.
#[derive(Debug, PartialEq, Eq)]
pub struct Reach {
    /// The inventory has a `[<env>_hosts]` group with at least one `ansible_host=` row.
    pub mesh_row: bool,
    /// The door registry's `type` for the environment (an underscored name is
    /// looked up hyphenated first, then as written), or none.
    pub door: Option<String>,
}

pub fn reach(inventory: &Path, doors: &Path, env: &str) -> Result<Reach> {
    use std::io::ErrorKind::NotFound;
    let group = format!("[{env}_hosts]");
    let mesh_row = match std::fs::read_to_string(inventory) {
        Ok(text) => {
            let mut lines = text.lines().map(str::trim).skip_while(|l| *l != group);
            lines.next().is_some()
                && lines
                    .take_while(|l| !l.starts_with('['))
                    .any(|l| !l.starts_with('#') && l.contains("ansible_host="))
        }
        Err(e) if e.kind() == NotFound => false,
        Err(e) => return Err(anyhow!("{}: {e}", inventory.display())),
    };
    let door = match std::fs::read_to_string(doors) {
        Ok(text) => {
            let v: serde_yaml::Value = serde_yaml::from_str(&text)
                .map_err(|e| anyhow!("{}: {e}", doors.display()))?;
            let key = env.replace('_', "-");
            v.get("doors")
                .and_then(|d| d.get(key.as_str()).or_else(|| d.get(env)))
                .and_then(|d| d.get("type"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
        }
        Err(e) if e.kind() == NotFound => None,
        Err(e) => return Err(anyhow!("{}: {e}", doors.display())),
    };
    Ok(Reach { mesh_row, door })
}

/// Follow `import_playbook` aliases to the play that declares `hosts:` and name
/// the target mode the driver must pass — `-e <var>=<group>` for a mandatory
/// variable, `-l <group>` for literal hosts. A component whose playbook is not
/// derivable from its name is declared in fleet-pins and reported, not refused.
fn playbook_check(repo: &Path, name: &str, comp: &Component) -> Check {
    let ledger = comp.ledger.as_deref().unwrap_or(name);
    let declared = format!(
        "declared in {}{}{}",
        comp.declared_in,
        comp.var
            .as_deref()
            .map(|v| format!(", var {v}"))
            .unwrap_or_default(),
        comp.image_workflow
            .as_deref()
            .map(|w| format!(", image from {w}"))
            .unwrap_or_default()
    );
    let ansible = repo.join("ops/ansible");
    let candidates = [
        ansible.join(format!("{ledger}-deploy.yml")),
        ansible.join(format!("{name}-deploy.yml")),
    ];
    let Some(start) = candidates.iter().find(|p| p.is_file()).cloned() else {
        return ok(
            &format!("playbook:{name}"),
            format!("no <component>-deploy.yml by name: declared via fleet-pins deploy_playbook, or not deployed by the driver; {declared}"),
        );
    };
    match resolve_target_mode(&ansible, &start) {
        Ok(r) => {
            let via = if r.play != start {
                format!(" via {}", rel(repo, &r.play))
            } else {
                String::new()
            };
            // A play that takes no extra argument reads as that, not as an
            // empty value after a colon.
            let how = if r.mode.trim().is_empty() {
                " takes no extra arguments".to_string()
            } else {
                format!(": {}", r.mode)
            };
            ok(
                &format!("playbook:{name}"),
                format!("{}{via}{how}; {declared}", rel(repo, &start)),
            )
        }
        Err(e) => refused(
            &format!("playbook:{name}"),
            format!("{}: {e}", rel(repo, &start)),
            "point the alias at a play with a hosts: line, or give the play one",
        ),
    }
}

/// What the driver must pass to reach a play's hosts, after following aliases.
#[derive(Debug, PartialEq, Eq)]
pub struct TargetMode {
    pub play: PathBuf,
    pub mode: String,
}

/// Follow `import_playbook` — written either bare or as
/// `ansible.builtin.import_playbook:` — up to three hops to the play that
/// declares `hosts:`. A mandatory-variable play gets `-e <var>=<group>`,
/// whichever variable it names; literal hosts get `-l <group>`.
pub fn resolve_target_mode(ansible: &Path, start: &Path) -> Result<TargetMode> {
    const KEY: &str = "import_playbook:";
    let mut play = start.to_path_buf();
    for _ in 0..3 {
        let text =
            std::fs::read_to_string(&play).map_err(|e| anyhow!("{}: {e}", play.display()))?;
        if let Some(hosts) = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("hosts:"))
        {
            let value = hosts.trim_start_matches("hosts:").trim().trim_matches('"');
            let mode = match value.strip_prefix("{{") {
                Some(rest) => {
                    let var: String = rest
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                        .collect();
                    format!("-e {var}=<group>")
                }
                None => "-l <group>".to_string(),
            };
            return Ok(TargetMode { play, mode });
        }
        let Some(imp) = text
            .lines()
            .find_map(|l| l.find(KEY).map(|i| l[i + KEY.len()..].trim().to_string()))
        else {
            anyhow::bail!(
                "no hosts: line and no import_playbook in {}",
                play.display()
            );
        };
        play = play.parent().unwrap_or(ansible).join(imp);
    }
    anyhow::bail!(
        "more than three import_playbook hops from {}",
        start.display()
    )
}

fn which(tool: &str) -> bool {
    Cmd::new("sh")
        .args(["-c", &format!("command -v {tool} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn python_has(modules: &[&str]) -> Result<bool> {
    let code = format!("import {}", modules.join(", "));
    // Quiet: the python:deps row reports a missing module, not a traceback.
    let status = Cmd::new("python3").args(["-c", &code]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status()?;
    Ok(status.success())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Cmd::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git {} failed", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Every `op://` value in the environment has at least vault/item/field, or
/// the row names the variable and the way around it (catalog GB-031).
fn op_refs_check(vars: impl Iterator<Item = (String, String)>) -> Check {
    let mut bad: Vec<String> = Vec::new();
    let mut seen = 0usize;
    for (k, v) in vars {
        if let Some(rest) = v.strip_prefix("op://") {
            seen += 1;
            let segments = rest.split('/').filter(|s| !s.is_empty()).count();
            if segments < 3 {
                bad.push(k);
            }
        }
    }
    bad.sort();
    if bad.is_empty() {
        return ok(
            "env:op-refs",
            match seen {
                0 => "no op:// reference in the environment (op run will validate only the env-file)".to_string(),
                n => format!("{n} op:// reference(s) in the environment, each with vault/item/field"),
            },
        );
    }
    refused(
        "env:op-refs",
        format!(
            "incomplete op:// reference in {}: op run refuses the whole run on it (too few '/': vault, item and field are required)",
            bad.join(", ")
        ),
        &format!(
            "env -u {} op run --env-file=… -- <command>   (or complete the reference to op://<vault>/<item>/<field>)",
            bad.join(" -u ")
        ),
    )
}

/// Under the lite preset the browser toolchain is not needed to stand a goatnet up:
/// it serves the release acceptance rows, and a goatlite cuts no release.
fn live_rows_skipped() -> Check {
    ok("tool:live-rows", "skipped: declared: a goatlite cuts no release, so the browser rows do not run from this machine".to_string())
}

/// A row only a release reads, skipped under the lite preset with its reason.
fn release_row_skipped(name: &str, what: &str) -> Check {
    ok(name, format!("skipped: declared: a goatlite cuts no release, so this machine does not need {what}"))
}

/// Under the lite preset phase 3 of the standup writes the secrets file
/// (`goatbuilder secrets init`), so before the first standup its absence is
/// the expected state, named with the phase that produces it, not a refusal.
fn lite_secrets_row(c: Check) -> Check {
    if c.name == "secrets:env-file" && !c.ok && c.detail.contains("does not exist") {
        return ok(&c.name, format!("{}: produced by phase 3 of the standup", c.detail));
    }
    c
}

/// ansible-core supports only some Python releases as its controller: 2.20 runs
/// on 3.12 to 3.14 and stops with a traceback on 3.15, which is what Homebrew's
/// `python` formula installs. On PATH is therefore not enough; the row runs
/// `ansible-playbook --version` and names the interpreter it reports.
fn ansible_row(out: std::io::Result<std::process::Output>) -> Check {
    const FIX: &str = "ansible-core 2.20 runs on Python 3.12 to 3.14: recreate the venv with one (macOS: brew install python@3.14, then python3.14 -m venv --clear ~/.venvs/goatlite) and reinstall its modules (docs/tutorials/00-goatlite.md section 1)";
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let core = text.lines().next().unwrap_or("").trim().to_string();
            let py = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("python version = "))
                .map(|v| v.split_whitespace().next().unwrap_or(v).to_string());
            match py {
                Some(v) => ok("tool:ansible-playbook", format!("{core} on Python {v} (the push converger)")),
                None => ok("tool:ansible-playbook", format!("{core} (the push converger)")),
            }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string();
            refused("tool:ansible-playbook", format!("ansible-playbook is on PATH but does not start: {last}"), FIX)
        }
        Err(e) => refused("tool:ansible-playbook", format!("ansible-playbook is on PATH but cannot run: {e}"), FIX),
    }
}

/// The image-pull token (goatlite.md §5 row 7). The goatnet's images are public
/// on ghcr.io and a goatlite standup pulls every one without credentials, so
/// unset is the normal case and prints no note; set, phase 5 logs in with it
/// (a mirror or private registry that needs one).
fn image_token_row(set: bool) -> Check {
    if set {
        return ok("secret:GHCR_READ_TOKEN", "set: phase 5 logs in to the registry with it".to_string());
    }
    ok("secret:GHCR_READ_TOKEN", "not set: the goatnet's images are public and pull without it".to_string())
}

/// The toolchain the live rows need on this machine: the console e2e suite's
/// node_modules, a Playwright that answers, and a chromium under its
/// browsers directory. The fix is the smoke script's own install line.
/// Only a release runs the live rows, so unless `required` (doctor
/// `--browser-tests`) a missing piece is a note: the row passes and carries
/// its fix.
fn live_rows_check(repo: &Path, required: bool) -> Check {
    let c = live_rows_toolchain(repo);
    if c.ok || required {
        return c;
    }
    Check {
        ok: true,
        detail: format!("{}; optional unless a release runs the live rows (`goatbuilder doctor --browser-tests` makes this a failure)", c.detail),
        ..c
    }
}

fn live_rows_toolchain(repo: &Path) -> Check {
    let e2e = repo.join("ops/goat-id-console/web/e2e");
    let fix = format!("(cd {} && npm ci && npx playwright install chromium)", e2e.display());
    if !e2e.join("node_modules").is_dir() {
        return refused(
            "tool:live-rows",
            format!("{} has no node_modules: the live rows (intake-form-browser-submit, review-gate-accepts-open-field-invite) run onboard-intake-smoke.sh from this machine and it exits 2 before touching the goatnet", rel(repo, &e2e)),
            &fix,
        );
    }
    let version = Cmd::new("npx")
        .args(["--no-install", "playwright", "--version"])
        .current_dir(&e2e)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let Some(version) = version else {
        return refused(
            "tool:live-rows",
            format!("{}: `npx playwright --version` does not answer (node_modules present, Playwright not runnable)", rel(repo, &e2e)),
            &fix,
        );
    };
    if !chromium_installed() {
        return refused(
            "tool:live-rows",
            format!("{} has {version} but no chromium in the Playwright browsers directory", rel(repo, &e2e)),
            &fix,
        );
    }
    ok("tool:live-rows", format!("{} present with chromium: the live rows can run from this machine", version))
}

/// Run a command with a deadline, so a tool that waits on a keychain or a
/// browser prompt (aws-vault, op) cannot hang doctor. Returns (success, stdout+stderr).
fn run_with_deadline(cmd: &str, args: &[&str], secs: u64) -> (bool, String) {
    let child = Cmd::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (false, format!("{cmd}: {e}")),
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
                return (status.success(), out.trim().to_string());
            }
            Ok(None) if start.elapsed().as_secs() >= secs => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, format!("{cmd} did not answer within {secs}s (waiting on a prompt?)"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return (false, format!("{cmd}: {e}")),
        }
    }
}

/// When this checkout carries a refs file or OP_ACCOUNT is set, `op whoami`
/// has to answer: every secret the plays template resolves through it.
fn op_session_check(repo: &Path, deep: bool) -> Check {
    let refs = std::fs::read_dir(repo.join(".envrc.local.d"))
        .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".refs")).count())
        .unwrap_or(0);
    let account = std::env::var("OP_ACCOUNT").ok().filter(|v| !v.is_empty());
    if refs == 0 && account.is_none() {
        return ok("tool:op-session", "no .envrc.local.d/*.refs and OP_ACCOUNT unset: nothing here resolves through op".to_string());
    }
    if !which("op") {
        return refused("tool:op-session", "the 1Password CLI (op) is not on PATH, and this checkout resolves secrets through it".to_string(), "brew install 1password-cli, then: eval $(op signin)");
    }
    if !deep {
        // Asking op who is signed in can raise a biometric prompt where the
        // machine has one, and wait on a session where it does not. A read-only
        // command may do neither on every run.
        return ok(
            "tool:op-session",
            format!("not checked: op is on PATH and {refs} refs file(s) are here; whether a session is open was not read, so nothing prompted you"),
        );
    }
    let (okay, out) = run_with_deadline("op", &["whoami"], 15);
    if okay {
        return ok("tool:op-session", format!("op signed in ({} refs file(s){})", refs, account.map(|a| format!(", OP_ACCOUNT={a}")).unwrap_or_default()));
    }
    let pinned = crate::config::op_account(repo).map(|a| format!("   (OP_ACCOUNT={a}, which goatbuilder.yaml pins)")).unwrap_or_default();
    refused("tool:op-session", format!("op is not signed in: {}", out.lines().next().unwrap_or("")), &format!("eval $(op signin){pinned}"))
}

/// The AWS profile every avenue runs under: `GOAT_AWS_PROFILE` (the driver's
/// name, goatlite.md §5 row 2), else the deprecated `GOAT_AWS_VAULT_PROFILE`
/// with a note, else what the checkout declares (`deploy.aws_profile`), else
/// `default`. `get` is the environment, injectable for tests.
fn aws_profile(get: &dyn Fn(&str) -> Option<String>, declared: Option<String>) -> (String, Option<String>) {
    let set = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(p) = set("GOAT_AWS_PROFILE") {
        return (p, None);
    }
    if let Some(p) = set("GOAT_AWS_VAULT_PROFILE") {
        return (p.clone(), Some(format!("GOAT_AWS_VAULT_PROFILE is deprecated: set GOAT_AWS_PROFILE={p} (the one name every avenue reads)")));
    }
    (declared.unwrap_or_else(|| "default".to_string()), None)
}

/// Why the AWS rows are skipped, or `None` when the run reaches AWS: the operator
/// declared `GOAT_AWS_MODE=none`, or every declared environment is reached
/// directly (`transport: direct` on each) and no environment or site says `ssm`.
/// An environment with no transport declared keeps the rows: the default
/// resolver may still open an SSM door for it.
fn aws_rows_skipped(cfg: Option<&config::Config>, mode: Option<&str>) -> Option<String> {
    if mode == Some("none") {
        return Some("GOAT_AWS_MODE=none: no door, no Terraform tree, no AWS call in this run".to_string());
    }
    let cfg = cfg?;
    if cfg.environments.is_empty() {
        return None;
    }
    let all_direct = cfg.environments.iter().all(|e| e.transport.as_deref() == Some("direct"));
    let any_ssm = cfg.environments.iter().any(|e| e.transport.as_deref() == Some("ssm") || e.sites.iter().any(|s| s.transport.as_deref() == Some("ssm")));
    (all_direct && !any_ssm).then(|| format!(
        "every declared environment ({}) is reached directly at its inventory address; no ssm door",
        cfg.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")
    ))
}

/// Whether the AWS config declares the profile: a read of a file, which unlocks
/// nothing. `AWS_CONFIG_FILE` overrides the path, as the AWS tools read it.
fn aws_profile_declared(profile: &str) -> Option<bool> {
    let path = std::env::var_os("AWS_CONFIG_FILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".aws/config")))?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().map(str::trim).any(|l| l == format!("[profile {profile}]") || l == format!("[{profile}]")))
}

/// What the lite preset derived from the four facts (ADR 1213 D2), in one row,
/// so the learner sees every decision the preset made for them.
fn lite_summary(l: &config::Lite, store: Option<&str>) -> String {
    let internet = match &l.internet {
        None => "LAN only".to_string(),
        Some(i) => format!(
            "internet zone {}{}{} (declared; growing out is not built yet)",
            i.zone,
            i.aws_profile.as_deref().map(|p| format!(", aws profile {p}")).unwrap_or_default(),
            i.cloudflare_token_ref.as_deref().map(|r| format!(", cloudflare token ref {r}")).unwrap_or_default()
        ),
    };
    format!(
        "goatlite {}: one box {} as {}, administrator {}, zone {}, environment {} (group {}_hosts, site {}, class core), disk encryption {}{}{}, secrets store {}, {}",
        l.name,
        l.host,
        l.ssh_user,
        l.admin,
        l.zone(),
        l.env_name(),
        l.env_name(),
        l.name,
        if l.disk_encryption.unwrap_or(true) { "on" } else { "off (declared exclusion)" },
        l.data_disk.as_deref().map(|d| format!(" (data disk {d})")).unwrap_or_default(),
        l.ssh_pubkey.as_deref().map(|k| format!(", ssh public key {k}")).unwrap_or_default(),
        store.unwrap_or("env-file"),
        internet
    )
}

/// The aws-vault credential the doors and the deploy wrapper run under, or the
/// declared reason the row is skipped (never silent). `GOAT_AWS_PROFILE` names
/// the profile; the old `GOAT_AWS_VAULT_PROFILE` still works and says so.
///
/// Exercising the credential unlocks the operator's credential store: a Keychain
/// prompt on a Mac, a passphrase on stdin with aws-vault's `pass` or `file`
/// backend elsewhere. A read-only command may not do that on every run, so by
/// default this reads the AWS config and says so; `--credentials` exercises it.
fn aws_vault_check(repo: &Path, skip: Option<&str>, deep: bool) -> Vec<Check> {
    let mut out = Vec::new();
    let (profile, note) = aws_profile(&|k| std::env::var(k).ok(), crate::settings::declared_aws_profile(repo));
    if let Some(n) = note {
        out.push(ok("tool:aws-profile", n));
    }
    // A goatnet that declares it needs no AWS says so here, rather than the row
    // going quiet (ADR 1213: a LAN goatnet reaches no door).
    if let Some(reason) = skip {
        out.push(ok("tool:aws-vault", format!("sts get-caller-identity skipped: declared: {reason}")));
        return out;
    }
    let keychain = std::env::var("AWS_VAULT_KEYCHAIN_NAME").ok().filter(|v| !v.is_empty());
    if !which("aws-vault") {
        out.push(refused("tool:aws-vault", "aws-vault is not on PATH (the SSM doors and ansible-preflight run under it)".to_string(), &format!("brew install aws-vault && aws-vault add {profile}")));
        return out;
    }
    let keychain_note = keychain.as_ref().map(|k| format!("; AWS_VAULT_KEYCHAIN_NAME={k} is set in this environment")).unwrap_or_default();
    if !deep {
        out.push(match aws_profile_declared(&profile) {
            Some(false) => refused("tool:aws-vault", format!("no profile {profile} in the AWS config{keychain_note}"), &format!("aws-vault add {profile}")),
            // Declared, or the config could not be read: either way nothing was
            // unlocked, and the row says what was not checked.
            _ => ok("tool:aws-vault", format!("not checked: profile {profile} is declared; its credentials were not read, so nothing unlocked your credential store{keychain_note}")),
        });
        return out;
    }
    let (okay, text) = run_with_deadline("aws-vault", &["exec", &profile, "--no-session", "--", "aws", "sts", "get-caller-identity"], 25);
    if okay {
        out.push(ok("tool:aws-vault", format!("profile {profile} answers sts get-caller-identity{keychain_note}")));
        return out;
    }
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
    let fix = aws_vault_fix(&profile, keychain.is_some(), &text);
    out.push(refused("tool:aws-vault", format!("profile {profile} does not answer sts get-caller-identity: {first}{keychain_note}"), &fix));
    out
}

/// What to do about a credential that did not answer. It reads the error first,
/// and never leads with deleting the stored key: the usual causes (an expired
/// `aws login` session, a keychain name set by direnv) leave the key intact.
fn aws_vault_fix(profile: &str, keychain_set: bool, out: &str) -> String {
    if keychain_set {
        // The command proves the cause; the second sentence is the way out, or
        // the loop is plain doctor saying "use direnv" and direnv's doctor
        // saying "AWS is broken", forever (.envrc exports the keychain name).
        return format!("env -u AWS_VAULT_KEYCHAIN_NAME aws-vault exec {profile} --no-session -- aws sts get-caller-identity   (if that answers, AWS_VAULT_KEYCHAIN_NAME points aws-vault at the wrong keychain. For this machine: echo 'unset AWS_VAULT_KEYCHAIN_NAME' > .envrc.local.d/90-local.envrc, then direnv allow. Taking it out of .envrc itself is a committed choice for everyone.)");
    }
    if out.contains("aws login") {
        return "aws login".to_string();
    }
    format!("aws-vault list   (if {profile} is not listed, or its key was rotated: aws-vault add {profile}, which replaces the stored key without deleting it first)")
}

/// rc.sh and release resolve image digests with crane and discard its error,
/// so a missing crane reads as "image not found" for an image that exists.
fn crane_check() -> Check {
    if which("crane") {
        ok("tool:crane", "resolves image digests for a candidate".to_string())
    } else {
        refused("tool:crane", "crane is not on PATH: a candidate's digest cannot be resolved, and the refusal reads as the image not existing".to_string(), "brew install crane")
    }
}

/// Every runner that runs goat locally (`goat_via: ""`) names, through its
/// values file, the context the live rows mint invites from
/// (`goat_sponsor_context`, the non-person sponsor identity, ADR 1207); it has
/// to exist under ~/.goat/contexts with an onboard_endpoint, or the onboarding
/// calls go to the portal (HTTP 405).
fn goat_contexts_check(repo: &Path) -> Vec<Check> {
    goat_contexts_check_in(repo, std::env::var_os("GOAT_PROFILE_DIR").map(PathBuf::from))
}

/// `goat_contexts_check` with the goat profile directory given (`GOAT_PROFILE_DIR`,
/// else `~/.goat` when `None`), so a test passes its own instead of setting it.
fn goat_contexts_check_in(repo: &Path, profile_dir: Option<PathBuf>) -> Vec<Check> {
    let path = repo.join("ops/validation/goatnet-runners.yaml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return vec![refused("tool:goat-contexts", format!("{}: {e}", rel(repo, &path)), "restore ops/validation/goatnet-runners.yaml from main")],
    };
    let doc: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(d) => d,
        Err(e) => return vec![refused("tool:goat-contexts", format!("{}: {e}", rel(repo, &path)), "fix the yaml")],
    };
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let contexts = profile_dir.unwrap_or_else(|| home.join(".goat")).join("contexts");
    let mut out = Vec::new();
    let Some(goatnets) = doc.get("goatnets").and_then(|g| g.as_mapping()) else {
        return vec![refused("tool:goat-contexts", format!("{} has no goatnets:", rel(repo, &path)), "fix the yaml")];
    };
    for (key, runner) in goatnets {
        let key = key.as_str().unwrap_or("?");
        let via = runner.get("goat_via").and_then(|v| v.as_str()).unwrap_or("");
        if !via.is_empty() {
            out.push(ok(&format!("tool:goat-contexts:{key}"), format!("goat runs on {via} for this goatnet's rows; no local context needed")));
            continue;
        }
        let mut missing = Vec::new();
        let mut no_endpoint = Vec::new();
        let mut present = Vec::new();
        let values = runner.get("values").and_then(|v| v.as_str()).unwrap_or("");
        let values: serde_yaml::Value = std::fs::read_to_string(repo.join(values)).ok().and_then(|t| serde_yaml::from_str(&t).ok()).unwrap_or_default();
        if let Some(name) = values.get("goat_sponsor_context").and_then(|v| v.as_str()).filter(|n| !n.is_empty()) {
            let profile = contexts.join(name).join("profile.toml");
            match std::fs::read_to_string(&profile) {
                Ok(t) if t.contains("onboard_endpoint") => present.push(name.to_string()),
                Ok(_) => no_endpoint.push(name.to_string()),
                Err(_) => missing.push(name.to_string()),
            }
        }
        if missing.is_empty() && no_endpoint.is_empty() {
            out.push(ok(&format!("tool:goat-contexts:{key}"), format!("contexts {} present with onboard_endpoint", present.join(", "))));
            continue;
        }
        let mut detail = Vec::new();
        if !missing.is_empty() {
            detail.push(format!("no profile for context {} under {}", missing.join(", "), contexts.display()));
        }
        if !no_endpoint.is_empty() {
            detail.push(format!("context {} has no onboard_endpoint, so onboarding calls would go to the portal (HTTP 405)", no_endpoint.join(", ")));
        }
        out.push(refused(
            &format!("tool:goat-contexts:{key}"),
            detail.join("; "),
            "goat --context <name> profile init <bundle.cbor> from a current bundle for each named context (or GOAT_ONBOARD_ENDPOINT=<onboard url> for one run when only the endpoint is missing)",
        ));
    }
    out
}

/// Playwright keeps browsers under $PLAYWRIGHT_BROWSERS_PATH, else
/// ~/Library/Caches/ms-playwright (macOS) or ~/.cache/ms-playwright (Linux).
fn chromium_installed() -> bool {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        dirs.push(PathBuf::from(p));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Library/Caches/ms-playwright"));
        dirs.push(home.join(".cache/ms-playwright"));
    }
    dirs.iter().any(|d| {
        std::fs::read_dir(d)
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("chromium")))
            .unwrap_or(false)
    })
}

/// Every input a standup of `site` will reach, from one static scan
/// (ops/scripts/standup-required-inputs.py, the same check goat-standup.sh runs as a
/// phase-0 gate): one FAIL row per missing input or unset secret, all at once.
pub fn site_inputs(repo: &Path, site: &str) -> Vec<Check> {
    let script = repo.join("ops/scripts/standup-required-inputs.py");
    let out = Cmd::new("python3").arg(&script).arg(site).arg("--json").current_dir(repo).output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return vec![refused("inputs", format!("cannot run {}: {e}", rel(repo, &script)), "install python3 with PyYAML")],
    };
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return vec![refused("inputs", format!("the input scan for {site} produced no result: {err}"), "run ops/scripts/standup-required-inputs.py <site> to see why")];
        }
    };
    site_input_rows(site, &v)
}

fn site_input_rows(site: &str, v: &serde_json::Value) -> Vec<Check> {
    let list = |k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|y| y.as_str()).unwrap_or("").to_string();
    let by = |x: &serde_json::Value| {
        x.get("required_by").and_then(|y| y.as_array()).map(|a| a.iter().filter_map(|i| i.as_str()).take(2).collect::<Vec<_>>().join(", ")).unwrap_or_default()
    };
    let mut rows = Vec::new();
    for m in list("missing") {
        rows.push(refused(
            &format!("input:{}", s(&m, "group")),
            format!("{} is required and not set: required by {}", s(&m, "var"), by(&m)),
            &format!("set {} in ops/ansible/group_vars/{}/ (or the inventory)", s(&m, "var"), s(&m, "group")),
        ));
    }
    for m in list("secrets") {
        rows.push(refused(
            &format!("secret:{}", s(&m, "group")),
            format!("{} reads env {}, which the secrets posture does not supply: required by {}", s(&m, "var"), s(&m, "env"), by(&m)),
            &format!("put the reference for {} in your secrets posture and run under it (ops/scripts/with-secrets <command>)", s(&m, "env")),
        ));
    }
    for m in list("leaves") {
        rows.push(refused(
            &format!("leaf:{}", s(&m, "group")),
            format!("GB-071 {} is not at {}: required by {}", s(&m, "leaf"), s(&m, "looked"), by(&m)),
            &format!("mint it into the manifest out_dir: {}", s(&m, "mint")),
        ));
    }
    for m in list("conditional") {
        rows.push(ok(&format!("input:{}", s(&m, "group")), format!("MAYBE {}: required under a condition the scan cannot evaluate ({})", s(&m, "var"), by(&m))));
    }
    for m in list("skipped") {
        rows.push(ok(&format!("phase {}", s(&m, "phase")), format!("{}: --limit {} selects no host of {}, so the play runs on nothing", s(&m, "play"), s(&m, "limit"), s(&m, "group"))));
    }
    // What this same run produces (standup-required-inputs.py), one row per producing
    // phase: not a gap, and the reader sees what the run will do about it.
    let produced = list("produced");
    let mut phases: Vec<(String, String)> = produced.iter().map(|p| (s(p, "phase"), s(p, "step"))).collect();
    phases.dedup();
    let later: Vec<Check> = phases.iter().map(|(phase, step)| {
        let names: Vec<String> = produced.iter().filter(|p| s(p, "phase") == *phase).map(|p| s(p, "name")).collect();
        ok(&format!("later:phase {phase}"), format!("produced by phase {phase} of this run ({step}): {}", names.join(", ")))
    }).collect();
    if rows.iter().all(|r| r.ok) {
        let n = v.get("checked").and_then(|x| x.as_u64()).unwrap_or(0);
        rows.insert(0, ok(&format!("inputs:{site}"), format!("{n} requirements checked across the standup's plays; none missing")));
    }
    rows.extend(later);
    rows
}

fn rel(repo: &Path, p: &Path) -> String {
    p.strip_prefix(repo).unwrap_or(p).display().to_string()
}

/// The page a person reads: what passed as one count, then one entry per
/// problem with its fix (docs/design/goatbuilder-output.md rule 8). Every
/// `secrets:<env>:<component>` row has the same cause, so they are one entry.
/// A row for a check that did not run. Its detail says so in its first words,
/// which is the whole claim it makes: not a pass, not a failure. `--credentials`
/// is what runs them, so the page says so once rather than on every row.
pub(crate) fn not_checked(c: &Check) -> bool {
    c.skipped()
}

pub fn summary(report: &crate::Report, repo: &Path, env: Option<&str>, site: Option<&str>, color: bool) -> String {
    let fill = |words: &str, first: &str, rest: &str| crate::tutor::fill(words, first, rest).trim_end().to_string();
    let failed: Vec<&Check> = report.checks.iter().filter(|c| !c.ok).collect();
    let is_secret_row = |c: &Check| c.name.starts_with("secrets:") && c.name.matches(':').count() == 2;
    let is_session = |c: &Check| c.name == "tool:op-session" || c.name == "secrets:op-session";
    // One row per component per goatnet, so the same component appears once for
    // each: count the components, which is what the line calls them.
    let components = {
        let mut names: Vec<&str> = failed
            .iter()
            .filter(|c| is_secret_row(c))
            .filter_map(|c| c.name.rsplit(':').next())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.len()
    };
    let signed_out = failed.iter().any(|c| is_session(c));
    let has_refs = std::fs::read_dir(repo.join(".envrc.local.d"))
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".refs")))
        .unwrap_or(false);

    let mut again = String::from("goatbuilder doctor");
    if let Some(e) = env {
        again.push_str(&format!(" --env {e}"));
    }
    if let Some(s) = site {
        again.push_str(&format!(" --site {s}"));
    }
    let bold = |t: &str| bold_text(t, color);
    let mut entries: Vec<String> = Vec::new();
    let mark = if color { "\x1b[31m\u{2717}\x1b[0m" } else { "\u{2717}" };
    if components > 0 || signed_out {
        let title = if components > 0 {
            format!("{} {} component{} missing some", bold("Secrets:"), bold(&components.to_string()), if components == 1 { "" } else { "s" })
        } else {
            // The store names itself, as the line under it does.
            format!("{} the {} store is signed out", bold("Secrets:"), crate::secrets::store(repo))
        };
        // In the declared store's own words: a team on sops or an env file is
        // never told to sign in to 1Password (ADR 1204). The names stand in for
        // themselves, since `--all` is where they are listed.
        let (why, steps) = secrets_advice(components, signed_out, &crate::secrets::store(repo), crate::secrets::keeps_references(repo), repo, &["the names --all lists".to_string()]);
        let mut e = format!("  {mark} {title}\n{}\n", fill(&why, "    ", "    "));
        for (i, step) in steps.iter().enumerate() {
            let first = if i == 0 { "    Fix: " } else { "         " };
            // A command too long for the page breaks with ` \` before an option,
            // never inside a quoted value, so it still runs as printed.
            let laid = match step.split_once(" -H ") {
                Some((head, tail)) if 9 + step.chars().count() > 80 => format!("{first}{head} \\\n           -H {}", tail.replacen(" > ", " \\\n           > ", 1)),
                _ => fill(step, first, "         "),
            };
            e.push_str(&yellow_fix(&laid, color));
            e.push('\n');
        }
        entries.push(e);
    }
    let repo_text = repo.display().to_string();
    let plain = |t: &str| t.replace(&format!("{repo_text}/"), "").replace(&repo_text, "This checkout").replace(" \u{2014} ", ": ");
    for c in failed.iter().filter(|c| !is_secret_row(c) && !is_session(c)) {
        let title = match c.name.as_str() {
            "checkout" => "Checkout",
            "tool:live-rows" => "Browser tests",
            "tool:aws-vault" => "AWS credentials",
            other => other,
        };
        // The row's own name goes with its friendly title, so a reader can find
        // it again in `--all`, where the rows are named and not titled.
        let head = if title.eq_ignore_ascii_case(&c.name) { title.to_string() } else { format!("{title} ({})", c.name) };
        let mut e = format!("  {mark} {}\n{}\n", bold(&head), fill(&plain(&c.detail), "    ", "    "));
        if let Some(fix) = &c.fix {
            e.push_str(&yellow_fix(&fix_lines(&plain(fix)), color));
            e.push('\n');
        }
        entries.push(e);
    }

    let total = report.checks.len();
    let skipped: Vec<&Check> = report.checks.iter().filter(|c| not_checked(c)).collect();
    let passed = total - failed.len() - skipped.len();
    let heading = match (env, site) {
        (Some(e), _) => format!("{} {e}", bold("Doctor:")),
        (None, Some(s)) => format!("{} {s}", bold("Doctor:")),
        (None, None) => bold("Doctor"),
    };
    // Three numbers, and none of them borrows from another: what passed, what
    // failed, and what never ran. "All N passed" is only true when N is every
    // check there was.
    let verdict = match (entries.len(), skipped.len()) {
        (0, 0) => format!("All {} checks passed.", bold(&total.to_string())),
        (0, k) => format!("{} check{} passed, and {} did not run.", bold(&passed.to_string()), if passed == 1 { "" } else { "s" }, bold(&k.to_string())),
        (n, 0) => format!("{} check{} passed. {} problem{} found.", bold(&passed.to_string()), if passed == 1 { "" } else { "s" }, bold(&n.to_string()), if n == 1 { "" } else { "s" }),
        (n, k) => format!("{} check{} passed. {} problem{} found, and {} check{} did not run.", bold(&passed.to_string()), if passed == 1 { "" } else { "s" }, bold(&n.to_string()), if n == 1 { "" } else { "s" }, bold(&k.to_string()), if k == 1 { "" } else { "s" }),
    };
    // Which checkout was judged, since the fixes run from the top of it.
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = match repo_text.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => repo_text.clone(),
    };
    let label = if shown.chars().count() + 10 > 80 { format!("{}\n{shown}", bold("Checkout:")) } else { format!("{} {shown}", bold("Checkout:")) };
    let mut out = format!("{heading}\n{label}\n\n{verdict}\n");
    for e in &entries {
        out.push('\n');
        out.push_str(e);
    }
    // Not a problem with this machine, but a real deploy from this shell needs it.
    let refs_loaded = std::env::vars_os().any(|(_, v)| ["op://", "bao://", "bw://"].iter().any(|r| v.to_string_lossy().starts_with(r)));
    let resolved = std::env::var_os("GOAT_SECRETS_RESOLVED").is_some();
    if has_refs && !refs_loaded && !resolved {
        let note = fill("Note: This shell has not loaded .envrc. A real deploy needs it: run the deploy with direnv exec . in front of it.", "", "      ");
        out.push_str(&format!("\n{}\n", note.replacen("Note:", &bold("Note:"), 1)));
    }
    for c in report.checks.iter().filter(|c| c.ok && c.fix.is_some()) {
        let note = fill(&format!("Note: {}. Fix: {}", plain(&c.detail), plain(c.fix.as_deref().unwrap_or_default())), "", "    ");
        out.push_str(&format!("\n{}\n", note.replacen("Note:", &bold("Note:"), 1)));
    }
    if !skipped.is_empty() {
        // Named, so nobody has to pass --all to find out what was left out, and
        // nobody reads a green page as a statement about their credentials.
        let names: Vec<&str> = skipped.iter().map(|c| c.name.as_str()).collect();
        let line = fill(&format!("Not checked: {}. Nothing unlocked your credential store or asked for a password. To check these too, add --credentials.", names.join(", ")), "", "      ");
        out.push_str(&format!("\n{}\n", line.replacen("Not checked:", &bold("Not checked:"), 1)));
    }
    out.push_str(&format!("\n{} {again} --all", bold("To see every check:")));
    out
}

/// Why secrets are missing and what to run, in the words of the store this
/// checkout declares (`secrets: store:`, ADR 1204). Whether this machine holds
/// them does not depend on the shell, so a step is never a rerun of the check.
fn secrets_advice(components: usize, signed_out: bool, store: &str, references: bool, repo: &Path, names: &[String]) -> (String, Vec<String>) {
    // A reference store keeps pointers here and fetches values per command; the
    // others keep the values. Either way, the store says how to supply them,
    // and which kind this one is comes from the posture, not from a list here.
    let why = match (components > 0, signed_out, references) {
        (true, true, _) => format!("Some have no reference on this machine, and the {store} store is signed out. --all names them."),
        (true, false, true) => format!("Some have no reference on this machine. --all names them. The {store} store resolves them per command."),
        (true, false, false) => format!("Some are not in the {store} store. --all names them."),
        _ => format!("The goatbuilder CLI cannot read the {store} store until you sign in."),
    };
    let mut steps = Vec::new();
    if store == "1password" {
        // This team's references are a file in a repository, so name the command
        // that fetches it rather than describing it.
        if components > 0 {
            let owner = crate::config::owner(repo).unwrap_or_else(|| "<owner>".to_string());
            // Fetched aside and moved into place: a fresh clone has no folder, a
            // refs file is 0600, and a failed `gh` must not empty the one you have.
            steps.push("mkdir -p .envrc.local.d &&".to_string());
            steps.push(format!("gh api repos/{owner}/goat-credentials/contents/refs/goat.refs -H 'Accept: application/vnd.github.raw' > .envrc.local.d/00-goat.refs.new &&"));
            steps.push("chmod 600 .envrc.local.d/00-goat.refs.new &&".to_string());
            steps.push("mv .envrc.local.d/00-goat.refs.new .envrc.local.d/00-goat.refs".to_string());
        }
        if signed_out {
            steps.push("eval $(op signin)".to_string());
        }
        return (why, steps);
    }
    steps.extend(crate::secrets::hint_lines(repo, names));
    if steps.is_empty() && signed_out {
        steps.push(format!("sign in to the {store} store"));
    }
    (why, steps)
}

fn bold_text(t: &str, color: bool) -> String {
    if color {
        format!("\x1b[1m{t}\x1b[0m")
    } else {
        t.to_string()
    }
}

/// The fix text in yellow so it stands out, with a bold `Fix:` label. Laid out
/// first: the color codes must not count toward the width.
fn yellow_fix(block: &str, color: bool) -> String {
    let lines: Vec<String> = block
        .trim_end()
        .lines()
        .map(|l| {
            if !color || l.len() < 9 || !l.is_char_boundary(9) {
                return l.to_string();
            }
            let (label, text) = l.split_at(9);
            let label = label.replace("Fix:", &bold_text("Fix:", true));
            format!("{label}\x1b[33m{text}\x1b[0m")
        })
        .collect();
    lines.join("\n")
}

/// `Fix: <text>` inside the page. A command too long for a line breaks after
/// `&&`, where it still runs as printed in bash and zsh.
fn fix_lines(fix: &str) -> String {
    fix_lines_at(fix, "    Fix: ", "         ")
}

/// Whether a fix is something to run, rather than something to do. A command
/// must survive being copied, so it is never broken anywhere but at `&&` or
/// `--`; a sentence can wrap anywhere. The first word decides it: a path, a
/// subshell, or one of the commands this tool tells people to run. A wrong
/// guess costs a long line or a wrapped sentence, never a broken command.
pub(crate) fn is_command(fix: &str) -> bool {
    let head = fix.split_whitespace().next().unwrap_or("");
    // `OP_ACCOUNT=... op run ...`: an environment assignment is a command's
    // first word as much as the command's own name is.
    let assignment = head.split_once('=').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'));
    if assignment {
        return true;
    }
    // Only the first word, so a sentence that quotes a command ("sign in: eval
    // $(op signin)") is still a sentence.
    head.starts_with("(cd")
        || head.contains('/')
        || matches!(
            head,
            "git" | "gh" | "goatbuilder" | "goat" | "eval" | "npm" | "npx" | "python3" | "direnv"
                | "echo" | "aws-vault" | "aws" | "env" | "op" | "bao" | "ssh" | "sudo" | "docker"
                | "cargo" | "make" | "bats" | "cd" | "export" | "unset" | "chmod" | "mkdir"
        )
}

/// A fix laid out under `first` and `rest`, broken only where a shell reads the
/// break as one command still (`--`, `&&`), never inside a path or a quoted
/// value. A fix with no such place stays on one long line: a line a person can
/// copy is worth more than a line that fits and does not run.
pub(crate) fn fix_lines_at(fix: &str, first: &str, rest: &str) -> String {
    // A fix is sometimes several steps, written as one sentence with semicolons
    // ("put a reference line ...; sign in: ...; then run: ..."). Each step is a
    // line: it reads as the list of things to do that it is, and no step ends up
    // in the middle of another one's sentence.
    let steps: Vec<&str> = fix.split("; ").collect();
    if steps.len() > 1 && !is_command(fix) {
        let mut out = String::new();
        for (i, step) in steps.iter().enumerate() {
            out.push_str(&one_step(step, if i == 0 { first } else { rest }, rest));
        }
        return out;
    }
    one_step(fix, first, rest)
}

/// One step of a fix, laid out under `first` and `rest`.
fn one_step(fix: &str, first: &str, rest: &str) -> String {
    let one = crate::tutor::fill(fix, first, rest).trim_end().to_string();
    if !one.contains('\n') {
        return format!("{one}\n");
    }
    // `--` starts the command a wrapper runs: a safe place for ` \\`. The tail
    // may carry prose after the command, so it wraps like everything else.
    if let Some((head, tail)) = fix.split_once(" -- ") {
        let (in1, in2) = (format!("{rest}  "), format!("{rest}    "));
        let tail = crate::tutor::fill(&format!("-- {tail}"), &in1, &in2);
        return format!("{first}{head} \\\n{}\n", tail.trim_end());
    }
    if !fix.contains(" && ") {
        // Nowhere safe to break. A command then stays on one long line, because
        // a line a person can copy is worth more than a line that fits and does
        // not run; a sentence wraps, because a sentence always reads.
        return if is_command(fix) { format!("{first}{fix}\n") } else { format!("{one}\n") };
    }
    // One command per line, whole. A command wrapped inside itself does not run,
    // so a part too long for the page is left too long for the page.
    let parts: Vec<&str> = fix.split(" && ").collect();
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        let tail = if i + 1 < parts.len() { " &&" } else { "" };
        out.push_str(&format!("{}{part}{tail}\n", if i == 0 { first } else { rest }));
    }
    out
}

#[cfg(test)]
mod fix_layout_tests {
    use super::*;

    #[test]
    fn a_command_stays_whole_and_a_sentence_wraps() {
        // A command with a path longer than the page: broken only at `&&`, so
        // every line is a command a shell runs.
        let long = "(cd /very/long/path/that/goes/on/and/on/for/ever/and/ever/and/then/some/more && npm ci && npx playwright install chromium)";
        let laid = fix_lines_at(long, "        fix: ", "             ");
        for l in laid.lines() {
            assert!(l.trim_end().ends_with("&&") || l.contains("chromium)"), "a line that is not a command: {l}");
        }
        assert_eq!(laid.lines().count(), 3, "{laid}");
        // A sentence with a long list wraps to the page, because it only reads.
        let prose = "put a reference line for each name (NAME=op://<vault>/<item>/<field>) in .envrc.local.d/*.refs: ALPHA_TOKEN, BETA_TOKEN, GAMMA_TOKEN, DELTA_TOKEN, EPSILON_TOKEN, ZETA_TOKEN";
        let laid = fix_lines_at(prose, "        fix: ", "             ");
        assert!(laid.lines().count() > 2, "{laid}");
        assert!(laid.lines().all(|l| l.chars().count() <= 80), "{laid}");
        // The posture's three steps read as three lines, and the one that
        // quotes a command is still a sentence, so it wraps with the rest.
        let three = "put a reference line for each name (NAME=op://<vault>/<item>/<field>) in .envrc.local.d/<file>.refs (goat's are in the goat-credentials repository, refs/goat.refs); sign in: eval $(op signin); then run: ops/scripts/with-secrets <this same command>";
        let laid = fix_lines_at(three, "        fix: ", "             ");
        assert!(laid.lines().all(|l| l.chars().count() <= 80), "{laid}");
        assert!(laid.contains("\n             sign in: eval $(op signin)\n"), "{laid}");
        assert!(laid.lines().last().unwrap().contains("with-secrets"), "{laid}");
        // A command with nowhere safe to break is left long, and runs.
        let one = "gh api repos/o/r/contents/refs/goat.refs > .envrc.local.d/00-goat.refs-but-much-much-longer-than-the-page-allows";
        let laid = fix_lines_at(one, "        fix: ", "             ");
        assert_eq!(laid.lines().count(), 1, "{laid}");
        assert!(laid.contains(one), "{laid}");
    }
}

fn ok(name: &str, detail: String) -> Check {
    Check {
        name: name.to_string(),
        ok: true,
        detail,
        fix: None,
    }
}

fn refused(name: &str, detail: String, fix: &str) -> Check {
    Check {
        name: name.to_string(),
        ok: false,
        detail,
        fix: Some(fix.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// goat-get's release token lives in its role defaults, not group_vars: the
    /// deploy gate must require it, as any one of its three names.
    #[test]
    fn a_role_default_secret_is_required() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "ops/ansible/goat-get-deploy.yml", "- hosts: \"{{ target }}\"\n  roles: [goat-get]\n");
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\n");
        // The env names are assembled here, not written as lookup('env', 'NAME') literals,
        // so .envrc.example's generator does not take the fixture's names for real ones.
        let env = |n: &str| format!("lookup('env', '{n}')");
        let defaults = [
            format!("goat_get_github_token: \"{{{{ {} | default({}, true) | default({}, true) }}}}\"", env("GOAT_CLI_RELEASE_TOKEN"), env("GH_TOKEN"), env("GHCR_READ_TOKEN")),
            format!("goat_get_cache: \"{{{{ {} }}}}/.cache\"", env("HOME")),
            format!("goat_get_optional: \"{{{{ {} | default('', true) }}}}\"", env("FIXTURE_OPTIONAL")),
            "goat_get_nb: \"{{ lookup('env', (site_id | mandatory | upper | replace('-', '_')) + '_NB_API_TOKEN') }}\"".to_string(),
            format!("goat_get_key: \"{{{{ {} }}}}\"  # or: FIXTURE_MANY", env("FIXTURE_ONE")),
            format!("goat_get_unused: \"{{{{ {} }}}}\"", env("FIXTURE_UNREAD")),
        ];
        write(r, "ops/ansible/roles/goat-get/defaults/main.yml", &(defaults.join("\n") + "\n"));
        write(
            r,
            "ops/ansible/roles/goat-get/tasks/main.yml",
            "- debug: msg=\"{{ goat_get_github_token }} {{ goat_get_cache }} {{ goat_get_optional }} {{ goat_get_nb }} {{ goat_get_key }}\"\n",
        );
        let got = secrets_needed(r, &["dev_hosts".into()], "goat-get", "goat-get", &r.join("ops/ansible/goat-get-deploy.yml"));
        assert_eq!(got.get("goat_get_github_token").map(String::as_str), Some("GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN"));
        assert_eq!(got.get("goat_get_nb").map(String::as_str), Some("DEV_A_NB_API_TOKEN"));
        assert_eq!(got.get("goat_get_key").map(String::as_str), Some("FIXTURE_ONE|FIXTURE_MANY"));
        assert!(!got.contains_key("goat_get_cache"), "a shell path is not a secret");
        assert!(!got.contains_key("goat_get_optional"), "a literal fallback makes it optional");
        assert!(!got.contains_key("goat_get_unused"), "a binding nothing reads is not a need");
        let have: std::collections::HashSet<String> = ["GH_TOKEN".to_string()].into();
        assert!(satisfied(&have, "GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN"));
        assert!(!satisfied(&have, "DEV_A_NB_API_TOKEN"));
        // A member group that sets the variable itself takes it over.
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\ngoat_get_github_token: literal\n");
        let got = secrets_needed(r, &["dev_hosts".into()], "goat-get", "goat-get", &r.join("ops/ansible/goat-get-deploy.yml"));
        assert!(!got.contains_key("goat_get_github_token"));
    }

    /// A store role the component includes holds its database password, named
    /// through a variable and read only on Postgres (as goat-zenoh-portal-store's).
    #[test]
    fn an_included_store_roles_password_is_required_on_postgres_only() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "ops/ansible/app-deploy.yml", "- hosts: \"{{ target }}\"\n  roles: [app]\n");
        write(r, "ops/ansible/roles/app/tasks/main.yml", "- name: app | the store\n  ansible.builtin.include_role:\n    name: app-store\n    public: true\n");
        write(
            r,
            "ops/ansible/roles/app-store/defaults/main.yml",
            "app_store: sqlite\napp_db_password_env: \"{{ site_id | upper | replace('-', '_') }}_APP_DB_PASS\"\napp_db_password: \"{{ lookup('env', app_db_password_env) }}\"  # when: app_store=postgres\n",
        );
        write(r, "ops/ansible/roles/app-store/tasks/main.yml", "- debug: msg=\"{{ app_db_password | length }}\"\n");
        let play = r.join("ops/ansible/app-deploy.yml");
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\napp_store: postgres\n");
        let got = secrets_needed(r, &["dev_hosts".into()], "app", "app", &play);
        assert_eq!(got.get("app_db_password").map(String::as_str), Some("DEV_A_APP_DB_PASS"));
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\napp_store: sqlite\n");
        let got = secrets_needed(r, &["dev_hosts".into()], "app", "app", &play);
        assert!(!got.contains_key("app_db_password"), "a sqlite goatnet reads no database password");
    }

    /// A secret a task reads (set_fact, an assert), in a later phase reached through an
    /// include, or in a role the play lists, is part of what the deploy needs, as a
    /// condition false on the goatnet takes it out. Mirrors
    /// tests/test_deploy_secrets_task_lookups.py.
    #[test]
    fn a_secret_a_task_reads_is_required_before_any_host() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "ops/ansible/app-deploy.yml", "- hosts: \"{{ target | mandatory }}\"\n  roles:\n    - role: app\n    - role: app-client\n");
        write(
            r,
            "ops/ansible/roles/app/defaults/main.yml",
            "app_s3_endpoint: \"\"\napp_apply_policy: true\napp_s3_key_envvar: \"{{ site_id | upper | replace('-', '_') }}_APP_S3_KEY\"\napp_root_user_envvar: \"{{ store_root_user_envvar | default('') }}\"\napp_token_envvar: \"FIXTURE_OPTIONAL\"\n",
        );
        write(
            r,
            "ops/ansible/roles/app/tasks/main.yml",
            concat!(
                "- name: resolve the S3 key\n  ansible.builtin.set_fact:\n    app_s3_key: \"{{ lookup('env', app_s3_key_envvar) }}\"\n  when: app_s3_endpoint | length > 0\n",
                "- name: optional, guarded by its own presence\n  ansible.builtin.copy:\n    content: \"{{ lookup('env', app_token_envvar) }}\"\n    dest: /tmp/x\n  when: lookup('env', app_token_envvar) | length > 0\n",
                "- name: later phase\n  ansible.builtin.include_tasks:\n    file: policy.yml\n  when: app_apply_policy | bool\n",
            ),
        );
        write(r, "ops/ansible/roles/app/tasks/policy.yml", "- block:\n    - ansible.builtin.set_fact:\n        app_root_user: \"{{ lookup('env', app_root_user_envvar) }}\"\n");
        write(r, "ops/ansible/roles/app-client/tasks/main.yml", "- ansible.builtin.assert:\n    that: (lookup('env', 'FIXTURE_CLIENT') | length) >= 16\n");
        let play = r.join("ops/ansible/app-deploy.yml");
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\napp_s3_endpoint: http://store:9000\nstore_root_user_envvar: FIXTURE_ROOT_USER\n");
        let got = secrets_needed(r, &["dev_hosts".into()], "app", "app", &play);
        assert_eq!(got.get("app_s3_key").map(String::as_str), Some("DEV_A_APP_S3_KEY"));
        assert_eq!(got.get("app_root_user").map(String::as_str), Some("FIXTURE_ROOT_USER"), "a later phase, through an include");
        assert_eq!(got.get("app-client:FIXTURE_CLIENT").map(String::as_str), Some("FIXTURE_CLIENT"), "a role the play lists");
        assert!(!got.values().any(|v| v.contains("FIXTURE_OPTIONAL")), "a task guarded by its own lookup is optional");
        write(r, "ops/ansible/group_vars/dev_hosts/vars.yml", "site_id: dev-a\napp_apply_policy: false\n");
        let got = secrets_needed(r, &["dev_hosts".into()], "app", "app", &play);
        assert!(!got.contains_key("app_s3_key"), "no endpoint, no S3 key");
        assert!(!got.contains_key("app_root_user"), "the policy phase is off");
    }

    /// The real audit-subscriber play in durable-write mode on a synthetic goatnet.
    #[test]
    fn audit_subscriber_names_its_s3_and_store_root_pairs() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let a = r.join("ops/ansible");
        let copy = |rel: &str| {
            let src = repo.join("ops/ansible").join(rel);
            let mut stack = vec![src.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap().flatten() {
                    let p = e.path();
                    let dst = a.join(rel).join(p.strip_prefix(&src).unwrap());
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
                        std::fs::copy(&p, &dst).unwrap();
                    }
                }
            }
        };
        copy("roles/audit-subscriber");
        copy("roles/rollback-ledger");
        std::fs::copy(repo.join("ops/ansible/audit-subscriber-deploy.yml"), a.join("audit-subscriber-deploy.yml")).unwrap();
        let fixture = "site_id: fixture\naudit_subscriber_s3_endpoint: \"http://127.0.0.1:9000\"\nminio_root_user_envvar: \"FIXTURE_MINIO_ROOT_USER\"\nminio_root_password_envvar: \"FIXTURE_MINIO_ROOT_PASSWORD\"\n";
        write(r, "ops/ansible/group_vars/fixture_hosts/audit-subscriber.yml", fixture);
        let got = secrets_needed(r, &["fixture_hosts".into()], "audit-subscriber", "audit-subscriber", &a.join("audit-subscriber-deploy.yml"));
        let names: std::collections::BTreeSet<&str> = got.values().map(String::as_str).collect();
        for n in ["FIXTURE_AUDIT_SUBSCRIBER_S3_ACCESS_KEY", "FIXTURE_AUDIT_SUBSCRIBER_S3_SECRET_KEY", "FIXTURE_MINIO_ROOT_USER", "FIXTURE_MINIO_ROOT_PASSWORD"] {
            assert!(names.contains(n), "{n} missing from {got:?}");
        }
    }

    /// The real goat-get role: the token is part of what a deploy needs.
    #[test]
    fn goat_get_on_zagros_dev_needs_its_release_token() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let play = repo.join("ops/ansible/goat-get-deploy.yml");
        let got = secrets_needed(&repo, &["zagros_dev_hosts".into()], "goat-get", "goat-get", &play);
        assert_eq!(got.get("goat_get_github_token").map(String::as_str), Some("GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN"));
    }

    /// An outside alert channel left empty is off, never a missing secret
    /// (ops/secrets/off-when-empty.txt); the goatnet's own ntfy still is one.
    #[test]
    fn observability_on_zagros_dev_needs_no_outside_alert_channel() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !repo.join("ops/ansible/group_vars/zagros_dev_hosts").is_dir() {
            eprintln!("skipped: no zagros_dev_hosts in this tree (a goatcore checkout carries no goatnet)");
            return;
        }
        let play = repo.join("ops/ansible/obs.yml");
        let got = secrets_needed(&repo, &["zagros_dev_hosts".into()], "observability", "observability", &play);
        for var in ["pagerduty_integration_key", "slack_webhook_url", "smtp_smarthost", "smtp_auth_username", "smtp_auth_password", "email_digest_recipients"] {
            assert!(!got.contains_key(var), "{var} is an outside alert channel; empty turns it off");
        }
        assert!(got.contains_key("ntfy_alertmanager_webhook_url"), "control: the goatnet's own channel is still a need");
        assert!(off_when_empty(&repo).contains("PAGERDUTY_INTEGRATION_KEY"));
    }

    #[test]
    fn site_inputs_lists_every_missing_input_and_secret() {
        let v: serde_json::Value = serde_json::json!({
            "checked": 6,
            "missing": [{"group": "zagros_hosts", "var": "alpha_zone", "required_by": ["roles/alpha/tasks/main.yml"]},
                        {"group": "zagros_hosts", "var": "alpha_token", "required_by": ["roles/alpha/tasks/main.yml"]}],
            "secrets": [{"group": "zagros_hosts", "var": "alpha_secret_env", "env": "GALAXY_ALPHA_SECRET", "required_by": ["x"]}],
            "conditional": [], "skipped": []
        });
        let rows = site_input_rows("zagros", &v);
        assert_eq!(rows.iter().filter(|r| !r.ok).count(), 3);
        assert!(rows.iter().any(|r| r.detail.contains("alpha_zone")));
        assert!(rows.iter().any(|r| r.detail.contains("GALAXY_ALPHA_SECRET")));
    }

    #[test]
    fn site_inputs_names_a_missing_leaf_and_its_mint_command() {
        let v = serde_json::json!({
            "checked": 4, "missing": [], "secrets": [], "conditional": [], "skipped": [],
            "leaves": [{"group": "zagros_hosts", "leaf": "zagros/router-cert.pem",
                        "looked": "/home/op/.goat/state/zagros-leaves/router-cert.pem",
                        "mint": "ops/scripts/mint-site-certs.sh zagros --only router", "required_by": ["roles/goat-zenoh"]}]
        });
        let rows = site_input_rows("zagros", &v);
        let bad: Vec<_> = rows.iter().filter(|r| !r.ok).collect();
        assert_eq!(bad.len(), 1);
        assert!(bad[0].detail.contains("GB-071") && bad[0].detail.contains("zagros-leaves/router-cert.pem"));
        assert!(bad[0].fix.as_deref().unwrap_or("").contains("mint-site-certs.sh zagros --only router"));
    }

    #[test]
    fn site_inputs_clean_site_is_one_ok_row() {
        let v = serde_json::json!({"checked": 91, "missing": [], "secrets": [], "conditional": [], "skipped": []});
        let rows = site_input_rows("zagros-dev", &v);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].ok && rows[0].detail.contains("91 requirements"));
    }

    #[test]
    fn goat_contexts_check_names_missing_and_endpointless_contexts() {
        let t = crate::testdir::path("gb-contexts-");
        std::fs::create_dir_all(t.join("ops/validation")).unwrap();
        std::fs::write(t.join("ops/validation/goatnet-runners.yaml"), "schema: goatnet-runners/v0\ngoatnets:\n  a:\n    goat_via: \"\"\n    values: va.yaml\n  c:\n    goat_via: \"\"\n    values: vc.yaml\n  b:\n    goat_via: user@host\n    values: vb.yaml\n").unwrap();
        std::fs::write(t.join("va.yaml"), "goat_context: a\ngoat_sponsor_context: acceptance\n").unwrap();
        std::fs::write(t.join("vc.yaml"), "goat_sponsor_context: absent\n").unwrap();
        let profiles = t.join("goat");
        std::fs::create_dir_all(profiles.join("contexts/acceptance")).unwrap();
        std::fs::write(profiles.join("contexts/acceptance/profile.toml"), "portal = \"x\"\n").unwrap();
        let rows = goat_contexts_check_in(&t, Some(profiles.clone()));
        let a = rows.iter().find(|r| r.name == "tool:goat-contexts:a").unwrap();
        assert!(!a.ok);
        assert!(a.detail.contains("context acceptance has no onboard_endpoint"), "{}", a.detail);
        assert!(!a.detail.contains("context a "), "the fabric context is not an invite context: {}", a.detail);
        assert!(a.fix.as_deref().unwrap_or("").contains("profile init"));
        let c = rows.iter().find(|r| r.name == "tool:goat-contexts:c").unwrap();
        assert!(!c.ok && c.detail.contains("no profile for context absent"), "{}", c.detail);
        let b = rows.iter().find(|r| r.name == "tool:goat-contexts:b").unwrap();
        assert!(b.ok && b.detail.contains("user@host"));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn under_lite_the_browser_rows_are_skipped_as_declared() {
        let c = live_rows_skipped();
        assert!(c.ok && c.detail.starts_with("skipped: declared"), "{}", c.detail);
    }

    #[test]
    fn under_lite_the_release_rows_are_skipped_as_declared() {
        for (name, what) in [("tool:gh", "release tags"), ("tool:crane", "a candidate's digest")] {
            let c = release_row_skipped(name, what);
            assert!(c.ok && c.fix.is_none() && c.detail.starts_with("skipped: declared"), "{c:?}");
        }
    }

    #[test]
    fn under_lite_a_missing_secrets_file_is_phase_3s_and_other_rows_keep_their_grade() {
        let missing = Check { name: "secrets:env-file".into(), ok: false, detail: ".envrc.local.d/secrets.env does not exist".into(), fix: Some("goatbuilder secrets init --env <goatnet>".into()) };
        let c = lite_secrets_row(missing);
        assert!(c.ok && c.detail.ends_with("produced by phase 3 of the standup"), "{c:?}");
        let unreadable = Check { name: "secrets:env-file".into(), ok: false, detail: ".envrc.local.d/secrets.env is mode 0644".into(), fix: Some("chmod 600".into()) };
        assert!(!lite_secrets_row(unreadable).ok);
        let other = Check { name: "secrets:posture".into(), ok: false, detail: "does not exist".into(), fix: None };
        assert!(!lite_secrets_row(other).ok);
    }

    #[cfg(unix)]
    fn output(code: i32, stdout: &str, stderr: &str) -> std::process::Output {
        use std::os::unix::process::ExitStatusExt;
        std::process::Output { status: std::process::ExitStatus::from_raw(code << 8), stdout: stdout.into(), stderr: stderr.into() }
    }

    #[cfg(unix)]
    #[test]
    fn an_ansible_that_does_not_start_refuses_and_names_the_python_range() {
        let good = ansible_row(Ok(output(0, "ansible-playbook [core 2.20.11]\n  config file = None\n  python version = 3.14.10 (main) [Clang]\n", "")));
        assert!(good.ok && good.detail.contains("core 2.20.11") && good.detail.contains("Python 3.14.10"), "{good:?}");
        let crash = ansible_row(Ok(output(1, "", "Traceback (most recent call last):\nAttributeError: module 'dataclasses' has no attribute '_is_type'\n")));
        assert!(!crash.ok && crash.detail.contains("_is_type"), "{crash:?}");
        assert!(crash.fix.as_deref().unwrap_or("").contains("python@3.14"), "{crash:?}");
        let gone = ansible_row(Err(std::io::Error::from(std::io::ErrorKind::NotFound)));
        assert!(!gone.ok);
    }

    #[test]
    fn the_image_token_row_never_refuses_and_needs_no_token() {
        let unset = image_token_row(false);
        assert!(unset.ok && unset.fix.is_none() && unset.detail.contains("public"), "{unset:?}");
        let set = image_token_row(true);
        assert!(set.ok && set.fix.is_none());
    }

    #[test]
    fn what_a_later_phase_produces_passes_with_its_phase_and_a_real_gap_still_refuses() {
        let later = serde_json::json!({"checked": 3, "produced": [
            {"group": "g", "kind": "secret", "name": "G_PG_PASS", "phase": "3", "step": "generate per-instance secrets"},
            {"group": "g", "kind": "leaf", "name": "g/router-cert.pem", "phase": "4", "step": "mint leaf certs"},
            {"group": "g", "kind": "secret", "name": "G_NB_API_TOKEN", "phase": "11", "step": "first login"}]});
        let rows = site_input_rows("g", &later);
        assert!(rows.iter().all(|r| r.ok), "{rows:?}");
        assert!(rows.iter().any(|r| r.name == "later:phase 3" && r.detail.contains("G_PG_PASS")), "{rows:?}");
        assert!(rows.iter().any(|r| r.name == "later:phase 11" && r.detail.contains("G_NB_API_TOKEN")), "{rows:?}");
        let gap = serde_json::json!({"checked": 3, "secrets": [{"group": "g", "var": "v", "env": "G_OTHER", "required_by": ["a play"]}],
            "produced": [{"group": "g", "kind": "secret", "name": "G_PG_PASS", "phase": "3", "step": "s"}]});
        assert!(site_input_rows("g", &gap).iter().any(|r| !r.ok && r.detail.contains("G_OTHER")));
    }

    #[test]
    fn live_rows_check_names_the_install_line_when_node_modules_is_absent() {
        let t = crate::testdir::path("gb-live-rows-");
        std::fs::create_dir_all(t.join("ops/goat-id-console/web/e2e")).unwrap();
        let c = live_rows_check(&t, true);
        assert!(!c.ok);
        assert!(c.detail.contains("has no node_modules"), "{}", c.detail);
        assert!(c.fix.as_deref().unwrap_or("").contains("npm ci && npx playwright install chromium"));
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Not asked for (no --browser-tests), the missing toolchain is a note: the
    /// row passes, keeps its fix, and the page prints it under "Note:".
    #[test]
    fn absent_node_modules_is_a_note_unless_browser_tests_is_asked_for() {
        let t = crate::testdir::path("gb-live-rows-note-");
        std::fs::create_dir_all(t.join("ops/goat-id-console/web/e2e")).unwrap();
        let c = live_rows_check(&t, false);
        assert!(c.ok, "{c:?}");
        assert!(c.detail.contains("has no node_modules") && c.detail.contains("--browser-tests"), "{}", c.detail);
        assert!(c.fix.as_deref().unwrap_or("").contains("npm ci && npx playwright install chromium"));
        let r = crate::Report::new("doctor", vec![c], crate::EXIT_REFUSED);
        let p = summary(&r, &t, None, None, false);
        assert!(p.contains("All 1 checks passed."), "{p}");
        assert!(p.contains("Note: ops/goat-id-console/web/e2e has no node_modules"), "{p}");
        let r = crate::Report::new("doctor", vec![live_rows_check(&t, false)], crate::EXIT_REFUSED);
        assert_eq!(r.exit, 0);
        let r = crate::Report::new("doctor", vec![live_rows_check(&t, true)], crate::EXIT_REFUSED);
        assert_eq!(r.exit, crate::EXIT_REFUSED);
        let _ = std::fs::remove_dir_all(&t);
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git").args(args).current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@example.org")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@example.org")
            .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// A checkout on a detached HEAD at its one commit; returns (dir, commit).
    fn detached_checkout(tag: &str) -> (crate::testdir::Held<PathBuf>, String) {
        let t = crate::testdir::path(&format!("gb-doctor-{tag}-"));
        std::fs::create_dir_all(&t).unwrap();
        git_in(&t, &["init", "-q", "-b", "main"]);
        std::fs::write(t.join("x"), "1").unwrap();
        git_in(&t, &["add", "x"]);
        git_in(&t, &["commit", "-q", "-m", "base"]);
        let head = git_in(&t, &["rev-parse", "HEAD"]);
        git_in(&t, &["checkout", "-q", "--detach"]);
        (t, head)
    }

    #[test]
    fn an_assembled_tree_on_its_detached_head_passes_and_names_its_source() {
        let (t, head) = detached_checkout("assembled");
        let overlay = "0123456789abcdef0123456789abcdef01234567";
        let ov = t.join("overlay");
        std::fs::create_dir_all(&ov).unwrap();
        std::fs::write(ov.join("assemble.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(t.join(crate::fresh::MARKER), format!("goatcore_ref={head}\noverlay_commit={overlay}\noverlay_path={}\n", ov.display())).unwrap();
        let c = checkout_check(&t, false);
        assert!(c.ok && c.fix.is_none(), "{c:?}");
        assert!(c.detail.contains("assembled tree"), "{}", c.detail);
        assert!(c.detail.contains(&format!("goatcore {}", &head[..9])) && c.detail.contains(&format!("overlay commit {overlay}")), "{}", c.detail);
        assert_eq!(crate::Report::new("doctor", vec![c], crate::EXIT_REFUSED).exit, 0);
        // No overlay checkout with an assemble.sh: release and rc.sh would refuse
        // (GB-009) but a deploy would not, so the row passes with a note.
        std::fs::remove_file(ov.join("assemble.sh")).unwrap();
        for marker in [format!("goatcore_ref={head}\noverlay_commit={overlay}\noverlay_path={}\n", ov.display()), format!("goatcore_ref={head}\noverlay_commit={overlay}\n")] {
            std::fs::write(t.join(crate::fresh::MARKER), marker).unwrap();
            let c = checkout_check(&t, false);
            assert!(c.ok, "{c:?}");
            assert!(c.detail.contains("GB-009") && c.detail.contains("GOAT_OVERLAY_DIR"), "{}", c.detail);
            assert!(c.fix.as_deref().unwrap_or("").contains("overlay_path="), "{c:?}");
            assert_eq!(crate::Report::new("doctor", vec![c], crate::EXIT_REFUSED).exit, 0);
        }
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn an_assembled_tree_whose_head_is_not_the_marked_commit_or_is_local_refuses() {
        let (t, head) = detached_checkout("assembled-moved");
        let dirty = "0123456789abcdef0123456789abcdef01234567-dirty";
        std::fs::write(t.join(crate::fresh::MARKER), format!("goatcore_ref={head}\noverlay_commit={dirty}\n")).unwrap();
        let c = checkout_check(&t, false);
        assert!(!c.ok && c.detail.contains(&format!("overlay_commit={dirty}")), "{c:?}");
        assert!(c.fix.as_deref().unwrap_or("").contains("re-assemble"), "{c:?}");
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        std::fs::write(t.join(crate::fresh::MARKER), format!("goatcore_ref={other}\noverlay_commit=abc\n")).unwrap();
        let c = checkout_check(&t, false);
        assert!(!c.ok && c.detail.contains("not what the marker says"), "{c:?}");
        std::fs::write(t.join(crate::fresh::MARKER), "goatcore_ref=local\noverlay_commit=abc\n").unwrap();
        let c = checkout_check(&t, false);
        assert!(!c.ok && c.detail.contains("goatcore_ref=local"), "{c:?}");
        std::fs::write(t.join(crate::fresh::MARKER), "overlay_commit=abc\n").unwrap();
        let c = checkout_check(&t, false);
        assert!(!c.ok && c.detail.contains("no goatcore_ref= line"), "{c:?}");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// F-586 keeps its teeth: a plain checkout on a detached HEAD, with no
    /// marker, is refused with the same wording as before.
    #[test]
    fn a_plain_checkout_on_a_detached_head_still_refuses() {
        let (t, _) = detached_checkout("plain-detached");
        let c = checkout_check(&t, false);
        assert!(!c.ok, "{c:?}");
        assert!(c.detail.contains("is on 'a detached HEAD': releases run from a main checkout") && c.detail.contains("F-586"), "{}", c.detail);
        assert_eq!(c.fix.as_deref(), Some("git checkout main && git pull --ff-only origin main"));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn op_refs_check_names_the_incomplete_variable_and_the_way_around_it() {
        let env = vec![
            ("GOOD".to_string(), "op://vault/item/field".to_string()),
            ("OTHER_PROJECT_GCP".to_string(), "op://vault/incomplete-item".to_string()),
            ("PLAIN".to_string(), "not a ref".to_string()),
        ];
        let c = op_refs_check(env.into_iter());
        assert!(!c.ok);
        assert!(c.detail.contains("OTHER_PROJECT_GCP"), "{}", c.detail);
        assert!(!c.detail.contains("GOOD"));
        assert!(c.fix.as_deref().unwrap_or("").contains("env -u OTHER_PROJECT_GCP"));
        let clean = op_refs_check(vec![("GOOD".to_string(), "op://v/i/f".to_string())].into_iter());
        assert!(clean.ok);
    }

    fn ansible() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible")
    }

    /// The F-587 shape: an alias written as `ansible.builtin.import_playbook:`.
    #[test]
    fn alias_to_mandatory_target_variable() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let a = ansible();
        let r = resolve_target_mode(&a, &a.join("microscope-deploy.yml")).unwrap();
        assert_eq!(r.mode, "-e target=<group>");
        assert!(r.play.ends_with("microscope.yml"));
    }

    #[test]
    fn alias_to_a_differently_named_variable() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let a = ansible();
        let r = resolve_target_mode(&a, &a.join("airdefenders-deploy.yml")).unwrap();
        assert_eq!(r.mode, "-e airdefenders_host_group=<group>");
    }

    #[test]
    fn literal_hosts_is_a_limit() {
        let a = ansible();
        let r = resolve_target_mode(&a, &a.join("crutch-tier-deploy.yml")).unwrap();
        assert_eq!(r.mode, "-l <group>");
        assert!(r.play.ends_with("crutch-tier-deploy.yml"));
    }

    #[test]
    fn missing_play_is_an_error() {
        let a = ansible();
        assert!(resolve_target_mode(&a, &a.join("no-such-deploy.yml")).is_err());
    }

    /// A throwaway repository holding only the two files `reach` reads.
    fn reach_repo(tag: &str, ini: Option<&str>, doors: Option<&str>) -> crate::testdir::Held<PathBuf> {
        let repo = crate::testdir::path(&format!("goatbuilder-reach-{tag}-"));
        let inv = repo.join("ops/ansible/inventory");
        std::fs::create_dir_all(&inv).unwrap();
        for f in ["mgmt-hosts-mesh.ini", "doors.yaml"] {
            let _ = std::fs::remove_file(inv.join(f));
        }
        if let Some(t) = ini {
            std::fs::write(inv.join("mgmt-hosts-mesh.ini"), t).unwrap();
        }
        if let Some(t) = doors {
            std::fs::write(inv.join("doors.yaml"), t).unwrap();
        }
        repo
    }

    fn env(name: &str) -> config::Environment {
        config::Environment {
            name: name.to_string(),
            converger: "push-ansible".to_string(),
            transport: None,
            inventory: None,
            groups: vec![],
            sites: vec![],
            peering_edge_reserved_udp: None,
            peering_directory: None,
        }
    }

    const MESH_ROW: &str = "[zagros_hosts]\nmgmt-zagros-01 ansible_host=100.127.0.45\n\n[zagros_dev_hosts]\n# the Thor\nmgmt-zagros-dev-01-thor ansible_host=100.127.0.155\n";

    #[test]
    fn mesh_row_and_ssm_door_is_reachable_two_ways() {
        let repo = reach_repo("both", Some(MESH_ROW), Some("doors:\n  zagros-dev:\n    type: ssm\n    port: 12203\n"));
        let r = reach(
            &repo.join("ops/ansible/inventory/mgmt-hosts-mesh.ini"),
            &repo.join("ops/ansible/inventory/doors.yaml"),
            "zagros_dev",
        )
        .unwrap();
        assert_eq!(r, Reach { mesh_row: true, door: Some("ssm".into()) });
        let c = reach_check(&repo, &env("zagros_dev"));
        assert!(c.ok, "{}", c.detail);
        assert!(c.detail.contains("over the mesh or through the door"));
    }

    /// No mesh row, and the only door an address on one operator's own network.
    #[test]
    fn a_one_machine_door_is_refused() {
        let repo = reach_repo("tailscale", Some("[zagros_hosts]\nx ansible_host=100.127.0.45\n"), Some("doors:\n  zagros-dev:\n    type: tailscale\n    ip: 100.127.1.7\n"));
        let c = reach_check(&repo, &env("zagros_dev"));
        assert!(!c.ok);
        assert!(c.detail.contains("door type 'tailscale'"), "{}", c.detail);
        assert!(c.fix.as_deref().unwrap().contains("doors.yaml"));
    }

    #[test]
    fn nothing_declared_is_refused_and_a_group_without_a_row_does_not_count() {
        let repo = reach_repo("none", Some("[zagros_dev_hosts]\n# TBD\n\n[zagros_hosts]\nx ansible_host=100.127.0.45\n"), None);
        let r = reach(
            &repo.join("ops/ansible/inventory/mgmt-hosts-mesh.ini"),
            &repo.join("ops/ansible/inventory/doors.yaml"),
            "zagros_dev",
        )
        .unwrap();
        assert_eq!(r, Reach { mesh_row: false, door: None });
        let c = reach_check(&repo, &env("zagros_dev"));
        assert!(!c.ok);
        assert!(c.detail.contains("no deployer's machine"), "{}", c.detail);
    }

    #[test]
    fn mesh_row_alone_is_reachable_from_the_profile_only() {
        let repo = reach_repo("mesh-only", Some(MESH_ROW), Some("doors: {}\n"));
        let c = reach_check(&repo, &env("zagros_dev"));
        assert!(c.ok, "{}", c.detail);
        assert!(c.detail.contains("profile"));
    }

    /// A LAN goatnet (ADR 1213): transport `direct` in the door registry is
    /// accepted — reached at its inventory address, no door — with or without a
    /// mesh row; `tailscale` stays refused.
    #[test]
    fn a_direct_transport_is_accepted_as_no_door() {
        let repo = reach_repo("direct", Some("[pasture_hosts]\npasture-box ansible_host=192.0.2.50\n"), Some("doors:\n  pasture:\n    type: direct\n"));
        let c = reach_check(&repo, &env("pasture"));
        assert!(c.ok, "{}", c.detail);
        assert!(c.detail.contains("transport direct") && c.detail.contains("no door"), "{}", c.detail);
        let repo = reach_repo("direct-norow", Some("[zagros_hosts]\nx ansible_host=100.127.0.45\n"), Some("doors:\n  pasture:\n    type: direct\n"));
        let c = reach_check(&repo, &env("pasture"));
        assert!(c.ok, "{}", c.detail);
        assert!(c.detail.contains("no door"), "{}", c.detail);
    }

    fn cfg_with(envs: Vec<config::Environment>) -> config::Config {
        let base = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/standup/goatbuilder.yaml")).unwrap();
        config::Config { environments: envs, ..base }
    }

    fn env_t(name: &str, transport: Option<&str>) -> config::Environment {
        config::Environment { transport: transport.map(str::to_string), ..env(name) }
    }

    /// goatlite.md §5 row 1: the AWS rows are skipped, as declared, under
    /// GOAT_AWS_MODE=none or when every environment is reached directly and no
    /// door is ssm; a default (undeclared) transport or any ssm keeps them.
    #[test]
    fn aws_rows_are_skipped_only_when_declared_or_when_nothing_reaches_aws() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        assert!(aws_rows_skipped(None, Some("none")).unwrap().contains("GOAT_AWS_MODE=none"));
        assert_eq!(aws_rows_skipped(None, None), None);
        let direct = cfg_with(vec![env_t("pasture", Some("direct"))]);
        assert!(aws_rows_skipped(Some(&direct), None).unwrap().contains("pasture"));
        assert!(aws_rows_skipped(Some(&direct), Some("aws-vault")).is_some(), "the mode only forces a skip, never a keep");
        let mixed = cfg_with(vec![env_t("pasture", Some("direct")), env_t("zagros", Some("ssm"))]);
        assert_eq!(aws_rows_skipped(Some(&mixed), None), None);
        let undeclared = cfg_with(vec![env_t("pasture", Some("direct")), env_t("zagros", None)]);
        assert_eq!(aws_rows_skipped(Some(&undeclared), None), None, "an undeclared transport may still open an SSM door");
        let mut site_ssm = env_t("pasture", Some("direct"));
        site_ssm.sites.push(config::Site { name: "pasture".into(), role: None, joins: None, converger: None, transport: Some("ssm".into()), classes: Default::default() });
        assert_eq!(aws_rows_skipped(Some(&cfg_with(vec![site_ssm])), None), None, "a site's ssm keeps the rows");
        // goat's own config declares no transports: its doctor keeps the AWS rows.
        let own = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/goatbuilder.yaml")).unwrap();
        assert_eq!(aws_rows_skipped(Some(&own), None), None);
        // The skipped rows are rows, not silence, and never refuse.
        let rows = aws_vault_check(Path::new("."), Some("test reason"), false);
        assert!(rows.iter().any(|c| c.name == "tool:aws-vault" && c.ok && c.detail.contains("skipped: declared: test reason")), "{rows:?}");
    }

    /// The fixture's four facts read back as every decision the preset made.
    #[test]
    fn the_lite_row_names_every_derived_decision() {
        let cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml")).unwrap();
        let row = lite_summary(cfg.lite.as_ref().unwrap(), cfg.secrets.as_ref().map(|s| s.store.as_str()));
        assert_eq!(row, "goatlite pasture: one box pasture-box.lan as ubuntu, administrator admin@example.org, zone pasture.lan, environment pasture (group pasture_hosts, site pasture, class core), disk encryption on, secrets store env-file, LAN only");
        let mut l = cfg.lite.unwrap();
        l.disk_encryption = Some(false);
        l.internet = Some(config::LiteInternet { zone: "pasture.example.org".into(), aws_profile: Some("pasture".into()), cloudflare_token_ref: None });
        l.data_disk = Some("/dev/sda".into());
        l.ssh_pubkey = Some("~/.ssh/id_ed25519.pub".into());
        let row = lite_summary(&l, Some("sops"));
        assert!(row.contains("disk encryption off (declared exclusion) (data disk /dev/sda), ssh public key ~/.ssh/id_ed25519.pub") && row.contains("secrets store sops") && row.contains("internet zone pasture.example.org, aws profile pasture (declared"), "{row}");
    }

    /// goatlite.md §5 row 2: one variable name. GOAT_AWS_PROFILE wins; the old
    /// GOAT_AWS_VAULT_PROFILE still works and is named as deprecated; default usareur-af.
    #[test]
    fn the_aws_profile_is_read_from_one_name_with_the_old_one_deprecated() {
        let get = |vars: &'static [(&str, &str)]| move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(aws_profile(&get(&[]), None), ("default".to_string(), None));
        assert_eq!(aws_profile(&get(&[("GOAT_AWS_PROFILE", "pasture")]), None), ("pasture".to_string(), None));
        assert_eq!(aws_profile(&get(&[("GOAT_AWS_PROFILE", "pasture"), ("GOAT_AWS_VAULT_PROFILE", "old")]), None), ("pasture".to_string(), None));
        let (p, note) = aws_profile(&get(&[("GOAT_AWS_VAULT_PROFILE", "old")]), None);
        assert_eq!(p, "old");
        assert!(note.unwrap().contains("deprecated"));
        assert_eq!(aws_profile(&get(&[("GOAT_AWS_PROFILE", "")]), None), ("default".to_string(), None), "empty is unset");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    fn plays(files: &[(&str, &str)]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for (n, b) in files {
            std::fs::write(t.path().join(n), b).unwrap();
        }
        t
    }

    #[test]
    fn a_direct_mandatory_target_is_an_extra_var() {
        let t = plays(&[("p.yml", "- name: x\n  hosts: \"{{ target | mandatory }}\"\n")]);
        let m = resolve_target_mode(t.path(), &t.path().join("p.yml")).unwrap();
        assert_eq!(m.mode, "-e target=<group>");
    }

    #[test]
    fn two_alias_hops_are_followed_to_the_play_that_declares_hosts() {
        let t = plays(&[
            ("a.yml", "- import_playbook: b.yml\n"),
            ("b.yml", "- import_playbook: c.yml\n"),
            ("c.yml", "- name: herd\n  hosts: \"{{ herd_target_hosts | mandatory }}\"\n"),
        ]);
        let m = resolve_target_mode(t.path(), &t.path().join("a.yml")).unwrap();
        assert_eq!(m.mode, "-e herd_target_hosts=<group>");
        assert!(m.play.ends_with("c.yml"));
    }

    #[test]
    fn more_than_three_hops_is_refused_rather_than_guessed() {
        let t = plays(&[
            ("a.yml", "- import_playbook: b.yml\n"),
            ("b.yml", "- import_playbook: c.yml\n"),
            ("c.yml", "- import_playbook: d.yml\n"),
            ("d.yml", "- name: d\n  hosts: all\n"),
        ]);
        let e = resolve_target_mode(t.path(), &t.path().join("a.yml")).unwrap_err().to_string();
        assert!(e.contains("three import_playbook hops"), "{e}");
    }

    #[test]
    fn a_play_with_neither_hosts_nor_an_import_is_refused() {
        let t = plays(&[("p.yml", "- name: tasks only\n  tasks: []\n")]);
        let e = resolve_target_mode(t.path(), &t.path().join("p.yml")).unwrap_err().to_string();
        assert!(e.contains("no hosts: line and no import_playbook"), "{e}");
    }

    #[test]
    fn every_repository_play_with_a_templated_target_resolves_to_an_extra_var() {
        // Both avenues read a templated `hosts:` only when it is double-quoted or
        // bare and not the first key of the play. A single-quoted '{{ target }}'
        // would be read as a plain `-l <group>` limit — the F-587 failure shape —
        // so no repository play may be written that way.
        let ansible = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible");
        let mut checked = 0;
        for e in std::fs::read_dir(&ansible).unwrap() {
            let p = e.unwrap().path();
            if p.extension().map(|x| x != "yml").unwrap_or(true) {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let Some(line) = text.lines().map(str::trim_start).find(|l| l.starts_with("hosts:") || l.starts_with("- hosts:")) else { continue };
            if !line.contains("{{") {
                continue;
            }
            let m = resolve_target_mode(&ansible, &p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(m.mode.starts_with("-e "), "{}: a templated hosts line resolves to `{}`, not an extra var", p.display(), m.mode);
            checked += 1;
        }
        assert!(checked >= 20, "expected the repository's templated plays, checked {checked}");
    }

    #[test]
    fn hosts_as_the_first_key_of_a_play_is_refused_by_name_not_misrouted() {
        // `- hosts: x` is valid Ansible. Both avenues read `hosts:` only as a
        // non-first key (deploy-component.sh's grep too), and no repository play
        // is written this way; the contract is that such a play is refused by
        // name rather than deployed with a guessed target mode.
        let t = plays(&[("p.yml", "- hosts: \"{{ target | mandatory }}\"\n")]);
        let e = resolve_target_mode(t.path(), &t.path().join("p.yml")).unwrap_err().to_string();
        assert!(e.contains("p.yml"), "{e}");
    }

    #[test]
    fn a_relative_path_is_rendered_against_the_repository() {
        let repo = Path::new("/r");
        assert_eq!(rel(repo, Path::new("/r/ops/x.yml")), "ops/x.yml");
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    fn fail(name: &str, detail: &str, fix: &str) -> Check {
        Check { name: name.into(), ok: false, detail: detail.into(), fix: Some(fix.into()) }
    }

    fn page(checks: Vec<Check>) -> String {
        let r = crate::Report::new("doctor", checks, crate::EXIT_REFUSED);
        summary(&r, Path::new("/no/such/repo"), Some("zagros_dev"), None, false)
    }

    #[test]
    fn the_fix_is_yellow_and_labels_are_bold_only_when_color_is_on() {
        let checks = vec![fail("checkout", "on a branch", "git checkout main")];
        let r = crate::Report::new("doctor", checks, crate::EXIT_REFUSED);
        let on = summary(&r, Path::new("/no/such/repo"), None, None, true);
        assert!(on.contains("    \x1b[1mFix:\x1b[0m \x1b[33mgit checkout main\x1b[0m"), "{on:?}");
        assert!(on.contains("\x1b[1m0\x1b[0m checks passed. \x1b[1m1\x1b[0m problem found."), "{on:?}");
        let off = summary(&r, Path::new("/no/such/repo"), None, None, false);
        assert!(off.contains("    Fix: git checkout main") && !off.contains('\x1b'), "{off:?}");
    }

    #[test]
    fn only_the_names_in_local_secret_files_are_read() {
        let d = crate::testdir::path("goatbuilder-doctor-names-");
        let local = d.join(".envrc.local.d");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("a.refs"), "# comment\nONE=op://v/i/f\nexport TWO=\"op://v/i/g\"\nFIVE=\nexport SIX=\"${SIX:-$OTHER}\"\n").unwrap();
        std::fs::write(local.join("b.envrc"), "export THREE=plaintext-value\n").unwrap();
        std::fs::write(local.join("c.txt"), "FOUR=ignored\n").unwrap();
        let got = names_on_this_machine(&d);
        let _ = std::fs::remove_dir_all(&d);
        for n in ["ONE", "TWO", "THREE"] {
            assert!(got.contains(n), "{n}: {got:?}");
        }
        assert!(!got.contains("FOUR"), "{got:?}");
        assert!(!got.contains("FIVE"), "an empty value is not a secret on this machine: {got:?}");
        assert!(!got.contains("SIX"), "a ${{VAR:-}} fallback is not a secret on this machine: {got:?}");
        assert!(got.iter().all(|n| !n.contains("plaintext") && !n.contains("op://")), "a value was kept: {got:?}");
    }

    #[test]
    fn a_secrets_fix_never_repeats_the_command_that_just_ran() {
        for (components, signed_out) in [(8, false), (8, true), (0, true)] {
            let (_, steps) = secrets_advice(components, signed_out, "1password", true, Path::new("/no/such/repo"), &[]);
            assert!(!steps.is_empty());
            assert!(steps.iter().all(|s| !s.contains("with-secrets") && !s.contains("doctor")), "{steps:?}");
        }
    }


    #[test]
    fn no_aws_fix_deletes_the_stored_key() {
        for (keychain, out) in [(true, "credentials missing"), (false, "Please reauthenticate using 'aws login'"), (false, "anything else")] {
            let fix = aws_vault_fix("usareur-af", keychain, out);
            assert!(!fix.contains("remove"), "{fix}");
        }
        assert_eq!(aws_vault_fix("usareur-af", false, "run aws login"), "aws login");
        // The way to re-enter a key stays named, for a key that really is bad.
        assert!(aws_vault_fix("usareur-af", false, "anything else").contains("aws-vault add usareur-af"));
        let keychain = aws_vault_fix("usareur-af", true, "credentials missing");
        assert!(keychain.contains("90-local.envrc") && keychain.contains("direnv allow"), "the keychain fix names the way out, not only the diagnostic: {keychain}");
    }

    #[test]
    fn passing_checks_are_one_count() {
        let p = page(vec![ok("tool:git", "the source of record".into()), ok("tool:gh", "tags".into())]);
        assert!(p.contains("All 2 checks passed."), "{p}");
        assert!(!p.contains("tool:git"), "{p}");
    }

    #[test]
    fn secrets_rows_with_one_cause_are_one_problem() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let p = page(vec![
            ok("tool:git", "x".into()),
            fail("secrets:op-session", "not signed in", "eval $(op signin)"),
            fail("secrets:zagros_dev:netvis", "3 of 3 not set: A, B, C", "long fix"),
            fail("secrets:zagros_dev:herd", "5 of 5 not set: D", "long fix"),
        ]);
        assert!(p.contains("1 check passed. 1 problem found."), "{p}");
        assert!(p.contains("Secrets: 2 components missing some"), "{p}");
        assert!(!p.contains("with-secrets"), "{p}");
        // The same component on three goatnets is one component (12, not 36).
        let three = crate::Report::new("doctor", vec![
            Check { name: "secrets:zagros:portal".into(), ok: false, detail: "1 of 1 not on this machine: A".into(), fix: None },
            Check { name: "secrets:zagros_dev:portal".into(), ok: false, detail: "1 of 1 not on this machine: A".into(), fix: None },
            Check { name: "secrets:zagros_prod:portal".into(), ok: false, detail: "1 of 1 not on this machine: A".into(), fix: None },
        ], crate::EXIT_REFUSED);
        let page = summary(&three, Path::new("/no/such/repo"), None, None, false);
        assert!(page.contains("1 component missing some"), "{page}");
        // The store's own words, never 1Password's, when another store is declared.
        let (why, steps) = secrets_advice(2, false, "sops", false, Path::new("/no/such/repo"), &["A".to_string()]);
        assert!(why.contains("not in the sops store") && !why.contains("1Password"), "{why}");
        assert!(steps.is_empty() || steps.iter().all(|s| !s.contains("op signin")), "{steps:?}");
        let (why, _) = secrets_advice(0, true, "env-file", false, Path::new("/no/such/repo"), &[]);
        assert!(why.contains("env-file store") && !why.contains("1Password"), "{why}");
        // Under the 1password store the fix is a command, not a description of
        // one. The page above reads the store from its checkout, so ask directly.
        // A checkout declares its owner (repo.source); this one is the repository the test is built in.
        let (why, steps) = secrets_advice(2, true, "1password", true, &Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."), &[]);
        assert!(why.contains("1password store is signed out"), "{why}");
        let script = steps[..4].join("\n");
        assert!(script.contains("/goat-credentials/contents/refs/goat.refs"), "{script}");
        assert_eq!(steps[4], "eval $(op signin)");
        // Run it: a failed fetch leaves the refs file alone; a good one lands 0600.
        let t = crate::testdir::path("goatbuilder-refs-fix-");
        let bin = t.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let run = |gh: &str| {
            let stub = bin.join("gh");
            std::fs::write(&stub, gh).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::process::Command::new("sh").arg("-c").arg(&script).current_dir(&t)
                .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())).status().unwrap().success()
        };
        assert!(run("#!/bin/sh\necho 'A=op://v/i/f'\n"), "a fresh clone has no .envrc.local.d");
        let refs = t.join(".envrc.local.d/00-goat.refs");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&refs).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!run("#!/bin/sh\nexit 1\n"));
        assert_eq!(std::fs::read_to_string(&refs).unwrap(), "A=op://v/i/f\n", "a failed gh emptied the refs file");
        let _ = std::fs::remove_dir_all(&t);
        assert!(p.lines().all(|l| l.chars().count() <= 80), "wider than the page: {p}");
        assert!(p.contains("Checkout: /no/such/repo"), "{p}");
        assert!(!p.contains("netvis") && !p.contains("long fix"), "{p}");
    }

    #[test]
    fn the_page_stays_inside_the_width_and_hides_the_checkout_path() {
        let repo = "/no/such/repo";
        let p = page(vec![
            fail("checkout", &format!("{repo} is on 'x' \u{2014} releases run from a main checkout"), "git checkout main"),
            fail("tool:live-rows", "no node_modules", &format!("(cd {repo}/ops/goat-id-console/web/e2e && npm ci && npx playwright install chromium)")),
        ]);
        assert!(p.contains("This checkout is on 'x': releases"), "{p}");
        assert!(p.contains("Browser tests"), "{p}");
        assert!(!p.split_once("passed").unwrap().1.contains(repo) && !p.contains('\u{2014}'), "{p}");
        for line in p.lines() {
            assert!(line.chars().count() <= 80, "wider than the page: {line}");
        }
        assert!(p.contains("npm ci &&\n"), "a long command breaks after &&: {p}");
        assert!(!p.contains("\n\n\n"), "one blank line between entries: {p}");
    }
}

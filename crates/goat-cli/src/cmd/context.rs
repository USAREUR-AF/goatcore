use crate::profile::{Profile, DEFAULT_CONTEXT};
use clap::Subcommand;
use std::io::Write;

#[derive(Subcommand, Debug)]
pub enum ContextCmd {
    /// List contexts known to this profile.
    Ls {
        /// Emit one JSON line per context instead of pretty output.
        #[arg(long)]
        json: bool,
    },
    /// Switch the active context (writes `current_context` to config.toml).
    Use { name: String },
    /// Show one context's resolved endpoints, role, org prefix, cert expiry.
    Show {
        /// Defaults to the active context.
        name: Option<String>,
    },
    /// Remove a context. Refuses to remove the active context unless
    /// another context is selected via `--switch-to <name>`.
    Rm {
        name: String,
        /// After removal, switch active context to this one.
        #[arg(long)]
        switch_to: Option<String>,
        /// Skip confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Write this goatnet's own facts into the context's profile.toml.
    ///
    /// Reads `goatnet.toml` from the goatnet's download surface
    /// (`get.<site>/goatnet.toml`): its NetBird management URL and group
    /// prefix, Ory URL, router endpoint and the hosts' SSH user. Commands that
    /// need a per-goatnet fact read it from the profile; goat holds none.
    Refresh {
        /// Defaults to the active context.
        name: Option<String>,
        /// Read the facts from this URL or local file instead of get.<site>.
        #[arg(long)]
        from: Option<String>,
        /// Print what would change without writing.
        #[arg(long)]
        dry_run: bool,
    },
}

pub async fn run(ctx: &super::Ctx, op: ContextCmd) -> anyhow::Result<()> {
    match op {
        ContextCmd::Ls { json } => ls(&ctx.profile, json),
        ContextCmd::Use { name } => use_(&ctx.profile, &name),
        ContextCmd::Show { name } => show(&ctx.profile, name.as_deref()),
        ContextCmd::Rm {
            name,
            switch_to,
            yes,
        } => rm(&ctx.profile, &name, switch_to.as_deref(), yes),
        ContextCmd::Refresh {
            name,
            from,
            dry_run,
        } => refresh(&ctx.profile, name.as_deref(), from.as_deref(), dry_run).await,
    }
}

/// The keys `goatnet.toml` may set in a profile. Anything else in the file is
/// ignored, so a goatnet's file can never overwrite the context's credentials
/// or the endpoints its signed bundle wrote (see BUNDLE_KEYS).
pub(crate) const GOATNET_KEYS: &[&str] = &[
    "goatnet",
    "router_endpoint",
    "netbird_mgmt_url",
    "netbird_token_env",
    "netbird_site_prefix",
    "netbird_rosenpass",
    "ory_auth_url",
    "ory_ssh_host",
    "ory_ssh_user",
    "mgmt_ssh_user",
    "demo_service",
    "i18n_feedback_email",
    // The goatnet's warrant membership declaration (`enforce` | `report` |
    // `none`), read by cmd::warrant_membership::governance.
    crate::cmd::warrant_membership::CONTEXT_CONVERGE_KEY,
];
const GOATNET_SCHEMA: &str = "goat/goatnet/v1";
/// Keys the signed bundle also writes: `goatnet.toml` fills them only when the
/// profile lacks them, never replaces the bundle's value.
const BUNDLE_KEYS: &[&str] = &["router_endpoint"];

/// Parse `goatnet.toml`: `schema` must be `goat/goatnet/v1`; returns the
/// recognised `key = "value"` pairs in file order, empty values dropped.
pub(crate) fn parse_goatnet_toml(body: &str) -> anyhow::Result<Vec<(String, String)>> {
    use crate::profile::extract_toml_field;
    match extract_toml_field(body, "schema") {
        Some(s) if s == GOATNET_SCHEMA => {}
        Some(s) => {
            anyhow::bail!("goatnet.toml has schema `{s}`; this goat reads `{GOATNET_SCHEMA}`")
        }
        None => anyhow::bail!("not a goatnet.toml: no `schema = \"{GOATNET_SCHEMA}\"` line"),
    }
    let mut out = Vec::new();
    for k in GOATNET_KEYS {
        if let Some(v) = extract_toml_field(body, k).filter(|v| !v.trim().is_empty()) {
            out.push((k.to_string(), v));
        }
    }
    if !out.iter().any(|(k, _)| k == "goatnet") {
        anyhow::bail!("goatnet.toml names no `goatnet`");
    }
    Ok(out)
}

/// Merge the facts into a profile.toml body: a key already present is
/// replaced in place, a new key is appended. Returns the new body and the
/// keys whose value changed. Refuses when the profile already names a
/// different goatnet, so a context is never re-pointed by a wrong URL.
pub(crate) fn merge_goatnet_facts(
    profile_toml: &str,
    facts: &[(String, String)],
) -> anyhow::Result<(String, Vec<String>)> {
    use crate::profile::extract_toml_field;
    let want = facts
        .iter()
        .find(|(k, _)| k == "goatnet")
        .map(|(_, v)| v.as_str());
    if let (Some(have), Some(want)) = (extract_toml_field(profile_toml, "goatnet"), want) {
        if have != want {
            anyhow::bail!(
                "this context is goatnet `{have}` but the facts are for `{want}`; \
                 check --from, or use the context for `{want}`"
            );
        }
    }
    let mut lines: Vec<String> = profile_toml.lines().map(str::to_string).collect();
    let mut changed = Vec::new();
    for (k, v) in facts {
        let new_line = format!("{k} = \"{v}\"");
        let pos = lines.iter().position(|l| {
            let t = l.trim_start();
            t.strip_prefix(k.as_str())
                .map(|r| r.trim_start().starts_with('='))
                .unwrap_or(false)
        });
        match pos {
            Some(i) if lines[i].trim() == new_line => {}
            Some(_) if BUNDLE_KEYS.contains(&k.as_str()) => {}
            Some(i) => {
                lines[i] = new_line;
                changed.push(k.clone());
            }
            None => {
                lines.push(new_line);
                changed.push(k.clone());
            }
        }
    }
    let mut body = lines.join("\n");
    body.push('\n');
    Ok((body, changed))
}

async fn refresh(
    profile: &Profile,
    name: Option<&str>,
    from: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let mut target = profile.clone();
    target.context = name
        .map(String::from)
        .unwrap_or_else(|| current_context(profile));
    target.require_initialized()?;

    let source = match from {
        Some(s) => s.to_string(),
        None => match crate::version_check::get_base_url(&target) {
            Some(base) => format!("{}/goatnet.toml", base.trim_end_matches('/')),
            None => anyhow::bail!(
                "cannot find this context's download surface (get.<site>): its profile has no \
                 portal.<site> endpoint. Pass --from <url-or-file> with the goatnet's goatnet.toml"
            ),
        },
    };
    let body = if source.starts_with("https://") || source.starts_with("http://") {
        let resp = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?
            .get(&source)
            .send()
            .await
            .map_err(|e| {
                anyhow::anyhow!("fetch {source}: {e} (is this machine on the goatnet's mesh?)")
            })?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "fetch {source}: HTTP {}; the goatnet's download surface does not serve goatnet.toml yet \
                 (goat-get role, docs/design/goat-cli-mesh-download.md)",
                resp.status()
            );
        }
        resp.text().await?
    } else {
        std::fs::read_to_string(&source).map_err(|e| anyhow::anyhow!("read {source}: {e}"))?
    };
    let facts = parse_goatnet_toml(&body).map_err(|e| anyhow::anyhow!("{source}: {e}"))?;
    let path = target.profile_toml_path();
    let current = target.read_classified(&path)?;
    let (merged, changed) = merge_goatnet_facts(&current, &facts)?;

    if changed.is_empty() {
        println!(
            "context `{}`: already current with {source}",
            target.context
        );
        return Ok(());
    }
    for k in &changed {
        let v = facts
            .iter()
            .find(|(fk, _)| fk == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        println!("  {k} = \"{v}\"");
    }
    if dry_run {
        println!(
            "context `{}`: {} key(s) would change (dry run, nothing written)",
            target.context,
            changed.len()
        );
        return Ok(());
    }
    write_profile_toml(&path, &merged)?;
    println!(
        "context `{}`: {} key(s) written from {source}",
        target.context,
        changed.len()
    );
    Ok(())
}

/// Replace profile.toml atomically, mode 0600 like the rest of the context.
fn write_profile_toml(path: &std::path::Path, body: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Run before a `goat admin` command. A context that names no goatnet yet (it
/// has never been refreshed) gets its goatnet's facts once, from
/// get.<site>/goatnet.toml, with a short timeout. Any failure is left to the
/// command, which names `goat context refresh` wherever it needs a fact.
pub(crate) async fn ensure_goatnet_facts(profile: &Profile) {
    let path = profile.profile_toml_path();
    let Ok(body) = std::fs::read_to_string(&path) else {
        return;
    };
    if crate::profile::extract_toml_field(&body, "goatnet").is_some() {
        return;
    }
    let Some(base) = crate::version_check::get_base_url(profile) else {
        return;
    };
    let url = format!("{}/goatnet.toml", base.trim_end_matches('/'));
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    else {
        return;
    };
    let Ok(resp) = client.get(&url).send().await else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(text) = resp.text().await else {
        return;
    };
    let Ok(facts) = parse_goatnet_toml(&text) else {
        return;
    };
    let Ok((merged, changed)) = merge_goatnet_facts(&body, &facts) else {
        return;
    };
    if !changed.is_empty() && write_profile_toml(&path, &merged).is_ok() {
        let g = facts
            .iter()
            .find(|(k, _)| k == "goatnet")
            .map(|(_, v)| v.as_str())
            .unwrap_or("?");
        eprintln!(
            "goat: context `{}` now carries goatnet `{g}`'s facts from {url}",
            profile.context
        );
    }
}

/// List context dirs under `<profile>/contexts/`. Marks the active
/// context with `*`. Pure-local; no network.
fn ls(profile: &Profile, json: bool) -> anyhow::Result<()> {
    let contexts = list_contexts(profile)?;
    let active = current_context(profile);
    if json {
        for name in &contexts {
            let entry = serde_json::json!({
                "name": name,
                "active": name == &active,
            });
            println!("{}", serde_json::to_string(&entry).unwrap_or_default());
        }
        return Ok(());
    }
    if contexts.is_empty() {
        println!("(no contexts — run `goat profile init <bundle.cbor>` to create one)");
        return Ok(());
    }
    for name in &contexts {
        let marker = if name == &active { "*" } else { " " };
        println!("{marker} {name}");
    }
    Ok(())
}

/// Switch the active context. Writes `current_context = "<name>"` to
/// the profile's config.toml. Refuses on a context that doesn't exist.
fn use_(profile: &Profile, name: &str) -> anyhow::Result<()> {
    let contexts = list_contexts(profile)?;
    if !contexts.iter().any(|c| c == name) {
        anyhow::bail!("context `{name}` does not exist. `goat context ls` shows what does.");
    }
    write_current_context(profile, name)?;
    println!("switched to context `{name}` ({})", profile.dir.display());
    Ok(())
}

/// Show one context's profile.toml + leaf-cert expiry. The named
/// context (or the active one if name is None) must be initialized.
fn show(profile: &Profile, name: Option<&str>) -> anyhow::Result<()> {
    let target_name = name
        .map(String::from)
        .unwrap_or_else(|| current_context(profile));
    let mut target = profile.clone();
    target.context = target_name.clone();

    target.require_initialized()?;
    // read_classified (not a bare `?`), matching cmd/profile.rs's identical
    // read — before this, the two `show` commands gave different-quality
    // errors for the exact same failure (a profile.toml that vanished or
    // became unreadable between the check above and this read).
    let body = target.read_classified(&target.profile_toml_path())?;
    print!("{body}");
    // Cert expiry (live, may diverge from profile.toml if hand-edited).
    if let Ok(unix) = leaf_cert_not_after_unix(&target.cert_path()) {
        println!("# mtls.cert.pem not_after = unix {unix}");
    }
    Ok(())
}

/// Remove a context directory. Refuses on the active context unless
/// the operator names a different `--switch-to`. Refuses on the
/// `default` context if it's the only one (degrades gracefully into
/// the empty-profile state otherwise).
fn rm(profile: &Profile, name: &str, switch_to: Option<&str>, yes: bool) -> anyhow::Result<()> {
    let contexts = list_contexts(profile)?;
    if !contexts.iter().any(|c| c == name) {
        anyhow::bail!("context `{name}` does not exist.");
    }
    let active = current_context(profile);
    if name == active {
        match switch_to {
            Some(target) if contexts.iter().any(|c| c == target) && target != name => {
                write_current_context(profile, target)?;
                println!("active context switched to `{target}` before removing `{name}`");
            }
            Some(target) => anyhow::bail!(
                "--switch-to `{target}` is not an existing other context (have: {})",
                contexts.join(", ")
            ),
            None => anyhow::bail!(
                "cannot remove the active context `{name}`. Pass --switch-to <other> to switch first, then remove."
            ),
        }
    }

    if !yes {
        let dir = profile.contexts_dir().join(name);
        eprint!("Delete context `{name}` at {}? [y/N] ", dir.display());
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("aborted (no confirmation)");
        }
    }
    let dir = profile.contexts_dir().join(name);
    {
        use anyhow::Context;
        std::fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
    }
    println!("removed context `{name}` ({})", dir.display());
    Ok(())
}

/// Enumerate contexts under `<profile>/contexts/`. A context is any
/// directory whose name doesn't end in `.bak` (rotation backups
/// from `goat profile rotate` are filtered out).
fn list_contexts(profile: &Profile) -> anyhow::Result<Vec<String>> {
    let dir = profile.contexts_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.ends_with(".bak"))
        .collect();
    out.sort();
    Ok(out)
}

/// Read `current_context` from `<profile>/config.toml`. Falls back to
/// `default` when the file is absent or the key is missing. Delegates to
/// the shared `profile::read_current_context` parser — the same one
/// `Profile::resolve` uses to pick the active context — so the `ls`/`use`
/// view can never disagree with what commands actually resolve to.
fn current_context(profile: &Profile) -> String {
    crate::profile::read_current_context(&profile.config_path())
        .unwrap_or_else(|| DEFAULT_CONTEXT.to_string())
}

/// Write `current_context = "<name>"` to `<profile>/config.toml`.
/// The file is mode 0600 — same as the per-context secret files.
fn write_current_context(profile: &Profile, name: &str) -> anyhow::Result<()> {
    let path = profile.config_path();
    let body = format!("current_context = \"{name}\"\n");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(body.as_bytes())?;
    // mode arg on OpenOptions only applies at create time; if the file
    // pre-existed (operator hand-edited config), tighten perms here.
    // Windows has no POSIX mode bits — user-profile ACLs cover it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Parse the leaf cert PEM and return `notAfter` as Unix seconds.
/// Mirrors the helper in cmd/profile.rs; kept locally so this module
/// stays self-contained.
fn leaf_cert_not_after_unix(path: &std::path::Path) -> anyhow::Result<i64> {
    let pem_bytes = std::fs::read(path)?;
    let blob = pem::parse(&pem_bytes)?;
    if blob.tag() != "CERTIFICATE" {
        anyhow::bail!("first PEM block is {}, want CERTIFICATE", blob.tag());
    }
    let (_, cert) = x509_parser::parse_x509_certificate(blob.contents())
        .map_err(|e| anyhow::anyhow!("x509 parse: {e}"))?;
    Ok(cert.validity().not_after.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture_profile(tmp: &TempDir, contexts: &[&str], active: Option<&str>) -> Profile {
        let dir = tmp.path().to_path_buf();
        let contexts_dir = dir.join("contexts");
        fs::create_dir_all(&contexts_dir).unwrap();
        for c in contexts {
            let ctx_dir = contexts_dir.join(c);
            fs::create_dir_all(&ctx_dir).unwrap();
            fs::write(
                ctx_dir.join("profile.toml"),
                format!(
                    "portal_endpoint = \"https://portal.test\"\nrouter_endpoint = \"tls/host:7447\"\ndevice_id = \"op-{c}\"\n"
                ),
            )
            .unwrap();
        }
        if let Some(a) = active {
            fs::write(
                dir.join("config.toml"),
                format!("current_context = \"{a}\"\n"),
            )
            .unwrap();
        }
        Profile {
            dir,
            context: active.unwrap_or(DEFAULT_CONTEXT).to_string(),
        }
    }

    #[test]
    fn list_contexts_filters_bak() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default", "second"], Some("default"));
        // Add a .bak sibling that should be filtered.
        fs::create_dir_all(p.contexts_dir().join("default.bak")).unwrap();
        let got = list_contexts(&p).unwrap();
        assert_eq!(got, vec!["default", "second"]);
    }

    #[test]
    fn current_context_falls_back_to_default() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default"], None);
        assert_eq!(current_context(&p), "default");
    }

    #[test]
    fn use_switches_and_persists() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default", "second"], Some("default"));
        use_(&p, "second").unwrap();
        assert_eq!(current_context(&p), "second");
        // Mode 0600 enforced (unix only — Windows has no POSIX bits).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(p.config_path()).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn use_refuses_unknown_context() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default"], Some("default"));
        let err = use_(&p, "nope").unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn rm_refuses_active_without_switch_to() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default", "second"], Some("default"));
        let err = rm(&p, "default", None, true).unwrap_err();
        assert!(err.to_string().contains("cannot remove the active"));
    }

    #[test]
    fn rm_with_switch_to_succeeds() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default", "second"], Some("default"));
        rm(&p, "default", Some("second"), true).unwrap();
        // active context switched.
        assert_eq!(current_context(&p), "second");
        // dir gone.
        assert!(!p.contexts_dir().join("default").exists());
    }

    #[test]
    fn rm_refuses_unknown_context() {
        let tmp = TempDir::new().unwrap();
        let p = fixture_profile(&tmp, &["default"], Some("default"));
        let err = rm(&p, "nope", None, true).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    const FACTS: &str = "# rendered\nschema = \"goat/goatnet/v1\"\ngoatnet = \"zagros\"\n\
        netbird_mgmt_url = \"https://netbird.zagros.example\"\nnetbird_rosenpass = \"false\"\n\
        mgmt_ssh_user = \"goat\"\nportal_endpoint = \"https://evil.example\"\n";

    #[test]
    fn goatnet_toml_needs_its_schema() {
        assert!(parse_goatnet_toml("goatnet = \"zagros\"\n").is_err());
        assert!(
            parse_goatnet_toml("schema = \"goat/goatnet/v9\"\ngoatnet = \"zagros\"\n").is_err()
        );
        assert!(
            parse_goatnet_toml("schema = \"goat/goatnet/v1\"\n").is_err(),
            "must name its goatnet"
        );
    }

    #[test]
    fn goatnet_toml_sets_only_its_own_keys() {
        let f = parse_goatnet_toml(FACTS).unwrap();
        assert!(f.iter().any(|(k, v)| k == "mgmt_ssh_user" && v == "goat"));
        assert!(
            !f.iter().any(|(k, _)| k == "portal_endpoint"),
            "a goatnet.toml must never overwrite a bundle-issued endpoint"
        );
    }

    #[test]
    fn goatnet_toml_carries_the_warrant_membership_declaration() {
        let f = parse_goatnet_toml(
            "schema = \"goat/goatnet/v1\"\ngoatnet = \"zagros\"\n\
             warrant_membership_converge = \"none\"\n",
        )
        .unwrap();
        assert!(f
            .iter()
            .any(|(k, v)| k == "warrant_membership_converge" && v == "none"));
        let (body, changed) = merge_goatnet_facts("goatnet = \"zagros\"\n", &f).unwrap();
        assert!(body.contains("warrant_membership_converge = \"none\"\n"));
        assert_eq!(changed, vec!["warrant_membership_converge".to_string()]);
    }

    #[test]
    fn merge_replaces_in_place_and_appends_new_keys() {
        let profile =
            "portal_endpoint = \"https://portal.zagros.example\"\nnetbird_rosenpass = \"true\"\n";
        let (body, changed) =
            merge_goatnet_facts(profile, &parse_goatnet_toml(FACTS).unwrap()).unwrap();
        assert!(body.starts_with(
            "portal_endpoint = \"https://portal.zagros.example\"\nnetbird_rosenpass = \"false\"\n"
        ));
        assert!(body.contains("mgmt_ssh_user = \"goat\"\n"));
        assert!(changed.contains(&"netbird_rosenpass".to_string()));
        let (_, again) = merge_goatnet_facts(&body, &parse_goatnet_toml(FACTS).unwrap()).unwrap();
        assert!(again.is_empty(), "a second refresh changes nothing");
    }

    #[test]
    fn merge_keeps_the_bundle_router_endpoint_and_fills_it_when_absent() {
        let facts = parse_goatnet_toml(
            "schema = \"goat/goatnet/v1\"\ngoatnet = \"zagros\"\nrouter_endpoint = \"tls/b:7447\"\n",
        )
        .unwrap();
        let (kept, _) = merge_goatnet_facts("router_endpoint = \"tls/a:7447\"\n", &facts).unwrap();
        assert!(kept.contains("router_endpoint = \"tls/a:7447\"") && !kept.contains("tls/b"));
        let (filled, _) = merge_goatnet_facts("portal_endpoint = \"x\"\n", &facts).unwrap();
        assert!(filled.contains("router_endpoint = \"tls/b:7447\""));
    }

    #[test]
    fn merge_refuses_facts_for_another_goatnet() {
        let profile = "goatnet = \"nova\"\n";
        let err = merge_goatnet_facts(profile, &parse_goatnet_toml(FACTS).unwrap()).unwrap_err();
        assert!(err.to_string().contains("`nova`"));
    }

    #[test]
    fn merge_does_not_match_a_longer_key_with_the_same_prefix() {
        let profile = "goatnet_label = \"x\"\n";
        let (body, _) = merge_goatnet_facts(profile, &parse_goatnet_toml(FACTS).unwrap()).unwrap();
        assert!(
            body.contains("goatnet_label = \"x\"\n") && body.contains("goatnet = \"zagros\"\n")
        );
    }
}

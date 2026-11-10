// `goat demo` — one command to run a fabric demo end-to-end.
//
// A demo is a service the goatnet runs beside the portal: `goat demo <name>`
// opens its `/demo` page in your browser AND tails the Zenoh traffic it
// publishes under `<name>/<site>/**` in this terminal — both halves of the
// demo from a single command, no macOS-only window-positioning script.
//
// Pure convenience over capabilities that already exist: the browser does the
// playing, `goat sub` does the watching. The demo name defaults to the
// goatnet's published `demo_service` (goatnet.toml, `goat context refresh`);
// the demo host + site_id are derived from the active profile's portal
// endpoint (sibling-surface swap), overridable with --url / --site for
// off-profile or localhost runs.

use clap::Args;

#[derive(Args, Debug)]
pub struct DemoArgs {
    /// The demo service to run: its host label and key prefix. Defaults to the
    /// goatnet's `demo_service` (written by `goat context refresh`).
    #[arg(value_name = "NAME")]
    pub target: Option<String>,

    /// Override the demo page URL (e.g. http://localhost:8095 for a local
    /// run, or a different goatnet). The `/demo` path is appended if absent.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,

    /// Override the site_id used for the watch key `<name>/<site>/**`.
    /// Defaults to the site derived from the profile (or the --url host).
    #[arg(long, value_name = "SITE")]
    pub site: Option<String>,

    /// Open the browser only; don't tail the fabric in this terminal.
    /// Combined with --no-open it's a dry-run: print the resolved URL +
    /// watch command and exit without opening anything.
    #[arg(long)]
    pub no_watch: bool,

    /// Tail the fabric only; don't open the browser.
    #[arg(long)]
    pub no_open: bool,

    /// Use the interactive Tail lens (event table + rate sparkline) for the
    /// watch instead of the default line-oriented tail. Requires a terminal.
    #[arg(long)]
    pub tui: bool,
}

pub async fn run(ctx: &super::Ctx, args: DemoArgs) -> anyhow::Result<()> {
    let name = demo_name(args.target.as_deref(), ctx.profile.goatnet_endpoints().demo_service)?;
    run_demo(ctx, &name, args).await
}

/// The demo to run: the one named on the command line, else the goatnet's
/// published `demo_service`. A name is one DNS label (it becomes the host's
/// first label and the key's first chunk), so anything else is refused.
fn demo_name(given: Option<&str>, published: Option<String>) -> anyhow::Result<String> {
    let name = match given {
        Some(n) => n.to_string(),
        None => published.ok_or_else(|| {
            anyhow::anyhow!(
                "no demo named and this context's goatnet publishes no `demo_service`. \
                 Name one (`goat demo <name>`), or run `goat context refresh` to read the \
                 goatnet's facts."
            )
        })?,
    };
    let ok = !name.is_empty()
        && name.len() <= 63
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !ok {
        anyhow::bail!("`{name}` is not a demo name: one lowercase DNS label (a-z, 0-9, -)");
    }
    Ok(name)
}

async fn run_demo(ctx: &super::Ctx, name: &str, args: DemoArgs) -> anyhow::Result<()> {
    // Resolve the demo URL + site_id: explicit flags win, else derive from the
    // active profile's portal endpoint.
    let (base_url, site) = resolve_target(ctx, name, &args)?;
    let demo_url = if base_url.trim_end_matches('/').ends_with("/demo") {
        base_url.trim_end_matches('/').to_string()
    } else {
        format!("{}/demo", base_url.trim_end_matches('/'))
    };
    let watch_key = format!("{name}/{site}/**");

    if !args.no_open {
        match open_in_browser(&demo_url) {
            Ok(()) => println!("opened {demo_url}"),
            Err(e) => {
                // Non-fatal: print the URL so the operator can open it by hand.
                eprintln!("could not open a browser automatically ({e}).");
                eprintln!("open this URL yourself:  {demo_url}");
            }
        }
    }

    if args.no_watch {
        if args.no_open {
            // Nothing to do — both halves were suppressed.
            println!("demo url: {demo_url}");
            println!("watch:    goat sub --literal '{watch_key}' --follow");
        }
        return Ok(());
    }

    println!("watching the fabric:  goat sub --literal '{watch_key}' --follow   (Ctrl-C to stop)");
    println!();

    // Delegate to the real `sub` path — same decode/alias/TUI behaviour as
    // `goat sub`, so the demo watch is the canonical tail, not a reimplementation.
    let sub_args = super::pubsub::SubArgs {
        key_pattern: watch_key,
        // `follow` is the accepted no-op alias for what is now the default;
        // `count: None` is what actually makes the watch unbounded.
        follow: true,
        decode: true,
        tui: args.tui,
        count: None,
        keys_only: false,
        for_: None,
        since: None,
        open: false,
        key_service_bundle: None,
        trust_domain: None,
        key_service_id: None,
        // The key is generated from the site, not typed, and the demo's prefix
        // is a seeded vendor prefix, not a slot alias — on a slot-pool
        // goatnet the resolver would refuse it.
        literal: true,
    };
    super::pubsub::run_sub(ctx, sub_args).await
}

/// Resolve (base_url, site_id) from explicit flags or the active profile.
fn resolve_target(ctx: &super::Ctx, name: &str, args: &DemoArgs) -> anyhow::Result<(String, String)> {
    // Both flags given: trust them outright (covers localhost / off-profile).
    if let (Some(url), Some(site)) = (&args.url, &args.site) {
        return Ok((url.clone(), site.clone()));
    }

    let derived = ctx.profile.derive_sibling(name);

    let base_url = match &args.url {
        Some(u) => u.clone(),
        None => derived
            .as_ref()
            .map(|(u, _)| u.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "couldn't derive the `{name}` URL from profile '{}'. \
                     Pass --url (e.g. --url https://{name}.<site>.netbird.<backbone>, \
                     or --url http://localhost:<port> for a local server).",
                    ctx.profile.context
                )
            })?,
    };

    let site = match &args.site {
        Some(s) => s.clone(),
        None => args
            .url
            .as_deref()
            .and_then(site_from_url)
            .or_else(|| derived.as_ref().map(|(_, s)| s.clone()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "couldn't derive site_id — pass --site <site> so the watch key \
                     '{name}/<site>/**' is correct."
                )
            })?,
    };

    Ok((base_url, site))
}

/// Best-effort site_id from an explicit `--url` of shape
/// `<scheme>://<name>.<site>.netbird.<backbone>`. Returns None for
/// hosts that don't carry a site (e.g. localhost).
fn site_from_url(url: &str) -> Option<String> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = host.split('/').next().unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
    let idx = parts.iter().position(|p| *p == "netbird")?;
    if idx < 1 {
        return None;
    }
    Some(parts[idx - 1].to_string())
}

/// Open a URL in the operator's default browser, cross-platform. No new crate
/// dependency — just the platform launcher.
fn open_in_browser(url: &str) -> anyhow::Result<()> {
    use std::process::Command;
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        // `start` is a cmd builtin; the empty "" is the window title arg.
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    }?;
    if !status.success() {
        anyhow::bail!("browser launcher exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_from_url_extracts_label_before_netbird() {
        assert_eq!(
            site_from_url("https://airdefenders.nova.netbird.nova.example"),
            Some("nova".to_string())
        );
        assert_eq!(
            site_from_url("https://airdefenders.zagros.netbird.example.net/demo"),
            Some("zagros".to_string())
        );
    }

    #[test]
    fn demo_name_prefers_the_argument_then_the_published_fact() {
        assert_eq!(demo_name(Some("tagdemo"), Some("other".into())).unwrap(), "tagdemo");
        assert_eq!(demo_name(None, Some("tagdemo".into())).unwrap(), "tagdemo");
        assert!(demo_name(None, None).is_err());
    }

    #[test]
    fn demo_name_is_one_dns_label() {
        for bad in ["", "Upper", "a/b", "a.b", "-a", "a-", "a*"] {
            assert!(demo_name(Some(bad), None).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn site_from_url_none_for_localhost() {
        assert_eq!(site_from_url("http://localhost:8095"), None);
        assert_eq!(site_from_url("http://127.0.0.1:8099/demo"), None);
    }
}

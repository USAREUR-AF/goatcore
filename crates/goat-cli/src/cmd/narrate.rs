// `goat narrate` — Phase D.C / G.6 standalone substrate-cascade reflection.
//
// "I just did a few things — show me what they did to the substrate." Reads
// recent metadata through the authenticated portal projection and renders it
// as a cascade of who-touched-what. Payload bodies are neither needed nor read.
//
// The inline `--narrate` flag on mutating commands (topic create/delete, pub)
// is the forward-looking everyday path; this standalone command is the
// backward-looking reflection over what already happened.

use clap::Args;

/// Default reflection window when `--since` is omitted: the recent past.
const DEFAULT_SINCE: &str = "60s";

#[derive(Args, Debug)]
pub struct NarrateArgs {
    /// Reflect over events since this point: a duration (`60s`, `5m`, `1h`)
    /// or an RFC3339 timestamp. Default: last 60s.
    #[arg(long)]
    pub since: Option<String>,
    /// Filter to a specific actor (an org prefix or its alias).
    #[arg(long)]
    pub actor: Option<String>,
    /// Emit one NDJSON line per cascade event.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(ctx: &super::Ctx, args: NarrateArgs) -> anyhow::Result<()> {
    let since = args.since.as_deref().unwrap_or(DEFAULT_SINCE);
    let reg = crate::alias::load_aliases(&ctx.profile).await;

    // Narrate spans everything this context can see → the `**` pattern.
    let events = super::replay::fetch_events(ctx, Some(since), "**", true).await?;

    // Optional actor filter — resolve an alias to its UUID anchor first so
    // `--actor maintainer-operator` matches the wire vendor prefix.
    let actor_anchor = args
        .actor
        .as_ref()
        .map(|a| reg.resolve_input(a).unwrap_or_else(|_| a.clone()));
    let events: Vec<_> = match &actor_anchor {
        Some(anchor) => events.into_iter().filter(|e| &e.vendor == anchor).collect(),
        None => events,
    };

    if events.is_empty() {
        eprintln!(
            "no substrate activity in the window (since {since}{})",
            args.actor
                .as_ref()
                .map(|a| format!(", actor={a}"))
                .unwrap_or_default()
        );
        return Ok(());
    }

    for ev in &events {
        if args.json {
            println!("{}", serde_json::to_string(ev)?);
        } else {
            let key = crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &ev.key_expr);
            let actor = reg.render(&ev.vendor);
            println!("[{}] {} → {} ({}B)", ev.ts, actor, key, ev.payload_len);
        }
    }
    eprintln!("{} substrate event(s) audited", events.len());
    Ok(())
}

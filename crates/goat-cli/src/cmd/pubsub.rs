// `goat pub` + `goat sub` — Phase D.A operator pub/sub.
//
// MVP scope: open a Zenoh mTLS session via the same path
// `goat doctor` uses (cert + key + CA-roots from the profile),
// then publish-once or subscribe-and-print. Deferred to Phase D.B:
//   --decode (parse the topic's portal-registered protobuf
//     descriptor + decode incoming bytes), --since (audit-tail
//     replay via MinIO S3 walker), --tui (richer interactive
//     pane). Each marked at clap-level today and rejected at
//     runtime with a "Phase D.B — coming soon" hint.

use crate::profile::Profile;
use clap::Args;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time::timeout;

/// Default connect/handshake deadline for opening a fabric session. The
/// operator can override it via `--timeout` / `GOAT_TIMEOUT` (see
/// `set_network_timeout_secs`) to fail faster (scripts) or tolerate a slow
/// link. Bounds the *connect* phase only — never steady-state streaming.
const ZENOH_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// MAX_LINKS retry budget. When a slot-pool cert pins the session ZID to its
/// anchor (ADR 1039), Zenoh permits exactly one transport per ZID, so a rapid
/// relaunch (or a second `goat` process under the same identity) is rejected
/// `MAX_LINKS` until the prior session reaps on the router (~seconds). We
/// catch that close + brief backoff + retry so the operator never sees a
/// transient collision — mandatory for any command that relaunches often (a
/// future `goat top` live dashboard). See F-209 §3 + the goat-top tee-up.
const MAX_LINKS_RETRIES: u32 = 6;
const MAX_LINKS_BACKOFF: Duration = Duration::from_millis(500);

/// Operator override (seconds) for the session-open connect deadline, set
/// once at startup from `--timeout` / `GOAT_TIMEOUT`. Unset → the default
/// [`ZENOH_OPEN_TIMEOUT`]. A process-global (not threaded through every
/// command's `Ctx`) because the only consumer is this one session-open path.
static NETWORK_TIMEOUT_OVERRIDE: OnceLock<Duration> = OnceLock::new();

/// Record the operator's `--timeout` / `GOAT_TIMEOUT` (seconds). Called once
/// from `Ctx::from_cli`. `None` or `0` leaves the default in force (0 is a
/// meaningless connect deadline). First write wins (set once at startup).
pub(crate) fn set_network_timeout_secs(secs: Option<u64>) {
    if let Some(s) = secs.filter(|s| *s > 0) {
        let _ = NETWORK_TIMEOUT_OVERRIDE.set(Duration::from_secs(s));
    }
}

/// The effective per-attempt connect deadline: the operator override if set,
/// else [`ZENOH_OPEN_TIMEOUT`].
fn session_open_timeout() -> Duration {
    NETWORK_TIMEOUT_OVERRIDE
        .get()
        .copied()
        .unwrap_or(ZENOH_OPEN_TIMEOUT)
}
/// How long `--confirm` waits for the storage read-back. Storage
/// replies are local to the router, so this is short; a denial returns
/// zero replies and we hit the timeout's empty-stream end quickly.
const CONFIRM_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Args, Debug)]
pub struct PubArgs {
    /// Topic key to publish to (e.g. `acme/observations/v1`).
    pub key: String,
    /// Payload text, given inline.
    ///
    /// Use --payload-file for a file, or pipe to stdin. With none of the
    /// three, reads stdin.
    #[arg(long, conflicts_with = "payload_file")]
    pub payload: Option<String>,
    /// Payload from a file (use --payload for inline text, or pipe stdin).
    #[arg(long)]
    pub payload_file: Option<PathBuf>,
    /// Stamp the sample's self-describing wire encoding. Shorthands:
    /// `json`, `cbor`, `text`, `protobuf`, `protobuf:<MessageName>`
    /// (the message name rides along so a consumer knows which descriptor
    /// to decode with). Any other value is passed through as a media type
    /// verbatim. Unset leaves the Zenoh default (`zenoh/bytes`).
    #[arg(long)]
    pub encoding: Option<String>,
    /// After publishing, read the value back from router storage to
    /// verify it landed.
    ///
    /// `put` is fire-and-forget: it returns locally even when the router
    /// denied the write (an un-onboarded prefix, or no write-ACL for
    /// your cert), so you normally get no signal. --confirm turns that
    /// into a clear landed / not-confirmed verdict and a non-zero exit
    /// when not confirmed. Use a unique payload — a stale identical
    /// value already in storage can mask a denial.
    #[arg(long)]
    pub confirm: bool,
    /// Publish with DROP congestion control for the loss-tolerant
    /// `volatile/<peer>/stream/**` firehose, so a high-rate producer
    /// never back-pressures the session.
    ///
    /// Incompatible with --confirm: volatile keys are not retained, so
    /// there is nothing to read back.
    #[arg(long, conflicts_with = "confirm")]
    pub volatile_stream: bool,

    /// Encrypt the payload to the release-set's granted audience before
    /// publishing (grant-keyed encryption, ADR 1094). The key you publish
    /// under must be your OWN producer anchor subtree; the DEK comes from the
    /// key bundle grant-keyd wraps to you. Needs the signer-trust flags below.
    #[arg(long)]
    pub sealed: bool,
    /// (with --sealed/--open) The key service's SPIRE trust bundle PEM — the
    /// root grant-keyd's bundle-signing SVID chains to. Fail-closed: an
    /// unverifiable signer is refused.
    #[arg(long)]
    pub key_service_bundle: Option<String>,
    /// (with --sealed/--open) The goatnet's SPIFFE trust domain (e.g.
    /// zagros.zagros.example).
    #[arg(long)]
    pub trust_domain: Option<String>,
    /// (with --sealed/--open) The key service's SPIFFE id. Defaults to
    /// spiffe://<trust-domain>/grant-keyd.
    #[arg(long)]
    pub key_service_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct SubArgs {
    /// Zenoh key expression to subscribe to (e.g. `acme/**`).
    pub key_pattern: String,
    /// Accepted and ignored: streaming until you stop it IS the default.
    /// Kept so the many runbooks, demo pages and muscle-memory invocations
    /// that spell `--follow`/`--watch` keep working unchanged.
    #[arg(long, visible_alias = "watch")]
    pub follow: bool,
    /// Decode payload bytes against the topic's portal-registered schema.
    #[arg(long)]
    pub decode: bool,
    /// Interactive Tail lens for live tail (default is line-oriented):
    /// event table, rate sparkline, on-demand decoded/hex viewer.
    /// Requires a terminal (refuses on a non-TTY).
    #[arg(long)]
    pub tui: bool,
    /// Exit after this many events. Omitted, `goat sub` streams until you
    /// stop it — a live tail is what an operator almost always wants, and
    /// the old exit-after-one default made a working subscription look like
    /// a failure. `--take` is the same flag (the spelling every health
    /// runbook already uses).
    #[arg(long, visible_alias = "take")]
    pub count: Option<usize>,
    /// Print only each event's key, never the payload. The prefix-census
    /// form: `goat sub '**' --keys-only --take 200 | sort -u`.
    #[arg(long, conflicts_with_all = ["decode", "open"])]
    pub keys_only: bool,
    /// Stop after this long (`30s`, `2m`, `1h30m`) whether or not `--take`
    /// was satisfied. A bounded tail otherwise waits forever for events that
    /// may never come, so a runbook acceptance check hangs instead of
    /// failing. With `--take`, an unmet count at the deadline exits non-zero;
    /// without it, reaching the deadline is simply the end of the tail.
    #[arg(long = "for", value_name = "DURATION")]
    pub for_: Option<String>,
    /// Replay past events from the audit-tail instead of subscribing live.
    /// A duration (`1h`, `90m`) or an RFC3339 timestamp.
    #[arg(long)]
    pub since: Option<String>,

    /// Decrypt each received sample with the grant-keyed DEK you hold
    /// (grant-keyed encryption, ADR 1094). Query-history-then-subscribe: you
    /// get retained samples plus the live tail. A sample you have no key for
    /// prints DENIED — the read cut, enforced by the crypto, not the router.
    #[arg(long, conflicts_with = "decode")]
    pub open: bool,
    /// (with --open) The key service's SPIRE trust bundle PEM.
    #[arg(long)]
    pub key_service_bundle: Option<String>,
    /// (with --open) The goatnet's SPIFFE trust domain.
    #[arg(long)]
    pub trust_domain: Option<String>,
    /// (with --open) The key service's SPIFFE id (default spiffe://<td>/grant-keyd).
    #[arg(long)]
    pub key_service_id: Option<String>,

    /// Subscribe on the key exactly as written — skip slot-alias resolution of
    /// the leading segment. For a namespace that is not a slot anchor on a
    /// slot-pool goatnet: a seeded per-vendor prefix such as `airdefenders/`
    /// (seeded beside the slot pool), where the resolver would refuse
    /// the segment as an unknown alias. Read-only, so a typo costs an empty
    /// tail, never a phantom write prefix (the F-177 case the fail-closed
    /// resolver exists for). `goat demo` sets this for its generated key.
    #[arg(long)]
    pub literal: bool,
}

/// The one place `goat sub` turns the operator's pattern into the key it
/// subscribes on: alias-resolved (ADR 1039, volatile-aware per ADR 1032) unless
/// `--literal` says the pattern already IS the key.
fn sub_pattern(
    reg: &panoscope_core::alias::AliasRegistry,
    pattern: &str,
    literal: bool,
) -> anyhow::Result<String> {
    let pattern = pattern.trim();
    if literal {
        return Ok(pattern.to_string());
    }
    crate::alias::resolve_key_volatile_aware(reg, pattern).map_err(|e| anyhow::anyhow!("{e}"))
}

pub async fn run_pub(ctx: &super::Ctx, mut args: PubArgs) -> anyhow::Result<()> {
    // Resolve an alias to the canonical UUID anchor (ADR 1039) before any
    // wire op; an unknown alias fails closed. Volatile-aware so a
    // `volatile/<peer>/...` key resolves the peer in the 2nd segment rather
    // than rejecting the literal `volatile` (ADR 1032). `shown` is the human
    // form for the confirmation line (honors --uuid).
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    args.key = crate::alias::resolve_key_volatile_aware(&reg, args.key.trim())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let shown = crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &args.key);

    let session = open_zenoh_session(&ctx.profile).await?;
    let mut payload = read_payload(args.payload.as_deref(), args.payload_file.as_deref()).await?;

    // --sealed: encrypt the payload to the release-set's granted audience
    // BEFORE it goes on the wire. The bytes published are the self-describing
    // sealed envelope; the read cut is the crypto (ADR 1094). Fail-closed: if
    // we can't acquire a verified DEK for our own release-set, we do not
    // publish plaintext.
    if args.sealed {
        let trust = crate::cmd::sealed::SealTrust::from_flags(
            args.key_service_bundle.clone(),
            args.trust_domain.clone(),
            args.key_service_id.clone(),
        )?;
        let me = crate::cmd::sealed::SelfMember::from_profile(&ctx.profile)?;
        payload = crate::cmd::sealed::seal_for(
            &session,
            &args.key,
            &payload,
            &me,
            &trust,
            &ctx.profile.router_endpoint_label(),
        )
        .await?;
        eprintln!("sealed: payload encrypted to the granted audience for release-set {}", me.anchor);
    }

    // --volatile-stream sets DROP congestion control so a high-rate firehose
    // never back-pressures the session; default is Zenoh's normal handling.
    let publisher = {
        let builder = session.declare_publisher(args.key.clone());
        let builder = if args.volatile_stream {
            builder.congestion_control(zenoh::qos::CongestionControl::Drop)
        } else {
            builder
        };
        builder
            .await
            .map_err(|e| anyhow::anyhow!("declare publisher on `{}`: {e}", args.key))?
    };
    let bytes = payload.len();
    // Keep a copy of the payload for the round-trip read-back only when
    // --confirm is set — otherwise `put` moves it with zero extra cost.
    let confirm_expect = if args.confirm {
        Some(payload.clone())
    } else {
        None
    };
    let narrator = crate::narrator::Narrator::new(ctx.narrate, ctx.narrate_json);
    // Self-describing wire encoding (zenoh-capability-adoption gap 1). When
    // --encoding is given, stamp the sample's Encoding so consumers
    // (flashlight/panoscope/the schema viewer) can pick a decoder from the
    // wire instead of an out-of-band lookup. Default (unset) leaves the
    // Zenoh default `zenoh/bytes`, unchanged from before.
    let mut put = publisher.put(payload);
    if let Some(enc) = args.encoding.as_deref() {
        put = put.encoding(goat_encoding(enc));
    }
    put.await
        .map_err(|e| anyhow::anyhow!("put on `{}`: {e}", args.key))?;
    eprintln!("published {bytes} bytes to `{shown}`");

    // --confirm: round-trip read-back. Runs BEFORE the optimistic
    // narrator "put confirmed" line so a real denial bails here and
    // never narrates a contradictory success. Zenoh `put` is
    // fire-and-forget — the only honest confirmation a publisher can
    // get is to query the key back out of storage.
    if let Some(expect) = confirm_expect.as_deref() {
        match confirm_landed(&session, &args.key, expect).await {
            ConfirmOutcome::Landed => {
                eprintln!(
                    "confirm: ✓ landed — the published value is retained and queryable in router storage"
                );
            }
            ConfirmOutcome::ValueMismatch => {
                // A value exists for the key but it isn't the one we just
                // published. Ambiguous: could be a denial masked by a
                // prior retained value, or a concurrent publisher. Warn,
                // but don't claim failure — exit 0.
                eprintln!(
                    "confirm: ⚠ a value exists for `{}` but does not match what was just published.",
                    args.key
                );
                eprintln!(
                    "  Possible denial masked by a prior retained value, or another publisher on the key."
                );
                eprintln!("  Re-test with a unique payload to disambiguate.");
            }
            ConfirmOutcome::Unverifiable(why) => {
                // The publish itself returned ok; we just couldn't run the
                // read-back (query error / timeout). Don't fail the publish
                // on an inconclusive check — exit 0 with a note.
                eprintln!("confirm: could not verify ({why}); the publish itself returned ok");
            }
            ConfirmOutcome::NotFound => {
                eprintln!(
                    "confirm: ✗ NOT confirmed — nothing came back for `{}`.",
                    args.key
                );
                eprintln!(
                    "  This is exactly what an ACL ingress-denial looks like to a publisher: `put`"
                );
                eprintln!(
                    "  returned locally (Zenoh is fire-and-forget) but the router accepted nothing."
                );
                eprintln!("  Likely causes:");
                eprintln!(
                    "    - the key's prefix is not onboarded for your cert (no write-ACL for `{}`)",
                    args.key
                );
                eprintln!(
                    "    - no storage is registered for the prefix (an un-onboarded namespace)"
                );
                eprintln!(
                    "  Run `goat doctor` and check your onboarded org prefix; see docs/design/goat-cli.md."
                );
                let _ = session.close().await;
                anyhow::bail!("publish to `{}` was not confirmed in router storage", args.key);
            }
        }
    }

    if narrator.is_enabled() {
        narrator.emit(crate::narrator::Consequence::ZenohPutConfirmed {
            key: args.key.clone(),
            bytes,
        });
        // Two inline cascade lines are still deferred for `pub --narrate`:
        //   - subscriber-delivery: Zenoh `liveliness` observation scoped to
        //     the key, then count of liveliness tokens that also exposed an
        //     `intake` query. Designed in docs/design/zenoh-trial-fabric.md
        //     as a future pubsub-observability handle.
        //   - audit-tail-receipt: would poll the just-published event back
        //     out of the audit-tail within the cascade deadline. The read
        //     path itself now exists (G.6 portal-presign — see `goat replay`
        //     / `goat narrate`); wiring an inline per-mutation poll is the
        //     remaining piece.
        // Until both land, we emit a Note so the operator sees the line is
        // deliberately deferred, not silently dropped.
        narrator.emit(crate::narrator::Consequence::Note(
            "subscriber-delivery + inline audit-tail-receipt lines are not yet wired; use `goat narrate` for the standalone audit-tail view".into()
        ));
        narrator.settled("pub");
    }
    let _ = session.close().await;
    Ok(())
}

pub async fn run_sub(ctx: &super::Ctx, args: SubArgs) -> anyhow::Result<()> {
    // Refuse the two bounds that ask for nothing, before anything is opened.
    // `--for 0` is already refused in `parse_for_duration`; `--count 0` is the
    // same request spelled the other way — the loop would exit satisfied
    // having never declared a subscriber, checked a router or read a sample,
    // and the closing summary would report a run that never happened.
    if args.count == Some(0) {
        anyhow::bail!(
            "--count/--take 0 asks for no events — drop the flag to stream until you stop it, \
             or ask for at least 1"
        );
    }
    // Parsed here, armed later: a malformed `--for` must fail before the
    // session is opened and the subscription is live, not after. The deadline
    // itself only starts once the tail is actually running (in `run_tail`).
    let deadline_after = args.for_.as_deref().map(parse_for_duration).transpose()?;
    // --tui opens the interactive Tail lens. TTY-gate FIRST (before any
    // profile/endpoint resolution or session open) so a non-TTY
    // invocation refuses cheaply and deterministically — a piped/CI
    // `goat sub --tui` never needs a profile or the mesh to get the
    // refusal. Per docs/design/goat-cli-tui.md (Phase C).
    if args.tui {
        crate::tui::require_tty("goat sub --tui")?;
        // Resolve an alias in the pattern (ADR 1039); unknown alias fails
        // closed. Volatile-aware (ADR 1032). The tui subscribes on canonical.
        let reg = crate::alias::load_aliases(&ctx.profile).await;
        let kp = sub_pattern(&reg, &args.key_pattern, args.literal)?;
        // Decode-on-select needs the portal's `.fds`; degrade to
        // hex-only (decode_available = false in the lens) when the
        // profile carries no portal rather than refusing to launch.
        let decode = ctx
            .profile
            .portal_client()
            .ok()
            .map(super::decode::DecodeCache::new);
        // Read-only subscriber: NO ZID pin (F-273 — a later same-cert
        // `goat pub` displaces a pinned subscriber's transport, silently).
        let session = open_session_readonly(&ctx.profile).await?;
        return crate::tui::tail::run(session, ctx.profile.router_endpoint_label(), decode, kp).await;
    }
    // --since: replay past events from the audit-tail instead of subscribing
    // live (Phase G.6). Shares the portal-presign read path with `goat replay`
    // and standalone `goat narrate`.
    if let Some(since) = args.since.as_deref() {
        let reg = crate::alias::load_aliases(&ctx.profile).await;
        let pattern = sub_pattern(&reg, &args.key_pattern, args.literal)?;
        let events = super::replay::fetch_events(ctx, Some(since), &pattern).await?;
        // `--take N` bounds a replay the same way it bounds a live tail, and
        // `--keys-only` reduces it to the prefix census the health runbooks
        // build with `| sort -u`.
        for ev in events.iter().take(sub_limit(args.count)) {
            if args.keys_only {
                println!(
                    "{}",
                    crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &ev.key_expr)
                );
            } else {
                println!("{}", super::replay::render_event(&reg, ctx.uuid, ev));
            }
        }
        if events.is_empty() {
            eprintln!(
                "no audit events for `{}` since {since}",
                crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &pattern)
            );
        } else {
            eprintln!("{} event(s) replayed", events.len());
        }
        return Ok(());
    }

    // Phase D.B.1 — --decode wired. Build the PortalClient now so a
    // missing profile / bad token surfaces E_PROFILE_MISSING /
    // E_TOKEN_INVALID at subscribe-time rather than on the first
    // sample.
    let mut decoder = if args.decode {
        let portal = ctx.profile.portal_client()?;
        Some(super::decode::DecodeCache::new(portal))
    } else {
        None
    };

    // Resolve an alias in the pattern to the canonical UUID anchor (ADR 1039);
    // unknown alias fails closed. Volatile-aware so `volatile/<peer>/**`
    // resolves the peer segment (ADR 1032). Subscribe on canonical; render
    // incoming keys back to aliases for display.
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let key_pattern = sub_pattern(&reg, &args.key_pattern, args.literal)?;
    let shown_pattern = crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &key_pattern);

    // Read-only subscriber: NO ZID pin. The router permits one
    // transport per ZID, and both halves of the self-subscription smoke test
    // ("goat sub my topic" + "goat pub to it") used to pin the same anchor —
    // the pub's session displaced the live subscriber and it received zero,
    // silently. A pure subscriber earns nothing from the pin (provenance is a
    // PUBLISHER property, src_zid == anchor; authz rides the cert CN), so it
    // takes a random ZID like `goat top` always has (open_session_readonly).
    let session = open_session_readonly(&ctx.profile).await?;

    // --open: the grant-keyed read path. Fetch+verify the release-set's key
    // bundles, admit the slots we can open, then query-history-then-subscribe
    // (the durable consumer pattern — a live-only subscriber misses anything
    // published before its declaration propagates; retention storage carries
    // the history, the tail carries new samples). A sample we hold no key for
    // prints DENIED — the read cut is the crypto, not the router.
    if args.open {
        return run_sub_open(
            ctx,
            &session,
            &args,
            &reg,
            &key_pattern,
            &shown_pattern,
            deadline_after,
        )
        .await;
    }

    let router = ctx.profile.router_endpoint_label();
    let subscriber = session
        .declare_subscriber(key_pattern.clone())
        .await
        .map_err(|e| crate::classify::classify_zenoh_err(e, &router, "declare a subscriber"))?;

    let want = sub_limit(args.count);
    eprintln!("{}", listening_line(&shown_pattern, &router, want));

    let self_cmd = self_cmd_of(&args);
    // `--literal` subscribes on the string as typed. When the leading segment
    // is ALSO a live slot alias, that string is not what is on the wire —
    // publishers put samples under the UUID anchor — so the tail is silent
    // forever and looks exactly like a quiet key. Worked out once here so the
    // quiet report can say it instead of leaving the operator to guess.
    let literal_hint = literal_alias_hint(&reg, &args);
    let tail = Tail {
        shown_pattern: &shown_pattern,
        router: &router,
        self_cmd: &self_cmd,
        literal_alias_hint: literal_hint.as_deref(),
        want,
        deadline_after,
        count_asked: args.count.is_some(),
    };
    let mut sink = SampleSink::Lines {
        reg: &reg,
        uuid: ctx.uuid,
        keys_only: args.keys_only,
        decoder: &mut decoder,
    };
    let run = run_tail(&session, subscriber.handler(), &tail, &mut sink, 0).await;
    let _ = session.close().await;
    finish_tail(&tail, run)
}

/// How long a silent subscription waits before saying, out loud, that it is
/// healthy and the key is simply quiet.
const QUIET_CHECKPOINT: std::time::Duration = std::time::Duration::from_secs(5);

/// How a live `goat sub` stopped. Only `NotAttached` and `LinkLost` are
/// failures; the other two are the operator getting what they asked for.
enum SubEnd {
    /// `--count` / `--take` was satisfied.
    Satisfied,
    /// Ctrl-C.
    Interrupted,
    /// The subscriber channel died mid-stream (session gone).
    LinkLost(String),
    /// The session is attached to no router at all.
    NotAttached,
    /// `--for <duration>` elapsed.
    Deadline,
}

/// The ZIDs of the routers this session is actually attached to. Empty means
/// the session is local-only — open, declaring fine, and connected to nothing.
async fn attached_routers(session: &zenoh::Session) -> Vec<String> {
    session
        .info()
        .routers_zid()
        .await
        .map(|zid| zid.to_string())
        .collect()
}

/// A second look before calling a session unattached. `open()` returning Ok
/// with no router attached is F-133, but it is also what a handshake still in
/// flight looks like — declaring `E_ROUTER_UNREACHABLE` at exactly the
/// checkpoint for a link that comes up a moment later is its own false
/// negative. Only two consecutive empty reads are treated as attached-to-
/// nothing.
const ATTACH_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Everything a live tail needs besides the subscriber itself. Built once and
/// shared by the plain tail and the grant-keyed (`--open`) one, so both get
/// the same Ctrl-C handling, the same `--for` deadline, the same quiet
/// checkpoint and the same F-133 attachment check. Keeping two copies of this
/// loop is how `--open` came to accept `--for` and silently ignore it.
struct Tail<'a> {
    shown_pattern: &'a str,
    router: &'a str,
    /// The operator's OWN invocation, `--literal` and all, so a hint printed
    /// mid-tail is copy-pasteable.
    self_cmd: &'a str,
    literal_alias_hint: Option<&'a str>,
    want: usize,
    deadline_after: Option<std::time::Duration>,
    /// Whether a count was asked for at all. An unmet `--count` is a failed
    /// expectation; an unbounded tail ending on Ctrl-C or `--for` is not.
    count_asked: bool,
}

/// Where a received sample goes — the only thing the two tails do
/// differently. Line-oriented printing for `goat sub`, grant-keyed opening
/// for `goat sub --open`.
enum SampleSink<'a> {
    Lines {
        reg: &'a panoscope_core::alias::AliasRegistry,
        uuid: bool,
        keys_only: bool,
        decoder: &'a mut Option<super::decode::DecodeCache>,
    },
    Opened {
        reg: &'a panoscope_core::alias::AliasRegistry,
        uuid: bool,
        store: &'a grant_keyd::datapath::KeyStore,
    },
}

impl SampleSink<'_> {
    async fn emit(&mut self, sample: &zenoh::sample::Sample) {
        let key = sample.key_expr().as_str().to_string();
        match self {
            SampleSink::Lines { reg, uuid, keys_only, decoder } => {
                // Render the (canonical) incoming key back to its alias for
                // display; decoding still keys on the canonical `key`.
                // Volatile-aware so a `volatile/<uuid>/...` sample renders the
                // peer, not the literal.
                let shown_key = crate::alias::display_key_volatile_aware(reg, *uuid, &key);
                if *keys_only {
                    println!("{shown_key}");
                    return;
                }
                let payload = sample.payload().to_bytes();
                match decoder.as_mut() {
                    Some(dec) => {
                        let decoded = dec.decode(&key, &payload).await;
                        println!("{shown_key} {decoded}");
                    }
                    None => print_sample(&shown_key, &payload),
                }
            }
            SampleSink::Opened { reg, uuid, store } => {
                emit_opened(store, reg, *uuid, &key, &sample.payload().to_bytes());
            }
        }
    }
}

/// How a tail ended, plus what the closing summary is entitled to say.
struct TailRun {
    end: SubEnd,
    got: usize,
    elapsed: std::time::Duration,
    /// True only if attachment to a router was actually OBSERVED during the
    /// run — a sample arrived, or the checkpoint read a non-empty router
    /// list. A run that ended before the checkpoint has checked nothing and
    /// must not claim it was attached.
    attachment_seen: bool,
}

/// The live tail: print samples until the count is satisfied, the deadline
/// passes, the operator interrupts, or the link dies — saying at the
/// checkpoint what continued silence means.
async fn run_tail(
    session: &zenoh::Session,
    rx: &zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>,
    tail: &Tail<'_>,
    sink: &mut SampleSink<'_>,
    already_got: usize,
) -> TailRun {
    let started = std::time::Instant::now();
    let mut got = already_got;
    let mut attachment_seen = false;
    // One ctrl_c future for the whole loop, not one per iteration: a freshly
    // created future can only observe a signal that arrives after it
    // registers, so re-creating it each pass leaves a window in which a
    // Ctrl-C landing between iterations is swallowed.
    let ctrlc = tokio::signal::ctrl_c();
    tokio::pin!(ctrlc);
    // The silence checkpoint. Without it, an operator watching a subscription
    // that has printed nothing has no way to tell "healthy and quiet" from
    // "broken" — the false negative this path exists to kill.
    let quiet_check = tokio::time::sleep(QUIET_CHECKPOINT);
    tokio::pin!(quiet_check);
    let mut quiet_check_pending = true;
    // `--for` was parsed before the session opened; it is ARMED here, from the
    // moment the tail is actually live.
    let deadline = tokio::time::sleep(tail.deadline_after.unwrap_or(std::time::Duration::MAX));
    tokio::pin!(deadline);

    let end = loop {
        if got >= tail.want {
            break SubEnd::Satisfied;
        }
        tokio::select! {
            biased;
            _ = &mut ctrlc => break SubEnd::Interrupted,
            _ = &mut deadline, if tail.deadline_after.is_some() => break SubEnd::Deadline,
            _ = &mut quiet_check, if quiet_check_pending => {
                quiet_check_pending = false;
                // An empty router list means zenoh opened a session attached
                // to nothing: `session.open()` returns Ok even when every
                // configured connect endpoint failed, and the
                // subscriber then declares happily against a local-only
                // session — indistinguishable from a quiet key unless we ask
                // who we are actually attached to.
                let mut routers = attached_routers(session).await;
                if routers.is_empty() {
                    tokio::time::sleep(ATTACH_GRACE).await;
                    routers = attached_routers(session).await;
                }
                if routers.is_empty() {
                    break SubEnd::NotAttached;
                }
                attachment_seen = true;
                if got == already_got {
                    for line in quiet_report(
                        tail.shown_pattern,
                        tail.self_cmd,
                        &routers,
                        started.elapsed(),
                    ) {
                        eprintln!("{line}");
                    }
                    if let Some(note) = tail.literal_alias_hint {
                        eprintln!("{note}");
                    }
                }
            }
            sample = rx.recv_async() => {
                let sample = match sample {
                    Ok(s) => s,
                    Err(e) => break SubEnd::LinkLost(e.to_string()),
                };
                sink.emit(&sample).await;
                got += 1;
            }
        }
    };
    TailRun {
        end,
        got,
        elapsed: started.elapsed(),
        // A sample that arrived is proof of attachment on its own.
        attachment_seen: attachment_seen || got > already_got,
    }
}

/// Turn how the tail ended into the closing line and the exit code. Shared by
/// both tails so `goat sub` and `goat sub --open` cannot drift on what an
/// unmet count, a dead link or an unattached session means.
fn finish_tail(tail: &Tail<'_>, run: TailRun) -> anyhow::Result<()> {
    let TailRun { end, got, elapsed, attachment_seen } = run;
    let interrupted = matches!(end, SubEnd::Interrupted);
    let summary = stop_summary(
        tail.shown_pattern,
        got,
        tail.want,
        elapsed,
        interrupted,
        attachment_seen,
    );
    match end {
        SubEnd::Satisfied | SubEnd::Interrupted | SubEnd::Deadline => {
            eprintln!("{summary}");
            // An explicit `--take N` that did not arrive is a failed
            // expectation, whichever way the wait ended — a runbook check has
            // to go red rather than pass on a partial read. An unbounded tail
            // ending on Ctrl-C or `--for` got exactly what was asked of it.
            if tail.count_asked && got < tail.want {
                anyhow::bail!(
                    "stopped before the requested {} event(s) arrived — got {got}",
                    tail.want
                );
            }
            Ok(())
        }
        // The link died mid-stream. Report what the run did see, then
        // classify the loss the way a failed session open is classified: a
        // router that went away is E_ROUTER_UNREACHABLE (5), not a bare exit
        // 1, and the raw multi-line zenoh text never becomes the primary
        // message.
        SubEnd::LinkLost(e) => {
            eprintln!("{summary}");
            Err(crate::classify::classify_zenoh_err(
                e,
                tail.router,
                &format!(
                    "keep the subscription to `{}` alive (it ended after {}/{} sample(s) in {})",
                    tail.shown_pattern,
                    got,
                    want_label(tail.want),
                    human_duration(elapsed)
                ),
            ))
        }
        // Not a quiet key and not a partial read: nothing was ever attached.
        SubEnd::NotAttached => Err(crate::errors::GoatError::RouterUnreachable {
            endpoint: tail.router.to_string(),
            hint: format!(
                "the fabric session opened but is attached to NO router after {} — zenoh's \
                 `open()` succeeds even when every connect endpoint failed, so the subscription \
                 to `{}` would have sat silent forever. Diagnose the router at {} with \
                 `goat doctor`.",
                human_duration(elapsed),
                tail.shown_pattern,
                tail.router
            ),
        }
        .into()),
    }
}

/// The operator's own invocation, rebuilt for a copy-pasteable hint. Built
/// from the flags they typed, not from the resolved pattern — handing back a
/// command that fails alias resolution on a slot-pool goatnet is the exact
/// trap the hint exists to get them out of.
fn self_cmd_of(args: &SubArgs) -> String {
    let mut cmd = "goat sub".to_string();
    if args.literal {
        cmd.push_str(" --literal");
    }
    if args.open {
        cmd.push_str(" --open");
    }
    format!("{cmd} '{}'", args.key_pattern.trim())
}

/// The `--literal`-on-an-alias note, or None when it does not apply.
fn literal_alias_hint(
    reg: &panoscope_core::alias::AliasRegistry,
    args: &SubArgs,
) -> Option<String> {
    if !args.literal {
        return None;
    }
    let typed = args.key_pattern.trim();
    crate::alias::resolve_key_volatile_aware(reg, typed)
        .ok()
        .filter(|resolved| resolved.as_str() != typed)
        .map(|resolved| literal_alias_note(typed, &resolved))
}

/// `--count N` / `--take N` bounds the tail; omitting it streams. The old
/// default was 1, which exited on the first event — and on a quiet key read
/// as a hang, then as a failure.
fn sub_limit(count: Option<usize>) -> usize {
    count.unwrap_or(usize::MAX)
}

fn want_label(want: usize) -> String {
    if want == usize::MAX {
        "∞".to_string()
    } else {
        want.to_string()
    }
}

/// The line printed the moment the subscription is live. It names the router,
/// so the operator can see the subscription is attached to something real,
/// and says plainly what silence will mean from here on.
fn listening_line(shown_pattern: &str, router: &str, want: usize) -> String {
    let mode = if want == usize::MAX {
        "streaming live events".to_string()
    } else {
        format!("stopping after {want} event(s)")
    };
    format!("listening on `{shown_pattern}` via {router} — {mode}. Ctrl-C to stop.")
}

/// What to say when the subscription has been up a while and seen nothing.
/// The point is to state the two things the operator cannot see: the session
/// IS attached to a router (named, so it is checkable), and nothing is
/// publishing on the key right now.
fn quiet_report(
    shown_pattern: &str,
    self_cmd: &str,
    routers: &[String],
    waited: std::time::Duration,
) -> Vec<String> {
    vec![
        format!(
            "… {} in, 0 events. Attached to router {} — the subscription is live; nothing is \
             publishing on `{shown_pattern}` right now.",
            human_duration(waited),
            routers.join(", "),
        ),
        // `self_cmd` is the operator's OWN invocation, `--literal` and all, so
        // the replay hint is copy-pasteable. Rebuilding it from the resolved
        // pattern would hand back a command that fails alias resolution on a
        // slot-pool goatnet — the exact trap this line exists to get them out of.
        format!("  what was published earlier:  {self_cmd} --since 1h"),
        "  what exists on this goatnet: goat topic ls".to_string(),
        format!("  whether a read is allowed:   goat access why me '{shown_pattern}'"),
    ]
}

/// The `--literal`-on-an-alias trap, spelled out. Subscribing verbatim to a
/// name that resolves to a UUID anchor can never match a sample, because the
/// publisher's wire key starts with the anchor — an empty tail that reads as
/// a dead topic. `--literal` still has a real use (a seeded vendor prefix the
/// fail-closed resolver refuses), so this is a note, not a refusal.
fn literal_alias_note(pattern: &str, resolved: &str) -> String {
    let alias = pattern.split('/').next().unwrap_or(pattern);
    format!(
        "  note: --literal subscribed to `{pattern}` verbatim, but `{alias}` is a live slot \
         alias — publishers write under `{resolved}`, so this tail can never match. Drop \
         --literal to subscribe on the anchor."
    )
}

/// Parse `--for`: `45s` / `2m` / `1h` / `1h30m`, or a bare number of seconds.
/// Deliberately narrow — a wrong guess here silently changes how long an
/// acceptance check waits, so anything unrecognised is refused rather than
/// rounded.
fn parse_for_duration(spec: &str) -> anyhow::Result<std::time::Duration> {
    let spec = spec.trim();
    if spec.is_empty() {
        anyhow::bail!("--for needs a duration like `30s`, `2m` or `1h30m`");
    }
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut saw_unit = false;
    for c in spec.chars() {
        match c {
            '0'..='9' => digits.push(c),
            's' | 'm' | 'h' => {
                let n: u64 = digits
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--for: expected a number before `{c}` in `{spec}`"))?;
                digits.clear();
                saw_unit = true;
                total += match c {
                    's' => n,
                    'm' => n * 60,
                    _ => n * 3600,
                };
            }
            _ => anyhow::bail!(
                "--for: `{spec}` is not a duration — use seconds/minutes/hours, e.g. `30s`, `2m`, `1h30m`"
            ),
        }
    }
    // A bare `--for 90` is seconds, matching `--timeout`.
    if !digits.is_empty() {
        if saw_unit {
            anyhow::bail!("--for: trailing `{digits}` in `{spec}` has no unit");
        }
        total += digits.parse::<u64>()?;
    }
    if total == 0 {
        anyhow::bail!("--for: `{spec}` is zero — a tail that stops before it starts is never what you want");
    }
    Ok(std::time::Duration::from_secs(total))
}

/// The closing line. Never silent: a run that saw nothing says so, with how
/// long it was connected — that is the evidence separating "quiet key" from
/// "broken tool".
fn stop_summary(
    shown_pattern: &str,
    got: usize,
    want: usize,
    elapsed: std::time::Duration,
    interrupted: bool,
    attachment_seen: bool,
) -> String {
    let verb = if interrupted { "stopped" } else { "done" };
    let base = format!(
        "{verb} after {} — {got} event(s) on `{shown_pattern}`",
        human_duration(elapsed)
    );
    if got == 0 {
        // Only the checkpoint establishes attachment, and a run that ended
        // before it reached the checkpoint (a short `--for`, an immediate
        // Ctrl-C) has checked nothing. Saying "attached the whole time"
        // there would be the same unearned confidence this command exists
        // to stop printing.
        if attachment_seen {
            format!("{base}; attached to a router when last checked, nothing was published")
        } else {
            format!(
                "{base}; the run ended before the {}s attachment check, so whether the session \
                 was attached is UNKNOWN — re-run without `--for` (or with a longer one) to find out",
                QUIET_CHECKPOINT.as_secs()
            )
        }
    } else if want != usize::MAX && got >= want {
        format!("{base} (the requested {want})")
    } else {
        base
    }
}

fn human_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// The `goat sub --open` body: grant-keyed decryption of a subscribed stream.
/// Verifies the key bundles' signer, admits the caller's slots, then
/// query-history-then-subscribes, opening each sealed sample and printing the
/// plaintext (or DENIED for a sample the caller holds no key for).
async fn run_sub_open(
    ctx: &super::Ctx,
    session: &zenoh::Session,
    args: &SubArgs,
    reg: &panoscope_core::alias::AliasRegistry,
    key_pattern: &str,
    shown_pattern: &str,
    deadline_after: Option<std::time::Duration>,
) -> anyhow::Result<()> {
    let router = ctx.profile.router_endpoint_label();
    let trust = crate::cmd::sealed::SealTrust::from_flags(
        args.key_service_bundle.clone(),
        args.trust_domain.clone(),
        args.key_service_id.clone(),
    )?;
    let me = crate::cmd::sealed::SelfMember::from_profile(&ctx.profile)?;
    let rs = crate::cmd::sealed::release_set_of(key_pattern)?;
    let (store, admitted) =
        crate::cmd::sealed::build_keystore(session, &rs, &me, &trust, &router).await?;
    if admitted == 0 {
        eprintln!(
            "open: no key bundle carried a slot for {} — every read will be DENIED (the expected \
             outcome for a non-member/outsider)",
            me.anchor
        );
    } else {
        eprintln!("open: hold {admitted} key generation(s) for release-set {rs}");
    }

    let want = sub_limit(args.count);
    eprintln!(
        "opening `{}` via {router} — {}. Ctrl-C to stop.",
        shown_pattern,
        if want == usize::MAX {
            "history, then streaming live events".to_string()
        } else {
            format!("history, then live, stopping after {want} event(s)")
        }
    );

    // (1) History: retained samples from storage. (2) Live tail. Both feed the
    // same open+print. A late joiner gets the stream from retention (#1920).
    let mut got = 0usize;
    let history = session
        .get(key_pattern.to_string())
        .timeout(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| crate::classify::classify_zenoh_err(e, &router, "query history"))?;
    while let Ok(reply) = history.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        emit_opened(&store, reg, ctx.uuid, sample.key_expr().as_str(), &sample.payload().to_bytes());
        got += 1;
        if got >= want {
            return Ok(());
        }
    }

    let subscriber = session
        .declare_subscriber(key_pattern.to_string())
        .await
        .map_err(|e| crate::classify::classify_zenoh_err(e, &router, "declare a subscriber"))?;

    // The same tail as the plain path, sample handling aside: `--for`, the
    // Ctrl-C arm, the quiet checkpoint and the F-133 attachment check all
    // apply here too. A grant-keyed reader in a runbook used to accept
    // `--for` and wait forever.
    let self_cmd = self_cmd_of(args);
    let literal_hint = literal_alias_hint(reg, args);
    let tail = Tail {
        shown_pattern,
        router: &router,
        self_cmd: &self_cmd,
        literal_alias_hint: literal_hint.as_deref(),
        want,
        deadline_after,
        count_asked: args.count.is_some(),
    };
    let mut sink = SampleSink::Opened { reg, uuid: ctx.uuid, store: &store };
    // `got` carries the history samples already opened above, so a `--take N`
    // partly satisfied from retention is not asked for N more from the tail.
    let run = run_tail(session, subscriber.handler(), &tail, &mut sink, got).await;
    finish_tail(&tail, run)
}

/// Open one sample with the keystore and print the plaintext (or DENIED).
fn emit_opened(
    store: &grant_keyd::datapath::KeyStore,
    reg: &panoscope_core::alias::AliasRegistry,
    uuid: bool,
    key: &str,
    body: &[u8],
) {
    let shown = crate::alias::display_key_volatile_aware(reg, uuid, key);
    match crate::cmd::sealed::open_sample(store, key, body) {
        Ok(plain) => match std::str::from_utf8(&plain) {
            Ok(text) => println!("{shown} {text}"),
            Err(_) => println!("{shown} <{} bytes decrypted (binary)>", plain.len()),
        },
        Err(reason) => println!("{shown} DENIED — {reason}"),
    }
}

/// Open a Zenoh session using the active profile's mTLS material.
/// Mirrors the path doctor step 3 uses; refactored here so future
/// pub/sub/compose surfaces can share one call site. Sibling
/// modules (e.g. cmd::mock) call this via `open_session_for`.
pub(crate) async fn open_session_for(profile: &Profile) -> anyhow::Result<zenoh::Session> {
    open_zenoh_session(profile).await
}

/// Open a session WITHOUT the ADR 1041 ZID-pin (random ZID). The router
/// permits one transport per ZID, so two pinned same-cert sessions collide
/// with `MAX_LINKS` — fatal for `goat blob`, whose send + receive are
/// concurrent sessions under one operator identity. Blob attribution
/// rides the key (`volatile/<anchor>/blob/...`) and authz rides the cert CN,
/// not the ZID, so a random ZID for a transient transfer loses nothing.
pub(crate) async fn open_session_for_transfer(profile: &Profile) -> anyhow::Result<zenoh::Session> {
    open_zenoh_session_opts(profile, false).await
}

/// Open a session for a READ-ONLY subscriber (`goat top`, `goat sub`, the
/// tail lens) WITHOUT the ADR 1041 ZID-pin (random ZID). Pinning the slot ZID earns no
/// provenance for a non-publisher (provenance is the publisher property
/// `src_zid == anchor`), and the router permits one transport per ZID — so a
/// held pinned session would MONOPOLIZE the operator's single slot identity,
/// giving any concurrent `goat` command (health/sub/another top) `MAX_LINKS`,
/// which Task 1's brief retry can't outlast on a session held for the duration
/// of the dashboard. A random ZID lets the dashboard coexist with other goat
/// commands; attribution/authz ride the cert CN, not the ZID. Same lever as
/// `open_session_for_transfer`.
pub(crate) async fn open_session_readonly(profile: &Profile) -> anyhow::Result<zenoh::Session> {
    open_zenoh_session_opts(profile, false).await
}

async fn open_zenoh_session(profile: &Profile) -> anyhow::Result<zenoh::Session> {
    open_zenoh_session_opts(profile, true).await
}

async fn open_zenoh_session_opts(
    profile: &Profile,
    pin_zid: bool,
) -> anyhow::Result<zenoh::Session> {
    // Check cert/key/ca-roots exist+are readable BEFORE handing their paths
    // to Zenoh — a missing/unreadable identity file used to surface as a
    // raw, multi-line Zenoh/tokio error, with a backtrace naming the build
    // machine's own cargo registry path and nothing about the actual cause.
    profile.require_mesh_ready()?;
    let toml = profile.read_classified(&profile.profile_toml_path())?;
    let router = crate::profile::resolve_router_endpoint(&toml).ok_or_else(|| {
        anyhow::anyhow!(
            "{} missing router_endpoint — re-run `goat profile init`",
            profile.profile_toml_path().display()
        )
    })?;

    // Name fallback (ADR 1082): when mesh DNS is down but a validly-signed
    // name snapshot answers, substitute the resolved IP into the locator so
    // pub/sub keeps working — with an honest stderr label. Live DNS is
    // never shadowed (the resolver tries it first); an IP-literal locator
    // (the common profile shape) short-circuits with no name involvement.
    let router = {
        let mut endpoint = router.clone();
        if let Some((host, port)) = crate::names::parse_zenoh_locator(&endpoint) {
            if let Ok(res) = crate::names::resolve_host_port(profile, &host, port).await {
                if let Some(note) = res.source.fallback_note() {
                    if let Some(sub) = crate::names::substitute_locator_host(&endpoint, &res.addr)
                    {
                        match note {
                            crate::names::FallbackNote::Snapshot(meta) => eprintln!(
                                "⚠ names: router resolved from signed snapshot serial {}, age {} (mesh DNS unreachable)",
                                meta.serial,
                                meta.age_human()
                            ),
                            crate::names::FallbackNote::SelfClaim { age_secs } => eprintln!(
                                "⚠ names: router resolved from a verified SELF-CLAIM, age {} (mesh DNS unreachable; signed by the name's own anchor)",
                                crate::names::age_human(age_secs)
                            ),
                            crate::names::FallbackNote::Observed { age_secs } => eprintln!(
                                "⚠ names: router resolved from NONCANONICAL (ad hoc) observed record, age {} (mesh DNS unreachable; name not in signed snapshot)",
                                crate::names::age_human(age_secs)
                            ),
                        }
                        endpoint = sub;
                    }
                }
            }
        }
        endpoint
    };

    let mut config = zenoh::Config::default();
    // Force client mode rather than the default peer mode. Without
    // this the session multicast-scouts for peers first + emits a
    // "Scouting delay elapsed" warning, AND samples published from
    // peer-mode sessions don't reliably round-trip through the
    // router. Client mode connects directly to the configured
    // endpoint, which is what every other goat-cli call wants.
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| anyhow::anyhow!("zenoh config mode: {e}"))?;
    config
        .insert_json5("connect/endpoints", &format!("[\"{router}\"]"))
        .map_err(|e| anyhow::anyhow!("zenoh config endpoint: {e}"))?;

    let tls = panoscope_core::tls::TlsConfig {
        enabled: true,
        root_ca_certificate: profile.ca_roots_path().display().to_string(),
        connect_certificate: profile.cert_path().display().to_string(),
        connect_private_key: profile.key_path().display().to_string(),
    };
    panoscope_core::tls::apply_tls_to_zenoh_config(&mut config, &tls)?;

    // Pin the Zenoh session id (ZID) to this peer's slot anchor (ADR 1039
    // increment 4b(i)). The slot anchor IS the cert CN (= sha256(salt:idx)
    // [:32], exactly 32 hex chars = 16 bytes, the ZID ceiling), so when the
    // cert is a slot-pool cert the transport ZID, the ACL slot, the
    // namespace anchor, and panoscope's node id become ONE UUID end-to-end —
    // and the node identity is stable across restarts (the bigger win) and
    // renders as the operator's alias instead of a fresh random hex.
    //
    // Self-gating on shape: only a 32-hex CN is pinned. Legacy/shared certs
    // (`CN=<vendor>-trial-peer`, the synth fabric's shared cert) are
    // NOT anchor-shaped, so they keep a random ZID — exactly today's
    // behavior, no collision. The ZID is NOT an authz handle (Zenoh keys
    // access_control on cert CN, never ZID — §10), so this is pure display/
    // transport plumbing; a parse failure just skips the pin, never fails
    // the session. Applies to `goat pub` only — `goat sub` deliberately does
    // NOT pin: with one transport per ZID, a later same-cert pub
    // displaced a pinned live subscriber silently; a pure subscriber earns no
    // provenance from the pin, so it keeps a random ZID (open_session_readonly).
    //
    // The pin value is `zid_pin_value(anchor)`, NOT the anchor verbatim: a ZID
    // is a u128 in MINIMAL hex, so an anchor with a leading `0` is rejected at
    // config load. Fail-open
    // twice over — normalize first, and if the insert still refuses, skip the
    // pin rather than kill the session, which is what the paragraph above
    // always promised and the `?` here did not deliver.
    if pin_zid {
        if let Some(pin) = slot_anchor_from_cert(profile).as_deref().map(zid_pin_value) {
            if config.insert_json5("id", &format!("\"{pin}\"")).is_err() {
                // Cosmetic plumbing only — a session with a random ZID
                // publishes, is attributed, and is authorized identically.
                eprintln!("⚠ zenoh: could not pin the session id to this slot — using a random ZID");
            }
        }
    }

    // Bounded, retrying session open:
    //   - each attempt is capped at session_open_timeout() (default 5s,
    //     tunable via --timeout/GOAT_TIMEOUT) so an unreachable router fails
    //     fast + clean instead of blocking;
    //   - a MAX_LINKS close (ZID-pin collision with a not-yet-reaped prior
    //     session of the same anchor) is retried after a brief backoff, up to
    //     MAX_LINKS_RETRIES, so a rapid relaunch never surfaces it.
    // Non-MAX_LINKS errors fail immediately (no point retrying a TLS/cert
    // fault). zenoh::Config is cloned per attempt since open() consumes it.
    let open_timeout = session_open_timeout();
    let mut attempt: u32 = 0;
    loop {
        match timeout(open_timeout, zenoh::open(config.clone())).await {
            Ok(Ok(s)) => {
                // We're connected — opportunistically nudge if this goat is
                // behind the version the goatnet hands out. Best-effort,
                // fail-silent, at most once per process, no added latency on
                // the warm-cache path (crate::version_check).
                crate::version_check::nag_if_outdated(profile).await;
                return Ok(s);
            }
            Ok(Err(e)) => {
                if e.to_string().contains("MAX_LINKS") {
                    if attempt < MAX_LINKS_RETRIES {
                        attempt += 1;
                        tokio::time::sleep(MAX_LINKS_BACKOFF).await;
                        continue;
                    }
                    // Retries exhausted on a MAX_LINKS collision. The router's
                    // per-peer link cap is full for this identity: the ZID is
                    // pinned to the cert anchor (ADR 1039/1041), so every `goat`
                    // command under one slot identity shares ONE transport.
                    // Surface what the operator can act on, not the raw zenoh text
                    // (which reads like a router outage).
                    let waited_s =
                        (MAX_LINKS_RETRIES as u64) * (MAX_LINKS_BACKOFF.as_millis() as u64) / 1000;
                    return Err(anyhow::anyhow!(
                        "router link limit reached for this identity — links from prior \
                         `goat` commands are still reclaiming on the router (retried \
                         {MAX_LINKS_RETRIES}× over ~{waited_s}s). Wait a few seconds and \
                         retry, or avoid running many concurrent goat sessions under one \
                         slot identity. (per-peer cap is transport/unicast/max_links on the router.)"
                    ));
                }
                // Local identity files are already confirmed present+readable
                // by require_mesh_ready() above, so classify what's left: a
                // reachability problem or a handshake/ACL rejection — never
                // the raw multi-line zenoh/tokio error text.
                return Err(crate::classify::classify_zenoh_err(
                    e,
                    &router,
                    "open a fabric session",
                ));
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "zenoh open timed out in {}s — router unreachable? tune with --timeout/GOAT_TIMEOUT; `goat doctor` to diagnose",
                    open_timeout.as_secs()
                ));
            }
        }
    }
}

/// Extract this profile's slot anchor for ZID-pinning: the leaf cert's
/// Subject CommonName, but ONLY when it is anchor-shaped (32 lowercase hex
/// chars per `panoscope_core::alias::is_anchor_shaped`). Returns `None` for
/// a legacy/shared CN (`<vendor>-trial-peer`), a missing/unparseable cert,
/// or no CN — in every such case the caller leaves the ZID random, which is
/// the pre-4b behavior. Best-effort + fail-open by design: a cosmetic pin
/// must never block opening the session.
/// The profile cert's slot anchor, if it has one — shared by `goat access key`
/// (default anchor) and the ZID pin below.
pub(crate) fn own_anchor(profile: &Profile) -> Option<String> {
    slot_anchor_from_cert(profile)
}

fn slot_anchor_from_cert(profile: &Profile) -> Option<String> {
    let pem_bytes = std::fs::read(profile.cert_path()).ok()?;
    let blob = pem::parse(&pem_bytes).ok()?;
    if blob.tag() != "CERTIFICATE" {
        return None;
    }
    let (_, cert) = x509_parser::parse_x509_certificate(blob.contents()).ok()?;
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())?;
    if panoscope_core::alias::is_anchor_shaped(cn) {
        Some(cn.to_string())
    } else {
        None
    }
}

/// The zenoh-legal ZID string for a slot anchor: the anchor with any leading
/// zeros stripped.
///
/// A zenoh ZID is a 128-bit id whose string form is MINIMAL lowercase hex, so
/// `uhlc::ID::from_str` refuses a leading `0` outright ("Leading 0s are not
/// valid", `uhlc-0.8.2/src/id.rs:258`). Slot anchors are 32 hex chars of a
/// hash, so a share of them begin with `0` — those identities could not pin,
/// and before this normalization the failed pin killed the session, taking
/// every publish-path command (`goat pub`, `health`, `presence`, `audit`,
/// `doctor`, `mock`, `names`) down with it while the read-only commands, which
/// never pin, kept working.
///
/// Stripping is the same remedy the router roles use for the derived per-host
/// ZID. It preserves the 128-bit VALUE — `0345…` and `345…` are the
/// same u128 — so the pin stays deterministic and stable across restarts, and
/// it is a no-op for the majority of anchors that do not start with `0`. The
/// cost is display-only: for an affected slot the ZID no longer renders as the
/// anchor character-for-character. Nothing correlates the two by string —
/// panoscope's census maps a ZID to an anchor through the PUBLISHED KEY
/// (`Census::zid_anchors`), never by comparing a ZID to an anchor — and authz
/// rides the cert CN either way.
fn zid_pin_value(anchor: &str) -> &str {
    anchor.trim_start_matches('0')
}

/// Map a `--encoding` value to a Zenoh `Encoding`. Mirrors the
/// `format_to_encoding` vocabulary the synth publishers use so an operator
/// types the same shorthand the fleet declares: `json` / `cbor` / `text` /
/// `protobuf` / `protobuf:<MessageName>`. The protobuf message name rides
/// in the encoding's schema field. Anything else is taken as a media-type
/// string verbatim.
fn goat_encoding(s: &str) -> zenoh::bytes::Encoding {
    use zenoh::bytes::Encoding;
    match s {
        "json" => Encoding::APPLICATION_JSON,
        "cbor" => Encoding::APPLICATION_CBOR,
        "text" => Encoding::TEXT_PLAIN,
        "protobuf" => Encoding::APPLICATION_PROTOBUF,
        other => match other.strip_prefix("protobuf:") {
            // `protobuf:<Name>` carries the message name; a bare `protobuf:`
            // (empty name) is just plain protobuf.
            Some(msg) if !msg.is_empty() => Encoding::APPLICATION_PROTOBUF.with_schema(msg),
            Some(_) => Encoding::APPLICATION_PROTOBUF,
            None => Encoding::from(other),
        },
    }
}

/// Resolve the publish payload bytes, in precedence order: inline
/// `--payload` text, then `--payload-file`, then stdin. Clap already
/// rejects `--payload` + `--payload-file` together (conflicts_with), so
/// at most one of the first two is set. The stdin fallback lets
/// operators script `echo 'hello' | goat pub <key>` without temp files.
async fn read_payload(
    inline: Option<&str>,
    path: Option<&std::path::Path>,
) -> anyhow::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    if let Some(text) = inline {
        Ok(text.as_bytes().to_vec())
    } else if let Some(p) = path {
        Ok(tokio::fs::read(p).await?)
    } else {
        let mut buf = Vec::new();
        tokio::io::stdin().read_to_end(&mut buf).await?;
        Ok(buf)
    }
}


/// Verdict of a `--confirm` round-trip read-back. Shared with
/// `goat doctor --topic`, which reuses the same write-then-read-back
/// probe to answer "can I write here?" before publishing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConfirmOutcome {
    /// The just-published value came back from storage — write accepted + retained.
    Landed,
    /// A value exists for the key but it isn't the one we just published.
    /// Ambiguous (stale prior value masking a denial, or a concurrent publisher).
    ValueMismatch,
    /// Nothing came back for the key — the denial / no-storage signal.
    NotFound,
    /// The read-back itself could not run (query error / timeout); the publish
    /// result is unknown, not failed.
    Unverifiable(String),
}

/// Query the router's storage for `key` and decide whether the value we just
/// published (`expect`) actually landed. Mirrors panoscope's proven
/// concrete-key read-back shape: `QueryTarget::All` (storage queryables are
/// not flagged "complete", so `AllComplete` returns 0) + no consolidation.
pub(crate) async fn confirm_landed(
    session: &zenoh::Session,
    key: &str,
    expect: &[u8],
) -> ConfirmOutcome {
    use zenoh::query::{ConsolidationMode, QueryTarget};

    let replies = match session
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(CONFIRM_QUERY_TIMEOUT)
        .await
    {
        Ok(r) => r,
        Err(e) => return ConfirmOutcome::Unverifiable(format!("storage query failed: {e}")),
    };

    let mut got: Vec<Vec<u8>> = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        // Error replies (a queryable answering with an error) don't count as a
        // retained value — skip them, same as panoscope's read path.
        let Ok(sample) = reply.result() else { continue };
        got.push(sample.payload().to_bytes().to_vec());
    }
    classify_confirm(expect, &got)
}

/// Pure classifier for the read-back result, split out so the landed /
/// mismatch / not-found logic is unit-testable without a live router.
fn classify_confirm(expect: &[u8], got: &[Vec<u8>]) -> ConfirmOutcome {
    if got.iter().any(|g| g.as_slice() == expect) {
        ConfirmOutcome::Landed
    } else if got.is_empty() {
        ConfirmOutcome::NotFound
    } else {
        ConfirmOutcome::ValueMismatch
    }
}

/// Render one received sample to stdout. Default form: `<key> <body>`
/// where body is shown as UTF-8 if printable, hex otherwise. Operators
/// can pipe to jq when the publisher uses JSON; Phase D.B's --decode
/// covers the protobuf case.
fn print_sample(key: &str, body: &[u8]) {
    match std::str::from_utf8(body) {
        Ok(s) if is_printable(s) => println!("{key} {s}"),
        _ => println!("{key} hex:{}", hex(body)),
    }
}

fn is_printable(s: &str) -> bool {
    s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t')
}

fn hex(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 2);
    for byte in b {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    // The router endpoint is read through crate::profile::resolve_router_endpoint
    // in the session open; the tests below still exercise the shared reader.
    use crate::profile::extract_toml_field as extract_toml_string;

    #[test]
    fn extract_toml_finds_router_endpoint() {
        let body = "router_endpoint = \"tls/host:7447\"\nportal_endpoint = \"https://p\"\n";
        assert_eq!(
            extract_toml_string(body, "router_endpoint"),
            Some("tls/host:7447".into())
        );
    }

    #[test]
    fn print_sample_prefers_utf8_when_printable() {
        // Smoke check that is_printable considers normal text printable
        assert!(is_printable("hello world"));
        assert!(is_printable("line1\nline2"));
        assert!(!is_printable("\x01\x02\x03"));
    }

    /// The default flip. `goat sub <key>` with no `--count`/`--take` streams;
    /// the old exit-after-one behaviour is now something you ask for.
    #[test]
    fn sub_streams_unless_a_count_is_asked_for() {
        assert_eq!(sub_limit(None), usize::MAX);
        assert_eq!(sub_limit(Some(1)), 1);
        assert_eq!(sub_limit(Some(200)), 200);
    }

    #[test]
    fn listening_line_names_the_router_and_the_mode() {
        let stream = listening_line("acme/**", "tls/router.example:7447", usize::MAX);
        assert!(stream.contains("listening on `acme/**`"), "{stream}");
        assert!(stream.contains("tls/router.example:7447"), "{stream}");
        assert!(stream.contains("streaming live events"), "{stream}");
        // A bounded run says what will end it, so silence after N is not read
        // as a crash.
        let bounded = listening_line("acme/**", "tls/router.example:7447", 2);
        assert!(bounded.contains("stopping after 2 event(s)"), "{bounded}");
        assert!(!bounded.contains("∞"), "{bounded}");
    }

    /// The false negative this whole path exists to kill: a healthy, attached
    /// subscription on a quiet key must SAY it is healthy and attached, name
    /// the router as checkable evidence, and hand over the next commands.
    #[test]
    fn quiet_report_says_attached_and_names_the_router() {
        let lines = quiet_report(
            "acme/**",
            "goat sub 'acme/**'",
            &["a1b2c3d4e5f60718".to_string()],
            std::time::Duration::from_secs(5),
        );
        let all = lines.join("\n");
        assert!(all.contains("Attached to router a1b2c3d4e5f60718"), "{all}");
        assert!(all.contains("the subscription is live"), "{all}");
        assert!(all.contains("nothing is publishing on `acme/**`"), "{all}");
        assert!(all.contains("--since 1h"), "{all}");
        assert!(all.contains("goat access why me"), "{all}");
        // It must never claim a denial it cannot see (F-177: a denied read is
        // silent, so "quiet" and "denied" look identical from here).
        assert!(!all.to_lowercase().contains("denied"), "{all}");
    }

    #[test]
    fn stop_summary_states_zero_events_as_a_fact_not_a_failure() {
        let none =
            stop_summary("acme/**", 0, usize::MAX, std::time::Duration::from_secs(90), true, true);
        assert!(none.contains("stopped after 1m30s"), "{none}");
        assert!(none.contains("0 event(s) on `acme/**`"), "{none}");
        assert!(none.contains("attached to a router when last checked"), "{none}");

        let satisfied =
            stop_summary("acme/**", 2, 2, std::time::Duration::from_secs(3), false, true);
        assert!(satisfied.starts_with("done after 3s"), "{satisfied}");
        assert!(satisfied.contains("(the requested 2)"), "{satisfied}");
    }

    /// A run that ended before the checkpoint — `--for 3s`, or an immediate
    /// Ctrl-C — has never asked whether the session was attached. Claiming it
    /// was is the same unearned confidence the quiet report exists to avoid.
    #[test]
    fn stop_summary_never_claims_an_attachment_it_did_not_check() {
        let unchecked =
            stop_summary("acme/**", 0, 2, std::time::Duration::from_secs(3), false, false);
        assert!(unchecked.contains("UNKNOWN"), "{unchecked}");
        assert!(!unchecked.contains("attached to a router"), "{unchecked}");
        // ... and it says how to find out.
        assert!(unchecked.contains("--for"), "{unchecked}");
    }

    /// `--count 0` and `--for 0` are the same request spelled two ways: a tail
    /// that stops before it starts. `--for` already refused it; the count must
    /// too, or the loop exits "satisfied" having declared nothing, checked no
    /// router and read no sample.
    #[test]
    fn a_zero_bound_is_refused_whichever_way_it_is_spelled() {
        assert!(parse_for_duration("0s").is_err());
        // `sub_limit(Some(0))` is only reachable if the guard in `run_sub`
        // lets it through, so pin the guard's condition itself.
        let zero = SubArgs {
            key_pattern: "acme/**".into(),
            follow: false,
            decode: false,
            tui: false,
            count: Some(0),
            keys_only: false,
            for_: None,
            since: None,
            open: false,
            key_service_bundle: None,
            trust_domain: None,
            key_service_id: None,
            literal: false,
        };
        assert_eq!(zero.count, Some(0));
        assert_eq!(sub_limit(zero.count), 0, "a zero bound would satisfy instantly");
    }

    /// The hint has to carry the flags that make it reproducible: `--literal`
    /// (the trap it is explaining) and `--open` (a different read path).
    #[test]
    fn self_cmd_carries_the_flags_that_change_what_is_read() {
        let mut args = SubArgs {
            key_pattern: " ops/mesh/health/** ".into(),
            follow: false,
            decode: false,
            tui: false,
            count: None,
            keys_only: false,
            for_: None,
            since: None,
            open: false,
            key_service_bundle: None,
            trust_domain: None,
            key_service_id: None,
            literal: true,
        };
        assert_eq!(self_cmd_of(&args), "goat sub --literal 'ops/mesh/health/**'");
        args.literal = false;
        args.open = true;
        assert_eq!(self_cmd_of(&args), "goat sub --open 'ops/mesh/health/**'");
    }

    #[test]
    fn quiet_report_replays_the_operators_own_invocation() {
        // --literal has to survive into the hint: on a slot-pool goatnet the
        // resolved pattern alone gives "no slot aliased `ops`".
        let lines = quiet_report(
            "ops/mesh/health/**",
            "goat sub --literal 'ops/mesh/health/**'",
            &["b2ec0bfb5f92ed36".to_string()],
            std::time::Duration::from_secs(5),
        );
        let all = lines.join("\n");
        assert!(all.contains("goat sub --literal 'ops/mesh/health/**' --since 1h"), "{all}");
    }

    #[test]
    fn literal_alias_note_names_the_alias_and_the_anchor_it_resolves_to() {
        let note = literal_alias_note(
            "demo-org-500/**",
            "0123456789abcdef0123456789abcdef/**",
        );
        assert!(note.contains("`demo-org-500` is a live slot alias"), "{note}");
        assert!(note.contains("0123456789abcdef0123456789abcdef/**"), "{note}");
        assert!(note.contains("Drop --literal"), "{note}");
    }

    #[test]
    fn parse_for_duration_accepts_units_and_combinations() {
        use std::time::Duration;
        assert_eq!(parse_for_duration("45s").unwrap(), Duration::from_secs(45));
        assert_eq!(parse_for_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_for_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_for_duration("1h30m").unwrap(), Duration::from_secs(5400));
        // A bare number is seconds, matching --timeout.
        assert_eq!(parse_for_duration(" 90 ").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn parse_for_duration_refuses_rather_than_guesses() {
        // Each of these could be "rounded" into something plausible; a wrong
        // guess silently changes how long an acceptance check waits.
        for bad in ["", "soon", "5x", "1h30", "0", "0s", "m5"] {
            assert!(parse_for_duration(bad).is_err(), "accepted `{bad}`");
        }
    }

    #[test]
    fn want_label_renders_an_unbounded_tail_as_infinity() {
        assert_eq!(want_label(usize::MAX), "∞");
        assert_eq!(want_label(7), "7");
    }

    #[test]
    fn human_duration_scales_from_seconds_to_hours() {
        use std::time::Duration;
        assert_eq!(human_duration(Duration::from_secs(5)), "5s");
        assert_eq!(human_duration(Duration::from_secs(59)), "59s");
        assert_eq!(human_duration(Duration::from_secs(60)), "1m00s");
        assert_eq!(human_duration(Duration::from_secs(3599)), "59m59s");
        assert_eq!(human_duration(Duration::from_secs(3600)), "1h00m");
        assert_eq!(human_duration(Duration::from_secs(7860)), "2h11m");
    }

    #[test]
    fn hex_pads_to_two_chars() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn goat_encoding_maps_shorthands_and_protobuf_schema() {
        assert_eq!(goat_encoding("json").to_string(), "application/json");
        assert_eq!(goat_encoding("cbor").to_string(), "application/cbor");
        assert_eq!(goat_encoding("text").to_string(), "text/plain");
        assert_eq!(goat_encoding("protobuf").to_string(), "application/protobuf");
        // the protobuf message name rides in the schema suffix
        assert_eq!(
            goat_encoding("protobuf:TrackUpdate").to_string(),
            "application/protobuf;TrackUpdate"
        );
        // a bare `protobuf:` with no name is treated as plain protobuf
        assert_eq!(
            goat_encoding("protobuf:").to_string(),
            "application/protobuf"
        );
        // an arbitrary media type passes through verbatim
        assert_eq!(goat_encoding("application/xml").to_string(), "application/xml");
    }

    #[test]
    fn confirm_landed_when_published_value_returns() {
        let expect = b"hello".to_vec();
        let got = vec![b"hello".to_vec()];
        assert_eq!(classify_confirm(&expect, &got), ConfirmOutcome::Landed);
    }

    #[test]
    fn confirm_landed_when_value_present_among_several_replies() {
        let expect = b"v2".to_vec();
        let got = vec![b"v1".to_vec(), b"v2".to_vec()];
        assert_eq!(classify_confirm(&expect, &got), ConfirmOutcome::Landed);
    }

    #[test]
    fn confirm_not_found_on_empty_replies() {
        // The denial / no-storage signal: nothing came back at all.
        let expect = b"hello".to_vec();
        assert_eq!(classify_confirm(&expect, &[]), ConfirmOutcome::NotFound);
    }

    #[test]
    fn confirm_value_mismatch_when_only_a_stale_value_returns() {
        // A prior retained value exists but it isn't ours — ambiguous.
        let expect = b"new".to_vec();
        let got = vec![b"old".to_vec()];
        assert_eq!(classify_confirm(&expect, &got), ConfirmOutcome::ValueMismatch);
    }

    #[test]
    fn confirm_matches_exact_bytes_not_prefix() {
        let expect = b"abc".to_vec();
        let got = vec![b"abcd".to_vec()];
        assert_eq!(classify_confirm(&expect, &got), ConfirmOutcome::ValueMismatch);
    }

    #[tokio::test]
    async fn read_payload_inline_returns_text_bytes() {
        // Inline --payload text is taken verbatim, no file/stdin touched.
        let got = read_payload(Some("{\"ping\":1}"), None).await.unwrap();
        assert_eq!(got, b"{\"ping\":1}");
    }

    #[tokio::test]
    async fn read_payload_inline_wins_over_file() {
        // Defense-in-depth: even if both reach read_payload (clap normally
        // rejects the combo), inline takes precedence and the file path is
        // never read — so a bogus path here must NOT error.
        let got = read_payload(Some("inline"), Some(std::path::Path::new("/no/such/file")))
            .await
            .unwrap();
        assert_eq!(got, b"inline");
    }

    #[tokio::test]
    async fn read_payload_reads_file_when_no_inline() {
        let dir = std::env::temp_dir();
        let p = dir.join("goat_pub_payload_test.bin");
        tokio::fs::write(&p, b"from-file").await.unwrap();
        let got = read_payload(None, Some(&p)).await.unwrap();
        assert_eq!(got, b"from-file");
        let _ = tokio::fs::remove_file(&p).await;
    }

    /// F-350: zenoh rejects a leading-zero ZID, so the pin must be the
    /// normalized form. This asserts against the real `zenoh::Config`, not our
    /// idea of the rule — the anchor that broke a live publisher and one whose
    /// every leading nibble is zero both have to be accepted after stripping.
    #[test]
    fn zid_pin_is_accepted_by_zenoh_for_leading_zero_anchors() {
        for anchor in [
            // The field-reported anchor (goat pub died at config parse).
            "03454ed1768a5b2eae91d3e1207f42f1",
            // Two more real-shaped anchors, and a many-zeros case.
            "012ed848757186451dc02f1a4d6c2da8",
            "000d6447b140fe8bf6efd54ca7d753ae",
            // Unaffected anchor: must still pin, unchanged.
            "7b9b61a4768a5b2eae91d3e1207f42f1",
        ] {
            let mut config = zenoh::Config::default();
            let pin = zid_pin_value(anchor);
            assert!(
                config.insert_json5("id", &format!("\"{pin}\"")).is_ok(),
                "zenoh refused the pin {pin} derived from anchor {anchor}"
            );
        }
    }

    #[test]
    fn zid_pin_value_strips_only_leading_zeros() {
        // No leading zero: byte-for-byte the anchor, so the ZID still renders
        // as the slot anchor for the overwhelming majority of identities.
        let plain = "7b9b61a4768a5b2eae91d3e1207f42f1";
        assert_eq!(zid_pin_value(plain), plain);
        // Leading zeros go; interior and trailing zeros stay (same u128).
        assert_eq!(
            zid_pin_value("03454ed1768a5b2eae91d3e1207f42f0"),
            "3454ed1768a5b2eae91d3e1207f42f0"
        );
        assert_eq!(
            zid_pin_value("000d6447b140fe8bf6efd54ca7d753ae"),
            "d6447b140fe8bf6efd54ca7d753ae"
        );
    }

    /// The unnormalized anchor is what zenoh rejects — kept as the regression's
    /// negative half, so a future "simplification" back to the raw anchor fails
    /// here rather than in someone's field session.
    #[test]
    fn raw_leading_zero_anchor_is_still_rejected_by_zenoh() {
        let mut config = zenoh::Config::default();
        let err = config
            .insert_json5("id", "\"03454ed1768a5b2eae91d3e1207f42f1\"")
            .expect_err("zenoh 1.9 must still reject a leading-zero id");
        assert!(
            err.to_string().contains("Leading 0s are not valid"),
            "unexpected rejection reason: {err}"
        );
    }
}

#[cfg(test)]
mod sub_pattern_tests {
    use super::sub_pattern;
    use panoscope_core::alias::AliasRegistry;
    use panoscope_core::portal::AliasRow;

    fn reg_with_alias() -> AliasRegistry {
        AliasRegistry::from_rows([AliasRow {
            alias: "tarsier".to_string(),
            slot_id: "0123456789abcdef0123456789abcdef".to_string(),
        }])
    }

    #[test]
    fn literal_skips_alias_resolution_on_a_slot_pool_goatnet() {
        // A seeded per-vendor prefix beside a slot pool: the resolver refuses
        // it as an unknown alias, --literal subscribes on it as written.
        let reg = reg_with_alias();
        assert!(sub_pattern(&reg, "airdefenders/zagros/**", false).is_err());
        assert_eq!(
            sub_pattern(&reg, " airdefenders/zagros/** ", true).unwrap(),
            "airdefenders/zagros/**"
        );
    }

    #[test]
    fn default_still_resolves_aliases() {
        let reg = reg_with_alias();
        assert_eq!(
            sub_pattern(&reg, "tarsier/obs/**", false).unwrap(),
            "0123456789abcdef0123456789abcdef/obs/**"
        );
    }
}

// `goat doctor` — local readiness and four fabric probes.
//
// Each step has a unique verdict (Ok / Warn / Skip / Fail) and a stable
// machine-readable name. Pretty mode prints `[✓|!|⏭|✗] step-N name —
// summary` lines; --json mode emits one NDJSON line per step on
// stdout so a wrapper script can parse without regex-shaping the
// pretty form.
//
// Step 1 — TCP-connect to the configured router host. This proves only
//   that the router port is reachable. Lightweight; <1.5s timeout.
// Step 2 — local TLS material: parse the certificate and CA bundle, match
//   the private key, and check the leaf validity interval. Step 3 handshakes.
// Step 3 — full Zenoh mTLS session: open a Zenoh session against the
//   router with the profile's mTLS material. The session open
//   completes the handshake and confirms the router opens a session
//   for this transport identity. Topic authorization is separate.
//   Closes immediately.
// Step 4 — portal /api/orgs: GET against the portal with the
//   profile's Bearer token. Confirms portal reachability + token
//   acceptance for the probed API.
//
// Outer-frame ZTOL precheck: every subcommand except
// init/version/completion/compose/explain caches a Step-1 verdict
// at ~/.goat/contexts/<name>/last-doctor.json with 60s TTL. Hit →
// skip. Miss/stale → run step 1 only. Phase B wires the cache file;
// per-subcommand consumption lands in Phase C as those subcommands
// gain real behavior.

use crate::bundle::GoatCliExtras;
use crate::errors::GoatError;
use crate::i18n::tr2;
use crate::profile::Profile;
use clap::Args;
use panoscope_core::portal::{portal_timeout, PortalClient};
use serde::Serialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::time::timeout;

const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
// Step-3 session-open budget lives in `crate::probe::ZENOH_OPEN_TIMEOUT`,
// shared with the `goat status` session line so both agree on "slow".
// Step-4 wraps the portal client in its own outer timeout. It MUST match the
// budget the client itself uses (`portal_timeout()`), or the two disagree and a
// slow-but-healthy portal on a lossy mesh gets reported as a false
// E_PORTAL_UNREACHABLE. Resolved at call time so `GOAT_PORTAL_TIMEOUT_SECS`
// is honored here too. See GH #143.

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Emit one JSON line per step instead of pretty output.
    #[arg(long)]
    pub json: bool,

    /// Run network probes despite earlier failures when their inputs exist.
    #[arg(long)]
    pub all: bool,

    /// Write-precheck a specific topic key BEFORE publishing: writes a
    /// unique sentinel to a scratch sub-key under <KEY> and reads it
    /// back. A round-trip means you can write here; a silent miss is
    /// exactly what an ACL ingress-denial looks like to a publisher
    /// Uses only your own publish credential — no COP/admin
    /// access — so a denied org can self-diagnose. Replaces the
    /// local-readiness and fabric sweep; runs the probe and exits.
    #[arg(long, value_name = "KEY")]
    pub topic: Option<String>,
}

/// A single step's outcome on the wire (--json) and on the page.
#[derive(Debug, Clone, Serialize)]
pub struct StepOutcome {
    pub step: u8,
    pub name: &'static str,
    pub verdict: Verdict,
    pub elapsed_ms: u128,
    /// One-line summary — canonical English, serialized into `--json`.
    pub summary: String,
    /// Non-empty on Fail; empty on Ok/Warn/Skip. Canonical English, serialized.
    pub hint: String,
    /// Stable reason for a failure. New diagnoses use this instead of
    /// matching translated or canonical prose to choose an exit code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<&'static str>,
    /// Individual local checks. Present only on step 0; older step records
    /// retain their established NDJSON shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<DiagnosticCheck>>,
    /// Active-locale render of `summary` for the pretty path; not
    /// serialized (the `--json` surface stays English by rule).
    #[serde(skip)]
    pub summary_loc: String,
    /// Active-locale render of `hint` for the pretty path; not serialized.
    #[serde(skip)]
    pub hint_loc: String,
    /// Name-plane annotation (ADR 1082): set when this step's target was
    /// resolved from the signed name snapshot because mesh DNS did not
    /// answer — "via snapshot serial N, age M". None (and absent from
    /// --json) on the live-DNS path, so the pre-existing JSON contract is
    /// byte-identical when no fallback is in play.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<String>,
    #[serde(skip)]
    pub names_loc: Option<String>,
}

impl StepOutcome {
    fn with_cause(mut self, cause: &'static str) -> Self {
        self.cause = Some(cause);
        self
    }

    /// Attach the fallback annotation to a step outcome — canonical
    /// (snapshot) and noncanonical (observed) get distinct labels.
    fn with_names_note(mut self, note: &crate::names::FallbackNote) -> Self {
        let (en, loc) = match note {
            crate::names::FallbackNote::Snapshot(meta) => tr2!(
                "doctor-names-note",
                serial = meta.serial.to_string(),
                age = meta.age_human()
            ),
            crate::names::FallbackNote::SelfClaim { age_secs } => tr2!(
                "doctor-names-selfclaim-note",
                age = crate::names::age_human(*age_secs)
            ),
            crate::names::FallbackNote::Observed { age_secs } => tr2!(
                "doctor-names-observed-note",
                age = crate::names::age_human(*age_secs)
            ),
        };
        self.names = Some(en);
        self.names_loc = Some(loc);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Skip,
    Fail,
}

/// A local observation with a stable identifier and an actionable remedy.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticCheck {
    pub id: &'static str,
    pub verdict: Verdict,
    pub summary: String,
    pub hint: String,
    #[serde(skip)]
    pub summary_loc: String,
    #[serde(skip)]
    pub hint_loc: String,
}

impl DiagnosticCheck {
    pub fn new(
        id: &'static str,
        verdict: Verdict,
        summary: (String, String),
        hint: (String, String),
    ) -> Self {
        Self {
            id,
            verdict,
            summary: summary.0,
            summary_loc: summary.1,
            hint: hint.0,
            hint_loc: hint.1,
        }
    }
}

pub async fn run(ctx: &super::Ctx, args: DoctorArgs) -> anyhow::Result<()> {
    let profile = &ctx.profile;

    // `--topic <key>` is its own mode: the active write-precheck. It
    // reuses the operator's own publish credential (no COP/admin), so
    // it answers "can I write here?" before publishing — the operator-
    // side escape from F-177's silent denial. Branch before the full
    // reachability sweep; the session open inside the probe surfaces a
    // reachability failure on its own.
    if let Some(key) = args.topic.as_deref() {
        profile.require_initialized()?;
        return run_topic_precheck(ctx, key, args.json).await;
    }

    // The local stage deliberately runs before any profile read. A fresh
    // machine must get a diagnostic record, including with --json.
    let local_start = Instant::now();
    let checks = super::doctor_preflight::run(profile).await;
    let local_verdict = if checks.iter().any(|c| c.verdict == Verdict::Fail) {
        Verdict::Fail
    } else if checks.iter().any(|c| c.verdict == Verdict::Warn) {
        Verdict::Warn
    } else {
        Verdict::Ok
    };
    let (summary, summary_loc) = match local_verdict {
        Verdict::Fail => tr2!("doctor-step0-fail-summary"),
        Verdict::Warn => tr2!("doctor-step0-warn-summary"),
        _ => tr2!("doctor-step0-ok-summary"),
    };
    let first_fail_hint = checks.iter().find(|c| c.verdict == Verdict::Fail);
    let step0 = StepOutcome {
        step: 0,
        name: "local-readiness",
        verdict: local_verdict,
        elapsed_ms: local_start.elapsed().as_millis(),
        summary,
        summary_loc,
        hint: first_fail_hint.map(|c| c.hint.clone()).unwrap_or_default(),
        hint_loc: first_fail_hint
            .map(|c| c.hint_loc.clone())
            .unwrap_or_default(),
        cause: None,
        checks: Some(checks),
        names: None,
        names_loc: None,
    };
    emit(&step0, args.json);

    // No endpoints exist without a readable profile. With --all, continue
    // past missing material and run probes whose inputs still exist.
    if profile.require_initialized().is_err()
        || (!args.all
            && step0.checks.as_ref().is_some_and(|cs| {
                cs.iter()
                    .any(|c| c.id.starts_with("profile.") && c.verdict == Verdict::Fail)
            }))
    {
        for step in 1..=4 {
            emit(&skipped_no_profile(step), args.json);
        }
        return Err(map_preflight_failure(&step0, profile).into());
    }

    // `read_classified` (not a bare `?`) so a missing/unreadable profile.toml
    // or portal.token — the one command whose entire job is diagnosing a
    // broken setup — gets the same classified, path-naming GoatError every
    // other command gets, instead of a bare `No such file or directory`.
    let toml = match profile.read_classified(&profile.profile_toml_path()) {
        Ok(toml) => Some(toml),
        Err(_) if args.all => None,
        Err(e) => return Err(e.into()),
    };
    let endpoints = toml.as_deref().map(parse_endpoints).unwrap_or_default();
    let portal_token = match profile.read_classified(&profile.portal_token_path()) {
        Ok(token) if !token.trim().is_empty() => Some(token.trim().to_string()),
        Ok(_) if args.all => None,
        Err(_) if args.all => None,
        Ok(_) => return Err(map_preflight_failure(&step0, profile).into()),
        Err(e) => return Err(e.into()),
    };

    let extras = GoatCliExtras {
        portal_endpoint: endpoints.portal_endpoint.clone().unwrap_or_default(),
        router_endpoint: endpoints.router_endpoint.clone().unwrap_or_default(),
        onboard_endpoint: None, // doctor probes portal/router; onboard not needed
        portal_token: portal_token
            .as_deref()
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
        mtls_cert: vec![], // not needed for doctor — paths used instead
        mtls_key: vec![],
        ca_roots: vec![],
    };

    let cert_path = profile.cert_path();
    let key_path = profile.key_path();
    let ca_roots_path = profile.ca_roots_path();

    // Name-resolution pass (ADR 1082): live DNS first, signed-snapshot
    // fallback second, per probe target. `Some` ONLY when the snapshot
    // answered after live DNS failed — the live path leaves every probe's
    // behavior byte-identical. A both-legs failure stays None; the probe
    // then fails by hostname with its existing message.
    let router_hp = endpoints
        .router_endpoint
        .as_deref()
        .and_then(parse_zenoh_locator);
    let portal_hp = endpoints
        .portal_endpoint
        .as_deref()
        .and_then(crate::cmd::status::parse_https_host_port);
    let (router_fb, portal_fb) = tokio::join!(
        resolve_fallback(profile, router_hp.as_ref()),
        resolve_fallback(profile, portal_hp.as_ref()),
    );

    let mut outcomes: Vec<StepOutcome> = Vec::with_capacity(5);
    outcomes.push(step0);
    let mut failed_early = local_verdict == Verdict::Fail;
    for step in 1u8..=4 {
        // Local certificate validation is independent of mesh reachability.
        if failed_early && !args.all && step != 2 {
            let outcome = skipped(step);
            emit(&outcome, args.json);
            outcomes.push(outcome);
            continue;
        }
        let outcome = match step {
            1 if endpoints.router_endpoint.is_none() => skipped_no_endpoint(1),
            1 => {
                let outcome = probe_ztol(&extras, router_fb.as_ref()).await;
                if outcome.verdict == Verdict::Fail {
                    refine_off_mesh(outcome).await
                } else {
                    outcome
                }
            }
            2 => validate_tls_material(&cert_path, &key_path, &ca_roots_path),
            3 if endpoints.router_endpoint.is_none() => skipped_no_endpoint(3),
            3 => {
                probe_zenoh_session(
                    &extras,
                    &cert_path,
                    &key_path,
                    &ca_roots_path,
                    router_fb.as_ref(),
                )
                .await
            }
            4 if endpoints.portal_endpoint.is_none() => skipped_no_endpoint(4),
            4 if portal_token.is_none() => skipped_no_portal_token(),
            4 => probe_portal(&extras, portal_fb.as_ref()).await,
            _ => unreachable!(),
        };
        if outcome.verdict == Verdict::Fail {
            failed_early = true;
        }
        emit(&outcome, args.json);
        outcomes.push(outcome);
    }

    // Cache the ZTOL Step-1 verdict for the outer-frame precheck.
    if let Some(step1) = outcomes.iter().find(|o| o.step == 1) {
        let _ = write_doctor_cache(profile, step1);
    }

    if outcomes.iter().any(|o| o.verdict == Verdict::Fail) {
        // Exit code comes from the FIRST failing step's verdict, mapped
        // through GoatError. Local findings retain their specific code;
        // a router TCP failure does not establish that enrollment failed.
        let first_fail = outcomes
            .iter()
            .find(|o| o.verdict == Verdict::Fail)
            .expect("we just checked");
        return Err(if first_fail.step == 0 {
            map_preflight_failure(first_fail, profile)
        } else {
            map_doctor_failure(first_fail, profile)
        }
        .into());
    }

    // Snapshot-fallback hint (pretty surface): when any probe ran on
    // fallback names, say so once with the remediation — silence is never
    // an option when serving stale names (ADR 1082 honesty rule).
    if !args.json {
        if let Some((_, note)) = router_fb.as_ref().or(portal_fb.as_ref()) {
            let (_, hint_loc) = match note {
                crate::names::FallbackNote::Snapshot(meta) => tr2!(
                    "doctor-names-hint",
                    serial = meta.serial.to_string(),
                    age = meta.age_human()
                ),
                crate::names::FallbackNote::SelfClaim { age_secs } => tr2!(
                    "doctor-names-selfclaim-hint",
                    age = crate::names::age_human(*age_secs)
                ),
                crate::names::FallbackNote::Observed { age_secs } => tr2!(
                    "doctor-names-observed-hint",
                    age = crate::names::age_human(*age_secs)
                ),
            };
            println!("[!] names: {hint_loc}");
        }
    }

    // Opportunistic name-snapshot cache refresh (ADR 1082): only at a
    // connectivity moment — no fallback in play and the mesh reachable.
    if router_fb.is_none() && portal_fb.is_none() {
        if let Some(step1) = outcomes.iter().find(|o| o.step == 1) {
            if step1.verdict == Verdict::Ok {
                crate::names::refresh(profile).await;
            }
        }
    }

    // Version-staleness line: doctor is the explicit "is my setup healthy"
    // command, so show where this goat sits vs the version the goatnet hands
    // out — current or behind. Pretty surface only (never on --json), and
    // best-effort: if the download surface is unreachable we just omit the
    // line, since the diagnostic steps above already cover reachability.
    if !args.json {
        if let Some(st) = crate::version_check::doctor_status(profile).await {
            if st.outdated {
                println!(
                    "[!] goat {} is out of date — {} is available: {}",
                    st.current, st.latest, st.install_url
                );
            } else {
                println!("[✓] goat {} (latest)", st.current);
            }
        }
    }

    // All-green flourish: pretty mode + every step passed, suppressed by
    // GOAT_NO_FUN / non-TTY (the Three Rules — never on the --json surface).
    if !args.json {
        if let Some(line) = crate::fun::all_green_flourish() {
            println!("{line}");
        }
    }
    Ok(())
}

/// The structured result of a `--topic` write-precheck. One JSON object
/// (not the NDJSON step stream) when `--json`; rendered as a single
/// glyph line otherwise. English-only on the wire, same rule as
/// `StepOutcome`.
#[derive(Debug, Clone, Serialize)]
pub struct PrecheckReport {
    /// Stable probe name so a wrapper can tell this apart from the
    /// 5-step NDJSON stream.
    pub probe: &'static str,
    /// The topic key the operator intends to publish to.
    pub topic: String,
    /// The scratch sub-key the sentinel was actually written to (under
    /// `topic`, so it shares `topic`'s write-ACL without polluting the
    /// real key).
    pub scratch_key: String,
    pub verdict: Verdict,
    pub elapsed_ms: u128,
    pub summary: String,
    pub hint: String,
}

/// `goat doctor --topic <key>` — the active write-precheck. Opens a
/// Zenoh session on the operator's own mTLS material (the same path
/// `goat pub` uses), publishes a unique sentinel to a scratch sub-key
/// under `key`, and reads it back from router storage. A round-trip
/// proves the write-ACL admits this prefix; a silent miss is precisely
/// what F-177 ingress-denial looks like to a publisher — `put` returns
/// locally but the router accepted nothing.
async fn run_topic_precheck(ctx: &super::Ctx, key: &str, json: bool) -> anyhow::Result<()> {
    let key_input = key.trim();
    if key_input.is_empty() {
        anyhow::bail!("--topic requires a non-empty topic key");
    }
    // Resolve an alias in the leading segment to the canonical UUID anchor
    // (ADR 1039) BEFORE prechecking, so `goat doctor --topic jane-operator/x`
    // tests the real namespace. An unknown alias fails closed here. Everything
    // downstream (scratch key, wire ops, --json) uses the canonical `key`.
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let key = reg
        .resolve_key(key_input)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let key = key.as_str();
    let start = Instant::now();
    let nonce = precheck_nonce();
    let scratch_key = scratch_key_for(key, &nonce);
    // Unique payload so a stale identical value at the scratch key can
    // never mask a denial (same lesson as `pub --confirm`). The scratch
    // key is itself nonce'd, so this is belt-and-suspenders.
    let payload = format!("goat-precheck {nonce}").into_bytes();

    let session = super::pubsub::open_session_for(&ctx.profile).await?;
    let publisher = session
        .declare_publisher(scratch_key.clone())
        .await
        .map_err(|e| anyhow::anyhow!("declare publisher on `{scratch_key}`: {e}"))?;
    if let Err(e) = publisher.put(payload.clone()).await {
        let _ = session.close().await;
        anyhow::bail!("sentinel put on `{scratch_key}` failed: {e}");
    }
    let outcome = super::pubsub::confirm_landed(&session, &scratch_key, &payload).await;
    let _ = session.close().await;

    let (verdict, summary, hint) = classify_precheck(key, &outcome);
    let report = PrecheckReport {
        probe: "write-precheck",
        topic: key.to_string(),
        scratch_key,
        verdict,
        elapsed_ms: start.elapsed().as_millis(),
        summary,
        hint,
    };

    if json {
        // --json stays canonical (UUID) per ADR 1039.
        let line = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
        println!("{line}");
    } else {
        // Pretty header shows the human alias (unless --uuid); summary/hint
        // keep the canonical key.
        let shown = crate::alias::display_key(&reg, ctx.uuid, key);
        print_precheck_pretty(&report, &shown);
    }

    if report.verdict == Verdict::Fail {
        // The detailed verdict + hint already printed above; bail for the
        // non-zero exit. Mirrors `pub --confirm`'s not-confirmed path.
        anyhow::bail!(
            "write-precheck for `{}` was not confirmed in router storage",
            report.topic
        );
    }
    Ok(())
}

/// Derive the scratch sub-key a precheck writes to. It lives UNDER the
/// target key (`<key>/.goat-precheck/<nonce>`), so the router's
/// prefix-scoped write-ACL governs it identically to `key` itself, but
/// the real topic value is never touched. Any trailing `/` on the
/// caller's key is normalized away so we don't emit a `//`.
fn scratch_key_for(key: &str, nonce: &str) -> String {
    format!("{}/.goat-precheck/{nonce}", key.trim_end_matches('/'))
}

/// 8-byte OS-random nonce, lowercase hex. Unique enough that neither the
/// scratch key nor the payload can collide with anything already in
/// storage.
fn precheck_nonce() -> String {
    use rand_core::{OsRng, RngCore};
    let mut buf = [0u8; 8];
    OsRng.fill_bytes(&mut buf);
    let mut out = String::with_capacity(16);
    for b in buf {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Map a confirm read-back outcome to the precheck verdict + operator-
/// facing prose. Pure so the landed / denied / inconclusive logic is
/// unit-testable without a live router. English only — this feeds the
/// `--json` surface verbatim.
fn classify_precheck(
    key: &str,
    outcome: &super::pubsub::ConfirmOutcome,
) -> (Verdict, String, String) {
    use super::pubsub::ConfirmOutcome;
    match outcome {
        ConfirmOutcome::Landed => (
            Verdict::Ok,
            format!("sentinel round-tripped through router storage — you can write to `{key}`"),
            String::new(),
        ),
        ConfirmOutcome::NotFound => (
            Verdict::Fail,
            format!("write to `{key}` was DENIED at the router — nothing came back"),
            format!(
                "the key's prefix is likely not onboarded for your cert (no write-ACL), or no storage is registered for it. Run `goat access why me {key}` to see which layer denies, or `goat admin onboard` the prefix."
            ),
        ),
        ConfirmOutcome::ValueMismatch => (
            // Near-impossible at a nonce'd scratch key; treat as
            // inconclusive rather than pass/fail.
            Verdict::Skip,
            format!("inconclusive — a value exists at the scratch key under `{key}` but is not the sentinel just written"),
            "re-run; the unique nonce should make this collision vanish.".to_string(),
        ),
        ConfirmOutcome::Unverifiable(why) => (
            Verdict::Skip,
            format!("could not verify the write to `{key}` ({why})"),
            "the sentinel publish itself returned ok; the read-back could not run. Re-try, or run `goat doctor` to check router/storage reachability.".to_string(),
        ),
    }
}

fn print_precheck_pretty(r: &PrecheckReport, display_topic: &str) {
    let glyph = match r.verdict {
        Verdict::Ok => "[✓]",
        Verdict::Warn => "[!]",
        Verdict::Skip => "[⏭]",
        Verdict::Fail => "[✗]",
    };
    println!(
        "{glyph} {probe} {topic} — {summary} ({ms}ms)",
        probe = r.probe,
        topic = display_topic,
        summary = r.summary,
        ms = r.elapsed_ms
    );
    if !r.hint.is_empty() {
        println!("     hint: {}", r.hint);
    }
}

fn step_name(n: u8) -> &'static str {
    match n {
        1 => "ztol-membership",
        2 => "zenoh-tls-material",
        3 => "zenoh-mtls-session",
        4 => "portal-api-orgs",
        _ => "?",
    }
}

fn print_pretty(o: &StepOutcome) {
    let glyph = match o.verdict {
        Verdict::Ok => "[✓]",
        Verdict::Warn => "[!]",
        Verdict::Skip => "[⏭]",
        Verdict::Fail => "[✗]",
    };
    let names_suffix = o
        .names_loc
        .as_deref()
        .map(|n| format!(" [{n}]"))
        .unwrap_or_default();
    println!(
        "{glyph} step-{step} {name} — {summary} ({ms}ms){names_suffix}",
        step = o.step,
        name = o.name,
        summary = o.summary_loc,
        ms = o.elapsed_ms
    );
    if !o.hint_loc.is_empty() {
        println!("     hint: {}", o.hint_loc);
    }
    if let Some(checks) = &o.checks {
        for c in checks {
            let glyph = match c.verdict {
                Verdict::Ok => "✓",
                Verdict::Warn => "!",
                Verdict::Skip => "⏭",
                Verdict::Fail => "✗",
            };
            println!("     [{glyph}] {} — {}", c.id, c.summary_loc);
            if !c.hint_loc.is_empty() {
                println!("         next: {}", c.hint_loc);
            }
        }
    }
}

fn emit(o: &StepOutcome, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(o).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print_pretty(o);
    }
}

fn skipped(step: u8) -> StepOutcome {
    let (summary, summary_loc) = tr2!("doctor-skipped-summary");
    StepOutcome {
        step,
        name: step_name(step),
        verdict: Verdict::Skip,
        elapsed_ms: 0,
        summary,
        summary_loc,
        hint: String::new(),
        hint_loc: String::new(),
        cause: None,
        checks: None,
        names: None,
        names_loc: None,
    }
}

fn skipped_no_profile(step: u8) -> StepOutcome {
    let mut outcome = skipped(step);
    let (summary, summary_loc) = tr2!("doctor-skipped-no-profile-summary");
    outcome.summary = summary;
    outcome.summary_loc = summary_loc;
    outcome
}

fn skipped_no_portal_token() -> StepOutcome {
    let mut outcome = skipped(4);
    let (summary, summary_loc) = tr2!("doctor-skipped-no-portal-token-summary");
    outcome.summary = summary;
    outcome.summary_loc = summary_loc;
    outcome
}

fn skipped_no_endpoint(step: u8) -> StepOutcome {
    let mut outcome = skipped(step);
    let field = if step == 4 {
        "portal_endpoint"
    } else {
        "router_endpoint"
    };
    let (summary, summary_loc) = tr2!("doctor-skipped-no-endpoint-summary", field = field);
    outcome.summary = summary;
    outcome.summary_loc = summary_loc;
    outcome
}

fn map_preflight_failure(o: &StepOutcome, profile: &Profile) -> GoatError {
    if let Err(e) = profile.require_initialized() {
        return e;
    }
    let first = o
        .checks
        .as_ref()
        .and_then(|cs| cs.iter().find(|c| c.verdict == Verdict::Fail));
    match first {
        Some(c) if c.id == "clock.skew" => GoatError::ClockSkew {
            hint: c.hint.clone(),
        },
        Some(c) if c.id == "profile.permission" => GoatError::ProfileDenied {
            context: profile.context.clone(),
            hint: c.hint.clone(),
        },
        Some(c) if c.id.starts_with("profile.") => GoatError::ProfileMissing {
            context: profile.context.clone(),
            hint: c.hint.clone(),
        },
        Some(c) if c.id == "mesh.client" => GoatError::DoctorInconclusive {
            hint: c.hint.clone(),
        },
        Some(c) => GoatError::OffMesh {
            hint: c.hint.clone(),
        },
        None => GoatError::OffMesh {
            hint: o.hint.clone(),
        },
    }
}

/// Step 1 — TCP-connect to the router host. The router endpoint shape is `<proto>/<host>:<port>` (Zenoh
/// locator); we parse host+port out for the probe.
async fn probe_ztol(
    extras: &GoatCliExtras,
    fb: Option<&(std::net::SocketAddr, crate::names::FallbackNote)>,
) -> StepOutcome {
    let start = Instant::now();
    let (host, port) = match parse_zenoh_locator(&extras.router_endpoint) {
        Some(hp) => hp,
        None => {
            return fail(
                1,
                start,
                tr2!("doctor-endpoint-malformed-summary"),
                tr2!(
                    "doctor-step1-endpoint-malformed-hint",
                    endpoint = extras.router_endpoint.clone()
                ),
            )
            .with_cause("router.config")
        }
    };
    let connect = async {
        match fb {
            Some((addr, _)) => TcpStream::connect(*addr).await,
            None => TcpStream::connect((host.as_str(), port)).await,
        }
    };
    let outcome = match timeout(TCP_PROBE_TIMEOUT, connect).await {
        Ok(Ok(_)) => ok(
            1,
            start,
            tr2!(
                "doctor-step1-ok-summary",
                host = host.clone(),
                port = port.to_string()
            ),
        ),
        Ok(Err(e)) => fail(
            1,
            start,
            tr2!("doctor-step1-conn-failed-summary"),
            tr2!("doctor-step1-conn-failed-hint", err = e.to_string()),
        ),
        Err(_) => fail(
            1,
            start,
            tr2!("doctor-step1-timeout-summary"),
            tr2!(
                "doctor-step1-timeout-hint",
                host = host.clone(),
                port = port.to_string(),
                ms = TCP_PROBE_TIMEOUT.as_millis().to_string()
            ),
        ),
    };
    match fb {
        Some((_, note)) => outcome.with_names_note(note),
        None => outcome,
    }
}

/// Resolve one probe target through the fallback resolver; `Some` only when
/// the answer came from the signed snapshot (mesh DNS did not answer).
async fn resolve_fallback(
    profile: &Profile,
    hp: Option<&(String, u16)>,
) -> Option<(std::net::SocketAddr, crate::names::FallbackNote)> {
    let (host, port) = hp?;
    match crate::names::resolve_host_port(profile, host, *port).await {
        Ok(r) => r.source.fallback_note().map(|note| (r.addr, note)),
        Err(_) => None,
    }
}

/// Step 2 — validate local TLS material. The previous implementation made
/// a second TCP connection and called it TLS; only step 3 did a handshake.
fn validate_tls_material(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
    ca_roots_path: &std::path::Path,
) -> StepOutcome {
    let start = Instant::now();
    let mut leaf_public_key = None;
    for (path, label) in [
        (ca_roots_path, "ca-roots.pem"),
        (cert_path, "mtls.cert.pem"),
    ] {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                return fail(
                    2,
                    start,
                    tr2!(
                        "doctor-step2-material-failed-summary",
                        file = label.to_string()
                    ),
                    tr2!("doctor-step2-material-failed-hint", err = e.to_string()),
                )
                .with_cause("tls.material")
            }
        };
        let certs = match parse_pem_certificates(&bytes) {
            Ok(certs) => certs,
            Err(_) => {
                return fail(
                    2,
                    start,
                    tr2!(
                        "doctor-step2-material-failed-summary",
                        file = label.to_string()
                    ),
                    tr2!(
                        "doctor-step2-material-invalid-hint",
                        file = label.to_string()
                    ),
                )
                .with_cause("tls.material")
            }
        };
        if path == cert_path {
            leaf_public_key = certs.first().cloned();
        }
    }
    let key = match std::fs::read(key_path) {
        Ok(key) => key,
        Err(e) => {
            return fail(
                2,
                start,
                tr2!(
                    "doctor-step2-material-failed-summary",
                    file = "mtls.key.pem".to_string()
                ),
                tr2!("doctor-step2-material-failed-hint", err = e.to_string()),
            )
            .with_cause("tls.material")
        }
    };
    let cert = leaf_public_key.expect("certificate parser returned a nonempty chain");
    if parse_private_key_public_key(&key).ok().as_deref() != Some(cert.2.as_slice()) {
        return fail(
            2,
            start,
            tr2!(
                "doctor-step2-material-failed-summary",
                file = "mtls.key.pem".to_string()
            ),
            tr2!("doctor-step2-key-invalid-hint"),
        )
        .with_cause("tls.material");
    }
    let (not_before, not_after, _) = cert;
    let now = chrono::Utc::now().timestamp();
    if now < not_before || now > not_after {
        return fail(
            2,
            start,
            tr2!("doctor-step2-cert-time-summary"),
            tr2!("doctor-step2-cert-time-hint"),
        )
        .with_cause("tls.cert-time");
    }
    ok(2, start, tr2!("doctor-step2-material-ok-summary"))
}

fn parse_pem_certificates(bytes: &[u8]) -> anyhow::Result<Vec<(i64, i64, Vec<u8>)>> {
    let blocks = pem::parse_many(bytes)?;
    anyhow::ensure!(!blocks.is_empty(), "no PEM certificates");
    blocks
        .iter()
        .map(|block| {
            anyhow::ensure!(block.tag() == "CERTIFICATE", "unexpected PEM block");
            let (remaining, cert) = x509_parser::parse_x509_certificate(block.contents())
                .map_err(|e| anyhow::anyhow!("X.509 parse: {e}"))?;
            anyhow::ensure!(remaining.is_empty(), "trailing certificate DER");
            Ok((
                cert.validity().not_before.timestamp(),
                cert.validity().not_after.timestamp(),
                cert.public_key().subject_public_key.data.to_vec(),
            ))
        })
        .collect()
}

fn parse_private_key_public_key(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let key = std::str::from_utf8(bytes)?;
    use ed25519_dalek::pkcs8::DecodePrivateKey as _;
    use p256::pkcs8::DecodePrivateKey as _;
    if let Ok(signing) = ed25519_dalek::SigningKey::from_pkcs8_pem(key) {
        return Ok(signing.verifying_key().as_bytes().to_vec());
    }
    if let Ok(signing) = p256::ecdsa::SigningKey::from_pkcs8_pem(key) {
        return Ok(signing
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec());
    }
    use p256::elliptic_curve::SecretKey;
    let signing = p256::ecdsa::SigningKey::from(SecretKey::<p256::NistP256>::from_sec1_pem(key)?);
    Ok(signing
        .verifying_key()
        .to_sec1_point(false)
        .as_bytes()
        .to_vec())
}

/// Step 3 — open a real Zenoh session with mTLS. The session-open
/// path executes the full handshake and checks that the router opens
/// a session for this identity. It does not prove topic-level access.
/// Close immediately. The probe itself lives in
/// `crate::probe` (shared with the `goat status` session line, #1399).
async fn probe_zenoh_session(
    extras: &GoatCliExtras,
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
    ca_roots_path: &std::path::Path,
    fb: Option<&(std::net::SocketAddr, crate::names::FallbackNote)>,
) -> StepOutcome {
    use crate::probe::SessionFailure;
    let start = Instant::now();
    let material = crate::probe::SessionMaterial {
        cert: cert_path,
        key: key_path,
        ca_roots: ca_roots_path,
    };
    let fb_addr = fb.map(|(addr, _)| *addr);
    let result = crate::probe::zenoh_session(
        &extras.router_endpoint,
        &material,
        fb_addr,
        crate::probe::ZENOH_OPEN_TIMEOUT,
    )
    .await;
    let outcome = match result {
        Ok(_) => ok(3, start, tr2!("doctor-step3-ok-summary")),
        Err(SessionFailure::Material(m)) => {
            // Name the specific missing file, so a missing mtls.cert.pem never
            // surfaces as a raw zenoh error inside doctor.
            let cause = m.describe();
            fail(
                3,
                start,
                tr2!("doctor-step3-open-failed-summary"),
                (cause.clone(), cause),
            )
            .with_cause("session.material")
        }
        Err(SessionFailure::Config(e)) => fail(
            3,
            start,
            tr2!("doctor-step3-config-failed-summary"),
            tr2!(
                "doctor-step3-config-failed-hint",
                endpoint = extras.router_endpoint.clone(),
                err = e.clone()
            ),
        )
        .with_cause("session.config"),
        Err(SessionFailure::TlsConfig(e)) => fail(
            3,
            start,
            tr2!("doctor-step3-tls-config-failed-summary"),
            tr2!("doctor-step3-tls-config-failed-hint", err = e.clone()),
        )
        .with_cause("session.config"),
        Err(SessionFailure::Open { message, shape }) => {
            let cause = match shape {
                crate::classify::Shape::Reachability => "session.reachability",
                crate::classify::Shape::HandshakeOrAcl => "session.handshake",
                crate::classify::Shape::Unknown => "session.unknown",
            };
            fail(
                3,
                start,
                tr2!("doctor-step3-open-failed-summary"),
                tr2!("doctor-step3-open-failed-hint", err = message.clone()),
            )
            .with_cause(cause)
        }
        Err(SessionFailure::Timeout(budget)) => fail(
            3,
            start,
            tr2!("doctor-step3-timeout-summary"),
            tr2!(
                "doctor-step3-timeout-hint",
                secs = budget.as_secs().to_string()
            ),
        )
        .with_cause("session.timeout"),
    };
    match fb {
        Some((_, note)) => outcome.with_names_note(note),
        None => outcome,
    }
}

/// The cause tag for a status the portal answered the step-4 probe with:
/// `portal.status.<code>:<path>`. Causes are `&'static str`; doctor probes
/// once per process, so leaking this one short string is the simple form.
fn portal_status_cause(status: u16, path: &str) -> &'static str {
    Box::leak(format!("{PORTAL_STATUS_CAUSE}{status}:{path}").into_boxed_str())
}

const PORTAL_STATUS_CAUSE: &str = "portal.status.";

/// Step 4 — portal /api/orgs with Bearer token. Confirms portal
/// reachability + auth + that the operator key is wired correctly.
async fn probe_portal(
    extras: &GoatCliExtras,
    fb: Option<&(std::net::SocketAddr, crate::names::FallbackNote)>,
) -> StepOutcome {
    let start = Instant::now();
    let token = String::from_utf8_lossy(&extras.portal_token).into_owned();
    let mut client = PortalClient::new(extras.portal_endpoint.clone(), token);
    if let Some((addr, _)) = fb {
        if let Some((host, _)) = crate::cmd::status::parse_https_host_port(&extras.portal_endpoint)
        {
            client = client.with_resolve(&host, *addr);
        }
    }
    let portal_probe_timeout = portal_timeout();
    let outcome = match timeout(portal_probe_timeout, client.list_vendors()).await {
        Ok(Ok(vendors)) => ok(
            4,
            start,
            tr2!("doctor-step4-ok-summary", n = vendors.len().to_string()),
        ),
        Ok(Err(e)) if e.http_status().is_some_and(|s| s.as_u16() == 401) => fail(
            4,
            start,
            tr2!("doctor-step4-token-rejected-summary"),
            tr2!(
                "doctor-step4-token-rejected-hint",
                status = "401".to_string()
            ),
        )
        .with_cause("portal.unauthorized"),
        Ok(Err(e)) if e.http_status().is_some_and(|s| s.as_u16() == 403) => fail(
            4,
            start,
            tr2!("doctor-step4-forbidden-summary"),
            tr2!("doctor-step4-forbidden-hint"),
        )
        .with_cause("portal.forbidden"),
        Ok(Err(panoscope_core::portal::PortalError::Decode(e))) => fail(
            4,
            start,
            tr2!("doctor-step4-decode-summary"),
            tr2!("doctor-step4-decode-hint", err = e.clone()),
        )
        .with_cause("portal.decode"),
        // Any other status is an answer (#2087): carry it, with the request
        // path, so the exit code says "not there" / "retry later" / "the
        // portal is failing" rather than "unreachable".
        Ok(Err(e)) if e.http_status().is_some() => fail(
            4,
            start,
            tr2!("doctor-step4-request-failed-summary"),
            tr2!("doctor-step4-request-failed-hint", err = e.to_string()),
        )
        .with_cause(portal_status_cause(
            e.http_status().map_or(0, |s| s.as_u16()),
            e.path(),
        )),
        Ok(Err(e)) => fail(
            4,
            start,
            tr2!("doctor-step4-request-failed-summary"),
            tr2!("doctor-step4-request-failed-hint", err = e.to_string()),
        ),
        Err(_) => fail(
            4,
            start,
            tr2!("doctor-step4-timeout-summary"),
            tr2!(
                "doctor-step4-timeout-hint",
                secs = portal_probe_timeout.as_secs().to_string(),
                endpoint = extras.portal_endpoint.clone()
            ),
        ),
    };
    match fb {
        Some((_, note)) => outcome.with_names_note(note),
        None => outcome,
    }
}

/// Build an Ok outcome. `summary` is the `(english, localized)` pair from
/// `tr2!`; no hint on success.
fn ok(step: u8, start: Instant, summary: (String, String)) -> StepOutcome {
    StepOutcome {
        step,
        name: step_name(step),
        verdict: Verdict::Ok,
        elapsed_ms: start.elapsed().as_millis(),
        summary: summary.0,
        summary_loc: summary.1,
        hint: String::new(),
        hint_loc: String::new(),
        cause: None,
        checks: None,
        names: None,
        names_loc: None,
    }
}

/// Build a Fail outcome. `summary` and `hint` are `(english, localized)`
/// pairs from `tr2!`. The English halves land in the serialized `--json`
/// fields; typed causes determine exit codes. The localized halves drive
/// the pretty render.
fn fail(
    step: u8,
    start: Instant,
    summary: (String, String),
    hint: (String, String),
) -> StepOutcome {
    StepOutcome {
        step,
        name: step_name(step),
        verdict: Verdict::Fail,
        elapsed_ms: start.elapsed().as_millis(),
        summary: summary.0,
        summary_loc: summary.1,
        hint: hint.0,
        hint_loc: hint.1,
        cause: None,
        checks: None,
        names: None,
        names_loc: None,
    }
}

fn map_doctor_failure(o: &StepOutcome, profile: &Profile) -> GoatError {
    match o.step {
        0 => GoatError::DoctorInconclusive {
            hint: o.hint.clone(),
        },
        1 if o.cause == Some("router.config") => GoatError::DoctorInconclusive {
            hint: o.hint.clone(),
        },
        1 => GoatError::RouterUnreachable {
            endpoint: configured_endpoint(profile, "router_endpoint", &o.summary),
            hint: o.hint.clone(),
        },
        2 => match o.cause {
            Some("tls.cert-time") => GoatError::CertExpired {
                hint: o.hint.clone(),
            },
            _ => GoatError::DoctorInconclusive {
                hint: o.hint.clone(),
            },
        },
        3 => match o.cause {
            Some("session.material") => GoatError::ProfileMissing {
                context: profile.context.clone(),
                hint: o.hint.clone(),
            },
            Some("session.timeout" | "session.reachability") => GoatError::RouterUnreachable {
                endpoint: configured_endpoint(profile, "router_endpoint", &o.summary),
                hint: o.hint.clone(),
            },
            _ => GoatError::DoctorInconclusive {
                hint: o.hint.clone(),
            },
        },
        4 => match o.cause {
            Some("portal.unauthorized") => GoatError::TokenInvalid {
                hint: o.hint.clone(),
            },
            Some("portal.forbidden") => GoatError::PortalForbidden {
                hint: o.hint.clone(),
            },
            Some("portal.decode") => GoatError::PortalBadResponse {
                hint: o.hint.clone(),
            },
            Some(c) if c.starts_with(PORTAL_STATUS_CAUSE) => {
                let (status, path) = c[PORTAL_STATUS_CAUSE.len()..]
                    .split_once(':')
                    .unwrap_or(("0", ""));
                crate::errors::portal_answer(status.parse().unwrap_or(0), path, None, "")
            }
            _ => GoatError::PortalUnreachable {
                endpoint: configured_endpoint(profile, "portal_endpoint", &o.summary),
                hint: o.hint.clone(),
            },
        },
        _ => GoatError::BundleInvalid {
            hint: o.hint.clone(),
        },
    }
}

fn configured_endpoint(profile: &Profile, field: &str, fallback: &str) -> String {
    std::fs::read_to_string(profile.profile_toml_path())
        .ok()
        .and_then(|body| crate::profile::extract_toml_field(&body, field))
        .unwrap_or_else(|| fallback.to_string())
}

/// Parse `<proto>/<host>:<port>` (Zenoh wire locator). Returns
/// (host, port) on success.
/// Budget for `netbird status --json` (local IPC round-trip).
const NETBIRD_STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// What the local NetBird client reports about the path into the mesh.
#[derive(Debug, PartialEq)]
pub(crate) enum MeshPath {
    /// Registered with the control server, but no tunnel to any peer and no
    /// relay reachable: the local network passes TCP 443 and blocks the data
    /// ports. `relays` / `stun` are the `host:port` endpoints that failed.
    ControlOnly {
        total_peers: usize,
        relays: Vec<String>,
        stun: Vec<String>,
    },
    /// Anything else: not registered, some peer connected, or a relay up.
    Other,
}

/// Classify `netbird status --json`. Pure for testability; `None` when the
/// output is not the shape this reads.
///
/// Control-only means ALL of: management connected; at least one peer in
/// the map; zero peers connected; and no relay (`rel://` / `rels://`)
/// available. A device on a site that allows only TCP 443 out shows exactly
/// this (zero peers connected, every relay `context deadline exceeded`), and
/// `connected: true` in the management API hides it.
pub(crate) fn classify_mesh_path(raw: &[u8]) -> Option<MeshPath> {
    let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
    let mgmt = v.get("management")?.get("connected")?.as_bool()?;
    let peers = v.get("peers")?;
    let total = peers.get("total")?.as_u64()? as usize;
    let connected = peers.get("connected")?.as_u64()? as usize;
    let details = v
        .get("relays")
        .and_then(|r| r.get("details"))
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let (mut relays, mut stun, mut relay_up) = (Vec::new(), Vec::new(), false);
    for d in &details {
        let uri = d.get("uri").and_then(|u| u.as_str()).unwrap_or_default();
        let up = d
            .get("available")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
        let endpoint = uri
            .split_once(':')
            .map(|(_, rest)| rest.trim_start_matches("//"))
            .unwrap_or(uri);
        if uri.starts_with("rel://") || uri.starts_with("rels://") {
            relay_up |= up;
            if !up {
                relays.push(endpoint.to_string());
            }
        } else if (uri.starts_with("stun:") || uri.starts_with("turn:")) && !up {
            stun.push(endpoint.to_string());
        }
    }
    if mgmt && total > 0 && connected == 0 && !relay_up {
        relays.sort();
        relays.dedup();
        stun.sort();
        stun.dedup();
        return Some(MeshPath::ControlOnly {
            total_peers: total,
            relays,
            stun,
        });
    }
    Some(MeshPath::Other)
}

/// Step 1 failed: ask the local NetBird client why before blaming "off-mesh".
/// A control-only path gets its own named failure with the firewall fix;
/// anything else (no daemon, unparseable output, a different failure) leaves
/// the original outcome untouched.
async fn refine_off_mesh(o: StepOutcome) -> StepOutcome {
    let bin = std::env::var("GOAT_NETBIRD_BIN").unwrap_or_else(|_| "netbird".into());
    refine_off_mesh_with_bin(o, &bin, NETBIRD_STATUS_TIMEOUT).await
}

async fn refine_off_mesh_with_bin(mut o: StepOutcome, bin: &str, budget: Duration) -> StepOutcome {
    // Use the same client selection and timeout cleanup as step 0. A status
    // command can hang here too when the daemon disappears between probes.
    let out = super::doctor_preflight::netbird_status(bin, budget).await;
    let raw = match out {
        Ok(r) if r.status.success() => r.stdout,
        _ => return o,
    };
    if let Some(MeshPath::ControlOnly {
        total_peers,
        relays,
        stun,
    }) = classify_mesh_path(&raw)
    {
        let relay_list = if relays.is_empty() {
            "the relays".to_string()
        } else {
            relays.join(", ")
        };
        let stun_list = if stun.is_empty() {
            "the same hosts".to_string()
        } else {
            stun.join(", ")
        };
        let (summary, summary_loc) = tr2!(
            "doctor-step1-control-only-summary",
            total = total_peers.to_string()
        );
        let (hint, hint_loc) = tr2!(
            "doctor-step1-control-only-hint",
            relays = relay_list.clone(),
            stun = stun_list.clone()
        );
        o.summary = summary;
        o.summary_loc = summary_loc;
        o.hint = hint;
        o.hint_loc = hint_loc;
    }
    o
}

fn parse_zenoh_locator(s: &str) -> Option<(String, u16)> {
    let after_proto = s.split_once('/')?.1;
    let (host, port) = after_proto.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host.to_string(), port))
}

/// Bare endpoint pair pulled out of profile.toml. We avoid a full
/// TOML parser dep here — the file has a known shape we wrote.
#[derive(Default)]
struct ProfileEndpoints {
    portal_endpoint: Option<String>,
    router_endpoint: Option<String>,
}

fn parse_endpoints(toml_body: &str) -> ProfileEndpoints {
    // Delegates to the shared, correct field reader (crate::profile::
    // extract_toml_field) instead of a local line-scan — the old local
    // version's `strip_prefix("portal_endpoint")` also matched a line like
    // `portal_endpoint_backup = "..."` and then mis-extracted its value as
    // if it were `portal_endpoint`.
    ProfileEndpoints {
        portal_endpoint: crate::profile::extract_toml_field(toml_body, "portal_endpoint")
            .filter(|value| !value.trim().is_empty()),
        router_endpoint: crate::profile::extract_toml_field(toml_body, "router_endpoint")
            .filter(|value| !value.trim().is_empty()),
    }
}

/// Write the doctor cache file for the outer-frame ZTOL precheck.
/// Subcommands consult this in Phase C; for now we just write it
/// so the contract is observable from `goat doctor`'s side.
fn write_doctor_cache(profile: &Profile, step1: &StepOutcome) -> std::io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let body = serde_json::json!({
        "ts_unix": now,
        "ztol_ok": step1.verdict == Verdict::Ok,
        "step1_summary": step1.summary,
    });
    let path = profile.context_dir().join("last-doctor.json");
    std::fs::write(path, serde_json::to_string(&body).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape of a control-only device: management and signal up, no peers
    // connected, every relay timed out, STUN still checking.
    const CONTROL_ONLY: &str = r#"{
      "management": {"url": "https://netbird.example:443", "connected": true, "error": ""},
      "signal": {"url": "https://netbird.example:443", "connected": true, "error": ""},
      "peers": {"total": 41, "connected": 0, "details": []},
      "relays": {"total": 4, "available": 0, "details": [
        {"uri": "stun:relay-a.example:3478", "available": false, "error": ""},
        {"uri": "stun:relay-b.example:3478", "available": false, "error": ""},
        {"uri": "rels://relay-a.example:33073", "available": false, "error": "context deadline exceeded"},
        {"uri": "rels://relay-b.example:33073", "available": false, "error": "context deadline exceeded"}
      ]}
    }"#;

    #[test]
    fn control_only_is_named_with_the_failed_endpoints() {
        assert_eq!(
            classify_mesh_path(CONTROL_ONLY.as_bytes()),
            Some(MeshPath::ControlOnly {
                total_peers: 41,
                relays: vec![
                    "relay-a.example:33073".into(),
                    "relay-b.example:33073".into()
                ],
                stun: vec!["relay-a.example:3478".into(), "relay-b.example:3478".into()],
            })
        );
    }

    #[test]
    fn one_connected_peer_is_not_control_only() {
        let raw = CONTROL_ONLY.replace(r#""connected": 0"#, r#""connected": 1"#);
        assert_eq!(classify_mesh_path(raw.as_bytes()), Some(MeshPath::Other));
    }

    #[test]
    fn a_reachable_relay_is_not_control_only() {
        // Peers still connecting, but the relay path is open: a slow start,
        // not a blocked network.
        let raw = CONTROL_ONLY.replacen(
            r#"{"uri": "rels://relay-a.example:33073", "available": false"#,
            r#"{"uri": "rels://relay-a.example:33073", "available": true"#,
            1,
        );
        assert_eq!(classify_mesh_path(raw.as_bytes()), Some(MeshPath::Other));
    }

    #[test]
    fn management_down_is_not_control_only() {
        let raw = CONTROL_ONLY.replacen(r#""connected": true"#, r#""connected": false"#, 1);
        assert_eq!(classify_mesh_path(raw.as_bytes()), Some(MeshPath::Other));
    }

    #[test]
    fn empty_peer_map_is_not_control_only() {
        // A first device on a fresh goatnet has nobody to connect to.
        let raw = CONTROL_ONLY.replace(r#""total": 41"#, r#""total": 0"#);
        assert_eq!(classify_mesh_path(raw.as_bytes()), Some(MeshPath::Other));
    }

    #[test]
    fn unparseable_status_is_none() {
        assert_eq!(classify_mesh_path(b"not json"), None);
        assert_eq!(classify_mesh_path(b"{}"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn router_failure_refinement_kills_timed_out_status_process() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("netbird");
        let pid_file = dir.path().join("pid");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let original = fail(
            1,
            Instant::now(),
            ("router unavailable".into(), "router unavailable".into()),
            (
                "compare the router path".into(),
                "compare the router path".into(),
            ),
        );
        let result =
            refine_off_mesh_with_bin(original, script.to_str().unwrap(), Duration::from_secs(1))
                .await;
        assert_eq!(result.summary, "router unavailable");
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut gone = false;
        for _ in 0..20 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if !gone {
            // Keep the regression fixture from leaking even if it fails.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(gone, "router-failure status child survived its deadline");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn router_failure_refinement_uses_the_selected_status_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("custom-netbird");
        std::fs::write(
            &script,
            format!("#!/bin/sh\n[ \"$*\" = 'status --json' ] || exit 1\n/bin/cat <<'STATUS'\n{CONTROL_ONLY}\nSTATUS\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let original = fail(
            1,
            Instant::now(),
            ("router unavailable".into(), "router unavailable".into()),
            (
                "compare the router path".into(),
                "compare the router path".into(),
            ),
        );
        let result =
            refine_off_mesh_with_bin(original, script.to_str().unwrap(), Duration::from_secs(1))
                .await;
        assert_eq!(result.verdict, Verdict::Fail);
        assert_ne!(result.summary, "router unavailable");
        assert!(result.hint.contains("relay-a.example:33073"));
    }

    #[test]
    fn tls_material_rejects_mismatched_and_malformed_keys() {
        use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        let ca_path = dir.path().join("ca.pem");
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&ca_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        assert_eq!(
            validate_tls_material(&cert_path, &key_path, &ca_path).verdict,
            Verdict::Ok
        );

        let other = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        std::fs::write(&key_path, other.serialize_pem()).unwrap();
        let mismatch = validate_tls_material(&cert_path, &key_path, &ca_path);
        assert_eq!(mismatch.cause, Some("tls.material"));
        std::fs::write(&key_path, "broken key").unwrap();
        assert_eq!(
            validate_tls_material(&cert_path, &key_path, &ca_path).cause,
            Some("tls.material")
        );
    }

    #[test]
    fn tls_material_rejects_malformed_later_ca_certificate() {
        use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        let ca_path = dir.path().join("ca.pem");
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        let mut bundle = cert.pem();
        bundle.push_str("\n-----BEGIN CERTIFICATE-----\nYmFk\n-----END CERTIFICATE-----\n");
        std::fs::write(&ca_path, bundle).unwrap();
        assert_eq!(
            validate_tls_material(&cert_path, &key_path, &ca_path).cause,
            Some("tls.material")
        );
    }

    #[test]
    fn parse_zenoh_locator_tls() {
        assert_eq!(
            parse_zenoh_locator("tls/100.127.0.193:7447"),
            Some(("100.127.0.193".into(), 7447))
        );
    }

    #[test]
    fn parse_zenoh_locator_tcp_hostname() {
        assert_eq!(
            parse_zenoh_locator("tcp/router.nova.netbird.test:7447"),
            Some(("router.nova.netbird.test".into(), 7447))
        );
    }

    #[test]
    fn parse_zenoh_locator_rejects_bare_host() {
        assert_eq!(parse_zenoh_locator("100.127.0.193:7447"), None);
    }

    #[test]
    fn parse_zenoh_locator_rejects_no_port() {
        assert_eq!(parse_zenoh_locator("tls/100.127.0.193"), None);
    }

    #[test]
    fn parse_endpoints_extracts_both() {
        let body = r#"
# goat profile.toml — written by `goat profile init` (ADR 1019).
portal_endpoint = "https://portal.nova.netbird.test"
router_endpoint = "tls/100.127.0.193:7447"
device_id = "operator-test"
issued_at_unix = 1779969600
"#;
        let eps = parse_endpoints(body);
        assert_eq!(
            eps.portal_endpoint.as_deref(),
            Some("https://portal.nova.netbird.test")
        );
        assert_eq!(
            eps.router_endpoint.as_deref(),
            Some("tls/100.127.0.193:7447")
        );
    }

    #[test]
    fn blank_endpoints_are_missing_inputs() {
        let endpoints = parse_endpoints("portal_endpoint = \"   \"\nrouter_endpoint = \"\"\n");
        assert!(endpoints.portal_endpoint.is_none());
        assert!(endpoints.router_endpoint.is_none());
        let outcome = skipped_no_endpoint(4);
        assert_eq!(outcome.verdict, Verdict::Skip);
        assert_eq!(
            outcome.summary,
            "skipped — profile.toml missing portal_endpoint"
        );
    }

    #[test]
    fn en_catalog_reproduces_doctor_strings_verbatim() {
        // Locks the externalized English to the pre-i18n literals, so the
        // serialized --json `summary`/`hint` stay byte-identical.
        use i18n_embed_fl::fl;
        let l = &*crate::i18n::EN_LOADER;
        assert_eq!(
            fl!(
                l,
                "doctor-step1-ok-summary",
                host = "100.127.0.1",
                port = "7447"
            ),
            "TCP connect to router host 100.127.0.1:7447 succeeded"
        );
        assert_eq!(
            fl!(l, "doctor-step1-timeout-hint", host = "h", port = "7447", ms = "1500"),
            "router host h:7447 did not answer in 1500ms — compare from another mesh peer; check tunnel, route and router service"
        );
        assert_eq!(
            fl!(l, "doctor-step4-ok-summary", n = "12"),
            "portal returned 12 org row(s)"
        );
        assert_eq!(
            fl!(l, "doctor-skipped-summary"),
            "skipped — prior step failed (re-run with --all to force)"
        );
    }

    #[test]
    fn step_outcome_json_is_stable_shape() {
        let o = StepOutcome {
            step: 1,
            name: step_name(1),
            verdict: Verdict::Fail,
            elapsed_ms: 1234,
            summary: "TCP timeout".into(),
            hint: "bring up netbird".into(),
            // Localized halves are deliberately different here to prove
            // they're NOT serialized — the --json surface stays English.
            summary_loc: "délai TCP".into(),
            hint_loc: "lancez netbird".into(),
            cause: None,
            checks: None,
            names: None,
            names_loc: None,
        };
        let s = serde_json::to_string(&o).unwrap();
        // Stable contract: every field present, verdict lowercase string.
        assert!(s.contains("\"step\":1"));
        assert!(s.contains("\"name\":\"ztol-membership\""));
        assert!(s.contains("\"verdict\":\"fail\""));
        assert!(s.contains("\"summary\":\"TCP timeout\""));
        assert!(s.contains("\"hint\":\"bring up netbird\""));
        assert!(s.contains("\"elapsed_ms\":1234"));
        // The localized fields must NOT leak into --json.
        assert!(!s.contains("summary_loc"));
        assert!(!s.contains("délai TCP"));
    }

    /// First `--json` insta snapshot of a 4-step doctor run that
    /// fails at step 1 (router TCP timeout). Represents the NDJSON contract
    /// operators / wrapper scripts rely on. elapsed_ms is masked
    /// because it varies per run.
    #[test]
    fn doctor_json_off_mesh_snapshot() {
        // Helper: the *_loc fields are skip-serialized, so their values
        // don't affect the snapshot; set them equal to the English here.
        let skip = |step: u8| StepOutcome {
            step,
            name: step_name(step),
            verdict: Verdict::Skip,
            elapsed_ms: 0,
            summary: "skipped — prior step failed (re-run with --all to force)".into(),
            hint: String::new(),
            summary_loc: "skipped — prior step failed (re-run with --all to force)".into(),
            hint_loc: String::new(),
            cause: None,
            checks: None,
            names: None,
            names_loc: None,
        };
        let outcomes = vec![
            StepOutcome {
                step: 1,
                name: step_name(1),
                verdict: Verdict::Fail,
                elapsed_ms: 0,
                summary: "TCP connect to router host timed out".into(),
                hint: "router host 100.127.0.193:7447 did not answer in 1500ms — compare from another mesh peer; check tunnel, route and router service"
                    .into(),
                summary_loc: "TCP connect to router host timed out".into(),
                hint_loc: "router host 100.127.0.193:7447 did not answer in 1500ms — compare from another mesh peer; check tunnel, route and router service"
                    .into(),
                cause: None,
                checks: None,
                names: None,
                names_loc: None,
            },
            skip(2),
            skip(3),
            skip(4),
        ];
        insta::assert_json_snapshot!(outcomes);
    }

    #[test]
    fn network_and_admission_failures_do_not_claim_expired_identity() {
        let profile = Profile {
            dir: std::path::PathBuf::new(),
            context: "test".into(),
        };
        let outcome = |step, cause| StepOutcome {
            step,
            name: step_name(step),
            verdict: Verdict::Fail,
            elapsed_ms: 0,
            summary: "probe failed".into(),
            hint: "inspect the reported result".into(),
            summary_loc: String::new(),
            hint_loc: String::new(),
            cause,
            checks: None,
            names: None,
            names_loc: None,
        };
        assert_eq!(
            map_doctor_failure(&outcome(1, None), &profile).code(),
            "E_ROUTER_UNREACHABLE"
        );
        assert_eq!(
            map_doctor_failure(&outcome(3, Some("session.handshake")), &profile).code(),
            "E_DOCTOR_INCONCLUSIVE"
        );
        assert_eq!(
            map_doctor_failure(&outcome(2, Some("tls.cert-time")), &profile).code(),
            "E_CERT_EXPIRED"
        );
        assert_eq!(
            map_doctor_failure(&outcome(4, Some("portal.forbidden")), &profile).code(),
            "E_PORTAL_FORBIDDEN"
        );
    }

    #[test]
    fn scratch_key_sits_under_the_target_topic() {
        // The sentinel goes UNDER <key> so it inherits <key>'s
        // prefix-scoped write-ACL without touching the real value.
        assert_eq!(
            scratch_key_for("org11x/observations/v1", "ab12cd34"),
            "org11x/observations/v1/.goat-precheck/ab12cd34"
        );
    }

    #[test]
    fn scratch_key_normalizes_trailing_slash() {
        // A caller-supplied trailing slash must not produce a `//`.
        assert_eq!(
            scratch_key_for("acme/telemetry/", "ff00"),
            "acme/telemetry/.goat-precheck/ff00"
        );
    }

    #[test]
    fn precheck_nonce_is_16_hex_chars() {
        let n = precheck_nonce();
        assert_eq!(n.len(), 16);
        assert!(n
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Two draws should differ (collision probability ~2^-64).
        assert_ne!(precheck_nonce(), precheck_nonce());
    }

    #[test]
    fn classify_precheck_landed_is_ok_no_hint() {
        let (v, summary, hint) =
            classify_precheck("acme/x", &crate::cmd::pubsub::ConfirmOutcome::Landed);
        assert_eq!(v, Verdict::Ok);
        assert!(summary.contains("you can write to `acme/x`"));
        assert!(hint.is_empty());
    }

    #[test]
    fn classify_precheck_not_found_is_fail_with_f177_remediation() {
        let (v, summary, hint) =
            classify_precheck("acme/x", &crate::cmd::pubsub::ConfirmOutcome::NotFound);
        assert_eq!(v, Verdict::Fail);
        assert!(summary.contains("DENIED"));
        assert!(summary.contains("nothing came back"));
        // Remediation names the self-service diagnosis path.
        assert!(hint.contains("goat access why me acme/x"));
    }

    #[test]
    fn classify_precheck_mismatch_and_unverifiable_are_inconclusive_skip() {
        let (v1, _, _) = classify_precheck("k", &crate::cmd::pubsub::ConfirmOutcome::ValueMismatch);
        assert_eq!(v1, Verdict::Skip);
        let (v2, summary, _) = classify_precheck(
            "k",
            &crate::cmd::pubsub::ConfirmOutcome::Unverifiable("storage query failed".into()),
        );
        assert_eq!(v2, Verdict::Skip);
        assert!(summary.contains("storage query failed"));
    }

    #[test]
    fn precheck_report_json_shape_is_stable() {
        let r = PrecheckReport {
            probe: "write-precheck",
            topic: "acme/x".into(),
            scratch_key: "acme/x/.goat-precheck/dead".into(),
            verdict: Verdict::Fail,
            elapsed_ms: 42,
            summary: "denied".into(),
            hint: "onboard the prefix".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"probe\":\"write-precheck\""));
        assert!(s.contains("\"topic\":\"acme/x\""));
        assert!(s.contains("\"scratch_key\":\"acme/x/.goat-precheck/dead\""));
        assert!(s.contains("\"verdict\":\"fail\""));
        assert!(s.contains("\"elapsed_ms\":42"));
    }
}

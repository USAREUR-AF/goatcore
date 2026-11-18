use crate::errors::GoatError;
use clap::Subcommand;
use panoscope_core::portal::PortalError;

#[derive(Subcommand, Debug)]
pub enum VendorCmd {
    /// List all orgs declared in the portal.
    Ls {
        /// Fuzzy-match against org prefix + display name.
        #[arg(long)]
        search: Option<String>,
        /// Emit one JSON line per org instead of a pretty table.
        #[arg(long)]
        json: bool,
    },
    /// Show one org's full record (prefix, display name, topic count).
    Show {
        prefix: String,
        /// Emit JSON instead of pretty.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(ctx: &super::Ctx, op: VendorCmd) -> anyhow::Result<()> {
    let portal = ctx.profile.portal_client()?;
    match op {
        VendorCmd::Ls { search, json } => ls(&portal, search.as_deref(), json).await,
        VendorCmd::Show { prefix, json } => show(&portal, &prefix, json).await,
    }
}

async fn ls(
    portal: &panoscope_core::portal::PortalClient,
    search: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let mut vendors = portal.list_vendors().await.map_err(map_portal_err)?;
    if let Some(needle) = search {
        let n = needle.to_lowercase();
        vendors.retain(|v| {
            v.prefix.to_lowercase().contains(&n) || v.display_name.to_lowercase().contains(&n)
        });
    }
    vendors.sort_by(|a, b| a.prefix.cmp(&b.prefix));

    if json {
        for v in &vendors {
            let line = serde_json::to_string(v).unwrap_or_default();
            println!("{line}");
        }
        return Ok(());
    }

    if vendors.is_empty() {
        println!("(no orgs)");
        return Ok(());
    }
    let prefix_w = vendors
        .iter()
        .map(|v| v.prefix.len())
        .max()
        .unwrap_or(6)
        .max(6);
    println!("{:<w$}  display_name", "prefix", w = prefix_w);
    for v in &vendors {
        println!(
            "{:<w$}  {}",
            v.prefix,
            if v.display_name.is_empty() {
                "(none)"
            } else {
                &v.display_name
            },
            w = prefix_w
        );
    }
    Ok(())
}

async fn show(
    portal: &panoscope_core::portal::PortalClient,
    prefix: &str,
    json: bool,
) -> anyhow::Result<()> {
    let vendors = portal.list_vendors().await.map_err(map_portal_err)?;
    let vendor = match vendors.iter().find(|v| v.prefix == prefix) {
        Some(v) => v.clone(),
        None => anyhow::bail!(
            "org `{prefix}` not registered at the portal — `goat org ls` shows what is"
        ),
    };
    // `.unwrap_or(0)` used to render "the query failed" identically to
    // "this org genuinely has zero topics" — the two are very different
    // facts (one means "try again" / "check access", the other is just the
    // answer). The JSON field is `null` when the query failed (a consumer
    // parsing an integer fails loudly instead of reading a fabricated 0),
    // the human render says "unknown", and stderr names the cause in both.
    let topic_result = portal.list_topics(prefix).await;
    if let Err(e) = &topic_result {
        eprintln!("⚠ topic count query failed — reported as unknown, not 0: {e}");
    }

    if json {
        let detail = VendorDetail {
            prefix: vendor.prefix,
            display_name: vendor.display_name,
            description: vendor.description,
            topic_count: topic_result.as_ref().ok().map(|t| t.len()),
        };
        println!("{}", serde_json::to_string(&detail)?);
        return Ok(());
    }
    println!("prefix:       {}", vendor.prefix);
    println!(
        "display_name: {}",
        if vendor.display_name.is_empty() {
            "(none)"
        } else {
            &vendor.display_name
        }
    );
    if !vendor.description.is_empty() {
        println!("description:  {}", vendor.description);
    }
    match &topic_result {
        Ok(topics) => println!("topic_count:  {}", topics.len()),
        Err(e) => println!("topic_count:  unknown (query failed: {e})"),
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct VendorDetail {
    prefix: String,
    display_name: String,
    description: String,
    /// `null` when the topic query failed, never a fabricated 0.
    topic_count: Option<usize>,
}

pub(super) const ONBOARD_ENDPOINT_FALLBACK_HINT: &str = "no onboard_endpoint in this profile and GOAT_ONBOARD_ENDPOINT is unset, so this call fell back to the portal URL (which may not serve /api/onboard/*) — set GOAT_ONBOARD_ENDPOINT=<console onboard URL> or re-init the profile from a current bundle (goat profile init <bundle.cbor>)";

pub(super) fn append_onboard_hint(
    portal: &panoscope_core::portal::PortalClient,
    err: anyhow::Error,
) -> anyhow::Error {
    if !portal.onboard_endpoint_is_fallback() {
        return err;
    }

    let mut err = err;
    if let Some(goat) = err.downcast_mut::<GoatError>() {
        goat.append_hint(ONBOARD_ENDPOINT_FALLBACK_HINT);
        return err;
    }

    anyhow::anyhow!("{err:#}\n  hint: {ONBOARD_ENDPOINT_FALLBACK_HINT}")
}

/// The error mapping for every onboard-API caller except the `operator-key`
/// verbs. It stays command-agnostic: the console answers 403 `operator_only`
/// from unrelated gates too (the org registry, proposal decisions), so
/// operator-key wording here would send `goat admin org register` to the wrong
/// verbs.
pub(super) fn map_onboard_err(
    portal: &panoscope_core::portal::PortalClient,
    e: PortalError,
) -> anyhow::Error {
    append_onboard_hint(portal, map_portal_err(e))
}

/// The error mapping for `goat admin operator-key` alone. Its refusals that
/// send a person to the identity console name the page for THIS profile's
/// goatnet, derived from the portal host, rather than a placeholder the person
/// has to fill in (#1373). Everything else falls through to the shared mapping.
pub(super) fn map_operator_key_err(
    portal: &panoscope_core::portal::PortalClient,
    e: PortalError,
) -> anyhow::Error {
    let page = console_operator_key_page(portal.base_url(), portal.onboard_base_url());
    let status = e.http_status();
    let code = e.error_code().map(str::to_owned);
    let server = server_line(&e);
    let mapped = match (status, code.as_deref()) {
        (Some(s), Some("operator_only")) if s.as_u16() == 403 => {
            operator_only_refusal(s, &server, &page)
        }
        (Some(s), Some("unbound_principal")) if s.as_u16() == 403 => {
            unbound_principal_refusal(s, &server, &page)
        }
        (Some(s), Some("missing_handle")) if s.as_u16() == 400 => missing_handle_refusal(s, &server),
        _ => map_portal_err(e),
    };
    append_onboard_hint(portal, mapped)
}

/// The identity console's operator-key page for the goatnet this profile
/// points at. Every goatnet's surfaces share one `<site_id>.<zone>` suffix
/// (`portal.`, `onboard.`, `admin.auth.`), so the page follows from the portal
/// host; a host of another shape (a local dev portal, an IP) gets the generic
/// form rather than a guessed name.
pub(super) fn console_operator_key_page(
    portal_url: Option<&str>,
    onboard_url: Option<&str>,
) -> String {
    let candidates = [(portal_url, "portal."), (onboard_url, "onboard.")];
    for (url, label) in candidates {
        let Some(host) = url.map(endpoint_host) else { continue };
        if let Some(site) = host.strip_prefix(label).filter(|s| !s.is_empty()) {
            return format!("https://admin.auth.{site}/operator-key");
        }
    }
    GENERIC_OPERATOR_KEY_PAGE.into()
}

/// The host of an endpoint URL, ASCII-lowercased. `GOAT_ONBOARD_ENDPOINT` is
/// typed by hand and nothing upstream normalises case, while DNS names are
/// case-insensitive: `https://Onboard.ZAGROS…` must derive the same page as the
/// lowercase form instead of falling back to the generic one.
fn endpoint_host(url: &str) -> String {
    let url = url.trim().to_ascii_lowercase();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(&url);
    rest.split(['/', ':']).next().unwrap_or(rest).to_string()
}

const GENERIC_OPERATOR_KEY_PAGE: &str =
    "the identity console's /operator-key page (https://admin.auth.<site_id>.<zone>/operator-key)";

/// 403 operator_only on the operator-key door. Two readings, and the hint
/// covers both: a verb that is operator-only by design (list, approve, a
/// request naming somebody else), or a self request or collect against a
/// console from before the self-service door, which refused every vp_ token
/// the same way (#1373).
fn operator_only_refusal(status: reqwest::StatusCode, server: &str, page: &str) -> anyhow::Error {
    GoatError::NotAdmin {
        hint: format!(
            "the console refused this as operator-only (HTTP {status} operator_only).{server}\n\
             \n\
             Which operator-key verbs take which token:\n\
             \x20 request (no handle), collect <id>   your OWN key — this profile's token is \
             enough on a console with CLI self-service. A console from before that door refuses \
             these too; there, the door is the console page: {page}\n\
             \x20 request <handle>, list, approve, reject, revoke, reconcile   your OPERATOR \
             key (op_…): --context <env>-op, or GOAT_PORTAL_TOKEN=op_… (`goat admin \
             operator-key --help`)\n\
             \n\
             Nothing was changed."
        ),
    }
    .into()
}

fn unbound_principal_refusal(status: reqwest::StatusCode, server: &str, page: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "the console could not tie this token to an onboarded person (HTTP {status} \
         unbound_principal).{server}\n  hint: ask an admin to finish your onboarding, or \
         request the key from the console page, which binds from your sign-in: {page}"
    )
}

/// 400 missing_handle: the console saw an OPERATOR key with no handle. An
/// operator key files on a named person's behalf and binds to nobody, so the
/// self case is the plain profile's, not this one's.
fn missing_handle_refusal(status: reqwest::StatusCode, server: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "the console wants a handle (HTTP {status} missing_handle).{server}\n  hint: this \
         profile's token is an operator key, which files for a NAMED person: pass their handle. \
         For your own key run the same command on your plain profile (the vp_ token), which the \
         console binds to you."
    )
}

/// The portal's own message as an indented follow-on stanza, or empty.
/// Every mapper that composes a server message into a curated hint uses
/// this one renderer: the same fact rendered two ways would make what the
/// operator sees depend on which verb refused them. The stanza form (rather
/// than a trailing sentence) survives long curated hints without running
/// them together into a wall of prose.
pub(super) fn server_line(e: &PortalError) -> String {
    e.server_message()
        .map(|m| format!("\n  server: {m}"))
        .unwrap_or_default()
}

/// Map a panoscope_core PortalError to the goat-cli error namespace
/// so the cargo-style render + stable exit code apply uniformly.
pub(super) fn map_portal_err(e: PortalError) -> anyhow::Error {
    // One status match covers the bare and body-carrying refusal shapes
    // alike (`PortalError::http_status`), so the two variants cannot drift
    // apart. The curated guidance explains the refusal class; the server's
    // own message — when the body carried one — rides along under it
    // instead of replacing it.
    if let Some(status) = e.http_status() {
        let server = server_line(&e);
        // 401 vs 403 are different problems, so don't collapse them. 401 =
        // the credential itself is bad (expired/garbage) → re-onboard. 403 =
        // the credential is valid but not authorized for this action — the
        // operator-only path: nothing was changed, and re-onboarding the same
        // identity won't help. Surfacing 403 as "token expired; re-onboard"
        // (the old behavior) sent non-admins down a dead-end.
        let err = match status.as_u16() {
            401 => GoatError::TokenInvalid {
                hint: format!(
                    "portal HTTP {status} — token expired or revoked; re-onboard.{server}"
                ),
            },
            // A non-operator key named somebody else's handle on the
            // operator-key self door (#1373). Still E_NOT_ADMIN, but the
            // generic "you hold no operator key" text below would tell that
            // person the wrong story.
            403 if e.error_code() == Some("self_only") => GoatError::NotAdmin {
                hint: format!(
                    "portal denied this request (HTTP {status}).\n\
                     \n\
                     A non-operator key may request an operator key only for ITSELF: omit the \
                     handle (it defaults to your own resolved identity) or pass your own handle, \
                     never somebody else's. To request one on behalf of somebody else, that has \
                     to come from an operator (op_) key — see `goat explain E_NOT_ADMIN`.{server}"
                ),
            },
            // ADR 1182: only a platform admin holds an operator key.
            403 if e.error_code() == Some("not_platform_admin") => GoatError::NotAdmin {
                hint: format!(
                    "the console refused this (HTTP {status} not_platform_admin).\n\
                     \n\
                     Only a platform admin holds an operator key (ADR 1182). Your slot key \
                     already covers reading the portal and everything on your own slot. If you \
                     were promoted and the console still records a lower level, an admin runs \
                     `goat admin platform-admin grant <your email> --env <goatnet>`, which \
                     records the level and requests and approves the key for you. Nothing was \
                     changed.{server}"
                ),
            },
            403 if e.error_code() == Some("unbound_principal") => {
                return unbound_principal_refusal(status, &server, GENERIC_OPERATOR_KEY_PAGE);
            }
            400 if e.error_code() == Some("missing_reason") => {
                return anyhow::anyhow!(
                    "the console wants to know what admin task needs the key (HTTP {status} \
                     missing_reason).{server}\n  hint: re-run with --reason \"...\" naming \
                     the task."
                );
            }
            // The request named a slot this key neither owns nor custodies.
            // An operator key is NOT the fix and must not be suggested: it is
            // whole-goatnet administration, and pointing participants at it is
            // how operator keys spread.
            403 if e.server_message().is_some_and(|m| m.contains("scope mismatch")) => {
                GoatError::SlotScope {
                    hint: format!(
                        "the portal refused this because the slot in the request is not this profile's slot (HTTP {status}). Nothing was changed.\n\
                         \n\
                         The first segment of a topic key is the slot that owns it. `goat profile \
                         show` prints this profile's slot as its publish anchor.\n\
                         \x20 - a machine (yours or your org's): it has its own slot and key. \
                         Use its bundle as a second context (`goat profile init <machine .cbor> \
                         --context <machine>`) and run the command there.\n\
                         \x20 - someone else's slot: ask that slot's owner or custodian.\n\
                         An operator key does not belong in this fix.{server}"
                    ),
                }
            }
            // One shared text for every operator-gated portal 403 — see
            // errors::operator_403_hint. This used to say only that operator
            // credentials are required, which sends the reader nowhere.
            403 => GoatError::NotAdmin {
                hint: format!(
                    "{}{server}",
                    crate::errors::operator_403_hint("this action", status)
                ),
            },
            // 409 is an ANSWER, not a failure to reach anything. Rendering it
            // as E_PORTAL_UNREACHABLE sent an operator looking for a network
            // problem when the portal had in fact told them the thing they
            // asked for had already happened, as on `operator-key
            // collect`, where the second attempt is the designed behaviour
            // rather than a fault.
            409 => {
                return anyhow::anyhow!(
                    "the portal refused this as a conflict (HTTP {status}) — the request is not \
                     in the state this action needs. Nothing was changed. For `operator-key \
                     collect` this means the key was already delivered once: it cannot be shown \
                     again, so revoke it and request a new one.{server}"
                );
            }
            // Any other status is an ANSWER too (#2087): a 404, a 429 or a
            // 5xx names what the portal objected to, with the status and the
            // request path, never "unreachable".
            _ => crate::errors::portal_answer(status.as_u16(), e.path(), e.retry_after(), &server),
        };
        return err.into();
    }
    let err = match &e {
        PortalError::Disabled => GoatError::ProfileMissing {
            context: "default".into(),
            hint: "portal client disabled (empty endpoint/token) — re-run `goat profile init`".into(),
        },
        // Both HTTP shapes were handled by the status dispatch above, so this
        // arm is dead in practice — but the invariant holding it up spans two
        // crates (it is only true while panoscope-core's `http_status()`
        // returns `Some` for exactly these variants). A third HTTP-ish variant,
        // or a conditional `http_status()`, would turn an operator's refusal
        // into a panic and a backtrace. In a path whose whole job is explaining
        // refusals clearly, that is the worst available failure mode, so fall
        // back to the same error the dispatch's own catch-all produces.
        PortalError::Http { status, .. } | PortalError::HttpWithBody { status, .. } => {
            crate::errors::portal_answer(status.as_u16(), e.path(), e.retry_after(), &server_line(&e))
        }
        PortalError::NotFound { path } => crate::errors::portal_answer(404, path, None, ""),
        PortalError::Transient(s) => GoatError::PortalUnreachable {
            endpoint: "portal".into(),
            hint: format!("portal transient: {s}"),
        },
        // The portal answered 2xx but the body didn't deserialize — a
        // response-contract mismatch, not an unreachable portal. Surface it
        // honestly so the operator upgrades goat instead of chasing a
        // network fault (the `goat schema show` decode break was this).
        PortalError::Decode(s) => GoatError::PortalBadResponse {
            hint: format!(
                "the portal is reachable and answered, but this goat build could not decode \
                 the response ({s}). This is almost always a portal/CLI version skew — update \
                 goat (re-install from `get.<site>.*`, or `goat doctor` shows if you're behind)."
            ),
        },
    };
    err.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_portal_err_401_to_token_invalid() {
        let e = PortalError::status(reqwest::StatusCode::UNAUTHORIZED);
        let mapped: anyhow::Error = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_TOKEN_INVALID");
    }

    #[test]
    fn map_portal_err_403_to_not_admin() {
        // 403 is authorization, not authentication: a valid-but-unscoped
        // credential. It must NOT collapse to E_TOKEN_INVALID (which would
        // tell a non-admin to re-onboard — a dead end).
        let e = PortalError::status(reqwest::StatusCode::FORBIDDEN);
        let mapped: anyhow::Error = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_NOT_ADMIN");
    }

    #[test]
    fn map_portal_err_self_only_names_the_real_reason() {
        // A non-operator key that named someone else's handle on
        // `operator-key request` (#1373's self-request door) is still an
        // E_NOT_ADMIN — but the hint must say "for yourself only", not the
        // generic "you hold no operator key at all", or the message tells a
        // person who genuinely doesn't have an operator key the wrong story.
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: Some("self_only".into()),
            message: "a non-operator key may only request an operator key for itself".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped: anyhow::Error = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_NOT_ADMIN");
        let hint = g.hint();
        assert!(
            hint.contains("only for ITSELF"),
            "hint should explain the self-only rule, got: {hint}"
        );
        assert!(
            !hint.contains("this profile is using a different token"),
            "hint should not fall back to the generic no-key-at-all message: {hint}"
        );
    }

    fn zagros_portal() -> panoscope_core::portal::PortalClient {
        panoscope_core::portal::PortalClient::new(
            "https://portal.zagros.netbird.zagros.example".into(),
            "k".into(),
        )
        .with_onboard_endpoint("https://onboard.zagros.netbird.zagros.example")
    }

    fn refused(status: u16, code: &str, message: &str) -> PortalError {
        PortalError::HttpWithBody {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            code: Some(code.into()),
            message: message.into(),
            path: String::new(),
            retry_after: None,
        }
    }

    #[test]
    fn console_page_derives_from_the_portal_host() {
        assert_eq!(
            console_operator_key_page(Some("https://portal.zagros.netbird.zagros.example"), None),
            "https://admin.auth.zagros.netbird.zagros.example/operator-key"
        );
        // The onboard host is the second choice, and a port does not leak in.
        assert_eq!(
            console_operator_key_page(
                Some("http://127.0.0.1:8090"),
                Some("https://onboard.zagros-dev.examplebird.zagros-dev.example:443/")
            ),
            "https://admin.auth.zagros-dev.examplebird.zagros-dev.example/operator-key"
        );
        // Neither shape: the generic form, never a guessed host.
        let generic = console_operator_key_page(Some("http://127.0.0.1:8090"), None);
        assert!(generic.contains("admin.auth.<site_id>.<zone>/operator-key"), "{generic}");
    }

    #[test]
    fn console_page_host_match_ignores_case() {
        // GOAT_ONBOARD_ENDPOINT is typed by hand. A capitalised scheme or host
        // names the same goatnet and must not fall back to the generic page.
        assert_eq!(
            console_operator_key_page(
                Some("http://127.0.0.1:8090"),
                Some("HTTPS://Onboard.ZAGROS.netbird.zagros.example/")
            ),
            "https://admin.auth.zagros.netbird.zagros.example/operator-key"
        );
        assert_eq!(
            console_operator_key_page(Some(" https://PORTAL.zagros.netbird.zagros.example:443 "), None),
            "https://admin.auth.zagros.netbird.zagros.example/operator-key"
        );
    }

    #[test]
    fn map_operator_key_err_operator_only_names_both_doors_and_the_page() {
        let mapped = map_operator_key_err(&zagros_portal(), refused(403, "operator_only", "operator-only action"));
        let g = mapped.downcast_ref::<GoatError>().expect("stays E_NOT_ADMIN");
        assert_eq!(g.code(), "E_NOT_ADMIN");
        let hint = g.hint();
        assert!(hint.contains("request (no handle), collect <id>"), "{hint}");
        assert!(hint.contains("--context <env>-op"), "{hint}");
        assert!(
            hint.contains("https://admin.auth.zagros.netbird.zagros.example/operator-key"),
            "{hint}"
        );
    }

    #[test]
    fn map_onboard_err_operator_only_keeps_the_shared_hint() {
        // The console's org-registry gate (requireOrgOperator) answers the same
        // 403 operator_only as the operator-key door. Every caller except the
        // operator-key verbs maps through here, and none of them may be told
        // about operator-key subcommands or the operator-key page.
        let body = refused(403, "operator_only", "org-registry mutations require an operator key");
        let mapped = map_onboard_err(&zagros_portal(), body);
        let g = mapped.downcast_ref::<GoatError>().expect("stays E_NOT_ADMIN");
        assert_eq!(g.code(), "E_NOT_ADMIN");
        let hint = g.hint();
        assert!(hint.contains("org-registry mutations require an operator key"), "{hint}");
        assert!(hint.contains("needs your OPERATOR key"), "{hint}");
        assert!(!hint.contains("Which operator-key verbs"), "{hint}");
        assert!(!hint.contains("admin.auth.zagros"), "{hint}");
    }

    #[test]
    fn map_operator_key_err_unbound_names_this_goatnets_console_page() {
        let mapped = map_operator_key_err(&zagros_portal(), refused(403, "unbound_principal", "no person"));
        assert!(mapped.downcast_ref::<GoatError>().is_none());
        let rendered = format!("{mapped:#}");
        assert!(
            rendered.contains("https://admin.auth.zagros.netbird.zagros.example/operator-key"),
            "{rendered}"
        );
        // Without a portal in hand the same code keeps the generic page.
        let generic = format!("{:#}", map_portal_err(refused(403, "unbound_principal", "no person")));
        assert!(generic.contains("admin.auth.<site_id>.<zone>/operator-key"), "{generic}");
    }

    #[test]
    fn map_operator_key_err_missing_handle_points_at_the_plain_profile() {
        let mapped = map_operator_key_err(&zagros_portal(), refused(400, "missing_handle", "handle is required"));
        assert!(mapped.downcast_ref::<GoatError>().is_none());
        let rendered = format!("{mapped:#}");
        assert!(rendered.contains("operator key, which files for a NAMED person"), "{rendered}");
        assert!(rendered.contains("plain profile"), "{rendered}");
    }

    #[test]
    fn map_portal_err_not_platform_admin_names_the_slot_key_and_the_grant_path() {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: Some("not_platform_admin".into()),
            message: "this identity's recorded level is iam-admin".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped: anyhow::Error = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_NOT_ADMIN");
        let hint = g.hint();
        assert!(hint.contains("recorded level is iam-admin"), "{hint}");
        assert!(hint.contains("slot key"), "{hint}");
        assert!(hint.contains("platform-admin grant"), "{hint}");
    }

    #[test]
    fn map_portal_err_unbound_and_missing_reason_say_what_to_do() {
        let unbound = map_portal_err(PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: Some("unbound_principal".into()),
            message: "this token does not resolve to an onboarded person".into(),
            path: String::new(),
            retry_after: None,
        });
        assert!(unbound.downcast_ref::<GoatError>().is_none());
        let rendered = format!("{unbound:#}");
        assert!(rendered.contains("finish your onboarding"), "{rendered}");
        assert!(rendered.contains("/operator-key"), "{rendered}");

        let reason = map_portal_err(PortalError::HttpWithBody {
            status: reqwest::StatusCode::BAD_REQUEST,
            code: Some("missing_reason".into()),
            message: "say what admin task needs it".into(),
            path: String::new(),
            retry_after: None,
        });
        assert!(reason.downcast_ref::<GoatError>().is_none());
        let rendered = format!("{reason:#}");
        assert!(rendered.contains("--reason"), "{rendered}");
    }

    #[test]
    fn map_portal_err_http_body_409_carries_the_real_message() {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::CONFLICT,
            code: Some("already_delivered".into()),
            message: "that key was already delivered once; revoke it and request a new one".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped: anyhow::Error = map_portal_err(e);
        let msg = format!("{mapped:#}");
        assert!(
            msg.contains("already delivered once"),
            "conflict message should surface the portal's real explanation, got: {msg}"
        );
    }

    #[test]
    fn map_portal_err_http_body_unparsed_code_falls_back_to_generic_admin_hint() {
        // A 403 whose code isn't one of the specifically-handled ones still
        // renders as E_NOT_ADMIN with a useful hint, not a raw/opaque error.
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: Some("some_future_code".into()),
            message: "some future refusal".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped: anyhow::Error = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_NOT_ADMIN");
    }

    #[test]
    fn map_portal_err_disabled_to_profile_missing() {
        let mapped: anyhow::Error = map_portal_err(PortalError::Disabled);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_PROFILE_MISSING");
    }

    #[test]
    fn map_portal_err_transient_to_portal_unreachable() {
        let mapped: anyhow::Error = map_portal_err(PortalError::Transient("dns".into()));
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_PORTAL_UNREACHABLE");
    }

    // A status the portal ANSWERED is never E_PORTAL_UNREACHABLE (#2087):
    // each carries its status and the request path, and a 429 its Retry-After.
    #[test]
    fn map_portal_err_answers_keep_status_path_and_retry_after() {
        let answer = |status: u16, retry_after: Option<&str>| PortalError::Http {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            path: "/api/orgs/acme/topics".into(),
            retry_after: retry_after.map(str::to_string),
        };
        for (e, code, exit) in [
            (answer(404, None), "E_PORTAL_NOT_FOUND", 7),
            (answer(429, Some("30")), "E_PORTAL_RATE_LIMITED", 8),
            (answer(502, None), "E_PORTAL_SERVER_ERROR", 9),
            (answer(400, None), "E_PORTAL_REFUSED", 4),
            (
                PortalError::NotFound {
                    path: "/api/orgs/acme/topics".into(),
                },
                "E_PORTAL_NOT_FOUND",
                7,
            ),
        ] {
            let mapped: anyhow::Error = map_portal_err(e);
            let g = mapped.downcast_ref::<GoatError>().expect("downcast");
            assert_eq!((g.code(), g.exit_code()), (code, exit));
            let shown = crate::errors::DisplayHuman(g).to_string();
            assert!(shown.contains("/api/orgs/acme/topics"), "{shown}");
            assert!(!shown.contains("unreachable"), "{shown}");
            if code == "E_PORTAL_RATE_LIMITED" {
                assert!(shown.contains("retry after 30 s"), "{shown}");
            }
        }
    }

    // A 2xx body the CLI can't parse must NOT masquerade as "unreachable" —
    // it maps to E_PORTAL_BAD_RESPONSE with an upgrade-goat hint. Distinct
    // exit-code parity with the other portal-tier failures is preserved.
    #[test]
    fn map_portal_err_decode_to_portal_bad_response() {
        let mapped: anyhow::Error = map_portal_err(PortalError::Decode("proto_source: expected string".into()));
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_PORTAL_BAD_RESPONSE");
        assert_ne!(g.code(), "E_PORTAL_UNREACHABLE");
    }

    #[test]
    fn onboard_endpoint_hint_present_on_fallback() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        );
        let mapped: anyhow::Error = map_onboard_err(&portal, PortalError::Transient("dns".into()));
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_PORTAL_UNREACHABLE");
        let rendered = format!("{display}", display = crate::errors::DisplayHuman(g));
        assert!(rendered.contains("GOAT_ONBOARD_ENDPOINT"));
        assert!(rendered.contains("bundle"));
    }

    #[test]
    fn onboard_endpoint_hint_absent_when_configured() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        )
        .with_onboard_endpoint("https://onboard.example");
        let mapped: anyhow::Error = map_onboard_err(&portal, PortalError::Transient("dns".into()));
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_PORTAL_UNREACHABLE");
        let rendered = format!("{display}", display = crate::errors::DisplayHuman(g));
        assert!(!rendered.contains("GOAT_ONBOARD_ENDPOINT"));
    }

    #[test]
    fn onboard_endpoint_hint_on_non_goaterror_arm() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        );
        // A 409 is the remaining refusal mapped to a plain error, not a
        // GoatError (a 404 is E_PORTAL_NOT_FOUND since #2087).
        let base: anyhow::Error = map_portal_err(PortalError::status(reqwest::StatusCode::CONFLICT));
        let mapped: anyhow::Error = append_onboard_hint(&portal, base);
        assert!(mapped.downcast_ref::<GoatError>().is_none());
        let rendered = format!("{mapped:#}");
        assert!(rendered.contains("HTTP 409"));
        assert!(rendered.contains("GOAT_ONBOARD_ENDPOINT"));
    }

    #[test]
    fn onboard_endpoint_hint_keeps_exit_code() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        );
        let base: anyhow::Error = map_portal_err(PortalError::status(reqwest::StatusCode::UNAUTHORIZED));
        let base_goat = base.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(base_goat.code(), "E_TOKEN_INVALID");
        let base_exit = base_goat.exit_code();

        let mapped: anyhow::Error = map_onboard_err(
            &portal,
            PortalError::status(reqwest::StatusCode::UNAUTHORIZED),
        );
        let mapped_goat = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(mapped_goat.code(), "E_TOKEN_INVALID");
        assert_eq!(mapped_goat.exit_code(), base_exit);
    }

    // Body-carrying twins must land on the SAME codes as the bare statuses
    // (one status dispatch covers both shapes), with the server's message
    // composed into the hint UNDER the curated guidance — never replacing it.
    #[test]
    fn map_portal_err_with_body_401_same_code_plus_server_message() {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::UNAUTHORIZED,
            code: None,
            message: "token revoked".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_TOKEN_INVALID");
        let dbg = format!("{g:?}");
        assert!(dbg.contains("re-onboard"), "{dbg}");
        assert!(dbg.contains("token revoked"), "{dbg}");
    }

    #[test]
    fn map_portal_err_with_body_403_same_code_plus_server_message() {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: None,
            message: "custodian mismatch".into(),
            path: String::new(),
            retry_after: None,
        };
        let mapped = map_portal_err(e);
        let g = mapped.downcast_ref::<GoatError>().expect("downcast");
        assert_eq!(g.code(), "E_NOT_ADMIN");
        let dbg = format!("{g:?}");
        // The curated F-410 operator-key guidance, not the pre-F-410 text it
        // replaced — a rebase that quietly restores the old string would show
        // up here rather than in production.
        assert!(dbg.contains("OPERATOR key"), "{dbg}");
        assert!(dbg.contains("custodian mismatch"), "{dbg}");
    }

    /// The server's message is a follow-on stanza, identical in shape to the
    /// one `machine.rs` renders — one concept, one rendering, so what the
    /// operator sees does not depend on which verb refused them.
    #[test]
    fn server_message_renders_as_its_own_stanza() {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: None,
            message: "custodian mismatch".into(),
            path: String::new(),
            retry_after: None,
        };
        assert_eq!(server_line(&e), "\n  server: custodian mismatch");
        assert_eq!(
            server_line(&PortalError::status(reqwest::StatusCode::FORBIDDEN)),
            ""
        );
    }
}

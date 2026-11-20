// SPDX-License-Identifier: Apache-2.0
//
// `goat pod` — a pod's local authority over the pod's own API (ADR 1251,
// docs/design/pod-carriage-and-provenance.md §4, §5, §7.4). It works when the
// pod is cut off from the core: the CLI talks only to the pod, signs in with
// the active profile's client certificate, and the pod checks it against its
// cached CA roots and the `local_authority` its signed rules name. Every
// change carries a reason, and every grant an end date (a revoke is at once);
// the pod keeps them in its signed,
// append-only ledger and reports them for ratification on reconnection
// (`goat carriage pending`).
//
// Fieldkit Gatepost offers the same six actions over the same API.

use super::carriage::{
    carriage_refusal, list_of, nonempty_reason, parse_until, rule_id, shown_time, slot,
};
use crate::errors::GoatError;
use crate::i18n::LANGUAGE_LOADER;
use crate::output::with_spinner;
use crate::profile::Profile;
use clap::{Args, Subcommand};
use i18n_embed_fl::fl;
use reqwest::Method;
use serde_json::{json, Value};

#[derive(Args, Debug)]
pub struct PodArgs {
    /// The pod's API, e.g. `https://pod.example:8443`. Default: $GOAT_POD_URL,
    /// then `pod_endpoint` in the active context's profile.toml.
    #[arg(long, global = true, env = "GOAT_POD_URL")]
    pub pod: Option<String>,
    /// The pod's 32-hex slot. The pod serves its own goat certificate,
    /// which names it by slot, not by the address dialled; goat checks the
    /// certificate chains to this context's CA and that its CN is this slot.
    /// Default: $GOAT_POD_SLOT, then `pod_slot` in profile.toml.
    #[arg(long, global = true, env = "GOAT_POD_SLOT")]
    pub pod_slot: Option<String>,
    #[command(subcommand)]
    pub op: PodCmd,
}

#[derive(Subcommand, Debug)]
pub enum PodCmd {
    /// Connected or cut off, rule generation and its age, sources connected,
    /// and carriage health per source.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// The pod's rules: central and local, with end dates and origin.
    Rules {
        #[command(subcommand)]
        op: PodRulesCmd,
    },
    /// Local changes waiting for an org lead to ratify or let lapse.
    Pending {
        /// Emit `{"pending":[…]}` as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PodRulesCmd {
    /// Effective rules, central and local, with end dates and origin.
    Ls {
        /// Emit `{"rules":[…]}` as JSON.
        #[arg(long)]
        json: bool,
    },
    /// A local rule letting a source use this pod until a set time.
    Allow {
        /// The source's 32-hex slot.
        source: String,
        /// End: an RFC 3339 time, or a length from now (`4h`, `24h`, `72h`).
        /// No later than the pod's longest rule length.
        #[arg(long)]
        until: String,
        /// Why; recorded in the pod's ledger.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a rule's end date.
    Extend {
        /// The rule id.
        rule: String,
        /// New end: an RFC 3339 time, or a length from now.
        #[arg(long)]
        until: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        json: bool,
    },
    /// Cut a source off this pod at once.
    Revoke {
        /// The source's 32-hex slot.
        source: String,
        /// Why; recorded in the pod's ledger.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        json: bool,
    },
    /// Bring back an archived rule on this pod.
    Reactivate {
        /// The rule id.
        rule: String,
        /// New end: an RFC 3339 time, or a length from now.
        #[arg(long)]
        until: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        json: bool,
    },
}

/// One call to the pod API.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PodCall {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
    pub answer: PodAnswer,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PodAnswer {
    Status,
    Rules,
    Rule,
    Pending,
}

pub async fn run(ctx: &super::Ctx, args: PodArgs) -> anyhow::Result<()> {
    let base = pod_url(args.pod.as_deref(), &ctx.profile)?;
    let http = pod_client(&ctx.profile, &base, args.pod_slot.as_deref())?;
    let (label, call, json) = request_for(args.op, chrono::Utc::now())?;
    let v = with_spinner(&ctx.output, label, send(&http, &base, &call)).await?;
    if json {
        println!("{}", json_output(&call, &v));
    } else {
        print!("{}", render(&call, &v));
    }
    Ok(())
}

/// `--pod` / `$GOAT_POD_URL` (clap merges both), then the profile's
/// `pod_endpoint`.
fn pod_url(flag: Option<&str>, profile: &Profile) -> anyhow::Result<String> {
    let url = flag
        .map(str::to_string)
        .or_else(|| profile.profile_field("pod_endpoint"))
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    match url {
        Some(u) if u.starts_with("https://") || u.starts_with("http://") => Ok(u),
        Some(u) => Err(GoatError::PodNotNamed {
            hint: format!("pod URL `{u}` must start with https://"),
        }
        .into()),
        None => Err(GoatError::PodNotNamed {
            hint: format!(
                "pass --pod https://<pod>:<port>, export GOAT_POD_URL, or set pod_endpoint in {}",
                profile.profile_toml_path().display()
            ),
        }
        .into()),
    }
}

/// An HTTPS client that signs in with the active context's certificate and
/// key, trusts only its CA roots, and accepts the pod's certificate when it
/// chains to them and its CN is the pod's slot (`<slot>` or `<slot>.g<N>`).
/// The address dialled is not checked against the certificate: a pod is
/// reached by 127.0.0.1 or a mesh address, and its leaf names its slot.
fn pod_client(
    profile: &Profile,
    base: &str,
    pod_slot: Option<&str>,
) -> anyhow::Result<reqwest::Client> {
    let timeout = std::time::Duration::from_secs(15);
    let b = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout);
    if !base.starts_with("https://") {
        return Ok(b.build()?);
    }
    let want = pod_slot
        .map(str::to_string)
        .or_else(|| profile.profile_field("pod_slot"))
        .ok_or_else(|| GoatError::PodNotNamed {
            hint: format!(
                "name the pod's slot: pass --pod-slot <32-hex>, export GOAT_POD_SLOT, or set \
                 pod_slot in {} (goat checks the pod's certificate names that slot)",
                profile.profile_toml_path().display()
            ),
        })?;
    let want = slot(&want, "pod")?;
    profile.require_mesh_ready()?;
    let cfg = panoscope_core::pinned_tls::client_config(
        &std::fs::read(profile.ca_roots_path())?,
        &std::fs::read(profile.cert_path())?,
        &std::fs::read(profile.key_path())?,
        leaf_is_pod(want),
    )?;
    Ok(b.use_preconfigured_tls(cfg).build()?)
}

/// Accept a leaf whose subject CN is the pod's slot, bare or with its
/// generation suffix (`<slot>.g<N>`).
pub(crate) fn leaf_is_pod(want: String) -> panoscope_core::pinned_tls::LeafCheck {
    std::sync::Arc::new(move |der: &[u8]| {
        let (_, cert) = x509_parser::parse_x509_certificate(der)
            .map_err(|e| format!("pod certificate unreadable: {e}"))?;
        let cn = cert
            .subject()
            .iter_common_name()
            .next()
            .and_then(|c| c.as_str().ok())
            .unwrap_or("")
            .to_string();
        let ok = cn == want
            || cn
                .strip_prefix(want.as_str())
                .and_then(|r| r.strip_prefix(".g"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if ok {
            Ok(())
        } else {
            Err(format!(
                "the server's certificate is `{cn}`, not pod `{want}`: wrong address, or not the pod"
            ))
        }
    })
}

pub(crate) fn request_for(
    op: PodCmd,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<(&'static str, PodCall, bool)> {
    let call = |method, path: String, body, answer, subject: String| PodCall {
        method,
        path,
        body,
        answer,
        subject,
    };
    Ok(match op {
        PodCmd::Status { json } => (
            "reading pod status",
            call(
                Method::GET,
                "/v1/status".into(),
                None,
                PodAnswer::Status,
                String::new(),
            ),
            json,
        ),
        PodCmd::Pending { json } => (
            "listing pending local changes",
            call(
                Method::GET,
                "/v1/pending".into(),
                None,
                PodAnswer::Pending,
                String::new(),
            ),
            json,
        ),
        PodCmd::Rules { op } => match op {
            PodRulesCmd::Ls { json } => (
                "listing pod rules",
                call(
                    Method::GET,
                    "/v1/rules".into(),
                    None,
                    PodAnswer::Rules,
                    String::new(),
                ),
                json,
            ),
            PodRulesCmd::Allow {
                source,
                until,
                reason,
                json,
            } => {
                let s = slot(&source, "source")?;
                (
                    "adding local rule",
                    call(
                        Method::POST,
                        "/v1/rules".into(),
                        Some(json!({"source": s, "until": parse_until(&until, now)?,
                                    "reason": nonempty_reason(&reason)?})),
                        PodAnswer::Rule,
                        s,
                    ),
                    json,
                )
            }
            PodRulesCmd::Extend {
                rule,
                until,
                reason,
                json,
            } => {
                let id = rule_id(&rule)?;
                (
                    "extending pod rule",
                    call(
                        Method::POST,
                        format!("/v1/rules/{id}/extend"),
                        Some(json!({"until": parse_until(&until, now)?,
                                    "reason": nonempty_reason(&reason)?})),
                        PodAnswer::Rule,
                        id,
                    ),
                    json,
                )
            }
            PodRulesCmd::Revoke {
                source,
                reason,
                json,
            } => {
                let s = slot(&source, "source")?;
                // A revoke takes effect at once and carries no end date.
                let body = json!({"reason": nonempty_reason(&reason)?});
                (
                    "revoking source on pod",
                    call(
                        Method::POST,
                        format!("/v1/sources/{s}/revoke"),
                        Some(body),
                        PodAnswer::Rule,
                        s,
                    ),
                    json,
                )
            }
            PodRulesCmd::Reactivate {
                rule,
                until,
                reason,
                json,
            } => {
                let id = rule_id(&rule)?;
                (
                    "reactivating pod rule",
                    call(
                        Method::POST,
                        format!("/v1/rules/{id}/reactivate"),
                        Some(json!({"until": parse_until(&until, now)?,
                                    "reason": nonempty_reason(&reason)?})),
                        PodAnswer::Rule,
                        id,
                    ),
                    json,
                )
            }
        },
    })
}

/// Send one call. A refusal the pod names (`error.code`) or a 403 becomes an
/// `E_CARRIAGE_*` error; any other failure says the pod answered and how.
async fn send(http: &reqwest::Client, base: &str, call: &PodCall) -> anyhow::Result<Value> {
    let url = format!("{base}{}", call.path);
    let mut req = http.request(call.method.clone(), &url);
    if let Some(b) = &call.body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| {
        // reqwest's own line is generic; the TLS refusal (wrong slot, wrong
        // CA) is in its causes.
        let mut why = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(c) = src {
            why = format!("{why}: {c}");
            src = c.source();
        }
        anyhow::Error::from(GoatError::PodUnreachable {
            pod: base.to_string(),
            hint: format!(
                "{why}. Check the URL, that this machine reaches the pod (on the pod's network \
                 or the mesh), that --pod-slot names this pod, and that the pod's certificate \
                 chains to this context's CA roots"
            ),
        })
    })?;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default();
    let v: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    if status.is_success() {
        return Ok(v);
    }
    let code = v.pointer("/error/code").and_then(Value::as_str);
    // The pod's refusal names itself either as `error.message` or as
    // `error.what` / `error.why` (the shape Gatepost renders).
    let text = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut message = match v.get("error") {
        Some(Value::String(m)) => m.clone(),
        _ => match text("/error/message") {
            m if m.is_empty() => text("/error/what"),
            m => m,
        },
    };
    let why = text("/error/why");
    if !why.is_empty() && !message.contains(&why) {
        message = if message.is_empty() {
            why
        } else {
            format!("{message} — {why}")
        };
    }
    // The pod's own fix rides with its message on every refusal: its words
    // name this pod's commands (`goat pod rules ls`), where a catalog hint
    // also serves the portal.
    let fix = text("/error/fix");
    let mut server = if message.is_empty() {
        String::new()
    } else {
        format!("\n  pod: {}", message.chars().take(400).collect::<String>())
    };
    if !fix.is_empty() {
        server += &format!("\n  fix: {}", fix.chars().take(400).collect::<String>());
    }
    if let Some(g) = carriage_refusal(code, status.as_u16(), &call.subject, &server) {
        return Err(g.into());
    }
    if code == Some("not_signed_in") || status.as_u16() == 401 {
        return Err(GoatError::PodNotSignedIn {
            hint: format!(
                "the pod signs people in by client certificate. Use a context that holds a \
                 goat certificate from this goatnet (`goat context ls`), and reach the pod over \
                 https:// so the certificate is sent.{server}"
            ),
        }
        .into());
    }
    Err(GoatError::PodRefused {
        status: status.as_u16(),
        path: call.path.clone(),
        hint: format!(
            "the pod is up and refused {} {}; its own words follow.{server}",
            call.method, call.path
        ),
    }
    .into())
}

fn json_output(call: &PodCall, v: &Value) -> Value {
    match call.answer {
        PodAnswer::Rules => json!({"rules": list_of(v, "rules"),
                                   "revocations": list_of(v, "revocations")}),
        PodAnswer::Pending => json!({"pending": list_of(v, "pending")}),
        _ => v.clone(),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// A count the pod may not know (`null` = not measured): shown as `unknown`,
/// never as 0.
fn count(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn render(call: &PodCall, v: &Value) -> String {
    let mut out = String::new();
    match call.answer {
        PodAnswer::Status => {
            let generation = v.get("generation").and_then(Value::as_i64).unwrap_or(0);
            let age = v
                .get("generation_age_seconds")
                .and_then(Value::as_i64)
                .map(human_age)
                .unwrap_or_else(|| "-".into());
            let state = match s(v, "state") {
                "" if v.get("connected").and_then(Value::as_bool) == Some(true) => "connected",
                "" => "cut off",
                st => st,
            };
            out += &fl!(
                LANGUAGE_LOADER,
                "pod-status-line",
                pod = s(v, "pod_slot").to_string(),
                state = state.to_string(),
                generation = generation.to_string(),
                age = age,
                carried = count(v, "sources_carried"),
                connected = count(v, "sources_connected"),
                pending = count(v, "pending")
            );
            out.push('\n');
            if let Some(you) = v.get("you") {
                let may = you.get("may_change").and_then(Value::as_bool) == Some(true);
                out += &if may {
                    fl!(
                        LANGUAGE_LOADER,
                        "pod-status-you-may-change",
                        identity = s(you, "identity").to_string()
                    )
                } else {
                    fl!(
                        LANGUAGE_LOADER,
                        "pod-status-you-read-only",
                        identity = s(you, "identity").to_string()
                    )
                };
                out.push('\n');
            }
            for src in v
                .get("sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let connected = match src.get("connected").and_then(Value::as_bool) {
                    Some(true) => "connected",
                    Some(false) => "not connected",
                    None => "connection unknown",
                };
                out += &format!(
                    "  {:<34} {:<9} {:<8} {:<18} until {}  {}\n",
                    s(src, "slot"),
                    s(src, "carriage"),
                    s(src, "origin"),
                    connected,
                    shown_time(src.get("until").unwrap_or(&Value::Null)),
                    s(src, "rule_id")
                );
            }
        }
        PodAnswer::Rules => {
            let rules = list_of(v, "rules");
            if rules.is_empty() {
                out += &fl!(LANGUAGE_LOADER, "carriage-no-rules");
                out.push('\n');
            }
            for r in rules {
                out += &rule_line(&r);
            }
            for r in list_of(v, "revocations") {
                out += &format!(
                    "revoked        {}  until {}  ({})\n",
                    s(&r, "source"),
                    shown_time(r.get("until").unwrap_or(&Value::Null)),
                    s(&r, "reason")
                );
            }
        }
        PodAnswer::Rule => {
            // A change answers `{"rule": …, "entry": …}`; a revoke's rule is
            // null; 202 adds `warning` (recorded, router rolled back).
            match v.get("rule").filter(|r| r.is_object()) {
                Some(r) => out += &rule_line(r),
                None if v.get("rule").is_some() => {
                    out += &fl!(
                        LANGUAGE_LOADER,
                        "pod-source-revoked",
                        source = call.subject.clone()
                    );
                    out.push('\n');
                }
                None => out += &rule_line(v),
            }
            let w = s(v, "warning");
            if !w.is_empty() {
                out += &format!("! {w}\n");
            }
        }
        PodAnswer::Pending => {
            let pending = list_of(v, "pending");
            if pending.is_empty() {
                out += &fl!(LANGUAGE_LOADER, "carriage-no-pending");
                out.push('\n');
            }
            for e in pending {
                let src = match s(&e, "source_slot") {
                    "" => s(&e, "source"),
                    x => x,
                };
                let who = match e.get("authority") {
                    Some(Value::Object(_)) => e["authority"]["identity"].as_str().unwrap_or(""),
                    Some(Value::String(a)) => a.as_str(),
                    _ => "",
                };
                out += &format!(
                    "{:<6} {:<10} {}  until {}  by {}  {}  ({})\n",
                    e.get("seq").map(|x| x.to_string()).unwrap_or_default(),
                    s(&e, "action"),
                    src,
                    shown_time(e.get("until").unwrap_or(&Value::Null)),
                    who,
                    s(&e, "ratification"),
                    s(&e, "reason")
                );
            }
        }
    }
    out
}

fn rule_line(r: &Value) -> String {
    let until = shown_time(r.get("until").unwrap_or(&Value::Null));
    let reason = s(r, "reason");
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!("  ({reason})")
    };
    let ratification = match s(r, "ratification") {
        "" => String::new(),
        x => format!("  ratification {x}"),
    };
    let source = match s(r, "source_slot") {
        "" => s(r, "source"),
        x => x,
    };
    format!(
        "{:<14} {:<8} {:<9} {}  until {until}  {}{ratification}{reason}\n",
        s(r, "id"),
        s(r, "origin"),
        s(r, "kind"),
        source,
        s(r, "state")
    )
}

fn human_age(secs: i64) -> String {
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 7200 => format!("{}m", s / 60),
        s if s < 172_800 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::GoatError;
    use chrono::TimeZone;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SRC: &str = "0123456789abcdef0123456789abcdef";

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 11, 19, 10, 0, 0)
            .unwrap()
    }

    fn rules(op: PodRulesCmd) -> PodCall {
        request_for(PodCmd::Rules { op }, now()).unwrap().1
    }

    #[test]
    fn every_verb_hits_the_pod_api_route_with_until_and_reason() {
        let c = rules(PodRulesCmd::Allow {
            source: SRC.into(),
            until: "4h".into(),
            reason: "radio relay".into(),
            json: false,
        });
        assert_eq!(
            (c.method.clone(), c.path.as_str()),
            (Method::POST, "/v1/rules")
        );
        assert_eq!(
            c.body.unwrap(),
            json!({"source": SRC, "until": "2026-11-19T14:00:00Z", "reason": "radio relay"})
        );
        let c = rules(PodRulesCmd::Extend {
            rule: "lr_2".into(),
            until: "24h".into(),
            reason: "r".into(),
            json: false,
        });
        assert_eq!(c.path, "/v1/rules/lr_2/extend");
        let c = rules(PodRulesCmd::Revoke {
            source: SRC.into(),
            reason: "lost".into(),
            json: false,
        });
        assert_eq!(c.path, format!("/v1/sources/{SRC}/revoke"));
        assert_eq!(c.body.unwrap(), json!({"reason": "lost"}));
        let c = rules(PodRulesCmd::Reactivate {
            rule: "cr_1".into(),
            until: "72h".into(),
            reason: "r".into(),
            json: false,
        });
        assert_eq!(c.path, "/v1/rules/cr_1/reactivate");
        let c = rules(PodRulesCmd::Ls { json: false });
        assert_eq!((c.method, c.path.as_str()), (Method::GET, "/v1/rules"));
        let (_, c, _) = request_for(PodCmd::Status { json: true }, now()).unwrap();
        assert_eq!(c.path, "/v1/status");
        let (_, c, _) = request_for(PodCmd::Pending { json: true }, now()).unwrap();
        assert_eq!(c.path, "/v1/pending");
    }

    #[test]
    fn a_change_without_a_reason_or_end_is_refused_locally() {
        let r = request_for(
            PodCmd::Rules {
                op: PodRulesCmd::Allow {
                    source: SRC.into(),
                    until: "4h".into(),
                    reason: " ".into(),
                    json: false,
                },
            },
            now(),
        );
        assert!(r.is_err());
        let r = request_for(
            PodCmd::Rules {
                op: PodRulesCmd::Allow {
                    source: SRC.into(),
                    until: "never".into(),
                    reason: "x".into(),
                    json: false,
                },
            },
            now(),
        );
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn status_renders_cut_off_and_sources() {
        let server = MockServer::start().await;
        let body = json!({"pod_slot": "fedcba9876543210fedcba9876543210", "state": "cut off",
                          "connected": false, "generation": 42, "generation_age_seconds": 7300,
                          "sources_carried": 1, "sources_connected": null, "pending": 2,
                          "you": {"identity": "test-pki:operator-1", "may_change": true},
                          "sources": [{"slot": SRC, "carried": true, "carriage": "expiring",
                                       "origin": "local", "rule_id": "lr_1",
                                       "until": "2026-11-20T10:00:00Z", "connected": null}]});
        Mock::given(method("GET"))
            .and(path("/v1/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;
        let (_, c, _) = request_for(PodCmd::Status { json: false }, now()).unwrap();
        let v = send(&reqwest::Client::new(), &server.uri(), &c)
            .await
            .unwrap();
        let text = render(&c, &v);
        assert!(text.contains("cut off"), "{text}");
        assert!(
            text.contains("generation 42") && text.contains("2h"),
            "{text}"
        );
        assert!(
            text.contains("connected unknown"),
            "null is unknown, not 0: {text}"
        );
        assert!(text.contains("2 local changes waiting"), "{text}");
        assert!(text.contains("may change its rules"), "{text}");
        assert!(
            text.contains(SRC) && text.contains("expiring") && text.contains("connection unknown"),
            "{text}"
        );
        assert_eq!(json_output(&c, &v), body);
    }

    #[tokio::test]
    async fn allow_posts_and_pod_refusals_map_to_catalog_codes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/rules"))
            .and(body_json(
                json!({"source": SRC, "until": "2026-11-29T10:00:00Z", "reason": "r"}),
            ))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
                "code": "carriage_until_too_far",
                "message": "until is past the longest rule length (7d)"}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/rules/cr_1/extend"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": {
                "code": "carriage_not_authorized", "what": "not a local authority",
                "why": "the rules name another identity", "fix": "ask them"}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/rules/cr_2/reactivate"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": {
                "code": "carriage_reactivation_window_passed", "message": "expired 200 days ago"}})))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let code = |e: anyhow::Error| e.downcast_ref::<GoatError>().unwrap().code();

        let c = rules(PodRulesCmd::Allow {
            source: SRC.into(),
            until: "10d".into(),
            reason: "r".into(),
            json: false,
        });
        let e = send(&http, &server.uri(), &c).await.unwrap_err();
        assert!(e
            .downcast_ref::<GoatError>()
            .unwrap()
            .hint()
            .contains("(7d)"));
        assert_eq!(code(e), "E_CARRIAGE_TOO_LONG");

        let c = rules(PodRulesCmd::Extend {
            rule: "cr_1".into(),
            until: "4h".into(),
            reason: "r".into(),
            json: false,
        });
        let e = send(&http, &server.uri(), &c).await.unwrap_err();
        assert!(e
            .downcast_ref::<GoatError>()
            .unwrap()
            .hint()
            .contains("not a local authority — the rules name another identity"));
        assert_eq!(code(e), "E_CARRIAGE_NOT_AUTHORIZED");

        let c = rules(PodRulesCmd::Reactivate {
            rule: "cr_2".into(),
            until: "4h".into(),
            reason: "r".into(),
            json: false,
        });
        let e = send(&http, &server.uri(), &c).await.unwrap_err();
        assert_eq!(code(e), "E_CARRIAGE_WINDOW_PASSED");
    }

    #[tokio::test]
    async fn rules_and_pending_render_and_normalize() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/rules"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"rules": [
                {"id": "cr_1", "origin": "central", "kind": "org", "source": SRC,
                 "source_slot": SRC, "until": null, "state": "active", "ratification": null},
                {"id": "lr_1", "origin": "local", "kind": "local", "source": SRC,
                 "until": "2026-11-20T10:00:00Z", "state": "active", "reason": "relay",
                 "ratification": "pending"}
            ], "revocations": [{"source": SRC, "until": null, "reason": "lost", "seq": 4}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"pending": []})))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = rules(PodRulesCmd::Ls { json: true });
        let v = send(&http, &server.uri(), &c).await.unwrap();
        let text = render(&c, &v);
        assert!(
            text.contains("central") && text.contains("until -"),
            "{text}"
        );
        assert!(
            text.contains("ratification pending") && text.contains("(relay)"),
            "{text}"
        );
        assert!(
            text.contains("revoked") && text.contains("(lost)"),
            "{text}"
        );
        let j = json_output(&c, &v);
        assert_eq!(j["rules"].as_array().unwrap().len(), 2);
        assert_eq!(j["revocations"].as_array().unwrap().len(), 1);

        let (_, c, _) = request_for(PodCmd::Pending { json: false }, now()).unwrap();
        let v = send(&http, &server.uri(), &c).await.unwrap();
        assert!(
            render(&c, &v).contains("nothing waiting"),
            "{}",
            render(&c, &v)
        );
    }

    #[tokio::test]
    async fn uncatalogued_pod_refusals_still_get_catalog_codes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/status"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": {
                "code": "not_signed_in", "message": "no client certificate",
                "what": "no client certificate"}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/rules"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
                "code": "invalid_request", "message": "source is not a slot",
                "what": "source is not a slot", "why": "a slot is 32 hex",
                "fix": "pass the 32-hex slot"}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/rules/cr_9/reactivate"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": {
                "code": "not_found", "message": "No rule cr_9 on this pod.",
                "fix": "List the rules with goat pod rules ls."}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/rules/cr_1/extend"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": {
                "code": "conflict", "message": "changed meanwhile"}})))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let g = |e: anyhow::Error| {
            let g = e.downcast_ref::<GoatError>().unwrap();
            (g.code(), g.hint().to_string())
        };
        let (_, c, _) = request_for(PodCmd::Status { json: false }, now()).unwrap();
        let (code, _) = g(send(&http, &server.uri(), &c).await.unwrap_err());
        assert_eq!(code, "E_POD_NOT_SIGNED_IN");
        let c = rules(PodRulesCmd::Allow {
            source: SRC.into(),
            until: "4h".into(),
            reason: "r".into(),
            json: false,
        });
        let (code, hint) = g(send(&http, &server.uri(), &c).await.unwrap_err());
        assert_eq!(code, "E_POD_REFUSED");
        assert!(
            hint.contains("source is not a slot — a slot is 32 hex"),
            "{hint}"
        );
        assert!(hint.contains("fix: pass the 32-hex slot"), "{hint}");
        let c = rules(PodRulesCmd::Extend {
            rule: "cr_1".into(),
            until: "4h".into(),
            reason: "r".into(),
            json: false,
        });
        let (code, hint) = g(send(&http, &server.uri(), &c).await.unwrap_err());
        assert_eq!(code, "E_CARRIAGE_STATE_CONFLICT");
        assert!(hint.contains("changed meanwhile"), "{hint}");
        // A catalogued refusal keeps the pod's own fix.
        let c = rules(PodRulesCmd::Reactivate {
            rule: "cr_9".into(),
            until: "4h".into(),
            reason: "r".into(),
            json: false,
        });
        let (code, hint) = g(send(&http, &server.uri(), &c).await.unwrap_err());
        assert_eq!(code, "E_CARRIAGE_UNKNOWN");
        assert!(
            hint.contains("fix: List the rules with goat pod rules ls."),
            "{hint}"
        );
        // Nothing listening: E_POD_UNREACHABLE.
        let (_, c, _) = request_for(PodCmd::Status { json: false }, now()).unwrap();
        let (code, _) = g(send(&http, "http://127.0.0.1:9", &c).await.unwrap_err());
        assert_eq!(code, "E_POD_UNREACHABLE");
    }

    #[test]
    fn change_answers_render_rule_revoke_and_warning() {
        let c = rules(PodRulesCmd::Revoke {
            source: SRC.into(),
            reason: "lost".into(),
            json: false,
        });
        let t = render(&c, &json!({"rule": null, "entry": {"entry": {"seq": 5}}}));
        assert!(t.contains(SRC) && t.contains("cut off this pod"), "{t}");
        let c = rules(PodRulesCmd::Allow {
            source: SRC.into(),
            until: "4h".into(),
            reason: "relay".into(),
            json: false,
        });
        let t = render(
            &c,
            &json!({"rule": {"id": "lr_9", "origin": "local", "kind": "local", "source": SRC,
                             "until": "2026-11-19T14:00:00Z", "state": "active"},
                    "warning": "recorded; the router rolled back"}),
        );
        assert!(t.starts_with("lr_9") && t.contains("! recorded"), "{t}");
    }

    /// A real mutual-TLS round trip: the pod serves its own goat leaf (CN =
    /// its slot, no name for the address dialled), the CLI presents its leaf,
    /// and the chain plus CN check decide. A leaf naming another slot, or one
    /// from another CA, is refused.
    #[tokio::test]
    async fn pod_tls_checks_chain_and_slot_not_hostname() {
        use panoscope_core::pinned_tls::rustls;
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
        use rustls::pki_types::pem::PemObject;
        use std::io::{Read, Write};

        const POD: &str = "fedcba9876543210fedcba9876543210";
        let ca = |name: &str| {
            let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
            p.distinguished_name.push(DnType::CommonName, name);
            p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            let k = KeyPair::generate().unwrap();
            (p.self_signed(&k).unwrap(), k)
        };
        let leaf = |cn: &str, issuer: &(rcgen::Certificate, KeyPair)| {
            let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
            p.distinguished_name.push(DnType::CommonName, cn);
            let k = KeyPair::generate().unwrap();
            (p.signed_by(&k, &issuer.0, &issuer.1).unwrap(), k)
        };
        let goatnet = ca("test goatnet CA");
        let other = ca("another CA");
        let client = leaf(&format!("{SRC}.g1"), &goatnet);

        // One-request HTTPS server presenting `server`, requiring a client
        // certificate from the goatnet CA.
        let serve = |server: (rcgen::Certificate, KeyPair)| {
            let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let mut roots = rustls::RootCertStore::empty();
            roots.add(goatnet.0.der().clone()).unwrap();
            let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                std::sync::Arc::new(roots),
                provider.clone(),
            )
            .build()
            .unwrap();
            let cfg = rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    vec![server.0.der().clone()],
                    rustls::pki_types::PrivateKeyDer::from_pem_slice(
                        server.1.serialize_pem().as_bytes(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let (sock, _) = listener.accept().unwrap();
                let conn = rustls::ServerConnection::new(std::sync::Arc::new(cfg)).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, sock);
                let mut buf = [0u8; 4096];
                let mut req = Vec::new();
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let body =
                    br#"{"pod_slot":"fedcba9876543210fedcba9876543210","state":"connected"}"#;
                let _ = write!(
                    tls,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls.write_all(body);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
            format!("https://127.0.0.1:{port}")
        };
        let client_for = |want: &str| {
            let cfg = panoscope_core::pinned_tls::client_config(
                goatnet.0.pem().as_bytes(),
                client.0.pem().as_bytes(),
                client.1.serialize_pem().as_bytes(),
                leaf_is_pod(want.to_string()),
            )
            .unwrap();
            reqwest::Client::builder()
                .use_preconfigured_tls(cfg)
                .build()
                .unwrap()
        };
        let (_, status, _) = request_for(PodCmd::Status { json: true }, now()).unwrap();

        let base = serve(leaf(&format!("{POD}.g3"), &goatnet));
        let v = send(&client_for(POD), &base, &status).await.unwrap();
        assert_eq!(v["state"], "connected");

        let base = serve(leaf(SRC, &goatnet));
        let e = send(&client_for(POD), &base, &status).await.unwrap_err();
        let g = e.downcast_ref::<GoatError>().unwrap();
        assert_eq!(g.code(), "E_POD_UNREACHABLE");
        assert!(
            g.hint().contains(&format!("is `{SRC}`, not pod")),
            "{}",
            g.hint()
        );

        let base = serve(leaf(POD, &other));
        assert!(send(&client_for(POD), &base, &status).await.is_err());
    }

    #[test]
    fn pod_url_needs_a_scheme_or_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: dir.path().to_path_buf(),
            context: "zagros".into(),
        };
        let e = pod_url(None, &p).unwrap_err();
        let g = e.downcast_ref::<GoatError>().unwrap();
        assert_eq!(g.code(), "E_POD_NOT_NAMED");
        assert!(g.hint().contains("--pod"));
        assert!(pod_url(Some("pod.example:8443"), &p).is_err());
        assert_eq!(
            pod_url(Some("https://pod.example:8443/"), &p).unwrap(),
            "https://pod.example:8443"
        );
    }
}

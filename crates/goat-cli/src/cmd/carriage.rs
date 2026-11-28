// SPDX-License-Identifier: Apache-2.0
//
// `goat carriage` — the portal side of ADR 1251 carriage rules
// (docs/design/pod-carriage-and-provenance.md §2, §7.5). Thin client over
// `/api/carriage/*`: the portal holds the rules, enforces consent, the
// longest rule length and the reactivation window; this wrapper builds the
// request, renders the answer, and maps the portal's refusals to the error
// catalog (`E_CARRIAGE_*`, ops/errors/catalog.yaml).
//
// The pod-side verbs (`goat pod …`, the pod's local API §7.4) live in
// `cmd/pod.rs` and share the `--until` parser and the refusal mapping here.

use crate::errors::GoatError;
use crate::i18n::LANGUAGE_LOADER;
use crate::output::with_spinner;
use clap::Subcommand;
use i18n_embed_fl::fl;
use panoscope_core::portal::{PortalClient, PortalError};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Subcommand, Debug)]
pub enum CarriageCmd {
    /// Register pods for an org (operator) and list registered pods.
    Pod {
        #[command(subcommand)]
        op: CarriagePodCmd,
    },
    /// Grant a named or temporary rule: a pod (or an org's pods) may carry a
    /// source (or an org's sources). The source's custodian grants; the pod's
    /// custodian accepts. `<source>` and `--pod` take a 32-hex slot, or
    /// `org:<org>` for every source or pod of that org.
    Allow {
        /// The source: a 32-hex slot, or `org:<org>`.
        source: String,
        /// The pod: a 32-hex slot, or `org:<org>`.
        #[arg(long)]
        pod: String,
        /// End of the rule: an RFC 3339 time, or a length from now
        /// (`4h`, `24h`, `72h`, `30d`). At most the goatnet's longest rule
        /// length (90 days by default).
        #[arg(long)]
        until: String,
        /// Why the rule exists; recorded with the rule.
        #[arg(long)]
        reason: String,
        /// `named` (one pod, one source) or `temporary` (for example an
        /// exercise's pods carrying an org's devices).
        #[arg(long, default_value = "named", value_parser = ["named", "temporary"])]
        kind: String,
        /// Emit the rule as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List carriage rules, optionally for one pod or one source.
    Ls {
        /// Only rules for this pod slot.
        #[arg(long)]
        pod: Option<String>,
        /// Only rules for this source slot.
        #[arg(long)]
        source: Option<String>,
        /// `proposed`, `active` (the portal default) or `archived`.
        #[arg(long, value_parser = ["proposed", "active", "archived"])]
        state: Option<String>,
        /// Emit `{"rules":[…]}` as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Move a rule's end date. The rule's owner only; never past the longest
    /// rule length.
    Extend {
        /// The rule id (`cr_…`).
        rule: String,
        /// New end: an RFC 3339 time, or a length from now (`72h`, `30d`).
        #[arg(long)]
        until: String,
        #[arg(long)]
        json: bool,
    },
    /// End a rule now; it is archived, not deleted.
    Revoke {
        /// The rule id (`cr_…`).
        rule: String,
        #[arg(long)]
        json: bool,
    },
    /// Bring back an archived rule, inside the goatnet's reactivation window
    /// (six months by default). After the window, grant a new rule.
    Reactivate {
        /// The rule id (`cr_…`).
        rule: String,
        /// New end (RFC 3339 or a length such as `30d`). Default: the
        /// portal gives the rule its original length again.
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Accept a proposed rule as the pod's custodian. A rule granted by
    /// someone who is not also the pod's custodian stays `proposed` until
    /// then.
    Accept {
        /// The rule id (`cr_…`).
        rule: String,
        #[arg(long)]
        json: bool,
    },
    /// Take a source out of its org's default rule (pods of its org stop
    /// carrying it), or with `--undo` put it back. The source's custodian only.
    OptOut {
        /// The source's 32-hex slot.
        source: String,
        /// Opt back in to the org rule.
        #[arg(long)]
        undo: bool,
        #[arg(long)]
        json: bool,
    },
    /// Local rules a cut-off pod made, waiting for an org lead to ratify or
    /// let lapse.
    Pending {
        /// `pending` (default), `ratified`, `lapsed` or `all`.
        #[arg(long, default_value = "pending",
              value_parser = ["pending", "ratified", "lapsed", "all"])]
        state: String,
        /// Emit `{"local_rules":[…]}` as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Ratify a pod's local rule (org lead of the pod's org).
    Ratify {
        /// The local rule id.
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Let a pod's local rule lapse (org lead of the pod's org).
    Lapse {
        /// The local rule id.
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CarriagePodCmd {
    /// Register a pod for an org (operator). Creates the org rule: the pod
    /// may carry every source of that org that has not opted out.
    Add {
        /// The pod's 32-hex slot.
        pod_slot: String,
        /// The org that runs the pod.
        #[arg(long)]
        org: String,
        #[arg(long)]
        json: bool,
    },
    /// List registered pods.
    Ls {
        /// Emit `{"pods":[…]}` as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// A carriage rule as the portal returns it (§7.5). Unknown fields are
/// ignored; `--json` prints the portal's own object, not this view.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Rule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub source_slot: Option<String>,
    #[serde(default)]
    pub source_org: Option<String>,
    #[serde(default)]
    pub pod_slot: Option<String>,
    #[serde(default)]
    pub pod_org: Option<String>,
    /// Unix seconds from the portal (RFC 3339 accepted too); null for an
    /// org rule.
    #[serde(default)]
    pub until: Value,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub state: String,
    /// The archived rule a reactivation replaced.
    #[serde(default)]
    pub reactivated_from: Option<String>,
}

/// A local rule a pod reported in its ledger, as the portal lists it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct LocalRule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub pod_slot: String,
    #[serde(default)]
    pub source_slot: String,
    #[serde(default)]
    pub action: String,
    /// Unix seconds (RFC 3339 accepted too).
    #[serde(default)]
    pub until: Value,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub authority_identity: String,
    #[serde(default)]
    pub state: String,
    /// The central rule ratification produced (an exclusion for a ratified
    /// local revoke).
    #[serde(default)]
    pub ratified_rule_id: Option<String>,
}

/// A time the portal sends as Unix seconds (or RFC 3339), shown as RFC 3339
/// UTC; `-` when absent.
pub(crate) fn shown_time(v: &Value) -> String {
    match v {
        Value::Number(n) => n
            .as_i64()
            .and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0))
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| n.to_string()),
        Value::String(s) if !s.is_empty() => s.clone(),
        _ => "-".into(),
    }
}

pub async fn run(ctx: &super::Ctx, op: CarriageCmd) -> anyhow::Result<()> {
    let portal = ctx.profile.portal_client()?;
    let out = &ctx.output;
    let (label, call, json) = request_for(op, chrono::Utc::now())?;
    let v = with_spinner(out, label, send(&portal, &call))
        .await
        .map_err(|e| map_carriage_err(e, call.subject()))?;
    if json {
        println!("{}", json_output(&call, &v));
    } else {
        print!("{}", render(&call, &v));
    }
    Ok(())
}

/// One portal call a verb makes: which route, with which body, and what
/// kind of answer to render.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Call {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
    pub answer: Answer,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    Pod,
    Pods,
    Rule(String),
    Rules,
    OptOut(String, bool),
    LocalRule(String),
    LocalRules,
}

impl Call {
    /// The rule or source a refusal names.
    fn subject(&self) -> &str {
        match &self.answer {
            Answer::Rule(s) | Answer::LocalRule(s) | Answer::OptOut(s, _) => s,
            _ => "",
        }
    }
}

/// Build the portal call for a verb. Pure, so every verb's route and body
/// is unit-tested without a portal.
pub(crate) fn request_for(
    op: CarriageCmd,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<(&'static str, Call, bool)> {
    let call = |method, path: String, body, answer| Call {
        method,
        path,
        body,
        answer,
    };
    Ok(match op {
        CarriageCmd::Pod {
            op:
                CarriagePodCmd::Add {
                    pod_slot,
                    org,
                    json,
                },
        } => {
            let pod_slot = slot(&pod_slot, "pod")?;
            (
                "registering pod",
                call(
                    Method::POST,
                    "/api/carriage/pods".into(),
                    Some(json!({"pod_slot": pod_slot, "org": org})),
                    Answer::Pod,
                ),
                json,
            )
        }
        CarriageCmd::Pod {
            op: CarriagePodCmd::Ls { json },
        } => (
            "listing pods",
            call(Method::GET, "/api/carriage/pods".into(), None, Answer::Pods),
            json,
        ),
        CarriageCmd::Allow {
            source,
            pod,
            until,
            reason,
            kind,
            json,
        } => {
            let mut body = serde_json::Map::new();
            let (k, v) = party(&source, "source")?;
            body.insert(k, Value::String(v));
            let (k, v) = party(&pod, "pod")?;
            body.insert(k, Value::String(v));
            body.insert("kind".into(), Value::String(kind));
            body.insert("until".into(), Value::String(parse_until(&until, now)?));
            body.insert("reason".into(), Value::String(nonempty_reason(&reason)?));
            (
                "granting carriage rule",
                call(
                    Method::POST,
                    "/api/carriage/rules".into(),
                    Some(Value::Object(body)),
                    Answer::Rule(String::new()),
                ),
                json,
            )
        }
        CarriageCmd::Ls {
            pod,
            source,
            state,
            json,
        } => {
            let mut q = Vec::new();
            if let Some(p) = pod {
                q.push(format!("pod={}", slot(&p, "pod")?));
            }
            if let Some(s) = source {
                q.push(format!("source={}", slot(&s, "source")?));
            }
            if let Some(s) = state {
                q.push(format!("state={s}"));
            }
            let path = if q.is_empty() {
                "/api/carriage/rules".to_string()
            } else {
                format!("/api/carriage/rules?{}", q.join("&"))
            };
            (
                "listing carriage rules",
                call(Method::GET, path, None, Answer::Rules),
                json,
            )
        }
        CarriageCmd::Extend { rule, until, json } => {
            let id = rule_id(&rule)?;
            (
                "extending carriage rule",
                call(
                    Method::POST,
                    format!("/api/carriage/rules/{id}/extend"),
                    Some(json!({"until": parse_until(&until, now)?})),
                    Answer::Rule(id),
                ),
                json,
            )
        }
        CarriageCmd::Revoke { rule, json } => {
            let id = rule_id(&rule)?;
            (
                "revoking carriage rule",
                call(
                    Method::POST,
                    format!("/api/carriage/rules/{id}/revoke"),
                    Some(json!({})),
                    Answer::Rule(id),
                ),
                json,
            )
        }
        CarriageCmd::Reactivate { rule, until, json } => {
            let id = rule_id(&rule)?;
            let body = match until {
                Some(u) => json!({"until": parse_until(&u, now)?}),
                None => json!({}),
            };
            (
                "reactivating carriage rule",
                call(
                    Method::POST,
                    format!("/api/carriage/rules/{id}/reactivate"),
                    Some(body),
                    Answer::Rule(id),
                ),
                json,
            )
        }
        CarriageCmd::Accept { rule, json } => {
            let id = rule_id(&rule)?;
            (
                "accepting carriage rule",
                call(
                    Method::POST,
                    format!("/api/carriage/rules/{id}/accept"),
                    Some(json!({})),
                    Answer::Rule(id),
                ),
                json,
            )
        }
        CarriageCmd::OptOut { source, undo, json } => {
            let s = slot(&source, "source")?;
            (
                "setting org-rule opt-out",
                call(
                    Method::PUT,
                    format!("/api/carriage/sources/{s}/org-rule"),
                    Some(json!({"opt_out": !undo})),
                    Answer::OptOut(s, !undo),
                ),
                json,
            )
        }
        CarriageCmd::Pending { state, json } => (
            "listing pending local rules",
            call(
                Method::GET,
                format!("/api/carriage/local-rules?state={state}"),
                None,
                Answer::LocalRules,
            ),
            json,
        ),
        CarriageCmd::Ratify { id, json } => {
            let id = rule_id(&id)?;
            (
                "ratifying local rule",
                call(
                    Method::POST,
                    format!("/api/carriage/local-rules/{id}/ratify"),
                    None,
                    Answer::LocalRule(id),
                ),
                json,
            )
        }
        CarriageCmd::Lapse { id, json } => {
            let id = rule_id(&id)?;
            (
                "letting local rule lapse",
                call(
                    Method::POST,
                    format!("/api/carriage/local-rules/{id}/lapse"),
                    None,
                    Answer::LocalRule(id),
                ),
                json,
            )
        }
    })
}

async fn send(portal: &PortalClient, call: &Call) -> Result<Value, PortalError> {
    portal
        .call_json(call.method.clone(), &call.path, call.body.as_ref())
        .await
}

/// The `--json` output: a list verb prints `{"<key>":[…]}` whatever shape
/// the portal wrapped it in; a single-object verb prints the portal's object.
fn json_output(call: &Call, v: &Value) -> Value {
    match call.answer {
        Answer::Pods => json!({"pods": list_of(v, "pods")}),
        Answer::Rules => json!({"rules": list_of(v, "rules")}),
        Answer::LocalRules => json!({"local_rules": list_of(v, "local_rules")}),
        _ => v.clone(),
    }
}

/// The array under `key`, or the answer itself when the portal answered with
/// a bare array. Anything else is an empty list.
pub(crate) fn list_of(v: &Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn render(call: &Call, v: &Value) -> String {
    let mut s = String::new();
    match &call.answer {
        Answer::Pod => {
            // `{"pod": {…}, "org_rule": <rule|null>}`; a bare pod object too.
            let p = v.get("pod").unwrap_or(v);
            let pod = p.get("pod_slot").and_then(Value::as_str).unwrap_or("");
            let org = p.get("org").and_then(Value::as_str).unwrap_or("");
            s += &fl!(
                LANGUAGE_LOADER,
                "carriage-pod-added",
                pod = pod.to_string(),
                org = org.to_string()
            );
            s.push('\n');
            if let Some(r) = v.get("org_rule").filter(|r| r.is_object()) {
                s += &rule_line(&serde_json::from_value(r.clone()).unwrap_or_default());
            }
        }
        Answer::Pods => {
            let pods = list_of(v, "pods");
            if pods.is_empty() {
                s += &fl!(LANGUAGE_LOADER, "carriage-no-pods");
                s.push('\n');
            }
            for p in pods {
                s += &format!(
                    "{:<34} {}\n",
                    p.get("pod_slot").and_then(Value::as_str).unwrap_or(""),
                    p.get("org").and_then(Value::as_str).unwrap_or("")
                );
            }
        }
        Answer::Rule(_) => {
            // revoke answers `{"rule": …, "local_rules_lapsed": N}`.
            let r: Rule =
                serde_json::from_value(v.get("rule").unwrap_or(v).clone()).unwrap_or_default();
            s += &rule_line(&r);
            if let Some(from) = r.reactivated_from.as_deref().filter(|f| !f.is_empty()) {
                s += &fl!(
                    LANGUAGE_LOADER,
                    "carriage-reactivated-from",
                    rule = from.to_string()
                );
                s.push('\n');
            }
            if let Some(n) = v.get("local_rules_lapsed").and_then(Value::as_i64) {
                s += &fl!(
                    LANGUAGE_LOADER,
                    "carriage-local-rules-lapsed",
                    count = n.to_string()
                );
                s.push('\n');
            }
        }
        Answer::Rules => {
            let rules = list_of(v, "rules");
            if rules.is_empty() {
                s += &fl!(LANGUAGE_LOADER, "carriage-no-rules");
                s.push('\n');
            }
            for r in rules {
                s += &rule_line(&serde_json::from_value(r).unwrap_or_default());
            }
        }
        Answer::OptOut(source, out) => {
            s += &if *out {
                fl!(
                    LANGUAGE_LOADER,
                    "carriage-opted-out",
                    source = source.clone()
                )
            } else {
                fl!(
                    LANGUAGE_LOADER,
                    "carriage-opted-in",
                    source = source.clone()
                )
            };
            s.push('\n');
        }
        Answer::LocalRule(_) => {
            let r: LocalRule = serde_json::from_value(v.clone()).unwrap_or_default();
            s += &local_rule_line(&r);
        }
        Answer::LocalRules => {
            let rules = list_of(v, "local_rules");
            if rules.is_empty() {
                s += &fl!(LANGUAGE_LOADER, "carriage-no-pending");
                s.push('\n');
            }
            for r in rules {
                s += &local_rule_line(&serde_json::from_value(r).unwrap_or_default());
            }
        }
    }
    s
}

fn rule_line(r: &Rule) -> String {
    let src = r
        .source_slot
        .clone()
        .or_else(|| r.source_org.as_ref().map(|o| format!("org:{o}")))
        .unwrap_or_default();
    let pod = r
        .pod_slot
        .clone()
        .or_else(|| r.pod_org.as_ref().map(|o| format!("org:{o}")))
        .unwrap_or_default();
    let until = shown_time(&r.until);
    let reason = if r.reason.is_empty() {
        String::new()
    } else {
        format!("  ({})", r.reason)
    };
    format!(
        "{:<14} {:<9} {src} -> {pod}  until {until}  {}{reason}\n",
        r.id, r.kind, r.state
    )
}

fn local_rule_line(r: &LocalRule) -> String {
    format!(
        "{:<14} pod {}  {} {}  until {}  {}  by {}  ({})\n",
        r.id,
        r.pod_slot,
        r.action,
        r.source_slot,
        shown_time(&r.until),
        r.state,
        r.authority_identity,
        r.reason
    ) + &r
        .ratified_rule_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| format!("  → rule {id}\n"))
        .unwrap_or_default()
}

/// A 32-hex slot anchor, lowercased.
pub(crate) fn slot(s: &str, what: &str) -> anyhow::Result<String> {
    let t = s.trim().to_ascii_lowercase();
    if t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(t)
    } else {
        anyhow::bail!("{what} `{s}` is not a 32-hex slot")
    }
}

/// `<slot>` or `org:<org>` → the body field and its value.
fn party(s: &str, role: &str) -> anyhow::Result<(String, String)> {
    if let Some(org) = s.strip_prefix("org:") {
        let org = org.trim();
        if org.is_empty() {
            anyhow::bail!("{role} `org:` names no org");
        }
        return Ok((format!("{role}_org"), org.to_string()));
    }
    slot(s, role)
        .map(|v| (format!("{role}_slot"), v))
        .map_err(|_| anyhow::anyhow!("{role} `{s}` is neither a 32-hex slot nor `org:<org>`"))
}

/// A rule id as typed, refused when it could not be one path segment.
pub(crate) fn rule_id(s: &str) -> anyhow::Result<String> {
    let t = s.trim();
    if t.is_empty()
        || !t
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        anyhow::bail!("`{s}` is not a rule id (letters, digits, `_` and `-` only)");
    }
    Ok(t.to_string())
}

pub(crate) fn nonempty_reason(s: &str) -> anyhow::Result<String> {
    let t = s.trim();
    if t.is_empty() {
        anyhow::bail!("--reason is empty: every carriage change records why it was made");
    }
    Ok(t.to_string())
}

/// Parse `--until`: an RFC 3339 time, or a length from `now` in hours
/// (`4h`) or days (`30d`). Returns RFC 3339 UTC with second precision. A
/// time in the past is refused here; the longest allowed length is a
/// goatnet (or pod) setting, checked by the server (`E_CARRIAGE_TOO_LONG`).
pub(crate) fn parse_until(s: &str, now: chrono::DateTime<chrono::Utc>) -> anyhow::Result<String> {
    use chrono::{DateTime, Duration, SecondsFormat, Utc};
    let t = s.trim();
    let at: DateTime<Utc> = if let Some(n) = t.strip_suffix('h').and_then(|n| n.parse::<i64>().ok())
    {
        now + Duration::hours(n)
    } else if let Some(n) = t.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        now + Duration::days(n)
    } else {
        DateTime::parse_from_rfc3339(t)
            .map_err(|_| {
                anyhow::anyhow!(
                    "--until `{s}` is neither an RFC 3339 time (2027-02-17T00:00:00Z) nor a \
                     length like 4h, 72h or 30d"
                )
            })?
            .with_timezone(&Utc)
    };
    if at <= now {
        anyhow::bail!("--until `{s}` is not in the future");
    }
    Ok(at.to_rfc3339_opts(SecondsFormat::Secs, true))
}

/// A carriage refusal, from the portal or a pod, as an error-catalog code.
/// `code` is the server's `error.code` (the portal's names, and the pod's
/// where it uses others); a bare 403 on a carriage route means the caller is
/// not authorized for the change. `None` = not a carriage refusal; the caller
/// falls back to its generic mapping (404 `not_found`, 503 `config`, …).
pub(crate) fn carriage_refusal(
    code: Option<&str>,
    status: u16,
    subject: &str,
    server: &str,
) -> Option<GoatError> {
    let rule = subject.to_string();
    let state = |why: &str| GoatError::CarriageStateConflict {
        rule: rule.clone(),
        hint: format!("{why}{server}"),
    };
    let unknown = |why: &str| GoatError::CarriageUnknown {
        hint: format!("{why}{server}"),
    };
    Some(match (code.unwrap_or(""), status) {
        ("carriage_rule_expired" | "carriage_rule_archived" | "local_rule_expired", _) => {
            GoatError::CarriageExpired {
                hint: format!(
                    "the rule ended or was revoked and is archived; an archived rule is \
                     reactivated, not extended: `goat carriage reactivate {rule}` (or `goat pod \
                     rules reactivate {rule}` on a cut-off pod).{server}"
                ),
                rule: rule.clone(),
            }
        }
        ("carriage_beyond_reactivation_window" | "carriage_reactivation_window_passed", _) => {
            GoatError::CarriageWindowPassed {
                hint: format!(
                    "the rule ended longer ago than this goatnet's reactivation window; grant a \
                     new rule with `goat carriage allow <source> --pod <pod> --until <time> \
                     --reason \"…\"`.{server}"
                ),
                rule: rule.clone(),
            }
        }
        ("carriage_until_required" | "carriage_over_max_length" | "carriage_until_too_far", _) => {
            GoatError::CarriageTooLong {
                hint: format!(
                    "a named, temporary or local rule needs an end date no later than the \
                     longest rule length (90 days unless this goatnet or pod sets another); pass \
                     an earlier --until, and extend the rule when it nears its end.{server}"
                ),
            }
        }
        ("carriage_until_past", _) => GoatError::CarriageEndInvalid {
            hint: format!(
                "the end date is already past by the server's clock; pass a later --until, \
                 and check this machine's clock if the time looked right.{server}"
            ),
        },
        ("carriage_exclusion_no_end", _) => GoatError::CarriageEndInvalid {
            hint: format!(
                "an exclusion (a ratified local revoke) has no end date and cannot be extended \
                 or reactivated; the source's custodian lifts it with `goat carriage revoke \
                 {rule}`.{server}"
            ),
        },
        ("carriage_org_rule_no_end", _) => GoatError::CarriageEndInvalid {
            hint: format!(
                "an org rule has no end date and cannot be given one; to stop an org's pods \
                 carrying one source use `goat carriage opt-out <source>`.{server}"
            ),
        },
        ("carriage_not_authorized", _) | (_, 403) => GoatError::CarriageNotAuthorized {
            hint: format!(
                "a rule is granted by the source's custodian and accepted by the pod's \
                 custodian; extend, revoke and reactivate belong to the rule's owner; ratify and \
                 lapse to the org lead of the pod's org; registering a pod to an operator; on a \
                 pod, only the identities its rules name as local authority may change rules. \
                 Ask that person, or use the context that holds their credential.{server}"
            ),
        },
        ("carriage_not_archived", _) => state(
            "the rule is not archived, so there is nothing to reactivate; to move its end date \
             use `goat carriage extend`.",
        ),
        ("carriage_already_reactivated", _) => state(
            "this archived rule was already reactivated as a newer rule; act on that one \
             (`goat carriage ls`).",
        ),
        ("carriage_not_proposed", _) => state(
            "only a proposed rule can be accepted; this one is already active or archived \
             (`goat carriage ls --state archived`).",
        ),
        ("local_rule_not_pending", _) => state(
            "the local rule was already ratified or let lapse; `goat carriage pending --state \
             all` shows it.",
        ),
        ("carriage_pod_org_mismatch", _) => state(
            "the pod is registered for another org than the one the rule names; check \
             `goat carriage pod ls`.",
        ),
        ("carriage_source_revoked", _) => state(
            "the source's identity was revoked; goat's revocations win over any carriage \
             rule, central or local.",
        ),
        ("conflict", _) => state(
            "the rule changed since it was last read (another change came first); list the \
             rules again and retry against their current state.",
        ),
        ("ledger_conflict", _) => state(
            "the pod's ledger conflicts with what the portal holds; the pod resends it on its \
             next connection.",
        ),
        ("pod_not_registered", _) => unknown(
            "the pod is not registered for carriage; an operator registers it with `goat \
             carriage pod add <pod-slot> --org <org>`.",
        ),
        ("source_not_live" | "pod_slot_not_live", _) => unknown(
            "the slot is not a live enrolled identity (never enrolled, or retired); check it \
             with `goat machine ls`.",
        ),
        ("not_found", _) | (_, 404) => unknown(
            "no rule, local rule or pod with that id; list them with `goat carriage ls` or \
             `goat carriage pending --state all`.",
        ),
        _ => return None,
    })
}

fn map_carriage_err(e: PortalError, subject: &str) -> anyhow::Error {
    if let Some(status) = e.http_status() {
        let server = super::vendor::server_line(&e);
        if let Some(g) = carriage_refusal(e.error_code(), status.as_u16(), subject, &server) {
            return g.into();
        }
    }
    super::vendor::map_portal_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SRC: &str = "0123456789abcdef0123456789abcdef";
    const POD: &str = "fedcba9876543210fedcba9876543210";

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 11, 19, 10, 0, 0)
            .unwrap()
    }

    #[test]
    fn until_accepts_lengths_and_rfc3339_and_refuses_the_past() {
        assert_eq!(parse_until("4h", now()).unwrap(), "2026-11-19T14:00:00Z");
        assert_eq!(parse_until("72h", now()).unwrap(), "2026-11-22T10:00:00Z");
        assert_eq!(parse_until("30d", now()).unwrap(), "2026-12-19T10:00:00Z");
        assert_eq!(
            parse_until("2027-02-17T01:00:00+01:00", now()).unwrap(),
            "2027-02-17T00:00:00Z"
        );
        assert!(parse_until("2026-11-01T00:00:00Z", now()).is_err());
        assert!(parse_until("0h", now()).is_err());
        assert!(parse_until("tomorrow", now()).is_err());
    }

    #[test]
    fn allow_builds_the_rule_body_for_slots_and_orgs() {
        let (_, c, json) = request_for(
            CarriageCmd::Allow {
                source: SRC.to_uppercase(),
                pod: "org:example-org".into(),
                until: "24h".into(),
                reason: " exercise ".into(),
                kind: "temporary".into(),
                json: true,
            },
            now(),
        )
        .unwrap();
        assert!(json);
        assert_eq!(c.method, Method::POST);
        assert_eq!(c.path, "/api/carriage/rules");
        assert_eq!(
            c.body.unwrap(),
            json!({"source_slot": SRC, "pod_org": "example-org", "kind": "temporary",
                   "until": "2026-11-20T10:00:00Z", "reason": "exercise"})
        );
    }

    #[test]
    fn bad_arguments_are_refused_before_any_request() {
        let allow = |source: &str, reason: &str| CarriageCmd::Allow {
            source: source.into(),
            pod: POD.into(),
            until: "4h".into(),
            reason: reason.into(),
            kind: "named".into(),
            json: false,
        };
        assert!(request_for(allow("not-a-slot", "r"), now()).is_err());
        assert!(request_for(allow("org:", "r"), now()).is_err());
        assert!(request_for(allow(SRC, "  "), now()).is_err());
        let ext = CarriageCmd::Extend {
            rule: "../x".into(),
            until: "4h".into(),
            json: false,
        };
        assert!(request_for(ext, now()).is_err());
    }

    #[test]
    fn every_verb_hits_its_design_route() {
        let cases: Vec<(CarriageCmd, Method, &str)> = vec![
            (
                CarriageCmd::Pod {
                    op: CarriagePodCmd::Add {
                        pod_slot: POD.into(),
                        org: "example-org".into(),
                        json: false,
                    },
                },
                Method::POST,
                "/api/carriage/pods",
            ),
            (
                CarriageCmd::Pod {
                    op: CarriagePodCmd::Ls { json: false },
                },
                Method::GET,
                "/api/carriage/pods",
            ),
            (
                CarriageCmd::Ls {
                    pod: Some(POD.into()),
                    source: None,
                    state: Some("archived".into()),
                    json: false,
                },
                Method::GET,
                "/api/carriage/rules?pod=fedcba9876543210fedcba9876543210&state=archived",
            ),
            (
                CarriageCmd::Revoke {
                    rule: "cr_1".into(),
                    json: false,
                },
                Method::POST,
                "/api/carriage/rules/cr_1/revoke",
            ),
            (
                CarriageCmd::Reactivate {
                    rule: "cr_1".into(),
                    until: None,
                    json: false,
                },
                Method::POST,
                "/api/carriage/rules/cr_1/reactivate",
            ),
            (
                CarriageCmd::OptOut {
                    source: SRC.into(),
                    undo: false,
                    json: false,
                },
                Method::PUT,
                "/api/carriage/sources/0123456789abcdef0123456789abcdef/org-rule",
            ),
            (
                CarriageCmd::Pending {
                    state: "pending".into(),
                    json: false,
                },
                Method::GET,
                "/api/carriage/local-rules?state=pending",
            ),
            (
                CarriageCmd::Accept {
                    rule: "cr_3".into(),
                    json: false,
                },
                Method::POST,
                "/api/carriage/rules/cr_3/accept",
            ),
            (
                CarriageCmd::Ratify {
                    id: "lr_7".into(),
                    json: false,
                },
                Method::POST,
                "/api/carriage/local-rules/lr_7/ratify",
            ),
            (
                CarriageCmd::Lapse {
                    id: "lr_7".into(),
                    json: false,
                },
                Method::POST,
                "/api/carriage/local-rules/lr_7/lapse",
            ),
        ];
        for (op, m, p) in cases {
            let (_, c, _) = request_for(op, now()).unwrap();
            assert_eq!((c.method, c.path.as_str()), (m, p));
        }
    }

    async fn portal_answering(
        m: &str,
        p: &str,
        status: u16,
        body: Value,
    ) -> (MockServer, PortalClient) {
        let server = MockServer::start().await;
        Mock::given(method(m))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let client = PortalClient::new(server.uri(), "vp_test".into());
        (server, client)
    }

    #[tokio::test]
    async fn ls_renders_rules_and_normalizes_json() {
        let rules = json!({"rules": [
            {"id": "cr_1", "kind": "org", "source_org": "example-org", "pod_org": "example-org",
             "until": null, "state": "active"},
            {"id": "cr_2", "kind": "named", "source_slot": SRC, "pod_slot": POD,
             "until": 1802822400, "reason": "field trial", "state": "active"}
        ]});
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/carriage/rules"))
            .and(query_param("source", SRC))
            .respond_with(ResponseTemplate::new(200).set_body_json(rules.clone()))
            .mount(&server)
            .await;
        let portal = PortalClient::new(server.uri(), "vp_test".into());
        let (_, c, _) = request_for(
            CarriageCmd::Ls {
                pod: None,
                source: Some(SRC.into()),
                state: None,
                json: true,
            },
            now(),
        )
        .unwrap();
        let v = send(&portal, &c).await.unwrap();
        let text = render(&c, &v);
        assert!(text.contains("cr_1") && text.contains("org:example-org -> org:example-org"));
        assert!(text.contains("until 2027-02-17T00:00:00Z") && text.contains("(field trial)"));
        assert_eq!(json_output(&c, &v), rules);
        // A bare array answer prints the same normalized shape.
        let bare = Value::Array(list_of(&rules, "rules"));
        assert_eq!(json_output(&c, &bare), rules);
    }

    #[test]
    fn pod_add_revoke_reactivate_and_local_rule_answers_render() {
        let call = |answer| Call {
            method: Method::POST,
            path: String::new(),
            body: None,
            answer,
        };
        let pod = json!({"pod": {"pod_slot": POD, "org": "example-org", "generation": 1},
                         "org_rule": {"id": "cr_o", "kind": "org", "source_org": "example-org",
                                      "pod_org": "example-org", "until": null, "state": "active"}});
        let t = render(&call(Answer::Pod), &pod);
        assert!(
            t.contains(POD) && t.contains("example-org") && t.contains("cr_o"),
            "{t}"
        );
        let revoked = json!({"rule": {"id": "cr_2", "kind": "named", "source_slot": SRC,
                             "pod_slot": POD, "until": 1802822400, "state": "archived"},
                             "local_rules_lapsed": 2});
        let t = render(&call(Answer::Rule("cr_2".into())), &revoked);
        assert!(
            t.starts_with("cr_2") && t.contains("archived") && t.contains("2 local"),
            "{t}"
        );
        let re = json!({"id": "cr_3", "kind": "named", "state": "active", "reactivated_from": "cr_2",
                        "until": 1802822400});
        let t = render(&call(Answer::Rule("cr_2".into())), &re);
        assert!(t.starts_with("cr_3") && t.contains("cr_2"), "{t}");
        let lr = json!({"id": "lr_1", "pod_slot": POD, "source_slot": SRC, "action": "allow",
                        "until": 1763560800, "reason": "relay", "authority_identity": "test-pki:operator-1",
                        "state": "pending"});
        let t = render(&call(Answer::LocalRule("lr_1".into())), &lr);
        assert!(
            t.contains("until 2025-11-19T14:00:00Z") && t.contains("test-pki:operator-1"),
            "{t}"
        );
        let ratified = json!({"id": "lr_2", "action": "revoke", "source_slot": SRC,
                              "state": "ratified", "ratified_rule_id": "cr_x"});
        let t = render(&call(Answer::LocalRule("lr_2".into())), &ratified);
        assert!(t.contains("until -") && t.contains("→ rule cr_x"), "{t}");
        let excl = json!({"rules": [{"id": "cr_x", "kind": "exclusion", "source_slot": SRC,
                                     "pod_slot": POD, "until": null, "state": "active"}]});
        let t = render(&call(Answer::Rules), &excl);
        assert!(t.contains("exclusion") && t.contains("until -"), "{t}");
    }

    #[tokio::test]
    async fn allow_posts_the_body_and_renders_the_rule() {
        let server = MockServer::start().await;
        let want = json!({"source_slot": SRC, "pod_slot": POD, "kind": "named",
                          "until": "2026-11-22T10:00:00Z", "reason": "trial"});
        Mock::given(method("POST"))
            .and(path("/api/carriage/rules"))
            .and(body_json(want))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "cr_9", "kind": "named", "source_slot": SRC, "pod_slot": POD,
                "until": "2026-11-22T10:00:00Z", "reason": "trial", "state": "active"})))
            .expect(1)
            .mount(&server)
            .await;
        let portal = PortalClient::new(server.uri(), "vp_test".into());
        let (_, c, _) = request_for(
            CarriageCmd::Allow {
                source: SRC.into(),
                pod: POD.into(),
                until: "72h".into(),
                reason: "trial".into(),
                kind: "named".into(),
                json: false,
            },
            now(),
        )
        .unwrap();
        let v = send(&portal, &c).await.unwrap();
        assert!(render(&c, &v).starts_with("cr_9"));
    }

    fn refusal_code(status: u16, code: &str) -> String {
        let e = PortalError::HttpWithBody {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            code: Some(code.into()),
            message: "refused".into(),
            path: String::new(),
            retry_after: None,
        };
        map_carriage_err(e, "cr_1")
            .downcast_ref::<GoatError>()
            .expect("a GoatError")
            .code()
            .to_string()
    }

    #[tokio::test]
    async fn portal_refusals_map_to_catalog_codes() {
        let (_s, portal) = portal_answering(
            "POST",
            "/api/carriage/rules/cr_1/extend",
            409,
            json!({"code": "carriage_rule_expired", "detail": "rule cr_1 is archived",
                   "error": {"code": "carriage_rule_expired", "message": "rule cr_1 is archived"}}),
        )
        .await;
        let (_, c, _) = request_for(
            CarriageCmd::Extend {
                rule: "cr_1".into(),
                until: "24h".into(),
                json: false,
            },
            now(),
        )
        .unwrap();
        let e = map_carriage_err(send(&portal, &c).await.unwrap_err(), c.subject());
        let g = e.downcast_ref::<GoatError>().unwrap();
        assert_eq!(g.code(), "E_CARRIAGE_EXPIRED");
        assert!(
            g.hint().contains("goat carriage reactivate cr_1"),
            "{}",
            g.hint()
        );
        assert!(g.hint().contains("rule cr_1 is archived"), "{}", g.hint());

        for (status, code, want) in [
            (
                409,
                "carriage_beyond_reactivation_window",
                "E_CARRIAGE_WINDOW_PASSED",
            ),
            (
                409,
                "carriage_reactivation_window_passed",
                "E_CARRIAGE_WINDOW_PASSED",
            ),
            (409, "carriage_rule_archived", "E_CARRIAGE_EXPIRED"),
            (409, "local_rule_expired", "E_CARRIAGE_EXPIRED"),
            (400, "carriage_until_required", "E_CARRIAGE_TOO_LONG"),
            (400, "carriage_over_max_length", "E_CARRIAGE_TOO_LONG"),
            (400, "carriage_until_past", "E_CARRIAGE_END_INVALID"),
            (400, "carriage_org_rule_no_end", "E_CARRIAGE_END_INVALID"),
            (400, "carriage_exclusion_no_end", "E_CARRIAGE_END_INVALID"),
            (409, "carriage_not_archived", "E_CARRIAGE_STATE_CONFLICT"),
            (
                409,
                "carriage_already_reactivated",
                "E_CARRIAGE_STATE_CONFLICT",
            ),
            (409, "carriage_not_proposed", "E_CARRIAGE_STATE_CONFLICT"),
            (409, "local_rule_not_pending", "E_CARRIAGE_STATE_CONFLICT"),
            (
                409,
                "carriage_pod_org_mismatch",
                "E_CARRIAGE_STATE_CONFLICT",
            ),
            (409, "carriage_source_revoked", "E_CARRIAGE_STATE_CONFLICT"),
            (409, "ledger_conflict", "E_CARRIAGE_STATE_CONFLICT"),
            (409, "conflict", "E_CARRIAGE_STATE_CONFLICT"),
            (404, "not_found", "E_CARRIAGE_UNKNOWN"),
            (404, "pod_not_registered", "E_CARRIAGE_UNKNOWN"),
            (404, "source_not_live", "E_CARRIAGE_UNKNOWN"),
            (404, "pod_slot_not_live", "E_CARRIAGE_UNKNOWN"),
            (503, "config", "E_PORTAL_SERVER_ERROR"),
        ] {
            assert_eq!(refusal_code(status, code), want, "{code}");
        }
        assert_eq!(
            refusal_code(400, "carriage_until_too_far"),
            "E_CARRIAGE_TOO_LONG"
        );
        assert_eq!(
            refusal_code(403, "carriage_not_authorized"),
            "E_CARRIAGE_NOT_AUTHORIZED"
        );
        assert_eq!(refusal_code(403, "forbidden"), "E_CARRIAGE_NOT_AUTHORIZED");
        // Anything else keeps the generic portal mapping.
        assert_eq!(refusal_code(401, "unauthorized"), "E_TOKEN_INVALID");
        assert_eq!(refusal_code(400, "validate"), "E_PORTAL_REFUSED");
    }
}

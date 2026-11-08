// Stable error-code namespace for goat-cli. Codes are the same string
// scripts grep on and that `goat explain <code>` (Phase E) will look
// up against docs/design/goat-cli-error-codes.md. Adding a variant ⇒
// also add the row in that doc and (if it maps to a non-generic exit
// path) wire it through `exit_code()`.
//
// Display impl renders cargo-style two-line output:
//
//   error: <summary>
//     help: <next-step>
//
// The top-level main.rs handler maps `GoatError` variants to the
// documented exit codes via `exit_code()`.

use crate::exit_codes;
use std::fmt;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Variants land as each subcommand wires them; Phase B-E sweep.
pub enum GoatError {
    /// `goat <anything>` invoked before `goat profile init` (or the
    /// referenced context lacks profile.toml / cert / key).
    #[error("E_PROFILE_MISSING")]
    ProfileMissing { context: String, hint: String },

    /// The profile is present but unreadable by this user (EACCES — e.g. a
    /// root-owned 0700 profile dir probed without sudo). Distinct from
    /// ProfileMissing so the operator is told to escalate, not to re-init.
    #[error("E_PROFILE_DENIED")]
    ProfileDenied { context: String, hint: String },

    /// Initial ZTOL-membership precheck (outer-frame, cached 60s) said
    /// we're not on the mesh.
    #[error("E_OFF_MESH")]
    OffMesh { hint: String },

    /// The device clock differs from the configured management server's
    /// authenticated HTTP Date by more than the enrollment tolerance.
    #[error("E_CLOCK_SKEW")]
    ClockSkew { hint: String },

    /// Router TCP path or Zenoh session could not be reached.
    #[error("E_ROUTER_UNREACHABLE")]
    RouterUnreachable { endpoint: String, hint: String },

    /// Our mTLS cert is outside its local validity interval.
    #[error("E_CERT_EXPIRED")]
    CertExpired { hint: String },

    /// Doctor observed a real failure but cannot establish a narrower cause.
    #[error("E_DOCTOR_INCONCLUSIVE")]
    DoctorInconclusive { hint: String },

    /// Portal REST API not reachable, or the auth probe failed for
    /// network reasons (DNS / TCP / TLS — not a 401).
    #[error("E_PORTAL_UNREACHABLE")]
    PortalUnreachable { endpoint: String, hint: String },

    /// Portal accepted the connection but rejected the Bearer token as
    /// invalid/expired (HTTP 401). The credential itself is bad — re-onboard.
    #[error("E_TOKEN_INVALID")]
    TokenInvalid { hint: String },

    /// The portal understood the credential but denied this doctor probe.
    #[error("E_PORTAL_FORBIDDEN")]
    PortalForbidden { hint: String },

    /// The portal answered HTTP 404: what the command named is not
    /// registered there. An answer, not a network fault (#2087).
    #[error("E_PORTAL_NOT_FOUND")]
    PortalNotFound { path: String, hint: String },

    /// The portal answered HTTP 429: this client is being rate-limited.
    /// `retry_after` is the portal's `Retry-After`, when sent.
    #[error("E_PORTAL_RATE_LIMITED")]
    PortalRateLimited {
        path: String,
        retry_after: Option<String>,
        hint: String,
    },

    /// The portal answered HTTP 5xx: it is up and failed on its own side.
    #[error("E_PORTAL_SERVER_ERROR")]
    PortalServerError { status: u16, path: String, hint: String },

    /// The portal answered with any other non-2xx status no command curates
    /// (a 400, 405, 410, 413, 422 ...): it is up and refused the request.
    #[error("E_PORTAL_REFUSED")]
    PortalRefused { status: u16, path: String, hint: String },

    /// The portal answered (2xx) but the body did not deserialize into the
    /// shape this build expects — a response-contract mismatch, almost
    /// always a portal/CLI version skew. Distinct from `PortalUnreachable`
    /// (the portal never answered / a network fault): here the portal is
    /// healthy and reachable, so "unreachable" would misdiagnose it and
    /// send the operator chasing their network. The fix is to upgrade goat.
    #[error("E_PORTAL_BAD_RESPONSE")]
    PortalBadResponse { hint: String },

    /// An operator-only (`goat admin`) action was attempted without admin
    /// authority. Two paths land here: (a) client-side, the active profile
    /// carries no operator credentials at all, so the command is refused
    /// before anything is sent; (b) server-side, the portal / NetBird mgmt
    /// API rejected the request with HTTP 403 — a mutation 403 means nothing
    /// changed. Distinct from `TokenInvalid` (401: the credential is
    /// expired/garbage): here the credential is *valid* but not *authorized*
    /// for this action. `goat admin` is inert for non-admins by design.
    #[error("E_NOT_ADMIN")]
    NotAdmin { hint: String },

    /// Topic create attempted against a name that's already registered.
    /// (Reserved for Phase C — the variant exists now so Phase C is
    /// a code-add, not a breaking signature change.)
    #[error("E_TOPIC_CONFLICT")]
    TopicConflict { topic: String, hint: String },

    /// Bundle CBOR parse / signature verify failed at `profile init`.
    #[error("E_BUNDLE_INVALID")]
    BundleInvalid { hint: String },

    /// The machine named on a `goat machine` verb is not exactly one enrolled
    /// machine this profile can see: no handle or anchor matches, or a handle
    /// matches machines under more than one custodian.
    #[error("E_MACHINE_UNRESOLVED")]
    MachineUnresolved { machine: String, hint: String },

    /// The enrolled machine has no single NetBird peer: none carries its name
    /// (never joined, or renamed off its handle), or several do.
    #[error("E_MESH_PEER_UNRESOLVED")]
    MeshPeerUnresolved { machine: String, hint: String },

    /// A NetBird management API call failed for a reason other than
    /// authorization (that is `E_NOT_ADMIN`): unreachable, a rejected token,
    /// an unexpected status, or a peer record missing a setting the update
    /// must send back. Nothing was changed.
    #[error("E_NETBIRD_FAILED")]
    NetbirdFailed { hint: String },

    /// `goat acl` / `goat warrant` found no ACL declaration to read: no
    /// --declaration, no GOAT_ACL_DECLARATION_DIR, none in the active context,
    /// and the working directory is in no checkout that carries one. Nothing
    /// of any deployment is compiled in, so there is nothing to fall back to.
    #[error("E_ACL_DECLARATION_MISSING")]
    AclDeclarationMissing { hint: String },
}

impl GoatError {
    /// By-value form of [`append_hint`](Self::append_hint), for call sites
    /// that append command-specific guidance (an escape hatch such as an
    /// `--endpoint` flag) inside a `map_err` closure. Rendering is delegated,
    /// so the two cannot drift into two different-looking `hint:` stanzas.
    pub fn with_extra_hint(mut self, extra: &str) -> Self {
        self.append_hint(extra);
        self
    }

    /// The stable string code (no leading whitespace, no trailing
    /// punctuation). Used for grep + Phase E `goat explain` lookup.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProfileMissing { .. } => "E_PROFILE_MISSING",
            Self::ProfileDenied { .. } => "E_PROFILE_DENIED",
            Self::OffMesh { .. } => "E_OFF_MESH",
            Self::ClockSkew { .. } => "E_CLOCK_SKEW",
            Self::RouterUnreachable { .. } => "E_ROUTER_UNREACHABLE",
            Self::CertExpired { .. } => "E_CERT_EXPIRED",
            Self::DoctorInconclusive { .. } => "E_DOCTOR_INCONCLUSIVE",
            Self::PortalUnreachable { .. } => "E_PORTAL_UNREACHABLE",
            Self::PortalBadResponse { .. } => "E_PORTAL_BAD_RESPONSE",
            Self::PortalNotFound { .. } => "E_PORTAL_NOT_FOUND",
            Self::PortalRateLimited { .. } => "E_PORTAL_RATE_LIMITED",
            Self::PortalServerError { .. } => "E_PORTAL_SERVER_ERROR",
            Self::PortalRefused { .. } => "E_PORTAL_REFUSED",
            Self::TokenInvalid { .. } => "E_TOKEN_INVALID",
            Self::PortalForbidden { .. } => "E_PORTAL_FORBIDDEN",
            Self::NotAdmin { .. } => "E_NOT_ADMIN",
            Self::TopicConflict { .. } => "E_TOPIC_CONFLICT",
            Self::BundleInvalid { .. } => "E_BUNDLE_INVALID",
            Self::MachineUnresolved { .. } => "E_MACHINE_UNRESOLVED",
            Self::MeshPeerUnresolved { .. } => "E_MESH_PEER_UNRESOLVED",
            Self::NetbirdFailed { .. } => "E_NETBIRD_FAILED",
            Self::AclDeclarationMissing { .. } => "E_ACL_DECLARATION_MISSING",
        }
    }

    /// Exit code per docs/design/goat-cli.md §Exit codes.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::ProfileMissing { .. }
            | Self::ProfileDenied { .. }
            | Self::BundleInvalid { .. } => exit_codes::EXIT_PROFILE,
            Self::OffMesh { .. } | Self::RouterUnreachable { .. } => exit_codes::EXIT_OFF_MESH,
            Self::ClockSkew { .. } => exit_codes::EXIT_CERT,
            Self::CertExpired { .. } => exit_codes::EXIT_CERT,
            Self::DoctorInconclusive { .. } => exit_codes::EXIT_GENERIC,
            Self::PortalNotFound { .. } => exit_codes::EXIT_PORTAL_NOT_FOUND,
            Self::PortalRateLimited { .. } => exit_codes::EXIT_PORTAL_RATE_LIMITED,
            Self::PortalServerError { .. } => exit_codes::EXIT_PORTAL_SERVER_ERROR,
            Self::PortalUnreachable { .. }
            | Self::PortalRefused { .. }
            | Self::PortalBadResponse { .. }
            | Self::TokenInvalid { .. }
            | Self::PortalForbidden { .. }
            | Self::NotAdmin { .. } => exit_codes::EXIT_PORTAL,
            Self::TopicConflict { .. } => exit_codes::EXIT_GENERIC,
            Self::MachineUnresolved { .. } | Self::MeshPeerUnresolved { .. } => {
                exit_codes::EXIT_GENERIC
            }
            Self::NetbirdFailed { .. } => exit_codes::EXIT_PORTAL,
            Self::AclDeclarationMissing { .. } => exit_codes::EXIT_GENERIC,
        }
    }

    // Human prose, localized behind the English arbiter (ADR 1024 /
    // docs/design/goat-cli-i18n.md). `summary()` renders to stderr only
    // via `DisplayHuman` and is NEVER serialized into --json, so it is
    // free to localize. The `{ $context }` / `{ $endpoint }` / `{ $topic }`
    // values are contract tokens passed through the localized frame
    // verbatim. `code()` (E_*) and the error:/help:/code: line labels
    // stay invariant by rule.
    /// The short, one-line, goat-authored summary — no `help:`/`code:`
    /// stanzas. For space-constrained UI contexts (a TUI status line, a
    /// truncated header) that want *a* human sentence rather than the full
    /// cargo-style `DisplayHuman` render or the bare `E_*` code `Display`
    /// gives by default (`#[error("E_...")]` on the enum).
    pub fn short_summary(&self) -> String {
        self.summary()
    }

    fn summary(&self) -> String {
        use crate::i18n::LANGUAGE_LOADER;
        use i18n_embed_fl::fl;
        match self {
            Self::ProfileMissing { context, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-profile-missing-summary",
                    context = context.clone()
                )
            }
            Self::ProfileDenied { context, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-profile-denied-summary",
                    context = context.clone()
                )
            }
            Self::OffMesh { .. } => fl!(LANGUAGE_LOADER, "error-off-mesh-summary"),
            Self::ClockSkew { .. } => fl!(LANGUAGE_LOADER, "error-clock-skew-summary"),
            Self::RouterUnreachable { endpoint, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-router-unreachable-summary",
                    endpoint = endpoint.clone()
                )
            }
            Self::CertExpired { .. } => fl!(LANGUAGE_LOADER, "error-cert-expired-summary"),
            Self::DoctorInconclusive { .. } => {
                fl!(LANGUAGE_LOADER, "error-doctor-inconclusive-summary")
            }
            Self::PortalUnreachable { endpoint, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-portal-unreachable-summary",
                    endpoint = endpoint.clone()
                )
            }
            Self::PortalBadResponse { .. } => {
                fl!(LANGUAGE_LOADER, "error-portal-bad-response-summary")
            }
            Self::PortalNotFound { path, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-portal-not-found-summary",
                    path = shown_path(path)
                )
            }
            Self::PortalRateLimited { path, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-portal-rate-limited-summary",
                    path = shown_path(path)
                )
            }
            Self::PortalServerError { status, path, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-portal-server-error-summary",
                    status = status.to_string(),
                    path = shown_path(path)
                )
            }
            Self::PortalRefused { status, path, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-portal-refused-summary",
                    status = status.to_string(),
                    path = shown_path(path)
                )
            }
            Self::TokenInvalid { .. } => fl!(LANGUAGE_LOADER, "error-token-invalid-summary"),
            Self::PortalForbidden { .. } => fl!(LANGUAGE_LOADER, "error-portal-forbidden-summary"),
            Self::NotAdmin { .. } => fl!(LANGUAGE_LOADER, "error-not-admin-summary"),
            Self::TopicConflict { topic, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-topic-conflict-summary",
                    topic = topic.clone()
                )
            }
            Self::BundleInvalid { .. } => fl!(LANGUAGE_LOADER, "error-bundle-invalid-summary"),
            Self::MachineUnresolved { machine, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-machine-unresolved-summary",
                    machine = machine.clone()
                )
            }
            Self::MeshPeerUnresolved { machine, .. } => {
                fl!(
                    LANGUAGE_LOADER,
                    "error-mesh-peer-unresolved-summary",
                    machine = machine.clone()
                )
            }
            Self::NetbirdFailed { .. } => fl!(LANGUAGE_LOADER, "error-netbird-failed-summary"),
            Self::AclDeclarationMissing { .. } => {
                fl!(LANGUAGE_LOADER, "error-acl-declaration-missing-summary")
            }
        }
    }

    pub(crate) fn hint(&self) -> &str {
        match self {
            Self::ProfileMissing { hint, .. }
            | Self::ProfileDenied { hint, .. }
            | Self::OffMesh { hint }
            | Self::ClockSkew { hint }
            | Self::RouterUnreachable { hint, .. }
            | Self::CertExpired { hint }
            | Self::DoctorInconclusive { hint }
            | Self::PortalUnreachable { hint, .. }
            | Self::PortalBadResponse { hint }
            | Self::PortalNotFound { hint, .. }
            | Self::PortalRateLimited { hint, .. }
            | Self::PortalServerError { hint, .. }
            | Self::PortalRefused { hint, .. }
            | Self::TokenInvalid { hint }
            | Self::PortalForbidden { hint }
            | Self::NotAdmin { hint }
            | Self::TopicConflict { hint, .. }
            | Self::BundleInvalid { hint }
            | Self::MachineUnresolved { hint, .. }
            | Self::MeshPeerUnresolved { hint, .. }
            | Self::NetbirdFailed { hint }
            | Self::AclDeclarationMissing { hint } => hint,
        }
    }

    /// Append an extra recovery hint to this error's existing help text.
    /// The additional line is rendered as a separate `hint:` stanza between
    /// the existing `help:` and `code:` lines.
    pub fn append_hint(&mut self, extra: &str) {
        if extra.is_empty() {
            return;
        }
        match self {
            Self::ProfileMissing { hint, .. }
            | Self::ProfileDenied { hint, .. }
            | Self::OffMesh { hint }
            | Self::ClockSkew { hint }
            | Self::RouterUnreachable { hint, .. }
            | Self::CertExpired { hint }
            | Self::DoctorInconclusive { hint }
            | Self::PortalUnreachable { hint, .. }
            | Self::PortalBadResponse { hint }
            | Self::PortalNotFound { hint, .. }
            | Self::PortalRateLimited { hint, .. }
            | Self::PortalServerError { hint, .. }
            | Self::PortalRefused { hint, .. }
            | Self::TokenInvalid { hint }
            | Self::PortalForbidden { hint }
            | Self::NotAdmin { hint }
            | Self::TopicConflict { hint, .. }
            | Self::BundleInvalid { hint }
            | Self::MachineUnresolved { hint, .. }
            | Self::MeshPeerUnresolved { hint, .. }
            | Self::NetbirdFailed { hint }
            | Self::AclDeclarationMissing { hint } => {
                hint.push('\n');
                hint.push_str("  hint: ");
                hint.push_str(extra);
            }
        }
    }
}

/// The request path as a summary shows it; a synthesized error with no
/// request behind it reads as the portal itself.
fn shown_path(path: &str) -> String {
    if path.is_empty() {
        "the portal API".to_string()
    } else {
        path.to_string()
    }
}

/// The error for an HTTP status the portal ANSWERED with that no command
/// curates (#2087). A 404, a 429 or a 5xx is the portal telling the operator
/// something; filing it under `E_PORTAL_UNREACHABLE` sent people to check
/// their network, DNS and tunnel while the portal was up. `server` is the
/// portal's own message stanza (`server_line`), or empty.
pub fn portal_answer(
    status: u16,
    path: &str,
    retry_after: Option<&str>,
    server: &str,
) -> GoatError {
    let path = path.to_string();
    match status {
        404 => GoatError::PortalNotFound {
            path,
            hint: format!(
                "the portal is up and answered HTTP 404: what this command named (an org, topic, \
                 schema, machine or person) is not registered there. Check the name, or list what \
                 exists (`goat vendor ls`, `goat topic ls`).{server}"
            ),
        },
        429 => {
            let when = match retry_after {
                Some(r) if r.bytes().all(|b| b.is_ascii_digit()) => format!("retry after {r} s"),
                Some(r) => format!("retry after {r}"),
                None => "wait a few seconds and retry".to_string(),
            };
            GoatError::PortalRateLimited {
                path,
                retry_after: retry_after.map(str::to_string),
                hint: format!(
                    "the portal is up and is rate-limiting this client (HTTP 429): {when}. \
                     A script calling goat in a tight loop is the usual cause; nothing is wrong \
                     with your network.{server}"
                ),
            }
        }
        500..=599 => GoatError::PortalServerError {
            status,
            path,
            hint: format!(
                "the portal is up and failed on its own side (HTTP {status}); nothing on your \
                 network or credential is at fault. Retry{}; if it persists, tell a platform \
                 admin — the portal's log on the goatnet's management host names the cause.{server}",
                retry_after
                    .map(|r| format!(" after {r}"))
                    .unwrap_or_default()
            ),
        },
        _ => GoatError::PortalRefused {
            status,
            path,
            hint: format!(
                "the portal is up and refused this request (HTTP {status}). A request this goat \
                 build sends that the portal does not accept is usually a version skew — `goat \
                 doctor` shows whether goat is behind.{server}"
            ),
        },
    }
}

/// Render a `GoatError` cargo-style for human stderr. The error code
/// is appended on its own line so a script can `grep '^code:'`.
pub struct DisplayHuman<'a>(pub &'a GoatError);

impl fmt::Display for DisplayHuman<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {}", self.0.summary())?;
        writeln!(f, "  help: {}", self.0.hint())?;
        write!(f, "  code: {}", self.0.code())
    }
}

/// The hint every operator-gated portal 403 should carry.
///
/// One text, one place. Before this, each command wrote its own: most said
/// only that operator credentials are required, which is true and leaves the
/// reader with nowhere to go. The overwhelmingly common cause is not missing
/// authority but the WRONG ONE of the person's two credentials — `goat profile
/// init` installs the `vp_` participant token out of the bundle, while
/// operator-gated endpoints want the `op_` key, which is collected separately
/// from the identity console and is deliberately never shipped inside a
/// bundle. Told only "requires an operator token", people ask for their bundle
/// to be re-issued, which cannot help and rotates their cert.
///
/// `action` names what was refused, in the imperative, so the first line reads
/// as a sentence: e.g. "setting a machine quota".
pub fn operator_403_hint(action: &str, status: impl std::fmt::Display) -> String {
    format!(
        "portal denied this request (HTTP {status}): {action} needs your OPERATOR key \
         (op_…), and this profile is using a different token. Nothing was changed.\n\
         \n\
         `goat profile init` installs the vp_ participant token from your bundle. The op_ \
         key is a separate credential you request and collect once — `goat admin \
         operator-key request --reason ...` then `collect <id>` on this profile's own token, \
         or the identity console's /operator-key page; it is never inside a bundle, so \
         re-issuing the bundle cannot fix this.\n\
         \n\
         Point this command at it, either way (bash/zsh shown; see below for Windows):\n\
         \x20 GOAT_PORTAL_TOKEN=op_… goat …\n\
         \x20 or use a paired operator context: --context <env>-op\n\
         \x20   cp -r ~/.goat/contexts/<env> ~/.goat/contexts/<env>-op\n\
         \x20   printf %s 'op_…' > ~/.goat/contexts/<env>-op/portal.token\n\
         \n\
         Windows (PowerShell): $env:GOAT_PORTAL_TOKEN='op_…'; goat …\n\
         \x20 or: Copy-Item -Recurse $env:USERPROFILE\\.goat\\contexts\\<env> \
         $env:USERPROFILE\\.goat\\contexts\\<env>-op\n\
         \x20   Set-Content -NoNewline $env:USERPROFILE\\.goat\\contexts\\<env>-op\\portal.token 'op_…'\n\
         \n\
         Do not overwrite the token in your main context — the vp_ key there is what the \
         participant-side calls use. If you hold no op_ key at all, `goat admin \
         operator-key request --reason ...` (no handle) is how a platform admin asks for \
         one; until it is approved, every other operator verb is not for your role."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portal_answer_is_classified_by_status_never_unreachable() {
        let cases = [
            (404, "E_PORTAL_NOT_FOUND", exit_codes::EXIT_PORTAL_NOT_FOUND),
            (429, "E_PORTAL_RATE_LIMITED", exit_codes::EXIT_PORTAL_RATE_LIMITED),
            (500, "E_PORTAL_SERVER_ERROR", exit_codes::EXIT_PORTAL_SERVER_ERROR),
            (503, "E_PORTAL_SERVER_ERROR", exit_codes::EXIT_PORTAL_SERVER_ERROR),
            (599, "E_PORTAL_SERVER_ERROR", exit_codes::EXIT_PORTAL_SERVER_ERROR),
            (400, "E_PORTAL_REFUSED", exit_codes::EXIT_PORTAL),
            (405, "E_PORTAL_REFUSED", exit_codes::EXIT_PORTAL),
            (422, "E_PORTAL_REFUSED", exit_codes::EXIT_PORTAL),
        ];
        for (status, code, exit) in cases {
            let e = portal_answer(status, "/api/orgs/acme/topics", None, "");
            assert_eq!(e.code(), code, "status {status}");
            assert_eq!(e.exit_code(), exit, "status {status}");
            let shown = DisplayHuman(&e).to_string();
            assert!(shown.contains("/api/orgs/acme/topics"), "{shown}");
            assert!(!shown.contains("unreachable"), "{shown}");
        }
    }

    #[test]
    fn the_exit_codes_tell_retryable_from_absent_from_down() {
        let codes = [
            exit_codes::EXIT_PORTAL,
            exit_codes::EXIT_PORTAL_NOT_FOUND,
            exit_codes::EXIT_PORTAL_RATE_LIMITED,
            exit_codes::EXIT_PORTAL_SERVER_ERROR,
        ];
        let mut sorted = codes.to_vec();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }

    #[test]
    fn a_429_carries_the_portal_retry_after() {
        let secs = portal_answer(429, "/api/catalog", Some("7"), "");
        assert!(secs.hint().contains("retry after 7 s"), "{}", secs.hint());
        let date = portal_answer(429, "/api/catalog", Some("Wed, 21 Oct 2026 07:28:00 GMT"), "");
        assert!(date.hint().contains("retry after Wed, 21 Oct 2026"), "{}", date.hint());
        let none = portal_answer(429, "/api/catalog", None, "");
        assert!(none.hint().contains("wait a few seconds"), "{}", none.hint());
        match secs {
            GoatError::PortalRateLimited { retry_after, .. } => {
                assert_eq!(retry_after.as_deref(), Some("7"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_server_message_rides_under_the_answer() {
        let e = portal_answer(503, "/api/catalog", None, "\n  server: restarting");
        assert!(e.hint().ends_with("\n  server: restarting"), "{}", e.hint());
    }

    #[test]
    fn a_synthesized_answer_with_no_path_names_the_portal() {
        let shown = DisplayHuman(&portal_answer(404, "", None, "")).to_string();
        assert!(shown.contains("the portal API"), "{shown}");
    }
}

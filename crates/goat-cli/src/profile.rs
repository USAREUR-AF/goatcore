// Profile + context resolution — directory location, context selection,
// paths to per-context cert/key/token files. Phase A: filesystem-path
// resolution only; Phase B fills in init/show/rotate + reading config.toml
// for `current_context`. Layout is context-shaped from day one even though
// Phase A only ever uses `default` — see docs/design/goat-cli.md
// "Profile + context model."

use std::path::{Path, PathBuf};

pub const DEFAULT_CONTEXT: &str = "default";

/// A router endpoint given on the command line (`--router`) or in
/// `$GOAT_ROUTER_ENDPOINT`, set once by `main` before any command runs.
///
/// It replaces the context's `router_endpoint` for this one invocation and
/// changes nothing else: the certificate, key and CA roots still come from
/// the context. Its use is reaching a goatnet's router through a local port
/// forward (`env-door.sh <goatnet> --forward <ip:port>`) from a laptop that
/// is not a peer of that goatnet. Nothing is written to the profile.
static ROUTER_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Record the `--router` / `$GOAT_ROUTER_ENDPOINT` value. Empty is ignored.
pub fn set_router_override(endpoint: &str) {
    let endpoint = endpoint.trim();
    if !endpoint.is_empty() {
        let _ = ROUTER_OVERRIDE.set(endpoint.to_string());
    }
}

/// The `--router` override, when one was given.
pub fn router_override() -> Option<&'static str> {
    ROUTER_OVERRIDE.get().map(String::as_str)
}

/// The router endpoint a command connects to: the `--router` override when
/// given, else `router_endpoint` read from `toml_body`. Every reader of the
/// router endpoint goes through this, so the override reaches all of them.
pub fn resolve_router_endpoint(toml_body: &str) -> Option<String> {
    pick_router_endpoint(router_override(), toml_body)
}

fn pick_router_endpoint(over: Option<&str>, toml_body: &str) -> Option<String> {
    over.map(str::to_string).or_else(|| {
        extract_toml_field(toml_body, "router_endpoint").filter(|v| !v.trim().is_empty())
    })
}

/// Resolve the user's home directory the ONE correct way: `$HOME` on Unix,
/// falling back to `%USERPROFILE%` on Windows (where `$HOME` is usually
/// unset). Before this, three call sites each re-implemented this lookup and
/// one of them (`bundle::ca_roots_dir`'s fallback) checked `$HOME` only —
/// silently resolving to nothing on a Windows box with no `$HOME` set. Every
/// caller that needs "the user's home dir" should go through this function
/// instead of hand-rolling the env-var precedence again.
pub fn resolve_home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "neither $HOME nor %USERPROFILE% is set; pass --profile-dir or set GOAT_PROFILE_DIR"
            )
        })
}

/// Read `current_context = "<name>"` from a `config.toml` path. Returns
/// `None` when the file is absent or the key is missing, so callers can
/// `.or_else()` into their own fallback (`resolve` → `"default"`). This
/// is the single parser shared by `resolve` (the active-context layer)
/// and `cmd::context` (the `ls`/`use`/`show` surface) so the two can't
/// drift. Hand-rolled rather than a TOML dep — the file has one key we
/// wrote, same lightweight pattern as the profile.toml extractors.
pub(crate) fn read_current_context(config_path: &Path) -> Option<String> {
    let body = std::fs::read_to_string(config_path).ok()?;
    extract_toml_field(&body, "current_context").filter(|s| !s.is_empty())
}

/// Resolved profile + active context. The profile directory may not
/// exist yet — that's the state before `goat profile init` runs.
#[derive(Debug, Clone)]
pub struct Profile {
    pub dir: PathBuf,
    pub context: String,
}

/// What a probe of the active context's profile.toml actually found.
/// `Absent` and `Denied` are different facts: one means run `profile init`,
/// the other means the profile is there and THIS user cannot read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitState {
    Initialized,
    Absent,
    Denied,
}

#[allow(dead_code)] // Phase A scaffold — Phase B (`profile init` / `doctor`) consumes these.
impl Profile {
    /// Resolve the active profile directory and context name. Precedence
    /// for profile dir: `--profile-dir` flag > `$GOAT_PROFILE_DIR` env >
    /// `$HOME/.goat`. Precedence for context: `--context` flag >
    /// `$GOAT_CONTEXT` env > `current_context` in `config.toml` >
    /// `"default"`.
    ///
    /// The `config.toml` layer is what makes `goat context use <name>`
    /// persistent: without it, every command silently fell back to
    /// `default` and only the explicit `--context` flag / `$GOAT_CONTEXT`
    /// were honored, so a switched context never took effect.
    pub fn resolve(
        override_dir: Option<&str>,
        override_context: Option<&str>,
    ) -> anyhow::Result<Self> {
        let dir = match override_dir {
            Some(p) => PathBuf::from(p),
            None => resolve_home_dir()?.join(".goat"),
        };
        let context = override_context
            .map(str::to_owned)
            .or_else(|| std::env::var("GOAT_CONTEXT").ok())
            .or_else(|| read_current_context(&dir.join("config.toml")))
            .unwrap_or_else(|| DEFAULT_CONTEXT.to_owned());
        Ok(Self { dir, context })
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.toml")
    }
    pub fn contexts_dir(&self) -> PathBuf {
        self.dir.join("contexts")
    }
    pub fn context_dir(&self) -> PathBuf {
        self.contexts_dir().join(&self.context)
    }

    pub fn profile_toml_path(&self) -> PathBuf {
        self.context_dir().join("profile.toml")
    }
    pub fn cert_path(&self) -> PathBuf {
        self.context_dir().join("mtls.cert.pem")
    }
    pub fn key_path(&self) -> PathBuf {
        self.context_dir().join("mtls.key.pem")
    }
    pub fn ca_roots_path(&self) -> PathBuf {
        self.context_dir().join("ca-roots.pem")
    }
    pub fn portal_token_path(&self) -> PathBuf {
        self.context_dir().join("portal.token")
    }
    pub fn bundle_path(&self) -> PathBuf {
        self.context_dir().join("bundle.cbor")
    }

    /// True when the active context's `profile.toml` exists. Phase B
    /// will firm this up to verify cert/key/token presence too.
    pub fn is_initialized(&self) -> bool {
        matches!(self.init_state(), InitState::Initialized)
    }

    /// Three-way probe behind `is_initialized`. `Path::exists()` swallows
    /// EACCES, so a root-owned profile dir (the pod's first-boot writes 0700
    /// root) used to read as "no profile" for an unprivileged user and surface
    /// E_PROFILE_MISSING for a profile that is right there. Callers that raise
    /// user-facing errors go through `require_initialized` instead, which
    /// names the real condition.
    ///
    /// The probe opens for read rather than stat-ing: every consumer's next
    /// step is a read, and `metadata()` passes a stat-able-but-unreadable
    /// file (dir searchable, file mode 000), which would re-surface the
    /// wrong-hint bug one call later. A single open covers both the
    /// dir-search denial and the file-read denial.
    pub fn init_state(&self) -> InitState {
        match std::fs::File::open(self.profile_toml_path()) {
            Ok(_) => InitState::Initialized,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => InitState::Denied,
            Err(_) => InitState::Absent,
        }
    }

    /// The one place a two-state failure becomes a user-facing error: Denied
    /// maps to E_PROFILE_DENIED, Absent to E_PROFILE_MISSING. Every command
    /// that needs an initialized profile calls this instead of hand-rolling
    /// the check, so a new state is a one-file change.
    ///
    /// The Denied hint deliberately does NOT say "re-run with sudo": bare
    /// sudo resets $HOME (env_reset), so goat would probe /root/.goat and
    /// report a fresh missing-profile — a second dead end.
    pub fn require_initialized(&self) -> Result<(), crate::errors::GoatError> {
        match self.init_state() {
            InitState::Initialized => Ok(()),
            InitState::Denied => Err(crate::errors::GoatError::ProfileDenied {
                context: self.context.clone(),
                hint: format!(
                    "the profile at {dir} exists but this user cannot read it \
                     (permission denied). Fix the ownership (`sudo chown -R $USER \
                     {dir}`) or run as the owning user with `sudo -E goat …` — \
                     bare `sudo` resets $HOME and would look in /root/.goat \
                     instead.",
                    dir = self.context_dir().display()
                ),
            }),
            InitState::Absent => Err(crate::errors::GoatError::ProfileMissing {
                context: self.context.clone(),
                hint: format!(
                    "no profile at {}. Run `goat profile init <bundle.cbor>` first.",
                    self.context_dir().display()
                ),
            }),
        }
    }

    /// Stronger check for the moment right before opening a real fabric
    /// (Zenoh) session: `require_initialized()` only confirms `profile.toml`
    /// is present+readable, but a session open also needs `mtls.cert.pem`,
    /// `mtls.key.pem`, and `ca-roots.pem`. Any of those three can be missing
    /// or unreadable — a partial `profile init`, disk full mid-write, a sync
    /// tool skipping dotfile-like names, antivirus quarantine — while
    /// `profile.toml` is perfectly fine, and until now nothing checked for
    /// that: the missing file was handed straight to Zenoh, which fails with
    /// its own raw, multi-line, internals-revealing error text instead of
    /// goat's classified `ProfileMissing`/`ProfileDenied`.
    ///
    /// Every call site that is about to `zenoh::open(...)` should call this
    /// FIRST, so a missing/unreadable identity file is reported clearly and
    /// by name before the raw Zenoh error is ever generated.
    pub fn require_mesh_ready(&self) -> Result<(), crate::errors::GoatError> {
        self.require_initialized()?;
        for (path, label) in [
            (self.cert_path(), "mtls.cert.pem"),
            (self.key_path(), "mtls.key.pem"),
            (self.ca_roots_path(), "ca-roots.pem"),
        ] {
            match std::fs::File::open(&path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    return Err(crate::errors::GoatError::ProfileDenied {
                        context: self.context.clone(),
                        hint: format!(
                            "{label} at {} exists but this user cannot read it (permission \
                             denied). Fix the ownership (`sudo chown -R $USER {}`) or run as \
                             the owning user.",
                            path.display(),
                            self.context_dir().display()
                        ),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(crate::errors::GoatError::ProfileMissing {
                        context: self.context.clone(),
                        hint: format!(
                            "{label} is missing at {} — re-run `goat profile init <bundle.cbor>`.",
                            path.display()
                        ),
                    });
                }
                // EISDIR, EIO, ELOOP: the file is there but broken. A re-init
                // may fix it; the operator needs to see the real reason first.
                Err(e) => {
                    return Err(crate::errors::GoatError::ProfileMissing {
                        context: self.context.clone(),
                        hint: format!(
                            "{label} at {} could not be opened ({e}) — fix the file, or re-run \
                             `goat profile init <bundle.cbor>`.",
                            path.display()
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// The router endpoint this context's fabric sessions are opened
    /// against, for error messages: `router_endpoint` from profile.toml, or
    /// the literal `router` when the profile cannot say (by then the session
    /// open itself has already failed loudly).
    pub fn router_endpoint_label(&self) -> String {
        self.goatnet_endpoints()
            .router_endpoint
            .unwrap_or_else(|| "router".to_string())
    }

    /// Classify a failed profile-file read: EACCES is a denied profile, not
    /// a missing one (the race between `require_initialized` and the read,
    /// or a per-file mode).
    pub(crate) fn read_error(&self, path: &Path, e: std::io::Error) -> crate::errors::GoatError {
        let hint = format!("read {}: {e}", path.display());
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            crate::errors::GoatError::ProfileDenied {
                context: self.context.clone(),
                hint,
            }
        } else {
            crate::errors::GoatError::ProfileMissing {
                context: self.context.clone(),
                hint,
            }
        }
    }

    /// Read any file under the active context's directory, classifying a
    /// failure the same way `portal_client()` already does for
    /// profile.toml/portal.token: `PermissionDenied` → `ProfileDenied`,
    /// anything else → `ProfileMissing`, both naming the exact path. Use
    /// this instead of a bare `std::fs::read_to_string(...)?` anywhere a
    /// profile-context file is read — a bare `?` surfaces as an unattributed
    /// `No such file or directory (os error 2)` with no filename and no
    /// remediation, which is exactly the class of unclear failure this
    /// exists to close off.
    pub(crate) fn read_classified(&self, path: &Path) -> Result<String, crate::errors::GoatError> {
        std::fs::read_to_string(path).map_err(|e| self.read_error(path, e))
    }

    /// Construct a PortalClient from the active context's
    /// profile.toml + portal.token. Returns ProfileMissing when the
    /// context isn't initialized. Wired by every Phase C subcommand
    /// that talks to the portal (vendor, topic).
    pub fn portal_client(
        &self,
    ) -> Result<panoscope_core::portal::PortalClient, crate::errors::GoatError> {
        self.require_initialized()?;
        let toml = std::fs::read_to_string(self.profile_toml_path())
            .map_err(|e| self.read_error(&self.profile_toml_path(), e))?;
        let portal_endpoint = extract_toml_field(&toml, "portal_endpoint").ok_or_else(|| {
            crate::errors::GoatError::ProfileMissing {
                context: self.context.clone(),
                hint: format!(
                    "{} missing portal_endpoint — re-run `goat profile init`",
                    self.profile_toml_path().display()
                ),
            }
        })?;
        let token = match portal_token_from_env() {
            Some(t) => t,
            None => std::fs::read_to_string(self.portal_token_path())
                .map_err(|e| self.read_error(&self.portal_token_path(), e))?
                .trim()
                .to_string(),
        };
        // Delegated attribution: when the human behind the shared
        // operator key is known — `$GOAT_OPERATOR_HANDLE` or the profile's
        // `operator_handle` — every portal request names them, so
        // sponsor/actor attribution (e.g. an invite's "invited by …") records
        // the person. Deliberately no context-name fallback here: a goatnet
        // name is not a person.
        let actor = std::env::var("GOAT_OPERATOR_HANDLE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| extract_toml_field(&toml, "operator_handle"));
        // The onboarding engine moved from the portal to goat-id-console
        // (docs/design/onboarding-extraction-plan.md). A distinct onboard endpoint
        // re-points `goat admin invite`/`queue` at the console; without one, onboard
        // calls fall back to the portal endpoint (pre-cutover behaviour), so old
        // profiles are unaffected. The cert-mint (`/api/orgs`), whoami, and access
        // surfaces always stay on the portal endpoint. Precedence: the
        // `GOAT_ONBOARD_ENDPOINT` env override (lets an operator/runner re-point
        // without re-issuing a bundle) beats the profile's `onboard_endpoint`.
        let onboard_endpoint = std::env::var("GOAT_ONBOARD_ENDPOINT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| extract_toml_field(&toml, "onboard_endpoint"));
        let client = panoscope_core::portal::PortalClient::new(portal_endpoint, token);
        let client = match onboard_endpoint {
            Some(url) => client.with_onboard_endpoint(&url),
            None => client,
        };
        Ok(match actor {
            Some(a) => client.with_operator_actor(&a),
            None => client,
        })
    }

    /// Read a single scalar field from the active context's `profile.toml`.
    /// Thin public wrapper over the same hand-rolled extractor the mesh/portal
    /// derivations use — subcommands that need one more optional profile key
    /// (e.g. `goat admin storm brake`'s `mgmt_ssh_host` / `mgmt_ssh_user` /
    /// `storm_brake_shed_units`) read it through here instead of re-opening the
    /// file with their own parser. Returns `None` when the profile or key is
    /// absent, so callers `.or_else()` into env/flag/default precedence.
    pub fn profile_field(&self, key: &str) -> Option<String> {
        let toml = std::fs::read_to_string(self.profile_toml_path()).ok()?;
        extract_toml_field(&toml, key).filter(|s| !s.is_empty())
    }

    /// The active context's portal bearer token, if present. Used by
    /// surfaces (e.g. `goat ui`'s panoscope snapshot read) that need
    /// the raw token rather than a full PortalClient. Returns None when
    /// the token file is absent/unreadable.
    pub fn portal_token(&self) -> Option<String> {
        portal_token_from_env().or_else(|| {
            std::fs::read_to_string(self.portal_token_path())
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    }

    /// Construct a NetbirdClient for the active context's mesh-management
    /// API — the surface `goat admin peer` uses to list and *eject* peers
    /// (the authoritative mesh-first cut).
    ///
    /// mgmt URL precedence: `mgmt_url_override` (CLI `--mgmt-url`), then
    /// `GOAT_NB_MGMT_URL`, then `<CONTEXT>_NB_MGMT_URL`, then derived from the
    /// profile's `portal_endpoint` host (`portal.<site>.netbird.<backbone>`
    /// becomes `netbird.<backbone>`).
    ///
    /// Admin-token precedence: `GOAT_NB_API_TOKEN`, then
    /// `<CONTEXT>_NB_API_TOKEN`. The NetBird admin token is a powerful secret,
    /// so it is env-driven only — never read from (or written to) the profile.
    ///
    /// Errors name the exact missing env var so an operator can fix it in one
    /// step, even mid-incident.
    pub fn netbird_client(
        &self,
        mgmt_url_override: Option<&str>,
    ) -> Result<panoscope_core::netbird::NetbirdClient, crate::errors::GoatError> {
        let ctx_env = self.context.to_uppercase().replace('-', "_");
        let token = std::env::var("GOAT_NB_API_TOKEN")
            .ok()
            .or_else(|| std::env::var(format!("{ctx_env}_NB_API_TOKEN")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| crate::errors::GoatError::ProfileMissing {
                context: self.context.clone(),
                hint: format!(
                    "NetBird admin token not set — export ${ctx_env}_NB_API_TOKEN \
                     (or $GOAT_NB_API_TOKEN). This is an admin secret; goat reads it \
                     from the environment, never the profile."
                ),
            })?;

        let mgmt_url = mgmt_url_override
            .map(|s| s.to_string())
            .or_else(|| std::env::var("GOAT_NB_MGMT_URL").ok())
            .or_else(|| std::env::var(format!("{ctx_env}_NB_MGMT_URL")).ok())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.derive_nb_mgmt_url())
            .ok_or_else(|| crate::errors::GoatError::ProfileMissing {
                context: self.context.clone(),
                hint: format!(
                    "NetBird mgmt URL not resolved — pass --mgmt-url, or export \
                     $GOAT_NB_MGMT_URL / ${ctx_env}_NB_MGMT_URL \
                     (e.g. https://netbird.<backbone-domain>)."
                ),
            })?;

        Ok(panoscope_core::netbird::NetbirdClient::new(mgmt_url, token))
    }

    /// Best-effort derive the NetBird mgmt URL from the profile's
    /// `portal_endpoint` host. `portal.<site>.netbird.<backbone>` becomes
    /// `https://netbird.<backbone>`. Returns None if the profile/host shape
    /// doesn't match (caller then requires explicit config).
    fn derive_nb_mgmt_url(&self) -> Option<String> {
        let toml = std::fs::read_to_string(self.profile_toml_path()).ok()?;
        let portal = extract_toml_field(&toml, "portal_endpoint")?;
        let host = portal
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let host = host.split('/').next().unwrap_or(host);
        let parts: Vec<&str> = host.split('.').collect();
        let idx = parts.iter().position(|p| *p == "netbird")?;
        if idx + 1 >= parts.len() {
            return None;
        }
        let backbone = parts[idx + 1..].join(".");
        if backbone.is_empty() {
            return None;
        }
        Some(format!("https://netbird.{backbone}"))
    }

    /// Best-effort derive a sibling service's host + site_id from the profile's
    /// `portal_endpoint`. `portal.<site>.netbird.<backbone>` with `label` yields
    /// `("https://<label>.<site>.netbird.<backbone>", "<site>")` — the
    /// sibling-surface swap used across the inspection surfaces. Returns None
    /// if the host doesn't match the `<label>.<site>.netbird.<backbone>` shape
    /// (caller then requires `--url`/`--site`).
    pub fn derive_sibling(&self, label: &str) -> Option<(String, String)> {
        let toml = std::fs::read_to_string(self.profile_toml_path()).ok()?;
        let portal = extract_toml_field(&toml, "portal_endpoint")?;
        let scheme = if portal.starts_with("http://") {
            "http"
        } else {
            "https"
        };
        let host = portal
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let host = host.split('/').next().unwrap_or(host);
        let parts: Vec<&str> = host.split('.').collect();
        let idx = parts.iter().position(|p| *p == "netbird")?;
        if idx < 1 {
            return None; // no <site> label before "netbird"
        }
        let site = parts[idx - 1].to_string();
        // Swap the leading label (e.g. "portal") for `label`, keeping
        // <site>.netbird.<backbone> intact.
        let rest = parts[1..].join(".");
        if rest.is_empty() {
            return None;
        }
        Some((format!("{scheme}://{label}.{rest}"), site))
    }

    /// The `<site>` label of the profile's `portal_endpoint`
    /// (`portal.<site>.netbird.<backbone>`), or None for another shape.
    pub fn derive_site(&self) -> Option<String> {
        self.derive_sibling("portal").map(|(_, s)| s)
    }
}

/// Tiny TOML field extractor — same shape as cmd::doctor::parse_endpoints.
/// Kept here so subcommand modules don't each re-implement it.
/// Per-goatnet service endpoints the native onboard needs, read from the active
/// context's `profile.toml`.
///
/// WHY THIS EXISTS. These four facts already have a source of truth in
/// `ops/ansible/group_vars/<site>_hosts/vars.yml`, and they were hand-copied into
/// three `match` arms in `cmd/admin.rs`. Any goatnet stood up after those arms
/// were last edited is invisible to the onboarder, so its first person cannot be
/// minted until a goat-cli RELEASE ships.
///
/// A context that declares these overrides the built-in table, so a NEW goatnet
/// is a config change rather than a release. The table remains as the fallback
/// for the goatnets already in it; nothing existing changes behaviour.
#[derive(Debug, Clone, Default)]
pub struct GoatnetEndpoints {
    /// `tls/<host>:7447` — mirrors `portal_router_endpoint`.
    pub router_endpoint: Option<String>,
    /// NetBird management API base — mirrors `public_mgmt_fqdn`.
    pub netbird_mgmt_url: Option<String>,
    /// Name of the env var holding this goatnet's NetBird PAT.
    pub netbird_token_env: Option<String>,
    /// ACL group-name prefix — mirrors `netbird_acl_taxonomy[site].site_prefix`.
    pub netbird_site_prefix: Option<String>,
    /// Intra-mesh PQC — mirrors `rosenpass_enabled`. Absent = leave the table's
    /// value alone rather than guessing; turning a transport ON by omission is
    /// the one mistake this must not make.
    pub netbird_rosenpass: Option<bool>,
    /// Ory public auth URL for this goatnet.
    pub ory_auth_url: Option<String>,
    /// SSH delegate for the site's Kratos admin API, and the account to use.
    pub ory_ssh_host: Option<String>,
    pub ory_ssh_user: Option<String>,
    /// This context's goatnet name. Names the
    /// declared roster an org-scope cascade resolves members from
    /// (`docs/operators/<goatnet>/roster.json`, ADR 1172). Distinct from the
    /// CONTEXT name, which is the operator's own label for a set of endpoints
    /// and is routinely something else (`acme-op`, `demoorg`); a cascade must
    /// not guess a goatnet from it.
    pub goatnet: Option<String>,
    /// The demo service `goat demo` opens when no name is given: its host label
    /// and its key prefix (`<name>.<site>.netbird.<backbone>`, `<name>/<site>/**`).
    pub demo_service: Option<String>,
    /// Where operators report translation errors (the machine-translation notice).
    pub i18n_feedback_email: Option<String>,
}

impl Profile {
    /// The per-goatnet endpoints that apply to `env` (an `--env` goatnet).
    ///
    /// A command can act on a goatnet other than the active context's
    /// (`goat admin onboard x --env zagros-prod` from the zagros context). The
    /// active profile's facts describe ITS goatnet, so they apply only when the
    /// profile names `env` or names no goatnet at all (a profile written before
    /// `goat context refresh`, whose behaviour is unchanged). When it names a
    /// different goatnet, the facts come from the sibling context whose profile
    /// names `env`; with no such context nothing is overridden.
    pub fn goatnet_endpoints_for(&self, env: Option<&str>) -> GoatnetEndpoints {
        let own = self.goatnet_endpoints();
        let (Some(env), Some(named)) = (env, own.goatnet.as_deref()) else {
            return own;
        };
        if named == env {
            return own;
        }
        let Ok(entries) = std::fs::read_dir(self.contexts_dir()) else {
            return GoatnetEndpoints::default();
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".bak") || name == self.context {
                continue;
            }
            let sibling = Profile {
                dir: self.dir.clone(),
                context: name,
            };
            let g = sibling.goatnet_endpoints();
            if g.goatnet.as_deref() == Some(env) {
                return g;
            }
        }
        GoatnetEndpoints::default()
    }

    /// Read whatever per-goatnet endpoints this context declares. Every field is
    /// optional: a profile written before this existed yields all-None and the
    /// caller keeps its built-in value, so old bundles are unaffected.
    pub fn goatnet_endpoints(&self) -> GoatnetEndpoints {
        let toml = match std::fs::read_to_string(self.profile_toml_path()) {
            Ok(t) => t,
            Err(_) => return GoatnetEndpoints::default(),
        };
        let g = |k: &str| extract_toml_field(&toml, k).filter(|v| !v.trim().is_empty());
        GoatnetEndpoints {
            router_endpoint: router_override()
                .map(str::to_string)
                .or_else(|| g("router_endpoint")),
            netbird_mgmt_url: g("netbird_mgmt_url"),
            netbird_token_env: g("netbird_token_env"),
            netbird_site_prefix: g("netbird_site_prefix"),
            netbird_rosenpass: g("netbird_rosenpass").and_then(|v| match v.trim() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None, // an unparseable value must not silently mean false
            }),
            ory_auth_url: g("ory_auth_url"),
            ory_ssh_host: g("ory_ssh_host"),
            ory_ssh_user: g("ory_ssh_user"),
            goatnet: g("goatnet"),
            demo_service: g("demo_service"),
            i18n_feedback_email: g("i18n_feedback_email"),
        }
    }
}

/// The single, correct hand-rolled `key = "value"` line reader — every
/// command that needs one scalar field out of a profile.toml/config.toml
/// body should call this instead of writing its own copy. Six near-identical
/// copies of this function existed across the codebase before this change;
/// four of them (this one included) used `.trim_start_matches('=')`, which
/// strips zero-or-more `=` WITHOUT requiring one to be present — so a
/// requested key that is a string-prefix of a different key on the same line
/// (e.g. asking for `"port"` on a line that actually reads `port_extra = 5`)
/// silently returns a garbage value instead of correctly reporting "not
/// found". `strip_prefix('=')` requires the `=` to actually be there, so a
/// non-matching line correctly falls through instead of being misread.
pub(crate) fn extract_toml_field(toml: &str, key: &str) -> Option<String> {
    for line in toml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let v = rest.trim().trim_matches('"').to_string();
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn router_override_replaces_the_profile_endpoint_only_when_given() {
        let body = "router_endpoint = \"tls/100.64.0.9:7447\"\n";
        assert_eq!(
            super::pick_router_endpoint(None, body).as_deref(),
            Some("tls/100.64.0.9:7447")
        );
        assert_eq!(
            super::pick_router_endpoint(Some("tls/127.0.0.1:17447"), body).as_deref(),
            Some("tls/127.0.0.1:17447")
        );
        // An override reaches a profile that names no router at all.
        assert_eq!(
            super::pick_router_endpoint(Some("tls/127.0.0.1:17447"), "goatnet = \"x\"\n")
                .as_deref(),
            Some("tls/127.0.0.1:17447")
        );
        assert_eq!(
            super::pick_router_endpoint(None, "router_endpoint = \"  \"\n"),
            None
        );
    }

    use super::*;

    fn write_config(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("config.toml"), body).unwrap();
    }

    fn ctx_with(dir: &std::path::Path, name: &str, body: &str) -> Profile {
        let d = dir.join("contexts").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("profile.toml"), body).unwrap();
        Profile {
            dir: dir.to_path_buf(),
            context: name.to_string(),
        }
    }

    #[test]
    fn endpoints_for_another_goatnet_come_from_its_own_context() {
        let tmp = tempfile::tempdir().unwrap();
        let zagros = ctx_with(
            tmp.path(),
            "zagros",
            "goatnet = \"zagros\"\nnetbird_mgmt_url = \"https://nb.zagros\"\n",
        );
        ctx_with(
            tmp.path(),
            "nova-op",
            "goatnet = \"nova\"\nnetbird_mgmt_url = \"https://nb.nova\"\n",
        );
        assert_eq!(
            zagros
                .goatnet_endpoints_for(Some("zagros"))
                .netbird_mgmt_url
                .as_deref(),
            Some("https://nb.zagros")
        );
        assert_eq!(
            zagros
                .goatnet_endpoints_for(Some("nova"))
                .netbird_mgmt_url
                .as_deref(),
            Some("https://nb.nova"),
            "--env nova from the zagros context must use nova's facts"
        );
        assert!(
            zagros
                .goatnet_endpoints_for(Some("vega"))
                .netbird_mgmt_url
                .is_none(),
            "a goatnet no context names gets no facts, never the active context's"
        );
        assert_eq!(
            zagros.goatnet_endpoints_for(None).goatnet.as_deref(),
            Some("zagros")
        );
    }

    #[test]
    fn a_profile_naming_no_goatnet_keeps_todays_overlay() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = ctx_with(tmp.path(), "default", "router_endpoint = \"tls/r:7447\"\n");
        assert_eq!(
            legacy
                .goatnet_endpoints_for(Some("zagros"))
                .router_endpoint
                .as_deref(),
            Some("tls/r:7447")
        );
    }

    #[test]
    fn extract_toml_field_does_not_confuse_a_prefix_key() {
        // The bug this regression-tests: the old `.trim_start_matches('=')`
        // implementation stripped zero-or-more `=` WITHOUT requiring one to
        // be present, so asking for "port" against a line that actually
        // reads "port_extra = 5" (port is a strict prefix of port_extra)
        // returned a garbage value instead of correctly reporting "not
        // found" and continuing to look for the real key.
        let toml = "port_extra = 5\nport = 7\n";
        assert_eq!(extract_toml_field(toml, "port"), Some("7".to_string()));
    }

    #[test]
    fn extract_toml_field_none_when_key_only_appears_as_a_prefix() {
        let toml = "port_extra = 5\n";
        assert_eq!(extract_toml_field(toml, "port"), None);
    }

    #[test]
    fn extract_toml_field_strips_quotes_and_whitespace() {
        let toml = "  portal_endpoint   =   \"https://example.test\"  \n";
        assert_eq!(
            extract_toml_field(toml, "portal_endpoint"),
            Some("https://example.test".to_string())
        );
    }

    #[test]
    fn require_mesh_ready_names_the_missing_cert_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "t".to_string(),
        };
        std::fs::create_dir_all(p.context_dir()).unwrap();
        std::fs::write(p.profile_toml_path(), "portal_endpoint = \"x\"\n").unwrap();
        // profile.toml exists (require_initialized would pass), but cert/
        // key/ca-roots do not — this is exactly the partially-broken-profile
        // shape that used to sail past every check and surface as a raw
        // Zenoh error three layers down instead of here.
        let err = p.require_mesh_ready().unwrap_err();
        assert_eq!(err.code(), "E_PROFILE_MISSING");
        match err {
            crate::errors::GoatError::ProfileMissing { hint, .. } => {
                assert!(hint.contains("mtls.cert.pem"), "hint: {hint}")
            }
            other => panic!("expected ProfileMissing, got {other:?}"),
        }
    }

    #[test]
    fn require_mesh_ready_ok_when_all_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "t".to_string(),
        };
        std::fs::create_dir_all(p.context_dir()).unwrap();
        std::fs::write(p.profile_toml_path(), "portal_endpoint = \"x\"\n").unwrap();
        std::fs::write(p.cert_path(), "cert").unwrap();
        std::fs::write(p.key_path(), "key").unwrap();
        std::fs::write(p.ca_roots_path(), "ca").unwrap();
        assert!(p.require_mesh_ready().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn require_mesh_ready_reports_an_unreadable_cert_as_denied_not_missing() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "t".to_string(),
        };
        std::fs::create_dir_all(p.context_dir()).unwrap();
        std::fs::write(p.profile_toml_path(), "portal_endpoint = \"x\"\n").unwrap();
        std::fs::write(p.cert_path(), "cert").unwrap();
        std::fs::write(p.key_path(), "key").unwrap();
        std::fs::write(p.ca_roots_path(), "ca").unwrap();
        let mode = |m: u32| {
            std::fs::set_permissions(p.cert_path(), std::fs::Permissions::from_mode(m)).unwrap()
        };
        mode(0o000);
        if std::fs::File::open(p.cert_path()).is_ok() {
            // Running as root: permissions do not bind, the denied state is
            // unobservable. Restore and skip.
            mode(0o600);
            return;
        }
        let err = p.require_mesh_ready().unwrap_err();
        mode(0o600);
        assert_eq!(err.code(), "E_PROFILE_DENIED");
        match err {
            crate::errors::GoatError::ProfileDenied { hint, .. } => {
                assert!(hint.contains("mtls.cert.pem"), "hint: {hint}")
            }
            other => panic!("expected ProfileDenied, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn init_state_three_way() {
        use std::os::unix::fs::PermissionsExt;
        let restore = |p: &std::path::Path, mode: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "t".to_string(),
        };

        // Absent: nothing on disk.
        assert!(matches!(p.init_state(), InitState::Absent));
        assert_eq!(
            p.require_initialized().unwrap_err().code(),
            "E_PROFILE_MISSING"
        );

        // Initialized: profile.toml present and readable.
        std::fs::create_dir_all(p.context_dir()).unwrap();
        std::fs::write(p.profile_toml_path(), "portal_endpoint = \"x\"\n").unwrap();
        assert!(matches!(p.init_state(), InitState::Initialized));
        assert!(p.require_initialized().is_ok());

        // Denied: file mode 000. `metadata()` still succeeds on this shape —
        // the open-for-read probe is what catches it.
        restore(&p.profile_toml_path(), 0o000);
        if std::fs::File::open(p.profile_toml_path()).is_ok() {
            // Running as root: permissions don't bind, the denied states are
            // unobservable. Restore and skip the denied assertions.
            restore(&p.profile_toml_path(), 0o600);
            return;
        }
        assert!(matches!(p.init_state(), InitState::Denied));
        assert_eq!(
            p.require_initialized().unwrap_err().code(),
            "E_PROFILE_DENIED"
        );
        restore(&p.profile_toml_path(), 0o600);

        // Denied: unsearchable context dir (the root-owned pod state shape).
        // Restored by the guard's Drop, so a failed assertion here surfaces as
        // the assertion rather than as a tempdir-cleanup IO error on top of it.
        let _guard = ModeGuard::set(p.context_dir(), 0o000, 0o700);
        assert!(matches!(p.init_state(), InitState::Denied));
    }

    /// Sets a path's mode for the rest of the scope and puts it back on drop,
    /// including while a panic unwinds — otherwise a failing assertion leaves
    /// an unsearchable dir behind and `TempDir`'s cleanup panics over the top
    /// of the real failure. POSIX mode bits, so unix-only like its caller.
    #[cfg(unix)]
    struct ModeGuard {
        path: std::path::PathBuf,
        restore_to: u32,
    }

    #[cfg(unix)]
    impl ModeGuard {
        fn set(path: PathBuf, mode: u32, restore_to: u32) -> Self {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            Self { path, restore_to }
        }
    }

    #[cfg(unix)]
    impl Drop for ModeGuard {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.restore_to),
            );
        }
    }

    #[test]
    fn with_extra_hint_appends_to_profile_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "t".to_string(),
        };
        let e = p
            .require_initialized()
            .unwrap_err()
            .with_extra_hint("Or pass --endpoint <url>.");
        match e {
            crate::errors::GoatError::ProfileMissing { hint, .. } => {
                assert!(hint.ends_with("Or pass --endpoint <url>."), "hint: {hint}")
            }
            other => panic!("expected ProfileMissing, got {other:?}"),
        }
    }

    #[test]
    fn read_current_context_parses_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "current_context = \"zagros\"\n");
        assert_eq!(
            read_current_context(&tmp.path().join("config.toml")),
            Some("zagros".to_string())
        );
    }

    #[test]
    fn read_current_context_none_when_file_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_current_context(&tmp.path().join("config.toml")), None);
    }

    #[test]
    fn read_current_context_none_when_key_missing_or_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "# no current_context here\nother = \"x\"\n");
        assert_eq!(read_current_context(&tmp.path().join("config.toml")), None);
        // An empty value must not be returned as the context name.
        write_config(tmp.path(), "current_context = \"\"\n");
        assert_eq!(read_current_context(&tmp.path().join("config.toml")), None);
    }

    #[test]
    fn read_current_context_tolerates_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "  current_context   =    \"demoorg\"  \n");
        assert_eq!(
            read_current_context(&tmp.path().join("config.toml")),
            Some("demoorg".to_string())
        );
    }

    #[test]
    fn resolve_honors_config_current_context_when_no_flag() {
        // The regression: a `goat context use <name>` write to config.toml
        // must actually take effect for commands that pass no --context.
        // GOAT_CONTEXT is a higher-precedence layer; skip rather than
        // mutate global env (racy under parallel tests, unsafe in 2024).
        if std::env::var_os("GOAT_CONTEXT").is_some() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "current_context = \"zagros\"\n");
        let p = Profile::resolve(Some(tmp.path().to_str().unwrap()), None).unwrap();
        assert_eq!(p.context, "zagros");
    }

    #[test]
    fn resolve_flag_beats_config_current_context() {
        // The flag is the top layer, so this holds regardless of env.
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "current_context = \"zagros\"\n");
        let p = Profile::resolve(Some(tmp.path().to_str().unwrap()), Some("default")).unwrap();
        assert_eq!(p.context, "default");
    }

    #[test]
    fn resolve_falls_back_to_default_with_no_config() {
        if std::env::var_os("GOAT_CONTEXT").is_some() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile::resolve(Some(tmp.path().to_str().unwrap()), None).unwrap();
        assert_eq!(p.context, DEFAULT_CONTEXT);
    }
}

/// `$GOAT_PORTAL_TOKEN`, when set to something non-empty.
///
/// goat-cli.md has documented this as the highest-precedence token source
/// since the CLI shipped ("the CI escape hatch … operators in headless
/// containers don't need a `~/.goat/`"), and it was never implemented: both
/// `load()` and `portal_token()` read the context file and nothing else.
/// The drift is not academic — a person hitting a 403 because their profile
/// holds the participant `vp_` key rather than their operator `op_` key
/// follows the documented escape hatch, sets the variable, sees no change,
/// and has no way to tell that the doc is describing something that does not
/// exist.
///
/// Empty or whitespace-only is treated as UNSET rather than as an empty
/// token: `GOAT_PORTAL_TOKEN=` in a shell profile should not silently
/// override a perfectly good token file with nothing.
pub(crate) fn portal_token_from_env() -> Option<String> {
    std::env::var("GOAT_PORTAL_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod portal_token_env_tests {
    use super::portal_token_from_env;

    // The doc promised this override since the CLI shipped and the code never
    // had it, so someone following the documented escape hatch out of a 403
    // saw no change and no explanation.
    #[test]
    fn env_token_is_honoured_and_trimmed() {
        let prev = std::env::var("GOAT_PORTAL_TOKEN").ok();
        unsafe { std::env::set_var("GOAT_PORTAL_TOKEN", "  op_from_env  ") };
        assert_eq!(portal_token_from_env().as_deref(), Some("op_from_env"));

        // Empty means UNSET, not "an empty token" — a stray `GOAT_PORTAL_TOKEN=`
        // in a shell profile must not blank out a working token file.
        unsafe { std::env::set_var("GOAT_PORTAL_TOKEN", "   ") };
        assert_eq!(portal_token_from_env(), None);

        unsafe { std::env::remove_var("GOAT_PORTAL_TOKEN") };
        assert_eq!(portal_token_from_env(), None);

        if let Some(v) = prev {
            unsafe { std::env::set_var("GOAT_PORTAL_TOKEN", v) };
        }
    }
}

// SPDX-License-Identifier: Apache-2.0
//
// goat-cli — unified operator command-line for the goat data fabric.
// See docs/design/goat-cli.md and ADR 1019.
//
// Phase A (this crate): scaffold + clap subcommand stubs + context-
// shaped on-disk layout. No working behavior beyond `goat version`,
// `goat profile path`, and `goat completion <shell>`. Phases B–E fill
// in the surfaces per the design doc's phasing table.

use clap::{Parser, Subcommand};

mod alias;
mod bundle;
mod classify;
mod cmd;
mod errors;
mod exit_codes;
mod fun;
mod health_wire;
mod i18n;
mod mutator;
mod names;
mod narrator;
mod output;
mod probe;
mod profile;
mod substrate;
mod tui;
mod version_check;

use output::ColorChoice;

/// Default log filter when neither `--verbose` nor `RUST_LOG` says otherwise.
///
/// `zenoh_link_tls::utils=error` is the one deliberate mute. That module logs
/// exactly one WARN in zenoh 1.9.0 — `Skipping name verification of TLS
/// server` (`zenoh-link-tls-1.9.0/src/utils.rs:339`) — and it is EXPECTED on
/// every goat session: the fabric authenticates on the certificate's URI-SAN,
/// not on a hostname, so `verify_name_on_connect` is off by design.
/// It was also the first thing printed by `goat sub`, above an otherwise
/// silent tail, and operators read the pair as "it failed" and stopped.
/// Muting it costs no diagnosis: real TLS trouble logs from
/// `zenoh_link_tls::unicast` (`tls handshake eof`, F-156), which stays at
/// WARN, and `-v` / `RUST_LOG` bring the muted line straight back.
const DEFAULT_LOG_FILTER: &str = "goat=info,warn,zenoh_link_tls::utils=error";

// NOTE: the `about`/`long_about` here and the `///` doc-comments on each
// subcommand are the clap-derive defaults AND the English source of truth,
// but they are NOT what the operator sees at runtime: `i18n::localize_command`
// swaps every one for the active-locale catalog string (incl. English) before
// parse. Editing prose here without also updating `i18n/en/goat_cli.ftl` will
// change rustdoc but NOT `goat --help`. See docs/design/goat-cli-i18n.md.
#[derive(Parser, Debug)]
#[command(
    name = "goat",
    version,
    about = "Unified operator command-line for the goat data fabric.",
    long_about = "goat — operator-facing CLI composing panoscope-core into one tool: \
                  profile init/reuse, ZTOL-aware reachability doctor, org portal browse, \
                  topic CRUD, pub/sub, stethoscope delegation, an interactive command \
                  composer, and substrate-cascade narration."
)]
pub struct Cli {
    /// Override the profile directory (default: $GOAT_PROFILE_DIR or ~/.goat).
    #[arg(long, global = true, env = "GOAT_PROFILE_DIR")]
    pub profile_dir: Option<String>,

    /// Override the active context (default: $GOAT_CONTEXT env, then
    /// current_context in config.toml, then "default").
    #[arg(long, global = true, env = "GOAT_CONTEXT")]
    pub context: Option<String>,

    /// Connect to this router endpoint (`tls/<ip>:<port>`) instead of the
    /// context's `router_endpoint`, for this command only. The context's
    /// certificate, key and CA roots are still used. For a local port
    /// forward: `env-door.sh <goatnet> --forward <router-ip>:7447`.
    #[arg(
        long,
        global = true,
        env = "GOAT_ROUTER_ENDPOINT",
        value_name = "ENDPOINT"
    )]
    pub router: Option<String>,

    /// Show the substrate cascade a mutating command sets off.
    #[arg(long, global = true)]
    pub narrate: bool,

    /// Emit the substrate cascade as NDJSON (one object per consequence),
    /// for incident logs + scripting. Implies --narrate.
    #[arg(long, global = true)]
    pub narrate_json: bool,

    /// Show raw UUID slot anchors instead of human aliases (ADR 1039).
    /// Aliases are the default display; this forces the canonical form.
    /// `--json` always carries the UUID regardless of this flag.
    #[arg(long, global = true)]
    pub uuid: bool,

    /// Enable verbose debug output (or set RUST_LOG=goat=debug).
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Color mode for terminal output (auto|always|never). NO_COLOR
    /// and CLICOLOR_FORCE env vars also respected per no-color.org.
    #[arg(long, global = true, default_value = "auto")]
    pub color: ColorChoice,

    /// Language for human-facing prose (e.g. en, uk). Falls back to
    /// GOAT_LANG, then the OS locale, then English. The machine surface
    /// (--json, error codes, command/flag names) is unaffected.
    #[arg(long, global = true, env = "GOAT_LANG")]
    pub lang: Option<String>,

    /// Max seconds to wait establishing a fabric session before failing fast
    /// (default 5). Bounds the connect/handshake phase for network commands
    /// (health/sub/pub/presence) so an unreachable router errors cleanly
    /// instead of blocking. Does NOT cap steady-state streaming — `sub
    /// --watch` runs as long as you want. Also reads GOAT_TIMEOUT.
    #[arg(long, global = true, env = "GOAT_TIMEOUT")]
    pub timeout: Option<u64>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Manage the local goat profile (mTLS cert + portal token + endpoint set).
    Profile {
        #[command(subcommand)]
        op: cmd::profile::ProfileCmd,
    },
    /// The CA roots this machine trusts (`ls` / `add` / `rm`).
    ///
    /// goat ships knowing about no goatnet — every root is one you add. That
    /// is what makes one binary usable against any environment: adding a
    /// goatnet, or rotating a CA, is not a new release.
    Ca {
        #[command(subcommand)]
        op: cmd::ca::CaCmd,
    },
    /// Manage contexts (`ls` / `use` / `show` / `rm`).
    Context {
        #[command(subcommand)]
        op: cmd::context::ContextCmd,
    },
    /// First-run wizard: pick sandbox or real, deliver an instant win,
    /// and point at the tutor.
    Init(cmd::init::InitArgs),
    /// A self-paced, hands-on tour of the fabric (vimtutor-style).
    Tutor(cmd::tutor::TutorArgs),
    /// Round-trip a hello through the fabric end-to-end (the hello-world
    /// smoke). Runs against the in-process sandbox or a real env.
    Hello(cmd::hello::HelloArgs),
    /// Diagnose local setup, mesh client, router path, session, and portal.
    ///
    /// Steps: local readiness → router path → TLS material → mTLS session → portal.
    /// Each failure reports the observation and a next action.
    Doctor(cmd::doctor::DoctorArgs),
    /// Federate with a partner: our half, their half, the agreement, and the
    /// configuration it implies (docs/design/federation-package-spec.md).
    ///
    /// Two parties each publish one signed package, both compose the pair
    /// independently, and both compare the resulting digest. Equal digests
    /// prove both sides hold the identical agreement — and that comparison may
    /// travel by any channel at all, because altering a digest in transit
    /// causes a visible mismatch and causing a false match would mean forging
    /// a signature.
    ///
    /// `lint`, `compose` and `diff` are pure: they read the files you name and
    /// nothing else, hold no credential and open no socket, so a partner can
    /// trust that handing you their package cannot touch anything. `apply` is
    /// the only verb that writes.
    ///
    /// When two digests disagree, the verb you want is `diff`.
    Federate {
        #[command(subcommand)]
        op: cmd::federate::FederateCmd,
    },
    /// Update the running `goat` binary from this goatnet's download
    /// surface (get.<site>). Verifies the published checksum, then atomically
    /// replaces the installed binary. Per-goatnet: uses the active context's
    /// surface. See `--dry-run` / `--force`.
    Update(cmd::update::UpdateArgs),
    /// Browse orgs registered in the portal.
    ///
    /// Named `org` since the ADR 1052 vendor→org rename; `vendor` stays a
    /// hidden alias so existing scripts and muscle memory keep working.
    #[command(name = "org", alias = "vendor")]
    Vendor {
        #[command(subcommand)]
        op: cmd::vendor::VendorCmd,
    },
    /// List, show, create, or delete topics.
    Topic {
        #[command(subcommand)]
        op: cmd::topic::TopicCmd,
    },
    /// Manage a topic's schema body: upload, ls, show, diff.
    Schema {
        #[command(subcommand)]
        op: cmd::schema::SchemaCmd,
    },
    /// Mesh ACL assertions — check what the declared ACL must and must not
    /// permit, WITHOUT applying anything.
    ///
    /// `goat acl test [scope]` reads the ACL declaration compiled into this
    /// binary and runs its `netbird_acl_tests` block through the same
    /// evaluator the PR gate and netvis `/assertions` use. No network, no
    /// context, no credentials — a check that needs a working profile is a
    /// check people skip. A scope, or a whole declaration, with no
    /// assertions fails: nothing was checked. `--none-expected` accepts that
    /// outcome when it is intended.
    Acl {
        #[command(subcommand)]
        op: cmd::acl::AclCmd,
    },
    /// Least-privilege network authorizations (ADR 1129).
    ///
    /// `goat warrant why-blocked` answers "why can't A reach B on port P"
    /// from the declaration in git — no network, no credentials. Most such
    /// reports are DNS, membership or a dead service rather than an ACL gap;
    /// when it IS a gap it prints the minimal port-set that closes it and the
    /// diff to put in a PR. `goat warrant apply --membership` converges a
    /// governed scope's peer membership to `ops/warrant/membership/<scope>.yaml`.
    Warrant {
        #[command(subcommand)]
        op: cmd::warrant::WarrantCmd,
    },
    /// Data-Access COP: who can access which data, and why.
    Access {
        #[command(subcommand)]
        op: cmd::access::AccessCmd,
    },
    /// Enroll and manage machines (system principals) against a custodian's
    /// enrollment grant (ADR 1096). `machine enroll <handle> --node-class …`.
    Machine {
        #[command(subcommand)]
        op: cmd::machine::MachineCmd,
    },
    /// Read the operator-action audit trail (`ops/audit/operator/**`) — the
    /// who-did-what record of backend admin actions, decrypted with your
    /// operator age identity.
    Audit {
        #[command(subcommand)]
        op: cmd::audit::AuditCmd,
    },
    /// Run a fabric demo end-to-end: open the browser-side traffic generator
    /// and tail the resulting Zenoh traffic in this terminal. With no name it runs
    /// the demo service the goatnet publishes (`demo_service`).
    Demo(cmd::demo::DemoArgs),
    /// Publish a payload to a topic key.
    Pub(cmd::pubsub::PubArgs),
    /// Subscribe to a topic key expression.
    Sub(cmd::pubsub::SubArgs),
    /// Send or receive a large file over the non-persisted `volatile/blob`
    /// class: chunk, BLOCK-publish, manifest, sha256-verify.
    Blob {
        #[command(subcommand)]
        op: cmd::blob::BlobCmd,
    },
    /// Replay past events from the audit-tail (read, not live subscribe).
    ///
    /// Reads the Object-Lock audit-tail via portal-minted presigned URLs —
    /// no MinIO credentials on the laptop.
    Replay(cmd::replay::ReplayArgs),
    /// Show fabric health (per-class load vs capacity + green/amber/red)
    /// and, with --me/--peer, a producer's own footprint.
    Health(cmd::health::HealthArgs),
    /// Is data flowing, and when did it last? Per-topic freshness across
    /// whatever observers are reachable — panoscope, the audit-tail, and a
    /// bounded live listen — with every row labelled by the observer that
    /// answered it. Distinguishes "no observer could answer" from "an
    /// observer looked back over a stated horizon and found nothing", and
    /// never claims a topic has never published. A bare invocation is the
    /// one-frame scriptable snapshot; --json is the stable machine contract.
    Flow(cmd::flow::FlowArgs),
    /// Name-fallback self-claims: publish this device's signed claim, or
    /// collect published claims for the get tier (ADR 1082 Amendment 2).
    Names {
        #[command(subcommand)]
        cmd: cmd::names::NamesCmd,
    },
    /// Live single-pane fabric dashboard (the top(1) of the fabric):
    /// every environment-health domain at once + a sortable producer
    /// ranking, auto-refreshing. Interactive screen is TTY-only; --once
    /// is the scriptable plain-text snapshot (ADR 1044).
    Top(cmd::top::TopArgs),
    /// Who is alive on the fabric right now — the native liveliness
    /// presence view (one-shot set or --watch join/leave edges).
    Presence(cmd::presence::PresenceArgs),
    /// Per-entity behavior lens (ADR 1090): spine + capability + merged
    /// activity timeline + per-source honesty, rendered from the
    /// mesh-side microscope service. Plain one-frame output; --json is
    /// the stable machine contract.
    Microscope(cmd::microscope::MicroscopeArgs),
    /// Identity common operating picture (the top(1) of identities): a
    /// searchable roster browser — who is in the goatnet, what each may
    /// do, and what they're doing. Interactive screen is TTY-only; --once
    /// is the scriptable plain-text snapshot (ADR 1047).
    Who(cmd::who::WhoArgs),
    /// Delegate a diagnostic verdict request to stethoscope.
    Inspect(cmd::inspect::InspectArgs),
    /// Interactive merged picker: browse the declared ⊕ live roster and
    /// hop to the Diagnose / Tail lens on the selected target.
    Ui(cmd::ui::UiArgs),
    /// Interactive command builder TUI.
    Compose(cmd::compose::ComposeArgs),
    /// Substrate-cascade reflection over recent activity. Not yet
    /// available (the everyday path is --narrate on a mutating command).
    Narrate(cmd::narrate::NarrateArgs),
    /// Print shell completion script to stdout.
    Completion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Browse the goat docs, or emit the full command reference
    /// (`--markdown`) for the hosted docs site.
    Docs(cmd::docs::DocsArgs),
    /// Hidden: a charming ASCII goat. There is, of course, a hidden goat.
    #[command(name = "goat", hide = true)]
    Goat,
    /// Hidden: RFC 1925, "The Twelve Networking Truths."
    #[command(name = "truths", hide = true)]
    Truths,
    /// Hidden: the Colossal Cave Adventure magic word.
    #[command(name = "xyzzy", hide = true)]
    Xyzzy,
    /// Hidden: HTTP 418, "I'm a teapot" (HTCPCP, RFC 2324).
    #[command(name = "brew", hide = true)]
    Brew,
    /// Hidden: a goat endless runner (crates/goat-fun).
    #[command(name = "bound", hide = true)]
    Bound,
    /// Hidden: emit completion candidates for shell hooks (one per line
    /// on stdout, errors silently swallowed). Wired into the operator's
    /// shell rc.
    #[command(name = "_complete", hide = true)]
    Complete {
        #[command(subcommand)]
        op: cmd::complete::CompleteCmd,
    },
    /// Interpret an error: a code (`E_OFF_MESH`, `GB-007`, `X-SSH-001`) or
    /// any error text, goat's own or a tool's words it carried — what
    /// happened, why, what to do, where to read (ADR 1199).
    Explain(cmd::explain::ExplainArgs),
    /// Publish synthetic samples to a topic. Default: trivial hello-
    /// world JSON at 1Hz. Pass `--format <.fds>` + `--message <Name>`
    /// to encode operator-supplied protobuf instead.
    Mock(cmd::mock::MockArgs),
    /// Operator-only administration: org lifecycle, onboard/revoke/
    /// rotate, mesh-peer cut, device + operator-tier provisioning, LAN triage.
    /// Authorized by your operator credentials (server-side via portal/NetBird
    /// token scope); inert for non-admins — an unauthorized call changes
    /// nothing and fails with `E_NOT_ADMIN`. (Mirrors the `help-cmd-admin`
    /// catalog string; the runtime help comes from there, see the NOTE above.)
    Admin {
        #[command(subcommand)]
        op: cmd::admin::AdminCmd,
    },
    /// Health synthesis: router port reachability + a real mTLS Zenoh
    /// session open + portal /api/health + local cert expiry + name, PQC
    /// and device state, one glyph line per check or one --json object.
    Status(cmd::status::StatusArgs),
    /// One-action mesh recovery: run the status probes, and when the
    /// mesh is down bounce the mesh client (netbird down → up, or
    /// goat-clientd disconnect/connect), wait for the router to come
    /// back, and re-probe — reports what it did and the verified end
    /// state, or explains why it couldn't heal.
    Heal(cmd::heal::HealArgs),
    /// Probe each hosted service in `ops/serve/services.yaml` and
    /// render one glyph line per service (or one --json object).
    Services(cmd::services::ServicesArgs),
    /// Wrap the local goat-clientd daemon via its JSON-RPC IPC socket
    /// (`~/.goat-client/goat-clientd.sock`): status / up / down /
    /// bundle import.
    Client {
        #[command(subcommand)]
        op: cmd::client::ClientCmd,
    },
    /// PATH-based extension discovery (kubectl-style). Any
    /// `goat-<name>` executable on PATH is invokable as `goat <name>`;
    /// `goat plugin list` enumerates installed plugins, `goat plugin
    /// which <name>` resolves the path.
    Plugin {
        #[command(subcommand)]
        op: cmd::plugin::PluginCmd,
    },
}

#[tokio::main]
async fn main() {
    // Pre-clap PATH-extension dispatch (kubectl-style). If the first
    // non-flag argument is NOT a built-in subcommand AND a matching
    // `goat-<name>` executable is on PATH, exec it with the remaining
    // args. clap never sees it. This runs before tracing init so
    // plugins inherit a clean stderr.
    if let Some(plugin_exit) = maybe_exec_plugin() {
        std::process::exit(plugin_exit);
    }

    // Resolve the active language BEFORE clap builds the command, so that
    // `--help`/`--version`/parse-error output (which clap renders and
    // exits inside `get_matches`, before any of our code runs again) is
    // localized. `preparse_lang` peeks `--lang`/`GOAT_LANG` out of argv;
    // `select_language` then negotiates OS-locale → English as usual
    // (ADR 1024 / docs/design/goat-cli-i18n.md). Ambient like the tracing
    // subscriber; every `fl!` — including the help swaps below — reads it.
    i18n::select_language(i18n::preparse_lang().as_deref());
    // The shared error catalog renders its prose in the same language,
    // bilingual with the binding English (ADR 1199 + ADR 1024).
    goat_errors::set_language(&i18n::active_language_tag());

    // Swap clap's compile-time `about`/`long_about`/flag-help for the
    // active-locale catalog strings on the command tree, then parse from
    // the mutated command (clap renders help/version/errors from it).
    use clap::{CommandFactory, FromArgMatches};
    let matches = i18n::localize_command(Cli::command()).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        // Unreachable in practice: `get_matches` already validated argv
        // and exited on error. Render via clap for parity if it ever is.
        Err(e) => e.exit(),
    };
    if let Some(router) = cli.router.as_deref() {
        profile::set_router_override(router);
        if profile::router_override().is_some() {
            eprintln!(
                "router: {router} (--router; replaces the context's router_endpoint for this command)"
            );
        }
    }

    let filter = if cli.verbose {
        // The trailing bare `warn` is what makes `-v` a SUPERSET of the
        // default filter. Without it this directive list has no global
        // fallback, so every target that is not `goat` or `panoscope_core`
        // — including the `zenoh_link_tls::utils` line muted below, and
        // every other dependency's WARN — is filtered out entirely, and
        // `-v` prints strictly LESS about the fabric than no flag at all.
        tracing_subscriber::EnvFilter::new("goat=debug,panoscope_core=info,warn")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER))
    };
    // Route diagnostic logs to STDERR so `--json` modes keep stdout
    // a clean NDJSON stream. tracing-subscriber's default writer is
    // stdout — this contaminated `goat doctor --json` step-3 output
    // (the zenoh router "Skipping name verification of TLS server"
    // WARN line landed between two JSON lines).
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    // Language is already resolved (pre-parse, above) so `--help` could be
    // localized; `cli.lang` here is the same value preparse peeked.

    // Tell the operator when they're reading machine-translated, not-yet-
    // native-reviewed output, and where to report errors (ADR 1024
    // decision 6). Skipped for the shell-completion paths (machine-
    // consumed stdout); the notice itself is stderr-only.
    if !matches!(
        cli.cmd,
        Cmd::Completion { .. } | Cmd::Complete { .. } | Cmd::Docs { .. }
    ) {
        // The feedback address is the active context's goatnet fact; a context
        // that cannot be resolved yet simply has none.
        let goatnet_email = profile::Profile::resolve(cli.profile_dir.as_deref(), cli.context.as_deref())
            .ok()
            .and_then(|p| p.goatnet_endpoints().i18n_feedback_email);
        i18n::warn_if_machine_translated(goatnet_email.as_deref());
    }

    let exit = run(cli).await;
    std::process::exit(exit);
}

async fn run(cli: Cli) -> i32 {
    let ctx = match cmd::Ctx::from_cli(&cli) {
        Ok(c) => c,
        Err(e) => return report_anyhow(e),
    };

    let result = match cli.cmd {
        Cmd::Ca { op } => cmd::ca::run(&ctx, op).await,
        Cmd::Profile { op } => cmd::profile::run(&ctx, op).await,
        Cmd::Context { op } => cmd::context::run(&ctx, op).await,
        Cmd::Init(args) => cmd::init::run(&ctx, args).await,
        Cmd::Tutor(args) => cmd::tutor::run(&ctx, args).await,
        Cmd::Hello(args) => cmd::hello::run(&ctx, args).await,
        Cmd::Federate { op } => cmd::federate::run(&ctx, op).await,
        Cmd::Doctor(args) => cmd::doctor::run(&ctx, args).await,
        Cmd::Update(args) => cmd::update::run(&ctx, args).await,
        Cmd::Vendor { op } => cmd::vendor::run(&ctx, op).await,
        Cmd::Topic { op } => cmd::topic::run(&ctx, op).await,
        Cmd::Schema { op } => cmd::schema::run(&ctx, op).await,
        Cmd::Acl { op } => cmd::acl::run(&ctx, op).await,
        Cmd::Warrant { op } => cmd::warrant::run(&ctx, op).await,
        Cmd::Access { op } => cmd::access::run(&ctx, op).await,
        Cmd::Machine { op } => cmd::machine::run(&ctx, op).await,
        Cmd::Audit { op } => cmd::audit::run(&ctx, op).await,
        Cmd::Demo(args) => cmd::demo::run(&ctx, args).await,
        Cmd::Pub(args) => cmd::pubsub::run_pub(&ctx, args).await,
        Cmd::Sub(args) => cmd::pubsub::run_sub(&ctx, args).await,
        Cmd::Blob { op } => cmd::blob::run(&ctx, op).await,
        Cmd::Replay(args) => cmd::replay::run(&ctx, args).await,
        Cmd::Health(args) => cmd::health::run(&ctx, args).await,
        Cmd::Flow(args) => cmd::flow::run(&ctx, args).await,
        Cmd::Names { cmd: sub } => cmd::names::run(&ctx, sub).await,
        Cmd::Top(args) => cmd::top::run(&ctx, args).await,
        Cmd::Presence(args) => cmd::presence::run(&ctx, args).await,
        Cmd::Microscope(args) => cmd::microscope::run(&ctx, args).await,
        Cmd::Who(args) => cmd::who::run(&ctx, args).await,
        Cmd::Inspect(args) => cmd::inspect::run(&ctx, args).await,
        Cmd::Ui(args) => cmd::ui::run(&ctx, args).await,
        Cmd::Compose(args) => cmd::compose::run(&ctx, args).await,
        Cmd::Narrate(args) => cmd::narrate::run(&ctx, args).await,
        Cmd::Complete { op } => cmd::complete::run(&ctx, op).await,
        Cmd::Explain(args) => cmd::explain::run(&ctx, args).await,
        Cmd::Mock(args) => cmd::mock::run(&ctx, args).await,
        Cmd::Admin { op } => {
            cmd::context::ensure_goatnet_facts(&ctx.profile).await;
            cmd::admin::run(&ctx, op).await
        }
        Cmd::Status(args) => cmd::status::run(&ctx, args).await,
        Cmd::Heal(args) => cmd::heal::run(&ctx, args).await,
        Cmd::Services(args) => cmd::services::run(&ctx, args).await,
        Cmd::Client { op } => cmd::client::run(&ctx, op).await,
        Cmd::Plugin { op } => cmd::plugin::run(&ctx, op).await,
        Cmd::Completion { shell } => {
            use clap::CommandFactory;
            let mut c = Cli::command();
            let name = c.get_name().to_string();
            clap_complete::generate(shell, &mut c, name, &mut std::io::stdout());
            Ok(())
        }
        Cmd::Docs(args) => cmd::docs::run(args),
        Cmd::Goat => cmd::easter::run(),
        Cmd::Truths => cmd::easter::truths(),
        Cmd::Xyzzy => cmd::easter::xyzzy(),
        Cmd::Brew => cmd::easter::brew(),
        Cmd::Bound => std::process::exit(goat_fun::bound("goat")),
    };

    match result {
        Ok(()) => exit_codes::EXIT_SUCCESS,
        Err(e) => report_anyhow(e),
    }
}

/// Render an `anyhow::Error` on stderr and return the appropriate exit
/// code. `GoatError`s (downcast) get the cargo-style two-line render
/// and their stable exit code; everything else falls through to the
/// generic exit code with `anyhow`'s default Display.
fn report_anyhow(err: anyhow::Error) -> i32 {
    // `goat federate` has already printed its structured report by the time it
    // bails, and several of its non-zero codes are OUTCOMES rather than faults
    // — a composition refusal is the mechanism working. Adding an `error:` line
    // on top of that report would misdescribe it.
    if let Some(code) = cmd::federate::exit_code_of(&err) {
        return code;
    }
    // `goat warrant apply` refusals carry their own code (2 = mode ceiling,
    // 3 = over --max-changes) so a timer wrapper can tell a designed refusal
    // from a fault.
    if let Some(w) = err.downcast_ref::<cmd::warrant::WarrantExit>() {
        eprintln!("error: {w}");
        return w.code;
    }
    if let Some(g) = err.downcast_ref::<errors::GoatError>() {
        let rendered = errors::DisplayHuman(g).to_string();
        eprintln!("{rendered}");
        // A tool's words carried in the hint are interpreted too (ADR 1199),
        // at a terminal or with GOAT_EXPLAIN=1; a captured run keeps the
        // three-line render scripts and golden tests know.
        if goat_errors::explain_on_stderr() {
            eprint!("{}", cmd::explain::passthrough_hits(&rendered, Some(g.code())));
        }
        g.exit_code()
    } else {
        // Not one of goat's own coded errors: say what it is, then what the
        // catalog knows about it — its own words, or a tool's it carried.
        let text = format!("{err:#}");
        eprintln!("error: {text}");
        if goat_errors::explain_on_stderr() {
            eprint!("{}", cmd::explain::passthrough_hits(&text, None));
        }
        exit_codes::EXIT_GENERIC
    }
}

/// kubectl-style PATH-extension dispatch. Walks args looking for the
/// first non-flag token; if it's NOT a built-in subcommand AND a
/// matching `goat-<name>` executable is on PATH, exec the plugin
/// with the remaining args and return its exit code.
///
/// We skip global flags that take values (`--profile-dir <p>`,
/// `--context <c>`, `--color <m>`) so that
/// `goat --color always acme foo` still resolves `acme` as the
/// subcommand. Boolean global flags (`--narrate`, `--verbose`/`-v`)
/// don't take values.
///
/// Returns `Some(exit_code)` if a plugin was exec'd (caller exits),
/// or `None` if normal clap parsing should proceed.
fn maybe_exec_plugin() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    let mut remaining: Vec<String> = Vec::new();

    // Walk global flags. Value-taking ones consume the next token.
    while let Some(a) = args.next() {
        match a.as_str() {
            // value-taking global flags
            "--profile-dir" | "--context" | "--color" | "--lang" => {
                let _ = args.next();
            }
            // value-taking forms with `=`
            x if x.starts_with("--profile-dir=")
                || x.starts_with("--context=")
                || x.starts_with("--color=")
                || x.starts_with("--lang=") => {}
            // bool global flags + their short forms
            "--narrate" | "--verbose" | "-v" => {}
            // non-flag → subcommand candidate
            x if !x.starts_with('-') => {
                if cmd::plugin::is_builtin_or_flag(x) {
                    return None;
                }
                if let Some(path) = cmd::plugin::find_plugin(x) {
                    remaining.extend(args);
                    return Some(run_plugin(&path, &remaining));
                }
                // No plugin found — let clap parse + emit its own
                // "unknown subcommand" error.
                return None;
            }
            // unknown flag — let clap handle it.
            _ => return None,
        }
    }
    // No subcommand → clap will show help.
    None
}

/// Hand control to a `goat-<name>` plugin executable.
///
/// On Unix we `exec()` so the plugin replaces this process — signals,
/// stdio, and the exit code pass through transparently and there's no
/// lingering parent. Windows has no `exec`, so we spawn-and-wait and
/// propagate the child's exit code instead (the brief parent process
/// is invisible to the operator).
#[cfg(unix)]
fn run_plugin(path: &std::path::Path, args: &[String]) -> i32 {
    use std::os::unix::process::CommandExt;
    // exec() only returns on failure.
    let err = std::process::Command::new(path).args(args).exec();
    eprintln!("error: exec {} failed: {err}", path.display());
    exit_codes::EXIT_GENERIC
}

#[cfg(not(unix))]
fn run_plugin(path: &std::path::Path, args: &[String]) -> i32 {
    match std::process::Command::new(path).args(args).status() {
        Ok(status) => status.code().unwrap_or(exit_codes::EXIT_GENERIC),
        Err(err) => {
            eprintln!("error: spawn {} failed: {err}", path.display());
            exit_codes::EXIT_GENERIC
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// `clap derive` builds a valid command — assert it before mutating.
    #[test]
    fn cli_command_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// Every `fl!` key `localize_command` references resolves (the macro
    /// compile-time-checks them against `en`), and the swapped prose
    /// actually lands on the rendered command tree. The process-global
    /// loader defaults to English with no `select_language` call, so this
    /// asserts the English catalog strings without racing the global on a
    /// `select`. Covers the root description, a global flag, and a
    /// subcommand one-liner — the three node kinds `localize_command`
    /// rewrites.
    #[test]
    fn localized_help_renders_catalog_strings() {
        let help = i18n::localize_command(Cli::command())
            .render_long_help()
            .to_string();
        // Root long_about.
        assert!(
            help.contains("composing panoscope-core into one tool"),
            "root long_about not localized:\n{help}"
        );
        // A subcommand one-liner from the Commands: list.
        assert!(
            help.contains("Diagnose local setup, mesh client, router path, session, and portal"),
            "doctor about not localized:\n{help}"
        );
        // A global flag's help from the Options: block.
        assert!(
            help.contains("Language for human-facing prose"),
            "--lang help not localized:\n{help}"
        );
    }
}

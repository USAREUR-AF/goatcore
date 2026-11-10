// Subcommand modules. Each module owns one top-level subcommand and
// stays self-contained — adding a new subcommand = one new file + one
// line in main.rs's Cmd enum.

use crate::output::OutputContext;
use crate::profile::Profile;

pub mod access;
pub mod acl;
pub mod admin;
pub mod audit;
pub mod blob;
pub mod ca;
pub mod client;
pub mod complete;
pub mod compose;
pub mod compose_context; // CONTEXT-PANE: delete this line to strip the compose v1 context pane
pub mod context;
pub mod decode;
pub mod demo;
pub mod docs;
pub mod doctor;
mod doctor_preflight;
pub mod easter;
pub mod explain;
pub mod federate;
pub mod flow;
pub mod heal;
pub mod health;
pub mod hello;
pub mod init;
pub mod inspect;
pub mod lan;
pub mod machine;
mod machine_affiliation;
mod machine_ssh;
pub mod members;
pub mod microscope;
pub mod mock;
pub mod names;
pub mod narrate;
pub mod platform_admin;
pub mod plugin;
pub mod presence;
pub mod profile;
pub mod pubsub;
pub mod replay;
pub mod schema;
pub mod sealed;
pub mod services;
pub mod status;
pub mod storm;
pub mod top;
pub mod topic;
pub mod tutor;
pub mod ui;
pub mod update;
pub mod vendor;
pub mod warrant;
pub mod who;

/// Shared context passed to every subcommand. Phase B adds the shared
/// `OutputContext` (NO_COLOR/TTY/CI) read once at top-level; Phase C+
/// will add HTTP clients, Zenoh session builder, embedded CA root.
pub struct Ctx {
    pub profile: Profile,
    #[allow(dead_code)] // Phase B step 3 (`doctor`) consumes via verdict glyph render.
    pub output: OutputContext,
    pub narrate: bool,
    /// `--narrate-json`: emit the cascade as NDJSON. Implies `narrate`.
    pub narrate_json: bool,
    /// `--uuid`: show raw slot anchors instead of human aliases (ADR 1039).
    pub uuid: bool,
}

impl Ctx {
    pub fn from_cli(cli: &crate::Cli) -> anyhow::Result<Self> {
        // Record the operator's --timeout/GOAT_TIMEOUT once at startup; the
        // session-open path (cmd::pubsub) reads it as the connect deadline.
        crate::cmd::pubsub::set_network_timeout_secs(cli.timeout);
        let profile = Profile::resolve(cli.profile_dir.as_deref(), cli.context.as_deref())?;
        // The trust store lives beside the contexts, so `--profile-dir` has to
        // move it too. Verification is a free function with no handle on the
        // CLI, so the resolved base is recorded here, once, for every command —
        // not just for `goat ca`.
        crate::bundle::set_profile_base(&profile.dir);
        Ok(Self {
            profile,
            output: OutputContext::from_env(cli.color),
            // --narrate-json implies narration; either flag turns the cascade on.
            narrate: cli.narrate || cli.narrate_json,
            narrate_json: cli.narrate_json,
            uuid: cli.uuid,
        })
    }
}

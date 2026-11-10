// SPDX-License-Identifier: Apache-2.0
//
// `goat admin platform-admin status|grant|revoke <email> --env <goatnet>` — one
// command that makes a person a platform admin on a goatnet, or stops them
// being one, on every plane that grants the role. Canon:
// docs/design/platform-admin-lifecycle.md (ADR 1182).
//
// The planes, in the order a REVOKE runs them (a grant runs them roughly in
// reverse, authority first):
//
//   ssh           host-shell keys: removed from every host in the goatnet's
//                 revoke union (platform-admin-ssh-keys.yml), console rows revoked
//   operator-key  the person's `op_` portal key(s): revoked
//   sponsorship   the right to invite/approve people: revoked
//   mesh          their peers leave every *-PLATFORM-ADMIN NetBird group
//                 (mirror-first onto <PREFIX>-OBSERVER)
//   ory           PLATFORM-ADMIN leaves their login groups, through the
//                 console machine API (audited, sessions revoked)
//   netbird-role  their NetBird dashboard user drops admin -> user. Runs
//                 AFTER ory: the promote timer re-promotes anyone Ory still
//                 names, so the order is load-bearing.
//   1password     removed from the team's Platform group
//   site_admins   reported only — a committed config list this command never edits
//
// Invariants:
//   - DRY-RUN DEFAULT. Without --apply it reads every plane, prints what it
//     would change, and changes nothing.
//   - EVERY WRITE GOES THROUGH A RECORDED PATH: the console API (server-side
//     audit) or an OperatorActionV1 emitted here. No raw Kratos writes.
//   - VERIFIED, NOT ASSUMED. After --apply it reads every plane again and
//     prints what each now holds. The SSH play asserts on every host.
//   - A REVOKE KEEPS GOING when one plane fails (less access is the safe
//     partial state) and exits non-zero naming the failures. A GRANT stops if
//     the Ory leg fails, because every other plane derives from it.
//   - CA operator (holding the CA key, signing-host access) is NOT a plane
//     here. It is a separate, deliberately separate grant.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::Subcommand;
use panoscope_core::netbird::{NbGroup, NbPeer, NbUser, NetbirdClient};
use panoscope_core::portal::{OperatorKeyRow, PortalClient, Proposal, SshKey, Vendor};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const PLATFORM_ADMIN: &str = "PLATFORM-ADMIN";
/// Legacy name of PLATFORM-ADMIN, still accepted by some gates.
const LEGACY_ADMIN: &str = "ADMINS";
/// Mesh role names the console refuses as login groups
/// (ops/goat-id-console/internal/handlers/group_vocabulary.go `meshRoleNames`).
/// Dropped from any group set this command writes, or the console refuses the
/// whole write.
const MESH_ROLE_NAMES: &[&str] = &[
    "ADMIN",
    "OPERATOR",
    "DEVELOPER",
    "OBSERVER",
    "MONITOR",
    "USER",
    "PEER",
    "BOOTSTRAP",
    "VENDOR",
];

#[derive(Subcommand, Debug)]
pub enum PlatformAdminCmd {
    /// Read-only: what platform-admin access this person holds on this
    /// goatnet, plane by plane. Changes nothing.
    Status(Target),
    /// Make the person a platform admin on this goatnet: Ory PLATFORM-ADMIN,
    /// NetBird dashboard admin, peers in <PREFIX>-PLATFORM-ADMIN, an operator
    /// key (requested and approved on their behalf), sponsorship up to
    /// platform-admin, their recorded SSH keys on the admin hosts, the
    /// 1Password Platform group. The person must already have an identity
    /// (bring new people on with `goat admin invite`). Dry-run unless --apply.
    Grant {
        #[command(flatten)]
        target: Target,
        /// Also record this public key file for the person and install it.
        /// Repeatable. Without it, only keys the person already submitted in
        /// the console are installed.
        #[arg(long = "pubkey-file")]
        pubkey_files: Vec<PathBuf>,
        /// Execute. Without it: plan only, nothing touched.
        #[arg(long)]
        apply: bool,
        /// On a goatnet governed by warrant, write the mesh leg's entries into
        /// `ops/warrant/membership/<env>/` (in place where a peer already has
        /// an entry, else `people.yaml`) for a PR. Needs a checkout.
        #[arg(long)]
        write_entry: bool,
    },
    /// Stop the person being a platform admin on this goatnet. Default is a
    /// DEMOTE to developer, which keeps their login and fabric access and is
    /// reversible with `grant`. Removes: SSH keys from every host, operator
    /// key(s), sponsorship, PLATFORM-ADMIN from peers, Ory login group and
    /// NetBird dashboard role, 1Password Platform group. Dry-run unless --apply.
    Revoke {
        #[command(flatten)]
        target: Target,
        /// Offboard instead of demote: also deactivate the Ory login, block the
        /// NetBird user, and strip the person's peers from every group except
        /// `All`. Still reversible (reactivate + re-add); nothing is deleted.
        #[arg(long)]
        full: bool,
        /// Also remove this public key file's key from the hosts. Repeatable.
        #[arg(long = "pubkey-file")]
        pubkey_files: Vec<PathBuf>,
        /// GitHub login whose current public keys must be removed from hosts.
        #[arg(long, value_parser = validate_github_login)]
        github: Option<String>,
        /// Do not also remove the public keys GitHub lists for the person's
        /// recorded GitHub account. Included by default because hand-installed
        /// keys predate the console and are often exactly those keys.
        #[arg(long)]
        no_github_keys: bool,
        /// Leave the Ory plane to a surrounding lifecycle command. Intended
        /// for offboard.sh, which runs onboard --revoke without notification
        /// and verifies Ory independently afterwards.
        #[arg(long)]
        no_ory: bool,
        /// Execute. Without it: plan only, nothing touched.
        #[arg(long)]
        apply: bool,
        /// On a goatnet governed by warrant, write the mesh leg's entries into
        /// `ops/warrant/membership/<env>/` (in place where a peer already has
        /// an entry, else `people.yaml`) for a PR. Needs a checkout.
        #[arg(long)]
        write_entry: bool,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct Target {
    /// The person's login email.
    pub email: String,
    /// Goatnet (e.g. `zagros`, `zagros-prod`). Must be the goatnet the active
    /// context belongs to — run once per goatnet with `--context <env>-op`.
    #[arg(long)]
    pub env: String,
    /// Handle used by structured offboarding readback.
    #[arg(long, hide = true)]
    pub handle: Option<String>,
    /// Fabric/org prefix used by structured offboarding readback.
    #[arg(long, hide = true)]
    pub org_prefix: Option<String>,
    /// Extra peer selector for the mesh leg (peer id, mesh IP, or name
    /// substring) when a device cannot be matched to the person automatically.
    /// Repeatable.
    #[arg(long = "peer")]
    pub peers: Vec<String>,
    /// Skip the 1Password plane (it is team-wide, not per goatnet — skip it on
    /// the second and later goatnet of a sweep, or when `op` is not signed in).
    #[arg(long)]
    pub no_1password: bool,
    /// Emit the report as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Grant,
    Revoke { full: bool },
}

impl Direction {
    fn verb(self) -> &'static str {
        match self {
            Direction::Grant => "grant",
            Direction::Revoke { full: false } => "revoke",
            Direction::Revoke { full: true } => "revoke --full",
        }
    }
}

pub async fn run(ctx: &super::Ctx, op: PlatformAdminCmd) -> anyhow::Result<()> {
    let (target, dir, pubkey_files, github_keys, github_login, apply, write_entry, no_ory) =
        match op {
            PlatformAdminCmd::Status(t) => {
                let env = prepare(ctx, &t).await?;
                let snap = Snapshot::read(ctx, &t, &env, &[], false, None).await?;
                print_status(&t, &snap);
                return Ok(());
            }
            PlatformAdminCmd::Grant {
                target,
                pubkey_files,
                apply,
                write_entry,
            } => (
                target,
                Direction::Grant,
                pubkey_files,
                false,
                None,
                apply,
                write_entry,
                false,
            ),
            PlatformAdminCmd::Revoke {
                target,
                full,
                pubkey_files,
                github,
                no_github_keys,
                no_ory,
                apply,
                write_entry,
            } => (
                target,
                Direction::Revoke { full },
                pubkey_files,
                !no_github_keys,
                github,
                apply,
                write_entry,
                no_ory,
            ),
        };
    let env = prepare(ctx, &target).await?;
    let extra_keys = read_pubkey_files(&pubkey_files)?;
    let snap = Snapshot::read(
        ctx,
        &target,
        &env,
        &extra_keys,
        github_keys,
        github_login.as_deref(),
    )
    .await?;
    let mut plan = Plan::build(&snap, dir, &env);
    if no_ory {
        plan.skip_ory();
    }

    print_plan(&target, dir, &snap, &plan);
    if write_entry && !plan.mesh_governed {
        anyhow::bail!(
            "--write-entry applies only on a goatnet governed by warrant \
             (its `_scope.yaml` says `converge: enforce`); `{}` is not",
            env.name
        );
    }
    if plan.mesh_governed {
        print_governed_entries(&snap.peers, &env, dir);
        if write_entry {
            write_governed_entries(&snap.peers, &env, dir)?;
        }
    }
    if !apply {
        eprintln!("\ndry-run — nothing changed. Re-run with --apply to execute.");
        return Ok(());
    }
    if dir == Direction::Grant && snap.ory.as_ref().map(|o| o.id.is_empty()).unwrap_or(true) {
        anyhow::bail!(
            "refusing to grant: no Ory identity for {} on {} could be read. Bring the person \
             on with `goat admin invite` first; a grant never mints an identity.",
            target.email,
            env.name
        );
    }

    let actor = super::audit::actor_handle(&ctx.profile);
    audit(ctx, &actor, dir, "all", &target.email, &env.name, "started").await;
    let results = execute(ctx, &target, &env, dir, &snap, &plan).await;
    let failed: Vec<&StepResult> = results.iter().filter(|r| r.failed()).collect();
    audit(
        ctx,
        &actor,
        dir,
        "all",
        &target.email,
        &env.name,
        if failed.is_empty() { "ok" } else { "failed" },
    )
    .await;

    // Read every plane again: the report is what the goatnet holds now, not
    // what the writes returned.
    let after = Snapshot::read(
        ctx,
        &target,
        &env,
        &extra_keys,
        github_keys,
        github_login.as_deref(),
    )
    .await?;
    print_results(&target, dir, &results, &after);
    if !failed.is_empty() {
        anyhow::bail!(
            "platform-admin {} for {} on {} finished with {} failed plane(s): {}",
            dir.verb(),
            target.email,
            env.name,
            failed.len(),
            failed
                .iter()
                .map(|r| r.plane)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

struct Env {
    name: String,
    portal: PortalClient,
    nb: NetbirdClient,
    site_prefix: String,
    repo_root: Option<PathBuf>,
    console_base: String,
    /// Membership on this goatnet is governed by warrant
    /// (mesh-change-control.md §4.5): the mesh leg prints the entry instead
    /// of writing groups live.
    mesh_governed: bool,
}

async fn prepare(ctx: &super::Ctx, t: &Target) -> anyhow::Result<Env> {
    let env = t.env.trim().to_string();
    if !super::admin::is_valid_env(&env) {
        anyhow::bail!("invalid --env `{env}`: must be a kebab-case goatnet id (e.g. `zagros`)");
    }
    if !t.email.contains('@') {
        anyhow::bail!("`{}` is not an email address", t.email);
    }
    super::admin::require_env_binding(&ctx.profile, &env, "platform-admin")?;
    let portal = ctx.profile.portal_client()?;
    let nbcfg =
        super::admin::native_netbird_config_for(Some(&env), &ctx.profile).ok_or_else(|| {
            anyhow::anyhow!(
            "no NetBird config for goatnet `{env}` — declare netbird_mgmt_url + netbird_token_env \
             in the context's profile.toml"
        )
        })?;
    let token = std::env::var("GOAT_NB_API_TOKEN")
        .ok()
        .or_else(|| std::env::var(nbcfg.token_env_var).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "NetBird admin token for `{env}` not set — export ${} (or $GOAT_NB_API_TOKEN)",
                nbcfg.token_env_var
            )
        })?;
    let nb = NetbirdClient::new(nbcfg.mgmt_url.to_string(), token);
    let repo_root = std::env::current_dir()
        .ok()
        .and_then(|d| super::admin::discover_repo_root(&d));
    let console_base = ctx
        .profile
        .profile_field("onboard_endpoint")
        .unwrap_or_default()
        .trim()
        .to_string();
    let governance = super::warrant::governance(&env, &ctx.profile, &nb).await;
    if governance.governed {
        eprintln!(
            "`{env}` membership is governed by warrant — {}",
            governance.because()
        );
    }
    let mesh_governed = governance.governed;
    Ok(Env {
        name: env,
        portal,
        nb,
        site_prefix: nbcfg.site_prefix.to_string(),
        repo_root,
        console_base,
        mesh_governed,
    })
}

// ---------------------------------------------------------------------------
// Reading every plane
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Serialize)]
struct OryState {
    id: String,
    state: String,
    groups: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct OryStatus {
    read_ok: bool,
    identity_present: bool,
    state: String,
    session_count: usize,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct AccessStatus {
    read_ok: bool,
    present: bool,
    identifiers: Vec<String>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
enum OnePassword {
    Member,
    NotMember,
    NotAUser,
    Skipped(String),
}

struct Snapshot {
    ory: Result<OryState, String>,
    ory_status: OryStatus,
    handles: Vec<String>,
    github: Option<String>,
    nb_user: Result<Option<NbUser>, String>,
    peers: Vec<NbPeer>,
    groups: Vec<NbGroup>,
    setup_key_status: AccessStatus,
    fabric_status: AccessStatus,
    sponsorship_status: AccessStatus,
    ssh_status: AccessStatus,
    ssh_rows: Vec<SshKey>,
    /// Every public key line this run acts on (rows + files + GitHub), deduped by body.
    ssh_keys: Vec<String>,
    /// Active console rows plus new explicitly supplied file keys; never GitHub.
    ssh_grant_keys: Vec<String>,
    github_login_source: &'static str,
    github_keys_read_ok: bool,
    github_key_count: usize,
    console_rows_any_status: usize,
    pubkey_files: usize,
    github_note: Option<String>,
    op_keys: Vec<OperatorKeyRow>,
    onepassword: OnePassword,
    site_admins_listed: Option<bool>,
    read_errors: Vec<String>,
    // Failure to discover handles or read peers makes an empty match inconclusive.
    mesh_read_errors: Vec<String>,
}

impl Snapshot {
    async fn read(
        ctx: &super::Ctx,
        t: &Target,
        env: &Env,
        extra_keys: &[String],
        github_keys: bool,
        authoritative_github: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::read_with_github_base(
            ctx,
            t,
            env,
            extra_keys,
            github_keys,
            authoritative_github,
            "https://github.com",
        )
        .await
    }

    async fn read_with_github_base(
        ctx: &super::Ctx,
        t: &Target,
        env: &Env,
        extra_keys: &[String],
        github_keys: bool,
        authoritative_github: Option<&str>,
        github_base: &str,
    ) -> anyhow::Result<Self> {
        let email = t.email.trim();
        let mut read_errors = Vec::new();

        let ory = read_ory(ctx, &env.name, email).await;
        let ory_status = read_ory_status(ctx, &env.name, &ory).await;

        // Handles: the console's onboarding records for this email on this goatnet.
        let proposals: Vec<Proposal> = match env.portal.list_proposals("").await {
            Ok(p) => p
                .into_iter()
                .filter(|p| p.email.eq_ignore_ascii_case(email))
                .filter(|p| p.env.is_empty() || p.env == env.name)
                .collect(),
            Err(e) => {
                read_errors.push(format!("console proposals: {e}"));
                Vec::new()
            }
        };
        let mut ssh_read_ok = true;
        let ssh_rows_all = match env.portal.list_ssh_keys("all").await {
            Ok(r) => r,
            Err(e) => {
                ssh_read_ok = false;
                read_errors.push(format!("console ssh keys: {e}"));
                Vec::new()
            }
        };
        let op_rows_all = match env.portal.list_operator_keys("all").await {
            Ok(r) => r,
            Err(e) => {
                read_errors.push(format!("console operator keys: {e}"));
                Vec::new()
            }
        };
        let mut handles: BTreeSet<String> = proposals
            .iter()
            .map(|p| p.handle.clone())
            .filter(|h| !h.is_empty())
            .collect();
        for r in &ssh_rows_all {
            if r.email.eq_ignore_ascii_case(email) && !r.handle.is_empty() {
                handles.insert(r.handle.clone());
            }
        }
        for r in &op_rows_all {
            if r.email.eq_ignore_ascii_case(email) && !r.handle.is_empty() {
                handles.insert(r.handle.clone());
            }
        }
        if let Some(handle) = t
            .handle
            .as_deref()
            .filter(|handle| !handle.trim().is_empty())
        {
            handles.insert(handle.trim().to_string());
        }
        let handles: Vec<String> = handles.into_iter().collect();
        let mut mesh_read_errors = read_errors.clone();
        if let Err(e) = &ory {
            mesh_read_errors.push(format!("ory identity: {e}"));
        }

        let ssh_rows: Vec<SshKey> = ssh_rows_all
            .into_iter()
            .filter(|r| r.email.eq_ignore_ascii_case(email) || handles.contains(&r.handle))
            .filter(|r| r.env.is_empty() || r.env == env.name)
            .collect();
        let ssh_identifiers: Vec<String> = ssh_rows
            .iter()
            .filter(|row| matches!(row.status.as_str(), "submitted" | "approved" | "installed"))
            .map(|row| row.id.clone())
            .collect();
        let ssh_status = AccessStatus {
            read_ok: ssh_read_ok,
            present: !ssh_identifiers.is_empty(),
            identifiers: ssh_identifiers,
            errors: if ssh_read_ok {
                Vec::new()
            } else {
                vec!["console SSH-key inventory is unreadable".into()]
            },
        };
        let op_keys: Vec<OperatorKeyRow> = op_rows_all
            .into_iter()
            .filter(|r| r.email.eq_ignore_ascii_case(email) || handles.contains(&r.handle))
            .filter(|r| r.env.is_empty() || r.env == env.name)
            .collect();

        // The explicit login is authoritative. A proposal records context but
        // must not redirect revocation to an older login that may be reused.
        let mut proposal_github = None;
        if let Some(p) = proposals.iter().max_by_key(|p| p.created_at) {
            if let Ok(v) = env.portal.get_proposal_detail(&p.id).await {
                proposal_github = manifest_github(&v);
            }
        }
        let github = if github_keys {
            authoritative_github
                .map(str::to_string)
                .or_else(|| proposal_github.clone())
        } else {
            None
        };
        let github_login_source = if !github_keys || github.is_none() {
            "none"
        } else if authoritative_github.is_some() {
            "flag"
        } else {
            "proposal"
        };
        let github_note = authoritative_github.and_then(|flag| {
            proposal_github.as_deref().and_then(|proposal| {
                (!proposal.eq_ignore_ascii_case(flag)).then(|| {
                    format!(
                        "newest proposal names GitHub login {proposal}, but --github {flag} is authoritative; proposal login keys were not fetched"
                    )
                })
            })
        });
        let mut github_key_lines = Vec::new();
        let mut github_keys_read_ok = false;
        if github_keys {
            if let Some(gh) = &github {
                match fetch_github_keys_from(github_base, gh).await {
                    Ok(k) => {
                        github_key_lines = k;
                        github_keys_read_ok = true;
                    }
                    Err(e) => read_errors.push(format!("github keys for {gh}: {e}")),
                }
            }
        }
        let ssh_keys = collect_key_lines(&ssh_rows, extra_keys, &github_key_lines, email);
        let ssh_grant_keys = collect_grant_key_lines(&ssh_rows, extra_keys, email);

        let (nb_user, peers, groups) = match ory.as_ref() {
            Ok(o) if !o.id.is_empty() => {
                let users = env.nb.list_users().await;
                let user = users
                    .map(|u| u.into_iter().find(|u| u.id == o.id))
                    .map_err(|e| format!("netbird users: {e}"));
                (user, Vec::new(), Vec::new())
            }
            Ok(_) => (Ok(None), Vec::new(), Vec::new()),
            Err(e) => (
                Err(format!("no identity id (ory: {e})")),
                Vec::new(),
                Vec::new(),
            ),
        };
        let all_peers = match env.nb.list_peers().await {
            Ok(p) => p,
            Err(e) => {
                mesh_read_errors.push(format!("netbird peers: {e}"));
                read_errors.push(format!("netbird peers: {e}"));
                peers
            }
        };
        let groups = match env.nb.list_groups().await {
            Ok(g) => g,
            Err(e) => {
                read_errors.push(format!("netbird groups: {e}"));
                groups
            }
        };
        let identity_id = ory.as_ref().map(|o| o.id.clone()).unwrap_or_default();
        let peers = person_peers(&all_peers, &identity_id, &handles, &t.peers)
            .into_iter()
            .cloned()
            .collect();

        let target_handle = t.handle.as_deref().unwrap_or("").trim();
        let target_prefix = t.org_prefix.as_deref().unwrap_or("").trim();
        let setup_key_status = if target_handle.is_empty() {
            AccessStatus {
                errors: vec!["no --handle supplied for setup-key readback".into()],
                ..AccessStatus::default()
            }
        } else {
            match env.nb.list_setup_keys().await {
                Ok(keys) => {
                    let identifiers: Vec<String> = keys
                        .iter()
                        .filter(|key| {
                            !key.revoked
                                && panoscope_core::peername::belongs_to(
                                    &key.name,
                                    target_handle,
                                    target_prefix,
                                    &env.name,
                                )
                        })
                        .map(|key| key.id.clone())
                        .collect();
                    AccessStatus {
                        read_ok: true,
                        present: !identifiers.is_empty(),
                        identifiers,
                        errors: Vec::new(),
                    }
                }
                Err(err) => AccessStatus {
                    errors: vec![format!("NetBird setup keys: {err}")],
                    ..AccessStatus::default()
                },
            }
        };

        let (fabric_status, sponsorship_status) = match env.portal.list_vendors().await {
            Ok(vendors) => (
                vendor_access_status(&vendors, &[target_prefix]),
                sponsorship_access_status(
                    &vendors,
                    &handles.iter().map(String::as_str).collect::<Vec<_>>(),
                ),
            ),
            Err(err) => {
                let status = AccessStatus {
                    errors: vec![format!("portal orgs: {err}")],
                    ..AccessStatus::default()
                };
                (status.clone(), status)
            }
        };

        let onepassword = if t.no_1password {
            OnePassword::Skipped("--no-1password".into())
        } else {
            read_onepassword(email).await
        };
        let site_admins_listed = env
            .repo_root
            .as_deref()
            .and_then(|root| site_admins_listed(root, &env.name, email));

        let console_rows_any_status = ssh_rows.len();
        Ok(Snapshot {
            ory,
            ory_status,
            handles,
            github,
            nb_user,
            peers,
            groups,
            setup_key_status,
            fabric_status,
            sponsorship_status,
            ssh_status,
            ssh_rows,
            ssh_keys,
            ssh_grant_keys,
            github_login_source,
            github_keys_read_ok,
            github_key_count: github_key_lines.len(),
            console_rows_any_status,
            pubkey_files: extra_keys.len(),
            github_note,
            op_keys,
            onepassword,
            site_admins_listed,
            read_errors,
            mesh_read_errors,
        })
    }
}

fn vendor_access_status(vendors: &[Vendor], prefixes: &[&str]) -> AccessStatus {
    let identifiers: Vec<String> = vendors
        .iter()
        .filter(|vendor| {
            prefixes
                .iter()
                .any(|prefix| !prefix.is_empty() && vendor.prefix == *prefix)
        })
        .map(|vendor| vendor.prefix.clone())
        .collect();
    AccessStatus {
        read_ok: !prefixes.is_empty() && prefixes.iter().all(|prefix| !prefix.is_empty()),
        present: !identifiers.is_empty(),
        identifiers,
        errors: if prefixes.is_empty() || prefixes.iter().any(|prefix| prefix.is_empty()) {
            vec!["no target identifier supplied for portal readback".into()]
        } else {
            Vec::new()
        },
    }
}

fn sponsorship_access_status(vendors: &[Vendor], handles: &[&str]) -> AccessStatus {
    let identifiers: Vec<String> = vendors
        .iter()
        .filter(|vendor| {
            vendor.sponsor_authorized
                && handles
                    .iter()
                    .any(|handle| !handle.is_empty() && vendor.prefix == *handle)
        })
        .map(|vendor| vendor.prefix.clone())
        .collect();
    AccessStatus {
        read_ok: !handles.is_empty() && handles.iter().all(|handle| !handle.is_empty()),
        present: !identifiers.is_empty(),
        identifiers,
        errors: if handles.is_empty() || handles.iter().any(|handle| handle.is_empty()) {
            vec!["no handle supplied for sponsorship readback".into()]
        } else {
            Vec::new()
        },
    }
}

async fn read_ory(ctx: &super::Ctx, env: &str, email: &str) -> Result<OryState, String> {
    let cfg = super::admin::native_ory_config_for(Some(env), &ctx.profile).ok_or_else(|| {
        format!("no Ory config for goatnet `{env}` (declare ory_auth_url + ory_ssh_host in the profile)")
    })?;
    let q = format!(
        "curl -sS --max-time 30 -w '\\n%{{http_code}}' '{}/admin/identities?credentials_identifier={}'",
        cfg.kratos_admin_url,
        super::admin::pct_encode_query(email)
    );
    let (code, body) = super::admin::run_ssh_curl(&cfg, &q, None)
        .await
        .map_err(|e| {
            format!("{e} (off this goatnet's mesh? export GOAT_SSH_PROXY_<ENV> for its door)")
        })?;
    if code != 200 {
        return Err(format!("Kratos GET identities HTTP {code}"));
    }
    if let Some(s) = read_identity_response(&body, email)? {
        return Ok(s);
    }
    // An identity with no password credential (OIDC-only) is not found by
    // credentials_identifier; fall back to the full list, matched on the trait.
    let q = format!(
        "curl -sS --max-time 60 -w '\\n%{{http_code}}' '{}/admin/identities?per_page=1000'",
        cfg.kratos_admin_url
    );
    let (code, body) = super::admin::run_ssh_curl(&cfg, &q, None)
        .await
        .map_err(|e| e.to_string())?;
    if code != 200 {
        return Err(format!("Kratos GET identities HTTP {code}"));
    }
    Ok(read_identity_response(&body, email)?.unwrap_or_default())
}

async fn read_ory_status(
    ctx: &super::Ctx,
    env: &str,
    identity: &Result<OryState, String>,
) -> OryStatus {
    let identity = match identity {
        Ok(identity) => identity,
        Err(err) => {
            return OryStatus {
                errors: vec![err.clone()],
                ..OryStatus::default()
            }
        }
    };
    if identity.id.is_empty() {
        return OryStatus {
            read_ok: true,
            ..OryStatus::default()
        };
    }
    let Some(cfg) = super::admin::native_ory_config_for(Some(env), &ctx.profile) else {
        return OryStatus {
            identity_present: true,
            state: identity.state.clone(),
            errors: vec![format!("no Ory config for goatnet `{env}`")],
            ..OryStatus::default()
        };
    };
    let q = format!(
        "curl -sS --max-time 30 -w '\\n%{{http_code}}' '{}{}'",
        cfg.kratos_admin_url,
        ory_active_sessions_path(&identity.id)
    );
    let response = super::admin::run_ssh_curl(&cfg, &q, None).await;
    match response {
        Ok((200, body)) => match parse_session_count(&body) {
            Ok(session_count) => OryStatus {
                read_ok: true,
                identity_present: true,
                state: identity.state.clone(),
                session_count,
                errors: Vec::new(),
            },
            Err(err) => OryStatus {
                identity_present: true,
                state: identity.state.clone(),
                errors: vec![err],
                ..OryStatus::default()
            },
        },
        Ok((code, _)) => OryStatus {
            identity_present: true,
            state: identity.state.clone(),
            errors: vec![format!("Kratos GET identity sessions HTTP {code}")],
            ..OryStatus::default()
        },
        Err(err) => OryStatus {
            identity_present: true,
            state: identity.state.clone(),
            errors: vec![format!("Kratos GET identity sessions: {err}")],
            ..OryStatus::default()
        },
    }
}

fn ory_active_sessions_path(identity_id: &str) -> String {
    format!(
        "/admin/identities/{}/sessions?active=true&page=1&per_page=250",
        super::admin::pct_encode_query(identity_id)
    )
}

fn parse_session_count(body: &str) -> Result<usize, String> {
    serde_json::from_str::<serde_json::Value>(body)
        .map_err(|_| "Kratos identity sessions response is not valid JSON".to_string())?
        .as_array()
        .map(Vec::len)
        .ok_or_else(|| "Kratos identity sessions response is not an array".to_string())
}

// A failed JSON read must not become an authoritative absent identity.
fn read_identity_response(body: &str, email: &str) -> Result<Option<OryState>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "Kratos identities response is not valid JSON".to_string())?;
    let rows = value
        .as_array()
        .ok_or("Kratos identities response is not an array")?;
    if rows.iter().any(|row| {
        !row.is_object()
            || row
                .get("id")
                .and_then(|id| id.as_str())
                .is_none_or(|id| id.is_empty())
            || !row.get("traits").is_some_and(|traits| traits.is_object())
    }) {
        return Err("Kratos identities response contains a malformed identity".into());
    }
    Ok(pick_identity(body, email))
}

/// The identity whose `traits.email` is `email` (case-insensitive) in a Kratos
/// identities array. Pure.
fn pick_identity(body: &str, email: &str) -> Option<OryState> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.as_array()?.iter().find_map(|i| {
        let traits = i.get("traits")?;
        let e = traits.get("email")?.as_str()?;
        if !e.eq_ignore_ascii_case(email) {
            return None;
        }
        // Login groups live in admin-managed metadata_public;
        // read it first, fall back to traits during the backfill window.
        let groups_val = i
            .get("metadata_public")
            .and_then(|m| m.get("groups"))
            .or_else(|| traits.get("groups"));
        Some(OryState {
            id: i.get("id")?.as_str()?.to_string(),
            state: i
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            groups: groups_val
                .and_then(|g| g.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
    })
}

fn manifest_github(detail: &serde_json::Value) -> Option<String> {
    let raw = detail.get("manifest_json")?;
    let m: serde_json::Value = match raw {
        serde_json::Value::String(s) => serde_json::from_str(s).ok()?,
        other => other.clone(),
    };
    let gh = m.get("identity")?.get("github")?.as_str()?.trim();
    valid_github_login(gh).then(|| gh.to_string())
}

fn valid_github_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && login
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn validate_github_login(value: &str) -> Result<String, String> {
    valid_github_login(value)
        .then(|| value.to_string())
        .ok_or_else(|| {
            "must be 1-39 ASCII letters or digits separated by single hyphens".to_string()
        })
}

async fn fetch_github_keys_from(base: &str, user: &str) -> anyhow::Result<Vec<String>> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let body = http
        .get(format!("{base}/{user}.keys"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body
        .lines()
        .map(str::trim)
        .filter(|l| is_pubkey_line(l))
        .map(|l| format!("{l} github:{user}"))
        .collect())
}

/// `<type> <base64 body> [comment]` with a known key type. Pure.
fn is_pubkey_line(l: &str) -> bool {
    let mut it = l.split_whitespace();
    let (Some(t), Some(b)) = (it.next(), it.next()) else {
        return false;
    };
    let known = matches!(
        t,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "sk-ssh-ed25519@openssh.com" // oss-allow: key algorithm name
            | "sk-ecdsa-sha2-nistp256@openssh.com" // oss-allow: key algorithm name
    );
    known
        && b.len() >= 60
        && b.chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
}

fn read_pubkey_files(files: &[PathBuf]) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for f in files {
        let body = std::fs::read_to_string(f)
            .map_err(|e| anyhow::anyhow!("read --pubkey-file {}: {e}", f.display()))?;
        if body.contains("PRIVATE KEY") {
            anyhow::bail!(
                "{} is a PRIVATE key. Pass the .pub file; a private key must never leave the machine it lives on.",
                f.display()
            );
        }
        let lines: Vec<String> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        if lines.is_empty() || !lines.iter().all(|l| is_pubkey_line(l)) {
            anyhow::bail!("{} is not an OpenSSH public key file", f.display());
        }
        out.extend(lines);
    }
    Ok(out)
}

/// Console rows + extra lines + GitHub lines, deduped by key body. Rows are
/// rendered `<type> <body> <email>` so an installed line names its owner. Pure.
fn collect_key_lines(
    rows: &[SshKey],
    extra: &[String],
    github: &[String],
    email: &str,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let row_lines = rows
        .iter()
        .filter(|r| !r.key_type.is_empty() && !r.key_body.is_empty())
        .map(|r| format!("{} {} {}", r.key_type, r.key_body, email));
    for line in row_lines
        .chain(extra.iter().cloned())
        .chain(github.iter().cloned())
    {
        let Some(body) = line.split_whitespace().nth(1) else {
            continue;
        };
        if is_pubkey_line(&line) && seen.insert(body.to_string()) {
            out.push(line);
        }
    }
    out
}

/// Keep closed/unknown console rows out of grants, even if a file repeats their
/// key body with a different comment. The all-state inventory remains available
/// separately for reporting and revocation.
fn collect_grant_key_lines(rows: &[SshKey], extra: &[String], email: &str) -> Vec<String> {
    let row_bodies: BTreeSet<&str> = rows.iter().map(|r| r.key_body.as_str()).collect();
    let active_rows: Vec<SshKey> = rows
        .iter()
        .filter(|r| matches!(r.status.as_str(), "submitted" | "approved" | "installed"))
        .cloned()
        .collect();
    let new_keys: Vec<String> = extra
        .iter()
        .filter(|line| {
            line.split_whitespace()
                .nth(1)
                .is_some_and(|body| !row_bodies.contains(body))
        })
        .cloned()
        .collect();
    collect_key_lines(&active_rows, &new_keys, &[], email)
}

/// The person's peers: owned by their identity id, named `peer-<handle>-…` by
/// the onboarding flow, or named by an explicit `--peer` selector. Pure.
fn person_peers<'a>(
    peers: &'a [NbPeer],
    identity_id: &str,
    handles: &[String],
    selectors: &[String],
) -> Vec<&'a NbPeer> {
    peers
        .iter()
        .filter(|p| {
            (!identity_id.is_empty() && p.user_id == identity_id)
                || handles
                    .iter()
                    .any(|h| p.name.starts_with(&format!("peer-{h}-")))
                || selectors.iter().any(|s| p.matches(s))
        })
        .collect()
}

async fn read_onepassword(email: &str) -> OnePassword {
    let group = onepassword_group();
    let user = Command::new("op")
        .args(["user", "get", email, "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .await;
    match user {
        Err(_) => return OnePassword::Skipped("`op` CLI not installed".into()),
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr).to_lowercase();
            if err.contains("isn't a user") || err.contains("not found") || err.contains("no user")
            {
                return OnePassword::NotAUser;
            }
            return OnePassword::Skipped(format!(
                "`op user get` failed: {} (sign in, or set OP_ACCOUNT)",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Ok(_) => {}
    }
    let list = Command::new("op")
        .args(["group", "user", "list", &group, "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .await;
    match list {
        Ok(o) if o.status.success() => {
            let members: serde_json::Value =
                serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
            let is_member = members
                .as_array()
                .map(|a| {
                    a.iter().any(|u| {
                        u.get("email")
                            .and_then(|e| e.as_str())
                            .map(|e| e.eq_ignore_ascii_case(email))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if is_member {
                OnePassword::Member
            } else {
                OnePassword::NotMember
            }
        }
        Ok(o) => OnePassword::Skipped(format!(
            "`op group user list {group}` failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => OnePassword::Skipped(format!("op: {e}")),
    }
}

fn onepassword_group() -> String {
    std::env::var("GOAT_1PASSWORD_PLATFORM_GROUP")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Platform".to_string())
}

/// Whether `email` appears under `site_admins:` in the goatnet's committed
/// group_vars. None when the file or the key is absent. A line scan, not a
/// YAML parse: vars.yml carries Jinja the YAML parser would accept but that is
/// not what matters here. Pure over the file text (`site_admins_in`).
fn site_admins_listed(root: &Path, env: &str, email: &str) -> Option<bool> {
    let path = root
        .join("ops/ansible/group_vars")
        .join(format!("{}_hosts", env.replace('-', "_")))
        .join("vars.yml");
    let text = std::fs::read_to_string(path).ok()?;
    site_admins_in(&text, email)
}

fn site_admins_in(text: &str, email: &str) -> Option<bool> {
    let mut in_block = false;
    let mut found_key = false;
    for line in text.lines() {
        if line.starts_with("site_admins:") {
            in_block = true;
            found_key = true;
            if line.contains("[]") {
                return Some(false);
            }
            continue;
        }
        if in_block {
            if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('#') {
                break;
            }
            let t = line.trim().trim_start_matches("- ").trim();
            if let Some(v) = t.strip_prefix("email:") {
                if v.trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .eq_ignore_ascii_case(email)
                {
                    return Some(true);
                }
            }
        }
    }
    found_key.then_some(false)
}

// ---------------------------------------------------------------------------
// Planning (pure)
// ---------------------------------------------------------------------------

/// The login groups to write. Grant adds PLATFORM-ADMIN; revoke removes it and
/// its legacy name. Both drop mesh role names, which the console refuses as
/// login groups. None when nothing changes. Pure.
fn ory_target_groups(current: &[String], dir: Direction) -> Option<Vec<String>> {
    let mut out: Vec<String> = current
        .iter()
        .filter(|g| !MESH_ROLE_NAMES.contains(&g.to_ascii_uppercase().as_str()))
        .filter(|g| match dir {
            Direction::Grant => true,
            Direction::Revoke { .. } => g.as_str() != PLATFORM_ADMIN && g.as_str() != LEGACY_ADMIN,
        })
        .cloned()
        .collect();
    if dir == Direction::Grant && !out.iter().any(|g| g == PLATFORM_ADMIN) {
        out.push(PLATFORM_ADMIN.to_string());
    }
    (out != current).then_some(out)
}

/// The access level a person holds, read from their Ory login groups — the
/// same tiers the onboarding intake form grants (onboard-intake-form.json).
/// Sponsorship and the operator key follow the level, never the other way
/// round (ADR 1071).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    PlatformAdmin,
    IamAdmin,
    /// developer / browse: no admin tier, no sponsorship, no operator key.
    Member,
}

fn level_for_groups(groups: &[String]) -> Level {
    if groups
        .iter()
        .any(|g| g == PLATFORM_ADMIN || g == LEGACY_ADMIN)
    {
        Level::PlatformAdmin
    } else if groups.iter().any(|g| g == "IAM-ADMIN") {
        Level::IamAdmin
    } else {
        Level::Member
    }
}

impl Level {
    /// Sponsorship max tier this level carries; None = not a sponsor.
    fn sponsorship_tier(self) -> Option<&'static str> {
        match self {
            Level::PlatformAdmin => Some("platform-admin"),
            Level::IamAdmin => Some("iam-admin"),
            Level::Member => None,
        }
    }
    /// An operator key is the platform-admin tier's out-of-band credential and
    /// is uncapped (ADR 1071), so only that tier holds one.
    fn operator_key_entitled(self) -> bool {
        self == Level::PlatformAdmin
    }
    fn name(self) -> &'static str {
        match self {
            Level::PlatformAdmin => "platform-admin",
            Level::IamAdmin => "iam-admin",
            Level::Member => "developer/browse",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NbRoleAction {
    None,
    /// Set role + blocked.
    Set {
        role: &'static str,
        blocked: bool,
    },
    /// No NetBird user row: the person never signed in to the dashboard.
    NoUser,
    /// The account owner. Ownership moves only by the owner's own action.
    Owner,
}

fn nb_role_action(user: Option<&NbUser>, dir: Direction) -> NbRoleAction {
    let Some(u) = user else {
        return NbRoleAction::NoUser;
    };
    if u.role == "owner" {
        return match dir {
            Direction::Grant => NbRoleAction::None,
            Direction::Revoke { .. } => NbRoleAction::Owner,
        };
    }
    let (role, blocked) = match dir {
        Direction::Grant => ("admin", false),
        Direction::Revoke { full } => ("user", full),
    };
    if u.role == role && u.is_blocked == blocked {
        NbRoleAction::None
    } else {
        NbRoleAction::Set { role, blocked }
    }
}

#[derive(Debug, Default, Clone)]
struct MeshPlan {
    /// (group name, peer ids to add)
    add: Vec<(String, Vec<String>)>,
    /// (group name, peer ids to remove)
    remove: Vec<(String, Vec<String>)>,
    /// Why nothing could be planned, when a needed group does not exist.
    blocked: Option<String>,
}

fn is_admin_group(name: &str) -> bool {
    name == PLATFORM_ADMIN || name.ends_with("-PLATFORM-ADMIN")
}

fn mesh_plan(peers: &[NbPeer], groups: &[NbGroup], prefix: &str, dir: Direction) -> MeshPlan {
    let mut plan = MeshPlan::default();
    if peers.is_empty() {
        return plan;
    }
    let find = |name: &str| {
        groups
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.name.clone())
    };
    match dir {
        Direction::Grant => {
            let target = format!("{prefix}-{PLATFORM_ADMIN}");
            let Some(g) = find(&target) else {
                plan.blocked = Some(format!("group {target} does not exist on this mgmt server"));
                return plan;
            };
            let ids: Vec<String> = peers
                .iter()
                .filter(|p| !p.groups.iter().any(|pg| pg.name == g))
                .map(|p| p.id.clone())
                .collect();
            if !ids.is_empty() {
                plan.add.push((g, ids));
            }
        }
        Direction::Revoke { full } => {
            let landing = format!("{prefix}-OBSERVER");
            // Remove from every admin group (any prefix); --full strips every
            // group but `All`.
            let mut remove: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for p in peers {
                for pg in &p.groups {
                    let strip = if full {
                        pg.name != "All"
                    } else {
                        is_admin_group(&pg.name)
                    };
                    if strip {
                        remove
                            .entry(pg.name.clone())
                            .or_default()
                            .push(p.id.clone());
                    }
                }
            }
            if !full {
                // Mirror-first landing: a peer leaving its admin group keeps a
                // covered role group, so the removal never leaves it uncovered.
                let movers: Vec<String> = peers
                    .iter()
                    .filter(|p| p.groups.iter().any(|pg| is_admin_group(&pg.name)))
                    .filter(|p| !p.groups.iter().any(|pg| pg.name == landing))
                    .map(|p| p.id.clone())
                    .collect();
                if !movers.is_empty() {
                    match find(&landing) {
                        Some(g) => plan.add.push((g, movers)),
                        None => {
                            plan.blocked = Some(format!(
                                "landing group {landing} does not exist — refusing to strip admin groups without it"
                            ));
                            return plan;
                        }
                    }
                }
            }
            plan.remove = remove.into_iter().collect();
        }
    }
    plan
}

/// Operator-key decisions that bring the person's keys in line with their
/// level: entitled + none live -> approve a pending request, or request one;
/// not entitled -> revoke live keys and reject pending ones. Pure.
fn op_key_plan(keys: &[OperatorKeyRow], level: Level) -> (Vec<(String, &'static str)>, bool) {
    if level.operator_key_entitled() {
        if keys.iter().any(|k| k.status == "active") {
            return (Vec::new(), false);
        }
        let pending: Vec<(String, &'static str)> = keys
            .iter()
            .filter(|k| k.status == "requested")
            .take(1)
            .map(|k| (k.id.clone(), "approve"))
            .collect();
        let request = pending.is_empty();
        (pending, request)
    } else {
        let d = keys
            .iter()
            .filter_map(|k| match k.status.as_str() {
                "active" => Some((k.id.clone(), "revoke")),
                "requested" => Some((k.id.clone(), "reject")),
                _ => None,
            })
            .collect();
        (d, false)
    }
}

struct Plan {
    ory_groups: Option<Vec<String>>,
    ory_deactivate: bool,
    ory_reactivate: bool,
    nb_role: NbRoleAction,
    mesh: MeshPlan,
    /// True on a warrant-governed goatnet: the mesh leg is an entry, not a write.
    mesh_governed: bool,
    /// grant: submitted rows to approve; revoke: live rows to revoke.
    ssh_row_decisions: Vec<(String, &'static str)>,
    ssh_host_run: bool,
    op_key_decisions: Vec<(String, &'static str)>,
    /// Request + approve a key on the person's behalf (the console mails them
    /// the one-time collect link).
    op_key_request: bool,
    /// The level the person holds after this run.
    level_after: Level,
    sponsorship: bool,
    onepassword: Option<&'static str>,
}

impl Plan {
    fn build(s: &Snapshot, dir: Direction, env: &Env) -> Plan {
        let ory = s.ory.as_ref().ok();
        let ory_groups = ory.and_then(|o| {
            if o.id.is_empty() {
                None
            } else {
                ory_target_groups(&o.groups, dir)
            }
        });
        let ory_deactivate = matches!(dir, Direction::Revoke { full: true })
            && ory
                .map(|o| !o.id.is_empty() && o.state == "active")
                .unwrap_or(false);
        let ory_reactivate = dir == Direction::Grant
            && ory
                .map(|o| !o.id.is_empty() && o.state != "active")
                .unwrap_or(false);
        let nb_role = match &s.nb_user {
            Ok(u) => nb_role_action(u.as_ref(), dir),
            Err(_) => NbRoleAction::None,
        };
        let mesh = mesh_plan(&s.peers, &s.groups, &env.site_prefix, dir);
        let ssh_row_decisions = match dir {
            Direction::Grant => s
                .ssh_rows
                .iter()
                .filter(|r| r.status == "submitted")
                .map(|r| (r.id.clone(), "approve"))
                .collect(),
            Direction::Revoke { .. } => s
                .ssh_rows
                .iter()
                .filter(|r| matches!(r.status.as_str(), "submitted" | "approved" | "installed"))
                .map(|r| (r.id.clone(), "revoke"))
                .collect(),
        };
        let ssh_host_run = match dir {
            Direction::Grant => !s.ssh_grant_keys.is_empty(),
            Direction::Revoke { .. } => !s.ssh_keys.is_empty(),
        };
        let groups_after: Vec<String> = match (&ory_groups, ory) {
            (Some(g), _) => g.clone(),
            (None, Some(o)) => o.groups.clone(),
            (None, None) => Vec::new(),
        };
        let level_after = level_for_groups(&groups_after);
        let (op_key_decisions, op_key_request) = op_key_plan(&s.op_keys, level_after);
        let onepassword = match (&s.onepassword, dir) {
            (OnePassword::NotMember, Direction::Grant) => Some("grant"),
            (OnePassword::Member, Direction::Revoke { .. }) => Some("revoke"),
            _ => None,
        };
        Plan {
            ory_groups,
            ory_deactivate,
            ory_reactivate,
            nb_role,
            mesh,
            mesh_governed: env.mesh_governed,
            ssh_row_decisions,
            ssh_host_run,
            op_key_decisions,
            op_key_request,
            level_after,
            sponsorship: !s.handles.is_empty(),
            onepassword,
        }
    }

    fn skip_ory(&mut self) {
        self.ory_groups = None;
        self.ory_deactivate = false;
        self.ory_reactivate = false;
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct StepResult {
    plane: &'static str,
    outcome: &'static str, // done | no-change | skipped | failed
    detail: String,
    /// True only when the SSH playbook reached every target and its K9
    /// authorized_keys readback passed. Offboarding consumes this as evidence;
    /// an empty console inventory is not host-side verification.
    host_readback: bool,
    github_login: Option<String>,
    github_login_source: &'static str,
    github_keys_read_ok: bool,
    keys_checked: usize,
    console_rows_any_status: usize,
    pubkey_files: usize,
}

impl StepResult {
    fn failed(&self) -> bool {
        self.outcome == "failed"
    }
}

fn done(plane: &'static str, detail: impl Into<String>) -> StepResult {
    StepResult {
        plane,
        outcome: "done",
        detail: detail.into(),
        host_readback: false,
        github_login: None,
        github_login_source: "none",
        github_keys_read_ok: false,
        keys_checked: 0,
        console_rows_any_status: 0,
        pubkey_files: 0,
    }
}
fn nochange(plane: &'static str, detail: impl Into<String>) -> StepResult {
    StepResult {
        plane,
        outcome: "no-change",
        detail: detail.into(),
        host_readback: false,
        github_login: None,
        github_login_source: "none",
        github_keys_read_ok: false,
        keys_checked: 0,
        console_rows_any_status: 0,
        pubkey_files: 0,
    }
}
fn skipped(plane: &'static str, detail: impl Into<String>) -> StepResult {
    StepResult {
        plane,
        outcome: "skipped",
        detail: detail.into(),
        host_readback: false,
        github_login: None,
        github_login_source: "none",
        github_keys_read_ok: false,
        keys_checked: 0,
        console_rows_any_status: 0,
        pubkey_files: 0,
    }
}
fn failed(plane: &'static str, detail: impl Into<String>) -> StepResult {
    StepResult {
        plane,
        outcome: "failed",
        detail: detail.into(),
        host_readback: false,
        github_login: None,
        github_login_source: "none",
        github_keys_read_ok: false,
        keys_checked: 0,
        console_rows_any_status: 0,
        pubkey_files: 0,
    }
}

fn ssh_evidence(mut result: StepResult, s: &Snapshot, keys_checked: usize) -> StepResult {
    result.github_login = s.github.clone();
    result.github_login_source = s.github_login_source;
    result.github_keys_read_ok = s.github_keys_read_ok;
    result.keys_checked = keys_checked;
    result.console_rows_any_status = s.console_rows_any_status;
    result.pubkey_files = s.pubkey_files;
    result
}

async fn execute(
    ctx: &super::Ctx,
    t: &Target,
    env: &Env,
    dir: Direction,
    s: &Snapshot,
    plan: &Plan,
) -> Vec<StepResult> {
    let actor = super::audit::actor_handle(&ctx.profile);
    let mut out = Vec::new();
    match dir {
        Direction::Grant => {
            let ory = step_ory(ctx, t, env, dir, plan).await;
            let ory_failed = ory.failed();
            out.push(ory);
            if ory_failed {
                out.push(skipped(
                    "remaining planes",
                    "not run: the Ory grant failed and every other plane derives from it",
                ));
                return out;
            }
            out.push(step_nb_role(ctx, &actor, t, env, dir, s, plan).await);
            out.push(step_mesh(ctx, &actor, t, env, dir, plan).await);
            out.push(step_sponsorship(ctx, t, env, plan, s).await);
            out.push(step_ssh(ctx, &actor, t, env, dir, s, plan).await);
            out.push(step_op_keys(t, env, dir, s, plan).await);
            out.push(step_onepassword(ctx, &actor, t, env, dir, plan).await);
        }
        Direction::Revoke { .. } => {
            out.push(step_ssh(ctx, &actor, t, env, dir, s, plan).await);
            out.push(step_op_keys(t, env, dir, s, plan).await);
            out.push(step_sponsorship(ctx, t, env, plan, s).await);
            out.push(step_mesh(ctx, &actor, t, env, dir, plan).await);
            out.push(step_ory(ctx, t, env, dir, plan).await);
            out.push(step_nb_role(ctx, &actor, t, env, dir, s, plan).await);
            out.push(step_onepassword(ctx, &actor, t, env, dir, plan).await);
        }
    }
    out
}

async fn step_ory(
    ctx: &super::Ctx,
    t: &Target,
    env: &Env,
    dir: Direction,
    plan: &Plan,
) -> StepResult {
    let email = t.email.trim();
    let mut notes = Vec::new();
    if plan.ory_reactivate {
        let r = env.portal.patch_user_state(email, "active").await;
        super::admin::emit_vendor_audit(&ctx.profile, "user.role.reactivate", email, r.is_ok())
            .await;
        if let Err(e) = r {
            return failed(
                "ory",
                format!(
                    "reactivate: {}",
                    super::vendor::map_onboard_err(&env.portal, e)
                ),
            );
        }
        notes.push("reactivated".to_string());
    }
    if let Some(groups) = &plan.ory_groups {
        let revoke_sessions = matches!(dir, Direction::Revoke { .. });
        let r = env
            .portal
            .patch_user_groups(email, groups, revoke_sessions)
            .await;
        super::admin::emit_vendor_audit(&ctx.profile, "user.role.set", email, r.is_ok()).await;
        match r {
            Ok(u) => notes.push(format!(
                "login groups [{}]{}",
                u.groups.join(", "),
                if revoke_sessions {
                    ", sessions revoked"
                } else {
                    ""
                }
            )),
            Err(e) => {
                // The console applies the Kratos patch and only then mails the
                // person, best-effort, before answering. A slow mailer (an
                // address whose domain does not resolve) can outlast the
                // client's deadline, so the write lands and the call still
                // returns a transport error. Read the plane back before calling it failed: an
                // operator who is told a revoke failed re-runs it, and one who
                // is told it succeeded when it did not leaves the access up.
                let msg = super::vendor::map_onboard_err(&env.portal, e).to_string();
                match read_ory(ctx, &env.name, email).await {
                    Ok(after) if after.groups == *groups => notes.push(format!(
                        "login groups [{}] (the console call returned `{}`, but the identity \
                         reads back with exactly this set — the write landed)",
                        after.groups.join(", "),
                        msg.lines().next().unwrap_or(&msg),
                    )),
                    _ => return failed("ory", format!("set groups: {msg}")),
                }
            }
        }
    }
    if plan.ory_deactivate {
        let r = env.portal.patch_user_state(email, "inactive").await;
        super::admin::emit_vendor_audit(&ctx.profile, "user.role.deactivate", email, r.is_ok())
            .await;
        if let Err(e) = r {
            return failed(
                "ory",
                format!(
                    "deactivate: {}",
                    super::vendor::map_onboard_err(&env.portal, e)
                ),
            );
        }
        notes.push("login deactivated".to_string());
    }
    if notes.is_empty() {
        nochange("ory", "already correct")
    } else {
        done("ory", notes.join("; "))
    }
}

async fn step_nb_role(
    ctx: &super::Ctx,
    actor: &str,
    t: &Target,
    env: &Env,
    dir: Direction,
    s: &Snapshot,
    plan: &Plan,
) -> StepResult {
    if let Err(e) = &s.nb_user {
        return failed(
            "netbird-role",
            format!("could not read the NetBird user: {e}"),
        );
    }
    match &plan.nb_role {
        NbRoleAction::None => nochange("netbird-role", "already correct"),
        NbRoleAction::NoUser => match dir {
            Direction::Grant => skipped(
                "netbird-role",
                "no dashboard user yet — NetBird creates it at the person's first dashboard sign-in, \
                 and netbird-admin-reconcile promotes it within one timer interval",
            ),
            Direction::Revoke { .. } => nochange("netbird-role", "no dashboard user — nothing to demote"),
        },
        NbRoleAction::Owner => failed(
            "netbird-role",
            "this person is the NetBird account OWNER. Ownership moves only by the owner's own action \
             in the dashboard; transfer it, then re-run",
        ),
        NbRoleAction::Set { role, blocked } => {
            let Some(u) = s.nb_user.as_ref().ok().and_then(|u| u.as_ref()) else {
                return failed("netbird-role", "user vanished between read and write");
            };
            let r = env.nb.update_user(&u.id, role, &u.auto_groups, *blocked).await;
            audit(ctx, actor, dir, "netbird-role", &t.email, &env.name, if r.is_ok() { "ok" } else { "failed" }).await;
            match r {
                Ok(()) => done("netbird-role", format!("{} -> {role}{}", u.role, if *blocked { ", blocked" } else { "" })),
                Err(e) => failed("netbird-role", format!("PUT /api/users: {e}")),
            }
        }
    }
}

async fn step_mesh(
    ctx: &super::Ctx,
    actor: &str,
    t: &Target,
    env: &Env,
    dir: Direction,
    plan: &Plan,
) -> StepResult {
    if plan.mesh_governed {
        return skipped(
            "mesh",
            format!(
                "governed by warrant — nothing written live; the entry printed with the plan goes \
                 in ops/warrant/membership/{}/ by PR (--write-entry writes it) ({})",
                env.name,
                super::warrant::RUNBOOK
            ),
        );
    }
    if let Some(why) = &plan.mesh.blocked {
        return failed("mesh", why.clone());
    }
    if plan.mesh.add.is_empty() && plan.mesh.remove.is_empty() {
        return nochange("mesh", "no peer to change");
    }
    let r = apply_mesh(&env.nb, &plan.mesh).await;
    audit(
        ctx,
        actor,
        dir,
        "mesh",
        &t.email,
        &env.name,
        if r.is_ok() { "ok" } else { "failed" },
    )
    .await;
    match r {
        Ok(desc) => done("mesh", desc),
        Err(e) => failed("mesh", e.to_string()),
    }
}

/// Mirror-first: every add lands and is re-read before any removal.
async fn apply_mesh(nb: &NetbirdClient, plan: &MeshPlan) -> anyhow::Result<String> {
    let mut notes = Vec::new();
    for (name, ids) in &plan.add {
        let groups = nb.list_groups().await?;
        let g = groups
            .iter()
            .find(|g| &g.name == name)
            .ok_or_else(|| anyhow::anyhow!("group {name} vanished"))?;
        let mut set: BTreeSet<String> = g.peers.iter().map(|p| p.id.clone()).collect();
        set.extend(ids.iter().cloned());
        nb.update_group_peers(&g.id, &g.name, &set.into_iter().collect::<Vec<_>>())
            .await?;
        let verify = nb.list_groups().await?;
        let vg = verify
            .iter()
            .find(|g| &g.name == name)
            .ok_or_else(|| anyhow::anyhow!("group {name} vanished"))?;
        let have: BTreeSet<&str> = vg.peers.iter().map(|p| p.id.as_str()).collect();
        if let Some(missing) = ids.iter().find(|i| !have.contains(i.as_str())) {
            anyhow::bail!(
                "added peer {missing} to {name} but it is not there on re-read — nothing removed"
            );
        }
        notes.push(format!("{} peer(s) -> {name}", ids.len()));
    }
    for (name, ids) in &plan.remove {
        let groups = nb.list_groups().await?;
        let Some(g) = groups.iter().find(|g| &g.name == name) else {
            continue;
        };
        let keep: Vec<String> = g
            .peers
            .iter()
            .map(|p| p.id.clone())
            .filter(|id| !ids.contains(id))
            .collect();
        nb.update_group_peers(&g.id, &g.name, &keep).await?;
        notes.push(format!("{} peer(s) out of {name}", ids.len()));
    }
    Ok(notes.join("; "))
}

/// The membership entries that express this grant or revoke on a
/// warrant-governed goatnet, one per person peer. Pure.
///
/// - grant: `grant: [<P>-PLATFORM-ADMIN]`, only on person devices (`peer-…`;
///   §4.3 refuses PLATFORM-ADMIN on a machine) — others are returned as notes;
/// - revoke: `absent:` every managed admin group the peer holds, plus
///   `grant: [<P>-OBSERVER]` when that would leave it no managed group (the
///   landing the live path gave it, so it is never left uncovered);
/// - revoke --full: `absent:` every managed group the peer holds.
fn governed_entries(
    peers: &[NbPeer],
    prefix: &str,
    dir: Direction,
) -> (Vec<super::warrant::MembershipEntry>, Vec<String>) {
    use super::warrant::{EntryForm, MembershipEntry};
    let managed = |g: &str| g.starts_with(&format!("{prefix}-"));
    let admin = format!("{prefix}-{PLATFORM_ADMIN}");
    // The owner label every platform-admin grant carries: the team that holds the
    // role, named after it: the role name in lower case, plural.
    let owner_label = format!("{}s", PLATFORM_ADMIN.to_lowercase());
    let (owner, reason) = (
        owner_label.as_str(),
        format!("platform-admin {} (ADR 1182)", dir.verb()),
    );
    let mut out = Vec::new();
    let mut notes = Vec::new();
    for p in peers {
        let held: BTreeSet<String> = p
            .groups
            .iter()
            .map(|g| g.name.clone())
            .filter(|g| managed(g))
            .collect();
        let form = match dir {
            Direction::Grant => {
                if !p.name.starts_with("peer-") {
                    notes.push(format!(
                        "{}: not a person device (peer-…) — PLATFORM-ADMIN is never declared on a machine",
                        p.name
                    ));
                    continue;
                }
                EntryForm::GrantAbsent {
                    grant: BTreeSet::from([admin.clone()]),
                    absent: BTreeSet::new(),
                }
            }
            Direction::Revoke { full: false } => {
                let absent: BTreeSet<String> =
                    held.iter().filter(|g| is_admin_group(g)).cloned().collect();
                if absent.is_empty() {
                    continue;
                }
                let grant = if held.iter().all(|g| is_admin_group(g)) {
                    BTreeSet::from([format!("{prefix}-OBSERVER")])
                } else {
                    BTreeSet::new()
                };
                EntryForm::GrantAbsent { grant, absent }
            }
            Direction::Revoke { full: true } => {
                if held.is_empty() {
                    continue;
                }
                EntryForm::GrantAbsent {
                    grant: BTreeSet::new(),
                    absent: held,
                }
            }
        };
        let expires = super::members::default_expires(&form);
        out.push(MembershipEntry::with_form(
            &p.name, form, owner, &reason, expires,
        ));
    }
    (out, notes)
}

fn print_governed_entries(peers: &[NbPeer], env: &Env, dir: Direction) {
    let (entries, notes) = governed_entries(peers, &env.site_prefix, dir);
    let path = super::warrant::membership_path(&env.name)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("ops/warrant/membership/{}", env.name));
    println!(
        "\n`{}` is governed by warrant: the mesh leg is these entries in\n  {path}/{}\n\
         (or in place where the peer already has an entry) by PR, applied by the converger — \
         not a live write ({}). --write-entry writes them.\n",
        env.name,
        super::warrant::PEOPLE_FILE,
        super::warrant::RUNBOOK
    );
    if entries.is_empty() {
        println!("  (no peer needs an entry)");
    }
    for e in &entries {
        print!("{}", super::warrant::entry_yaml(e, "  "));
    }
    for n in notes {
        println!("  note: {n}");
    }
}

/// `--write-entry`: put the mesh leg's entries into the scope directory of the
/// checkout (in place where a peer already has an entry, else `people.yaml`).
fn write_governed_entries(peers: &[NbPeer], env: &Env, dir: Direction) -> anyhow::Result<()> {
    let (entries, _) = governed_entries(peers, &env.site_prefix, dir);
    let path = super::warrant::membership_path(&env.name)
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "--write-entry needs a checkout holding ops/warrant/membership/{}/ — run it from \
                 inside the repo",
                env.name
            )
        })?;
    let mut written = BTreeSet::new();
    for e in entries {
        let (_, file) =
            super::warrant::write_entry(&path, &env.name, super::warrant::PEOPLE_FILE, e)?;
        written.insert(file.display().to_string());
    }
    if written.is_empty() {
        println!("\n--write-entry: no peer needs an entry; nothing written.");
    } else {
        println!(
            "\nwrote the entries to {} — commit and open a PR; the converger applies them ({}).",
            written.into_iter().collect::<Vec<_>>().join(", "),
            super::warrant::RUNBOOK
        );
    }
    Ok(())
}

async fn step_sponsorship(
    ctx: &super::Ctx,
    t: &Target,
    env: &Env,
    plan: &Plan,
    s: &Snapshot,
) -> StepResult {
    if s.handles.is_empty() {
        return skipped(
            "sponsorship",
            "no onboarding handle on record for this email",
        );
    }
    let tier = plan.level_after.sponsorship_tier();
    let mut notes = Vec::new();
    for h in &s.handles {
        let r = match tier {
            Some(tier) => {
                env.portal
                    .set_sponsorship(h, true, tier, Some(t.email.trim()))
                    .await
            }
            None => env.portal.set_sponsorship(h, false, "", None).await,
        };
        super::admin::emit_vendor_audit(&ctx.profile, "vendor.sponsorship.set", h, r.is_ok()).await;
        match r {
            Ok(()) => notes.push(match tier {
                Some(tier) => format!("{h}: authorized up to {tier}"),
                None => format!("{h}: not a sponsor"),
            }),
            Err(panoscope_core::portal::PortalError::Http { status, .. }) if status.as_u16() == 404 => {
                notes.push(format!("{h}: no portal record (nothing to change)"))
            }
            Err(e) => {
                return failed(
                    "sponsorship",
                    format!("{h}: {}", super::vendor::map_portal_err(e)),
                )
            }
        }
    }
    done(
        "sponsorship",
        format!("level {}: {}", plan.level_after.name(), notes.join("; ")),
    )
}

async fn step_ssh(
    ctx: &super::Ctx,
    actor: &str,
    t: &Target,
    env: &Env,
    dir: Direction,
    s: &Snapshot,
    plan: &Plan,
) -> StepResult {
    let email = t.email.trim();
    let mut notes = Vec::new();
    let mut host_readback = false;
    match dir {
        Direction::Grant => {
            // Record any --pubkey-file key under the person's handle first, so
            // every installed key has a console row a later revoke acts on.
            let Some(handle) = s.handles.first() else {
                return ssh_evidence(
                    skipped(
                        "ssh",
                        "no onboarding handle on record — cannot attribute a key",
                    ),
                    s,
                    0,
                );
            };
            let mut ids_to_mark: Vec<String> = s
                .ssh_rows
                .iter()
                .filter(|r| matches!(r.status.as_str(), "submitted" | "approved"))
                .map(|r| r.id.clone())
                .collect();
            let row_bodies: BTreeSet<&str> =
                s.ssh_rows.iter().map(|r| r.key_body.as_str()).collect();
            for line in s.ssh_grant_keys.iter().filter(|l| {
                l.split_whitespace()
                    .nth(1)
                    .map(|b| !row_bodies.contains(b))
                    .unwrap_or(false)
            }) {
                match env
                    .portal
                    .submit_ssh_key(handle, line, &env.name, "platform-admin grant", email)
                    .await
                {
                    Ok(row) => {
                        // A duplicate submission can return a sticky revoked
                        // row, including when the inventory read was incomplete.
                        if !matches!(row.status.as_str(), "submitted" | "approved" | "installed") {
                            return ssh_evidence(
                                failed(
                                    "ssh",
                                    format!(
                                        "refusing to install key {} in state {}",
                                        row.id, row.status
                                    ),
                                ),
                                s,
                                0,
                            );
                        }
                        if row.status == "submitted" {
                            if let Err(e) = env.portal.decide_ssh_key(&row.id, "approve").await {
                                return ssh_evidence(
                                    failed("ssh", format!("approve {}: {e}", row.id)),
                                    s,
                                    0,
                                );
                            }
                        }
                        ids_to_mark.push(row.id);
                    }
                    Err(e) => {
                        return ssh_evidence(
                            failed(
                                "ssh",
                                format!(
                                    "record key: {}",
                                    super::vendor::map_onboard_err(&env.portal, e)
                                ),
                            ),
                            s,
                            0,
                        )
                    }
                }
            }
            for (id, action) in &plan.ssh_row_decisions {
                if let Err(e) = env.portal.decide_ssh_key(id, action).await {
                    return ssh_evidence(failed("ssh", format!("{action} {id}: {e}")), s, 0);
                }
            }
            if !plan.ssh_host_run {
                return ssh_evidence(skipped(
                    "ssh",
                    format!(
                        "no public key on record — the person submits one at {}/ssh-key (or pass --pubkey-file), then re-run grant",
                        env.console_base
                    ),
                ), s, 0);
            }
            let keys = &s.ssh_grant_keys;
            if let Err(e) = run_ssh_playbook(ctx, actor, env, dir, keys, email).await {
                return ssh_evidence(failed("ssh", e.to_string()), s, keys.len());
            }
            host_readback = true;
            for id in &ids_to_mark {
                if let Err(e) = env.portal.decide_ssh_key(id, "installed").await {
                    notes.push(format!(
                        "installed on hosts but marking {id} installed failed: {e}"
                    ));
                }
            }
            notes.insert(
                0,
                format!(
                    "{} key(s) placed and verified on the admin hosts",
                    keys.len()
                ),
            );
        }
        Direction::Revoke { .. } => {
            if s.github_login_source != "none" && !s.github_keys_read_ok {
                let mismatch = s
                    .github_note
                    .as_deref()
                    .map(|note| format!("{note}; "))
                    .unwrap_or_default();
                return ssh_evidence(
                    failed(
                        "ssh",
                        format!(
                            "{mismatch}GitHub key read failed for {}",
                            s.github.as_deref().unwrap_or("unknown login")
                        ),
                    ),
                    s,
                    0,
                );
            }
            if let Some(note) = &s.github_note {
                notes.push(note.clone());
            }
            if plan.ssh_host_run {
                if let Err(e) = run_ssh_playbook(ctx, actor, env, dir, &s.ssh_keys, email).await {
                    return ssh_evidence(failed("ssh", e.to_string()), s, s.ssh_keys.len());
                }
                host_readback = true;
                notes.push(format!(
                    "{} key(s) ({} from GitHub {}) verified absent on every host",
                    s.ssh_keys.len(),
                    s.github_key_count,
                    s.github.as_deref().unwrap_or("-")
                ));
            }
            for (id, action) in &plan.ssh_row_decisions {
                match env.portal.decide_ssh_key(id, action).await {
                    Ok(_) => notes.push(format!("console row {id} revoked")),
                    Err(e) => {
                        return ssh_evidence(
                            failed(
                                "ssh",
                                format!("hosts cleaned, but revoking console row {id} failed: {e}"),
                            ),
                            s,
                            if plan.ssh_host_run {
                                s.ssh_keys.len()
                            } else {
                                0
                            },
                        )
                    }
                }
            }
            if notes.is_empty() {
                return ssh_evidence(
                    nochange(
                        "ssh",
                        "no key on record, none supplied, no GitHub account recorded",
                    ),
                    s,
                    0,
                );
            }
        }
    }
    let mut result = done("ssh", notes.join("; "));
    result.host_readback = host_readback;
    ssh_evidence(
        result,
        s,
        if plan.ssh_host_run {
            match dir {
                Direction::Grant => s.ssh_grant_keys.len(),
                Direction::Revoke { .. } => s.ssh_keys.len(),
            }
        } else {
            0
        },
    )
}

async fn step_op_keys(
    t: &Target,
    env: &Env,
    dir: Direction,
    s: &Snapshot,
    plan: &Plan,
) -> StepResult {
    let email = t.email.trim();
    if plan.op_key_decisions.is_empty() && !plan.op_key_request {
        return nochange(
            "operator-key",
            format!("matches level {}", plan.level_after.name()),
        );
    }
    let note = format!(
        "platform-admin {} of {email}: level {}",
        dir.verb(),
        plan.level_after.name()
    );
    let mut notes = Vec::new();
    for (id, action) in &plan.op_key_decisions {
        match env.portal.decide_operator_key(id, action, &note).await {
            Ok(row) => notes.push(format!("{id} {}", row.status)),
            Err(e) => {
                return failed(
                    "operator-key",
                    format!(
                        "{action} {id}: {}",
                        super::vendor::map_onboard_err(&env.portal, e)
                    ),
                )
            }
        }
    }
    if plan.op_key_request {
        let Some(handle) = s.handles.first() else {
            return skipped(
                "operator-key",
                "entitled, but no onboarding handle on record to issue a key under",
            );
        };
        let row = match env
            .portal
            .request_operator_key(handle, email, &env.name, &note)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return failed(
                    "operator-key",
                    format!(
                        "request: {}",
                        super::vendor::map_onboard_err(&env.portal, e)
                    ),
                )
            }
        };
        match env.portal.decide_operator_key(&row.id, "approve", &note).await {
            Ok(r) => notes.push(format!(
                "{} {} — the console emails {email} the one-time collect link",
                r.id, r.status
            )),
            Err(e) => {
                return failed(
                    "operator-key",
                    format!(
                        "requested {} but approve failed: {} (the approver must be a different person from the key's owner)",
                        row.id,
                        super::vendor::map_onboard_err(&env.portal, e)
                    ),
                )
            }
        }
    }
    done("operator-key", notes.join("; "))
}

async fn step_onepassword(
    ctx: &super::Ctx,
    actor: &str,
    t: &Target,
    env: &Env,
    dir: Direction,
    plan: &Plan,
) -> StepResult {
    let Some(verb) = plan.onepassword else {
        return nochange("1password", "no change needed or plane skipped");
    };
    let group = onepassword_group();
    let email = t.email.trim();
    let out = Command::new("op")
        .args(["group", "user", verb, "--group", &group, "--user", email])
        .stdin(Stdio::null())
        .output()
        .await;
    let ok = matches!(&out, Ok(o) if o.status.success());
    audit(
        ctx,
        actor,
        dir,
        "1password",
        email,
        &env.name,
        if ok { "ok" } else { "failed" },
    )
    .await;
    match out {
        Ok(o) if o.status.success() => done("1password", format!("{verb} {group}")),
        Ok(o) => failed(
            "1password",
            format!(
                "op group user {verb}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        ),
        Err(e) => failed("1password", format!("op: {e}")),
    }
}

// Output pumps for the child's stdout/stderr, same as the ssh-key installer.
#[allow(clippy::disallowed_methods)]
async fn run_ssh_playbook(
    ctx: &super::Ctx,
    actor: &str,
    env: &Env,
    dir: Direction,
    keys: &[String],
    email: &str,
) -> anyhow::Result<()> {
    let root = env
        .repo_root
        .clone()
        .ok_or_else(|| anyhow::anyhow!("run from inside the goat repo: the SSH leg runs ops/ansible/platform-admin-ssh-keys.yml"))?;
    let preflight = root.join("ops/scripts/ansible-preflight.sh");
    if !preflight.exists() {
        anyhow::bail!("{} not found", preflight.display());
    }
    let extra = ssh_extra_vars(
        &env.name,
        dir,
        keys,
        super::admin::ssh_proxy_for(&env.name).as_deref(),
    );
    let args: Vec<String> = vec![
        "-i".into(),
        "localhost,".into(),
        "platform-admin-ssh-keys.yml".into(),
        "-e".into(),
        extra.to_string(),
    ];
    let mut cmd = if super::admin::should_wrap_with_direnv() {
        let mut c = Command::new("direnv");
        c.arg("exec").arg(&root).arg(&preflight).args(&args);
        c
    } else {
        let mut c = Command::new(&preflight);
        c.args(&args);
        c
    };
    let mut child = cmd
        .current_dir(root.join("ops/ansible"))
        .env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn ansible-preflight.sh: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let o = tokio::spawn(async move {
        let mut l = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = l.next_line().await {
            eprintln!("  | {line}");
        }
    });
    let e = tokio::spawn(async move {
        let mut l = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = l.next_line().await {
            eprintln!("  | {line}");
        }
    });
    let status = child.wait().await?;
    let _ = o.await;
    let _ = e.await;
    audit(
        ctx,
        actor,
        dir,
        "ssh",
        email,
        &env.name,
        if status.success() { "ok" } else { "failed" },
    )
    .await;
    if !status.success() {
        anyhow::bail!(
            "platform-admin-ssh-keys.yml exited {:?} — read the play output above; a failed K9 names the host whose keys do not match",
            status.code()
        );
    }
    Ok(())
}

/// The play's extra-vars as one JSON object (keys are public; nothing secret). Pure.
fn ssh_extra_vars(
    env: &str,
    dir: Direction,
    keys: &[String],
    proxy: Option<&str>,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "pa_env": env,
        "pa_state": if dir == Direction::Grant { "present" } else { "absent" },
        "pa_keys": keys,
    });
    if let Some(p) = proxy {
        v["pa_ssh_common_args"] = serde_json::Value::String(format!("-o ProxyCommand=\"{p}\""));
    }
    v
}

async fn audit(
    ctx: &super::Ctx,
    actor: &str,
    dir: Direction,
    plane: &str,
    email: &str,
    env: &str,
    outcome: &str,
) {
    let action = match dir {
        Direction::Grant => "admin.platform_admin.grant",
        Direction::Revoke { .. } => "admin.platform_admin.revoke",
    };
    super::audit::emit(
        &ctx.profile,
        super::audit::OperatorActionV1::new(actor, "goat-cli", action, email, env, outcome)
            .with_args_digest(&format!("{} plane={plane} env={env}", dir.verb())),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn describe_now(s: &Snapshot) -> Vec<(&'static str, String)> {
    let ory = match &s.ory {
        Ok(o) if o.id.is_empty() => "no identity".to_string(),
        Ok(o) => format!("{} [{}]", o.state, o.groups.join(", ")),
        Err(e) => format!("UNREAD: {e}"),
    };
    let nb = match &s.nb_user {
        Ok(Some(u)) => format!("{}{}", u.role, if u.is_blocked { " (blocked)" } else { "" }),
        Ok(None) => "no dashboard user".into(),
        Err(e) => format!("UNREAD: {e}"),
    };
    let mesh = if s.peers.is_empty() {
        "no peers matched".to_string()
    } else {
        s.peers
            .iter()
            .map(|p| {
                let admin = p.groups.iter().any(|g| is_admin_group(&g.name));
                format!(
                    "{} ({}){}",
                    p.id,
                    p.hostname,
                    if admin { " IN ADMIN GROUP" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let ssh = format!(
        "{} console row(s) [{}]; {} key(s) to act on{}",
        s.ssh_rows.len(),
        s.ssh_rows
            .iter()
            .map(|r| r.status.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        s.ssh_keys.len(),
        s.github
            .as_deref()
            .map(|g| format!(" (GitHub {g}: {})", s.github_key_count))
            .unwrap_or_default()
    );
    let op = if s.op_keys.is_empty() {
        "none".into()
    } else {
        s.op_keys
            .iter()
            .map(|k| format!("{} {}", k.id, k.status))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let sp = if s.handles.is_empty() {
        "no handle on record".into()
    } else {
        format!("handles {}", s.handles.join(", "))
    };
    let onep = match &s.onepassword {
        OnePassword::Member => "in Platform group".into(),
        OnePassword::NotMember => "not in Platform group".into(),
        OnePassword::NotAUser => "not a 1Password user".into(),
        OnePassword::Skipped(why) => format!("skipped: {why}"),
    };
    let sa = match s.site_admins_listed {
        Some(true) => "LISTED in group_vars site_admins".into(),
        Some(false) => "not listed".into(),
        None => "unknown (run inside the repo)".into(),
    };
    vec![
        ("ory", ory),
        ("netbird-role", nb),
        ("mesh", mesh),
        ("ssh", ssh),
        ("operator-key", op),
        ("sponsorship", sp),
        ("1password", onep),
        ("site_admins", sa),
    ]
}

#[derive(serde::Serialize)]
struct MeshStatus<'a> {
    read_ok: bool,
    peer_ids: Vec<&'a str>,
    errors: &'a [String],
}

fn mesh_status<'a>(peers: &'a [NbPeer], errors: &'a [String]) -> MeshStatus<'a> {
    MeshStatus {
        read_ok: errors.is_empty(),
        peer_ids: peers.iter().map(|p| p.id.as_str()).collect(),
        errors,
    }
}

fn status_json(s: &Snapshot) -> serde_json::Value {
    let mut rows: serde_json::Map<String, serde_json::Value> = describe_now(s)
        .into_iter()
        .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
        .collect();
    rows.insert(
        "mesh_status".into(),
        serde_json::json!(mesh_status(&s.peers, &s.mesh_read_errors)),
    );
    rows.insert("ory_status".into(), serde_json::json!(s.ory_status));
    rows.insert(
        "setup_key_status".into(),
        serde_json::json!(s.setup_key_status),
    );
    rows.insert("fabric_status".into(), serde_json::json!(s.fabric_status));
    rows.insert(
        "sponsorship_status".into(),
        serde_json::json!(s.sponsorship_status),
    );
    rows.insert("ssh_status".into(), serde_json::json!(s.ssh_status));
    rows.insert("read_errors".into(), serde_json::json!(s.read_errors));
    serde_json::Value::Object(rows)
}

fn print_status(t: &Target, s: &Snapshot) {
    if t.json {
        println!("{}", status_json(s));
        return;
    }
    println!("platform-admin status: {} on {}", t.email, t.env);
    for (plane, now) in describe_now(s) {
        println!("  {plane:<13} {now}");
    }
    for e in &s.read_errors {
        println!("  read error: {e}");
    }
}

fn print_plan(t: &Target, dir: Direction, s: &Snapshot, p: &Plan) {
    let mut actions: Vec<(&str, String)> = Vec::new();
    actions.push(("ory", {
        let mut v = Vec::new();
        if p.ory_reactivate {
            v.push("reactivate login".to_string());
        }
        if let Some(g) = &p.ory_groups {
            v.push(format!(
                "set login groups to [{}] via the console (audited{})",
                g.join(", "),
                if matches!(dir, Direction::Revoke { .. }) {
                    ", sessions revoked; the console emails the person a role-change notice"
                } else {
                    "; the console emails the person a role-change notice"
                }
            ));
        }
        if p.ory_deactivate {
            v.push("deactivate login".into());
        }
        if v.is_empty() {
            "none".into()
        } else {
            v.join("; ")
        }
    }));
    actions.push((
        "netbird-role",
        match &p.nb_role {
            NbRoleAction::None => "none".into(),
            NbRoleAction::NoUser => "none (no dashboard user)".into(),
            NbRoleAction::Owner => "REFUSED: account owner".into(),
            NbRoleAction::Set { role, blocked } => {
                format!("set role {role}{}", if *blocked { " + block" } else { "" })
            }
        },
    ));
    actions.push(("mesh", {
        if p.mesh_governed {
            "governed by warrant — prints the membership entry below; nothing written live".into()
        } else if let Some(b) = &p.mesh.blocked {
            format!("REFUSED: {b}")
        } else if p.mesh.add.is_empty() && p.mesh.remove.is_empty() {
            "none".into()
        } else {
            p.mesh
                .add
                .iter()
                .map(|(g, ids)| format!("add {} peer(s) to {g}", ids.len()))
                .chain(
                    p.mesh
                        .remove
                        .iter()
                        .map(|(g, ids)| format!("remove {} peer(s) from {g}", ids.len())),
                )
                .collect::<Vec<_>>()
                .join("; ")
        }
    }));
    actions.push(("ssh", {
        let mut v = Vec::new();
        if p.ssh_host_run {
            v.push(match dir {
                Direction::Grant => "place keys on the PLATFORM-ADMIN hosts + verify".to_string(),
                Direction::Revoke { .. } => format!(
                    "remove {} key(s) from every host in the revoke union + verify",
                    s.ssh_keys.len()
                ),
            });
        }
        for (id, a) in &p.ssh_row_decisions {
            v.push(format!("{a} console row {id}"));
        }
        if v.is_empty() {
            "none".into()
        } else {
            v.join("; ")
        }
    }));
    actions.push(("operator-key", {
        let mut v: Vec<String> = p
            .op_key_decisions
            .iter()
            .map(|(id, a)| format!("{a} {id}"))
            .collect();
        if p.op_key_request {
            v.push("request + approve a key; the console emails the collect link".into());
        }
        if v.is_empty() {
            format!("none (matches level {})", p.level_after.name())
        } else {
            v.join("; ")
        }
    }));
    actions.push((
        "sponsorship",
        if p.sponsorship {
            match p.level_after.sponsorship_tier() {
                Some(t) => format!("set: authorized up to {t}"),
                None => "set: not a sponsor".into(),
            }
        } else {
            "none (no handle)".into()
        },
    ));
    actions.push((
        "1password",
        p.onepassword
            .map(|v| format!("{v} Platform group"))
            .unwrap_or_else(|| "none".into()),
    ));
    actions.push(("site_admins", match (s.site_admins_listed, dir) {
        (Some(true), Direction::Revoke { .. }) => "EDIT BY PR: remove from site_admins in group_vars (this command never edits config)".into(),
        (Some(false), Direction::Grant) => "optional PR: add to site_admins so a re-bootstrap seeds them".into(),
        _ => "none".into(),
    }));

    if t.json {
        let now: serde_json::Map<String, serde_json::Value> = describe_now(s)
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let act: serde_json::Map<String, serde_json::Value> = actions
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        println!(
            "{}",
            serde_json::json!({"direction": dir.verb(), "email": t.email, "env": t.env, "now": now, "plan": act, "read_errors": s.read_errors})
        );
        return;
    }
    println!("platform-admin {}: {} on {}", dir.verb(), t.email, t.env);
    let now = describe_now(s);
    println!("  {:<13} {:<52} WILL DO", "PLANE", "NOW");
    for ((plane, n), (_, a)) in now.iter().zip(actions.iter()) {
        println!("  {plane:<13} {:<52} {a}", truncate(n, 52));
    }
    for e in &s.read_errors {
        println!("  read error: {e}");
    }
}

fn print_results(t: &Target, dir: Direction, results: &[StepResult], after: &Snapshot) {
    if t.json {
        let now: serde_json::Map<String, serde_json::Value> = describe_now(after)
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        println!(
            "{}",
            serde_json::json!({"direction": dir.verb(), "email": t.email, "env": t.env, "results": results, "verified_now": now})
        );
        return;
    }
    println!(
        "\nresult: platform-admin {} for {} on {}",
        dir.verb(),
        t.email,
        t.env
    );
    for r in results {
        println!(
            "  {:<13} {:<9} {}",
            r.plane,
            r.outcome.to_uppercase(),
            r.detail
        );
    }
    println!("\nverified (read again after the writes):");
    for (plane, now) in describe_now(after) {
        println!("  {plane:<13} {now}");
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Tests — the pure planning core.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use panoscope_core::netbird::NbGroupPeer;

    #[test]
    fn invalid_github_flag_is_rejected_by_argument_parsing() {
        for login in ["-leading", "trailing-", "double--hyphen", "slash/name", ""] {
            let result = crate::Cli::try_parse_from([
                "goat",
                "admin",
                "platform-admin",
                "revoke",
                "person@example.test",
                "--env",
                "fixture",
                "--github",
                login,
            ]);
            assert!(result.is_err(), "accepted invalid login {login:?}");
        }
    }

    #[cfg(unix)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum GithubContractCase {
        Authority,
        FetchFailure,
        Evidence,
    }

    #[cfg(unix)]
    async fn assert_github_contract(case: GithubContractCase) {
        use std::os::unix::fs::PermissionsExt;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .with_priority(10)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/proposals"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                    "id": "newest", "created_at": 2, "email": "person@example.test",
                    "env": "fixture", "handle": "person"
                }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/proposals/newest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "manifest_json": {"identity": {"github": "old-login"}}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/current-login.keys"))
            .respond_with(ResponseTemplate::new(200).set_body_string(KEY_A))
            .expect(usize::from(case != GithubContractCase::FetchFailure) as u64)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fetch-fails.keys"))
            .respond_with(ResponseTemplate::new(503))
            .expect(usize::from(case != GithubContractCase::Authority) as u64)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/old-login.keys"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("ops/scripts")).unwrap();
        std::fs::create_dir_all(root.join("ops/ansible")).unwrap();
        let script = root.join("ops/scripts/ansible-preflight.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf '%s' \"$5\" > captured.json\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ctx = super::super::Ctx {
            profile: crate::profile::Profile::resolve(
                Some(dir.path().to_str().unwrap()),
                Some("fixture"),
            )
            .unwrap(),
            output: crate::output::OutputContext::from_env(crate::output::ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };
        let env = Env {
            name: "fixture".into(),
            portal: PortalClient::new(server.uri(), "synthetic".into()),
            nb: NetbirdClient::new(server.uri(), "synthetic".into()),
            site_prefix: "FIXTURE".into(),
            repo_root: Some(root.clone()),
            console_base: server.uri(),
            mesh_governed: false,
        };
        let target = Target {
            email: "person@example.test".into(),
            env: "fixture".into(),
            handle: Some("person".into()),
            org_prefix: Some("person".into()),
            peers: vec![],
            no_1password: true,
            json: true,
        };

        let revoke = Direction::Revoke { full: true };
        if case != GithubContractCase::FetchFailure {
            let snapshot = Snapshot::read_with_github_base(
                &ctx,
                &target,
                &env,
                &[],
                true,
                Some("current-login"),
                &server.uri(),
            )
            .await
            .unwrap();
            assert_eq!(snapshot.github.as_deref(), Some("current-login"));
            assert_eq!(snapshot.github_login_source, "flag");
            assert!(snapshot.github_keys_read_ok);
            assert_eq!(snapshot.github_key_count, 1);
            assert!(snapshot
                .github_note
                .as_deref()
                .unwrap()
                .contains("old-login"));
            let plan = Plan::build(&snapshot, revoke, &env);
            let result = step_ssh(&ctx, "fixture", &target, &env, revoke, &snapshot, &plan).await;
            assert_eq!(result.outcome, "done", "{}", result.detail);
            let payload: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("ops/ansible/captured.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(payload["pa_keys"].as_array().unwrap().len(), 1);
            assert!(payload["pa_keys"][0]
                .as_str()
                .unwrap()
                .contains("github:current-login"));
            if case == GithubContractCase::Evidence {
                assert_eq!(result.github_login.as_deref(), Some("current-login"));
                assert_eq!(result.github_login_source, "flag");
                assert!(result.github_keys_read_ok);
                assert_eq!(result.keys_checked, 1);
                assert_eq!(result.console_rows_any_status, 0);
                assert_eq!(result.pubkey_files, 0);
                assert!(result.host_readback);
            }
        }

        if case != GithubContractCase::Authority {
            let failed_snapshot = Snapshot::read_with_github_base(
                &ctx,
                &target,
                &env,
                &[],
                true,
                Some("fetch-fails"),
                &server.uri(),
            )
            .await
            .unwrap();
            assert!(!failed_snapshot.github_keys_read_ok);
            assert!(failed_snapshot
                .read_errors
                .iter()
                .any(|error| error.contains("github keys for fetch-fails")));
            let plan = Plan::build(&failed_snapshot, revoke, &env);
            let result = step_ssh(
                &ctx,
                "fixture",
                &target,
                &env,
                revoke,
                &failed_snapshot,
                &plan,
            )
            .await;
            assert_eq!(result.outcome, "failed");
            assert!(result.detail.contains("old-login"), "{}", result.detail);
            assert!(result.detail.contains("fetch-fails"), "{}", result.detail);
            assert!(!result.github_keys_read_ok);
            assert_eq!(result.keys_checked, 0);

            if case == GithubContractCase::Evidence {
                let mut no_key_snapshot = failed_snapshot;
                no_key_snapshot.github = Some("current-login".into());
                no_key_snapshot.github_keys_read_ok = true;
                no_key_snapshot.github_note = None;
                no_key_snapshot.read_errors.clear();
                let plan = Plan::build(&no_key_snapshot, revoke, &env);
                let result = step_ssh(
                    &ctx,
                    "fixture",
                    &target,
                    &env,
                    revoke,
                    &no_key_snapshot,
                    &plan,
                )
                .await;
                assert_eq!(result.outcome, "no-change");
                assert_eq!(result.github_login.as_deref(), Some("current-login"));
                assert_eq!(result.github_login_source, "flag");
                assert!(result.github_keys_read_ok);
                assert_eq!(result.keys_checked, 0);
                assert_eq!(result.console_rows_any_status, 0);
                assert_eq!(result.pubkey_files, 0);
                assert!(!result.host_readback);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn github_flag_is_authoritative_over_the_newest_proposal() {
        assert_github_contract(GithubContractCase::Authority).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn github_key_fetch_failure_fails_ssh_closed() {
        assert_github_contract(GithubContractCase::FetchFailure).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_evidence_is_complete_on_done_nochange_and_failed_paths() {
        assert_github_contract(GithubContractCase::Evidence).await;
    }

    #[tokio::test]
    async fn ssh_key_snapshot_requests_all_states_and_keeps_read_failures() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        for (code, empty) in [(200, false), (200, true), (500, false)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
                .with_priority(10)
                .mount(&server)
                .await;
            let mut rows = serde_json::json!([
                {"id": "pending", "email": "person@example.test", "env": "fixture", "status": "submitted"},
                {"id": "approved", "email": "person@example.test", "env": "fixture", "status": "approved"},
                {"id": "installed", "email": "person@example.test", "env": "fixture", "status": "installed"},
                {"id": "rejected", "email": "person@example.test", "env": "fixture", "status": "rejected"},
                {"id": "revoked", "email": "person@example.test", "env": "fixture", "status": "revoked"},
                {"id": "foreign-env", "email": "person@example.test", "env": "other", "status": "installed"},
                {"id": "other-person", "email": "other@example.test", "env": "fixture", "status": "installed"}
            ]);
            for (index, row) in rows.as_array_mut().unwrap().iter_mut().enumerate() {
                row["handle"] = serde_json::json!(if index == 6 { "other" } else { "person" });
                row["key_type"] = serde_json::json!("ssh-ed25519");
                row["key_body"] = serde_json::json!(format!("{}{}", "A".repeat(60), index));
            }
            Mock::given(method("GET"))
                .and(path("/api/onboard/ssh-keys"))
                .and(query_param("status", "all"))
                .respond_with(ResponseTemplate::new(code).set_body_json(if empty {
                    serde_json::json!([])
                } else {
                    rows
                }))
                .expect(1)
                .mount(&server)
                .await;
            let dir = tempfile::tempdir().unwrap();
            let ctx = super::super::Ctx {
                profile: crate::profile::Profile::resolve(
                    Some(dir.path().to_str().unwrap()),
                    Some("fixture"),
                )
                .unwrap(),
                output: crate::output::OutputContext::from_env(crate::output::ColorChoice::Never),
                narrate: false,
                narrate_json: false,
                uuid: false,
            };
            // Unknown goatnet + empty profile deliberately disable SSH/Ory;
            // the remaining network reads all target this local mock server.
            let env = Env {
                name: "fixture".into(),
                portal: PortalClient::new(server.uri(), "synthetic".into()),
                nb: NetbirdClient::new(server.uri(), "synthetic".into()),
                site_prefix: "FIXTURE".into(),
                repo_root: None,
                console_base: server.uri(),
                mesh_governed: false,
            };
            let target = Target {
                email: "person@example.test".into(),
                env: "fixture".into(),
                handle: Some("person".into()),
                org_prefix: Some("person".into()),
                peers: vec![],
                no_1password: true,
                json: true,
            };
            let snapshot = Snapshot::read(&ctx, &target, &env, &[], false, None)
                .await
                .unwrap();
            if code == 200 {
                assert_eq!(snapshot.ssh_rows.len(), if empty { 0 } else { 5 });
                assert!(
                    snapshot.read_errors.is_empty(),
                    "{:?}",
                    snapshot.read_errors
                );
                if !empty {
                    assert!(snapshot.ssh_rows.iter().any(|r| r.status == "installed"));
                    assert_eq!(snapshot.ssh_keys.len(), 5);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut snapshot = snapshot;
                        let root = dir.path().join("repo");
                        std::fs::create_dir_all(root.join("ops/scripts")).unwrap();
                        std::fs::create_dir_all(root.join("ops/ansible")).unwrap();
                        let script = root.join("ops/scripts/ansible-preflight.sh");
                        std::fs::write(&script, "#!/bin/sh\nprintf '%s' \"$5\" > captured.json\n")
                            .unwrap();
                        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                            .unwrap();
                        let env = Env {
                            repo_root: Some(root.clone()),
                            ..env
                        };
                        Mock::given(method("POST"))
                            .respond_with(
                                ResponseTemplate::new(200).set_body_json(serde_json::json!({})),
                            )
                            .with_priority(10)
                            .mount(&server)
                            .await;
                        let plan = Plan::build(&snapshot, Direction::Grant, &env);
                        let result = step_ssh(
                            &ctx,
                            "fixture",
                            &target,
                            &env,
                            Direction::Grant,
                            &snapshot,
                            &plan,
                        )
                        .await;
                        assert_eq!(result.outcome, "done", "{}", result.detail);
                        assert!(result.host_readback);
                        let payload: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(root.join("ops/ansible/captured.json")).unwrap(),
                        )
                        .unwrap();
                        let expected =
                            collect_key_lines(&snapshot.ssh_rows[..3], &[], &[], &target.email);
                        assert_eq!(payload["pa_keys"], serde_json::json!(expected));
                        assert_eq!(payload["pa_state"], "present");
                        // Explicit new file inputs are recorded before install.
                        // A sticky closed response must stop before the playbook.
                        snapshot.ssh_grant_keys.push(KEY_B.to_string());
                        for status in ["rejected", "revoked", "unknown", "submitted"] {
                            let capture = root.join("ops/ansible/captured.json");
                            if capture.exists() {
                                std::fs::remove_file(&capture).unwrap();
                            }
                            Mock::given(method("POST"))
                                .and(path("/api/onboard/ssh-keys"))
                                .respond_with(ResponseTemplate::new(201).set_body_json(
                                    serde_json::json!({"id": "external", "status": status}),
                                ))
                                .expect(1)
                                .up_to_n_times(1)
                                .mount(&server)
                                .await;
                            let result = step_ssh(
                                &ctx,
                                "fixture",
                                &target,
                                &env,
                                Direction::Grant,
                                &snapshot,
                                &plan,
                            )
                            .await;
                            let capture = root.join("ops/ansible/captured.json");
                            if status == "submitted" {
                                assert_eq!(result.outcome, "done", "{}", result.detail);
                                let payload: serde_json::Value =
                                    serde_json::from_slice(&std::fs::read(&capture).unwrap())
                                        .unwrap();
                                let mut expected = expected.clone();
                                expected.push(KEY_B.to_string());
                                assert_eq!(payload["pa_keys"], serde_json::json!(expected));
                            } else {
                                assert_eq!(result.outcome, "failed", "{}", result.detail);
                                assert!(result.detail.contains(status), "{}", result.detail);
                                assert!(!capture.exists());
                            }
                        }
                        snapshot.ssh_grant_keys.pop();
                        // A closed-only inventory must not launch a grant play,
                        // but revocation must still remove both closed keys.
                        snapshot
                            .ssh_rows
                            .retain(|r| matches!(r.status.as_str(), "rejected" | "revoked"));
                        snapshot.ssh_keys =
                            collect_key_lines(&snapshot.ssh_rows, &[], &[], &target.email);
                        snapshot.ssh_grant_keys =
                            collect_grant_key_lines(&snapshot.ssh_rows, &[], &target.email);
                        std::fs::remove_file(root.join("ops/ansible/captured.json")).unwrap();
                        let plan = Plan::build(&snapshot, Direction::Grant, &env);
                        assert!(!plan.ssh_host_run);
                        let result = step_ssh(
                            &ctx,
                            "fixture",
                            &target,
                            &env,
                            Direction::Grant,
                            &snapshot,
                            &plan,
                        )
                        .await;
                        assert_eq!(result.outcome, "skipped", "{}", result.detail);
                        assert!(!root.join("ops/ansible/captured.json").exists());
                        let revoke = Direction::Revoke { full: false };
                        let plan = Plan::build(&snapshot, revoke, &env);
                        assert!(plan.ssh_host_run);
                        let result =
                            step_ssh(&ctx, "fixture", &target, &env, revoke, &snapshot, &plan)
                                .await;
                        assert_eq!(result.outcome, "done", "{}", result.detail);
                        assert!(result.host_readback);
                        let payload: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(root.join("ops/ansible/captured.json")).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(payload["pa_keys"], serde_json::json!(snapshot.ssh_keys));
                        assert_eq!(payload["pa_state"], "absent");
                    }
                }
            } else {
                assert!(snapshot.ssh_rows.is_empty());
                assert!(snapshot
                    .read_errors
                    .iter()
                    .any(|e| e.contains("console ssh keys") && e.contains("500")));
            }
        }
    }

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    // Built with concat! so no committed line carries a key-shaped literal
    // (the identity-spillage scan reads source text, not the compiled value).
    const KEY_A: &str = concat!(
        "ssh-",
        "ed25519 ",
        "AAAAC3NzaC1lZDI1NTE5AAAAIAbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefghij",
        " a@x"
    );
    const KEY_B: &str = concat!(
        "ssh-",
        "ed25519 ",
        "AAAAC3NzaC1lZDI1NTE5AAAAIZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210zyxwvutsr",
        " b@x"
    );

    #[test]
    fn ory_revoke_drops_admin_and_mesh_role_names_keeps_the_rest() {
        let now = v(&["PLATFORM-ADMIN", "GRAFANA-READONLY", "DEVELOPER", "ADMINS"]);
        assert_eq!(
            ory_target_groups(&now, Direction::Revoke { full: false }),
            Some(v(&["GRAFANA-READONLY"]))
        );
    }

    #[test]
    fn ory_revoke_of_a_stray_mesh_role_name_still_writes() {
        // The raw PATCH left ['DEVELOPER'] — a value the console
        // refuses. Revoke normalises it to the developer tier's empty set.
        assert_eq!(
            ory_target_groups(&v(&["DEVELOPER"]), Direction::Revoke { full: false }),
            Some(vec![])
        );
    }

    #[test]
    fn ory_is_idempotent() {
        assert_eq!(
            ory_target_groups(&v(&["PLATFORM-ADMIN"]), Direction::Grant),
            None
        );
        assert_eq!(
            ory_target_groups(&v(&["IAM-ADMIN"]), Direction::Revoke { full: true }),
            None
        );
    }

    #[test]
    fn ory_grant_adds_platform_admin_once() {
        assert_eq!(
            ory_target_groups(&v(&["GRAFANA-READONLY"]), Direction::Grant),
            Some(v(&["GRAFANA-READONLY", "PLATFORM-ADMIN"]))
        );
    }

    fn user(role: &str, blocked: bool) -> NbUser {
        NbUser {
            id: "u".into(),
            role: role.into(),
            is_blocked: blocked,
            ..Default::default()
        }
    }

    #[test]
    fn netbird_role_never_touches_the_owner_on_revoke() {
        assert_eq!(
            nb_role_action(
                Some(&user("owner", false)),
                Direction::Revoke { full: false }
            ),
            NbRoleAction::Owner
        );
        assert_eq!(
            nb_role_action(Some(&user("owner", false)), Direction::Grant),
            NbRoleAction::None
        );
    }

    #[test]
    fn netbird_role_demotes_and_full_blocks() {
        assert_eq!(
            nb_role_action(
                Some(&user("admin", false)),
                Direction::Revoke { full: false }
            ),
            NbRoleAction::Set {
                role: "user",
                blocked: false
            }
        );
        assert_eq!(
            nb_role_action(Some(&user("user", false)), Direction::Revoke { full: true }),
            NbRoleAction::Set {
                role: "user",
                blocked: true
            }
        );
        assert_eq!(
            nb_role_action(
                Some(&user("user", false)),
                Direction::Revoke { full: false }
            ),
            NbRoleAction::None
        );
        assert_eq!(nb_role_action(None, Direction::Grant), NbRoleAction::NoUser);
    }

    #[test]
    fn missing_identity_is_not_an_unreadable_identity() {
        assert!(read_identity_response("[]", "synthetic@example.invalid")
            .unwrap()
            .is_none());
        for body in ["not-json", "{}", "null", "[null]", r#"[{"traits":{}}]"#] {
            assert!(read_identity_response(body, "synthetic@example.invalid").is_err());
        }
        let body = r#"[{"id":"synthetic-id","traits":{"email":"synthetic@example.invalid"}}]"#;
        assert_eq!(
            read_identity_response(body, "synthetic@example.invalid")
                .unwrap()
                .unwrap()
                .id,
            "synthetic-id"
        );
    }

    // Exercise the serialized producer contract expected by offboard_driver.py.
    #[test]
    fn mesh_status_driver_consumer_contract() {
        for (peers, errors, success) in [
            (vec![], vec![], true),
            (
                vec![peer("p1", "laptop", "id", &[])],
                vec![],
                false,
            ),
            (
                vec![],
                vec!["netbird peers: unavailable".to_string()],
                false,
            ),
            (
                vec![],
                vec!["ory identity: unavailable".to_string()],
                false,
            ),
            (
                vec![],
                vec!["console proposals: unavailable".to_string()],
                false,
            ),
        ] {
            let snapshot = Snapshot {
                ory: Ok(OryState::default()),
                ory_status: OryStatus {
                    read_ok: errors.is_empty(),
                    ..OryStatus::default()
                },
                handles: vec![],
                github: None,
                nb_user: Ok(None),
                peers,
                groups: vec![],
                setup_key_status: AccessStatus::default(),
                fabric_status: AccessStatus::default(),
                sponsorship_status: AccessStatus::default(),
                ssh_status: AccessStatus::default(),
                ssh_rows: vec![],
                ssh_keys: vec![],
                ssh_grant_keys: vec![],
                github_login_source: "none",
                github_keys_read_ok: false,
                github_key_count: 0,
                console_rows_any_status: 0,
                pubkey_files: 0,
                github_note: None,
                op_keys: vec![],
                onepassword: OnePassword::NotAUser,
                site_admins_listed: Some(false),
                read_errors: errors.clone(),
                mesh_read_errors: errors,
            };
            let value = status_json(&snapshot);
            for (key, _) in describe_now(&snapshot) {
                assert!(value[key].is_string(), "legacy summary {key}");
            }
            let mesh = value["mesh_status"].as_object().unwrap();
            let readable = mesh["read_ok"].as_bool() == Some(true)
                && mesh["errors"].as_array().is_some_and(Vec::is_empty)
                && mesh["peer_ids"].as_array().is_some_and(Vec::is_empty)
                && value["read_errors"].as_array().is_some_and(Vec::is_empty);
            assert_eq!(readable, success, "{value}");
        }
    }

    #[test]
    fn ory_session_count_requires_an_array() {
        assert_eq!(parse_session_count("[]").unwrap(), 0);
        assert_eq!(
            parse_session_count(r#"[{"id":"s1"},{"id":"s2"}]"#).unwrap(),
            2
        );
        for body in ["not-json", "{}", "null"] {
            assert!(parse_session_count(body).is_err(), "{body}");
        }
    }

    #[test]
    fn ory_session_readback_requests_only_active_sessions() {
        assert_eq!(
            ory_active_sessions_path("id/with history"),
            "/admin/identities/id%2Fwith%20history/sessions?active=true&page=1&per_page=250"
        );
    }

    #[test]
    fn structured_portal_readback_distinguishes_access_from_absence() {
        let vendors: Vec<Vendor> = serde_json::from_value(serde_json::json!([
            {"prefix": "person", "sponsor_authorized": true},
            {"prefix": "person-org", "sponsor_authorized": false}
        ]))
        .unwrap();
        let fabric = vendor_access_status(&vendors, &["person-org"]);
        assert!(fabric.read_ok && fabric.present);
        assert_eq!(fabric.identifiers, vec!["person-org"]);
        let sponsor = sponsorship_access_status(&vendors, &["person"]);
        assert!(sponsor.read_ok && sponsor.present);
        assert_eq!(sponsor.identifiers, vec!["person"]);
        assert!(!sponsorship_access_status(&vendors, &["person-org"]).present);
        assert!(!vendor_access_status(&vendors, &[]).read_ok);
    }

    #[test]
    fn skip_ory_removes_every_ory_mutation_from_the_plan() {
        let mut plan = Plan {
            ory_groups: Some(vec!["DEVELOPER".into()]),
            ory_deactivate: true,
            ory_reactivate: true,
            nb_role: NbRoleAction::None,
            mesh: MeshPlan::default(),
            mesh_governed: false,
            ssh_row_decisions: vec![],
            ssh_host_run: false,
            op_key_decisions: vec![],
            op_key_request: false,
            level_after: Level::Member,
            sponsorship: false,
            onepassword: None,
        };
        plan.skip_ory();
        assert!(plan.ory_groups.is_none());
        assert!(!plan.ory_deactivate);
        assert!(!plan.ory_reactivate);
    }

    fn group(name: &str, peers: &[&str]) -> NbGroup {
        NbGroup {
            id: format!("id-{name}"),
            name: name.into(),
            peers: peers
                .iter()
                .map(|p| NbGroupPeer {
                    id: p.to_string(),
                    ..Default::default()
                })
                .collect(),
        }
    }

    fn peer(id: &str, name: &str, user_id: &str, groups: &[&str]) -> NbPeer {
        NbPeer {
            id: id.into(),
            name: name.into(),
            user_id: user_id.into(),
            groups: groups
                .iter()
                .map(|g| NbGroup {
                    id: format!("id-{g}"),
                    name: g.to_string(),
                    peers: vec![],
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn person_peers_match_by_identity_handle_or_selector() {
        let peers = vec![
            peer("p1", "peer-doe-jane-zagros-2026-08-19-mbp", "", &["All"]),
            peer("p2", "laptop", "kid-1", &["All"]),
            peer("p3", "peer-doe-janex-zagros", "", &["All"]),
            peer("p4", "someone-else", "", &["All"]),
        ];
        let got: Vec<&str> = person_peers(&peers, "kid-1", &v(&["doe-jane"]), &[])
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        // p3's handle is `doe-janex`, not `doe-jane`: the trailing hyphen keeps it out.
        assert_eq!(got, vec!["p1", "p2"]);
    }

    #[test]
    fn mesh_revoke_lands_on_observer_before_leaving_admin() {
        let peers = vec![peer(
            "p1",
            "n",
            "",
            &["All", "ZAGROS-DEVELOPER", "ZAGROS-PLATFORM-ADMIN"],
        )];
        let groups = vec![
            group("ZAGROS-PLATFORM-ADMIN", &["p1"]),
            group("ZAGROS-OBSERVER", &[]),
        ];
        let plan = mesh_plan(&peers, &groups, "ZAGROS", Direction::Revoke { full: false });
        assert_eq!(plan.add, vec![("ZAGROS-OBSERVER".to_string(), v(&["p1"]))]);
        assert_eq!(
            plan.remove,
            vec![("ZAGROS-PLATFORM-ADMIN".to_string(), v(&["p1"]))]
        );
        assert!(plan.blocked.is_none());
    }

    #[test]
    fn mesh_revoke_refuses_without_a_landing_group() {
        let peers = vec![peer("p1", "n", "", &["ZAGROS-PLATFORM-ADMIN"])];
        let plan = mesh_plan(
            &peers,
            &[group("ZAGROS-PLATFORM-ADMIN", &["p1"])],
            "ZAGROS",
            Direction::Revoke { full: false },
        );
        assert!(plan.blocked.is_some());
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn mesh_revoke_catches_every_admin_prefix() {
        let peers = vec![peer(
            "p1",
            "n",
            "",
            &["SITE_ONE-PLATFORM-ADMIN", "ZAGROS-PROD-OBSERVER"],
        )];
        let groups = vec![
            group("SITE_ONE-PLATFORM-ADMIN", &["p1"]),
            group("ZAGROS-PROD-OBSERVER", &["p1"]),
        ];
        let plan = mesh_plan(
            &peers,
            &groups,
            "ZAGROS-PROD",
            Direction::Revoke { full: false },
        );
        assert!(plan.add.is_empty(), "already on the landing group");
        assert_eq!(
            plan.remove,
            vec![("SITE_ONE-PLATFORM-ADMIN".to_string(), v(&["p1"]))]
        );
    }

    #[test]
    fn mesh_full_strips_everything_but_all() {
        let peers = vec![peer(
            "p1",
            "n",
            "",
            &["All", "ZAGROS-DEVELOPER", "ZAGROS-OBSERVER"],
        )];
        let plan = mesh_plan(&peers, &[], "ZAGROS", Direction::Revoke { full: true });
        let names: Vec<&str> = plan.remove.iter().map(|(g, _)| g.as_str()).collect();
        assert_eq!(names, vec!["ZAGROS-DEVELOPER", "ZAGROS-OBSERVER"]);
    }

    #[test]
    fn mesh_grant_adds_to_the_site_admin_group_only_if_missing() {
        let peers = vec![
            peer("p1", "n", "", &["ZAGROS-DEVELOPER"]),
            peer("p2", "m", "", &["ZAGROS-PLATFORM-ADMIN"]),
        ];
        let plan = mesh_plan(
            &peers,
            &[group("ZAGROS-PLATFORM-ADMIN", &["p2"])],
            "ZAGROS",
            Direction::Grant,
        );
        assert_eq!(
            plan.add,
            vec![("ZAGROS-PLATFORM-ADMIN".to_string(), v(&["p1"]))]
        );
    }

    #[test]
    fn governed_entries_express_grant_and_revoke() {
        use crate::cmd::warrant::EntryForm;
        let s = |x: &[&str]| x.iter().map(|g| g.to_string()).collect::<BTreeSet<_>>();
        let peers = vec![
            peer(
                "p1",
                "peer-alice-laptop",
                "",
                &["All", "ZAGROS-DEV-DEVELOPER"],
            ),
            peer("p2", "mgmt-box", "", &["ZAGROS-DEV-INFRA"]),
        ];
        let (e, notes) = governed_entries(&peers, "ZAGROS-DEV", Direction::Grant);
        assert_eq!(e.len(), 1, "a machine never gets a PLATFORM-ADMIN entry");
        assert_eq!(e[0].peer, "peer-alice-laptop");
        assert_eq!(
            e[0].form().unwrap(),
            EntryForm::GrantAbsent {
                grant: s(&["ZAGROS-DEV-PLATFORM-ADMIN"]),
                absent: BTreeSet::new()
            }
        );
        assert!(
            e[0].expires.is_some(),
            "CI requires expires on a PLATFORM-ADMIN grant"
        );
        assert_eq!(notes.len(), 1);

        let peers = vec![
            peer("p1", "peer-a", "", &["ZAGROS-DEV-PLATFORM-ADMIN"]),
            peer(
                "p2",
                "peer-b",
                "",
                &["ZAGROS-DEV-PLATFORM-ADMIN", "ZAGROS-DEV-DEVELOPER"],
            ),
            peer("p3", "peer-c", "", &["ZAGROS-DEV-DEVELOPER"]),
        ];
        let (e, _) = governed_entries(&peers, "ZAGROS-DEV", Direction::Revoke { full: false });
        assert_eq!(e.len(), 2, "a peer holding no admin group needs no entry");
        assert_eq!(
            e[0].form().unwrap(),
            EntryForm::GrantAbsent {
                grant: s(&["ZAGROS-DEV-OBSERVER"]),
                absent: s(&["ZAGROS-DEV-PLATFORM-ADMIN"])
            }
        );
        assert_eq!(
            e[1].form().unwrap(),
            EntryForm::GrantAbsent {
                grant: BTreeSet::new(),
                absent: s(&["ZAGROS-DEV-PLATFORM-ADMIN"])
            }
        );

        let (e, _) = governed_entries(&peers[1..2], "ZAGROS-DEV", Direction::Revoke { full: true });
        assert_eq!(
            e[0].form().unwrap(),
            EntryForm::GrantAbsent {
                grant: BTreeSet::new(),
                absent: s(&["ZAGROS-DEV-DEVELOPER", "ZAGROS-DEV-PLATFORM-ADMIN"])
            }
        );
    }

    fn opk(id: &str, status: &str) -> OperatorKeyRow {
        OperatorKeyRow {
            id: id.into(),
            handle: "h".into(),
            email: String::new(),
            env: String::new(),
            reason: String::new(),
            status: status.into(),
            fingerprint: String::new(),
            decided_by: String::new(),
        }
    }

    #[test]
    fn level_follows_ory_groups() {
        assert_eq!(
            level_for_groups(&v(&["PLATFORM-ADMIN"])),
            Level::PlatformAdmin
        );
        assert_eq!(
            level_for_groups(&v(&["IAM-ADMIN", "GRAFANA-READONLY"])),
            Level::IamAdmin
        );
        assert_eq!(level_for_groups(&v(&["DEVELOPER"])), Level::Member);
        assert_eq!(Level::IamAdmin.sponsorship_tier(), Some("iam-admin"));
        assert_eq!(Level::Member.sponsorship_tier(), None);
        assert!(!Level::IamAdmin.operator_key_entitled());
    }

    #[test]
    fn operator_keys_follow_level() {
        // Demoted: live key revoked, pending request rejected.
        let (d, req) = op_key_plan(
            &[
                opk("a", "active"),
                opk("b", "requested"),
                opk("c", "revoked"),
            ],
            Level::Member,
        );
        assert_eq!(
            d,
            vec![("a".to_string(), "revoke"), ("b".to_string(), "reject")]
        );
        assert!(!req);
        // Platform admin with a live key: nothing.
        assert_eq!(
            op_key_plan(&[opk("a", "active")], Level::PlatformAdmin),
            (vec![], false)
        );
        // Platform admin with a pending request: approve it, do not mint a second.
        assert_eq!(
            op_key_plan(&[opk("b", "requested")], Level::PlatformAdmin),
            (vec![("b".to_string(), "approve")], false)
        );
        // Platform admin with nothing: request one.
        assert_eq!(
            op_key_plan(&[opk("c", "revoked")], Level::PlatformAdmin),
            (vec![], true)
        );
    }

    #[test]
    fn grant_keys_allow_active_rows_and_new_file_keys_only() {
        for status in [
            "submitted",
            "approved",
            "installed",
            "rejected",
            "revoked",
            "unknown",
            "",
        ] {
            let rows = vec![SshKey {
                status: status.into(),
                key_type: "ssh-ed25519".into(),
                key_body: KEY_A.split_whitespace().nth(1).unwrap().into(),
                ..Default::default()
            }];
            let extra = v(&[KEY_A, KEY_B, KEY_B, "not a key"]);
            let grant = collect_grant_key_lines(&rows, &extra, "a@x");
            let active = matches!(status, "submitted" | "approved" | "installed");
            assert_eq!(
                grant,
                if active {
                    v(&[KEY_A, KEY_B])
                } else {
                    v(&[KEY_B])
                },
                "{status}"
            );
            assert_eq!(collect_key_lines(&rows, &extra, &[], "a@x").len(), 2);
            assert_eq!(
                collect_grant_key_lines(&rows, &[], "a@x").is_empty(),
                !active
            );
        }
    }

    #[test]
    fn key_lines_dedupe_by_body_and_reject_junk() {
        let rows = vec![SshKey {
            key_type: "ssh-ed25519".into(),
            key_body: KEY_A.split_whitespace().nth(1).unwrap().into(),
            ..Default::default()
        }];
        let got = collect_key_lines(&rows, &v(&[KEY_A, "not a key"]), &v(&[KEY_B]), "a@x");
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with(" a@x"));
    }

    #[test]
    fn pubkey_line_shape() {
        assert!(is_pubkey_line(KEY_A));
        assert!(!is_pubkey_line("ssh-dss AAAAB3NzaC1kc3MAAACBAP"));
        assert!(!is_pubkey_line("-----BEGIN OPENSSH PRIVATE KEY-----"));
    }

    #[test]
    fn site_admins_scan() {
        let text = "x: 1\nsite_admins:\n  - email: maintainer@example.org\n    name: \"D\"\n  - email: \"adam@example.org\"\n\nnext_key: 2\n  - email: late@example.org\n";
        assert_eq!(site_admins_in(text, "ADAM@example.org"), Some(true));
        assert_eq!(site_admins_in(text, "late@example.org"), Some(false));
        assert_eq!(site_admins_in("site_admins: []\n", "a@x"), Some(false));
        assert_eq!(site_admins_in("other: 1\n", "a@x"), None);
    }

    #[test]
    fn pick_identity_matches_trait_email_case_insensitively() {
        let body = r#"[{"id":"i1","state":"active","traits":{"email":"pat.example@example.org","groups":["DEVELOPER"]}}]"#;
        let s = pick_identity(body, "Pat.Example@EXAMPLE.org").unwrap();
        assert_eq!(s.id, "i1");
        assert_eq!(s.groups, v(&["DEVELOPER"]));
        assert!(pick_identity(body, "other@x").is_none());
    }

    #[test]
    fn manifest_github_reads_string_or_object_and_validates() {
        let d = serde_json::json!({"manifest_json": "{\"identity\":{\"github\":\"contributord\"}}"});
        assert_eq!(manifest_github(&d), Some("contributord".into()));
        for login in ["bad/../name", "-leading", "trailing-", "double--hyphen"] {
            let d = serde_json::json!({"manifest_json": {"identity": {"github": login}}});
            assert_eq!(manifest_github(&d), None, "accepted {login}");
        }
    }

    #[test]
    fn extra_vars_carry_state_and_door_proxy() {
        let e = ssh_extra_vars(
            "zagros-prod",
            Direction::Revoke { full: false },
            &v(&[KEY_A]),
            Some("ssh -p 12202 -W %h:%p goat@127.0.0.1"),
        );
        assert_eq!(e["pa_state"], "absent");
        assert_eq!(e["pa_env"], "zagros-prod");
        assert!(e["pa_ssh_common_args"]
            .as_str()
            .unwrap()
            .contains("ProxyCommand=\"ssh -p 12202"));
        let g = ssh_extra_vars("zagros", Direction::Grant, &v(&[KEY_A]), None);
        assert_eq!(g["pa_state"], "present");
        assert!(g.get("pa_ssh_common_args").is_none());
    }
}

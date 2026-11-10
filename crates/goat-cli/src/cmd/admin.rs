// `goat admin <subcmd>` — operator-everything-tool surface (Phase F).
//
// Today's surface (Phase F.1): admin vendor create/delete. Pure-
// portal POST/DELETE on /api/orgs, gated by the operator's
// PORTAL_OPERATOR_API_KEY scope server-side. The CLI just submits;
// 401/403 surfaces if the token isn't admin.
//
// Phase F.2 adds `goat admin onboard <handle> [--revoke]` — the
// ansible-playbook wrapper. Kept out of this PR so the wrapper's
// child-process + streaming + SIGINT machinery is its own focused
// review.

#[path = "admin_affiliation.rs"]
mod affiliation;

use clap::Subcommand;
use panoscope_core::peername;
use panoscope_core::portal::{NewInvite, NewVendor, OnboardedOrg, PortalError, Proposal};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Subcommand, Debug)]
pub enum AdminCmd {
    /// Org lifecycle (admin-gated).
    ///
    /// Named `org` since the ADR 1052 vendor→org rename; `vendor` stays a
    /// hidden alias so existing scripts keep working.
    #[command(name = "org", alias = "vendor")]
    Vendor {
        #[command(subcommand)]
        op: VendorCmd,
    },
    /// Sponsor a candidate: mint a one-time onboarding invite (ADR 1062).
    ///
    /// Every field is OPTIONAL (minimal-burden): mint a bare link and let the
    /// enrollee fill their own details. Read a spec from a YAML file with
    /// `--file`, tweak it in your `$EDITOR` before firing with `--edit`, or pass
    /// fields as flags. Prints the one-time nomination link to hand the
    /// candidate. Escalated admin tiers (`iam-admin`/`platform-admin`) may be
    /// offered here — sponsor-only — and are flagged for scrutiny.
    Invite {
        /// Read the invite spec from a YAML file. Recognized keys (all optional):
        /// `env`, `tier`, `affiliation`, `site`, `machine_quota`, `ttl_hours`,
        /// `note`, `collab`. Flags below override matching file keys.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Open the spec in `$EDITOR` before firing (like `git commit`). Starts
        /// from `--file` if given, else a commented template. On empty/unchanged
        /// save it still fires the (possibly bare) invite.
        #[arg(long)]
        edit: bool,
        /// Target goatnet (e.g. `zagros`).
        #[arg(long)]
        env: Option<String>,
        /// Access tier: `developer` | `browse` | `iam-admin` | `platform-admin`.
        /// The two admin tiers are ESCALATED (scrutinized). Omit to let the
        /// enrollee choose (non-escalated only).
        #[arg(long)]
        tier: Option<String>,
        /// Affiliation kind: `vendor` | `organic` | `partner` | `civil`.
        #[arg(long)]
        affiliation: Option<String>,
        /// Site (defaults to the environment).
        #[arg(long)]
        site: Option<String>,
        /// Machine-enrollment quota the person may use later.
        #[arg(long)]
        machine_quota: Option<i64>,
        /// One-time link lifetime in hours (default 72, max 720).
        #[arg(long)]
        ttl_hours: Option<i64>,
        /// Free-text note for your own reference.
        #[arg(long)]
        note: Option<String>,
        /// Include the person in the collaboration environment (Outline / file
        /// sharing / Zulip — cloud SSO, separate from the mesh). Defaults to
        /// ON, matching the console's invite page; pass `--collab false` to
        /// exclude them.
        #[arg(long)]
        collab: Option<bool>,
        /// Email the one-time nomination link to this candidate right after
        /// minting (via the console SES mailer) — so the automated mint is
        /// self-contained. Omit to just print the link for manual hand-off.
        #[arg(long)]
        email: Option<String>,
        /// Emit the raw JSON mint response instead of the friendly summary.
        #[arg(long)]
        json: bool,
    },
    /// Review + act on the onboarding queue from the CLI (the admin's stage-3
    /// gate — mirrors /admin/onboard-queue.html): list proposals, approve (→ the
    /// runner mints, or run the emitted command), or reject.
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// Host-shell SSH keys: submit one for somebody, review the queue, decide.
    /// The CLI face of the console's /ssh-key + /onboarding/ssh-keys pages —
    /// same store, same gate, same audit (docs/design/ssh-key-self-service.md).
    ///
    /// SUBMITTING GRANTS NOTHING. An admin approves, then the installer places
    /// the key on the hosts that person's ROLE may reach. `approved` is a
    /// decision; `installed` is a fact about a host.
    #[command(name = "ssh-key")]
    SshKey {
        #[command(subcommand)]
        op: SshKeyAction,
    },
    /// Manage existing onboarding invites: list the queue or revoke an open one.
    /// (The singular `invite` command mints a new one.)
    #[command(name = "invites")]
    Invites {
        #[command(subcommand)]
        op: InviteAction,
    },
    /// Change an EXISTING person's login access — set their Ory groups, or flip
    /// their activation state — through the console's machine API. The CLI face
    /// of the web UI's user admin: the server runs the same last-admin guard,
    /// operator audit, and role-change email, so a `goat admin role` change and
    /// a console one are identical. For a NEW person use `invite`; this never
    /// mints an identity. See docs/design/onboarding-notifications.md.
    Role {
        #[command(subcommand)]
        op: AdminRoleCmd,
    },
    /// Operator API keys — the per-person `op_` key that makes a Portal admin
    /// action attributable to a named person rather than to a shared
    /// credential.
    ///
    /// Issuing one used to mean editing a gitignored envrc and redeploying the
    /// portal. It no longer does: approving writes the key file the portal
    /// reads, and the portal re-reads that directory while running, so the key
    /// is live within seconds. Revoking removes the file, the same way round.
    ///
    /// Same engine as the console pages at `/operator-key` and
    /// `/onboarding/operator-keys` — docs/design/operator-action-audit.md.
    #[command(name = "operator-key")]
    OperatorKey {
        #[command(subcommand)]
        op: AdminOperatorKeyCmd,
    },
    /// REVOKE an operator / org by handle — the live offboarding path (ADR 1031).
    ///
    /// The ENROLL direction is RETIRED (July 2026, ADR 1062 cutover) and refuses.
    /// It minted every surface from a hand-authored manifest and wrote an
    /// out-of-band bundle carrying the person's private key; the live path never
    /// puts a key in an operator's hands. Bring people on with
    /// `goat admin invite --env <goatnet>` and the three-stage candidate flow,
    /// then reconcile the roster with ops/scripts/sync-onboarded-manifests.py.
    /// The refusal names the override if you genuinely need it.
    ///
    /// Two implementations underneath, both fenced for enroll and both live for
    /// revoke: a native fast path (portal + NetBird API + Ory over SSH, seconds)
    /// which is the DEFAULT when the manifest is native-serviceable, and
    /// `ops/scripts/ansible-preflight.sh operator-onboard.yml` otherwise.
    /// `--native` / `--ansible` force one. Streams child output line-by-line;
    /// SIGINT propagates via the process group (tokio::process default).
    Onboard {
        /// Handle (kebab-case slug). Manifest must exist at
        /// `ops/operators/<handle>.yml` (flat / legacy) or, when
        /// `--env <env>` is passed, `ops/operators/<env>/<handle>.yml`
        /// in the discovered repo root. Omit when using `--org`.
        handle: Option<String>,
        /// ORG-scope revoke (cascade). Instead of one handle, revoke EVERY
        /// active member of the affiliation org `<id>` (the `affiliation.id`,
        /// e.g. `acme`) — persons + systems, the whole unit. Resolves the
        /// members from the declared registry (`docs/operators/roster.json`,
        /// the affiliation-grouped roster) and runs the per-handle revoke for
        /// each, after printing the blast radius and asking to confirm
        /// (skip the prompt with `--yes`). Requires `--revoke`; mutually
        /// exclusive with a positional handle. Identity grammar B.4 / ADR 1031
        /// org-cascade (`docs/design/credential-revocation-and-offboarding.md`).
        #[arg(long, conflicts_with = "handle")]
        org: Option<String>,
        /// Skip the interactive confirmation prompt for `--org` cascades.
        /// For scripted/non-interactive use; the blast radius is still printed.
        #[arg(long)]
        yes: bool,
        /// Environment / goatnet the manifest belongs to (e.g. `zagros`,
        /// `zagros-prod`). Selects the per-env manifest subfolder
        /// `ops/operators/<env>/<handle>.yml` and is passed to the
        /// playbook as `-e operator_env=<env>` so the per-site maps
        /// (NetBird mgmt URL, Ory Kratos admin host, Zenoh portal)
        /// resolve to that goatnet. Required on every live goatnet: with no
        /// env the native legs resolve to no config and defer.
        #[arg(long)]
        env: Option<String>,
        /// Revoke instead of enroll. Cuts every provisioned surface — NetBird
        /// peer(s), Ory login, the Zenoh slot (guarded when teammates share it) —
        /// and flips the manifest to `status: revoked`. Reversible by default
        /// (Ory deactivate, registry deprecate): a returning principal is a
        /// re-onboard, not lost. Takes the native fast path (~seconds) when the
        /// env/manifest is native-serviceable; `--ansible` forces the playbook.
        #[arg(long)]
        revoke: bool,
        /// With `--revoke`: terminally DELETE the Ory identity instead of the
        /// default reversible deactivate (state=inactive). Irreversible — use
        /// for a hard offboard where the login must not be recoverable. The
        /// Zenoh slot + NetBird peer are cut either way. Mirrors the playbook's
        /// `-e revoke_purge=true` (Phase R2.3).
        #[arg(long, requires = "revoke")]
        purge: bool,
        /// With `--revoke`: batch-revoke every handle listed in FILE (one
        /// kebab-case handle per line; `#` comments and blanks ignored). Each is
        /// revoked natively and independently — a per-handle failure is reported,
        /// never aborts the batch. The generalized cohort-revoke path (the sweep
        /// tool). Mutually exclusive with a positional handle and `--org`.
        #[arg(long, requires = "revoke", conflicts_with_all = ["handle", "org"])]
        from: Option<PathBuf>,
        /// Re-mint a partial bundle in place: run the (hardened) revoke leg then
        /// the enroll leg as one operation, so every surface is freshly minted
        /// and rendered into one complete bundle. The all-or-nothing render means
        /// a half-provisioned bundle can't be completed by a plain re-run — this
        /// is the supported repair. The revoke leg frees the surfaces but leaves
        /// the manifest `status: active` (via `-e onboard_repair=true`) so the
        /// enroll leg re-mints onto it. See P1.5 in
        /// docs/design/onboard-rotate-revoke-streamlining.md. Mutually exclusive
        /// with --revoke.
        #[arg(long, conflicts_with = "revoke")]
        repair: bool,
        /// Revoke every live HANDSET key this person holds on the goatnet and
        /// mint a fresh one (a lost or replaced phone, ADR 1175). Touches
        /// nothing else: the device key, the Ory login and the Zenoh slot stay
        /// as they are, and no bundle is rendered. Native only. Refuses when the
        /// manifest says `netbird.handset_limit: 0`.
        #[arg(long, conflicts_with_all = ["revoke", "repair", "ansible", "from", "org"])]
        reissue_handset_key: bool,
        /// Mint a fresh DEVICE setup key for an already-onboarded person whose
        /// key has expired or been used up, and revoke any device key of theirs
        /// that is still live, so the person holds exactly one. Touches nothing
        /// else: enrolled peers, the handset key, the Ory login and the Zenoh
        /// slot stay as they are, and no bundle is rendered. Groups and the key
        /// name come from the manifest; with no manifest on this machine they
        /// come from the person's most recent device key on the goatnet (what
        /// they were actually granted), and the command says which it used.
        /// Records an operator-action audit event naming the operator, the
        /// person and the key. Native only. Console: "Reissue device key".
        #[arg(long, conflicts_with_all = ["revoke", "repair", "ansible", "from", "org", "reissue_handset_key"])]
        reissue_device_key: bool,
        /// With `--reissue-device-key`: how many enrollments the fresh key
        /// allows (1-100). Default: the manifest's `netbird.usage_limit`, else 5.
        #[arg(long, requires = "reissue_device_key", value_parser = clap::value_parser!(u32).range(1..=100))]
        uses: Option<u32>,
        /// With either reissue flag: who asked for the key (the console user's
        /// email when the onboard-runner runs it on their behalf). Recorded on
        /// the audit event beside the operator who ran the command.
        #[arg(long)]
        requested_by: Option<String>,
        /// Force the native fast path (the DEFAULT for a plain enroll — provisions
        /// all three identity surfaces directly, no Ansible, seconds not minutes;
        /// mirrors `goat admin rotate`). Mints the Zenoh wire-layer (`POST
        /// /api/orgs` → slot/cert/key), the NetBird mesh setup key, AND the Ory
        /// Kratos login identity (over an SSH shell-out to the site Thor's Kratos
        /// admin API) — P2.1/P2.2/P2.4. Native config covers zagros/zagros-prod/zagros-dev; other
        /// envs defer that leg to Ansible with a clear note. The Ory leg runs last —
        /// a failure there leaves the Zenoh/NetBird bundle usable, marked `partial`,
        /// never silently dropped. The Ory operator-IdP surface stays on Ansible.
        /// As of goat-cli 0.4.13 the native path is the DEFAULT for BOTH enroll and
        /// revoke; this flag now only FORCES it (erroring if the env/manifest isn't
        /// native-serviceable) — pass `--ansible` for the legacy playbook. Not valid
        /// with --repair (which sequences a revoke+enroll through the playbook). See
        /// docs/design/onboard-rotate-revoke-streamlining.md.
        #[arg(long, conflicts_with_all = ["repair", "ansible"])]
        native: bool,
        /// Onboard the Zenoh slot WITHOUT minting its fabric credential
        /// (mint-on-collect, docs/design/fabric-credential-collection.md). The
        /// slot, the person linkage and the org row are created; no api_key and
        /// no cert are. The person mints their own from the console's
        /// fabric-credential page once they can sign in — which is the only way
        /// the key reaches them rather than passing through an operator and an
        /// email, since the plaintext can never be shown a second time.
        ///
        /// OFF BY DEFAULT, deliberately: a goatnet whose console does not yet
        /// serve the collection page would leave enrollees with no credential
        /// and no way to get one. Turn it on per goatnet once that page is
        /// deployed. Native path only; the Ansible path is unaffected.
        #[arg(long, conflicts_with = "ansible")]
        defer_credential: bool,
        /// Force the legacy Ansible playbook (`operator-onboard.yml`) instead of the
        /// native fast path — for enroll OR revoke. Use it when you need a surface
        /// native doesn't do (Ory operator-IdP, an env with no native config,
        /// observer/prod Zenoh flavours) or to fall back deliberately. Repair
        /// always uses the playbook regardless.
        #[arg(long, conflicts_with = "native")]
        ansible: bool,
        /// Override repo root (defaults to searching upward from cwd
        /// for a `.git` directory).
        #[arg(long)]
        repo_root: Option<PathBuf>,
        /// With `--revoke`: record what happens to this person's CLOUD
        /// COLLABORATION account (Outline / files / chat behind Dex+Kanidm) —
        /// `keep` or `revoke`. That plane is a separate identity store the mesh
        /// revoke deliberately does not cut, so the choice is a decision this
        /// command RECORDS (receipt line + audit event), not a cut it performs:
        /// goat-cli holds no collab credentials, and minting a second client +
        /// credential path for them is exactly what the provisioner seam avoids.
        /// `--collab revoke` prints the one command that does cut it
        /// (`onboard-runner -collab revoke`). Omit the flag and the receipt says
        /// `undecided` — honest, and the reconcile will keep surfacing the
        /// person until someone chooses. Native-path only. See F-274 +
        /// docs/design/credential-revocation-and-offboarding.md.
        #[arg(long, requires = "revoke", value_name = "keep|revoke")]
        collab: Option<CollabDecision>,
        /// Don't actually run ansible-playbook; print the command
        /// + exit 0. Useful for previewing in CI / scripts.
        #[arg(long)]
        dry_run: bool,
    },
    /// Re-issue (rotate) a peer's cert in place — same anchor, fresh
    /// leaf. The "rotate" sibling of `--revoke`: use it for expiry
    /// renewal, post-compromise rotation, or retrofitting a new URI-SAN
    /// scheme onto an already-issued cert, all WITHOUT changing the
    /// peer's anchor / alias / write namespace / ACL. Pure-portal POST
    /// /api/orgs/{anchor}/rotate, operator-gated server-side. The
    /// selector is an anchor (32-hex slot_id) or its slot alias; the
    /// alias is resolved to the anchor before the call. Prints the fresh
    /// one-time bundle URL — fetch it, swap the cert+key on the peer, and
    /// restart it. See docs/design/credential-rotation.md.
    Rotate {
        /// Anchor (slot_id) or its slot alias to re-issue. Omit when using
        /// `--org`.
        selector: Option<String>,
        /// ORG-scope rotation (cascade). Instead of one selector, re-issue the
        /// leaf cert of EVERY active member of the affiliation org `<id>` (the
        /// `affiliation.id`) in place — the rotate sibling of the `onboard
        /// --org --revoke` cascade, at the same granularity. Resolves the
        /// members from the declared registry (`docs/operators/roster.json`)
        /// and resolves each to its slot anchor via the alias map; members with
        /// no resolvable anchor are reported and skipped (fail-visible, never
        /// silent). Prints the blast radius and asks to confirm (skip with
        /// `--yes`). Mutually exclusive with a positional selector. Identity
        /// grammar / ADR 1044 (`docs/design/credential-rotation.md`).
        #[arg(long, conflicts_with = "selector")]
        org: Option<String>,
        /// Skip the interactive confirmation prompt for `--org` cascades.
        /// The blast radius is still printed.
        #[arg(long)]
        yes: bool,
        /// Repo root holding `docs/operators/roster.json` for `--org`
        /// resolution. Defaults to the nearest `.git` ancestor of the cwd.
        #[arg(long)]
        repo_root: Option<PathBuf>,
        /// Don't actually POST; resolve the selector(s) + print the intended
        /// request(s) + exit 0.
        #[arg(long)]
        dry_run: bool,
        /// Reissue ONLY the portal key, leaving the mTLS cert untouched
        /// (goat#1193). The lighter, safer path for the recurring case:
        /// someone lost / mistyped / stale-pasted their portal key but their
        /// cert is fine and they are still publishing. Swaps the key in place
        /// with zero cert churn — no bundle, no download, no peer restart.
        /// Without it, `rotate` re-mints the cert too. Single-anchor only, not
        /// the `--org` cascade.
        #[arg(long, conflicts_with = "org")]
        key_only: bool,
    },
    /// Grant or revoke an identity's sponsorship authorization (ADR
    /// 1071) — whether it may mint candidate-onboarding invites and
    /// vouch at the stage-2 gate, and up to which tier. The case-by-case
    /// lever for identities that didn't get sponsorship by tier default
    /// at onboarding (iam-admin / platform-admin get it automatically).
    /// Pure-portal POST /api/orgs/{id}/sponsorship, operator-gated
    /// server-side; accepts an anchor or the human handle.
    Sponsorship {
        /// Anchor (slot_id) or human handle of the identity.
        id: String,
        /// Grant sponsorship up to this tier
        /// (browse | developer | iam-admin | platform-admin).
        #[arg(long, conflicts_with = "revoke")]
        max_tier: Option<String>,
        /// Bind the sponsor's Ory login email so they can use the
        /// id-admin-console onboarding page (sponsor-scoped: they see and
        /// vouch only their own invitees there). Optional — without it the
        /// sponsor still mints/vouches via CLI/API with their own portal key.
        #[arg(long, conflicts_with = "revoke")]
        email: Option<String>,
        /// Revoke sponsorship authorization instead of granting it.
        #[arg(long)]
        revoke: bool,
    },
    /// LAN troubleshooting toolkit for relocated goat mgmt hosts that
    /// don't appear on the WAN: `scan` finds them on the local LAN by
    /// MAC even when DHCP moved their IP; `doctor` runs a layered
    /// L0–L4 triage. Depends on NO goatnet/mesh service — works when
    /// the mesh does not. See docs/design/goat-cli.md §"goat admin lan".
    Lan {
        #[command(subcommand)]
        op: super::lan::LanCmd,
    },
    /// goat-client device lifecycle (admin-gated). Today: bundle
    /// minting via `goat admin client onboard <handle>` which reads
    /// the per-device manifest at `ops/devices/<handle>.yml` and
    /// translates it into a `bundle-create` invocation. Per-site
    /// fleet config (CA-id, mode, age-identity / piv-slot) comes
    /// from CLI flags + env vars, NOT the per-device manifest.
    Client {
        #[command(subcommand)]
        op: AdminClientCmd,
    },
    /// Mesh-peer lifecycle — the authoritative mesh-first cut. `ls`
    /// lists mesh peers; `cut` ejects one (deletes its NetBird peer, so
    /// its WireGuard tunnel drops). On a brokerless slot-pool fabric the
    /// Zenoh write-ACL is pre-baked for the whole pool, so deleting an
    /// org record does NOT stop a flooder — removing the *peer* does
    /// (a cert is inert without a mesh path). This is the one-command
    /// emergency cut: `goat health` finds the hog, `goat admin peer cut`
    /// ejects it. Needs the NetBird admin token in the environment
    /// ($<CONTEXT>_NB_API_TOKEN, e.g. ZAGROS_NB_API_TOKEN, or
    /// $GOAT_NB_API_TOKEN).
    Peer {
        #[command(subcommand)]
        op: PeerCmd,
    },
    /// Make someone a platform admin on a goatnet, or stop them being one, on
    /// every plane at once: Ory login group, NetBird dashboard role, their
    /// peers' mesh groups, host SSH keys, operator key, sponsorship, and the
    /// 1Password Platform group. `status` reads; `grant` / `revoke` plan and,
    /// with --apply, execute and re-read. Revoke demotes to developer by
    /// default; --full offboards. Canon: docs/design/platform-admin-lifecycle.md.
    #[command(name = "platform-admin")]
    PlatformAdmin {
        #[command(subcommand)]
        op: super::platform_admin::PlatformAdminCmd,
    },
    /// Membership transfer engine (ADR 1088): move entities between canonical
    /// and satellite groups (probation, quarantine, exercise sets) — audited,
    /// coverage-gated, mirror-first, idempotent; dry-run by default.
    Members {
        #[command(subcommand)]
        op: super::members::MembersCmd,
    },
    /// Backend-OPERATOR tier provisioning (admin-gated). Distinct from
    /// `onboard` (the participant tier): `operator onboard <handle>` wraps
    /// `ops/ansible/operator-tier-onboard.yml` to mint the operator's
    /// `operator-trial-peer` Zenoh leaf (writes `ops/**`), install their host
    /// SSH key, grant the Ory admin group on their existing identity, render
    /// the operator bundle, and PRINT the two blast-radius additions (portal
    /// key, age recipient — never auto-edited). The manifest must carry an
    /// `operator:` block. See docs/operations/admin-operator-guide.md.
    Operator {
        #[command(subcommand)]
        op: AdminOperatorCmd,
    },
    /// Toggle PQC on an ALREADY-ENROLLED peer at BOTH layers — the Zenoh
    /// kx-mode state file (F-259 Tier 1 kill-switch, `/var/lib/goat/pqc-mode`;
    /// new processes latch it, long-running surfaces flip on restart) and
    /// intra-mesh Rosenpass (ADR 0602) — no setup key, no re-mint. The
    /// rosenpass leg runs `netbird down` then `netbird up` with the explicit
    /// flag (`--enable-rosenpass --rosenpass-permissive` for `on`,
    /// `--enable-rosenpass=false` for `off`). Both the `down` and the
    /// explicit-`false` matter: a bare `up` no-ops on a connected peer AND
    /// persists the prior setting, so it silently changes nothing.
    /// The toggle briefly drops + re-establishes the tunnel.
    /// Permissive means `on` never breaks connectivity to not-yet-enabled
    /// peers, and PQC only actually engages between two Rosenpass-enabled peers
    /// — so a mixed fleet is safe.
    ///
    /// This is the OTHER-peer surface — admin-gated (needs the NetBird API
    /// token plus SSH). To toggle THIS host, use `goat client pqc` instead
    /// (self-service, no admin token). Here you MUST pass `--peer` or
    /// `--fleet`. With `--peer <sel>` it SSHes to a mesh peer (the one-command
    /// "PQC-ify everything hosted on a mgmt-host" — e.g. Thor's own peer
    /// PQ-secures every service riding it). With `--fleet` it prints the
    /// per-peer plan for the whole mesh (each peer holds its OWN setting, so a
    /// fleet flip is a per-peer re-up, not an instant switch; the durable path
    /// is the `rosenpass_enabled` ansible group_var + re-onboard). See
    /// docs/design/rosenpass-intra-mesh-pqc.md.
    Pqc {
        /// `on` (enable Rosenpass) or `off` (revert to classical WireGuard).
        state: PqcState,
        /// Toggle a remote mesh peer over SSH instead of this host. Selector is
        /// an exact peer id, exact mesh IP, or a case-insensitive name/hostname
        /// substring (as shown by `goat admin peer ls`). Refuses if it matches
        /// more than one peer. SSHes `<ssh-user>@<mesh-ip>` and runs the
        /// netbird command with `sudo`.
        #[arg(long, conflicts_with = "fleet")]
        peer: Option<String>,
        /// Print the per-peer plan for every mesh peer (the honest "sweep":
        /// PQC is per-peer, so this emits the command to run on each rather
        /// than fanning SSH into heterogeneous peers). Combine with `--json`
        /// for machine-readable output.
        #[arg(long)]
        fleet: bool,
        /// SSH user for `--peer` (default: the context's `mgmt_ssh_user`,
        /// written by `goat context refresh`).
        #[arg(long)]
        ssh_user: Option<String>,
        /// Override the NetBird mgmt URL for peer resolution (else env / profile).
        #[arg(long)]
        mgmt_url: Option<String>,
        /// Skip the confirmation prompt (`--peer` mutates the remote mesh client).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Resolve + print what WOULD run, change nothing, exit 0.
        #[arg(long)]
        dry_run: bool,
        /// Emit JSON instead of the human summary.
        #[arg(long)]
        json: bool,
    },
    /// Control-plane storm resilience (ADR 1081). Today: `storm brake` — the
    /// MANUAL operator brake fired when the relay-health host-net band shows a
    /// signaling storm (futility ratio ≥0.5 at ≥100 msgs/min). It flushes the
    /// signal container, tightens L2 limits (if deployed), sheds declared
    /// non-essential publishers, and records one audited operator action.
    /// Reaches the mgmt host over SSH (host+user from the goat context). NO
    /// automation / timers — auto-engage is a separately-gated follow-on. See
    /// docs/operations/storm-brake-runbook.md.
    Storm {
        #[command(subcommand)]
        op: super::storm::StormCmd,
    },
}

/// `keep` / `revoke` for `goat admin onboard --revoke --collab <…>` — the
/// operator's decision about the collaboration-environment account, carried into
/// the revoke receipt and the audit event. It is a RECORD, not a cut: see the
/// flag's doc comment and F-274.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollabDecision {
    /// The person keeps their collab account (mesh access is cut, collab is not).
    Keep,
    /// The collab account should be cut too — recorded here, executed by
    /// `onboard-runner -collab revoke -handle <handle> -yes` on the host holding
    /// the collab credentials.
    Revoke,
}

/// `on` / `off` for `goat admin pqc`.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PqcState {
    /// Enable PQC: zenoh kx-mode `pq` + Rosenpass
    /// (`--enable-rosenpass --rosenpass-permissive`).
    On,
    /// Disable PQC: zenoh kx-mode `classical` + Rosenpass off — revert the
    /// peer to classical crypto at both layers.
    Off,
}

impl PqcState {
    fn is_on(self) -> bool {
        matches!(self, PqcState::On)
    }
    fn label(self) -> &'static str {
        match self {
            PqcState::On => "on",
            PqcState::Off => "off",
        }
    }
}

/// `goat admin queue <subcmd>` — the onboarding review queue from the CLI.
#[derive(Subcommand, Debug)]
pub enum QueueAction {
    /// List proposals. Default `sponsor_approved` (the ones ready for the admin).
    Ls {
        /// Status filter: `proposed` | `sponsor_approved` | `approved` |
        /// `rejected` | `onboarded` | `failed` | `offboarded` | `onboarding` |
        /// `enrolled` | `archived` | `offboard-requested` | `offboarding` |
        /// `offboard-failed` | `all`. Default `sponsor_approved`, or
        /// `all` when `--email` is given (an explicit `--status` always wins).
        #[arg(long)]
        status: Option<String>,
        /// Filter to one goatnet (e.g. `zagros`).
        #[arg(long)]
        env: Option<String>,
        /// Show only this address's proposals (exact match, ASCII
        /// case-insensitive — the same fold as the store's collision key)
        /// — the lookup for a duplicate-email 409, which names the proposal
        /// blocking a submit; this finds it whatever state it is in, so it
        /// defaults to `--status all`. An explicit `--status` or `--env`
        /// can hide the blocking proposal. A non-escalated sponsor sees only their
        /// own invitees (the server scopes the list), so an existing
        /// proposal can still come back empty here.
        #[arg(long)]
        email: Option<String>,
        /// Emit raw JSON instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// Approve a sponsor-approved proposal → the runner mints it (or run the
    /// emitted command). Refused unless the sponsor has already vouched.
    Approve {
        /// Proposal id (`prop-…`).
        id: String,
        /// Emit the raw JSON (manifest + command) instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// Reject a proposal (at either the sponsor or admin stage).
    Reject {
        /// Proposal id (`prop-…`).
        id: String,
        /// Reason, recorded on the proposal.
        #[arg(long)]
        note: Option<String>,
    },
    /// Sponsor-vouch a proposed candidate (stage 2) → moves it to
    /// `sponsor_approved` for the admin's stage-3. Refused on an incomplete draft.
    SponsorApprove {
        /// Proposal id (`prop-…`).
        id: String,
        /// Emit raw JSON ack instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// Sponsor-decline a proposed candidate (stage 2) → `rejected`.
    SponsorReject {
        /// Proposal id (`prop-…`).
        id: String,
        /// Reason, recorded on the proposal.
        #[arg(long)]
        note: Option<String>,
    },
}

/// `goat admin invites <subcmd>` — manage existing onboarding invites.
#[derive(Subcommand, Debug)]
pub enum InviteAction {
    /// List invites. Default `invited` (the still-open ones).
    Ls {
        /// Status filter: `invited` | `submitted` | `expired` | `revoked` | `all`.
        #[arg(long, default_value = "invited")]
        status: String,
        /// Filter to one goatnet (e.g. `zagros`).
        #[arg(long)]
        env: Option<String>,
        /// Emit raw JSON instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// Revoke an open invite before it is consumed (fails closed on use).
    Revoke {
        /// Invite id.
        id: String,
    },
}

/// `goat admin ssh-key <subcmd>` — the out-of-band half of host-shell access:
/// an admin submitting or deciding a key without the person touching the
/// console. Every route is operator-only server-side.
#[derive(Subcommand, Debug)]
pub enum SshKeyAction {
    /// List submitted keys. Default `submitted` — what is waiting on a decision.
    Ls {
        /// Status filter: `submitted` | `approved` | `installed` | `rejected` |
        /// `revoked` | `all`.
        #[arg(long, default_value = "submitted")]
        status: String,
        /// Filter to one goatnet (e.g. `zagros-prod`).
        #[arg(long)]
        env: Option<String>,
        /// Emit raw JSON instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// Record a public key FOR SOMEBODY ELSE — the "send it to an admin over
    /// chat" path, for people who would rather not use the console.
    ///
    /// Takes a FILE, not a pasted string: it is the difference between
    /// `~/.ssh/id_ed25519.pub` and `~/.ssh/id_ed25519`, and a pasted private
    /// key is the one input that must never reach the store.
    Submit {
        /// The person's handle, as onboarding knows them.
        handle: String,
        /// Path to their PUBLIC key file (`*.pub`).
        #[arg(long)]
        pubkey_file: String,
        /// Goatnet this key is for.
        #[arg(long)]
        env: Option<String>,
        /// Note for the reviewer — e.g. which machine this is.
        #[arg(long, default_value = "")]
        note: String,
        /// Login email for this admin (for the display gate and legacy backfill).
        #[arg(long)]
        email: Option<String>,
    },
    /// Approve a submitted key. This is a DECISION, not an installation: the
    /// installer still has to place it, and until then the person has no shell.
    Approve {
        /// Key id (from `ls`).
        id: String,
    },
    /// Reject a submitted key. Terminal — it cannot be approved afterwards.
    Reject {
        /// Key id (from `ls`).
        id: String,
    },
    /// Revoke a key. Works from every live state, because revocation must never
    /// be the operation refused on a technicality.
    Revoke {
        /// Key id (from `ls`).
        id: String,
    },
    /// Install every approved host-shell key for a goatnet. Wraps
    /// `operator-ssh-key-install.yml` via `ops/scripts/ansible-preflight.sh`,
    /// resolving the console URL and operator API key from local config.
    ///
    /// Default behaviour: fetch + preview the approved queue and ask for
    /// confirmation before running the installer. `--check` stops after the
    /// preview, and `--yes` skips the confirmation prompt.
    Install {
        /// Target goatnet slug (e.g. `zagros`, `zagros-prod`). Required.
        #[arg(long)]
        env: String,
        /// Preview-only: print what WOULD be installed and change nothing.
        #[arg(long)]
        check: bool,
        /// Skip the interactive confirmation prompt for a real install.
        #[arg(long)]
        yes: bool,
    },
}

/// `goat admin role <subcmd>` — login-access changes on an existing identity,
/// driven through the console machine API (POST /api/users/groups | /state).
#[derive(Subcommand, Debug)]
pub enum AdminRoleCmd {
    /// Set a person's Ory login groups to EXACTLY `--groups` (the full desired
    /// set; a person keeping IAM-ADMIN and gaining nothing else still passes the
    /// whole list). Escalated admin groups (PLATFORM-ADMIN / IAM-ADMIN) are
    /// allowed on this operator-key path and fully attributed in the audit.
    Set {
        /// The person's email (the dedup key; the console resolves it to their
        /// Kratos identity).
        #[arg(long)]
        email: String,
        /// Comma-separated full desired group set, e.g.
        /// `--groups PLATFORM-ADMIN` or `--groups IAM-ADMIN,GRAFANA-READONLY`.
        /// Pass `--groups ""` to clear all login groups. Required so an omitted
        /// flag can never silently clear a person's groups.
        #[arg(long, value_delimiter = ',', required = true)]
        groups: Vec<String>,
        /// Also force-log-out the person's live sessions (use on a reduction so
        /// old claims don't linger).
        #[arg(long)]
        revoke_sessions: bool,
        /// Emit raw JSON instead of the human line.
        #[arg(long)]
        json: bool,
    },
    /// Reactivate a deactivated identity (state → active) — the pre-step before
    /// granting a returning principal.
    Reactivate {
        /// The person's email.
        #[arg(long)]
        email: String,
        /// Explicitly append a revocation-ledger reinstatement via privileged SSH.
        #[arg(long, requires_all = ["reason", "env"])]
        reinstate: bool,
        /// Reason recorded in both console audit and the immutable ledger.
        #[arg(long, requires = "reinstate")]
        reason: Option<String>,
        /// Goatnet for the privileged management-host recovery path.
        #[arg(long, requires = "reinstate")]
        env: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Deactivate an identity (state → inactive; reversible with `reactivate`).
    /// Refused if it would remove the last platform-admin.
    Deactivate {
        /// The person's email.
        #[arg(long)]
        email: String,
        #[arg(long)]
        json: bool,
    },
}

/// `goat admin operator <subcmd>` — the backend-operator tier (F-207 /
/// admin-operator-guide). Separate from the participant `onboard` so its
/// distinct surfaces (operator leaf, host SSH, ADMINS grant) are explicit.
#[derive(Subcommand, Debug)]
pub enum AdminOperatorCmd {
    /// Provision the operator tier for `<handle>` (a participant manifest that
    /// also carries an `operator:` block). Wraps `operator-tier-onboard.yml`.
    Onboard {
        /// Handle (kebab-case slug). Manifest at `ops/operators/<env>/<handle>.yml`
        /// (or flat `ops/operators/<handle>.yml` with no `--env`).
        handle: String,
        /// Goatnet the manifest belongs to (e.g. `zagros`). Passed to the
        /// playbook as `-e operator_env=<env>`.
        #[arg(long)]
        env: Option<String>,
        /// Override repo root (defaults to the nearest `.git` ancestor of cwd).
        #[arg(long)]
        repo_root: Option<PathBuf>,
        /// Don't run ansible — print the command + exit 0.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PeerCmd {
    /// List mesh peers (id, name, mesh IP, connected, last-seen, groups).
    /// The IP shown is the mesh address (100.64.x.x on wt0) — NOT a LAN
    /// address. Direct-vs-relayed is peer-local runtime state (`netbird
    /// status` / the mesh-health domain), not in the mgmt-API peer list.
    Ls {
        /// Emit a JSON array instead of the table.
        #[arg(long)]
        json: bool,
        /// Override the NetBird mgmt URL (else env / derived from profile).
        #[arg(long)]
        mgmt_url: Option<String>,
    },
    /// Eject a peer from the mesh — the authoritative cut. Selector is an
    /// exact peer id, an exact mesh IP, or a case-insensitive substring of
    /// the peer name/hostname (as shown by `ls`). Refuses if the selector
    /// matches more than one peer (re-run with a more specific selector).
    Cut {
        /// Peer id | mesh IP | name/hostname substring.
        selector: String,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Override the NetBird mgmt URL (else env / derived from profile).
        #[arg(long)]
        mgmt_url: Option<String>,
        /// Resolve + print the target peer, but don't delete. Exit 0.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum AdminClientCmd {
    /// Mint a wg-cp0 bundle for a device. Reads the manifest at
    /// `ops/devices/<handle>.yml`, runs the operator-side validate.py
    /// pre-flight, then runs `bundle-create` with flags derived from
    /// the manifest + per-site fleet config. Output bundle lands at
    /// `ops/devices/out/<handle>-device.cbor` (mode 0600). Operator
    /// hands that file to the device-side operator who runs
    /// `goat client bundle import <path>` on the device.
    Onboard {
        /// Handle (kebab-case slug). Manifest must exist at
        /// `ops/devices/<handle>.yml`.
        handle: String,
        /// CA identifier passed to bundle-create -ca-id (e.g.
        /// `dev-goat-ca-2026-04-20`). Required when
        /// $GOAT_DEV_CA_ID is not set.
        #[arg(long, env = "GOAT_DEV_CA_ID")]
        ca_id: Option<String>,
        /// Signing mode: dev (age-encrypted file) | prod (YubiKey
        /// PIV slot 9c). Defaults to dev.
        #[arg(long, default_value = "dev")]
        mode: String,
        /// Dev mode. Path to age identity file. Defaults to
        /// $GOAT_DEV_CA_AGE_IDENTITY when --mode=dev.
        #[arg(long, env = "GOAT_DEV_CA_AGE_IDENTITY")]
        age_identity: Option<PathBuf>,
        /// Dev mode. Path to age-encrypted CA private key. Defaults
        /// to $GOAT_DEV_CA_KEY when --mode=dev.
        #[arg(long, env = "GOAT_DEV_CA_KEY")]
        ca_key: Option<PathBuf>,
        /// Optional. Path to a PEM TLS trust anchor, embedded in the
        /// bundle as tls_trust_anchor.
        #[arg(long, env = "GOAT_TLS_TRUST_ANCHOR")]
        tls_trust_anchor: Option<PathBuf>,
        /// Optional. Snitch collector URL (Block 61). When the
        /// manifest's `bundle.snitch` is true AND this is set, the
        /// wrapper registers the minted snitch_probe_key.
        #[arg(long, env = "SNITCH_COLLECTOR")]
        snitch_collector: Option<String>,
        /// Override repo root (defaults to searching upward from cwd
        /// for a `.git` directory).
        #[arg(long)]
        repo_root: Option<PathBuf>,
        /// Print the bundle-create invocation + exit 0 without
        /// running it. Useful for previewing without offline-CA
        /// material loaded.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum VendorCmd {
    /// Create a new org in the portal. Admin-gated server-side.
    Create {
        /// Org prefix (kebab-case, [a-z0-9-]{1,32}). Used as the
        /// X.509 URI-SAN authority + the topic-key leading segment.
        prefix: String,
        /// Human-readable display name (shown in the portal org
        /// catalog UI). Defaults to the prefix.
        #[arg(long)]
        display_name: Option<String>,
        /// Mark as synthetic (trashgoat-style fake org). Defaults
        /// to false (real org).
        #[arg(long)]
        synthetic: bool,
        /// Auto-approve at create time. Admin tokens can do this; non-
        /// admin tokens get 403 if they try.
        #[arg(long)]
        auto_approve: bool,
        /// Don't actually POST; print the body + exit 0.
        #[arg(long)]
        dry_run: bool,
    },
    /// Revoke an org. Portal refuses with 409 if the org still
    /// has non-retired topics — retire them first via `goat topic
    /// delete --hard <key>`.
    Delete {
        prefix: String,
        /// Skip confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Don't actually DELETE; print the intended request + exit 0.
        #[arg(long)]
        dry_run: bool,
    },
    // ------------------------------------------------------------------
    // ADR 1112 — the first-class ORG REGISTRY (console-side affiliation
    // relations: nation → service → unit). Distinct from Create/Delete
    // above, which are the PORTAL's producer/slot org lifecycle.
    // ------------------------------------------------------------------
    /// Render the org registry as a tree (roots, children indented), with
    /// kind/nation/aliases/status. Read-only.
    Tree,
    /// Fuzzy-match a typed org name against the registry (ADR 1112 §7).
    /// NEVER auto-merges — similar spelling is not the same org; this ranks
    /// candidates for a human to judge. 404 = matching not enabled here.
    Suggest {
        /// The name as typed (any language/spelling — normalization folds
        /// case, diacritics, punctuation).
        name: String,
        /// Scope candidates to a nation (ISO alpha-3 / NATO / EU).
        #[arg(long)]
        nation: Option<String>,
    },
    /// Register an org in the registry (deliberate act; audited). Use
    /// `--despite <candidate>` for every suggest candidate you reviewed and
    /// judged DISTINCT — recorded as negative pairs so that pairing is never
    /// re-suggested as a top candidate.
    Register {
        /// Kebab org id (immutable — never renamed; a rename is a merge).
        id: String,
        #[arg(long)]
        display_name: Option<String>,
        /// vendor | organic | partner | civil.
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        nation: Option<String>,
        /// Parent org id (must already exist — chains build top-down).
        #[arg(long)]
        parent: Option<String>,
        /// authoritative | operator-verified | open-source | self-asserted
        /// (default operator-verified for a CLI act).
        #[arg(long)]
        provenance: Option<String>,
        #[arg(long)]
        justification: Option<String>,
        /// Candidate org id you reviewed and judged distinct (repeatable).
        #[arg(long)]
        despite: Vec<String>,
    },
    /// Set (or clear) an org's parent — an effective-dated containment edge;
    /// history survives, cycles are refused at write time.
    SetParent {
        id: String,
        /// The new parent org id-or-alias. Omit with --clear to detach.
        #[arg(long, conflicts_with = "clear")]
        parent: Option<String>,
        /// Detach: the org becomes a root.
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        justification: Option<String>,
    },
    /// Correct an existing org's facts (kind / nation / composition / display
    /// name) in place — audited; never touches identity, grants, or keys (org ids are
    /// immutable and off every wire anchor, ADR 1112 §3). The verb the
    /// `origin` kind mislabel lacked.
    Update {
        id: String,
        /// vendor | organic | partner | civil.
        #[arg(long)]
        kind: Option<String>,
        /// ISO 3166-1 alpha-3 (e.g. LVA) or a marker (NATO, EU).
        #[arg(long)]
        nation: Option<String>,
        #[arg(long)]
        display_name: Option<String>,
        /// national | multinational — is this organisation one sovereign's, or
        /// constituted by a body of nations? (ADR 1171). Pass an empty string to
        /// clear it back to not-yet-declared, which kind and nation refuse.
        #[arg(long)]
        composition: Option<String>,
        #[arg(long)]
        justification: Option<String>,
    },
    /// Record a name-variant/abbreviation alias for an org (accretes matcher
    /// recall; aliases share one namespace with org ids).
    Alias {
        id: String,
        alias: String,
        /// name-variant | abbreviation (absorbed-id is minted only by merge).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Alias-absorb merge (ADR 1112 §8): LOSER's id becomes a permanent
    /// alias of --into; everything keyed to it keeps resolving; children
    /// re-parent effective-dated; reversible via unmerge. A merge is ALSO a
    /// governance union — review both orgs' authority sets (govern admins,
    /// grants-as-grantee, multipub pools: `goat access show`, `goat access
    /// co-publish list-topics`) and pass --acknowledge-governance-union.
    Merge {
        loser: String,
        #[arg(long)]
        into: String,
        #[arg(long)]
        justification: Option<String>,
        /// Assert you reviewed the combined authority set. Fail-closed
        /// without it (server-side too).
        #[arg(long)]
        acknowledge_governance_union: bool,
    },
    /// Reactivate an absorbed org (the merge reversal). Parent edges are NOT
    /// auto-reverted — effective-dated history is never rewritten; re-attach
    /// with set-parent deliberately.
    Unmerge { id: String },
}

pub async fn run(ctx: &super::Ctx, op: AdminCmd) -> anyhow::Result<()> {
    // `goat admin` is operator-only. Refuse the certain non-admin case up
    // front (inert: nothing is sent) with a clear E_NOT_ADMIN; everything
    // else flows to its authoritative server-side / local-credential check,
    // which also surfaces E_NOT_ADMIN on a 403. See `ensure_admin_capability`.
    ensure_admin_capability(&ctx.profile, &op)?;

    // Onboard + Client don't need a portal client (each runs its own
    // pre-flight + child process), so build the client lazily.
    if let AdminCmd::Onboard {
        handle,
        org,
        yes,
        env,
        revoke,
        purge,
        from,
        repair,
        reissue_handset_key,
        reissue_device_key,
        uses,
        requested_by,
        native,
        defer_credential,
        ansible,
        repo_root,
        collab,
        dry_run,
    } = &op
    {
        // `--collab` records its decision on the native revoke receipt; the
        // Ansible revoke leg has no receipt to carry it. Refuse rather than
        // accept a decision we would silently drop (F-274 is exactly the class
        // of bug where a collab fact went unrecorded).
        if collab.is_some() && *ansible {
            anyhow::bail!(
                "--collab is a native-revoke option (it lands on the native revoke receipt + audit event); \
                 the Ansible revoke leg has nowhere to record it. Drop --ansible, or make the collab \
                 decision explicitly with `onboard-runner -collab keep|revoke`."
            );
        }
        // --purge is a native-revoke feature (the Ansible wrapper here doesn't
        // thread `-e revoke_purge`); refuse the incoherent combo up front.
        if *purge && *ansible {
            anyhow::bail!(
                "--purge is a native-path option; `--ansible --purge` isn't wired. \
                 Drop --ansible (native handles the purge), or run the playbook directly \
                 with `-e revoke_purge=true`."
            );
        }
        // `--from FILE` — batch cohort revoke, one handle per line. The generalized
        // sweep tool: each handle is revoked natively + independently, a per-handle
        // failure is reported not fatal. (clap: --from requires --revoke, conflicts
        // with handle/org.)
        if let Some(path) = from {
            return revoke_native_batch_from_file(
                ctx,
                path,
                env.as_deref(),
                *purge,
                repo_root.as_deref(),
                *dry_run,
                *collab,
            )
            .await;
        }
        if let Some(org_id) = org {
            if !*revoke {
                anyhow::bail!(
                    "--org is a revoke cascade selector; pass --revoke too \
                     (there is no whole-org enroll)"
                );
            }
            // Native org cascade (concurrent per-member native revoke) unless
            // `--ansible` forces the legacy per-member playbook loop.
            if *ansible {
                return onboard_org(
                    &ctx.profile,
                    org_id,
                    env.as_deref(),
                    repo_root.as_deref(),
                    *dry_run,
                    *yes,
                )
                .await;
            }
            return revoke_native_org(
                ctx,
                org_id,
                env.as_deref(),
                *purge,
                repo_root.as_deref(),
                *dry_run,
                *yes,
                *collab,
            )
            .await;
        }
        let handle = handle.as_deref().ok_or_else(|| {
            anyhow::anyhow!("provide a positional <handle>, or --org <affiliation-id> --revoke")
        })?;
        if *reissue_handset_key {
            return reissue_handset_key_native(
                &ctx.profile,
                handle,
                env.as_deref(),
                repo_root.as_deref(),
                requested_by.as_deref(),
                *dry_run,
            )
            .await;
        }
        if *reissue_device_key {
            return reissue_device_key_native(
                &ctx.profile,
                handle,
                env.as_deref(),
                repo_root.as_deref(),
                *uses,
                requested_by.as_deref(),
                *dry_run,
            )
            .await;
        }
        // P1.5 — `--repair` = hardened revoke leg (frees surfaces, keeps the
        // manifest active) then enroll leg (re-mints + renders every surface
        // together). Sequenced via the playbook so a partial bundle is rebuilt
        // without the all-or-nothing render leaving a gap. Bail if the revoke leg
        // fails so we never re-mint onto un-freed state. (repair stays Ansible.)
        if *repair {
            eprintln!("goat admin onboard --repair: step 1/2 — freeing surfaces (revoke leg)");
            onboard(
                &ctx.profile,
                handle,
                env.as_deref(),
                true,
                true,
                repo_root.as_deref(),
                *dry_run,
            )
            .await?;
            eprintln!(
                "goat admin onboard --repair: step 2/2 — re-minting all surfaces (enroll leg)"
            );
            return onboard(
                &ctx.profile,
                handle,
                env.as_deref(),
                false,
                false,
                repo_root.as_deref(),
                *dry_run,
            )
            .await;
        }
        // Native fast path is the DEFAULT for BOTH enroll and revoke (portal +
        // NetBird API + Ory-over-SSH, no Ansible, ~seconds). `--native` forces it;
        // `--ansible` forces the playbook; otherwise default native when the
        // env/manifest is native-serviceable, else fall back to Ansible with a note
        // (Ory operator-IdP, no native config, observer/prod Zenoh flavours).
        let want_native = if *native {
            true
        } else if *ansible {
            false
        } else {
            let elig = if *revoke {
                native_revoke_eligibility(repo_root.as_deref(), handle, env.as_deref(), &ctx.profile)
            } else {
                native_eligibility(repo_root.as_deref(), handle, env.as_deref())
            };
            match elig {
                Ok(None) => true,
                Ok(Some(reason)) => {
                    eprintln!(
                        "note: using the Ansible {} path — {reason}.",
                        if *revoke { "revoke" } else { "onboard" }
                    );
                    false
                }
                // A missing / unreadable / unparseable manifest: stay quiet and
                // fall through to the Ansible path, which emits the authoritative
                // "no operator manifest at <path>" error. No noisy eligibility note.
                Err(_) => false,
            }
        };
        if *revoke {
            if want_native {
                return revoke_native(
                    ctx,
                    handle,
                    env.as_deref(),
                    *purge,
                    false,
                    repo_root.as_deref(),
                    *dry_run,
                    *collab,
                )
                .await;
            }
            if collab.is_some() {
                // Same reason as `--ansible --collab`: the fallback playbook path
                // has no receipt, so the decision would evaporate.
                anyhow::bail!(
                    "--collab needs the native revoke path, and this handle/env fell back to Ansible \
                     (see the note above). Record the collab decision with \
                     `onboard-runner -collab keep|revoke` instead."
                );
            }
            return onboard(
                &ctx.profile,
                handle,
                env.as_deref(),
                true,
                false,
                repo_root.as_deref(),
                *dry_run,
            )
            .await;
        }
        // ------------------------------------------------------------------
        // ENROLL FENCE — the other half of operator-onboard.yml's "refuse the retired enroll leg" step.
        //
        // The playbook's enroll leg was retired in the July 2026 ADR 1062
        // cutover and refuses at its "refuse the retired enroll leg" step. That gate only ever covered the
        // ANSIBLE implementation. `onboard_native()` below does the same job in
        // Rust — mints the NetBird setup key, the Zenoh slot + cert + portal
        // api_key and the Ory Kratos identity, then writes a bundle carrying
        // PRIVATE KEY material into ops/operators/out/ — and had no gate at all.
        //
        // So which fence a hand-authored manifest hit was decided by
        // native_eligibility(): whether it declares a Zenoh trial publisher, has
        // ory_idp off, is not an observer or prod manifest. Every one of those is
        // a feature-COVERAGE test and none is a safety test, which meant the
        // ADR 1062 rule that nobody both invites and admits could be defeated on
        // a production goatnet by one command against a hand-written file
        // This closes that half.
        //
        // REVOKE is deliberately untouched on both paths — it is the live
        // offboarding mechanism (ADR 1031) and has already returned above.
        //
        // --dry-run passes: onboard_native returns early and mints nothing, so
        // reading the plan stays available while performing it does not.
        //
        // The escape hatch mirrors the playbook's variable name on purpose, so
        // the two read as ONE fence rather than two unrelated guards. It is
        // deliberately ugly and deliberately not a flag you would pass by habit:
        // if you are using it you should be able to say in one sentence why the
        // console path cannot serve, and you should expect to be asked.
        if !*dry_run && !enroll_leg_override() && !driven_by_onboard_runner() {
            let env_label = env.as_deref().unwrap_or("<env>");
            anyhow::bail!(
                "REFUSED: the ENROLL leg is retired (July 2026, ADR 1062 cutover).\n\
                 \n\
                 This mints every surface from a hand-authored manifest and writes an\n\
                 out-of-band bundle carrying a PRIVATE KEY for an operator to carry to\n\
                 the person. The live path never puts a person's key in an operator's\n\
                 hands. It applies to the native fast path as much as to the playbook:\n\
                 both do the same thing, and only the playbook used to say so.\n\
                 \n\
                 Use the three-stage candidate flow\n\
                 (docs/design/three-stage-candidate-onboarding.md):\n\
                 \n\
                 \x20 goat admin invite --env {env_label} …\n\
                 \n\
                 then the candidate accepts in goat-id-console and onboard-runner\n\
                 mints. Afterwards reconcile the declared roster with:\n\
                 \n\
                 \x20 ops/scripts/sync-onboarded-manifests.py --env {env_label} …\n\
                 \n\
                 The manifest at ops/operators/{env_label}/{handle}.yml is an OUTPUT of\n\
                 that flow, not an input to this one.\n\
                 \n\
                 REVOKE still runs here and is unaffected:  --revoke\n\
                 --dry-run also still runs, and mints nothing.\n\
                 \n\
                 If you genuinely must enroll this way (bootstrapping a goatnet with no\n\
                 console yet is the only case anyone has produced), re-run with:\n\
                 \n\
                 \x20 GOAT_I_UNDERSTAND_THE_ENROLL_LEG_IS_RETIRED=true goat admin onboard …\n\
                 \n\
                 which is the same override the playbook takes as\n\
                 -e i_understand_the_enroll_leg_is_retired=true."
            );
        }
        if want_native {
            return onboard_native(
                ctx,
                handle,
                env.as_deref(),
                repo_root.as_deref(),
                *dry_run,
                *defer_credential,
            )
            .await;
        }
        return onboard(
            &ctx.profile,
            handle,
            env.as_deref(),
            false,
            false,
            repo_root.as_deref(),
            *dry_run,
        )
        .await;
    }
    if let AdminCmd::Client { op } = &op {
        return run_client(&ctx.profile, op).await;
    }
    // Operator-tier onboard: like Onboard, runs its own pre-flight + child
    // process, no portal client needed.
    if let AdminCmd::Operator { op } = &op {
        let AdminOperatorCmd::Onboard {
            handle,
            env,
            repo_root,
            dry_run,
        } = op;
        return operator_onboard(
            &ctx.profile,
            handle,
            env.as_deref(),
            repo_root.as_deref(),
            *dry_run,
        )
        .await;
    }
    // Peer uses the NetBird mgmt API (not the portal) — handle before the
    // portal client is built.
    if let AdminCmd::Peer { op } = op {
        return run_peer(ctx, op).await;
    }
    // Members (the ADR 1088 transfer engine) uses the NetBird mgmt API and,
    // for the Ory/marker legs, Kratos over the SSH hop — no portal client.
    if let AdminCmd::Members { op } = op {
        return super::members::run(ctx, op).await;
    }
    // platform-admin builds its own per-goatnet clients (console, NetBird,
    // Kratos over SSH) keyed by --env.
    if let AdminCmd::PlatformAdmin { op } = op {
        return super::platform_admin::run(ctx, op).await;
    }
    // Pqc actuates the local (or `--peer` over SSH) netbird client and reads
    // back `netbird status`; no portal client needed. `--fleet` uses the
    // NetBird mgmt API only to enumerate peers for the plan.
    if let AdminCmd::Pqc {
        state,
        peer,
        fleet,
        ssh_user,
        mgmt_url,
        yes,
        dry_run,
        json,
    } = &op
    {
        return run_pqc(
            ctx,
            &PqcArgs {
                state: *state,
                peer: peer.as_deref(),
                fleet: *fleet,
                ssh_user: &ssh_user
                    .clone()
                    .or_else(|| ctx.profile.profile_field("mgmt_ssh_user"))
                    .unwrap_or_default(),
                mgmt_url: mgmt_url.as_deref(),
                yes: *yes,
                dry_run: *dry_run,
                json: *json,
            },
        )
        .await;
    }
    // Rotate needs both the portal client AND alias resolution (it accepts
    // an anchor or its slot alias), so handle it here with ctx in scope.
    if let AdminCmd::Rotate {
        selector,
        org,
        yes,
        repo_root,
        dry_run,
        key_only,
    } = &op
    {
        if let Some(org_id) = org {
            return rotate_org(ctx, org_id, repo_root.as_deref(), *dry_run, *yes).await;
        }
        let selector = selector.as_deref().ok_or_else(|| {
            anyhow::anyhow!("provide a positional <anchor|alias>, or --org <affiliation-id>")
        })?;
        if *key_only {
            return rekey(ctx, selector, *dry_run).await;
        }
        return rotate(ctx, selector, *dry_run).await;
    }
    // Lan + Storm deliberately need no portal client either — they must work
    // when the mesh + every goatnet service is down (that is the whole point of
    // the storm brake).
    let op = match op {
        AdminCmd::Lan { op } => return super::lan::run(ctx, op).await,
        AdminCmd::Storm { op } => return super::storm::run(ctx, op).await,
        other => other,
    };

    // `ssh-key install` does not need a PortalClient — it resolves the console
    // endpoint + operator key keyed by `--env` and then wraps the Ansible
    // installer. Handle it before constructing the portal client so missing
    // env-specific config is reported directly.
    if let AdminCmd::SshKey {
        op: SshKeyAction::Install { ref env, check, yes },
    } = op
    {
        return run_ssh_key_install(&ctx.profile, env.clone(), check, yes).await;
    }

    let portal = ctx.profile.portal_client()?;
    match op {
        AdminCmd::Onboard { .. } => unreachable!(),
        AdminCmd::Client { .. } => unreachable!(),
        AdminCmd::Operator { .. } => unreachable!(),
        AdminCmd::Peer { .. } => unreachable!(),
        AdminCmd::Members { .. } => unreachable!(),
        AdminCmd::PlatformAdmin { .. } => unreachable!(),
        AdminCmd::Pqc { .. } => unreachable!(),
        AdminCmd::Rotate { .. } => unreachable!(),
        AdminCmd::Lan { .. } => unreachable!(),
        AdminCmd::Storm { .. } => unreachable!(),
        AdminCmd::Invite {
            file,
            edit,
            env,
            tier,
            affiliation,
            site,
            machine_quota,
            ttl_hours,
            note,
            collab,
            email,
            json,
        } => {
            run_invite(
                &portal,
                file,
                edit,
                env,
                tier,
                affiliation,
                site,
                machine_quota,
                ttl_hours,
                note,
                collab,
                email,
                json,
            )
            .await
        }
        AdminCmd::Queue { action } => run_queue(&portal, action).await,
        AdminCmd::SshKey { op } => run_ssh_key(&portal, op).await,
        AdminCmd::Invites { op } => run_invites(&portal, op).await,
        AdminCmd::Role { op } => run_role(&ctx.profile, &portal, op).await,
        AdminCmd::OperatorKey { op } => run_operator_key(&portal, op).await,
        AdminCmd::Sponsorship {
            id,
            max_tier,
            email,
            revoke,
        } => {
            let (authorized, tier) = if revoke {
                (false, String::new())
            } else {
                let t = max_tier.ok_or_else(|| {
                    anyhow::anyhow!(
                        "granting sponsorship requires --max-tier <browse|developer|iam-admin|platform-admin> \
                         (the level cap is part of the decision); use --revoke to remove it"
                    )
                })?;
                (true, t)
            };
            let res = portal
                .set_sponsorship(&id, authorized, &tier, email.as_deref())
                .await
                .map_err(super::vendor::map_portal_err);
            emit_vendor_audit(&ctx.profile, "vendor.sponsorship.set", &id, res.is_ok()).await;
            res?;
            if authorized {
                match &email {
                    Some(e) => println!(
                        "sponsorship granted to `{id}` (max tier: {tier}; console login bound to {e})"
                    ),
                    None => println!("sponsorship granted to `{id}` (max tier: {tier})"),
                }
            } else {
                println!("sponsorship revoked from `{id}`");
            }
            Ok(())
        }
        AdminCmd::Vendor { op } => match op {
            VendorCmd::Create {
                prefix,
                display_name,
                synthetic,
                auto_approve,
                dry_run,
            } => {
                let req = NewVendor {
                    prefix: prefix.clone(),
                    display_name: display_name.unwrap_or_else(|| prefix.clone()),
                    synthetic,
                    auto_approve,
                    sponsor_authorized: false,
                    sponsor_max_tier: String::new(),
                    sponsor_email: String::new(),
                    ..Default::default()
                };
                if dry_run {
                    eprintln!("dry-run: POST <portal>/api/orgs");
                    eprintln!(
                        "  body: {}",
                        serde_json::to_string(&req).unwrap_or_default()
                    );
                    return Ok(());
                }
                // Best-effort operator-action audit: emit after the
                // portal call so the recorded outcome is the real one. Never
                // changes this command's own exit behavior.
                let res = portal
                    .create_vendor(req)
                    .await
                    .map_err(super::vendor::map_portal_err);
                emit_vendor_audit(&ctx.profile, "vendor.create", &prefix, res.is_ok()).await;
                res?;
                println!("created org `{prefix}`");
                Ok(())
            }
            VendorCmd::Delete {
                prefix,
                yes,
                dry_run,
            } => {
                if dry_run {
                    eprintln!("dry-run: DELETE <portal>/api/orgs/{prefix}");
                    return Ok(());
                }
                if !yes {
                    use std::io::Write;
                    eprint!("Revoke org `{prefix}` at the portal? [y/N] ");
                    std::io::stderr().flush().ok();
                    let mut answer = String::new();
                    std::io::stdin().read_line(&mut answer)?;
                    if !answer.trim().eq_ignore_ascii_case("y") {
                        anyhow::bail!("aborted (no confirmation)");
                    }
                }
                let outcome = portal.delete_vendor(&prefix).await;
                // Best-effort operator-action audit. A 409 (non-retired
                // topics) is a failed delete — record it as such.
                emit_vendor_audit(&ctx.profile, "vendor.delete", &prefix, outcome.is_ok()).await;
                match outcome {
                    Ok(()) => {
                        println!("revoked org `{prefix}`");
                        Ok(())
                    }
                    Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                        anyhow::bail!(
                            "portal refused: org `{prefix}` has non-retired topics — \
                             retire them first via `goat topic delete --hard <key>`"
                        )
                    }
                    Err(e) => Err(super::vendor::map_portal_err(e)),
                }
            }
            VendorCmd::Tree => {
                let orgs = portal
                    .org_registry_list()
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                if orgs.is_empty() {
                    println!("org registry is empty (no orgs registered yet)");
                    return Ok(());
                }
                render_org_tree(&orgs);
                Ok(())
            }
            VendorCmd::Suggest { name, nation } => {
                let cs = match portal
                    .org_registry_suggest(&name, nation.as_deref().unwrap_or(""))
                    .await
                {
                    Ok(cs) => cs,
                    Err(PortalError::Http { status, .. }) if status.as_u16() == 404 => {
                        return Err(super::vendor::append_onboard_hint(
                            &portal,
                            anyhow::anyhow!(
                                // Do NOT assert a cause from a bare status code. A 404 on
                                // this path has at least three sources and the client cannot
                                // tell them apart: the route may not be reachable at all (the
                                // onboard host routed only /api/onboard/ until #503), the org
                                // may genuinely not exist (GetOrg -> ErrNotFound), or org
                                // matching may indeed be off. The previous text asserted the
                                // third unconditionally, which is FALSE on any goatnet where
                                // the flag is on. The server already returns a typed
                                // `org_match_disabled` code -- surfacing it needs
                                // PortalError::Http to carry the body (56 construction sites,
                                // 9 files); until then, name the possibilities honestly.
                                "org-registry lookup returned 404. Either the org is not \
                                 registered on this goatnet, org matching is disabled \
                                 (ORG_MATCH_ENABLED), or the org-registry route is not \
                                 reachable at this endpoint"
                            ),
                        ));
                    }
                    Err(e) => return Err(super::vendor::map_onboard_err(&portal, e)),
                };
                if cs.is_empty() {
                    println!("no candidates for {name:?} — a new org would be genuinely new here");
                    return Ok(());
                }
                println!("candidates for {name:?} (a human confirms — similar spelling is NOT the same org):");
                for c in &cs {
                    let mut facts = Vec::new();
                    if !c.display_name.is_empty() {
                        facts.push(c.display_name.clone());
                    }
                    if !c.nation.is_empty() {
                        facts.push(format!("nation {}", c.nation));
                    }
                    if !c.parent.is_empty() {
                        facts.push(format!("under {}", c.parent));
                    }
                    let demoted = if c.demoted {
                        "  [previously judged distinct]"
                    } else {
                        ""
                    };
                    println!(
                        "  {:5}  {:.2}  {}  ({}) via {}{}",
                        c.band,
                        c.score,
                        c.id,
                        facts.join(" · "),
                        c.via,
                        demoted
                    );
                }
                Ok(())
            }
            VendorCmd::Register {
                id,
                display_name,
                kind,
                nation,
                parent,
                provenance,
                justification,
                despite,
            } => {
                let v = portal
                    .org_registry_register(
                        &id,
                        display_name.as_deref().unwrap_or(""),
                        kind.as_deref().unwrap_or(""),
                        nation.as_deref().unwrap_or(""),
                        parent.as_deref().unwrap_or(""),
                        provenance.as_deref().unwrap_or("operator-verified"),
                        justification.as_deref().unwrap_or(""),
                        &despite,
                    )
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.register", &id, true).await;
                println!("registered org `{}`{}", v.id, org_facts_suffix(&v));
                Ok(())
            }
            VendorCmd::SetParent {
                id,
                parent,
                clear,
                justification,
            } => {
                if parent.is_none() && !clear {
                    anyhow::bail!("pass --parent <org> to attach, or --clear to detach");
                }
                let v = portal
                    .org_registry_set_parent(
                        &id,
                        parent.as_deref().unwrap_or(""),
                        justification.as_deref().unwrap_or(""),
                    )
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.set-parent", &id, true).await;
                if v.parent.is_empty() {
                    println!("`{}` is now a root org", v.id);
                } else {
                    println!("`{}` is now under `{}`", v.id, v.parent);
                }
                Ok(())
            }
            VendorCmd::Update {
                id,
                kind,
                nation,
                display_name,
                composition,
                justification,
            } => {
                if kind.is_none() && nation.is_none() && display_name.is_none()
                    && composition.is_none()
                {
                    anyhow::bail!(
                        "pass at least one of --kind / --nation / --composition / --display-name"
                    );
                }
                let v = portal
                    .org_registry_update(
                        &id,
                        kind.as_deref(),
                        nation.as_deref(),
                        display_name.as_deref(),
                        composition.as_deref(),
                        justification.as_deref().unwrap_or(""),
                    )
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.update", &id, true).await;
                println!(
                    "`{}` updated: kind={} nation={} composition={} display_name={}",
                    v.id,
                    v.kind,
                    v.nation,
                    if v.composition.is_empty() { "(not declared)" } else { &v.composition },
                    if v.display_name.is_empty() { "(id)" } else { &v.display_name }
                );
                Ok(())
            }
            VendorCmd::Alias { id, alias, kind } => {
                let v = portal
                    .org_registry_add_alias(&id, &alias, kind.as_deref().unwrap_or("name-variant"))
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.alias", &id, true).await;
                println!(
                    "`{}` now resolves to `{}` (aliases: {})",
                    alias,
                    v.id,
                    v.aliases.join(", ")
                );
                Ok(())
            }
            VendorCmd::Merge {
                loser,
                into,
                justification,
                acknowledge_governance_union,
            } => {
                if !acknowledge_governance_union {
                    anyhow::bail!(
                        "a merge is also a GOVERNANCE UNION: `{loser}`'s members/grants resolve into \
                         `{into}` and combine authority sets.\n\
                         Review both orgs first:\n\
                         goat access show               (grants where either org is grantee)\n\
                         goat access co-publish list-topics   (multipub pools)\n\
                         then re-run with --acknowledge-governance-union"
                    );
                }
                let v = portal
                    .org_registry_merge(&loser, &into, justification.as_deref().unwrap_or(""), true)
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.merge", &loser, true).await;
                println!(
                    "merged `{loser}` into `{}` — `{loser}` is now a permanent alias; reverse with \
                     `goat admin org unmerge {loser}`",
                    v.id
                );
                Ok(())
            }
            VendorCmd::Unmerge { id } => {
                let v = portal
                    .org_registry_unmerge(&id)
                    .await
                    .map_err(|e| super::vendor::map_onboard_err(&portal, e))?;
                emit_vendor_audit(&ctx.profile, "org-registry.unmerge", &id, true).await;
                println!(
                    "`{}` reactivated (status {}). Parent edges were NOT auto-reverted — \
                     re-attach with `goat admin org set-parent` if needed.",
                    v.id, v.status
                );
                Ok(())
            }
        },
    }
}

/// Suffix of distinguishing org facts for one-line command output.
fn org_facts_suffix(v: &panoscope_core::portal::OrgRegistryOrg) -> String {
    let mut facts = Vec::new();
    if !v.kind.is_empty() {
        facts.push(v.kind.clone());
    }
    if !v.nation.is_empty() {
        facts.push(v.nation.clone());
    }
    if !v.parent.is_empty() {
        facts.push(format!("under {}", v.parent));
    }
    if facts.is_empty() {
        String::new()
    } else {
        format!("  ({})", facts.join(" · "))
    }
}

/// Render the org registry as a tree: roots first, children indented under
/// their parent. Absorbed orgs render dimly-marked at the root level (their
/// resolution routes to the winner via the alias).
fn render_org_tree(orgs: &[panoscope_core::portal::OrgRegistryOrg]) {
    use std::collections::BTreeMap;
    let mut children: BTreeMap<&str, Vec<&panoscope_core::portal::OrgRegistryOrg>> =
        BTreeMap::new();
    let mut roots: Vec<&panoscope_core::portal::OrgRegistryOrg> = Vec::new();
    for o in orgs {
        if o.parent.is_empty() || o.status == "absorbed" {
            roots.push(o);
        } else {
            children.entry(o.parent.as_str()).or_default().push(o);
        }
    }
    fn line(o: &panoscope_core::portal::OrgRegistryOrg, depth: usize) -> String {
        let mut s = format!("{}{}", "  ".repeat(depth), o.id);
        let mut facts = Vec::new();
        if !o.display_name.is_empty() && o.display_name != o.id {
            facts.push(o.display_name.clone());
        }
        if !o.kind.is_empty() {
            facts.push(o.kind.clone());
        }
        if !o.nation.is_empty() {
            facts.push(o.nation.clone());
        }
        if !o.aliases.is_empty() {
            facts.push(format!("aka {}", o.aliases.join(", ")));
        }
        if o.status == "absorbed" {
            facts.push("ABSORBED".into());
        }
        if !facts.is_empty() {
            s.push_str(&format!("  ({})", facts.join(" · ")));
        }
        s
    }
    fn walk(
        o: &panoscope_core::portal::OrgRegistryOrg,
        depth: usize,
        children: &std::collections::BTreeMap<&str, Vec<&panoscope_core::portal::OrgRegistryOrg>>,
        seen: &mut std::collections::BTreeSet<String>,
    ) {
        if !seen.insert(o.id.clone()) {
            return; // cycle guard (the store refuses cycles; belt+suspenders)
        }
        println!("{}", line(o, depth));
        if let Some(kids) = children.get(o.id.as_str()) {
            for k in kids {
                walk(k, depth + 1, children, seen);
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for r in &roots {
        walk(r, 0, &children, &mut seen);
    }
    // Orphans whose parent isn't in the listing (shouldn't happen; honest).
    for o in orgs {
        if !seen.contains(&o.id) {
            println!("{}  (parent `{}` not in listing)", line(o, 0), o.parent);
        }
    }
}

/// Best-effort operator-action audit for a portal vendor mutation.
/// `action` is `vendor.create` / `vendor.delete`, `target` the prefix, and the
/// outcome maps `ok`/`failed`. The env is the active profile context (the
/// goatnet the portal call ran against). Never affects the caller's exit.
pub(crate) async fn emit_vendor_audit(
    profile: &crate::profile::Profile,
    action: &str,
    prefix: &str,
    ok: bool,
) {
    let actor = crate::cmd::audit::actor_handle(profile);
    let outcome = if ok { "ok" } else { "failed" };
    crate::cmd::audit::emit(
        profile,
        crate::cmd::audit::OperatorActionV1::new(
            &actor,
            "portal",
            action,
            prefix,
            profile.context.clone(),
            outcome,
        )
        .with_args_digest(&format!("{action} prefix={prefix}")),
    )
    .await;
}

/// `goat admin peer` — list / eject mesh peers via the NetBird mgmt API.
/// `goat admin invite` — mint a one-time onboarding invite (ADR 1062). Builds a
/// [`NewInvite`] from the optional `--file` (YAML) ⊕ flag overrides, optionally
/// opens `$EDITOR`, then POSTs and prints the nomination link. The server
/// enforces the minimal-burden + escalated-role rules regardless of this client.
#[allow(clippy::too_many_arguments)]
async fn run_invite(
    portal: &panoscope_core::portal::PortalClient,
    file: Option<PathBuf>,
    edit: bool,
    env: Option<String>,
    tier: Option<String>,
    affiliation: Option<String>,
    site: Option<String>,
    machine_quota: Option<i64>,
    ttl_hours: Option<i64>,
    note: Option<String>,
    collab: Option<bool>,
    email: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    // Start from the file (if any), then apply flag overrides.
    let mut req: NewInvite = match &file {
        Some(p) => {
            let raw = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
            serde_yaml::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("parse {} as invite YAML: {e}", p.display()))?
        }
        None => NewInvite::default(),
    };
    if let Some(v) = env {
        req.env = v;
    }
    if let Some(v) = tier {
        req.tier = v;
    }
    if let Some(v) = affiliation {
        req.affiliation = v;
    }
    if let Some(v) = site {
        req.site = v;
    }
    if let Some(v) = machine_quota {
        req.machine_quota = v;
    } else if matches!(req.tier.to_ascii_lowercase().trim(), "platform-admin")
        && req.machine_quota == 0
    {
        // Admin tiers start with a real machine allowance instead of zero.
        //
        // A platform admin stands machines up as a matter of course, and a
        // quota of 0 means their FIRST enrollment fails with
        // `no_enrollment_grant` — an operator discovering, mid-task, that
        // they need someone to authorize them to do the thing they are the
        // authority for. That is friction with no safety value: they hold an
        // operator token and can already raise the ceiling themselves, so a
        // zero start buys a round-trip, not a control.
        //
        // 10 is the working default. Explicit
        // --machine-quota still wins, including 0 for someone who genuinely
        // should enroll nothing.
        req.machine_quota = DEFAULT_ADMIN_MACHINE_QUOTA;
    }
    if let Some(v) = ttl_hours {
        req.ttl_hours = v;
    }
    if let Some(v) = note {
        req.note = v;
    }
    if let Some(v) = collab {
        req.collab = Some(v);
    }
    if let Some(v) = email {
        req.email = v;
    }

    if edit {
        req = edit_invite_in_editor(&req)?;
    }

    // Local heads-up for escalated tiers (the server also flags them).
    if matches!(
        req.tier.to_ascii_lowercase().trim(),
        "iam-admin" | "platform-admin"
    ) {
        eprintln!(
            "⚠  Offering an ESCALATED admin tier ({}). It is flagged for scrutiny at both the \
             sponsor and admin review stages.",
            req.tier
        );
    }

    let minted = portal
        .mint_invite(&req)
        .await
        .map_err(|e| super::vendor::map_onboard_err(portal, e))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": minted.id, "sponsor": minted.sponsor, "env": minted.env,
                "token": minted.token, "nomination_url": minted.nomination_url,
                "nomination_path": minted.nomination_path, "intake_url": minted.intake_url,
                "expires_at": minted.expires_at, "escalated": minted.escalated,
                "emailed_to": minted.emailed_to, "email_error": minted.email_error,
            }))?
        );
        return Ok(());
    }

    // Prefer the PUBLIC front-desk link (reachable off the mesh) when present.
    let (link, scope) = if !minted.intake_url.is_empty() {
        (
            minted.intake_url.clone(),
            "public — send to anyone, on or off the mesh",
        )
    } else if !minted.nomination_url.is_empty() {
        (
            minted.nomination_url.clone(),
            "mesh-internal — reachable only from inside the mesh",
        )
    } else {
        (minted.nomination_path.clone(), "relative path")
    };
    println!("Invite {} minted (sponsor {}).", minted.id, minted.sponsor);
    if minted.escalated {
        println!("⚠  ESCALATED admin role — reviewers will scrutinize this onboarding.");
    }
    if !minted.emailed_to.is_empty() {
        println!("✅ Nomination link emailed to {}.", minted.emailed_to);
    } else if !minted.email_error.is_empty() {
        println!(
            "⚠  requested email send failed ({}) — hand off the link below manually.",
            minted.email_error
        );
    }
    println!("\nSend this one-time link to the candidate ({scope}):\n  {link}\n");
    println!(
        "It works once and expires. The token is shown only here — mint a new invite if lost."
    );
    Ok(())
}

/// Round-trips a [`NewInvite`] through `$EDITOR` as a commented YAML template.
fn edit_invite_in_editor(seed: &NewInvite) -> anyhow::Result<NewInvite> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    // Hand-built template so every optional key is visible for editing (the
    // struct's serializer skips empties). Seed values are pre-filled.
    let q = |s: &str| {
        if s.is_empty() {
            String::new()
        } else {
            s.to_string()
        }
    };
    let template = format!(
        "# goat admin invite — one-time onboarding invite (ADR 1062).\n\
         # Every field is OPTIONAL — leave one blank to let the enrollee fill it.\n\
         env: {}\n\
         tier: {}            # developer | browse | iam-admin | platform-admin  (admin tiers are ESCALATED, sponsor-only)\n\
         affiliation: {}     # vendor | organic | partner | civil\n\
         site: {}\n\
         machine_quota: {}\n\
         ttl_hours: {}       # 0 = default (72h); max 720\n\
         note: {}\n",
        q(&seed.env),
        q(&seed.tier),
        q(&seed.affiliation),
        q(&seed.site),
        seed.machine_quota,
        seed.ttl_hours,
        q(&seed.note),
    );
    let path = std::env::temp_dir().join(format!("goat-invite-{}.yaml", std::process::id()));
    std::fs::write(&path, template.as_bytes())?;
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("launch editor {editor}: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        anyhow::bail!("editor exited non-zero — aborting");
    }
    let edited = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    serde_yaml::from_str(&edited).map_err(|e| anyhow::anyhow!("parse edited invite YAML: {e}"))
}

/// `goat admin queue <ls|approve|reject>` — the admin's stage-3 review + action
/// surface from the CLI (the counterpart to /admin/onboard-queue.html). Reaches
/// the portal over the operator bearer; the two-gate order is enforced portal-
/// side (approve refuses a proposal the sponsor hasn't vouched for → a clear 409).
async fn run_queue(
    portal: &panoscope_core::portal::PortalClient,
    action: QueueAction,
) -> anyhow::Result<()> {
    match action {
        QueueAction::Ls {
            status,
            env,
            email,
            json,
        } => {
            let email = queue_ls_flag_value(
                "email",
                "pass the candidate's address, e.g. --email a@b.test",
                email.as_deref(),
            )?;
            let env = queue_ls_flag_value(
                "env",
                "pass a goatnet (e.g. zagros) or drop the flag",
                env.as_deref(),
            )?;
            let status = queue_ls_flag_value(
                "status",
                "pass a status from --help, `all`, or drop the flag",
                status.as_deref(),
            )?;
            let status = queue_ls_effective_status(status.as_deref(), email.is_some());
            if let Some(hint) = queue_ls_lookup_hint(&status, env.as_deref(), email.as_deref()) {
                eprintln!("{hint}");
            }
            let want = queue_ls_status_param(&status);
            let mut rows = portal
                .list_proposals_filtered(want, email.as_deref())
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, e.into()))?;
            queue_ls_retain(&mut rows, env.as_deref(), email.as_deref());
            if json {
                // Re-fetch note: the Proposal struct is Serialize-free, so print
                // the fields we surface as a JSON array by hand-building values.
                let arr: Vec<serde_json::Value> = rows.iter().map(queue_row_json).collect();
                println!("{}", serde_json::to_string_pretty(&arr)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!(
                    "no proposals ({})",
                    queue_ls_filter_desc(&status, env.as_deref(), email.as_deref())
                );
                return Ok(());
            }
            println!(
                "{:<22}  {:<18}  {:<20}  {:<6}  {:<16}  FLAGS",
                "ID", "HANDLE", "EMAIL", "ENV", "STATUS"
            );
            for p in &rows {
                let mut flags = Vec::new();
                if p.escalated {
                    flags.push("ESCALATED");
                }
                if !p.complete {
                    flags.push("draft");
                }
                if p.petitions.trim() != "[]" && !p.petitions.trim().is_empty() {
                    flags.push("change-request");
                }
                println!(
                    "{:<22}  {:<18}  {:<20}  {:<6}  {:<16}  {}",
                    p.id,
                    trunc(&p.handle, 18),
                    trunc(&p.email, 20),
                    trunc(&p.env, 6),
                    trunc(&p.status, 16),
                    flags.join(" "),
                );
            }
            eprintln!(
                "\n{} shown. Approve: `goat admin queue approve <id>` · reject: `goat admin queue reject <id>`",
                rows.len()
            );
            Ok(())
        }
        QueueAction::Approve { id, json } => {
            let res = match portal.decide_proposal(&id, "approve", "").await {
                Ok(r) => r,
                Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                    anyhow::bail!(
                        "cannot approve {id}: it is not sponsor-approved yet — a sponsor must vouch \
                         for it first (stage 2). See `goat admin queue ls --status proposed`."
                    );
                }
                Err(e) => return Err(super::vendor::append_onboard_hint(portal, e.into())),
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&queue_approve_json(&res))?
                );
                return Ok(());
            }
            println!("approved {} ({})", res.handle, res.id);
            println!("  the onboarding runner will mint it automatically (~30s), if deployed.");
            println!(
                "  manual fallback: write the manifest to {} then run:",
                res.manifest_path
            );
            println!("    {}", res.command);
            Ok(())
        }
        QueueAction::Reject { id, note } => {
            portal
                .decide_proposal(&id, "reject", note.as_deref().unwrap_or(""))
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, e.into()))?;
            println!("rejected {id}");
            Ok(())
        }
        QueueAction::SponsorApprove { id, json } => {
            match portal.decide_proposal(&id, "sponsor-approve", "").await {
                Ok(_) => {}
                Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                    anyhow::bail!(
                        "cannot sponsor-approve {id}: it is not a complete, proposed draft — \
                         the candidate must finish their submission first, or it is already past \
                         stage 2. See `goat admin queue ls --status proposed`."
                    );
                }
                Err(e) => return Err(super::vendor::append_onboard_hint(portal, e.into())),
            }
            if json {
                println!(
                    "{}",
                    serde_json::json!({"id": id, "status": "sponsor_approved"})
                );
                return Ok(());
            }
            println!("sponsor-approved {id} → now awaiting admin approval (stage 3).");
            eprintln!("  admin acts next: `goat admin queue approve {id}`");
            Ok(())
        }
        QueueAction::SponsorReject { id, note } => {
            portal
                .decide_proposal(&id, "sponsor-reject", note.as_deref().unwrap_or(""))
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, e.into()))?;
            println!("sponsor-rejected {id}");
            Ok(())
        }
    }
}

/// Truncate a string to `n` chars for the queue table.
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

fn queue_row_json(p: &Proposal) -> serde_json::Value {
    serde_json::json!({
        "id": p.id, "handle": p.handle, "email": p.email, "env": p.env,
        "status": p.status, "escalated": p.escalated, "complete": p.complete,
        "sponsor": p.sponsor, "petitions": p.petitions,
    })
}

/// Effective `queue ls` status filter. An explicit `--status` always wins;
/// otherwise `--email` widens the default to `all` — the blocking proposal a
/// refused submit names can sit in ANY state, and an email lookup that kept
/// the `sponsor_approved` default would usually print nothing. The plain
/// default stays `sponsor_approved` (the admin's ready-for-decision view).
fn queue_ls_effective_status(explicit: Option<&str>, email_given: bool) -> String {
    match explicit {
        Some(s) => s.to_string(),
        None if email_given => "all".to_string(),
        None => "sponsor_approved".to_string(),
    }
}

/// The `?status=` wire value for an effective `queue ls` status: `all` (any
/// case) means NO param — the console returns every row and the CLI filters
/// client-side. Everything else passes through verbatim.
fn queue_ls_status_param(status: &str) -> &str {
    if status.eq_ignore_ascii_case("all") {
        ""
    } else {
        status
    }
}

/// A `queue ls` filter value: trimmed; a blank value is refused loudly,
/// naming the flag. Blank filters are almost always `--flag "$UNSET_VAR"`
/// accidents — a blank --status listed everything and a blank --env listed
/// nothing, both silently.
fn queue_ls_flag_value(
    flag: &str,
    hint: &str,
    raw: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--{flag} is blank — {hint}");
    }
    Ok(Some(trimmed.to_string()))
}

/// Row filters for `queue ls`: each is exact + ASCII-case-insensitive, and
/// they AND together. Email is a whole-value match on purpose — it is the
/// store's collision key, and a substring match can act on the
/// wrong person's row.
fn queue_ls_retain(rows: &mut Vec<Proposal>, env: Option<&str>, email: Option<&str>) {
    if let Some(e) = env {
        rows.retain(|p| p.env.eq_ignore_ascii_case(e));
    }
    if let Some(m) = email {
        rows.retain(|p| p.email.eq_ignore_ascii_case(m));
    }
}

/// Names every filter that shaped the answer, for the empty-result line — an
/// empty listing that doesn't say what it was filtered by reads as "the store
/// is empty", which is the goat#334 defect in miniature.
fn queue_ls_filter_desc(status: &str, env: Option<&str>, email: Option<&str>) -> String {
    let mut parts = vec![format!("status={status}")];
    if let Some(e) = env {
        parts.push(format!("env={e}"));
    }
    if let Some(m) = email {
        parts.push(format!("email={m}"));
    }
    parts.join(", ")
}

fn queue_ls_lookup_hint(
    status: &str,
    env: Option<&str>,
    email: Option<&str>,
) -> Option<&'static str> {
    if email.is_some() && (!status.eq_ignore_ascii_case("all") || env.is_some()) {
        Some("The status or environment filter can hide an existing proposal. To check for a duplicate, use --email with --status all and no --env. Results are limited to proposals your account can see.")
    } else {
        None
    }
}

fn queue_approve_json(r: &panoscope_core::portal::ApproveResult) -> serde_json::Value {
    serde_json::json!({
        "id": r.id, "status": r.status, "handle": r.handle, "env": r.env,
        "manifest_path": r.manifest_path, "command": r.command, "manifest_yaml": r.manifest_yaml,
    })
}

/// `goat admin role <subcmd>` — login-access changes on an existing identity via
/// the console machine API. Mirrors the console user-admin: server-side the same
/// last-admin guard, operator audit, and role-change email run; the CLI also
/// records its own OperatorActionV1, matching how `sponsorship` attributes.
async fn run_role(
    profile: &crate::profile::Profile,
    portal: &panoscope_core::portal::PortalClient,
    op: AdminRoleCmd,
) -> anyhow::Result<()> {
    match op {
        AdminRoleCmd::Set {
            email,
            groups,
            revoke_sessions,
            json,
        } => {
            let groups = clean_group_list(groups);
            if groups.iter().any(|g| {
                let u = g.to_ascii_uppercase();
                u == "PLATFORM-ADMIN" || u == "ADMINS" || u == "IAM-ADMIN"
            }) {
                eprintln!(
                    "⚠  Setting an escalated admin group for {email}. This grants admin-console \
                     access; the change is attributed to you in the audit."
                );
            }
            let res = portal
                .patch_user_groups(&email, &groups, revoke_sessions)
                .await
                .map_err(|e| super::vendor::map_onboard_err(portal, e));
            emit_vendor_audit(profile, "user.role.set", &email, res.is_ok()).await;
            let out = res?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": out.id, "email": out.email, "groups": out.groups,
                    }))?
                );
            } else {
                println!(
                    "{}: login groups set to [{}] — role-change email queued to the console mailer",
                    out.email,
                    out.groups.join(", ")
                );
            }
            Ok(())
        }
        AdminRoleCmd::Reactivate {
            email,
            reinstate,
            reason,
            env,
            json,
        } => {
            if reinstate {
                run_role_reinstate(
                    profile,
                    portal,
                    &email,
                    reason.as_deref().unwrap_or(""),
                    env.as_deref(),
                    json,
                )
                .await
            } else {
                run_role_state(profile, portal, &email, "active", json).await
            }
        }
        AdminRoleCmd::Deactivate { email, json } => {
            run_role_state(profile, portal, &email, "inactive", json).await
        }
    }
}

fn reinstate_remote_command(id: &str, actor: &str, reason: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        valid_revoke_identity(id),
        "invalid reinstatement identity UUID"
    );
    anyhow::ensure!(
        !actor.trim().is_empty() && actor.len() <= 256 && !actor.contains(['\r', '\n']),
        "invalid reinstatement actor"
    );
    anyhow::ensure!(
        !reason.trim().is_empty() && reason.len() <= 512 && !reason.contains(['\r', '\n']),
        "a single-line reinstatement reason of at most 512 bytes is required"
    );
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
    Ok(format!(
        "sudo -n /usr/local/libexec/goat-reinstate-identity --identity {} --actor {} --reason {}",
        quote(id),
        quote(actor),
        quote(reason)
    ))
}

async fn run_role_reinstate(
    profile: &crate::profile::Profile,
    portal: &panoscope_core::portal::PortalClient,
    email: &str,
    reason: &str,
    env: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let env = env
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("explicit reinstatement requires --env"))?;
    let cfg = native_ory_config_for(Some(env), profile)
        .ok_or_else(|| anyhow::anyhow!("no native Ory management endpoint for this environment"))?;
    // The console authenticates the operator and resolves the exact tenant UUID
    // before any privileged ledger write. A participant cannot trigger recovery.
    let (id, actor) = portal
        .check_identity_reinstatement(email, reason, env)
        .await
        .map_err(|e| super::vendor::map_onboard_err(portal, e))?;
    let remote = reinstate_remote_command(&id, &actor, reason)?;
    emit_vendor_audit(profile, "identity.reinstate.started", &id, true).await;
    let args = ssh_kratos_argv(&cfg, &remote);
    let output = Command::new("ssh")
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        emit_vendor_audit(profile, "identity.reinstate", &id, false).await;
        anyhow::bail!(
            "privileged reinstatement failed; identity activation not attempted: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    anyhow::ensure!(
        receipt.get("id").and_then(|v| v.as_str()) == Some(id.as_str())
            && receipt.get("actor").and_then(|v| v.as_str()) == Some(actor.as_str())
            && receipt.get("reinstated").and_then(|v| v.as_bool()) == Some(true),
        "reinstatement receipt did not match the authorized request"
    );
    let result = portal.activate_reinstated_identity(&id).await;
    emit_vendor_audit(profile, "identity.reinstate", &id, result.is_ok()).await;
    let out = result.map_err(|e| super::vendor::map_onboard_err(portal, e))?;
    anyhow::ensure!(
        out.id == id && out.state == "active",
        "reactivation response did not match the authorized identity"
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"id":out.id,"email":out.email,"state":out.state})
            )?
        );
    } else {
        println!(
            "{}: explicitly reinstated; state verified active",
            out.email
        );
    }
    Ok(())
}

async fn run_role_state(
    profile: &crate::profile::Profile,
    portal: &panoscope_core::portal::PortalClient,
    email: &str,
    state: &str,
    json: bool,
) -> anyhow::Result<()> {
    let res = portal
        .patch_user_state(email, state)
        .await
        .map_err(|e| super::vendor::map_onboard_err(portal, e));
    emit_vendor_audit(profile, "user.state.set", email, res.is_ok()).await;
    let out = res?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": out.id, "email": out.email, "state": out.state,
            }))?
        );
    } else {
        println!("{}: state set to {}", out.email, out.state);
    }
    Ok(())
}

/// Trim + drop empties from a --groups value (so `--groups ""` clears cleanly).
fn clean_group_list(gs: Vec<String>) -> Vec<String> {
    gs.into_iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect()
}

/// Local pre-flight on a key file, run BEFORE anything leaves the machine.
///
/// The server validates too, and its parser is the real security boundary. This
/// exists because a private key should never travel AT ALL — not over the wire,
/// not into a server log, not into an error message quoting what was rejected.
/// Catching it here means the bytes stay on the operator's disk.
fn vet_pubkey_before_send(contents: &str) -> Result<(), String> {
    let t = contents.trim();
    if t.is_empty() {
        return Err("file is empty".into());
    }
    if t.contains("PRIVATE KEY") {
        return Err(
            "this looks like a PRIVATE key — it contains 'PRIVATE KEY'. Send the \
             PUBLIC half instead: the matching `.pub` file (one line, starting \
             `ssh-ed25519`). Nothing was sent."
                .into(),
        );
    }
    if t.lines().count() > 1 {
        return Err(
            "a public key is ONE line; this file has several. Private keys are \
             multi-line, so check you picked the `.pub` file. Nothing was sent."
                .into(),
        );
    }
    if !t.starts_with("ssh-") && !t.starts_with("ecdsa-") {
        return Err(
            "does not start with a key type (`ssh-ed25519`, `ssh-rsa`, `ecdsa-…`). \
             Nothing was sent."
                .into(),
        );
    }
    Ok(())
}

async fn run_ssh_key(
    portal: &panoscope_core::portal::PortalClient,
    action: SshKeyAction,
) -> anyhow::Result<()> {
    match action {
        SshKeyAction::Ls { status, env, json } => {
            let want = if status.eq_ignore_ascii_case("all") {
                "all"
            } else {
                &status
            };
            let mut rows = portal
                .list_ssh_keys(want)
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, e.into()))?;
            if let Some(e) = env.as_deref() {
                rows.retain(|k| k.env.eq_ignore_ascii_case(e));
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &rows
                            .iter()
                            .map(|k| serde_json::json!({
                                "id": k.id, "handle": k.handle, "email": k.email,
                                "env": k.env, "key_type": k.key_type,
                                "fingerprint": k.fingerprint, "status": k.status,
                                "decided_by": k.decided_by, "expires_at": k.expires_at,
                            }))
                            .collect::<Vec<_>>()
                    )?
                );
                return Ok(());
            }
            if rows.is_empty() {
                println!("no ssh keys ({status} filter)");
                return Ok(());
            }
            println!(
                "{:<22}  {:<18}  {:<10}  {:<12}  {:<50}",
                "ID", "HANDLE", "ENV", "STATUS", "FINGERPRINT"
            );
            for k in &rows {
                println!(
                    "{:<22}  {:<18}  {:<10}  {:<12}  {:<50}",
                    trunc(&k.id, 22),
                    trunc(&k.handle, 18),
                    trunc(&k.env, 10),
                    trunc(&k.status, 12),
                    trunc(&k.fingerprint, 50),
                );
            }
            // Say what the states MEAN, because the difference between the last
            // two is the difference between believing you have a shell and
            // having one.
            eprintln!(
                "\n{} shown. submitted = waiting on a decision, grants nothing. \
                 approved = decided, NOT yet on any host. installed = the key is \
                 in authorized_keys and the shell works.",
                rows.len()
            );
            Ok(())
        }

        SshKeyAction::Submit {
            handle,
            pubkey_file,
            env,
            note,
            email,
        } => {
            let raw = std::fs::read_to_string(&pubkey_file)
                .map_err(|e| anyhow::anyhow!("cannot read {pubkey_file}: {e}"))?;
            let pubkey = raw.trim();
            if let Err(why) = vet_pubkey_before_send(pubkey) {
                anyhow::bail!("{pubkey_file}: {why}");
            }
            // Trim + shape-check before the row is minted: the installer's
            // per-row env gate compares byte-for-byte, so a stray space in
            // --env would create a row that is skipped env_mismatch on every
            // run forever while looking fine in the console.
            let envname = env.map(|e| e.trim().to_string()).unwrap_or_default();
            if !envname.is_empty() && !is_valid_env(&envname) {
                anyhow::bail!(
                    "invalid --env `{envname}`: must be a kebab-case goatnet id, \
                     2–32 chars (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `zagros-prod`)"
                );
            }
            let email = email.map(|e| e.trim().to_string()).unwrap_or_default();
            let k = portal
                .submit_ssh_key(&handle, pubkey, &envname, &note, &email)
                .await?;
            println!("recorded {} for {} ({})", k.fingerprint, k.handle, k.status);
            eprintln!(
                "This grants no access yet. Approve it with \
                 `goat admin ssh-key approve {}`, then run the installer.",
                k.id
            );
            Ok(())
        }

        SshKeyAction::Approve { id } => match portal.decide_ssh_key(&id, "approve").await {
            Ok(k) => {
                println!("approved {} for {}", k.fingerprint, k.handle);
                eprintln!(
                    "NOT installed yet — the installer places it on the hosts \
                         {}'s role may reach, then marks it installed. Until then \
                         they still have no shell.",
                    k.handle
                );
                Ok(())
            }
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                anyhow::bail!("cannot approve {id}: it is no longer awaiting a decision (already decided, or revoked).");
            }
            Err(e) => Err(e.into()),
        },

        SshKeyAction::Reject { id } => match portal.decide_ssh_key(&id, "reject").await {
            Ok(k) => {
                println!("rejected {} for {}", k.fingerprint, k.handle);
                Ok(())
            }
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                anyhow::bail!("cannot reject {id}: it is no longer awaiting a decision.");
            }
            Err(e) => Err(e.into()),
        },

        SshKeyAction::Revoke { id } => match portal.decide_ssh_key(&id, "revoke").await {
            Ok(k) => {
                println!("revoked {} for {}", k.fingerprint, k.handle);
                eprintln!(
                    "The console row is revoked. The key is removed from \
                         authorized_keys by the offboarding cut \
                         (operator-onboard.yml Phase R1.6) — run it to make this \
                         true on the hosts."
                );
                Ok(())
            }
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                anyhow::bail!("cannot revoke {id}: it is already rejected or revoked.");
            }
            Err(e) => Err(e.into()),
        },
        // Dispatched by run() before the portal client is built (install needs
        // no PortalClient), same pattern as the AdminCmd::* arms above it.
        SshKeyAction::Install { .. } => unreachable!(),
    }
}

/// Refuse an env-keyed admin verb whose `--env` is not the goatnet the active
/// profile belongs to. The authoritative binding is the profile.toml `goatnet`
/// key — a fact about which goatnet's console this profile's onboard_endpoint
/// belongs to. The context NAME is only a naming convention, so it serves as a
/// fallback for profiles that predate the key, never as an override of it: a
/// context named `zagros` whose profile is pinned to another goatnet must still
/// be refused.
pub(crate) fn require_env_binding(
    profile: &crate::profile::Profile,
    env: &str,
    verb: &str,
) -> anyhow::Result<()> {
    match profile.profile_field("goatnet").as_deref().map(str::trim) {
        Some(g) if g == env => Ok(()),
        Some(g) => {
            anyhow::bail!(
                "{verb} is env-keyed: the active profile ({}) is pinned to \
                 goatnet `{g}` but --env targets `{env}`. Switch to a profile for `{env}` \
                 (`goat --context <name> …`) — never re-point this one.",
                profile.profile_toml_path().display(),
            );
        }
        None if profile.context == env => Ok(()),
        None => {
            anyhow::bail!(
                "{verb} is env-keyed and the active profile carries no `goatnet` \
                 binding. Current context is `{}` but --env targets `{env}`. Either switch to \
                 the context for `{env}` (`goat --context {env} …` / GOAT_CONTEXT), or — if this \
                 context's onboard_endpoint really is `{env}`'s console — add `goatnet = \"{env}\"` \
                 to {} so the binding is a recorded fact instead of a naming convention.",
                profile.context,
                profile.profile_toml_path().display(),
            );
        }
    }
}

/// Env-keyed `goat admin ssh-key install` implementation shared by the CLI
/// admin dispatcher and the ssh-key subcommand handler. Resolves the console
/// endpoint and per-env operator API key keyed by `--env`, then wraps the
/// Ansible installer playbook with preview/confirm semantics.
// spawn_blocking allow: admin path, one interactive confirmation prompt —
// the blocking stdin read must race Ctrl-C (see the select! below), and the
// hot-path ban in clippy.toml sanctions exactly this reviewed use.
#[allow(clippy::disallowed_methods)]
async fn run_ssh_key_install(
    profile: &crate::profile::Profile,
    env: String,
    check: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let env = env.trim().to_string();
    if !is_valid_env(&env) {
        anyhow::bail!(
            "invalid --env `{env}`: must be a kebab-case goatnet id, \
             2–32 chars (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `zagros-prod`)"
        );
    }

    require_env_binding(profile, &env, "ssh-key install")?;

    // Console endpoint for this goatnet, resolved from the per-context
    // profile.toml. GOAT_ONBOARD_ENDPOINT is deliberately ignored here so a
    // process-wide override cannot re-point ssh-key installs to another
    // goatnet's console.
    let mut base = profile
        .profile_field("onboard_endpoint")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ssh-key install needs an `onboard_endpoint` for goatnet `{env}` \
                 in the active profile ({}). Run `goat profile init <bundle.cbor>` \
                 for this goatnet, or add an onboard_endpoint line to profile.toml.",
                profile.profile_toml_path().display(),
            )
        })?;
    base = base.trim().to_string();
    if base.is_empty() {
        anyhow::bail!(
            "ssh-key install resolved an empty onboard_endpoint for `{env}` — \
             check the onboard_endpoint value in profile.toml."
        );
    }
    if base.chars().any(char::is_whitespace) {
        anyhow::bail!(
            "ssh-key install refuses an onboard_endpoint with whitespace (`{base}`) — \
             fix the onboard_endpoint value in profile.toml for goatnet `{env}`."
        );
    }
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        anyhow::bail!(
            "ssh-key install needs an http(s) console endpoint, got `{base}` — \
             set onboard_endpoint = \"https://onboard.<site>.<zone>\" in profile.toml for `{env}`."
        );
    }

    // Per-env personal onboard API key. Env name is derived from the goatnet
    // slug (acme → ACME_PORTAL_OPERATOR_API_KEY, acme-prod →
    // ACME_PROD_PORTAL_OPERATOR_API_KEY) and MUST be set in the operator's
    // environment (e.g. .envrc.local.d) — it is a per-person secret, never
    // read from the profile.
    let env_prefix = env.to_uppercase().replace('-', "_");
    let key_var = format!("{}_PORTAL_OPERATOR_API_KEY", env_prefix);
    let api_key = std::env::var(&key_var)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ssh-key install needs your PERSONAL onboard API key for goatnet `{env}` — \
                 export ${} in your operator env (for example in .envrc.local.d/30-site-services.envrc) \
                 after collecting it via `goat admin operator-key collect`.",
                key_var,
            )
        })?;

    if check {
        println!(
            "Preview-only: installing approved host-shell keys on `{env}` via {} (check mode)",
            base
        );
        ssh_key_install_playbook(profile, &env, &base, &api_key, true).await?;
        return Ok(());
    }

    println!(
        "Preview: goatnet `{env}` via console {} — approved keys only (installer check run follows)",
        base
    );
    ssh_key_install_playbook(profile, &env, &base, &api_key, true).await?;

    if !yes {
        use std::io::{IsTerminal, Write};
        // Piped/automated invocations must fail fast, not block forever on a
        // prompt nobody will answer. --yes is the deliberate way to skip it.
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "stdin is not a terminal, so the install confirmation cannot be asked — \
                 pass --yes for a non-interactive install, or --check for preview-only"
            );
        }
        eprintln!(
            "Note: the real run reads the approved queue again at execution time — \
             keys approved after this preview are included."
        );
        eprint!("Install these approved keys on `{env}`? [y/N] ");
        std::io::stderr().flush().ok();
        // Race the blocking stdin read against Ctrl-C. The preview playbook
        // run leaves tokio's process-global SIGINT handler installed (its
        // ctrl_c task outlives the run), so the raw read_line would otherwise
        // transparently restart across the consumed signal and the one moment
        // the flow exists for aborting would be the one moment Ctrl-C is inert.
        let answer = tokio::select! {
            r = tokio::task::spawn_blocking(|| {
                let mut s = String::new();
                std::io::stdin().read_line(&mut s).map(|_| s)
            }) => r??,
            _ = tokio::signal::ctrl_c() => {
                anyhow::bail!("aborted (Ctrl-C at the confirmation prompt; nothing installed)");
            }
        };
        if !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("aborted (no confirmation)");
        }
    }

    ssh_key_install_playbook(profile, &env, &base, &api_key, false).await?;
    Ok(())
}

async fn run_invites(
    portal: &panoscope_core::portal::PortalClient,
    action: InviteAction,
) -> anyhow::Result<()> {
    match action {
        InviteAction::Ls { status, env, json } => {
            let want = if status.eq_ignore_ascii_case("all") {
                ""
            } else {
                &status
            };
            let mut rows = portal
                .list_invites(want)
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, e.into()))?;
            if let Some(e) = env.as_deref() {
                rows.retain(|i| i.env.eq_ignore_ascii_case(e));
            }
            if json {
                let arr: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "id": i.id, "sponsor": i.sponsor, "env": i.env,
                            "status": i.status, "tier": i.tier(),
                            "expires_at": i.expires_at, "escalated": i.escalated,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&arr)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("no invites ({} filter)", status);
                return Ok(());
            }
            println!(
                "{:<24}  {:<18}  {:<6}  {:<10}  {:<14}  FLAGS",
                "ID", "SPONSOR", "ENV", "STATUS", "TIER"
            );
            for i in &rows {
                let mut flags = Vec::new();
                if i.escalated {
                    flags.push("ESCALATED");
                }
                println!(
                    "{:<24}  {:<18}  {:<6}  {:<10}  {:<14}  {}",
                    trunc(&i.id, 24),
                    trunc(&i.sponsor, 18),
                    trunc(&i.env, 6),
                    trunc(&i.status, 10),
                    trunc(&i.tier(), 14),
                    flags.join(" "),
                );
            }
            eprintln!(
                "\n{} shown. Revoke an open one: `goat admin invites revoke <id>`",
                rows.len()
            );
            Ok(())
        }
        InviteAction::Revoke { id } => match portal.revoke_invite(&id).await {
            Ok(()) => {
                println!("revoked invite {id}");
                Ok(())
            }
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                anyhow::bail!("cannot revoke {id}: the invite is no longer open (already consumed, expired, or revoked).");
            }
            Err(PortalError::Http { status, .. }) if status.as_u16() == 404 => {
                anyhow::bail!("invite {id} not found.");
            }
            Err(e) => Err(super::vendor::append_onboard_hint(portal, e.into())),
        },
    }
}

async fn run_peer(ctx: &super::Ctx, op: PeerCmd) -> anyhow::Result<()> {
    use panoscope_core::netbird::NbPeer;

    match op {
        PeerCmd::Ls { json, mgmt_url } => {
            let nb = ctx.profile.netbird_client(mgmt_url.as_deref())?;
            let mut peers = nb.list_peers().await.map_err(map_nb_err)?;
            peers.sort_by_key(|p| p.name.to_lowercase());
            if json {
                let rows: Vec<_> = peers
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "id": p.id, "name": p.name, "hostname": p.hostname,
                            "mesh_ip": p.ip, "connected": p.connected,
                            "last_seen": p.last_seen, "os": p.os,
                            "groups": p.group_names(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if peers.is_empty() {
                println!("no mesh peers");
                return Ok(());
            }
            println!(
                "{:<22}  {:<3}  {:<15}  {:<28}  GROUPS",
                "PEER ID", "UP", "MESH IP", "NAME"
            );
            for p in &peers {
                let up = if p.connected { "●" } else { "·" };
                println!(
                    "{:<22}  {:<3}  {:<15}  {:<28}  {}",
                    p.id,
                    up,
                    p.ip,
                    truncate(&p.name, 28),
                    p.group_names().join(",")
                );
            }
            Ok(())
        }
        PeerCmd::Cut {
            selector,
            yes,
            mgmt_url,
            dry_run,
        } => {
            let nb = ctx.profile.netbird_client(mgmt_url.as_deref())?;
            let peers = nb.list_peers().await.map_err(map_nb_err)?;
            let matches: Vec<&NbPeer> = peers.iter().filter(|p| p.matches(&selector)).collect();
            match matches.as_slice() {
                [] => anyhow::bail!(
                    "no mesh peer matches `{selector}` — `goat admin peer ls` shows the roster"
                ),
                [_, _, ..] => {
                    eprintln!(
                        "`{selector}` matches {} peers — be more specific:",
                        matches.len()
                    );
                    for p in &matches {
                        eprintln!("  {}  {}  {}", p.id, p.ip, p.name);
                    }
                    anyhow::bail!("ambiguous selector");
                }
                [p] => {
                    let p = *p;
                    println!(
                        "target: {} ({})  mesh_ip={}  connected={}  groups=[{}]",
                        p.name,
                        p.id,
                        p.ip,
                        p.connected,
                        p.group_names().join(",")
                    );
                    if dry_run {
                        println!(
                            "dry-run: DELETE <netbird>/api/peers/{} (not executed)",
                            p.id
                        );
                        return Ok(());
                    }
                    if !yes {
                        use std::io::Write;
                        eprint!(
                            "Eject peer `{}` from the mesh? Its WireGuard tunnel drops immediately. [y/N] ",
                            p.name
                        );
                        std::io::stderr().flush().ok();
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line)?;
                        if !matches!(line.trim(), "y" | "Y" | "yes") {
                            anyhow::bail!("aborted (no confirmation)");
                        }
                    }
                    nb.delete_peer(&p.id).await.map_err(map_nb_err)?;
                    println!("cut peer `{}` ({}) — mesh tunnel severed", p.name, p.id);
                    Ok(())
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// `goat admin pqc on|off` — intra-mesh PQC (Rosenpass) toggle (ADR 0602)
// ---------------------------------------------------------------------

/// The `netbird up` argv tail per PQC state.
///
/// `off` MUST pass `--enable-rosenpass=false`, NOT omit the flag: a bare
/// `netbird up` PERSISTS the peer's prior Rosenpass setting, so omitting it is
/// a silent no-op that leaves PQC on: QR stays `true` after a flagless re-up,
/// and `--enable-rosenpass=false` drives it to `false`. Permissive is only
/// meaningful while Rosenpass is on, so
/// `off` doesn't touch it.
const ROSENPASS_ON_FLAGS: &[&str] = &["--enable-rosenpass", "--rosenpass-permissive"];
const ROSENPASS_OFF_FLAGS: &[&str] = &["--enable-rosenpass=false"];

fn rosenpass_flags(state: PqcState) -> &'static [&'static str] {
    if state.is_on() {
        ROSENPASS_ON_FLAGS
    } else {
        ROSENPASS_OFF_FLAGS
    }
}

/// Overall budget for a local `netbird up`/`status` (up blocks while it
/// reconnects). Mirrors heal.rs's actuator cap.
const PQC_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Write the Zenoh kx-mode state file — the F-259 Tier 1 kill-switch's second
/// layer (docs/design/pqc-kill-switch.md). `on` → `pq`, `off` → `classical`;
/// new goat/zenoh processes latch the mode at their first TLS session build
/// (long-running surfaces pick it up on restart — the seam is the
/// process-default rustls provider, not per-session).
///
/// System path first; per-user fallback when the system path can't be
/// created. Refuses the fallback when a system file already EXISTS but can't
/// be updated: resolver precedence is system-then-user, so the stale system
/// value would keep winning and the flip would be a silent no-op.
fn write_zenoh_pqc_mode(state: PqcState) -> anyhow::Result<(std::path::PathBuf, bool)> {
    use panoscope_core::tls;
    let mode = if state.is_on() {
        tls::PqcMode::Pq
    } else {
        tls::PqcMode::Classical
    };
    let system = std::path::PathBuf::from(tls::PQC_MODE_SYSTEM_FILE);
    let payload = format!("{}\n", mode.label());
    let sys_attempt = (|| -> std::io::Result<()> {
        if let Some(dir) = system.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&system, &payload)
    })();
    match sys_attempt {
        Ok(()) => Ok((system, true)),
        Err(e) => {
            if system.exists() {
                anyhow::bail!(
                    "cannot update {} ({e}) and the file already exists — its value \
                     overrides any user-level fallback, so the zenoh flip would be a \
                     silent no-op. Re-run with sudo.",
                    system.display()
                );
            }
            let user = tls::pqc_mode_user_file().ok_or_else(|| {
                anyhow::anyhow!("no HOME dir for the user-level pqc-mode fallback")
            })?;
            if let Some(dir) = user.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&user, &payload)?;
            Ok((user, false))
        }
    }
}

/// Extract the `Quantum resistance:` line out of a `netbird status` dump —
/// NetBird's own report of whether Rosenpass is active on this peer (e.g.
/// `true (permissive)` / `false`). Pure so it's unit-testable.
fn quantum_line(status: &str) -> Option<String> {
    status
        .lines()
        .map(str::trim)
        .find(|l| l.to_lowercase().starts_with("quantum resistance:"))
        .map(|l| l.to_string())
}

/// The `goat admin pqc` CLI fields, bundled so the handlers stay under the
/// argument-count lint (and so the borrowed &str fields share one lifetime).
struct PqcArgs<'a> {
    state: PqcState,
    peer: Option<&'a str>,
    fleet: bool,
    ssh_user: &'a str,
    mgmt_url: Option<&'a str>,
    yes: bool,
    dry_run: bool,
    json: bool,
}

async fn run_pqc(ctx: &super::Ctx, args: &PqcArgs<'_>) -> anyhow::Result<()> {
    if args.fleet {
        return pqc_fleet_plan(ctx, args).await;
    }
    if let Some(sel) = args.peer {
        return pqc_remote(ctx, args, sel).await;
    }
    // `goat admin pqc` is the OTHER-peer surface (needs the NetBird API token +
    // SSH). Toggling THIS host is self-service — no admin scope — so it lives at
    // `goat client pqc` instead of behind the admin gate.
    anyhow::bail!(
        "`goat admin pqc` manages OTHER peers — pass `--peer <selector>` or `--fleet`. \
         To toggle PQC on THIS host, use `goat client pqc {}` (self-service, no admin token).",
        args.state.label()
    )
}

/// Flip PQC on THIS host at BOTH layers — the Zenoh kx-mode state file
/// then the netbird Rosenpass re-up — and read back `netbird
/// status` to report the verified state (never an assumed one). This is the
/// self-service entry point behind `goat client pqc` (no admin token — it
/// only reconfigures the local host, which the caller already controls).
///
/// The Zenoh file is written FIRST and a write failure aborts before netbird
/// is touched: a half-flip (rosenpass classical, zenoh still PQ) is worse
/// than failing fast.
///
/// `netbird up` NO-OPS on an already-connected peer ("Already connected") and
/// does NOT re-apply a changed `--enable-rosenpass`, so a toggle needs
/// **down → up**. This briefly
/// drops + re-establishes the tunnel; permissive means no ACL change and no
/// connectivity loss to already-enabled peers.
pub(crate) async fn pqc_local(
    state: PqcState,
    netbird_bin: &str,
    yes: bool,
    dry_run: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut up_args = vec!["up".to_string()];
    up_args.extend(rosenpass_flags(state).iter().map(|s| s.to_string()));
    let up_display = format!("{netbird_bin} {}", up_args.join(" "));
    let cmd_display = format!("{netbird_bin} down && {up_display}");
    let zenoh_mode = if state.is_on() { "pq" } else { "classical" };

    if dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "target": "local", "state": state.label(),
                    "command": cmd_display, "executed": false,
                    "zenoh_mode": zenoh_mode,
                    "zenoh_mode_file": panoscope_core::tls::PQC_MODE_SYSTEM_FILE,
                }))?
            );
        } else {
            println!(
                "dry-run: would write `{zenoh_mode}` to {} (user-level fallback if \
                 unwritable), then run `{cmd_display}` (not executed)",
                panoscope_core::tls::PQC_MODE_SYSTEM_FILE
            );
        }
        return Ok(());
    }

    if !yes && !json {
        use std::io::Write;
        eprint!(
            "Turn PQC {} on THIS host — zenoh kx-mode `{zenoh_mode}` + \
             `{cmd_display}`? netbird briefly drops + re-establishes the tunnel. [y/N] ",
            state.label()
        );
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            anyhow::bail!("aborted (no confirmation)");
        }
    }

    // Zenoh layer first — abort before netbird on failure (no half-flip).
    let (zenoh_file, zenoh_system) = write_zenoh_pqc_mode(state)?;
    if !json {
        println!(
            "zenoh: kx-mode `{zenoh_mode}` written to {}{}",
            zenoh_file.display(),
            if zenoh_system {
                ""
            } else {
                " (user-level fallback — daemons running as other users won't see it)"
            }
        );
        println!(
            "  new goat/zenoh processes latch it at their first TLS session; \
             restart long-running surfaces to flip them"
        );
    }

    // down first (a bare `up` would no-op on a connected peer); "not connected"
    // is fine, so we don't gate on the down result — the up is what matters.
    let down = run_netbird(netbird_bin, &["down".to_string()]).await;
    let up = run_netbird(netbird_bin, &up_args).await;
    if !json {
        println!("{cmd_display}: {}", if up.0 { "ok" } else { "FAILED" });
        if !up.1.is_empty() {
            println!("  {}", first_line_of(&up.1));
        }
    }
    // Read-back verify — the actual, not-assumed state.
    let status = run_netbird(netbird_bin, &["status".to_string()]).await;
    let qr = quantum_line(&status.1);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "target": "local", "state": state.label(),
                "command": cmd_display, "executed": true,
                "zenoh_mode": zenoh_mode,
                "zenoh_mode_file": zenoh_file.display().to_string(),
                "zenoh_mode_system_scope": zenoh_system,
                "down_ok": down.0, "up_ok": up.0,
                "quantum_resistance": qr,
            }))?
        );
    } else {
        match &qr {
            Some(l) => println!("verified: {l}"),
            None => println!(
                "verified: netbird status did not report a Quantum resistance line \
                 (older client, or run `{netbird_bin} status` manually)"
            ),
        }
        if !up.0 {
            println!(
                "note: `{up_display}` did not exit 0 — if it's a permission error, \
                 re-run with sudo (`sudo {up_display}`)."
            );
        }
    }
    if up.0 {
        Ok(())
    } else {
        anyhow::bail!("netbird up did not succeed")
    }
}

/// Flip PQC on a remote mesh peer over SSH, both layers: the Zenoh kx-mode
/// state file then rosenpass. Resolves the selector against the
/// NetBird peer list (same matcher as `peer cut`), then SSHes to the peer's
/// mesh IP and launches a **detached** file-write + `down → up`.
///
/// Two hard constraints drive the design: (1) `netbird up` no-ops on a
/// connected peer, so the
/// toggle needs `down` first; (2) `netbird down` drops `wt0` — the very mesh
/// tunnel this SSH rides — so the sequence must run detached (`nohup … &`) to
/// survive the session drop. We therefore can't read the Quantum-resistance
/// line back in the same round-trip; the peer's (often relayed) link re-forms
/// asynchronously, so we report "issued" + how to verify, never an assumed
/// success.
async fn pqc_remote(ctx: &super::Ctx, args: &PqcArgs<'_>, selector: &str) -> anyhow::Result<()> {
    let PqcArgs {
        state,
        ssh_user,
        mgmt_url,
        yes,
        dry_run,
        json,
        ..
    } = *args;
    let nb = ctx.profile.netbird_client(mgmt_url)?;
    let peers = nb.list_peers().await.map_err(map_nb_err)?;
    let matches: Vec<_> = peers.iter().filter(|p| p.matches(selector)).collect();
    let p = match matches.as_slice() {
        [] => anyhow::bail!(
            "no mesh peer matches `{selector}` — `goat admin peer ls` shows the roster"
        ),
        [_, _, ..] => {
            eprintln!(
                "`{selector}` matches {} peers — be more specific:",
                matches.len()
            );
            for p in &matches {
                eprintln!("  {}  {}  {}", p.id, p.ip, p.name);
            }
            anyhow::bail!("ambiguous selector");
        }
        [p] => *p,
    };

    let flags = format!(" {}", rosenpass_flags(state).join(" "));
    let dest = format!("{ssh_user}@{}", p.ip);
    let zmode = if state.is_on() { "pq" } else { "classical" };
    let zfile = panoscope_core::tls::PQC_MODE_SYSTEM_FILE;
    // Detached zenoh-file write + down → up. The zenoh kx-mode file goes first
    // then each netbird
    // call tries NOPASSWD sudo (the mgmt-host baseline) then falls back to
    // non-sudo. The status read-back is written to a file on the peer for the
    // operator to inspect once the link re-forms.
    // NOTE: `seq` is embedded in DOUBLE quotes by `remote` below, so every
    // expansion that must happen when the detached sequence RUNS (not when the
    // remote login shell parses the launch line) is escaped as `\$`.
    let seq = format!(
        "(sudo -n mkdir -p /var/lib/goat 2>/dev/null && \
          echo {zmode} | sudo -n tee {zfile} >/dev/null) || \
         (mkdir -p \\$HOME/.config/goat && echo {zmode} > \\$HOME/.config/goat/pqc-mode); \
         (sudo -n netbird down 2>/dev/null || netbird down); sleep 3; \
         (sudo -n netbird up{flags} 2>/dev/null || netbird up{flags}); \
         (sudo -n netbird status 2>/dev/null || netbird status 2>/dev/null) \
         | grep -i 'quantum resistance' > /tmp/goat-pqc-verify.log 2>&1; \
         echo zenoh: \\$(cat {zfile} 2>/dev/null || cat \\$HOME/.config/goat/pqc-mode \
         2>/dev/null || echo unknown) >> /tmp/goat-pqc-verify.log"
    );
    let remote = format!("nohup sh -c \"{seq}\" >/tmp/goat-pqc.log 2>&1 & echo launched");

    if dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "target": p.name, "peer_id": p.id, "mesh_ip": p.ip,
                    "state": state.label(), "ssh": dest,
                    "remote_command": format!(
                        "echo {zmode} > {zfile} && netbird down && netbird up{flags}"
                    ),
                    "executed": false,
                }))?
            );
        } else {
            println!(
                "dry-run: ssh {dest} -> detached `echo {zmode} > {zfile} && \
                 netbird down && netbird up{flags}` on `{}` ({}) (not executed)",
                p.name, p.id
            );
        }
        return Ok(());
    }

    if !yes && !json {
        use std::io::Write;
        eprint!(
            "Turn PQC {} on peer `{}` ({}) over ssh {dest}? It drops + re-establishes \
             the peer's tunnel (its link may re-form slowly if relayed). [y/N] ",
            state.label(),
            p.name,
            p.ip
        );
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            anyhow::bail!("aborted (no confirmation)");
        }
    }

    if !json {
        eprintln!("ssh {dest}: launching detached netbird down && up{flags} ...");
    }
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "StrictHostKeyChecking=accept-new",
                &dest,
                &remote,
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let (launched, text) = match out {
        Err(_) => (false, "ssh hit the 30s budget before launching".to_string()),
        Ok(Err(e)) => (false, format!("could not spawn ssh: {e}")),
        Ok(Ok(o)) => {
            let mut s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if !err.is_empty() {
                if !s.is_empty() {
                    s.push_str(" | ");
                }
                s.push_str(&err);
            }
            (o.status.success() && s.contains("launched"), s)
        }
    };
    let verify_hint =
        format!("ssh {dest} 'cat /tmp/goat-pqc-verify.log' (or `netbird status` on the peer)");

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "target": p.name, "peer_id": p.id, "mesh_ip": p.ip,
                "state": state.label(), "ssh": dest, "launched": launched,
                "verify": verify_hint, "output": text,
            }))?
        );
    } else if launched {
        println!(
            "peer `{}`: PQC {} issued (detached down → up). The peer's tunnel re-forms \
             asynchronously — verify with:\n  {verify_hint}",
            p.name,
            state.label()
        );
    } else {
        println!(
            "peer `{}`: could not launch — {}",
            p.name,
            first_line_of(&text)
        );
    }
    if launched {
        Ok(())
    } else {
        anyhow::bail!("remote down/up did not launch")
    }
}

/// The honest fleet "sweep": PQC is a per-peer setting, so this prints the
/// exact command to run on each mesh peer rather than fanning SSH into
/// heterogeneous peers (laptops/Orins/RPis that won't all accept the same SSH).
/// The DURABLE fleet path is the `rosenpass_enabled` ansible group_var +
/// re-onboard; this is the operability helper for reachable peers.
async fn pqc_fleet_plan(ctx: &super::Ctx, args: &PqcArgs<'_>) -> anyhow::Result<()> {
    let PqcArgs {
        state,
        mgmt_url,
        json,
        ..
    } = *args;
    let nb = ctx.profile.netbird_client(mgmt_url)?;
    let mut peers = nb.list_peers().await.map_err(map_nb_err)?;
    peers.sort_by_key(|p| p.name.to_lowercase());

    if json {
        let rows: Vec<_> = peers
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name, "peer_id": p.id, "mesh_ip": p.ip,
                    "connected": p.connected,
                    "command": format!("goat admin pqc {} --peer {}", state.label(), p.id),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "state": state.label(), "peer_count": peers.len(), "peers": rows,
            }))?
        );
        return Ok(());
    }

    println!(
        "PQC {} — fleet plan ({} peers). Each peer holds its OWN Rosenpass setting, so a",
        state.label(),
        peers.len()
    );
    println!("fleet flip is a per-peer re-up, NOT an instant switch. Run per reachable peer:");
    println!();
    for p in &peers {
        let up = if p.connected { "●" } else { "·" };
        println!(
            "  {up} goat admin pqc {} --peer {}   # {}",
            state.label(),
            p.id,
            p.name
        );
    }
    println!();
    println!(
        "Each command flips BOTH layers on the peer: the zenoh kx-mode file (new \
         processes latch it; restart long-running surfaces) and rosenpass. \
         Durable fleet path: set `rosenpass_enabled: true` in the goatnet's ansible group_var \
         and re-onboard (every new peer is then PQC by construction). \
         See docs/design/rosenpass-intra-mesh-pqc.md + docs/design/pqc-kill-switch.md."
    );
    Ok(())
}

/// Run the local netbird CLI with a timeout; returns (success, combined
/// stdout+stderr). Mirrors heal.rs's actuator command handling.
async fn run_netbird(bin: &str, args: &[String]) -> (bool, String) {
    match tokio::time::timeout(
        PQC_CMD_TIMEOUT,
        tokio::process::Command::new(bin).args(args).output(),
    )
    .await
    {
        Err(_) => (
            false,
            format!("timed out after {}s", PQC_CMD_TIMEOUT.as_secs()),
        ),
        Ok(Err(e)) => (false, format!("failed to run `{bin}`: {e}")),
        Ok(Ok(out)) => {
            let mut detail = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !err.is_empty() {
                if !detail.is_empty() {
                    detail.push_str(" | ");
                }
                detail.push_str(&err);
            }
            (out.status.success(), detail)
        }
    }
}

fn first_line_of(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// `goat admin rotate <anchor|alias>` — re-issue a peer's cert in place.
/// Resolves a slot alias to its anchor (fail-closed against a known alias
/// set, per ADR 1039), then POSTs the portal rotate endpoint and prints
/// the fresh one-time bundle URL. The anchor / alias / write namespace /
/// ACL are unchanged; only the leaf cert + portal api_key are new.
async fn rotate(ctx: &super::Ctx, selector: &str, dry_run: bool) -> anyhow::Result<()> {
    // Resolve alias→anchor up front so the operator can type either. An
    // anchor-shaped token passes through; an unknown alias against a known
    // set fails closed (no phantom prefix — the F-177 trap).
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let anchor = reg
        .resolve_input(selector.trim())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let alias_note = reg
        .alias_of(&anchor)
        .map(|a| format!(" (alias `{a}`)"))
        .unwrap_or_default();

    if dry_run {
        eprintln!("dry-run: POST <portal>/api/orgs/{anchor}/rotate{alias_note}");
        return Ok(());
    }

    let portal = ctx.profile.portal_client()?;
    rotate_one(&portal, &anchor, &alias_note).await?;
    eprintln!(
        "Identity unchanged — same anchor, alias, write namespace, and ACL; only the leaf \
         cert + api_key are new. Fetch the bundle (one-time, 24 h) from a box on the goatnet's \
         mesh — the portal host must resolve — then swap the cert+key on the peer and restart it."
    );
    Ok(())
}

/// Absolutize the portal-relative `bundle_url` for display (#1196). `base` MUST
/// be `PortalClient::base_url()` — the origin the rotate call was made against,
/// never a host derived from the anchor/org (cross-org wrong-host bug).
fn full_bundle_url(base: Option<&str>, bundle_url: &str) -> String {
    if bundle_url.starts_with("http://") || bundle_url.starts_with("https://") {
        return bundle_url.to_string();
    }
    match base {
        Some(b) => format!(
            "{}/{}",
            b.trim_end_matches('/'),
            bundle_url.trim_start_matches('/')
        ),
        None => bundle_url.to_string(),
    }
}

/// POST one anchor's in-place rotation and print the fresh bundle. The shared
/// core of single-target `rotate` and the `rotate --org` cascade.
async fn rotate_one(
    portal: &panoscope_core::portal::PortalClient,
    anchor: &str,
    alias_note: &str,
) -> anyhow::Result<()> {
    let res = match portal.rotate_vendor(anchor).await {
        Ok(r) => r,
        Err(PortalError::NotFound { .. }) => anyhow::bail!(
            "portal has no peer `{anchor}`{alias_note} — rotation re-issues an existing \
             cert in place; onboard a new one with `goat admin onboard` instead"
        ),
        Err(e) => return Err(super::vendor::map_portal_err(e)),
    };
    println!("rotated cert for `{}`{alias_note}", res.prefix);
    println!("  uri_san:    {}", res.uri_san);
    println!(
        "  bundle_url: {}",
        full_bundle_url(portal.base_url(), &res.bundle_url)
    );
    println!("  api_key:    {}", res.api_key);
    Ok(())
}

/// `goat admin rotate <anchor> --key-only` — reissue ONLY the portal key
/// (goat#1193). The lighter sibling of `rotate`: swaps the portal key in place
/// and leaves the mTLS cert untouched, so there is no bundle to fetch and no
/// peer restart. For the recurring "lost/stale portal key, cert is fine, still
/// publishing" case. Resolves alias→anchor up front, same as `rotate`.
async fn rekey(ctx: &super::Ctx, selector: &str, dry_run: bool) -> anyhow::Result<()> {
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let anchor = reg
        .resolve_input(selector.trim())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let alias_note = reg
        .alias_of(&anchor)
        .map(|a| format!(" (alias `{a}`)"))
        .unwrap_or_default();

    if dry_run {
        eprintln!("dry-run: POST <portal>/api/orgs/{anchor}/rekey{alias_note}");
        return Ok(());
    }

    let portal = ctx.profile.portal_client()?;
    let key = rekey_one(&portal, &anchor, &alias_note).await?;
    println!("reissued portal key for `{anchor}`{alias_note}");
    println!("  api_key: {key}");
    eprintln!(
        "Key-only reissue — the mTLS cert, anchor, alias, write namespace, and ACL are \
         unchanged. Hand over the new api_key; nothing to download and no peer restart \
         (the cert was not touched, so publishing is unaffected). The peer's existing fabric \
         bundle still carries the OLD key — paste this one into the profile rather than \
         re-running `goat profile init` from that bundle."
    );
    Ok(())
}

/// POST one anchor's key-only reissue and return the fresh portal key. The
/// shared, unit-testable seam behind `rekey` (mirrors `rotate_one`).
async fn rekey_one(
    portal: &panoscope_core::portal::PortalClient,
    anchor: &str,
    alias_note: &str,
) -> anyhow::Result<String> {
    match portal.rekey_vendor(anchor).await {
        Ok(r) => Ok(r.api_key),
        Err(PortalError::NotFound { .. }) => anyhow::bail!(
            "portal has no peer `{anchor}`{alias_note} — rekey reissues an existing peer's \
             portal key; onboard a new one with `goat admin onboard` instead, or this portal \
             predates rekey (upgrade the portal image)"
        ),
        Err(e) => Err(super::vendor::map_portal_err(e)),
    }
}

/// Map each org member to its slot anchor via the alias map, keyed on the
/// member's `vendor_tag`. This is the AUTHORITATIVE join, not a guess: a slot
/// is onboarded with `prefix = zenoh.trial.vendor`, the portal sets the slot
/// alias to that prefix, and the onboard drift-check enforces
/// `zenoh.trial.vendor == vendor.short_tag` — which is exactly the
/// `vendor_tag` the roster carries. So `vendor_tag` IS the slot alias for
/// every portal-minted member.
///
/// We deliberately do NOT fall back to the member's `handle`: the handle is
/// not the slot alias, so a handle lookup would only ever "succeed" by
/// colliding with some OTHER member's vendor_tag alias — a mis-rotation. A
/// member with no `vendor_tag` (or whose slot isn't in the alias map — not
/// portal-minted, or revoked) resolves to `None` and is reported+skipped, not
/// guessed. Pure over the registry so it is unit-testable without a portal.
fn resolve_member_anchors<'a>(
    reg: &panoscope_core::alias::AliasRegistry,
    members: &'a [OrgMember],
) -> Vec<(&'a OrgMember, Option<String>)> {
    members
        .iter()
        .map(|m| {
            let anchor = m
                .vendor_tag
                .as_deref()
                .and_then(|t| reg.resolve_input(t).ok());
            (m, anchor)
        })
        .collect()
}

/// `goat admin rotate --org <id>` — the org-scope rotation cascade (the
/// rotate sibling of `onboard --org --revoke`, ADR 1044 / identity grammar).
/// Resolves the active affiliation members from the declared registry, maps
/// each to its slot anchor via the alias map keyed on the member's
/// `vendor_tag` (the authoritative slot-alias key — see
/// `resolve_member_anchors`), prints the blast radius, confirms (unless
/// `--yes`), then rotates EACH resolved anchor in place.
///
/// A member with no `vendor_tag`, or whose slot isn't in the alias map (not
/// portal-minted / revoked), resolves to nothing and is reported+skipped,
/// never silently dropped (the F-177 discipline) and never guessed via a
/// non-authoritative key. Sequential (ordered output), per-member failure is
/// non-fatal to the rest, exit is non-zero if any resolved member failed to
/// rotate or any member was unresolved.
async fn rotate_org(
    ctx: &super::Ctx,
    org_id: &str,
    repo_root: Option<&Path>,
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    let members = resolve_org_members(
        &repo_root,
        org_id,
        ctx.profile.goatnet_endpoints().goatnet.as_deref(),
    )?;
    if members.is_empty() {
        anyhow::bail!(
            "no active members found for org `{org_id}` in the registry — nothing to \
             rotate (is the affiliation.id correct? check docs/operators/roster.json)"
        );
    }

    // Map each member to its slot anchor via the alias map (vendor_tag first —
    // the cert reconciliation key — then handle). Unresolved members are kept
    // so the blast radius is honest about what will NOT be rotated.
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let resolved = resolve_member_anchors(&reg, &members);
    let resolvable = resolved.iter().filter(|(_, a)| a.is_some()).count();

    // Blast radius — always printed, even with --yes.
    eprintln!(
        "ORG ROTATE CASCADE — org `{org_id}` ({} active member(s), {resolvable} with a \
         resolvable anchor):",
        members.len()
    );
    for (m, anchor) in &resolved {
        match anchor {
            Some(a) => {
                let alias = reg
                    .alias_of(a)
                    .map(|x| format!(" alias `{x}`"))
                    .unwrap_or_default();
                eprintln!("  - {}  [{}] -> {a}{alias}", m.handle, m.kind);
            }
            None => eprintln!("  - {}  [{}] -> (no anchor — SKIP)", m.handle, m.kind),
        }
    }
    if resolvable == 0 {
        anyhow::bail!(
            "none of the {} member(s) of `{org_id}` resolved to a slot anchor — they have no \
             `vendor_tag`, or their slots aren't in the portal alias map (not portal-minted, \
             or revoked). Check the portal is reachable + the roster is current; rotate any \
             individually with `goat admin rotate <anchor|alias>`",
            members.len()
        );
    }
    eprintln!(
        "Each resolved member's leaf cert is re-issued in place (same anchor / alias / write \
         namespace / ACL; new leaf + api_key)."
    );

    if dry_run {
        eprintln!("dry-run: no rotation executed.");
        return Ok(());
    }

    if !yes {
        eprint!("Rotate the {resolvable} resolved member(s) of `{org_id}`? [y/N] ");
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            anyhow::bail!("aborted — no members rotated");
        }
    }

    let portal = ctx.profile.portal_client()?;
    let mut failures = 0usize;
    for (m, anchor) in &resolved {
        let Some(anchor) = anchor else {
            eprintln!(
                "=== SKIP {} (org `{org_id}`) — no resolvable anchor ===",
                m.handle
            );
            failures += 1;
            continue;
        };
        let alias_note = reg
            .alias_of(anchor)
            .map(|a| format!(" (alias `{a}`)"))
            .unwrap_or_default();
        eprintln!("=== rotating {} -> {anchor} (org `{org_id}`) ===", m.handle);
        if let Err(e) = rotate_one(&portal, anchor, &alias_note).await {
            eprintln!("  FAILED to rotate {}: {e}", m.handle);
            failures += 1;
        }
    }

    if failures > 0 {
        anyhow::bail!(
            "{failures} of {} member(s) of `{org_id}` were not rotated (unresolved or failed) \
             — see above; rotation is idempotent, so re-run after resolving",
            members.len()
        );
    }
    eprintln!(
        "org `{org_id}` rotate cascade complete — {resolvable} member(s) re-issued. Fetch each \
         one-time bundle (24 h) from a box on the goatnet's mesh — the portal host must resolve \
         — then swap the cert+key on the peer and restart it."
    );
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// True when the environment carries any signal that the caller is set up to
/// act as an operator: a per-person operator handle, the operator audit roster,
/// a NetBird admin token (generic or per-context), or offline-CA material. A
/// plain participant/vendor laptop has none of these. We only need the *env*
/// signals: the one case `ensure_admin_capability` refuses is an uninitialized
/// profile, where profile.toml (and any `operator_handle` in it) doesn't exist.
fn has_operator_signal(profile: &crate::profile::Profile) -> bool {
    let set = |k: &str| {
        std::env::var(k)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    };
    if set("GOAT_OPERATOR_HANDLE") || set("GOAT_OPERATOR_AGE_RECIPIENTS") {
        return true;
    }
    let ctx_env = profile.context.to_uppercase().replace('-', "_");
    if set("GOAT_NB_API_TOKEN") || set(&format!("{ctx_env}_NB_API_TOKEN")) {
        return true;
    }
    // Offline-CA material — only an operator minting device/client bundles.
    [
        "GOAT_DEV_CA_ID",
        "GOAT_DEV_CA_KEY",
        "GOAT_DEV_CA_AGE_IDENTITY",
    ]
    .iter()
    .any(|k| set(k))
}

/// Up-front, no-false-block guard for the operator-only `goat admin` surface.
///
/// `goat admin` is for operators. The CLI cannot *prove* admin authority
/// locally — the same portal token serves browse and admin, and authority is
/// decided server-side — so we deliberately do NOT guess-and-block a
/// configured profile here; that path flows to the portal/NetBird call and
/// surfaces a 403 as a clear `E_NOT_ADMIN`. The single case we *can* refuse
/// with certainty (no network round trip, zero risk of false-blocking a real
/// admin) is: the active profile isn't initialized AND no operator credential
/// is present in the environment — i.e. a non-operator typed `goat admin …`.
/// We fail that closed, inert, with a helpful message.
///
/// `lan` is exempt: it mutates nothing, needs no credentials, and is the
/// break-glass diagnostic meant to work when the mesh and every service are
/// down — refusing it on missing operator env would defeat its purpose.
/// The pure decision behind `ensure_admin_capability` (extracted so the
/// truth table is unit-testable without env/filesystem state). `true` = let
/// the command proceed to its authoritative check; `false` = refuse up front.
/// We proceed whenever it's `lan` (break-glass, exempt), the profile is
/// initialized (authority decided downstream), or any operator env signal is
/// present. Only the all-false row is refused — the certain non-operator case.
fn admin_may_proceed(is_lan: bool, profile_initialized: bool, has_operator_signal: bool) -> bool {
    is_lan || profile_initialized || has_operator_signal
}

fn ensure_admin_capability(
    profile: &crate::profile::Profile,
    op: &AdminCmd,
) -> Result<(), crate::errors::GoatError> {
    // Lan AND Storm are break-glass: they mutate nothing through the portal
    // (Storm's authority is the operator's SSH key to the mgmt host) and must
    // work when the mesh + every goatnet service is down — refusing either on a
    // missing operator profile would defeat the purpose during an outage.
    let is_break_glass = matches!(op, AdminCmd::Lan { .. } | AdminCmd::Storm { .. });
    if admin_may_proceed(
        is_break_glass,
        profile.is_initialized(),
        has_operator_signal(profile),
    ) {
        return Ok(());
    }
    // A denied (root-owned) profile is not a missing one: an operator whose
    // state dir needs privileges should hear E_PROFILE_DENIED, not "you have
    // no operator credentials".
    if let Err(e @ crate::errors::GoatError::ProfileDenied { .. }) = profile.require_initialized()
    {
        return Err(e);
    }
    Err(crate::errors::GoatError::NotAdmin {
        hint: format!(
            "`goat admin` is operator-only, and the active profile `{}` has no operator \
             credentials configured — no initialized profile and no operator environment. \
             Nothing was sent. If you are an operator, activate your operator profile (`goat \
             context use <name>` after `goat profile init`) and source your operator \
             environment (e.g. GOAT_OPERATOR_HANDLE, the NetBird admin token). If you are not, \
             this command isn't for your role — the everyday surface is `goat pub` / `sub` / \
             `top` / `who` / `health`. See the Administration chapter in the goat docs.",
            profile.context
        ),
    })
}

fn map_nb_err(e: panoscope_core::netbird::NetbirdError) -> anyhow::Error {
    use panoscope_core::netbird::NetbirdError as E;
    match e {
        E::Disabled => anyhow::anyhow!(
            "NetBird mgmt client not configured — set the admin token + mgmt URL \
             (see `goat admin peer --help`)"
        ),
        E::NotFound => anyhow::anyhow!("peer not found (already cut?)"),
        // 403 = the NetBird admin token authenticated but isn't authorized to
        // manage peers — the operator-only path, surfaced as E_NOT_ADMIN so a
        // non-admin gets the same clear "not for your role" answer the portal
        // path gives (and a peer `cut` 403 means nothing was severed). Other
        // HTTP codes stay generic mgmt-API faults (wrong scope / bad URL).
        E::Http { status } if status.as_u16() == 403 => crate::errors::GoatError::NotAdmin {
            hint: format!(
                "NetBird mgmt API denied this request (HTTP {status}). The admin token is valid \
                 but not authorized to manage peers — `goat admin peer` is operator-only. Nothing \
                 was changed. Use an admin-scoped NetBird token, or contact your goat administrator."
            ),
        }
        .into(),
        E::Http { status } => anyhow::anyhow!(
            "NetBird mgmt API returned HTTP {status} — token wrong scope, or URL incorrect"
        ),
        E::Transient(s) => anyhow::anyhow!("NetBird mgmt API unreachable: {s}"),
    }
}

/// Walk up from `start` looking for a directory containing `.git`.
/// Returns the first such directory (the repo root) or `None`.
/// Construct the argv passed to `ansible-preflight.sh operator-onboard.yml`.
/// Inventory shape per `ops/ansible/operator-onboard.yml` usage block:
/// `-i localhost, --connection local`. The playbook adds remote hosts at
/// runtime via add_host (Phase 2.0) so the operator never has to maintain
/// a real inventory file for this play. Lifted out of `onboard()` so
/// tests can lock the args list in without spawning ansible.
///
/// When `env` is `Some`, an extra `-e operator_env=<env>` token-pair is
/// appended so the playbook resolves the per-env manifest subfolder
/// (`ops/operators/<env>/<handle>.yml`) and the per-site maps for that
/// goatnet. Omitted entirely for the flat legacy layout.
fn build_onboard_ansible_args(handle: &str, env: Option<&str>, revoke: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-i".into(),
        "localhost,".into(),
        "--connection".into(),
        "local".into(),
        "operator-onboard.yml".into(),
        "-e".into(),
        format!("operator={handle}"),
    ];
    if let Some(env) = env {
        args.push("-e".into());
        args.push(format!("operator_env={env}"));
    }
    if revoke {
        args.push("-e".into());
        args.push("action=revoke".into());
    }
    args
}

/// Construct the argv for `ansible-preflight.sh operator-ssh-key-install.yml` —
/// the host-shell installer. Mirrors the onboard wrapper shape (local
/// connection, playbook name first, extra-vars for env + console URL) and
/// deliberately **does not** carry the API key on argv: the installer reads it
/// from the `ONBOARD_API_KEY` environment variable (or a provided
/// `onboard_api_key` extra-var), so the key never appears in `ps` output.
fn build_ssh_key_install_args(
    env: &str,
    check: bool,
    api_base: &str,
    repo_root: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-i".into(),
        "localhost,".into(),
        "--connection".into(),
        "local".into(),
        "operator-ssh-key-install.yml".into(),
        "-e".into(),
        format!("onboard_env={env}"),
        "-e".into(),
        format!("onboard_api_base={api_base}"),
        // The play's manifest root defaults to /var/lib/onboard-runner/repo —
        // a mgmt-host path that exists on no operator laptop. This wrapper
        // runs the play locally, and the manifests live in the checkout
        // (ops/operators/<env>/<handle>.yml, same source operator-onboard.yml
        // reads), so point the play at the repo root it already discovered.
        // JSON form, not key=value: Ansible's parse_kv splits key=value
        // extra-vars on whitespace INSIDE the argv element, so a checkout
        // path containing a space would silently truncate the manifest root
        // and every approved key would be falsely skip-reported no_manifest.
        "-e".into(),
        serde_json::json!({ "onboard_runner_repo": repo_root }).to_string(),
    ];
    if check {
        args.push("-e".into());
        args.push("ssh_key_install_check=true".into());
    }
    args
}

/// P1.6 — should we run the playbook under `direnv exec` to load the operator
/// env? The onboard reads its per-surface tokens (<ENV>_*_TOKEN,
/// *_PORTAL_OPERATOR_API_KEY, …) from the project's gitignored `.envrc.local.d`,
/// which direnv loads. When `goat admin` is invoked from a non-interactive shell
/// (automation, CI), direnv has NOT loaded them, so surfaces silently skipped
/// and the bundle came out half-provisioned.
///
/// Wrap only when the env is NOT already loaded (`DIRENV_DIR` unset) AND direnv
/// is on PATH. An interactive operator (direnv hook active → `DIRENV_DIR` set)
/// takes the direct path unchanged; if direnv is absent entirely we also run
/// direct and rely on the playbook's portal-key preflight + P1.1 completeness
/// gate to catch a missing token loudly rather than ship a partial.
pub(crate) fn should_wrap_with_direnv() -> bool {
    if std::env::var_os("DIRENV_DIR").is_some() {
        return false; // env already loaded in this shell
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|p| p.join("direnv").is_file())
}

/// Construct the argv for `ansible-preflight.sh operator-tier-onboard.yml`
/// (the backend-operator tier). Same `-i localhost, --connection local`
/// shape as the participant onboard; `-e operator=<handle>` plus, when set,
/// `-e operator_env=<env>`.
/// Lifted out so a test can lock the args without spawning ansible.
fn build_operator_tier_args(handle: &str, env: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-i".into(),
        "localhost,".into(),
        "--connection".into(),
        "local".into(),
        "operator-tier-onboard.yml".into(),
        "-e".into(),
        format!("operator={handle}"),
    ];
    if let Some(env) = env {
        args.push("-e".into());
        args.push(format!("operator_env={env}"));
    }
    args
}

/// Validate an `--env` value before it is interpolated into a filesystem
/// path. Mirrors the `^[a-z][a-z0-9-]*[a-z0-9]$` site-id shape used across
/// the manifests + group_vars; rejects path separators / traversal so a
/// stray `--env ../../etc` can never escape `ops/operators/`.
pub(crate) fn is_valid_env(env: &str) -> bool {
    let bytes = env.as_bytes();
    if env.len() < 2 || bytes.len() > 32 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return false;
    }
    env.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub(crate) fn discover_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// One member row of the declared registry `docs/operators/roster.json`
/// (generated by `ops/operators/gen-roster.py`). `org` is the
/// `affiliation.id`; only the fields the org cascades need.
#[derive(serde::Deserialize)]
struct RosterMember {
    handle: String,
    #[serde(rename = "type", default)]
    kind: String,
    org: String,
    status: String,
    /// The per-identity `vendor.short_tag` (= cert-CN reconciliation key,
    /// per B.2); `null` for members minted without one. Used by the rotate
    /// cascade as a second key to resolve a member to its slot anchor.
    #[serde(default)]
    vendor_tag: Option<String>,
}

#[derive(serde::Deserialize)]
struct RosterFile {
    #[serde(default)]
    members: Vec<RosterMember>,
}

/// An active org member as the cascades see it — the declared-registry view.
/// `vendor_tag` is the per-identity `short_tag` when present (the rotate
/// cascade's anchor-resolution key); the revoke cascade only needs the handle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OrgMember {
    handle: String,
    kind: String,
    vendor_tag: Option<String>,
}

/// Resolve the ACTIVE members of an affiliation org from the declared
/// registry — the principals an org-scope cascade (revoke or rotate) spans.
/// The registry (`roster.json`, affiliation-grouped) is the SoT for org
/// membership; a missing/unreadable roster is a hard error so an org cascade
/// can never silently resolve to "nobody" (and under-cut). Sorted by handle.
///
/// `goatnet` names WHICH declared registry. A cascade runs against one
/// goatnet, so it resolves that goatnet's own roster —
/// `docs/operators/<goatnet>/roster.json` (ADR 1172). With no goatnet it falls
/// back to the controller-side union, which spans every environment: the cut
/// itself stays correct, because each member is revoked against the configured
/// goatnet and one that never admitted them simply has nobody to cut, but the
/// blast radius printed to the operator names people from other environments.
/// The fallback warns rather than refusing, so an old profile keeps working.
fn resolve_org_members(
    repo_root: &Path,
    org_id: &str,
    goatnet: Option<&str>,
) -> anyhow::Result<Vec<OrgMember>> {
    let roster_path = match goatnet {
        Some(g) => repo_root.join(format!("docs/operators/{g}/roster.json")),
        None => {
            eprintln!(
                "WARNING: no goatnet named, so the blast radius below is resolved from \
                 the controller-side union docs/operators/roster.json, which spans EVERY \
                 environment. Set `goatnet = \"<name>\"` in this context's profile.toml \
                 to resolve this goatnet's own roster instead (ADR 1172)."
            );
            repo_root.join("docs/operators/roster.json")
        }
    };
    let raw = std::fs::read_to_string(&roster_path).map_err(|e| {
        anyhow::anyhow!(
            "cannot read the declared registry {} ({e}); regenerate it with \
             ops/operators/gen-roster.py{} before an org cascade",
            roster_path.display(),
            goatnet.map(|g| format!(" --env {g}")).unwrap_or_default()
        )
    })?;
    let roster: RosterFile =
        serde_json::from_str(&raw).map_err(|e| anyhow::anyhow!("roster.json is malformed: {e}"))?;
    let mut out: Vec<OrgMember> = roster
        .members
        .into_iter()
        .filter(|m| m.org == org_id && m.status == "active")
        .map(|m| OrgMember {
            handle: m.handle,
            kind: m.kind,
            vendor_tag: m.vendor_tag.filter(|t| !t.is_empty()),
        })
        .collect();
    out.sort();
    Ok(out)
}

/// `goat admin onboard --org <id> --revoke` — the org-scope revoke cascade
/// (identity grammar B.4 / ADR 1031). Resolves the active affiliation members
/// from the registry, prints the blast radius, confirms (unless `--yes`), then
/// runs the per-handle revoke for EACH. Sequential so the audit trail is
/// ordered and a partial failure is visible; one member's failure does not
/// abort the rest (each is reported, exit is non-zero if any failed). Reuses
/// the per-handle `onboard(revoke)` path verbatim, so every sibling gets the
/// same mesh-first cut + Ory deactivate + status flip the single-principal
/// revoke is mock-tested for. Reversible (no `--purge` on the cascade).
async fn onboard_org(
    profile: &crate::profile::Profile,
    org_id: &str,
    env: Option<&str>,
    repo_root: Option<&Path>,
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    let members = resolve_org_members(&repo_root, org_id, env)?;
    if members.is_empty() {
        anyhow::bail!(
            "no active members found for org `{org_id}` in the registry — nothing to \
             revoke (is the affiliation.id correct? check docs/operators/roster.json)"
        );
    }

    // Blast radius — always printed, even with --yes.
    eprintln!(
        "ORG REVOKE CASCADE — org `{org_id}` ({} active member(s)):",
        members.len()
    );
    for m in &members {
        eprintln!("  - {}  [{}]", m.handle, m.kind);
    }
    eprintln!(
        "Each gets the per-handle revoke (mesh peers cut, Ory deactivated, status -> \
         revoked). Reversible — no --purge."
    );

    if dry_run {
        eprintln!("dry-run: no revoke executed.");
        return Ok(());
    }

    if !yes {
        eprint!(
            "Revoke all {} member(s) of `{org_id}`? [y/N] ",
            members.len()
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            anyhow::bail!("aborted — no members revoked");
        }
    }

    let mut failures = 0usize;
    for m in &members {
        eprintln!("=== revoking {} (org `{org_id}`) ===", m.handle);
        if let Err(e) = onboard(
            profile,
            &m.handle,
            env,
            true,
            false,
            Some(&repo_root),
            false,
        )
        .await
        {
            eprintln!("  FAILED to revoke {}: {e}", m.handle);
            failures += 1;
        }
    }

    if failures > 0 {
        anyhow::bail!(
            "{failures} of {} member(s) failed to revoke — see above; re-run to retry \
             (revoke is idempotent)",
            members.len()
        );
    }
    eprintln!(
        "org `{org_id}` cascade complete — {} member(s) revoked.",
        members.len()
    );
    Ok(())
}

/// `goat admin onboard <handle> [--revoke]` — thin wrapper around
/// `ops/scripts/ansible-preflight.sh operator-onboard.yml`. We keep
/// the heavy lifting in ansible (where it already works and is
/// auditable) and just provide:
///   - manifest pre-flight (refuse early if the handle has no manifest)
///   - line-buffered stdout/stderr streaming so operators see ansible
///     progress in real time (default `Command::status()` buffers)
///   - SIGINT propagation so Ctrl-C cleanly stops the playbook (tokio
///     spawns the child in its own process group; we kill it on signal)
///   - non-zero exit code surfaced as a clean error message
// The disallowed-methods lint (clippy.toml) pins goat-relay's hot-path
// tasks to a shard via spawn_local; that constraint is goat-relay-only.
// goat-cli is a CLI on a multi-thread runtime — these spawns just stream
// child stdout/stderr + handle SIGINT, so the explicit allow is the
// clippy.toml-prescribed escape hatch.
#[allow(clippy::disallowed_methods)]
pub async fn onboard(
    profile: &crate::profile::Profile,
    handle: &str,
    env: Option<&str>,
    revoke: bool,
    // P1.5 — true only on the revoke leg of a `--repair` run; appends
    // `-e onboard_repair=true` so the revoke frees surfaces without flipping the
    // manifest status, leaving it active for the immediately-following enroll leg.
    repair: bool,
    repo_root: Option<&Path>,
    dry_run: bool,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!(
                "invalid --env `{env}`: must be a kebab-case goatnet id \
                 (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `zagros-prod`)"
            );
        }
    }

    let cwd = std::env::current_dir()?;
    let repo_root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); \
                 pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    // Per-env layout: ops/operators/<env>/<handle>.yml. Flat legacy
    // layout (no --env): ops/operators/<handle>.yml.
    let operators_dir = repo_root.join("ops/operators");
    let manifest = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest.exists() {
        anyhow::bail!(
            "no operator manifest at {} — author it first per docs/design/operator-onboard-utilities.md",
            manifest.display()
        );
    }

    let ansible_dir = repo_root.join("ops/ansible");
    let preflight = repo_root.join("ops/scripts/ansible-preflight.sh");
    if !preflight.exists() {
        anyhow::bail!(
            "ansible-preflight.sh not found at {} — repo layout looks wrong",
            preflight.display()
        );
    }

    let mut args = build_onboard_ansible_args(handle, env, revoke);
    if revoke && repair {
        // The revoke leg of a --repair: free surfaces but keep the manifest
        // status active for the enroll leg (gates off the R5.3 status flip).
        args.push("-e".into());
        args.push("onboard_repair=true".into());
    }

    if dry_run {
        eprintln!(
            "dry-run: (cwd {}) {} {}",
            ansible_dir.display(),
            preflight.display(),
            args.join(" "),
        );
        return Ok(());
    }

    // Operator-action audit: emit `started` BEFORE spawning
    // ansible, then `ok`/`failed` AFTER. `admin.onboard` for enroll,
    // `admin.revoke` for revoke. All best-effort — never affects this command's exit.
    let audit_action = if revoke && repair {
        "admin.repair-revoke"
    } else if revoke {
        "admin.revoke"
    } else {
        "admin.onboard"
    };
    let audit_env = audit_goatnet(env, profile);
    let audit_actor = crate::cmd::audit::actor_handle(profile);
    let audit_digest = format!("onboard handle={handle} env={audit_env} revoke={revoke}");
    crate::cmd::audit::emit(
        profile,
        crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            audit_action,
            handle,
            audit_env,
            "started",
        )
        .with_args_digest(&audit_digest),
    )
    .await;

    // P1.6 — load the operator env via direnv when it isn't already loaded, so
    // a non-interactive invocation doesn't silently skip token-gated surfaces.
    let mut command = if should_wrap_with_direnv() {
        eprintln!(
            "goat admin onboard: DIRENV_DIR unset — running under `direnv exec {}` to load operator tokens",
            repo_root.display()
        );
        let mut c = Command::new("direnv");
        c.arg("exec").arg(&repo_root).arg(&preflight).args(&args);
        c
    } else {
        let mut c = Command::new(&preflight);
        c.args(&args);
        c
    };
    let mut child = command
        .current_dir(&ansible_dir)
        // macOS Objective-C fork safety nag — see CLAUDE.md §Running Ansible.
        // Harmless on Linux.
        .env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn ansible-preflight.sh: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("{line}");
        }
    });

    // SIGINT propagation: spawn a separate task to await Ctrl-C and
    // signal via oneshot. This keeps the mutable borrow on `child`
    // contained to the select arm, so the kill-then-wait recovery
    // path on Ctrl-C can re-borrow `child` cleanly.
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(());
    });

    let interrupted: bool;
    let wait_result: std::io::Result<std::process::ExitStatus> = tokio::select! {
        s = child.wait() => { interrupted = false; s }
        _ = shutdown_rx => {
            eprintln!("\ngoat admin onboard: SIGINT received, killing ansible-preflight.sh");
            interrupted = true;
            let _ = child.kill().await;
            child.wait().await
        }
    };

    // Drain stdout/stderr readers so the pipes don't get truncated
    // mid-line; aborting the tasks is fine since the child is dead
    // and the readers will hit EOF naturally either way.
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    // Finish-emit the operator-action outcome. Best-effort; mirrors the
    // `started` event's actor/action/target/env so the pair correlates.
    let finish_audit = |outcome: &'static str, err: Option<String>| {
        let mut a = crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            audit_action,
            handle,
            audit_env,
            outcome,
        )
        .with_args_digest(&audit_digest);
        if let Some(e) = err {
            a = a.with_error(e);
        }
        crate::cmd::audit::emit(profile, a)
    };

    if interrupted {
        let msg = format!(
            "aborted by Ctrl-C (child exit: {})",
            wait_result
                .map(|st| format!("{st}"))
                .unwrap_or_else(|e| format!("wait-failed: {e}")),
        );
        finish_audit("failed", Some(msg.clone())).await;
        return Err(anyhow::anyhow!("{msg}"));
    }
    let status = match wait_result {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("child wait failed: {e}");
            finish_audit("failed", Some(msg.clone())).await;
            return Err(anyhow::anyhow!("{msg}"));
        }
    };

    if !status.success() {
        let msg = format!(
            "ansible-preflight.sh exited {} for operator={} action={}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            handle,
            if revoke { "revoke" } else { "enroll" },
        );
        finish_audit("failed", Some(msg.clone())).await;
        anyhow::bail!("{msg}");
    }
    finish_audit("ok", None).await;
    eprintln!(
        "goat admin onboard: {} for {} complete",
        if revoke { "revoke" } else { "enroll" },
        handle,
    );
    Ok(())
}

/// `goat admin ssh-key install --env <goatnet> [--check] [--yes]` — wrapper
/// around `ops/scripts/ansible-preflight.sh operator-ssh-key-install.yml`.
///
/// The installer lives in Ansible; this helper does the same substrate work as
/// `goat admin onboard`:
///   - resolve repo root from cwd
///   - stream ansible stdout/stderr in real time
///   - propagate SIGINT to the child
///   - best-effort operator-action audit (`sshkey.install`) for real runs
///
/// `check_mode = true` runs the play in check mode (`ssh_key_install_check=true`)
/// so it reports what WOULD be installed and changes nothing; real runs carry
/// the audit event.
#[allow(clippy::disallowed_methods)]
async fn ssh_key_install_playbook(
    profile: &crate::profile::Profile,
    env: &str,
    api_base: &str,
    api_key: &str,
    check_mode: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = discover_repo_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "could not locate repo root from {} (no .git ancestor); pass --profile-dir or run from inside the repo",
            cwd.display()
        )
    })?;

    let ansible_dir = repo_root.join("ops/ansible");
    let preflight = repo_root.join("ops/scripts/ansible-preflight.sh");
    if !preflight.exists() {
        anyhow::bail!(
            "ansible-preflight.sh not found at {} — repo layout looks wrong",
            preflight.display()
        );
    }

    let args =
        build_ssh_key_install_args(env, check_mode, api_base, &repo_root.to_string_lossy());

    let audit_actor = crate::cmd::audit::actor_handle(profile);
    let audit_env = env;
    let audit_digest = format!("ssh-key install env={env} api_base={api_base} check={check_mode}");

    if !check_mode {
        crate::cmd::audit::emit(
            profile,
            crate::cmd::audit::OperatorActionV1::new(
                &audit_actor,
                "goat-cli",
                "sshkey.install",
                "ssh-keys",
                audit_env,
                "started",
            )
            .with_args_digest(&audit_digest),
        )
        .await;
    }

    // Load the operator env via direnv when it isn't already loaded, so a
    // non-interactive invocation doesn't silently skip token-gated surfaces.
    let mut command = if should_wrap_with_direnv() {
        eprintln!(
            "goat admin ssh-key install: DIRENV_DIR unset — running under `direnv exec {}` to load operator tokens",
            repo_root.display()
        );
        let mut c = Command::new("direnv");
        c.arg("exec").arg(&repo_root).arg(&preflight).args(&args);
        c
    } else {
        let mut c = Command::new(&preflight);
        c.args(&args);
        c
    };

    let mut child = command
        .current_dir(&ansible_dir)
        // macOS Objective-C fork safety nag — see CLAUDE.md §Running Ansible.
        // Harmless on Linux.
        .env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        // Secure key transit: the installer reads the operator API key from
        // `ONBOARD_API_KEY` (or a provided onboard_api_key extra-var). The key
        // never appears on argv.
        .env("ONBOARD_API_KEY", api_key)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn ansible-preflight.sh: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("{line}");
        }
    });

    // SIGINT propagation: spawn a separate task to await Ctrl-C and signal via
    // oneshot. This keeps the mutable borrow on `child` contained to the
    // select arm, so the kill-then-wait recovery path on Ctrl-C can re-borrow
    // `child` cleanly.
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(());
    });

    let interrupted: bool;
    let wait_result: std::io::Result<std::process::ExitStatus> = tokio::select! {
        s = child.wait() => { interrupted = false; s }
        _ = shutdown_rx => {
            eprintln!("\ngoat admin ssh-key install: SIGINT received, killing ansible-preflight.sh");
            interrupted = true;
            let _ = child.kill().await;
            child.wait().await
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if interrupted {
        let msg = format!(
            "aborted by Ctrl-C (child exit: {})",
            wait_result
                .map(|st| format!("{st}"))
                .unwrap_or_else(|e| format!("wait-failed: {e}")),
        );
        if !check_mode {
            crate::cmd::audit::emit(
                profile,
                crate::cmd::audit::OperatorActionV1::new(
                    &audit_actor,
                    "goat-cli",
                    "sshkey.install",
                    "ssh-keys",
                    audit_env,
                    "failed",
                )
                .with_args_digest(&audit_digest)
                .with_error(msg.clone()),
            )
            .await;
        }
        return Err(anyhow::anyhow!("{msg}"));
    }

    let status = match wait_result {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("child wait failed: {e}");
            if !check_mode {
                crate::cmd::audit::emit(
                    profile,
                    crate::cmd::audit::OperatorActionV1::new(
                        &audit_actor,
                        "goat-cli",
                        "sshkey.install",
                        "ssh-keys",
                        audit_env,
                        "failed",
                    )
                    .with_args_digest(&audit_digest)
                    .with_error(msg.clone()),
                )
                .await;
            }
            return Err(anyhow::anyhow!("{msg}"));
        }
    };

    if !status.success() {
        let msg = format!(
            "ansible-preflight.sh exited {} for ssh-key install env={}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            env,
        );
        if !check_mode {
            crate::cmd::audit::emit(
                profile,
                crate::cmd::audit::OperatorActionV1::new(
                    &audit_actor,
                    "goat-cli",
                    "sshkey.install",
                    "ssh-keys",
                    audit_env,
                    "failed",
                )
                .with_args_digest(&audit_digest)
                .with_error(msg.clone()),
            )
            .await;
        }
        anyhow::bail!("{msg}");
    }

    if !check_mode {
        crate::cmd::audit::emit(
            profile,
            crate::cmd::audit::OperatorActionV1::new(
                &audit_actor,
                "goat-cli",
                "sshkey.install",
                "ssh-keys",
                audit_env,
                "ok",
            )
            .with_args_digest(&audit_digest),
        )
        .await;
        eprintln!(
            "goat admin ssh-key install: env={} complete — approved keys installed",
            env
        );
    }

    Ok(())
}

// ------------------------------------------------------------------
// P2.1 — native Zenoh-surface onboard (no Ansible)
// ------------------------------------------------------------------

/// Minimal projection of an operator manifest — only the fields the native
/// Zenoh onboard reads. serde_yaml ignores the rest. See
/// ops/operators/example-vendor.yml.example for the full shape.
#[derive(Debug, Default, serde::Deserialize)]
struct OnboardManifest {
    #[serde(default)]
    identity: MfIdentity,
    // Org-neutral noun (ADR 1052); `#[serde(alias)]` keeps reading the legacy
    // `vendor:` block so existing manifests still parse. Author `org:` going forward.
    #[serde(default, alias = "vendor")]
    org: MfOrg,
    #[serde(default)]
    netbird: MfNetbird,
    #[serde(default)]
    ory_site: MfSurface,
    #[serde(default)]
    ory_idp: MfSurface,
    #[serde(default)]
    zenoh: MfZenoh,
    #[serde(default)]
    affiliation: MfAffiliation,
    #[serde(default)]
    sponsorship: MfSponsorship,
    // Machine-enrollment grant seeded at onboard (ADR 1096): how many machines
    // this person may enroll as a custodian without a later operator grant. The
    // unit each machine inherits is the manifest's `affiliation.id`. 0 = no grant.
    #[serde(default)]
    machine_quota: i64,
    /// The manifest's `collab:` block — the cloud collaboration environment
    /// decision (schema §collab). Read-only here: the revoke path uses it to
    /// know whether this person has anything on that plane, so the receipt can
    /// say `n/a` instead of implying a decision nobody had to make.
    #[serde(default)]
    collab: MfCollab,
}

/// Machine-enrollment quota seeded for an escalated admin tier at invite time.
/// See the tier branch in the invite builder for why zero is the wrong start.
const DEFAULT_ADMIN_MACHINE_QUOTA: i64 = 10;

/// The manifest's `collab:` block. `enabled` is the
/// sponsor's grant decision; `account` is the provider account id when one was
/// recorded (the onboard-runner reconcile backfills it).
#[derive(Debug, Default, serde::Deserialize)]
struct MfCollab {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    account: String,
}

/// The manifest's `sponsorship:` block (ADR 1071) — whether this identity may
/// sponsor candidate onboardings, and up to which tier. Passed through to the
/// portal registry row at mint, where the invite/vouch gates enforce it. The
/// iam-admin / platform-admin tier presets set it by default; everyone else
/// gets it case-by-case.
#[derive(Debug, Default, serde::Deserialize)]
struct MfSponsorship {
    #[serde(default)]
    authorized: bool,
    #[serde(default)]
    max_tier: String,
    /// Optional Ory-login console binding (ADR 1072) — lets the sponsor use
    /// the id-admin-console onboarding page scoped to their own invitees.
    #[serde(default)]
    email: String,
}

/// The manifest's `affiliation:` block — the grammar dimension the Ory `aff`
/// token claim (Phase D) carries. Only `id`/`kind`/`nation` reach the claim; the
/// richer registry fields (unit_code, edges) stay registry-side.
#[derive(Debug, Default, Clone, serde::Deserialize)]
struct MfAffiliation {
    #[serde(default)]
    sector: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    nation: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct MfIdentity {
    #[serde(default)]
    name: String,
    // The Kratos identifier for the Ory leg (P2.4). Email-shaped; IS the person.
    #[serde(default)]
    email: String,
    // Lifecycle state the revoke path flips (`active` → `revoked`). The native
    // revoke's teammate guard (R4.3) also reads siblings' `status` to decide
    // whether a shared Zenoh anchor still has active members.
    #[serde(default)]
    status: String,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
struct MfOrg {
    #[serde(default)]
    display_name: String,
    // Threaded into the peer label (peer-<handle>-<short_tag>-<env>-<date>); the
    // legacy `vendor.short_tag` key still deserializes via the alias (ADR 1052).
    #[serde(default)]
    short_tag: String,
}

/// An Ory surface block (`ory_site:` / `ory_idp:`). The native Ory leg (P2.4)
/// reads `ory_site` — `sites` (which goatnets to create the Kratos identity on),
/// `groups` (SUFFIX names expanded to `<SITE>-<suffix>` in the `groups[]` claim),
/// and `credentials` (password / passkey). The extra fields deserialize
/// harmlessly on `ory_idp` (which the native path never provisions).
#[derive(Debug, Default, serde::Deserialize)]
struct MfSurface {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    sites: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    credentials: MfOryCredentials,
}

/// `ory_site.credentials` — which credential method(s) to mint. Both optional so
/// an omitted block falls back to the playbook default (password=true,
/// passkey=false, per `ory_site_credentials_default`).
#[derive(Debug, Default, serde::Deserialize)]
struct MfOryCredentials {
    #[serde(default)]
    password: Option<bool>,
    #[serde(default)]
    passkey: Option<bool>,
}

/// The manifest's `netbird:` block — the fields the native mesh-enrollment leg
/// reads. `auto_groups` carries SUFFIX names (e.g. `OPERATOR`); the leg expands
/// each to `<site_prefix>-<suffix>` and resolves the UUID.
#[derive(Debug, Default, serde::Deserialize)]
struct MfNetbird {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    role: String,
    #[serde(default)]
    ttl_days: Option<u64>,
    /// Why this manifest asks for longer than the role default. REQUIRED when
    /// `ttl_days` exceeds it — see `check_ttl_override` for the reasoning.
    #[serde(default)]
    ttl_reason: Option<String>,
    #[serde(default)]
    usage_limit: Option<u32>,
    #[serde(default)]
    auto_groups: Vec<String>,
    /// How many phones/tablets the person's HANDSET key enrols (ADR 1175).
    /// The handset key is OBSERVER-only whatever `auto_groups` says; `0`
    /// mints no handset key; absent means `DEFAULT_HANDSET_LIMIT`.
    #[serde(default)]
    handset_limit: Option<u32>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct MfZenoh {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    trial: Option<MfZenohTrial>,
    // Present only on the read-only-observer / prod flavours; the native path
    // doesn't handle them (no portal mint) — used only to emit a precise bail.
    #[serde(default)]
    observer: Option<serde_yaml::Value>,
    #[serde(default)]
    prod: Option<serde_yaml::Value>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct MfZenohTrial {
    // The publisher prefix. `org` is the going-forward noun (ADR 1052); the
    // legacy `vendor` key still deserializes via the alias.
    #[serde(default, alias = "vendor")]
    org: String,
    // ADR 1138 — allocate this org's slot from the pre-cut PARTNER BLOCK bound
    // to this lane-set (e.g. ["REL-ALL","REL-PARTNER"]) instead of the lowest free
    // slot, so the minted CN matches a partner subject the router has already
    // rendered: no ACL edit, no fabric restart.
    //
    // Absent = an ordinary slot. For a PARTNER that means they are NOT
    // partitioned and read the whole fabric until somebody hand-carves them,
    // silently. Which lanes they may read is a release decision, so it
    // is stated here rather than derived from affiliation.nation.
    #[serde(default)]
    partner_lanes: Vec<String>,
}

/// Router endpoint for `env`, from the context whose profile names that goatnet
/// (`goat context refresh` writes it from get.<site>/goatnet.toml). goat holds
/// no goatnet of its own; no context for `env` resolves to None. A pre-fill hint:
/// the portal mints the authoritative endpoint into the bundle tarball's NOTES.md.
fn native_router_endpoint_for(
    env: Option<&str>,
    profile: &crate::profile::Profile,
) -> Option<String> {
    profile.goatnet_endpoints_for(env).router_endpoint
}

/// Per-goatnet NetBird config the native mesh-enrollment leg needs: the mgmt-API
/// origin, the operator-token env var, and the ACL group-name site prefix.
/// Each value comes from the goatnet's group_vars through goatnet.toml
/// (roles/goat-get, `goat_get_goatnet_facts`).
pub(crate) struct NbEnvConfig {
    pub(crate) mgmt_url: &'static str,
    pub(crate) token_env_var: &'static str,
    pub(crate) site_prefix: &'static str,
    /// Intra-mesh PQC (Rosenpass, ADR 0602): when true, the rendered
    /// `netbird up` join command appends `--enable-rosenpass
    /// --rosenpass-permissive` so a peer onboarded via this native path is
    /// PQ-secured — parity with the goatnet's onboarding-console toggle
    /// (`id_admin_console_notify_rosenpass`) and the ansible
    /// `rosenpass_enabled` group_var. Per-goatnet, default off.
    rosenpass: bool,
}

/// NetBird config for `env`, from the context whose profile names that goatnet.
/// A mgmt URL and a token env var are both required; without them the mesh leg
/// is skipped with a pointer to `goat context refresh`. Rosenpass is on only for
/// an explicit `true`.
pub(crate) fn native_netbird_config_for(
    env: Option<&str>,
    profile: &crate::profile::Profile,
) -> Option<NbEnvConfig> {
    let g = profile.goatnet_endpoints_for(env);
    let (Some(url), Some(tok)) = (g.netbird_mgmt_url, g.netbird_token_env) else {
        return None;
    };
    Some(NbEnvConfig {
        mgmt_url: leak_or(Some(url), ""),
        token_env_var: leak_or(Some(tok), ""),
        site_prefix: leak_or(g.netbird_site_prefix, ""),
        // Absent or unparseable is off: a transport is never turned on by omission.
        rosenpass: g.netbird_rosenpass.unwrap_or(false),
    })
}

/// NbEnvConfig holds `&'static str`. A profile-sourced value is owned, so it is
/// leaked deliberately: it lives for
/// the process either way, one allocation per goatnet per run, and the
/// alternative is threading a lifetime through every native leg for no gain.
fn leak_or(v: Option<String>, fallback: &'static str) -> &'static str {
    match v {
        Some(s) if !s.trim().is_empty() => Box::leak(s.into_boxed_str()),
        _ => fallback,
    }
}

/// How many devices one person's setup key enrols when the manifest does not
/// say. Mirrors `netbird.usage_limit` in the console's onboard-intake-form.json
/// and `netbird_setup_key_usage_limit_default` in operator-onboard.yml.
///
/// Not 1. Falling back to a single-use key looks conservative and is the
/// opposite: it issues the most restrictive credential the system can express
/// in a field where restriction buys no security — a one-shot key does not make
/// a laptop safer, it guarantees the person comes back to an operator.
const DEFAULT_SETUP_KEY_USAGE_LIMIT: u32 = 5;

// Compile-time, not a test: the property that matters is "never 1", and a
// build failure is a stronger fence than a test someone can skip. Setting this
// to 1 would reinstate the silent single-use key this constant exists to stop.
const _: () = assert!(
    DEFAULT_SETUP_KEY_USAGE_LIMIT > 1,
    "a manifest that omits usage_limit must not silently mint a one-shot key; \
     asking for one explicitly with `netbird.usage_limit: 1` still works"
);

/// How many phones/tablets one person's HANDSET key enrols when the manifest
/// does not say (ADR 1175, docs/design/mobile-device-onboarding.md §2.2). The
/// same default lives in `netbird.handset_limit` of both copies of
/// operator-manifest.schema.json and in the console's onboard-intake-form.json;
/// change all of them together. `0` is the explicit "no handset key" value.
const DEFAULT_HANDSET_LIMIT: u32 = 1;
/// Upper bound on `netbird.handset_limit` — matches the schema's `maximum`.
const MAX_HANDSET_LIMIT: u32 = 5;

/// The handset reuse count the manifest asks for, clamped to the schema range.
fn effective_handset_limit(nb: &MfNetbird) -> u32 {
    nb.handset_limit
        .unwrap_or(DEFAULT_HANDSET_LIMIT)
        .min(MAX_HANDSET_LIMIT)
}

/// NetBird name of the handset key that pairs with a device key named `key_name`.
fn handset_key_name(key_name: &str) -> String {
    format!("{key_name}{HANDSET_KEY_SUFFIX}")
}

/// Suffix that distinguishes a person's handset setup key from their device key.
/// Appended to the KEY name only; `auto_peer_name` stays the device key's name so
/// a phone is named `peer-<handle>[-<tag>]-<site>-<date>-<device>` like a laptop.
const HANDSET_KEY_SUFFIX: &str = "-handset";

/// The summary line the onboard-runner scrapes for the handset key. Fixed
/// format: `  handset key:  <KEY>  (key `<name>`)`.
fn handset_key_line(key: &str, name: &str) -> String {
    format!("  handset key:  {key}  (key `{name}`)")
}

/// Default mesh-credential TTL (days) by NetBird role, mirroring
/// `netbird_role_ttl_days_default` in the playbook. A manifest `ttl_days`
/// overrides this; an unknown/blank role falls back to the conservative
/// peer default.
///
/// `peer` was 90 days previously . A welcome package is written to be
/// acted on the day it lands, so the long window bought nothing and left a live
/// mesh credential sitting in a mailbox for eleven of its twelve weeks. Note
/// that "conservative" for the unknown-role fallback now means SHORT: an
/// unrecognised role gets the tightest ordinary default rather than the
/// loosest, because a role we cannot name is not one we should extend trust to.
/// Pre-provisioning ahead of a move is the intended exception and is per-person
/// — `netbird.ttl_days` in the manifest — never a change to this table.
fn role_ttl_days(role: &str) -> u64 {
    match role {
        "bootstrap" => 1,
        "admin" => 7,
        "operator" => 30,
        _ => 7, // peer + unknown
    }
}

/// Refuse a credential window longer than the role default unless the manifest
/// says why.
///
/// The peer default was cut from 90 days to 7 because a welcome
/// package is meant to be acted on the day it lands. The exception — kit
/// provisioned ahead of a move, where the credential must exist before it can be
/// redeemed — is real but narrow, and it was left as a CONVENTION: any manifest
/// could simply set a bigger number and nothing would ask.
///
/// A convention that costs nothing to ignore is not a control. This makes the
/// exception cost one sentence, which is the whole point: the person widening
/// the window is the person who knows why, and the reason belongs next to the
/// number rather than in somebody's memory.
///
/// It CANNOT be a CI check. Operator manifests are gitignored — they carry names
/// and addresses — so no job in the repo ever sees one. The mint is the only
/// place that reads the file, which makes it the only place this can be enforced.
///
/// Shorter than the default always passes: tightening a credential needs no
/// justification.
fn check_ttl_override(nb: &MfNetbird) -> anyhow::Result<()> {
    let Some(requested) = nb.ttl_days else {
        return Ok(());
    };
    let default = role_ttl_days(&nb.role);
    if requested <= default {
        return Ok(());
    }
    let reason = nb.ttl_reason.as_deref().unwrap_or("").trim().to_string();
    if reason.len() >= 12 {
        eprintln!(
            "note: credential window widened to {requested}d (role default {default}d) — {reason}"
        );
        return Ok(());
    }
    anyhow::bail!(
        "netbird.ttl_days is {requested}, longer than the {default}-day default for role \
         `{role}`, and netbird.ttl_reason does not say why.\n\n\
         A credential that outlives the week it was sent in sits redeemable in a mailbox for \
         the rest of its life. The exception is narrow — kit provisioned before it can be \
         enrolled, typically ahead of a move — so state it in the manifest:\n\n  \
         netbird:\n    ttl_days: {requested}\n    ttl_reason: \"issued ahead of the \
         <name> movement; enrolled on arrival\"\n\n\
         Or drop ttl_days and take the {default}-day default. See \
         docs/operations/setup-key-rotation.md.",
        role = nb.role
    );
}

/// The result of the native NetBird mesh-enrollment leg (P2.2): the minted
/// setup key + the metadata the bundle renders. Held so the bundle md is
/// written once, after both legs run.
struct NbProvision {
    setup_key: String,
    mgmt_url: String,
    key_name: String,
    expires_days: u64,
    usage_limit: u32,
    /// Fully-qualified group names (`<site_prefix>-<suffix>`) for display.
    group_full_names: Vec<String>,
    /// Append the Rosenpass PQC flags to the rendered join command
    /// (from the goatnet's `NbEnvConfig.rosenpass`).
    rosenpass: bool,
    /// The OBSERVER-only setup key for the person's phones/tablets (ADR 1175);
    /// `None` when the manifest set `netbird.handset_limit: 0`.
    handset_key: Option<String>,
    /// NetBird name of that key (`<key_name>-handset`).
    handset_key_name: Option<String>,
    /// Every setup key THIS run minted (device key, then handset key), as the
    /// NetBird metadata a revoke needs. A later leg that fails revokes exactly
    /// these and nothing else (#1587): a key the run did not mint is never
    /// touched.
    minted: Vec<panoscope_core::netbird::SetupKeyMeta>,
}

/// Render the NetBird section of the bundle (empty string when the mesh leg
/// didn't run). Pure over the provision result so it's unit-testable.
fn render_netbird_section(nb: Option<&NbProvision>) -> String {
    let Some(nb) = nb else { return String::new() };
    let groups = if nb.group_full_names.is_empty() {
        "(site default group only)".to_string()
    } else {
        nb.group_full_names.join(", ")
    };
    // Intra-mesh PQC (Rosenpass, ADR 0602): on a Rosenpass-enabled goatnet the
    // join command carries the flags so the peer is post-quantum-secured from
    // enrollment. Permissive = no connectivity break to classical peers.
    let (rp_flags, rp_note) = if nb.rosenpass {
        (
            " \\\n         --enable-rosenpass --rosenpass-permissive",
            "This goatnet enables post-quantum protection (Rosenpass): the join \
             command above carries `--enable-rosenpass --rosenpass-permissive`, so \
             this peer's WireGuard sessions are PQ-secured. Permissive mode means \
             it still talks to not-yet-enabled peers.\n\n",
        )
    } else {
        ("", "")
    };
    format!(
        "\n## NetBird mesh access\n\
         \n\
         The setup key enrolls this peer onto the mesh. One-time per use; \
         `usage_limit` is how many peers may enroll with it.\n\
         \n\
         | | |\n\
         |---|---|\n\
         | Setup key | `{key}` |\n\
         | Management URL | {mgmt_url} |\n\
         | Groups | {groups} |\n\
         | Expires | {expires_days} days |\n\
         | Usage limit | {usage_limit} |\n\
         \n\
         ### Join the mesh\n\
         \n\
         ```bash\n\
         sudo netbird up --management-url {mgmt_url} --setup-key {key}{rp_flags}\n\
         ```\n\
         \n\
         {rp_note}{handset}",
        key = nb.setup_key,
        mgmt_url = nb.mgmt_url,
        groups = groups,
        expires_days = nb.expires_days,
        usage_limit = nb.usage_limit,
        rp_flags = rp_flags,
        rp_note = rp_note,
        handset = render_handset_subsection(nb),
    )
}

/// The "Phone or tablet" subsection of the NetBird section — only when a
/// handset key was minted (ADR 1175). The handset key is a separate,
/// OBSERVER-only credential, so the phone never inherits the laptop's groups.
fn render_handset_subsection(nb: &NbProvision) -> String {
    let Some(key) = nb.handset_key.as_deref() else {
        return String::new();
    };
    format!(
        "### Phone or tablet\n\
         \n\
         Your phone or tablet uses its own key, below, not the one above. It \
         joins as a browsing device (OBSERVER) whatever your laptop's groups are.\n\
         \n\
         | | |\n\
         |---|---|\n\
         | Handset key | `{key}` |\n\
         | Management URL | {mgmt_url} |\n\
         \n\
         1. Install NetBird from the App Store or Google Play.\n\
         2. Open the app, choose Change Server, and enter the management URL above.\n\
         3. Enter the handset key above and connect. Accept the VPN prompt.\n\
         4. iOS and Android run one VPN at a time: turn any other VPN off first.\n\
         5. Android only: Settings → Network & internet → Private DNS → Off, or \
         mesh names will not resolve.\n\
         \n\
         The phone reaches the same web services as a laptop, through the same \
         login. It cannot run the `goat` command, publish to the fabric, or receive \
         files.\n\
         \n",
        key = key,
        mgmt_url = nb.mgmt_url,
    )
}

/// Render the native onboarding bundle (markdown). Pure over the portal +
/// NetBird results so it's unit-testable without either service. Carries the
/// one-time credentials (setup key, api_key, bundle URL) up top as crash
/// insurance, then the join + fetch-and-first-publish steps. Mirrors the Zenoh
/// + NetBird sections of `ops/ansible/templates/operator-onboard/bundle.md.j2`.
fn render_bundle_md(
    handle: &str,
    env: Option<&str>,
    o: Option<&OnboardedOrg>,
    portal_url: &str,
    router_endpoint: Option<&str>,
    nb: Option<&NbProvision>,
    ory: Option<&OryProvision>,
) -> String {
    let envname = env.unwrap_or("unset");
    // Name the surfaces this bundle actually provisioned, in onboarding order.
    let mut sfx: Vec<&str> = Vec::new();
    if nb.is_some() {
        sfx.push("NetBird mesh access");
    }
    if ory.is_some() {
        sfx.push("Ory OIDC login");
    }
    if o.is_some() {
        sfx.push("Zenoh wire-layer");
    }
    let provisioned = format!("the **{}**", sfx.join(" + "));
    let netbird_section = render_netbird_section(nb);
    let ory_section = render_ory_section(ory);
    // Zenoh section only for a manifest with a Zenoh surface. A browse/web-only
    // tier (o=None) gets NetBird + Ory sections and no Zenoh block.
    let zenoh_section = match o {
        None => String::new(),
        Some(o) => {
            let anchor = if o.slot_id.is_empty() {
                &o.prefix
            } else {
                &o.slot_id
            };
            let router = router_endpoint
                .map(|r| format!("`{r}`"))
                .unwrap_or_else(|| "see the tarball's NOTES.md (portal-minted)".to_string());
            let bundle_full = full_bundle_url(Some(portal_url), &o.bundle_url);
            format!(
                "## Zenoh trial fabric\n\
                 \n\
                 Two distinct credentials:\n\
                 \n\
                 1. **X.509 wire-layer cert + key** — what the router validates at session establishment. Inside the tarball below.\n\
                 2. **Portal API key** — authenticates you to the portal for topic registration + schema upload + inspector calls. Distinct from the X.509 cert by design.\n\
                 \n\
                 | | |\n\
                 |---|---|\n\
                 | Write anchor (slot_id) | `{anchor}` |\n\
                 | Handle (alias) | `{system_handle}` |\n\
                 | URI-SAN | `{uri_san}` |\n\
                 | Portal | {portal_url} |\n\
                 | Router endpoint (mTLS) | {router} |\n\
                 | Portal API key | `{api_key}` |\n\
                 | Bundle tarball URL (one-time, ~24 h) | {bundle_full} |\n\
                 \n\
                 ### Fetch the cert tarball (do this first — the URL is one-time)\n\
                 \n\
                 ```bash\n\
                 mkdir -p ~/{handle}-zenoh && cd ~/{handle}-zenoh\n\
                 curl -fsSL '{bundle_full}' -o bundle.tar.gz\n\
                 tar xzf bundle.tar.gz\n\
                 # Inside: the cert + key (keep the key mode 0400), the fabric CA root,\n\
                 # a NOTES.md with the authoritative router endpoint, and a goat-cli\n\
                 # profile (.cbor) for a one-command `goat profile init`.\n\
                 ```\n\
                 \n\
                 ### Register your first topic\n\
                 \n\
                 ```bash\n\
                 curl -X POST -H 'Content-Type: application/json' \\\n\
                      -H 'Authorization: Bearer {api_key}' \\\n\
                      -d '{{\"key_pattern\":\"{anchor}/foo/v1\",\"format\":\"json\",\"description\":\"first topic\"}}' \\\n\
                      {portal_url}/api/orgs/{anchor}/topics\n\
                 ```\n\
                 \n\
                 The portal UI at {portal_url} does the same through a form; bring the api_key as a Bearer token.\n",
                handle = handle,
                anchor = anchor,
                system_handle = if o.system_handle.is_empty() { handle } else { &o.system_handle },
                uri_san = if o.uri_san.is_empty() { "(minted at provisioning — see your cert)" } else { &o.uri_san },
                portal_url = portal_url,
                router = router,
                api_key = o.api_key,
                bundle_full = bundle_full,
            )
        }
    };
    format!(
        "# Onboarding bundle — `{handle}` ({envname})\n\
         \n\
         Native onboard (`goat admin onboard --native`). This provisions {provisioned}.\n\
         {netbird_section}\
         {ory_section}\
         {zenoh_section}",
        handle = handle,
        envname = envname,
        provisioned = provisioned,
        netbird_section = netbird_section,
        ory_section = ory_section,
        zenoh_section = zenoh_section,
    )
}

/// Native NetBird mesh-enrollment leg (P2.2): resolve the manifest's auto-group
/// suffixes to UUIDs and mint a setup key, mirroring the playbook's Phase 1, with
/// no Ansible. Returns `Ok(None)` when the leg doesn't apply (netbird disabled,
/// or an env without native NetBird config — warned, deferred to Ansible), or on
/// dry-run. Returns `Err` on a hard failure (token unset, a group missing, API
/// error) — the caller runs this BEFORE the Zenoh leg so a mesh failure leaves
/// nothing Zenoh-side consumed and the onboard is cleanly retryable.
async fn provision_netbird_native(
    env: Option<&str>,
    profile: &crate::profile::Profile,
    handle: &str,
    nb: &MfNetbird,
    org_short_tag: &str,
    dry_run: bool,
) -> anyhow::Result<Option<NbProvision>> {
    if !nb.enabled {
        return Ok(None);
    }
    let cfg = match native_netbird_config_for(env, profile) {
        Some(c) => c,
        None => {
            eprintln!(
                "note: no NetBird facts for goatnet `{}` — run `goat context refresh` in that \
                 goatnet's context (it reads get.<site>/goatnet.toml); the mesh leg is skipped.",
                env.unwrap_or("(none)")
            );
            return Ok(None);
        }
    };
    let envname = env.unwrap_or("unset");
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    // The peer-name grammar lives in ops/identity/peer-name-grammar.yaml,
    // generated into panoscope_core::peername and byte-parity fenced against the
    // Go consumers that PARSE what this mints (ops/ci/check-peer-name-grammar-sync.sh).
    let key_name = peername::mint(handle, org_short_tag, envname, &date);
    check_ttl_override(nb)?;
    let ttl_days = nb.ttl_days.unwrap_or_else(|| role_ttl_days(&nb.role));
    // A manifest that omits `usage_limit` must not silently get a SINGLE-USE
    // key. One-off is a legitimate thing to ask for and stays expressible — set
    // `netbird.usage_limit: 1` — but it is the wrong thing to fall back to: the
    // first successful enrolment consumes the key and every attempt after it
    // returns "couldn't add peer: setup key is invalid", so the person is
    // blocked by a second machine, a reinstall, or the native Windows client
    // after installing under WSL. Nothing in the surfaces displays the limit
    // either.
    //
    // 5 matches the `netbird.usage_limit` default in the console's
    // onboard-intake-form.json and `netbird_setup_key_usage_limit_default` in
    // operator-onboard.yml. Three implementations of one policy, in three
    // languages, with no shared source — if you change one, change all three.
    let usage_limit = nb.usage_limit.unwrap_or(DEFAULT_SETUP_KEY_USAGE_LIMIT).max(1);
    let key_type = if usage_limit > 1 {
        "reusable"
    } else {
        "one-off"
    };
    let full_names: Vec<String> = nb
        .auto_groups
        .iter()
        .map(|s| format!("{}-{}", cfg.site_prefix, s))
        .collect();

    if dry_run {
        eprintln!(
            "dry-run: GET {}/api/groups → resolve {:?}",
            cfg.mgmt_url, full_names
        );
        eprintln!(
            "dry-run: POST {}/api/setup-keys  name={key_name} type={key_type} ttl_days={ttl_days} \
             usage_limit={usage_limit} groups={:?}",
            cfg.mgmt_url, nb.auto_groups
        );
        let handset_limit = effective_handset_limit(nb);
        if handset_limit > 0 {
            eprintln!(
                "dry-run: POST {}/api/setup-keys  name={} type={} ttl_days={ttl_days} \
                 usage_limit={handset_limit} groups=[\"OBSERVER\"(+\"HANDSET\" if the group exists)] auto_peer_name={key_name}",
                cfg.mgmt_url,
                handset_key_name(&key_name),
                setup_key_type(handset_limit),
            );
        } else {
            eprintln!("dry-run: netbird.handset_limit=0 — no handset key");
        }
        return Ok(None);
    }

    let token = std::env::var(cfg.token_env_var).unwrap_or_default();
    if token.trim().is_empty() {
        anyhow::bail!(
            "NetBird mgmt token env var `{}` is unset — the mesh leg can't mint a setup key. \
             Set it (see ops/operators/README.md §Preconditions) and re-run, or run the Ansible \
             NetBird leg.",
            cfg.token_env_var
        );
    }
    let nbc = panoscope_core::netbird::NetbirdClient::new(cfg.mgmt_url.to_string(), token);

    let group_ids = if nb.auto_groups.is_empty() {
        Vec::new()
    } else {
        let groups = nbc
            .list_groups()
            .await
            .map_err(|e| anyhow::anyhow!("NetBird GET /api/groups failed: {e}"))?;
        panoscope_core::netbird::resolve_group_ids(&groups, cfg.site_prefix, &nb.auto_groups)
            .map_err(|missing| {
                anyhow::anyhow!(
                    "NetBird: group(s) not found on `{envname}`: {}. Create them first with \
                     `ansible-playbook -i localhost, --connection local netbird-acl-bootstrap.yml \
                     -e target_site={envname}`, then re-run.",
                    missing.join(", ")
                )
            })?
    };

    let key = nbc
        .create_setup_key(panoscope_core::netbird::NewSetupKey {
            name: key_name.clone(),
            key_type: key_type.to_string(),
            expires_in: ttl_days * 86_400,
            revoked: false,
            auto_groups: group_ids.clone(),
            usage_limit,
            ephemeral: false,
            auto_peer_name: key_name.clone(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("NetBird POST /api/setup-keys failed: {e}"))?;
    if key.key.trim().is_empty() {
        anyhow::bail!("NetBird returned an empty setup key");
    }
    let mut minted = vec![minted_key_meta(&key, &key_name, &group_ids)];

    // Handset key (ADR 1175): a second, OBSERVER-only key for the person's
    // phones and tablets, so a handset never inherits the laptop's groups. Same
    // TTL, same peer-name prefix; only the key NAME carries the `-handset`
    // suffix. Minted after the device key so a failure here leaves a usable
    // device key and a clear error, not a half-provisioned mesh leg.
    let handset_limit = effective_handset_limit(nb);
    let (handset_key, handset_key_name) = if handset_limit > 0 {
        match mint_handset_key(&nbc, &cfg, &key_name, ttl_days, handset_limit, envname).await {
            Ok((h, h_groups)) => {
                minted.push(minted_key_meta(&h, &handset_key_name(&key_name), &h_groups));
                (Some(h.key), Some(h.name))
            }
            Err(e) => {
                // The device key was minted a moment ago and the command is about
                // to fail, so nobody will ever be sent it (#1587).
                revoke_keys(&nbc, &minted).await;
                return Err(e);
            }
        }
    } else {
        (None, None)
    };

    Ok(Some(NbProvision {
        setup_key: key.key,
        mgmt_url: cfg.mgmt_url.to_string(),
        key_name,
        expires_days: ttl_days,
        usage_limit,
        group_full_names: full_names,
        rosenpass: cfg.rosenpass,
        handset_key,
        handset_key_name,
        minted,
    }))
}

/// The revoke metadata for a key this run just created. The name falls back to
/// the one we asked for when NetBird's echo omits it.
fn minted_key_meta(
    key: &panoscope_core::netbird::SetupKeyResult,
    requested_name: &str,
    auto_groups: &[String],
) -> panoscope_core::netbird::SetupKeyMeta {
    panoscope_core::netbird::SetupKeyMeta {
        id: key.id.clone(),
        name: if key.name.trim().is_empty() {
            requested_name.to_string()
        } else {
            key.name.clone()
        },
        auto_groups: auto_groups.to_vec(),
        revoked: false,
    }
}

/// The keys a failed run must revoke: only those it minted, only those NetBird
/// gave an id for (an id-less key cannot be addressed, and guessing by name could
/// hit a key from an earlier run), and none twice.
fn keys_to_revoke_after_failure(
    minted: &[panoscope_core::netbird::SetupKeyMeta],
) -> Vec<&panoscope_core::netbird::SetupKeyMeta> {
    let mut seen = std::collections::HashSet::new();
    minted
        .iter()
        .filter(|k| !k.id.trim().is_empty() && !k.revoked && seen.insert(k.id.clone()))
        .collect()
}

/// Revoke the given keys, reporting each. Best-effort: a revoke failure is
/// printed with the key name so an operator can finish it, and never replaces
/// the error that made the run fail.
async fn revoke_keys(
    nbc: &panoscope_core::netbird::NetbirdClient,
    minted: &[panoscope_core::netbird::SetupKeyMeta],
) {
    for k in keys_to_revoke_after_failure(minted) {
        match nbc.revoke_setup_key(k).await {
            Ok(()) => eprintln!("revoked setup key `{}` ({}) minted by this failed run", k.name, k.id),
            Err(e) => eprintln!(
                "warning: could not revoke setup key `{}` ({}) minted by this failed run: {e}. \
                 It is valid and unheld — revoke it in NetBird.",
                k.name, k.id
            ),
        }
    }
}

/// Revoke the setup keys a native onboard minted, after a LATER leg failed
/// (#1587). Without this a failed-then-retried mint left the first attempt's
/// keys valid and unsent: live enrolment credentials nobody was given.
async fn revoke_minted_setup_keys(
    env: Option<&str>,
    profile: &crate::profile::Profile,
    nb: Option<&NbProvision>,
) {
    let Some(nb) = nb else { return };
    if keys_to_revoke_after_failure(&nb.minted).is_empty() {
        return;
    }
    let Some(cfg) = native_netbird_config_for(env, profile) else { return };
    let token = std::env::var(cfg.token_env_var).unwrap_or_default();
    if token.trim().is_empty() {
        eprintln!(
            "warning: `{}` is unset, so the setup key(s) this failed run minted could not be \
             revoked: {}. Revoke them in NetBird.",
            cfg.token_env_var,
            nb.minted.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", ")
        );
        return;
    }
    let nbc = panoscope_core::netbird::NetbirdClient::new(nb.mgmt_url.clone(), token);
    revoke_keys(&nbc, &nb.minted).await;
}

/// NetBird key type for a reuse count: reusable above one, one-off at one.
fn setup_key_type(usage_limit: u32) -> &'static str {
    if usage_limit > 1 {
        "reusable"
    } else {
        "one-off"
    }
}

/// The group suffixes a handset key carries: `OBSERVER` always, plus `HANDSET`
/// when the goatnet has created that device-attribute group (ADR 1175 §2.4 —
/// it distributes the primary handset nameserver group and sits in no ACL
/// rule). Pure over the group list so the choice is unit-tested; a goatnet
/// that has not switched handset DNS on mints exactly what it minted before.
fn handset_group_suffixes(groups: &[panoscope_core::netbird::NbGroup], site_prefix: &str) -> Vec<String> {
    let handset = format!("{site_prefix}-HANDSET");
    let mut out = vec!["OBSERVER".to_string()];
    if groups.iter().any(|g| g.name == handset && !g.id.is_empty()) {
        out.push("HANDSET".to_string());
    }
    out
}

/// Mint the OBSERVER-only handset key that pairs with the device key
/// `key_name` (ADR 1175). `auto_peer_name` is the device key's name, unsuffixed,
/// so the phone lands on the same `peer-<handle>…-<site>-<date>-<device>`
/// grammar every other peer follows and resolves to its person.
async fn mint_handset_key(
    nbc: &panoscope_core::netbird::NetbirdClient,
    cfg: &NbEnvConfig,
    key_name: &str,
    ttl_days: u64,
    handset_limit: u32,
    envname: &str,
) -> anyhow::Result<(panoscope_core::netbird::SetupKeyResult, Vec<String>)> {
    let groups = nbc
        .list_groups()
        .await
        .map_err(|e| anyhow::anyhow!("NetBird GET /api/groups failed: {e}"))?;
    let suffixes = handset_group_suffixes(&groups, cfg.site_prefix);
    if suffixes.len() == 1 {
        eprintln!(
            "HANDSET group not present on {envname}; phones use the match-domain nameserver group \
             (create `{}-HANDSET` with netbird-acl-bootstrap.yml to give them the primary handset resolver)",
            cfg.site_prefix
        );
    }
    let group_ids = panoscope_core::netbird::resolve_group_ids(&groups, cfg.site_prefix, &suffixes)
        .map_err(|missing| {
            anyhow::anyhow!(
                "NetBird: group(s) not found on `{envname}`: {} — the handset key needs \
                 `{}-OBSERVER`. Create it with `netbird-acl-bootstrap.yml -e target_site={envname}` \
                 and re-run.",
                missing.join(", "),
                cfg.site_prefix
            )
        })?;
    let name = handset_key_name(key_name);
    let key = nbc
        .create_setup_key(panoscope_core::netbird::NewSetupKey {
            name: name.clone(),
            key_type: setup_key_type(handset_limit).to_string(),
            expires_in: ttl_days * 86_400,
            revoked: false,
            auto_groups: group_ids.clone(),
            usage_limit: handset_limit,
            ephemeral: false,
            auto_peer_name: key_name.to_string(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("NetBird POST /api/setup-keys (handset) failed: {e}"))?;
    if key.key.trim().is_empty() {
        anyhow::bail!("NetBird returned an empty handset setup key");
    }
    Ok((key, group_ids))
}

/// The non-revoked handset keys that belong to `handle` on `env` — what a
/// `--reissue-handset-key` run revokes before minting a fresh one. Pure over the
/// key list so the selection is unit-tested; keyed on the same `belongs_to`
/// anchor the revoke cut uses, narrowed to the `-handset` suffix so a device
/// key is never swept by a handset reissue.
fn handset_keys_to_revoke<'a>(
    keys: &'a [panoscope_core::netbird::SetupKeyMeta],
    handle: &str,
    tag: &str,
    env: &str,
) -> Vec<&'a panoscope_core::netbird::SetupKeyMeta> {
    keys.iter()
        .filter(|k| {
            !k.revoked
                && k.name.ends_with(HANDSET_KEY_SUFFIX)
                && revoke_peer_matches(&k.name, handle, tag, env)
        })
        .collect()
}

/// Validate `--env` and resolve the repo root for a key reissue. Shared by both
/// reissue verbs so they refuse the same malformed input the same way.
fn reissue_repo_root(env: Option<&str>, repo_root: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!(
                "invalid --env `{env}`: must be a kebab-case goatnet id \
                 (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `nova`)"
            );
        }
    }
    let cwd = std::env::current_dir()?;
    match repo_root {
        Some(p) => Ok(p.to_path_buf()),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); pass --repo-root <path>",
                cwd.display()
            )
        }),
    }
}

/// The person's operator manifest for a reissue, or `None` when this machine
/// does not hold one. Manifests are gitignored and live where the person was
/// minted (the onboard-runner host for a console onboarding), so an operator
/// laptop usually has none.
fn read_reissue_manifest(
    repo_root: &Path,
    handle: &str,
    env: Option<&str>,
) -> anyhow::Result<Option<(PathBuf, OnboardManifest)>> {
    let operators_dir = repo_root.join("ops/operators");
    let manifest_path = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: OnboardManifest = serde_yaml::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;
    Ok(Some((manifest_path, manifest)))
}

/// Emit one operator-action record for a key reissue. The target is the PERSON
/// the key is for, the actor is whoever ran the command (the runner stamps
/// `GOAT_OPERATOR_HANDLE`), and the digest names the key, its reuse count and
/// the requester — so "who minted which key for whom, on whose request" is one
/// record in the audit trail rather than a NetBird activity row attributed to
/// the service token.
async fn emit_reissue_audit(
    profile: &crate::profile::Profile,
    action: &str,
    handle: &str,
    env: &str,
    outcome: &str,
    digest: &str,
    err: Option<String>,
) {
    let actor = crate::cmd::audit::actor_handle(profile);
    let mut a = crate::cmd::audit::OperatorActionV1::new(&actor, "goat-cli", action, handle, env, outcome)
        .with_args_digest(digest);
    if let Some(e) = err {
        a = a.with_error(e);
    }
    crate::cmd::audit::emit(profile, a).await;
}

/// What a reissue audit record describes. Never carries the key itself — only
/// its name, reuse count, lifetime and groups, plus who asked.
struct ReissueDigest<'a> {
    kind: &'a str,
    handle: &'a str,
    env: &'a str,
    key_name: &'a str,
    uses: u32,
    ttl_days: u64,
    groups: &'a [String],
    requested_by: Option<&'a str>,
}

impl ReissueDigest<'_> {
    /// The redacted summary the audit record hashes.
    fn render(&self) -> String {
        format!(
            "reissue-{}-key handle={} env={} key={} uses={} ttl_days={} groups={} requested_by={}",
            self.kind,
            self.handle,
            self.env,
            self.key_name,
            self.uses,
            self.ttl_days,
            self.groups.join(","),
            self.requested_by
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(operator)")
        )
    }
}

/// `goat admin onboard <handle> --reissue-handset-key`: revoke every live
/// handset key the person holds on this goatnet and mint a fresh one (a lost or
/// replaced phone, ADR 1175 §2.6). Touches nothing else — no device key, no
/// Ory, no bundle. The manifest's `netbird.handset_limit` sets the reuse count;
/// `0` or `netbird.enabled: false` refuses, because there is nothing to reissue.
fn require_reissue_active(manifest: &OnboardManifest) -> anyhow::Result<()> {
    anyhow::ensure!(
        !manifest.identity.status.trim().eq_ignore_ascii_case("revoked"),
        "replacement refused: the operator manifest is revoked"
    );
    Ok(())
}

async fn reissue_handset_key_native(
    profile: &crate::profile::Profile,
    handle: &str,
    env: Option<&str>,
    repo_root: Option<&Path>,
    requested_by: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let repo_root = reissue_repo_root(env, repo_root)?;
    let Some((_, manifest)) = read_reissue_manifest(&repo_root, handle, env)? else {
        anyhow::bail!(
            "no operator manifest for `{handle}` under {}/ops/operators — nothing to reissue",
            repo_root.display()
        );
    };
    require_reissue_active(&manifest)?;
    let nb = &manifest.netbird;
    if !nb.enabled {
        anyhow::bail!("`{handle}` has netbird.enabled: false — no mesh key to reissue");
    }
    let handset_limit = effective_handset_limit(nb);
    if handset_limit == 0 {
        anyhow::bail!(
            "`{handle}` has netbird.handset_limit: 0 — no handset key is issued for this person. \
             Raise it in the manifest first."
        );
    }
    // With no --env the key is for the active context's goatnet: the one its
    // profile names, else the context's own name.
    let envname: String = env
        .map(str::to_string)
        .or_else(|| profile.goatnet_endpoints().goatnet)
        .unwrap_or_else(|| profile.context.clone());
    let envname = envname.as_str();
    let cfg = native_netbird_config_for(env, profile).ok_or_else(|| {
        anyhow::anyhow!(
            "no NetBird facts for goatnet `{}` — run `goat context refresh` in that goatnet's context; \
             the handset key is reissued natively only.",
            envname
        )
    })?;
    let tag = manifest.org.short_tag.trim().to_string();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let key_name = peername::mint(handle, &tag, envname, &date);
    check_ttl_override(nb)?;
    let ttl_days = nb.ttl_days.unwrap_or_else(|| role_ttl_days(&nb.role));

    let token = std::env::var(cfg.token_env_var).unwrap_or_default();
    if token.trim().is_empty() && !dry_run {
        anyhow::bail!(
            "NetBird mgmt token env var `{}` is unset — cannot reissue the handset key.",
            cfg.token_env_var
        );
    }
    let nbc = panoscope_core::netbird::NetbirdClient::new(cfg.mgmt_url.to_string(), token.clone());
    let existing = if dry_run && token.trim().is_empty() {
        Vec::new()
    } else {
        nbc.list_setup_keys()
            .await
            .map_err(|e| anyhow::anyhow!("NetBird GET /api/setup-keys failed: {e}"))?
    };
    let doomed = handset_keys_to_revoke(&existing, handle, &tag, envname);

    if dry_run {
        for k in &doomed {
            eprintln!("dry-run: revoke setup key `{}`", k.name);
        }
        eprintln!(
            "dry-run: POST {}/api/setup-keys  name={} type={} ttl_days={ttl_days} \
             usage_limit={handset_limit} groups=[\"OBSERVER\"(+\"HANDSET\" if the group exists)] auto_peer_name={key_name}",
            cfg.mgmt_url,
            handset_key_name(&key_name),
            setup_key_type(handset_limit),
        );
        return Ok(());
    }

    let handset_name = handset_key_name(&key_name);
    let observer = [format!("{}-OBSERVER", cfg.site_prefix)];
    let digest = ReissueDigest {
        kind: "handset",
        handle,
        env: envname,
        key_name: &handset_name,
        uses: handset_limit,
        ttl_days,
        groups: &observer,
        requested_by,
    }
    .render();
    const ACTION: &str = "admin.reissue-handset-key";
    emit_reissue_audit(profile, ACTION, handle, envname, "started", &digest, None).await;
    let result: anyhow::Result<panoscope_core::netbird::SetupKeyResult> = async {
        for k in &doomed {
            nbc.revoke_setup_key(k)
                .await
                .map_err(|e| anyhow::anyhow!("NetBird revoke of `{}` failed: {e}", k.name))?;
            eprintln!("revoked handset key `{}`", k.name);
        }
        mint_handset_key(&nbc, &cfg, &key_name, ttl_days, handset_limit, envname)
            .await
            .map(|(key, _groups)| key)
    }
    .await;
    let fresh = match result {
        Ok(k) => k,
        Err(e) => {
            emit_reissue_audit(profile, ACTION, handle, envname, "failed", &digest, Some(e.to_string())).await;
            return Err(e);
        }
    };
    emit_reissue_audit(
        profile,
        ACTION,
        handle,
        envname,
        "ok",
        &format!("{digest} netbird_key_id={}", fresh.id),
        None,
    )
    .await;
    println!(
        "reissued handset key for `{handle}` on {envname} ({} old key(s) revoked)",
        doomed.len()
    );
    println!("{}", handset_key_line(&fresh.key, &fresh.name));
    Ok(())
}

/// The DEVICE setup keys that belong to `handle` on `env`, oldest first by the
/// date the name carries. Handset keys are excluded, so a device reissue never
/// sweeps a phone's key. Pure over the key list so the selection is unit-tested.
fn device_keys_of<'a>(
    keys: &'a [panoscope_core::netbird::SetupKeyMeta],
    handle: &str,
    tag: &str,
    env: &str,
) -> Vec<&'a panoscope_core::netbird::SetupKeyMeta> {
    let mut out: Vec<_> = keys
        .iter()
        .filter(|k| !k.name.ends_with(HANDSET_KEY_SUFFIX) && revoke_peer_matches(&k.name, handle, tag, env))
        .collect();
    out.sort_by(|a, b| {
        key_name_date(strip_legacy_reissue_suffix(&a.name))
            .cmp(key_name_date(strip_legacy_reissue_suffix(&b.name)))
    });
    out
}

/// The trailing `-YYYY-MM-DD` a minted key name ends with (without the dash),
/// or `""` when the name does not end in a date.
fn key_name_date(name: &str) -> &str {
    let Some(cut) = name.len().checked_sub(10) else {
        return "";
    };
    if cut == 0 || !name.is_char_boundary(cut) || !name[..cut].ends_with('-') {
        return "";
    }
    let tail = &name[cut..];
    let ok = tail.bytes().enumerate().all(|(i, c)| match i {
        4 | 7 => c == b'-',
        _ => c.is_ascii_digit(),
    });
    if ok {
        tail
    } else {
        ""
    }
}

// Historical suffix added by the pre-grammar Rosenpass reissue flow. It is not
// part of a minted key name, but existing setup keys carrying it must remain
// reissuable while their manifests are backfilled.
const LEGACY_ROSENPASS_REISSUE_SUFFIX: &str = "-rosenpass-reissue";

fn strip_legacy_reissue_suffix(name: &str) -> &str {
    name.strip_suffix(LEGACY_ROSENPASS_REISSUE_SUFFIX).unwrap_or(name)
}

/// A prior key name with its date replaced by `date` — how a reissue
/// with no manifest keeps the person's exact name grammar (handle, org tag,
/// goatnet) without having to know the tag. The one known historical Rosenpass
/// suffix is removed so the replacement returns to the canonical grammar.
/// `None` if the name carries no accepted date form.
fn redate_key_name(name: &str, date: &str) -> Option<String> {
    let canonical = strip_legacy_reissue_suffix(name);
    let old = key_name_date(canonical);
    if old.is_empty() {
        return None;
    }
    Some(format!("{}{date}", &canonical[..canonical.len() - old.len()]))
}

/// `goat admin onboard <handle> --reissue-device-key [--uses N]`: mint a fresh
/// device setup key for a person whose key expired or was used up, revoke any of
/// their device keys that is still live, and record who did it for whom.
///
/// Grants come from the manifest when this machine holds it. Without one (the
/// usual case on an operator laptop — manifests live on the onboard-runner host)
/// they come from the person's most recent device key on the goatnet: the grant
/// they actually hold, never a wider default. A person with neither is refused.
/// Existing peers are untouched — revoking a key only stops NEW enrollments.
async fn reissue_device_key_native(
    profile: &crate::profile::Profile,
    handle: &str,
    env: Option<&str>,
    repo_root: Option<&Path>,
    uses: Option<u32>,
    requested_by: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let repo_root = reissue_repo_root(env, repo_root)?;
    let Some(envname) = env.filter(|e| !e.trim().is_empty()) else {
        anyhow::bail!("--reissue-device-key needs --env <goatnet>: a key is minted on one goatnet");
    };
    let cfg = native_netbird_config_for(Some(envname), profile).ok_or_else(|| {
        anyhow::anyhow!(
            "no NetBird facts for goatnet `{envname}` — run `goat context refresh` in that goatnet's \
             context; the device key is reissued natively only."
        )
    })?;
    let manifest = read_reissue_manifest(&repo_root, handle, Some(envname))?;
    if let Some((_, m)) = &manifest {
        require_reissue_active(m)?;
        if !m.netbird.enabled {
            anyhow::bail!("`{handle}` has netbird.enabled: false — no mesh key to reissue");
        }
        check_ttl_override(&m.netbird)?;
    }
    let token = std::env::var(cfg.token_env_var).unwrap_or_default();
    if token.trim().is_empty() {
        anyhow::bail!(
            "NetBird mgmt token env var `{}` is unset — cannot reissue the device key.",
            cfg.token_env_var
        );
    }
    let nbc = panoscope_core::netbird::NetbirdClient::new(cfg.mgmt_url.to_string(), token);
    let keys = nbc
        .list_setup_keys()
        .await
        .map_err(|e| anyhow::anyhow!("NetBird GET /api/setup-keys failed: {e}"))?;
    let groups = nbc
        .list_groups()
        .await
        .map_err(|e| anyhow::anyhow!("NetBird GET /api/groups failed: {e}"))?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Resolve the key name, grants, lifetime and default reuse count from the
    // manifest, else from the person's newest device key.
    let (tag, key_name, group_ids, ttl_days, default_uses, source) = match &manifest {
        Some((path, m)) => {
            let nb = &m.netbird;
            let tag = m.org.short_tag.trim().to_string();
            let ids = panoscope_core::netbird::resolve_group_ids(&groups, cfg.site_prefix, &nb.auto_groups)
                .map_err(|missing| {
                    anyhow::anyhow!("NetBird: group(s) not found on `{envname}`: {}", missing.join(", "))
                })?;
            (
                tag.clone(),
                peername::mint(handle, &tag, envname, &date),
                ids,
                nb.ttl_days.unwrap_or_else(|| role_ttl_days(&nb.role)),
                nb.usage_limit.unwrap_or(DEFAULT_SETUP_KEY_USAGE_LIMIT).max(1),
                format!("manifest {}", path.display()),
            )
        }
        None => {
            // With no manifest the org tag is unknown, so match the untagged
            // grammar only (`peer-<handle>-<env>-`) — the form every per-uuid
            // goatnet mints. A tagged person needs their manifest.
            let prior = device_keys_of(&keys, handle, "", envname);
            let Some(last) = prior.last() else {
                anyhow::bail!(
                    "`{handle}` has no manifest here and no device key named `peer-{handle}-{envname}-…` on \
                     {envname} to copy grants from. Run this where the manifest lives (the onboard-runner \
                     host), or use the console's \"Reissue device key\"."
                );
            };
            if last.auto_groups.is_empty() {
                anyhow::bail!(
                    "`{}` carries no groups — refusing to mint a key that grants nothing. Use the manifest.",
                    last.name
                );
            }
            let name = redate_key_name(&last.name, &date)
                .ok_or_else(|| anyhow::anyhow!("key `{}` carries no date to replace", last.name))?;
            (
                String::new(),
                name,
                last.auto_groups.clone(),
                role_ttl_days("peer"),
                DEFAULT_SETUP_KEY_USAGE_LIMIT,
                format!("prior key `{}`", last.name),
            )
        }
    };
    let usage_limit = uses.unwrap_or(default_uses).max(1);
    let group_names: Vec<String> = group_ids
        .iter()
        .map(|id| {
            groups
                .iter()
                .find(|g| &g.id == id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| id.clone())
        })
        .collect();
    let doomed: Vec<_> = device_keys_of(&keys, handle, &tag, envname)
        .into_iter()
        .filter(|k| !k.revoked)
        .collect();

    eprintln!("grants from {source}: {}", group_names.join(", "));
    if dry_run {
        for k in &doomed {
            eprintln!("dry-run: revoke setup key `{}`", k.name);
        }
        eprintln!(
            "dry-run: POST {}/api/setup-keys  name={key_name} type={} ttl_days={ttl_days} \
             usage_limit={usage_limit} groups={group_names:?} auto_peer_name={key_name}",
            cfg.mgmt_url,
            setup_key_type(usage_limit),
        );
        return Ok(());
    }

    let digest = ReissueDigest {
        kind: "device",
        handle,
        env: envname,
        key_name: &key_name,
        uses: usage_limit,
        ttl_days,
        groups: &group_names,
        requested_by,
    }
    .render();
    const ACTION: &str = "admin.reissue-device-key";
    emit_reissue_audit(profile, ACTION, handle, envname, "started", &digest, None).await;
    let result: anyhow::Result<panoscope_core::netbird::SetupKeyResult> = async {
        for k in &doomed {
            nbc.revoke_setup_key(k)
                .await
                .map_err(|e| anyhow::anyhow!("NetBird revoke of `{}` failed: {e}", k.name))?;
            eprintln!("revoked device key `{}`", k.name);
        }
        let key = nbc
            .create_setup_key(panoscope_core::netbird::NewSetupKey {
                name: key_name.clone(),
                key_type: setup_key_type(usage_limit).to_string(),
                expires_in: ttl_days * 86_400,
                revoked: false,
                auto_groups: group_ids.clone(),
                usage_limit,
                ephemeral: false,
                auto_peer_name: key_name.clone(),
            })
            .await
            .map_err(|e| anyhow::anyhow!("NetBird POST /api/setup-keys failed: {e}"))?;
        if key.key.trim().is_empty() {
            anyhow::bail!("NetBird returned an empty setup key");
        }
        Ok(key)
    }
    .await;
    let fresh = match result {
        Ok(k) => k,
        Err(e) => {
            emit_reissue_audit(profile, ACTION, handle, envname, "failed", &digest, Some(e.to_string())).await;
            return Err(e);
        }
    };
    emit_reissue_audit(
        profile,
        ACTION,
        handle,
        envname,
        "ok",
        &format!("{digest} netbird_key_id={}", fresh.id),
        None,
    )
    .await;
    println!(
        "reissued device key for `{handle}` on {envname}: {usage_limit} use(s), {ttl_days} day(s) \
         ({} old key(s) revoked)",
        doomed.len()
    );
    println!("  setup key:    {}  (key `{}`)", fresh.key, fresh.name);
    Ok(())
}

/// Per-goatnet Ory site-Kratos config the native OIDC leg (P2.4) needs: the
/// Thor's mesh SSH endpoint (the Kratos admin API binds `127.0.0.1:4434`
/// in-container and is deliberately NOT on the mesh/public — it creates any
/// identity with no auth of its own, so it must be reached FROM the Thor), the
/// in-container admin URL, and the public login URL for the bundle. Mirrored
/// from `ory_site_kratos_admin_host_map` in `ops/ansible/operator-onboard.yml`
/// (the SoT). Unknown env → None, and the Ory leg is skipped with a pointer to
/// the Ansible path.
pub(crate) struct OryEnvConfig {
    pub(crate) ssh_host: &'static str,
    ssh_user: &'static str,
    pub(crate) kratos_admin_url: String,
    auth_url: &'static str,
    /// `ssh -o ProxyCommand=…` for reaching the Thor through the goatnet's env
    /// door when the laptop is not on that mesh (docs/design/env-doors.md).
    /// From `GOAT_SSH_PROXY_<ENV>`; None = direct.
    ssh_proxy: Option<String>,
}

/// `GOAT_SSH_PROXY_<ENV>` (e.g. `GOAT_SSH_PROXY_ACME="ssh -p 2222 -W %h:%p
/// ops@127.0.0.1"`): the ProxyCommand every SSH leg to that goatnet uses,
/// so a goatnet reached through its door needs one export, not a profile flip.
pub(crate) fn ssh_proxy_for(env: &str) -> Option<String> {
    let key = format!("GOAT_SSH_PROXY_{}", env.to_uppercase().replace('-', "_"));
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Ory config for `env`, from the context whose profile names that goatnet. An
/// auth URL and an SSH host are both required; without them the login leg is
/// skipped with a pointer to `goat context refresh`.
pub(crate) fn native_ory_config_for(
    env: Option<&str>,
    profile: &crate::profile::Profile,
) -> Option<OryEnvConfig> {
    let g = profile.goatnet_endpoints_for(env);
    let (Some(auth_url), Some(ssh_host)) = (g.ory_auth_url, g.ory_ssh_host) else {
        return None;
    };
    let kratos_admin_url = std::env::var("GOAT_ONBOARD_KRATOS_ADMIN_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:4434".to_string());
    Some(OryEnvConfig {
        ssh_host: leak_or(Some(ssh_host), ""),
        ssh_user: leak_or(g.ory_ssh_user, ""),
        kratos_admin_url,
        auth_url: leak_or(Some(auth_url), ""),
        ssh_proxy: env.and_then(ssh_proxy_for),
    })
}

/// Result of the native Ory leg: the login pointer + whatever credential was
/// minted. `password` XOR `recovery_link` is Some on a fresh create (per the
/// requested method); both None when the identity already existed (idempotent
/// skip). Rendered into the bundle so the operator shares the login OOB.
struct OryProvision {
    auth_url: String,
    email: String,
    password: Option<String>,
    recovery_link: Option<String>,
    already_existed: bool,
    /// The Kratos identity UUID, carried out of the Ory leg so the caller can
    /// record the person->slot linkage (slots.identity_id, ADR 1051 / the
    /// identity fragility audit).
    ///
    /// The native onboard mints the Zenoh slot BEFORE it creates the identity,
    /// so there is no UUID in existence at slot-allocation time — which is why
    /// a slot can carry identity_id NULL, with ownership falling back to
    /// a name join. This field is how the linkage gets recorded without
    /// reordering the legs (the slot-then-Ory order is what makes the onboard
    /// atomic: a failed login leg rolls the slot back).
    identity_id: Option<String>,
}

/// Percent-encode a query-component value (email) so it survives both the URL
/// and the single-quoted remote shell word. Encodes everything but the RFC 3986
/// unreserved set.
pub(crate) fn pct_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A 24-char password from `ascii_letters + digits + !@#%^*-_=+` (mirrors
/// `ory_site_password_chars`), drawn from the OS CSPRNG with rejection sampling
/// so the charset distribution is unbiased.
fn generate_password(len: usize) -> String {
    use rand_core::{OsRng, RngCore};
    const CHARS: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#%^*-_=+";
    let n = CHARS.len() as u32; // 72
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    let reject_above = 256u16 - (256u16 % n as u16); // largest multiple of n ≤ 256
    while out.len() < len {
        OsRng.fill_bytes(&mut buf);
        for &b in buf.iter() {
            if out.len() >= len {
                break;
            }
            if (b as u16) < reject_above {
                out.push(CHARS[(b as u32 % n) as usize] as char);
            }
        }
    }
    out
}

/// Build the `ssh` argv for a delegated Kratos-admin curl. The remote command is
/// a fixed string carrying NO secrets — the JSON body (email/password) is piped
/// via stdin (`curl --data-binary @-`), so nothing sensitive lands in an argv or
/// process listing. Pure, for unit testing.
fn ssh_kratos_argv(cfg: &OryEnvConfig, remote_cmd: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=10".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    if let Some(proxy) = &cfg.ssh_proxy {
        args.push("-o".into());
        args.push(format!("ProxyCommand={proxy}"));
    }
    args.push(format!("{}@{}", cfg.ssh_user, cfg.ssh_host));
    args.push(remote_cmd.to_string());
    args
}

/// Split a `curl -w '\n%{http_code}'` stdout into `(body, status)`. The status
/// is the final line (3 digits, no trailing newline); everything before the last
/// newline is the response body.
fn split_curl_status(raw: &str) -> anyhow::Result<(String, u16)> {
    match raw.rfind('\n') {
        Some(i) => {
            let code: u16 = raw[i + 1..].trim().parse().map_err(|_| {
                anyhow::anyhow!("could not parse curl status from tail {:?}", &raw[i + 1..])
            })?;
            Ok((raw[..i].to_string(), code))
        }
        None => {
            let code: u16 = raw
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("curl output had no status line: {:?}", raw))?;
            Ok((String::new(), code))
        }
    }
}

/// True when the onboard should reach Kratos admin at localhost directly instead
/// of SSHing to the site Thor — set when the caller is ALREADY on the Thor (the
/// on-Thor onboard-runner). The curl target (`http://127.0.0.1:4434`) is identical
/// either way; local mode just drops the ssh wrapper, so the runner needs no SSH
/// key at all. Env-gated (`GOAT_ONBOARD_KRATOS_LOCAL=1`).
fn onboard_kratos_local() -> bool {
    parse_kratos_local(std::env::var("GOAT_ONBOARD_KRATOS_LOCAL").ok())
}

/// Pure truthiness parse for `GOAT_ONBOARD_KRATOS_LOCAL` (`1`/`true`, case-insensitive).
fn parse_kratos_local(v: Option<String>) -> bool {
    match v {
        Some(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        None => false,
    }
}

/// Run a Kratos-admin curl on the site Thor, optionally piping a JSON body via
/// stdin. Returns `(http_status, body)`. By default the curl runs over SSH to the
/// Thor; when `GOAT_ONBOARD_KRATOS_LOCAL` is set (the on-Thor runner) it runs
/// locally via `sh -c` against the same `127.0.0.1:4434` target — no SSH. A
/// non-zero process exit is a transport failure (curl exits 0 for any completed
/// HTTP transaction regardless of status); the HTTP status comes from `-w`.
pub(crate) async fn run_ssh_curl(
    cfg: &OryEnvConfig,
    remote_cmd: &str,
    stdin_body: Option<&str>,
) -> anyhow::Result<(u16, String)> {
    let local = onboard_kratos_local();
    let mut cmd = if local {
        let mut c = Command::new("sh");
        c.arg("-c").arg(remote_cmd);
        c
    } else {
        let args = ssh_kratos_argv(cfg, remote_cmd);
        let mut c = Command::new("ssh");
        c.args(&args);
        c
    };
    cmd.stdin(if stdin_body.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if local {
            anyhow::anyhow!("spawn local kratos curl: {e}")
        } else {
            anyhow::anyhow!("spawn ssh {}@{}: {e}", cfg.ssh_user, cfg.ssh_host)
        }
    })?;
    if let Some(body) = stdin_body {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("curl child stdin unavailable"))?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.shutdown().await?; // EOF so the curl finishes reading @-
    }
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        if local {
            anyhow::bail!(
                "local kratos curl failed (exit {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        anyhow::bail!(
            "ssh to {} failed (exit {:?}): {}",
            cfg.ssh_host,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = split_curl_status(&stdout)?;
    Ok((code, body))
}

/// Assemble the Kratos `POST /admin/identities` body. Uses `serde_json`, which
/// emits non-ASCII as literal UTF-8 (NOT `\uXXXX`) — so a diacritic name crosses
/// the wire as raw bytes, dodging the Postgres SQLSTATE-22P05 reject the Ansible
/// path had to hand-serialize around. The identity org block is emitted under the
/// `org` trait key — the ADR 1052 Phase-4 rename of the legacy `vendor` key. Every
/// site's Kratos schema carries `org` unconditionally; only the deprecated `vendor`
/// block is gated (ory_site_vendor_trait_compat), so emitting `org` validates on
/// both compat=true and compat=false goatnets. The `aff` claim (Phase D)
/// is emitted only when `aff` is `Some` — the caller passes it gated on
/// `GOAT_ORY_AFF_TRAIT_ENABLED`, mirroring the Ansible path's
/// `ory_site_affiliation_trait_enabled` so the two mint paths agree: both omit
/// `aff` today (Phase D deploy-gated), both emit it the day the flag flips + the
/// Kratos schema carries the trait.
/// The `traits.org.short_tag` value for the Kratos identity: the real ORG, which
/// the manifest carries in `affiliation.id` (e.g. `acme-corp`). Falls back to the
/// manifest `org/vendor.short_tag` only when there is no affiliation, so
/// pre-affiliation manifests still onboard. This decouples the org identity from
/// the per-person Zenoh prefix: on per-uuid goatnets `vendor.short_tag` is forced
/// to equal the handle (the `short_tag == zenoh.trial.vendor` drift-check), so
/// copying it into the org trait yielded a degenerate handle-as-org value
/// (`jane-doe` instead of `acme-corp`).
fn org_trait_short_tag<'a>(org: &'a MfOrg, affiliation: &'a MfAffiliation) -> &'a str {
    let aff_id = affiliation.id.trim();
    if aff_id.is_empty() {
        org.short_tag.trim()
    } else {
        aff_id
    }
}

fn build_identity_body(
    email: &str,
    name: &str,
    groups: &[String],
    org: &MfOrg,
    aff: Option<&MfAffiliation>,
    password: Option<&str>,
) -> String {
    use serde_json::json;
    let mut traits = serde_json::Map::new();
    traits.insert("email".into(), json!(email));
    traits.insert("name".into(), json!(name));
    // Authorization fields go to admin-managed metadata_public, never to
    // self-editable traits: the Kratos self-service settings
    // flow can write traits but NOT metadata_public, so a user cannot self-grant
    // a login group. `groups` is always emitted (empty when none) so a
    // metadata-first reader never falls back to a self-edited traits value.
    let mut meta = serde_json::Map::new();
    meta.insert("groups".into(), json!(groups));
    if let Some(a) = aff {
        if !a.kind.trim().is_empty() {
            let mut av = serde_json::Map::new();
            av.insert("id".into(), json!(a.id));
            av.insert("kind".into(), json!(a.kind));
            if let Some(n) = a.nation.as_deref().filter(|n| !n.trim().is_empty()) {
                av.insert("nation".into(), json!(n));
            }
            meta.insert("aff".into(), serde_json::Value::Object(av));
        }
    }
    let dn = org.display_name.trim();
    let st = org.short_tag.trim();
    if !dn.is_empty() || !st.is_empty() {
        let mut v = serde_json::Map::new();
        if !st.is_empty() {
            v.insert("short_tag".into(), json!(st));
        }
        if !dn.is_empty() {
            v.insert("display_name".into(), json!(dn));
        }
        meta.insert("org".into(), serde_json::Value::Object(v));
    }
    let mut body = serde_json::Map::new();
    body.insert("schema_id".into(), json!("default"));
    body.insert("state".into(), json!("active"));
    body.insert("traits".into(), serde_json::Value::Object(traits));
    body.insert("metadata_public".into(), serde_json::Value::Object(meta));
    if let Some(pw) = password {
        body.insert(
            "credentials".into(),
            json!({ "password": { "config": { "password": pw } } }),
        );
    }
    body.insert(
        "verifiable_addresses".into(),
        json!([{ "value": email, "via": "email", "verified": true,
                 "verified_at": "1970-01-01T00:00:00Z", "status": "completed" }]),
    );
    body.insert(
        "recovery_addresses".into(),
        json!([{ "value": email, "via": "email" }]),
    );
    serde_json::Value::Object(body).to_string()
}

/// Mint a one-shot (15-min) Kratos recovery link so the operator can enrol a
/// WebAuthn passkey. `POST /admin/recovery/link` over the same SSH hop.
async fn mint_recovery_link(cfg: &OryEnvConfig, identity_id: &str) -> anyhow::Result<String> {
    use serde_json::json;
    let body = json!({ "identity_id": identity_id, "expires_in": "15m" }).to_string();
    let remote = format!(
        "curl -sS --max-time 30 -X POST -H 'Content-Type: application/json' --data-binary @- \
         -w '\\n%{{http_code}}' '{}/admin/recovery/link'",
        cfg.kratos_admin_url
    );
    let (code, resp) = run_ssh_curl(cfg, &remote, Some(&body)).await?;
    if code != 200 && code != 201 {
        anyhow::bail!(
            "Ory Kratos POST /admin/recovery/link returned HTTP {code} — body: {}",
            resp.chars().take(300).collect::<String>()
        );
    }
    serde_json::from_str::<serde_json::Value>(&resp)
        .ok()
        .and_then(|v| {
            v.get("recovery_link")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "recovery/link response had no recovery_link: {}",
                resp.chars().take(200).collect::<String>()
            )
        })
}

/// The refusal for a reused slot (#1576) whose recorded person linkage names a
/// different identity than the one this onboard just resolved, or `None` when
/// stamping is safe (not reused, nothing recorded, or the same identity).
fn reused_slot_identity_conflict(o: &OnboardedOrg, ory_identity_id: &str) -> Option<String> {
    let recorded = o.identity_id.trim();
    if !o.reused || recorded.is_empty() || recorded == ory_identity_id {
        return None;
    }
    Some(format!(
        "the reused fabric slot `{}` is linked to identity {recorded}, but this onboard resolved \
         identity {ory_identity_id}; refusing to relink one person's slot to another. Check which \
         person owns handle `{}` before going further.",
        o.prefix,
        if o.system_handle.is_empty() { &o.prefix } else { &o.system_handle }
    ))
}

/// The Kratos JSON-patch that brings an EXISTING identity's name and org in line
/// with the manifest of a returning person, plus one "field: before -> after"
/// line per change for the log. Empty when nothing differs.
///
/// Touches only `traits.name` and the org object, and the org only where the
/// identity already keeps it: `metadata_public.org` when `metadata_public` is an
/// object (where a fresh onboard writes it, ADR 1188), and `traits.org` when the
/// identity still carries that legacy key. It never creates `metadata_public`
/// on an identity that has none — a metadata-first reader treats that block's
/// presence as meaningful — and never emits an op for credentials, groups or
/// state.
fn identity_resync_patch(
    existing: &serde_json::Value,
    name: &str,
    org: &MfOrg,
) -> (Vec<serde_json::Value>, Vec<String>) {
    use serde_json::json;
    let mut ops = Vec::new();
    let mut changes = Vec::new();
    let name = name.trim();
    let traits = existing.get("traits");
    if !name.is_empty() {
        let cur = traits.and_then(|t| t.get("name")).and_then(|v| v.as_str()).unwrap_or("");
        if cur != name {
            ops.push(json!({"op": "add", "path": "/traits/name", "value": name}));
            changes.push(format!("traits.name: {cur:?} -> {name:?}"));
        }
    }
    let st = org.short_tag.trim();
    let dn = org.display_name.trim();
    if st.is_empty() && dn.is_empty() {
        return (ops, changes);
    }
    let mut want = serde_json::Map::new();
    if !st.is_empty() {
        want.insert("short_tag".into(), json!(st));
    }
    if !dn.is_empty() {
        want.insert("display_name".into(), json!(dn));
    }
    let want = serde_json::Value::Object(want);
    let render = |v: Option<&serde_json::Value>| v.map_or("(none)".to_string(), |x| x.to_string());
    if let Some(cur) = traits.and_then(|t| t.get("org")) {
        if cur != &want {
            ops.push(json!({"op": "add", "path": "/traits/org", "value": want}));
            changes.push(format!("traits.org: {} -> {want}", render(Some(cur))));
        }
    }
    if let Some(meta) = existing.get("metadata_public").and_then(|m| m.as_object()) {
        let cur = meta.get("org");
        if cur != Some(&want) {
            ops.push(json!({"op": "add", "path": "/metadata_public/org", "value": want}));
            changes.push(format!("metadata_public.org: {} -> {want}", render(cur)));
        }
    }
    (ops, changes)
}

/// Native Ory OIDC leg (P2.4): create the person's Kratos identity on the site
/// Thor, directly over SSH, with no Ansible. Mirrors the playbook's Phase 2 —
/// idempotent GET-by-email, `POST /admin/identities`, optional passkey recovery
/// link. Returns `Ok(None)` when the leg doesn't apply (ory_site disabled, the
/// env has no native Ory config, this env isn't in `ory_site.sites`, or dry-run).
/// Returns `Err` on a hard failure — the CALLER runs this AFTER the Zenoh mint so
/// an Ory failure leaves a usable Zenoh/NetBird bundle (surfaced loudly, never
/// silently partial).
async fn provision_ory_native(
    env: Option<&str>,
    profile: &crate::profile::Profile,
    handle: &str,
    manifest: &OnboardManifest,
    dry_run: bool,
) -> anyhow::Result<Option<OryProvision>> {
    let ory = &manifest.ory_site;
    if !ory.enabled {
        return Ok(None);
    }
    let envname = env.unwrap_or("(none)");
    let cfg = match native_ory_config_for(env, profile) {
        Some(c) => c,
        None => {
            eprintln!(
                "note: no Ory facts for goatnet `{envname}` — run `goat context refresh` in that \
                 goatnet's context (it reads get.<site>/goatnet.toml); the login leg is skipped."
            );
            return Ok(None);
        }
    };
    // Native is single-env. Provision this env's site if the manifest targets it
    // (empty `sites` = "the enabled surface applies here"); defer any other sites
    // to Ansible.
    let targets_env = ory.sites.is_empty() || ory.sites.iter().any(|s| s == envname);
    if !targets_env {
        eprintln!(
            "note: manifest ory_site.sites = {:?} does not include `{envname}`; --native \
             provisions Ory for the --env site only. Run the Ansible path for the other sites.",
            ory.sites
        );
        return Ok(None);
    }
    let others: Vec<&String> = ory.sites.iter().filter(|s| s.as_str() != envname).collect();
    if !others.is_empty() {
        eprintln!(
            "note: manifest ory_site.sites also lists {others:?} — --native provisions only the \
             --env `{envname}` site; run the Ansible path `goat admin onboard {handle}` for the rest."
        );
    }

    let email = manifest.identity.email.trim();
    if email.is_empty() {
        anyhow::bail!(
            "manifest identity.email is empty — the Ory leg needs the Kratos identifier (email). \
             Set identity.email and re-run."
        );
    }
    let name = manifest.identity.name.trim();
    let want_password = ory.credentials.password.unwrap_or(true);
    let want_passkey = ory.credentials.passkey.unwrap_or(false);
    if !want_password && !want_passkey {
        anyhow::bail!(
            "manifest ory_site.credentials requests neither password nor passkey — set at least \
             one (default is password: true)."
        );
    }
    // Ory service-permission groups are written VERBATIM into the `groups` claim
    // — they are self-describing names (`PLATFORM-ADMIN`, `IAM-ADMIN`, `OBS-ADMIN`,
    // …, SoT ops/permissions/group-permissions.yml) that the dashboards / consoles
    // / MinIO gate on directly. NO site-prefix: a claim like `<SITE>-IAM-ADMIN` would
    // match no policy. (Distinct from NetBird `auto_groups`, which ARE site-prefixed
    // — that's the mesh-role axis. See docs/design/onboard-tier-role-model.md.)
    let groups: Vec<String> = ory.groups.clone();

    if dry_run {
        eprintln!(
            "dry-run: ssh {}@{} → GET {}/admin/identities?credentials_identifier=<email> (idempotency)",
            cfg.ssh_user, cfg.ssh_host, cfg.kratos_admin_url
        );
        eprintln!(
            "dry-run: if absent, POST {}/admin/identities  groups={groups:?} password={want_password} passkey={want_passkey}",
            cfg.kratos_admin_url
        );
        return Ok(None);
    }

    // 1. Idempotency: GET by email.
    let get_remote = format!(
        "curl -sS --max-time 30 -w '\\n%{{http_code}}' '{}/admin/identities?credentials_identifier={}'",
        cfg.kratos_admin_url,
        pct_encode_query(email)
    );
    let (code, body) = run_ssh_curl(&cfg, &get_remote, None).await?;
    if code != 200 {
        anyhow::bail!(
            "Ory Kratos GET /admin/identities returned HTTP {code} (expected 200) — body: {}",
            body.chars().take(300).collect::<String>()
        );
    }
    let existing = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.as_array().and_then(|a| a.first()).cloned());
    let existing_id = existing
        .as_ref()
        .and_then(|first| first.get("id"))
        .and_then(|id| id.as_str())
        .map(String::from);

    if let Some(id) = existing_id {
        // Idempotent skip of create — but a returning person (a restore, a
        // re-onboard) keeps the org and name from their FIRST onboarding unless
        // they are brought in line here, so an old org does not outlive the new manifest.
        // Only the org and name move; credentials, groups and state never do.
        let org_now = MfOrg {
            display_name: manifest.org.display_name.clone(),
            short_tag: org_trait_short_tag(&manifest.org, &manifest.affiliation).to_string(),
        };
        if let Some(found) = existing.as_ref() {
            let (ops, changes) = identity_resync_patch(found, name, &org_now);
            if !ops.is_empty() {
                let patch_remote = format!(
                    "curl -sS --max-time 30 -X PATCH -H 'Content-Type: application/json; charset=utf-8' \
                     --data-binary @- -w '\\n%{{http_code}}' '{}/admin/identities/{id}'",
                    cfg.kratos_admin_url
                );
                let patch_body = serde_json::Value::Array(ops).to_string();
                let (pcode, pbody) = run_ssh_curl(&cfg, &patch_remote, Some(&patch_body)).await?;
                if pcode != 200 {
                    anyhow::bail!(
                        "Ory Kratos PATCH /admin/identities/{id} (org/name resync) returned HTTP \
                         {pcode} — body: {}",
                        pbody.chars().take(300).collect::<String>()
                    );
                }
                for c in &changes {
                    eprintln!("note: Ory identity {id}: {c}");
                }
            }
        }
        let recovery_link = if want_passkey {
            Some(mint_recovery_link(&cfg, &id).await?)
        } else {
            None
        };
        eprintln!(
            "note: Ory identity for {email} already exists on `{envname}` (id {id}) — create \
             skipped (idempotent)."
        );
        return Ok(Some(OryProvision {
            auth_url: cfg.auth_url.into(),
            email: email.into(),
            password: None,
            recovery_link,
            already_existed: true,
            identity_id: Some(id),
        }));
    }

    // 2. Create the identity.
    let password = if want_password {
        Some(generate_password(24))
    } else {
        None
    };
    // Phase D gate: emit the `aff` claim only when GOAT_ORY_AFF_TRAIT_ENABLED is
    // set (the native analog of the Ansible `ory_site_affiliation_trait_enabled`
    // deploy var). Default off keeps today's behaviour + matches the Ansible path.
    let aff_enabled = std::env::var("GOAT_ORY_AFF_TRAIT_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let aff = aff_enabled.then_some(&manifest.affiliation);
    // The org identity trait's short_tag is the real ORG — which lives in the
    // manifest's `affiliation.id` (e.g. acme-corp), NOT `org/vendor.short_tag`.
    // On per-uuid goatnets the latter is forced to equal the handle (the Zenoh
    // prefix) by the short_tag==zenoh.trial.vendor drift-check, so copying it into
    // traits.org.short_tag produces a degenerate handle-as-org value (jane-doe
    // instead of acme-corp).
    // Prefer affiliation.id; fall back to the manifest org.short_tag only when the
    // manifest carries no affiliation (keeps pre-affiliation manifests working).
    let org_for_trait = MfOrg {
        display_name: manifest.org.display_name.clone(),
        short_tag: org_trait_short_tag(&manifest.org, &manifest.affiliation).to_string(),
    };
    let identity_body = build_identity_body(
        email,
        name,
        &groups,
        &org_for_trait,
        aff,
        password.as_deref(),
    );
    let create_remote = format!(
        "curl -sS --max-time 30 -X POST -H 'Content-Type: application/json; charset=utf-8' \
         --data-binary @- -w '\\n%{{http_code}}' '{}/admin/identities'",
        cfg.kratos_admin_url
    );
    let (code, body) = run_ssh_curl(&cfg, &create_remote, Some(&identity_body)).await?;
    if code != 200 && code != 201 {
        anyhow::bail!(
            "Ory Kratos POST /admin/identities returned HTTP {code} (expected 200/201) — body: {}. \
             (An HTTP 400 naming an unknown `aff`/trait means the env's identity schema lacks it; \
             native never sets `aff`, so this points at a schema drift.)",
            body.chars().take(400).collect::<String>()
        );
    }
    let new_id = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(String::from));

    // 3. Passkey recovery link (when requested).
    let recovery_link = if want_passkey {
        match &new_id {
            Some(id) => Some(mint_recovery_link(&cfg, id).await?),
            None => {
                eprintln!(
                    "warning: Ory identity created but the response carried no id — cannot mint a \
                     passkey recovery link; the password credential still works."
                );
                None
            }
        }
    } else {
        None
    };

    Ok(Some(OryProvision {
        auth_url: cfg.auth_url.into(),
        email: email.into(),
        password,
        recovery_link,
        already_existed: false,
        identity_id: new_id,
    }))
}

/// Render the Ory-login section of the bundle (empty when the leg didn't run).
/// Pure over the provision result so it's unit-testable.
fn render_ory_section(o: Option<&OryProvision>) -> String {
    let Some(o) = o else { return String::new() };
    let mut cred = String::new();
    if let Some(pw) = &o.password {
        cred.push_str(&format!("| Temporary password | `{pw}` |\n"));
    }
    if let Some(link) = &o.recovery_link {
        cred.push_str(&format!(
            "| Passkey-enrolment link (one-time, 15 min) | {link} |\n"
        ));
    }
    if o.already_existed && cred.is_empty() {
        cred.push_str("| Status | identity already existed — use your current credential |\n");
    }
    let note = if o.recovery_link.is_some() {
        "Open the passkey-enrolment link within 15 minutes to register a WebAuthn passkey. Share it OOB."
    } else if o.password.is_some() {
        "Change the temporary password at first login. Share it OOB, separately from this bundle."
    } else {
        "The identity was already provisioned — use your existing credential."
    };
    format!(
        "\n## Ory login (OIDC identity)\n\
         \n\
         This identity signs you in to the mesh web surfaces (portal, panoscope, dashboards) via \
         single sign-on. Log in at the URL below.\n\
         \n\
         | | |\n\
         |---|---|\n\
         | Login URL | {auth} |\n\
         | Email (username) | `{email}` |\n\
         {cred}\
         \n\
         {note}\n",
        auth = o.auth_url,
        email = o.email,
    )
}

// ============================ native revoke (P2.5) ============================
//
// The revoke sibling of `onboard_native`. Mirrors `operator-onboard.yml`'s revoke
// leg (Phases R1–R5) against the same surfaces with the same guards, natively:
//   R1   NetBird peers  — delete every peer named `peer-<handle>[-<tag>]-<env>-…`
//                         (the authoritative disconnect: the WireGuard tunnel drops)
//   R1.5 NetBird keys   — revoke the departing principal's setup keys (best-effort;
//                         only stops NEW joins, so failure is tolerated)
//   R2   Ory identity   — deactivate (state=inactive, reversible) or --purge DELETE
//   R4   Zenoh slot     — delete the portal org, GUARDED: kept when another active
//                         manifest shares its short_tag, unless org-scope forces it
//   R5   manifest       — flip `status: active` → `status: revoked`
//
// The `mistake this closes`: revoke used to have no native path, so a cohort revoke
// fell to a ~90-task Ansible run per handle (minutes each). This is the seconds-each
// peer of `onboard --native`. See docs/design/onboard-rotate-revoke-streamlining.md P2.5.

/// Shared, fetch-once context for a revoke run (single, org-cascade, or batch):
/// the portal client, the NetBird client + its peer/setup-key snapshots, the
/// active-manifest tag census for the teammate guard, and the resolved Ory
/// config. Built once so a batch of N handles makes one peer-list fetch, not N.
struct RevokeShared {
    portal: panoscope_core::portal::PortalClient,
    nb: Option<panoscope_core::netbird::NetbirdClient>,
    peers: Vec<panoscope_core::netbird::NbPeer>,
    // Retain lookup failure until the manifest tells us whether mesh is enabled.
    setup_keys: Result<Vec<panoscope_core::netbird::SetupKeyMeta>, String>,
    /// short_tag → the ACTIVE manifest handles (file stems) in THIS env's operators
    /// dir carrying it. The teammate guard (R4.3) counts entries other than the
    /// principal being revoked. A start-of-run snapshot (not mutated mid-batch), so
    /// a shared anchor is kept-on-doubt. Env-scoped + non-recursive so a person's
    /// legacy flat manifest doesn't count as a teammate of their per-env one.
    active_tag_handles: std::collections::HashMap<String, Vec<String>>,
    ory: Option<OryEnvConfig>,
    env_label: String,
    repo_root: PathBuf,
}

/// Console proposal states the revoke leg may offboard. It mirrors the
/// console's `onboardstore.OffboardableStatuses` (Go, one exported source of
/// truth in ops/goat-id-console/internal/onboardstore/proposals.go): goat-cli
/// ships apart from the console and cannot import that set, so
/// ops/ci/check-offboardable-status-parity.py fails CI when the two differ
/// (#579). A state added on one side only means the CLI silently stops
/// offboarding rows the console considers eligible.
const CONSOLE_OFFBOARDABLE_STATUSES: [&str; 3] = ["onboarded", "enrolled", "archived"];

/// The note the revoke leg sends with a console-row offboard, chosen per row
/// so it never destroys what an admin wrote. The console's note rule is
/// blank-keeps / non-blank-replaces (`MarkOffboardedFrom`), so:
///
/// - `onboarded` / `enrolled`: send a blank note, and the row's existing
///   note stays exactly as it was (a fixed sentence here wiped it — #1283).
/// - `archived`: the console refuses a blank note because the transition is
///   one-way (#1229), so send one that carries the old archive reason
///   forward instead of replacing it with a sentence that names the tool
///   and gives no reason.
///
/// The console truncates a decision note to `REVOKE_NOTE_MAX_BYTES` silently
/// from the tail, and an archive reason is stored uncapped, so a long reason
/// is bounded HERE, with a visible marker, so the cut never lands on the end
/// of the very reason this note exists to carry forward.
fn revoke_offboard_note(status: &str, existing_note: &str) -> String {
    if !status.eq_ignore_ascii_case("archived") {
        return String::new();
    }
    let reason = existing_note.trim();
    if reason.is_empty() {
        return "offboarded by goat-cli revoke".to_string();
    }
    const PREFIX: &str = "offboarded by goat-cli revoke; archived because: ";
    const MARKER: &str = " […]";
    let budget = REVOKE_NOTE_MAX_BYTES - PREFIX.len();
    if reason.len() <= budget {
        return format!("{PREFIX}{reason}");
    }
    let keep = budget - MARKER.len();
    let mut cut = keep;
    while !reason.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{PREFIX}{}{MARKER}", &reason[..cut])
}

/// The console's decision-note byte cap (`maxDecisionNoteBytes` in
/// goat-id-console); a longer note is cut from the tail without a word.
const REVOKE_NOTE_MAX_BYTES: usize = 4096;

/// The note for a console that answers 409 to a row offboard. The leg stays
/// `failed` either way, because the row is still live and blocks a fresh
/// invite; what changes is which layer the note sends the operator to (#579).
///
/// goat-cli ships apart from the per-goatnet console (ADR 1083), and consoles
/// built before #456 offboard only `onboarded` rows. During a rollout a
/// current CLI lists an `enrolled` or `archived` row as eligible, and the old
/// console refuses it. "Offboard it in the identity console" would send the
/// operator to a console that cannot do it yet, so for any status other than
/// `onboarded` the note names the console's release as the thing to fix. A
/// current console answers 409 only when the row changed state after it was
/// listed, which a re-run settles.
fn offboard_conflict_note(id: &str, handle: &str, env: &str, status: &str) -> String {
    if status.eq_ignore_ascii_case("onboarded") {
        return format!(
            "  note: console offboard for proposal {id} (handle {handle} env {env}) did not complete — the onboarding console row is still live and will block a fresh invite until it is offboarded in the identity console."
        );
    }
    format!(
        "  note: the identity console for {env} refused to offboard proposal {id} (handle {handle}), listed as {status}. Consoles built before #456 offboard only onboarded rows: if the {env} console predates it, deploy its current release, then re-run this revoke. If it is current, the row changed state after it was listed; re-run this revoke. Until then the row is still live and blocks a fresh invite."
    )
}

/// Offboard EVERY offboard-eligible console proposal row for `handle` in
/// `env_name`. Returns "offboarded" when every eligible proposal was marked
/// offboarded as part of this revoke, "none" when no eligible proposal
/// exists, or "failed" when any console call did not succeed (HTTP
/// 409/other error). Best-effort: a failure is recorded in the outcome and
/// audit but does not abort other revoke legs.
async fn revoke_console_proposal(
    portal: &panoscope_core::portal::PortalClient,
    handle: &str,
    env_name: &str,
    dry_run: bool,
) -> anyhow::Result<&'static str> {
    let mut rows = portal.list_proposals("").await?;
    rows.retain(|p| {
        p.handle.eq_ignore_ascii_case(handle)
            && p.env.eq_ignore_ascii_case(env_name)
            && CONSOLE_OFFBOARDABLE_STATUSES
                .iter()
                .any(|s| p.status.eq_ignore_ascii_case(s))
    });
    // Offboard ALL eligible rows, not a chosen one: the re-enrollment path
    // (offboard -> re-invite -> re-onboard) can leave several rows for one
    // handle+env, and a leftover `archived` row still blocks a fresh invite
    // on the same email (the #414 duplicate-email guard treats it as
    // active). Terminal rows (`offboarded` / `rejected` / `failed`) are
    // excluded above, so they are never decided (the #196 dead-end).
    if rows.is_empty() {
        // No offboard-eligible row (none exists, or all are already terminal).
        return Ok("none");
    }
    if dry_run {
        for p in &rows {
            eprintln!(
                "dry-run: console offboard for proposal {} (handle {} env {})",
                p.id, p.handle, p.env
            );
        }
        return Ok("offboarded");
    }
    let mut any_failed = false;
    let mut any_offboarded = false;
    for p in &rows {
        match portal
            .decide_proposal(&p.id, "offboard", &revoke_offboard_note(&p.status, &p.note))
            .await
        {
            Ok(_) => any_offboarded = true,
            Err(e) if e.http_status().map(|s| s.as_u16()) == Some(409) => {
                eprintln!(
                    "{}",
                    offboard_conflict_note(&p.id, &p.handle, &p.env, &p.status)
                );
                any_failed = true;
            }
            // Row vanished between list and decide: already gone, not a failure.
            Err(PortalError::Http { status, .. }) if status.as_u16() == 404 => {}
            // The key was refused: every later row would be refused the same
            // way, so warn once and stop instead of sending N doomed requests.
            // Every remaining row stays uncut, so the leg still reads failed.
            Err(PortalError::Http { status, .. }) if matches!(status.as_u16(), 401 | 403) => {
                eprintln!(
                    "  warning: console offboard for proposal {} (handle {} env {}) was refused ({status}) — the operator key is not accepted, so the remaining eligible rows were not attempted; fix the key and re-run this revoke.",
                    p.id, p.handle, p.env
                );
                any_failed = true;
                break;
            }
            // Any other failure is this row's, not the leg's: record it and
            // keep going, so one transient 500 or timeout cannot leave every
            // later eligible row uncut (#578).
            Err(e) => {
                eprintln!(
                    "  warning: console offboard for proposal {} (handle {} env {}) failed: {e} — the onboarding console row is still live; re-run this revoke or offboard the proposal in the identity console before re-inviting.",
                    p.id, p.handle, p.env
                );
                any_failed = true;
            }
        }
    }
    Ok(if any_failed {
        "failed"
    } else if any_offboarded {
        "offboarded"
    } else {
        "none"
    })
}

/// Result of the portal-key leg (R3), for the receipt + the any-failed gate.
#[derive(Debug)]
struct OperatorKeyRevoke {
    /// Compact receipt tag: "none" (held nothing), "revoked=N closed=M", plus
    /// " failed=K" when a per-key decide did not land.
    summary: String,
    /// True when at least one active-key revoke or pending-request close failed,
    /// so the overall revoke is reported `partial`, never a silent success.
    any_failed: bool,
}

/// Revoke every PORTAL KEY (the per-person `op_` key) the person holds, across
/// ALL goatnets (R3). The mesh and login legs do not touch this credential, so
/// without this leg an offboarded admin keeps a live portal key.
///
/// Two live states are actioned, going by the key TABLE, never by role — anyone
/// *can* hold a key, so a role check would miss key-holders the manifest
/// doesn't mark as admins:
///   - `active` → revoke (removes the portal's key file; it stops accepting it)
///   - `requested` → reject (close the pending request so an admin cannot approve
///     it after the person is gone)
///
/// Handle match is case-insensitive and NOT env-scoped: a person can hold keys
/// on more than one goatnet and every one must be cut. "All goatnets" means
/// every env-labelled row in the console this profile points at — the key
/// table is one table with an `env` column; a key recorded in a DIFFERENT
/// goatnet's console instance is out of this run's reach and needs a run
/// against that profile. Best-effort per key — a per-key failure is recorded
/// in the summary and reported, never aborts the rest. Returns "none" when the
/// person held no key in a live state.
///
/// The reject arm goes through the console's requester ≠ decider rule
/// (two-person, #210). Offboarding somebody ELSE passes it by construction;
/// a self-offboard — or a row whose owner identity the console cannot compare
/// — is refused 403 and lands as `failed` → `partial`, deliberately: a second
/// person must finish that cut.
async fn revoke_operator_keys(
    portal: &panoscope_core::portal::PortalClient,
    handle: &str,
    dry_run: bool,
) -> Result<OperatorKeyRevoke, PortalError> {
    // One listing across every state, filtered client-side by handle — mirrors
    // the console-proposal leg (R0). `all` is LOAD-BEARING: the console
    // defaults an empty/missing status to REQUESTED only, which would hide
    // every ACTIVE key from this leg and leave live keys standing.
    let rows = portal.list_operator_keys("all").await?;
    let mine: Vec<_> = rows
        .iter()
        .filter(|r| r.handle.eq_ignore_ascii_case(handle))
        .collect();

    let mut revoked = 0usize;
    let mut closed = 0usize;
    let mut failed = 0usize;
    let note = "offboard: portal-key leg of `goat admin onboard --revoke`";

    for r in &mine {
        // The verb is chosen by the key's live state; terminal rows
        // (revoked/rejected) are already cut, so they are skipped.
        let action = match r.status.trim().to_ascii_lowercase().as_str() {
            "active" => "revoke",
            "requested" => "reject",
            _ => continue,
        };
        if dry_run {
            eprintln!(
                "dry-run: {action} portal key {} (handle {} env {} status {})",
                r.id, r.handle, r.env, r.status
            );
            if action == "revoke" {
                revoked += 1;
            } else {
                closed += 1;
            }
            continue;
        }
        match portal.decide_operator_key(&r.id, action, note).await {
            Ok(_) => {
                if action == "revoke" {
                    revoked += 1;
                } else {
                    closed += 1;
                }
            }
            // Gone from the server entirely: nothing is left live, not our
            // failure. (The console keeps terminal rows, so a 404 here means
            // the row itself vanished, not that it went terminal — that case
            // is the 409 below.)
            Err(PortalError::NotFound { .. }) => {}
            Err(PortalError::Http { status, .. }) if status.as_u16() == 404 => {}
            // 409 wrong_state: the row left the state we listed it in during
            // the list→decide window. Usually someone else already cut it
            // (benign), but a REJECT can 409 because the pending request was
            // APPROVED in the window — which is a LIVE key. Re-read the row
            // and act on what it is NOW; only a still-live key that cannot be
            // cut counts as a failure. One retry, no recursion.
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                match portal.list_operator_keys("all").await {
                    Ok(fresh) => {
                        let now = fresh
                            .iter()
                            .find(|f| f.id == r.id)
                            .map(|f| f.status.trim().to_ascii_lowercase());
                        let retry = match now.as_deref() {
                            Some("active") => Some("revoke"),
                            Some("requested") => Some("reject"),
                            // Terminal or gone: already cut, nothing live.
                            _ => None,
                        };
                        if let Some(retry) = retry {
                            match portal.decide_operator_key(&r.id, retry, note).await {
                                Ok(_) => {
                                    if retry == "revoke" {
                                        revoked += 1;
                                    } else {
                                        closed += 1;
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "  warning: portal key {} (env {}) changed state mid-run and the {retry} failed: {e}",
                                        r.id, r.env
                                    );
                                    failed += 1;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  warning: portal-key {action} for {} (env {}) returned wrong-state and the re-read failed: {e}",
                            r.id, r.env
                        );
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "  warning: portal-key {action} for {} (env {}) failed: {e}",
                    r.id, r.env
                );
                failed += 1;
            }
        }
    }

    let any_failed = failed > 0;
    let summary = if revoked == 0 && closed == 0 && failed == 0 {
        "none".to_string()
    } else {
        let mut s = format!("revoked={revoked} closed={closed}");
        if failed > 0 {
            s.push_str(&format!(" failed={failed}"));
        }
        s
    };
    if summary == "none" {
        eprintln!("  portal-key: no portal keys for {handle} — nothing to cut");
    }
    Ok(OperatorKeyRevoke { summary, any_failed })
}

/// Result of the SSH-key residual leg (R3.5), for the receipt + the any-failed gate.
#[derive(Debug)]
struct SshKeyResidual {
    /// Compact receipt tag: "none" (no live rows), else "still-installed:N"
    /// and/or "closed:M", plus " failed:K" when a pending close did not land.
    /// The counts survive a failure: a close error must never hide a live login.
    summary: String,
    /// True when a key is still installed or a pending close failed. Either
    /// way the person has, or can still get, a shell, so the revoke is partial.
    partial: bool,
}

/// SSH-key residual leg (R3.5), show half (goat#371). The native path cannot
/// remove a key file from the hosts, so installed rows are REPORTED, loudly,
/// and their record is left telling the truth: flipping an installed row to
/// revoked while the file survives would hide the residual. Rows that never
/// reached a host (submitted/approved) ARE closed, so a key cannot be installed
/// after the person is gone.
///
/// Handle match is case-insensitive and NOT env-scoped, for the same reason as
/// the portal-key leg (R3): the console's key table spans every goatnet it
/// records. Best-effort per key — a failed close is counted, never aborts.
async fn revoke_ssh_keys(
    portal: &panoscope_core::portal::PortalClient,
    handle: &str,
    dry_run: bool,
) -> Result<SshKeyResidual, PortalError> {
    // `all` is LOAD-BEARING, as in the portal-key leg: the console defaults an
    // empty status to SUBMITTED only, which would hide installed and approved rows.
    let rows = portal.list_ssh_keys("all").await?;
    let (mut installed, mut closed, mut failed) = (0usize, 0usize, 0usize);
    for r in rows
        .iter()
        .filter(|r| r.handle.eq_ignore_ascii_case(handle))
    {
        match r.status.as_str() {
            "installed" => {
                installed += 1;
                eprintln!("{}", still_installed_warning(r));
            }
            "submitted" | "approved" => match close_pending_ssh_key(portal, r, dry_run).await {
                Ok(true) => closed += 1,
                Ok(false) => {}
                Err(e) => {
                    eprintln!("  warning: shell-key close {} failed: {e}", r.id);
                    failed += 1;
                }
            },
            // rejected / revoked: already cut.
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if installed > 0 {
        parts.push(format!("still-installed:{installed}"));
    }
    if closed > 0 {
        parts.push(format!("closed:{closed}"));
    }
    if failed > 0 {
        parts.push(format!("failed:{failed}"));
    }
    let summary = if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(" ")
    };
    Ok(SshKeyResidual {
        summary,
        partial: installed > 0 || failed > 0,
    })
}

/// Close one pre-install shell-key grant. `Ok(true)` when this run closed it
/// (or would, on a dry-run); `Ok(false)` when it was already cut — gone (404),
/// or rejected/revoked between the list and the decide (409: the console's
/// revoke accepts every live state, so a conflict means the row is terminal);
/// `Err` when the close did not land.
async fn close_pending_ssh_key(
    portal: &panoscope_core::portal::PortalClient,
    r: &panoscope_core::portal::SshKey,
    dry_run: bool,
) -> Result<bool, PortalError> {
    if dry_run {
        eprintln!(
            "dry-run: close pending shell-key grant {} (env `{}`)",
            r.id, r.env
        );
        return Ok(true);
    }
    match portal.decide_ssh_key(&r.id, "revoke").await {
        Ok(_) => Ok(true),
        Err(e) if matches!(e.http_status().map(|s| s.as_u16()), Some(404 | 409)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// The warning for a shell key still on the hosts. It names a cleanup that
/// works for every key: the ansible revoke (Phase R1.6) removes only the
/// manifest's `operator.ssh_pubkey`, and a key installed from the console
/// queue is never written to the manifest, so R1.6 alone can leave it in
/// place. The fingerprint is the one `ssh-keygen -lf authorized_keys` prints.
fn still_installed_warning(r: &panoscope_core::portal::SshKey) -> String {
    let fingerprint = if r.fingerprint.is_empty() {
        "no fingerprint recorded"
    } else {
        r.fingerprint.as_str()
    };
    format!(
        "  warning: shell key {id} ({fingerprint}, env `{env}`) is STILL INSTALLED on the hosts.\n           \
         The native revoke cannot remove authorized_keys entries, and the ansible revoke\n           \
         (Phase R1.6) removes only the manifest's operator.ssh_pubkey. Remove this key's\n           \
         line from authorized_keys on the `{env}` hosts, then close the record with\n           \
         `goat admin ssh-key revoke {id}`.",
        id = r.id,
        env = r.env,
    )
}

/// Per-handle revoke result, for the batch/cascade summary.
struct RevokeOutcome {
    handle: String,
    peers_cut: usize,
    ory: &'static str, // "deactivated" | "purged" | "already-absent" | "skipped" | "failed"
    zenoh: &'static str, // "deleted" | "kept-teammates" | "none" | "already-absent" | "failed"
    manifest: &'static str, // "revoked" | "already-revoked" | "no-status-line"
    /// Console proposal row leg: "offboarded" when every offboard-eligible
    /// proposal (a status in `CONSOLE_OFFBOARDABLE_STATUSES`) was
    /// marked offboarded as part of the revoke, "none" when no matching
    /// offboard-eligible proposal was found, or "failed" when any
    /// console call did not succeed (HTTP 409/other error). Included
    /// in the receipt so a failure of this leg is never reported
    /// as overall success.
    proposal: &'static str,
    /// SSH-key residual leg, show half (goat#371): "still-installed:N"
    /// counts keys whose file survives on the hosts (reported, record left
    /// truthful — the native path cannot remove authorized_keys entries),
    /// "closed:M" counts pre-install grant rows revoked so nothing installs
    /// after the person is gone, "failed:K" counts closes that did not land
    /// (alongside the other counts, never instead of them), "none" when the
    /// handle has no live rows, "skipped" with no portal, "failed" alone when
    /// the console could not be consulted. still-installed and any failure
    /// grade the audit partial.
    ssh_keys: String,
    /// The collaboration-environment DECISION this revoke recorded —
    /// "n/a" (nothing on that plane), "keep", "revoke-requested" (the cut is
    /// `onboard-runner -collab revoke`), or "undecided" (no `--collab` passed
    /// for someone who has collab). Never a cut this command performed: goat-cli
    /// holds no collab credentials.
    collab: &'static str,
    /// Portal-key leg (R3): "none" when the person held no portal key, else
    /// "revoked=N closed=M" (active keys cut, pending requests closed), plus
    /// " failed=K" when a per-key decide did not land. Owned (unlike the other
    /// tags) because it carries counts. Feeds the any-failed gate.
    operator_keys: String,
    /// NetBird user leg (R2.5): "blocked" | "already-blocked" | "absent" (no
    /// NetBird account for the identity) | "skipped" | "failed". The NetBird
    /// user id of a person who signs in through Ory is their Kratos identity
    /// id, so this leg runs only when R2 resolved one.
    netbird_user: &'static str,
    /// True when ANY leg (ory/zenoh/manifest/proposal/operator_keys) failed.
    /// Computed once here (the same boolean `revoke_one` already used to tag
    /// the audit event's "partial" status) and now actually reaches the
    /// caller — before this field existed, a revoke that cut the mesh peer
    /// but left the Ory login and portal op_ key alive printed a quiet
    /// summary line and exited 0, because nothing downstream of `revoke_one`
    /// ever inspected the per-leg outcomes it already computed.
    any_failed: bool,
}

impl RevokeOutcome {
    /// Whether this outcome should count as a failure for exit-code/summary
    /// purposes — `any_failed`, but never on a dry-run (nothing was actually
    /// attempted, so there is nothing to have failed). Was three independent
    /// copies of `any_failed && !dry_run` across the three revoke call
    /// sites; a future change to this rule now only needs to land here.
    fn should_fail(&self, dry_run: bool) -> bool {
        self.any_failed && !dry_run
    }
}

/// Native-revoke eligibility: serviceable when the env has a native NetBird +
/// Ory config. Returns `Some(reason)` to defer to Ansible,
/// `None` when native can do it. `Err` on a missing/unparseable manifest (the
/// caller then silently falls back to Ansible, whose own error is clearer).
fn native_revoke_eligibility(
    repo_root: Option<&Path>,
    handle: &str,
    env: Option<&str>,
    profile: &crate::profile::Profile,
) -> anyhow::Result<Option<String>> {
    let cwd = std::env::current_dir()?;
    let root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd)
            .ok_or_else(|| anyhow::anyhow!("no .git ancestor from {}", cwd.display()))?,
    };
    let operators_dir = root.join("ops/operators");
    let manifest_path = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest_path.exists() {
        anyhow::bail!("no manifest at {}", manifest_path.display());
    }
    // Parse to surface an unparseable manifest as Err (silent Ansible fallback).
    let _m: OnboardManifest = serde_yaml::from_str(&std::fs::read_to_string(&manifest_path)?)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;
    if native_netbird_config_for(env, profile).is_none() {
        return Ok(Some(format!(
            "no NetBird/Ory facts for goatnet `{}` — run `goat context refresh` in that goatnet's context",
            env.unwrap_or("(none)")
        )));
    }
    Ok(None)
}

/// Does this peer name belong to the given principal? Anchored on the full keyed
/// prefix so an org-mate's peer (which carries a SHARED tag mid-name) is never
/// falsely attributed — the exact bug the identity-keyed sweep hit on
/// . The rule itself lives in ops/identity/peer-name-grammar.yaml,
/// generated into panoscope_core::peername and proven by the same conformance
/// vectors the portal's Go implementation runs; the playbook's R1.2 regex
/// `^peer-<handle><tag_seg>-<site>-` is the third expression of it, fenced by
/// ops/ci/check-peer-name-grammar-sync.sh.
fn revoke_peer_matches(name: &str, handle: &str, tag: &str, env: &str) -> bool {
    peername::belongs_to(name, handle, tag, env)
}

/// The operators dir for an env: `ops/operators/<env>` per-env, else the flat
/// `ops/operators`. The teammate-guard census + all manifest reads scope to this.
fn operators_env_dir(repo_root: &Path, env: Option<&str>) -> PathBuf {
    match env {
        Some(env) => repo_root.join("ops/operators").join(env),
        None => repo_root.join("ops/operators"),
    }
}

/// Census active-manifest handles per short_tag in ONE operators dir
/// (non-recursive) — the teammate-guard input (R4.3). Scoped to the env dir so a
/// person's legacy flat manifest never counts as a teammate of their per-env one;
/// the handle is the file stem (the authoritative key the onboard/revoke uses). A
/// shared Zenoh anchor must not be deleted while another active member of the same
/// goatnet still publishes under it.
fn census_active_tag_handles(env_dir: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(env_dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let Some(handle) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(m) = serde_yaml::from_str::<OnboardManifest>(&text) else {
            continue;
        };
        let tag = m.org.short_tag.trim();
        if !tag.is_empty() && m.identity.status.trim() == "active" {
            out.entry(tag.to_string())
                .or_default()
                .push(handle.to_string());
        }
    }
    out
}

/// Flip a manifest's `status: active` → `status: revoked` in place (R5). Returns
/// the outcome tag. Idempotent: an already-revoked manifest is a no-op. Matches
/// the 2-space-indented `status:` under the `identity:` block.
fn flip_manifest_status_to_revoked(path: &Path) -> anyhow::Result<&'static str> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    if text.contains("\n  status: revoked") {
        return Ok("already-revoked");
    }
    let new = text.replacen("\n  status: active", "\n  status: revoked", 1);
    if new == text {
        return Ok("no-status-line");
    }
    std::fs::write(path, new).map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
    Ok("revoked")
}

/// Build the shared, fetch-once revoke context for `env`.
async fn build_revoke_shared(
    ctx: &super::Ctx,
    env: Option<&str>,
    repo_root: &Path,
) -> anyhow::Result<RevokeShared> {
    let portal = ctx.profile.portal_client()?;
    // The goatnet this revoke NAMES in its output. --env, else the active
    // context; never a hardcoded goatnet.
    let env_label = audit_goatnet(env, &ctx.profile).to_string();

    // NetBird client + peer/key snapshots (once). None when the env has no native
    // config or the token is unset — the peer-cut leg is then skipped with a note.
    let (nb, peers, setup_keys) = match native_netbird_config_for(env, &ctx.profile) {
        Some(cfg) => {
            let token = std::env::var(cfg.token_env_var).unwrap_or_default();
            if token.trim().is_empty() {
                eprintln!(
                    "note: NetBird token env `{}` unset — the peer-cut leg is skipped \
                     (set it to cut mesh peers). Zenoh + Ory + manifest legs still run.",
                    cfg.token_env_var
                );
                (None, Vec::new(), Ok(Vec::new()))
            } else {
                let client =
                    panoscope_core::netbird::NetbirdClient::new(cfg.mgmt_url.to_string(), token);
                let peers = client.list_peers().await.unwrap_or_else(|e| {
                    eprintln!("warning: NetBird list_peers failed ({e}) — peer-cut leg degraded");
                    Vec::new()
                });
                let keys = client.list_setup_keys().await
                    .map_err(|e| format!("NetBird setup-key snapshot failed; cannot verify revocation: {e}"));
                (Some(client), peers, keys)
            }
        }
        None => (None, Vec::new(), Ok(Vec::new())),
    };

    Ok(RevokeShared {
        portal,
        nb,
        peers,
        setup_keys,
        active_tag_handles: census_active_tag_handles(&operators_env_dir(repo_root, env)),
        ory: native_ory_config_for(env, &ctx.profile),
        env_label,
        repo_root: repo_root.to_path_buf(),
    })
}

/// Revoke ONE handle across the five surfaces, using the shared context. Legs are
/// best-effort past the peer cut: a failure on one surface is recorded in the
/// outcome and reported, never aborts the others (a partial revoke that leaves a
/// surface up must be VISIBLE, not silent — F-177 discipline). `org_scope=true`
/// (org cascade) force-deletes a shared Zenoh anchor; otherwise the teammate guard
/// keeps it.
// One more parameter than clippy's default: the revoke surfaces are a flat
// list by nature (each is an independent cut) and the collab decision joins them.
#[allow(clippy::too_many_arguments)]
async fn revoke_one(
    ctx: &super::Ctx,
    shared: &RevokeShared,
    handle: &str,
    env: Option<&str>,
    purge: bool,
    org_scope: bool,
    dry_run: bool,
    collab: Option<CollabDecision>,
) -> anyhow::Result<RevokeOutcome> {
    let operators_dir = shared.repo_root.join("ops/operators");
    let manifest_path = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest_path.exists() {
        anyhow::bail!(
            "no operator manifest at {} — nothing to revoke",
            manifest_path.display()
        );
    }
    let manifest: OnboardManifest = serde_yaml::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;

    let tag = manifest.org.short_tag.trim().to_string();
    let env_name = shared.env_label.as_str();

    // Audit start, mirroring onboard-native.
    let audit_actor = crate::cmd::audit::actor_handle(&ctx.profile);
    // The collaboration-environment decision. Resolved BEFORE the cuts so
    // it rides in the audit event's args digest from `started` onward — the
    // decision is part of what was intended, not a footnote to what happened.
    let collab_outcome = collab_decision_outcome(&manifest, collab);
    let audit_digest = format!(
        "revoke-native handle={handle} env={env_name} purge={purge} org_scope={org_scope} \
         collab={collab_outcome}"
    );
    if !dry_run {
        crate::cmd::audit::emit(
            &ctx.profile,
            crate::cmd::audit::OperatorActionV1::new(
                &audit_actor,
                "goat-cli",
                "admin.revoke-native",
                handle,
                env_name,
                "started",
            )
            .with_args_digest(&audit_digest),
        )
        .await;
    }

    // ---- R0: console proposal row (offboard) ----
    let proposal_outcome: &'static str = match revoke_console_proposal(
        &shared.portal,
        handle,
        env_name,
        dry_run,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("  warning: console offboard leg failed for {handle}: {e}");
            eprintln!(
                "           the onboarding console row is still live; re-run this revoke or offboard the proposal in the identity console before re-inviting."
            );
            if shared.portal.onboard_endpoint_is_fallback() {
                eprintln!(
                    "           hint: {}",
                    super::vendor::ONBOARD_ENDPOINT_FALLBACK_HINT
                );
            }
            "failed"
        }
    };

    // ---- R1: NetBird peer cut (authoritative disconnect) ----
    let targets: Vec<&panoscope_core::netbird::NbPeer> = if manifest.netbird.enabled {
        shared
            .peers
            .iter()
            .filter(|p| revoke_peer_matches(&p.name, handle, &tag, env_name))
            .collect()
    } else {
        Vec::new()
    };
    let mut peers_cut = 0usize;
    for p in &targets {
        if dry_run {
            eprintln!("dry-run: DELETE peer {} ({})", p.name, p.id);
            peers_cut += 1;
            continue;
        }
        if let Some(nb) = shared.nb.as_ref() {
            match nb.delete_peer(&p.id).await {
                Ok(()) => peers_cut += 1,
                Err(panoscope_core::netbird::NetbirdError::NotFound) => {} // already gone
                Err(e) => eprintln!("  warning: peer {} delete failed: {e}", p.name),
            }
        }
    }

    // ---- R1.5: setup keys must be revoked to prevent new joins ----
    let mut setup_keys_failed = false;
    if manifest.netbird.enabled {
        match (&shared.nb, &shared.setup_keys) {
            (_, Err(error)) => {
                setup_keys_failed = true;
                eprintln!("  warning: setup-key revocation unverified for {handle}: {error}");
            }
            (None, _) => {
                setup_keys_failed = true;
                eprintln!("  warning: setup-key revocation unverified for {handle}: NetBird client unavailable");
            }
            (Some(nb), Ok(keys)) => {
                for key in keys.iter().filter(|key| {
                    !key.revoked && revoke_peer_matches(&key.name, handle, &tag, env_name)
                }) {
                    if dry_run {
                        eprintln!("dry-run: revoke setup-key {} ({})", key.name, key.id);
                    } else if let Err(error) = nb.revoke_setup_key(key).await {
                        setup_keys_failed = true;
                        eprintln!("  warning: setup-key `{}` revoke failed: {error}", key.name);
                    }
                }
            }
        }
    }

    // ---- R2: Ory identity (deactivate, or --purge DELETE) ----
    let email = manifest.identity.email.trim().to_string();
    let revoke_targets =
        revoke_target_directory(&ctx.profile, env_name, handle, &email, shared.ory.as_ref());
    let (ory_outcome, _identity_id): (&'static str, Option<String>) =
        if !manifest.ory_site.enabled || email.is_empty() {
            ("skipped", None)
        } else if let Some(cfg) = shared.ory.as_ref() {
            revoke_ory_identity(cfg, &email, purge, dry_run, &revoke_targets)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("  warning: Ory revoke leg failed for {email}: {e}");
                    ("failed", None)
                })
        } else {
            eprintln!(
                "  note: no native Ory config for env `{env_name}` — Ory identity NOT deactivated; \
                 run the Ansible revoke to cut the login."
            );
            ("skipped", None)
        };

    // ---- R2.5: NetBird user block ----
    // Deleting the peers (R1) cuts the devices; blocking the account stops a
    // new enrolment through SSO and, on a replicated goatnet, is recorded by
    // the revocation guard so no other site's write can undo it (ADR 1193).
    let netbird_user_outcome = if !manifest.netbird.enabled || !manifest.ory_site.enabled {
        "skipped"
    } else if dry_run {
        "would-block"
    } else {
        // Reload independently even if PATCH/DELETE failed after checkpointing.
        // A restarted purge no longer needs the deleted Kratos row to identify
        // an OIDC user whose NetBird email/name fields can be empty.
        match (read_revoke_targets(&revoke_targets), shared.nb.as_ref()) {
            (Ok(ids), Some(nb)) if !ids.is_empty() => {
                let mut failed = false;
                for id in ids {
                    if let Err(e) = block_netbird_user(nb, &id, false).await {
                        eprintln!("  warning: NetBird user block failed for {id}: {e}");
                        failed = true;
                    }
                }
                if failed {
                    "failed"
                } else {
                    "verified"
                }
            }
            (result, _) => {
                eprintln!("  warning: NetBird account cut unverifiable: no configured client or durable identity target ({result:?})");
                "failed"
            }
        }
    };

    // ---- R3: portal keys (op_) — revoke active, close pending, all goatnets ----
    // This credential survives every other leg. Goes by the key table, never by role. Best-effort
    // like the other portal legs — a per-key failure is recorded, never aborts.
    let operator_keys = match revoke_operator_keys(&shared.portal, handle, dry_run).await {
        Ok(o) => o,
        // No portal configured on this profile: no portal keys can exist to
        // cut, so this is "skipped", never a spurious `partial`.
        Err(PortalError::Disabled) => OperatorKeyRevoke {
            summary: "skipped".to_string(),
            any_failed: false,
        },
        Err(e) => {
            eprintln!(
                "  warning: portal-key list failed for {handle}: {e}\n           \
                 portal keys were NOT cut — re-run this revoke, or use \
                 `goat admin operator-key list --status active` then `revoke <id>`."
            );
            OperatorKeyRevoke {
                summary: "failed".to_string(),
                any_failed: true,
            }
        }
    };

    // ---- R3.5: SSH-key residual, show half (goat#371) ----
    // Installed keys are reported and grade the run partial; pre-install
    // grants are closed. Contract on `revoke_ssh_keys`.
    let ssh_keys = match revoke_ssh_keys(&shared.portal, handle, dry_run).await {
        Ok(o) => o,
        // No portal configured on this profile: no console rows to read.
        Err(PortalError::Disabled) => SshKeyResidual {
            summary: "skipped".to_string(),
            partial: false,
        },
        Err(e) => {
            eprintln!("  warning: shell-key list failed for {handle}: {e}");
            eprintln!(
                "           any installed shell key is still live and UNREPORTED; check `goat admin ssh-key` / the console queue."
            );
            SshKeyResidual {
                summary: "failed".to_string(),
                partial: true,
            }
        }
    };

    // ---- R4: Zenoh slot delete, teammate-guarded ----
    let zenoh_outcome: &'static str = match manifest
        .zenoh
        .trial
        .as_ref()
        .map(|t| t.org.trim().to_string())
    {
        Some(vendor) if !vendor.is_empty() => {
            // R4.3 guard: count OTHER active manifests (same env, by handle)
            // sharing this tag. Excluding self by handle (not by count) means a
            // person's own duplicate/legacy entry can't masquerade as a teammate.
            let shared_others = if tag.is_empty() {
                0
            } else {
                shared
                    .active_tag_handles
                    .get(&tag)
                    .map(|hs| hs.iter().filter(|h| h.as_str() != handle).count())
                    .unwrap_or(0)
            };
            if !org_scope && shared_others > 0 {
                "kept-teammates"
            } else if dry_run {
                eprintln!("dry-run: DELETE portal org `{vendor}`");
                "deleted"
            } else {
                match shared.portal.delete_vendor(&vendor).await {
                    Ok(()) => "deleted",
                    Err(PortalError::NotFound { .. }) => "already-absent",
                    Err(e) => {
                        eprintln!("  warning: portal delete_vendor `{vendor}` failed: {e}");
                        "failed"
                    }
                }
            }
        }
        _ => "none",
    };

    // ---- R5: manifest status flip ----
    let manifest_outcome = if dry_run {
        "revoked"
    } else {
        flip_manifest_status_to_revoked(&manifest_path).unwrap_or_else(|e| {
            eprintln!("  warning: manifest status flip failed: {e}");
            "failed"
        })
    };

    // Audit finish. "ok" unless a surface hard-failed.
    let any_failed = setup_keys_failed
        || ory_outcome == "failed"
        || netbird_user_outcome == "failed"
        || zenoh_outcome == "failed"
        || manifest_outcome == "failed"
        || proposal_outcome == "failed"
        || operator_keys.any_failed
        // A still-installed shell key means the person keeps a working
        // login: that is a partial cut however cleanly the other legs ran.
        || ssh_keys.partial;
    if !dry_run {
        let mut a = crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            "admin.revoke-native",
            handle,
            env_name,
            if any_failed { "partial" } else { "ok" },
        )
        .with_args_digest(&audit_digest);
        if any_failed {
            a = a.with_error(format!(
                "setup_keys={} ory={ory_outcome} netbird_user={netbird_user_outcome} zenoh={zenoh_outcome} manifest={manifest_outcome} proposal={proposal_outcome} operator_keys={} ssh_keys={}",
                if setup_keys_failed { "failed" } else { "ok" },
                operator_keys.summary, ssh_keys.summary
            ));
        }
        crate::cmd::audit::emit(&ctx.profile, a).await;
    }

    // Say the collab half out loud on the spot: an operator who cut the mesh and
    // walked away is the F-274 failure mode, and the line below is the only place
    // the still-live account gets mentioned at the moment of the cut.
    print_collab_follow_up(handle, collab_outcome);

    Ok(RevokeOutcome {
        handle: handle.to_string(),
        peers_cut,
        ory: ory_outcome,
        zenoh: zenoh_outcome,
        manifest: manifest_outcome,
        proposal: proposal_outcome,
        ssh_keys: ssh_keys.summary,
        collab: collab_outcome,
        operator_keys: operator_keys.summary,
        netbird_user: netbird_user_outcome,
        any_failed,
    })
}

/// Resolve the collab receipt value from the manifest's collab block + the
/// operator's `--collab` choice. `n/a` when the person has nothing on that plane
/// — no decision to make, so none is implied.
fn collab_decision_outcome(
    manifest: &OnboardManifest,
    collab: Option<CollabDecision>,
) -> &'static str {
    let has_collab = manifest.collab.enabled || !manifest.collab.account.trim().is_empty();
    match (has_collab, collab) {
        (false, None) => "n/a",
        // An explicit choice for someone the manifest shows no collab for is still
        // recorded: the manifest may simply not carry the record yet (that IS
        // F-274), and the operator's own knowledge outranks a gap in the file.
        (_, Some(CollabDecision::Keep)) => "keep",
        (_, Some(CollabDecision::Revoke)) => "revoke-requested",
        (true, None) => "undecided",
    }
}

/// Print the collab follow-up an operator needs, and nothing when there is none.
fn print_collab_follow_up(handle: &str, outcome: &'static str) {
    match outcome {
        "revoke-requested" => eprintln!(
            "  collab: DECISION RECORDED (revoke) — the mesh revoke does not cut it. \
             Run on the host holding the collab credentials:\n    \
             onboard-runner -collab revoke -handle {handle} -yes"
        ),
        "undecided" => eprintln!(
            "  collab: {handle} has a collaboration-environment account and NO decision was \
             recorded — mesh access is cut, collab access is NOT. Re-run with \
             `--collab keep|revoke`, or decide now with `onboard-runner -collab status|revoke \
             -handle {handle}`."
        ),
        _ => {}
    }
}

/// The Ory leg (R2): GET the identity by email, then PATCH state=inactive
/// (reversible) and delete its sessions, or DELETE (--purge; Kratos drops the
/// sessions with the identity). An absent identity is `already-absent`, not an
/// error (idempotent). Returns the outcome tag and the identity id.
///
/// Sessions are per site (never replicated, ADR 1010 D8): this call ends them
/// at the site `kratos_admin_url` names. On a replicated goatnet the revocation
/// guard ends them at every other site when the deactivation arrives there
/// (ADR 1193).
async fn revoke_ory_identity(
    cfg: &OryEnvConfig,
    email: &str,
    purge: bool,
    dry_run: bool,
    targets: &Path,
) -> anyhow::Result<(&'static str, Option<String>)> {
    if dry_run {
        eprintln!(
            "dry-run: ssh {}@{} → GET identity by email, then {} it",
            cfg.ssh_user,
            cfg.ssh_host,
            if purge {
                "DELETE"
            } else {
                "deactivate (state=inactive)"
            }
        );
        return Ok((if purge { "purged" } else { "deactivated" }, None));
    }
    let get = format!(
        "curl -sS --max-time 30 -w '\\n%{{http_code}}' '{}/admin/identities?credentials_identifier={}'",
        cfg.kratos_admin_url,
        pct_encode_query(email)
    );
    let (code, body) = run_ssh_curl(cfg, &get, None).await?;
    if code != 200 {
        anyhow::bail!(
            "Kratos GET identities HTTP {code}: {}",
            body.chars().take(200).collect::<String>()
        );
    }
    let identities: Vec<serde_json::Value> = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("Kratos identity lookup was not a valid list: {e}"))?;
    if identities.is_empty() {
        return Ok(("already-absent", None));
    }
    anyhow::ensure!(
        identities.len() == 1,
        "Kratos identity lookup was ambiguous"
    );
    let id = identities[0]
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("Kratos identity lookup omitted its UUID"))?
        .to_string();
    // Must reach durable storage before DELETE can make this UUID undiscoverable.
    // A corrupt or unwritable checkpoint refuses the Ory mutation.
    read_revoke_targets(targets)?;
    record_revoke_target(targets, &id)?;
    if purge {
        let del = format!(
            "curl -sS --max-time 30 -X DELETE -w '\\n%{{http_code}}' '{}/admin/identities/{id}'",
            cfg.kratos_admin_url
        );
        let (c, b) = run_ssh_curl(cfg, &del, None).await?;
        if c == 204 || c == 404 {
            Ok(("purged", Some(id)))
        } else {
            anyhow::bail!(
                "Kratos DELETE HTTP {c}: {}",
                b.chars().take(200).collect::<String>()
            );
        }
    } else {
        let patch = format!(
            "curl -sS --max-time 30 -X PATCH -H 'Content-Type: application/json' \
             --data-binary @- -w '\\n%{{http_code}}' '{}/admin/identities/{id}'",
            cfg.kratos_admin_url
        );
        let (c, b) = run_ssh_curl(
            cfg,
            &patch,
            Some("[{\"op\":\"replace\",\"path\":\"/state\",\"value\":\"inactive\"}]"),
        )
        .await?;
        if c != 200 {
            anyhow::bail!(
                "Kratos PATCH HTTP {c}: {}",
                b.chars().take(200).collect::<String>()
            );
        }
        let sessions = format!(
            "curl -sS --max-time 30 -X DELETE -w '\\n%{{http_code}}' '{}/admin/identities/{id}/sessions'",
            cfg.kratos_admin_url
        );
        let (c, b) = run_ssh_curl(cfg, &sessions, None).await?;
        // 404 = no sessions to end.
        if c == 204 || c == 404 {
            Ok(("deactivated", Some(id)))
        } else {
            anyhow::bail!(
                "Kratos DELETE sessions HTTP {c}: {}",
                b.chars().take(200).collect::<String>()
            );
        }
    }
}

// Durable per-target checkpoints are additive: a later UUID using the same
// handle does not erase unfinished cuts for an earlier incarnation. The hash
// binds the journal to the profile context, goatnet, handle, email and endpoint.
fn revoke_target_directory(
    profile: &crate::profile::Profile,
    env: &str,
    handle: &str,
    email: &str,
    cfg: Option<&OryEnvConfig>,
) -> PathBuf {
    use sha2::{Digest, Sha256};
    let endpoint = cfg
        .map(|c| format!("{}@{}:{}", c.ssh_user, c.ssh_host, c.kratos_admin_url))
        .unwrap_or_default();
    let key =
        serde_json::to_vec(&(env, handle, email, endpoint)).expect("string tuple serialization");
    profile
        .context_dir()
        .join("revoke-targets")
        .join(format!("{:x}", Sha256::digest(key)))
}

fn valid_revoke_identity(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

fn read_revoke_targets(directory: &Path) -> anyhow::Result<Vec<String>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        anyhow::ensure!(
            entry.file_type()?.is_file(),
            "unexpected revocation checkpoint entry"
        );
        let id: String = serde_json::from_slice(&std::fs::read(entry.path())?)?;
        anyhow::ensure!(
            valid_revoke_identity(&id)
                && entry.file_name() == std::ffi::OsStr::new(&format!("{id}.json")),
            "invalid revocation checkpoint"
        );
        ids.push(id);
    }
    ids.sort();
    Ok(ids)
}

fn record_revoke_target(directory: &Path, id: &str) -> anyhow::Result<()> {
    use std::io::Write;
    anyhow::ensure!(
        valid_revoke_identity(id),
        "Kratos returned an invalid identity UUID"
    );
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join(format!("{id}.json"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(&serde_json::to_vec(id)?)?;
            file.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Crash-truncated or conflicting state must not be treated as durable.
            let stored: String = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(stored == id, "conflicting revocation checkpoint");
        }
        Err(e) => return Err(e.into()),
    }
    // Retrying a completed-but-unsynced write also establishes durability.
    std::fs::OpenOptions::new().read(true).write(true).open(&path)?.sync_all()?;
    #[cfg(unix)]
    for parent in directory.ancestors().take(3) {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// R2.5: block the NetBird account whose id is the Kratos identity id.
/// `PUT /api/users/{id}` replaces the record, so role and auto_groups are
/// echoed back unchanged.
async fn block_netbird_user(
    nb: &panoscope_core::netbird::NetbirdClient,
    identity_id: &str,
    dry_run: bool,
) -> anyhow::Result<&'static str> {
    let users = nb
        .list_users()
        .await
        .map_err(|e| anyhow::anyhow!("GET /api/users: {e}"))?;
    let Some(u) = users.iter().find(|u| u.id == identity_id) else {
        return Ok("absent");
    };
    if u.is_blocked {
        return Ok("already-blocked");
    }
    if dry_run {
        eprintln!("dry-run: PUT /api/users/{} is_blocked=true", u.id);
        return Ok("blocked");
    }
    match nb.update_user(&u.id, &u.role, &u.auto_groups, true).await {
        Ok(()) | Err(panoscope_core::netbird::NetbirdError::NotFound) => {}
        Err(e) => anyhow::bail!("PUT /api/users/{}: {e}", u.id),
    }
    let stored = nb
        .list_users()
        .await
        .map_err(|e| anyhow::anyhow!("NetBird block readback failed: {e}"))?;
    match stored.iter().find(|row| row.id == identity_id) {
        None => Ok("absent"),
        Some(row) if row.is_blocked => Ok("blocked"),
        _ => anyhow::bail!("NetBird user block did not persist"),
    }
}

/// Print a per-handle outcome line.
fn print_revoke_outcome(o: &RevokeOutcome) {
    eprintln!(
        "  {}{}: peers_cut={} ory={} netbird_user={} zenoh={} manifest={} proposal={} operator_keys={} ssh_keys={} collab={}",
        if o.any_failed { "[PARTIAL] " } else { "" },
        o.handle,
        o.peers_cut,
        o.ory,
        o.netbird_user,
        o.zenoh,
        o.manifest,
        o.proposal,
        o.operator_keys,
        o.ssh_keys,
        o.collab
    );
}

/// `goat admin onboard <handle> --revoke` (native) — the single-handle path.
#[allow(clippy::too_many_arguments)]
async fn revoke_native(
    ctx: &super::Ctx,
    handle: &str,
    env: Option<&str>,
    purge: bool,
    org_scope: bool,
    repo_root: Option<&Path>,
    dry_run: bool,
    collab: Option<CollabDecision>,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!("invalid --env `{env}`: must be a kebab-case goatnet id");
        }
    }
    let cwd = std::env::current_dir()?;
    let root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor)",
                cwd.display()
            )
        })?,
    };
    let shared = build_revoke_shared(ctx, env, &root).await?;
    let o = revoke_one(ctx, &shared, handle, env, purge, org_scope, dry_run, collab).await?;
    eprintln!(
        "{}revoked `{handle}`:",
        if dry_run { "dry-run: would have " } else { "" }
    );
    print_revoke_outcome(&o);
    // `revoke_one` already computes any_failed (it uses the same boolean to
    // tag the audit event "partial" vs "ok") — before this check existed,
    // that computation never reached the caller, so a revoke that cut the
    // mesh peer but left the Ory login or portal op_ key alive printed a
    // quiet summary line and exited 0.
    if o.should_fail(dry_run) {
        anyhow::bail!(
            "revoke of `{handle}` completed with at least one leg failed (see [PARTIAL] line \
             above) — the person may still hold a working credential on the failed leg(s). \
             Re-run to retry (idempotent) or repair the named leg manually."
        );
    }
    Ok(())
}

/// `goat admin onboard --org <id> --revoke` (native cascade) — resolves active
/// members from the registry, confirms the blast radius, then revokes each
/// natively with `org_scope=true` (so a shared Zenoh anchor IS deleted). Peers +
/// keys are fetched ONCE via the shared context. One member's failure is reported,
/// never aborts the rest (exit non-zero if any failed).
#[allow(clippy::too_many_arguments)]
async fn revoke_native_org(
    ctx: &super::Ctx,
    org_id: &str,
    env: Option<&str>,
    purge: bool,
    repo_root: Option<&Path>,
    dry_run: bool,
    yes: bool,
    collab: Option<CollabDecision>,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!("invalid --env `{env}`: must be a kebab-case goatnet id");
        }
    }
    let cwd = std::env::current_dir()?;
    let root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor)",
                cwd.display()
            )
        })?,
    };
    let members = resolve_org_members(&root, org_id, env)?;
    if members.is_empty() {
        anyhow::bail!(
            "no active members found for org `{org_id}` in the registry — nothing to revoke"
        );
    }
    eprintln!(
        "ORG REVOKE CASCADE (native) — org `{org_id}` ({} active member(s)):",
        members.len()
    );
    for m in &members {
        eprintln!("  - {}  [{}]", m.handle, m.kind);
    }
    eprintln!(
        "Each gets the native per-handle revoke (mesh peers cut, Ory {}, Zenoh anchor \
         deleted, status -> revoked).",
        if purge { "PURGED" } else { "deactivated" }
    );
    if dry_run {
        eprintln!("dry-run: no revoke executed.");
    } else if !yes {
        eprint!(
            "Revoke all {} member(s) of `{org_id}`? [y/N] ",
            members.len()
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            anyhow::bail!("aborted — no members revoked");
        }
    }
    let shared = build_revoke_shared(ctx, env, &root).await?;
    let mut failures = 0usize;
    for m in &members {
        match revoke_one(ctx, &shared, &m.handle, env, purge, true, dry_run, collab).await {
            // An Ok result can still carry a failed leg (any_failed) — that
            // must count toward `failures` too, or a partially-failed member
            // silently passes as success in the summary below.
            Ok(o) => {
                let should_fail = o.should_fail(dry_run);
                print_revoke_outcome(&o);
                if should_fail {
                    failures += 1;
                }
            }
            Err(e) => {
                eprintln!("  FAILED to revoke {}: {e}", m.handle);
                failures += 1;
            }
        }
    }
    if failures > 0 {
        anyhow::bail!(
            "{failures} of {} member(s) failed to revoke — see above; re-run to retry (idempotent)",
            members.len()
        );
    }
    eprintln!(
        "org `{org_id}` cascade complete — {} member(s) revoked.",
        members.len()
    );
    Ok(())
}

/// `goat admin onboard --revoke --from FILE` (native batch) — the generalized
/// cohort-revoke tool. Reads one kebab-case handle per line (`#` comments +
/// blanks ignored), revokes each natively + independently. Peers + keys fetched
/// ONCE. A per-handle failure is reported, never aborts the batch; exit is
/// non-zero if any failed.
async fn revoke_native_batch_from_file(
    ctx: &super::Ctx,
    path: &Path,
    env: Option<&str>,
    purge: bool,
    repo_root: Option<&Path>,
    dry_run: bool,
    collab: Option<CollabDecision>,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!("invalid --env `{env}`: must be a kebab-case goatnet id");
        }
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read handle list {}: {e}", path.display()))?;
    let handles = parse_handle_list(&text);
    if handles.is_empty() {
        anyhow::bail!(
            "no handles in {} (one kebab-case handle per line)",
            path.display()
        );
    }
    let cwd = std::env::current_dir()?;
    let root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor)",
                cwd.display()
            )
        })?,
    };
    eprintln!(
        "BATCH REVOKE (native) — {} handle(s) from {}{}{}",
        handles.len(),
        path.display(),
        if purge { ", Ory PURGE" } else { "" },
        if dry_run { " [dry-run]" } else { "" }
    );
    let shared = build_revoke_shared(ctx, env, &root).await?;
    let mut failures = 0usize;
    let mut ok = 0usize;
    for h in &handles {
        match revoke_one(ctx, &shared, h, env, purge, false, dry_run, collab).await {
            Ok(o) => {
                let should_fail = o.should_fail(dry_run);
                print_revoke_outcome(&o);
                // Same fix as the org cascade: an Ok result with a failed leg
                // must count as a failure here too, not just a hard Err.
                if should_fail {
                    failures += 1;
                } else {
                    ok += 1;
                }
            }
            Err(e) => {
                eprintln!("  FAILED {h}: {e}");
                failures += 1;
            }
        }
    }
    eprintln!(
        "batch complete — {ok} revoked, {failures} failed{}.",
        if dry_run { " (dry-run)" } else { "" }
    );
    if failures > 0 {
        anyhow::bail!("{failures} handle(s) failed — see above; re-run to retry (idempotent)");
    }
    Ok(())
}

/// Parse a batch handle list: one handle per line, `#` comments + blank lines
/// ignored, surrounding whitespace trimmed. Pure for unit tests.
fn parse_handle_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Decide whether a plain enroll should take the native fast path. Returns
/// `Ok(None)` when the manifest is fully native-serviceable, or `Ok(Some(reason))`
/// when it needs the Ansible playbook (a surface native doesn't do). Reads +
/// parses the manifest read-only; `Err` on a missing/unparseable manifest (the
/// caller then falls back to Ansible, whose own error is clearer). Kept separate
/// from `onboard_native` so `--native` stays strict (hard-errors on ineligible)
/// while the DEFAULT path degrades gracefully.
fn native_eligibility(
    repo_root: Option<&Path>,
    handle: &str,
    env: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let cwd = std::env::current_dir()?;
    let root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd)
            .ok_or_else(|| anyhow::anyhow!("no .git ancestor from {}", cwd.display()))?,
    };
    let operators_dir = root.join("ops/operators");
    let manifest_path = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest_path.exists() {
        anyhow::bail!("no manifest at {}", manifest_path.display());
    }
    let m: OnboardManifest = serde_yaml::from_str(&std::fs::read_to_string(&manifest_path)?)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;
    if !m.zenoh.enabled {
        return Ok(Some(
            "manifest has zenoh.enabled=false (native onboards a Zenoh publisher)".into(),
        ));
    }
    if m.zenoh.observer.is_some() {
        return Ok(Some(
            "manifest is a read-only observer (native mints a publisher)".into(),
        ));
    }
    if m.zenoh.prod.is_some() {
        return Ok(Some(
            "manifest targets zenoh.prod (native does trial-fabric publishers only)".into(),
        ));
    }
    let has_trial = m
        .zenoh
        .trial
        .as_ref()
        .map(|t| !t.org.trim().is_empty())
        .unwrap_or(false);
    if !has_trial {
        return Ok(Some(
            "manifest has no zenoh.trial.org publisher prefix".into(),
        ));
    }
    if m.ory_idp.enabled {
        return Ok(Some(
            "manifest enables ory_idp (operator-IdP), which the native path doesn't provision"
                .into(),
        ));
    }
    Ok(None)
}

/// The valid `ory_site.groups` service-permission values, read from the manifest
/// schema's enum (`ops/operators/schema/operator-manifest.schema.json`), which CI
/// (`check-ory-groups-enum.sh`) keeps in lock-step with the SoT
/// `ops/permissions/group-permissions.yml`. Used to fail a typo'd group fast
/// before minting an identity whose claim would match no policy.
fn valid_ory_groups(repo_root: &Path) -> anyhow::Result<Vec<String>> {
    let p = repo_root.join("ops/operators/schema/operator-manifest.schema.json");
    let body = std::fs::read_to_string(&p)
        .map_err(|e| anyhow::anyhow!("read ory-groups enum from {}: {e}", p.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))?;
    v.pointer("/properties/ory_site/properties/groups/items/enum")
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .ok_or_else(|| {
            anyhow::anyhow!("schema missing the ory_site.groups enum at {}", p.display())
        })
}

/// The enroll fence's escape hatch. Mirrors `operator-onboard.yml`'s
/// `-e i_understand_the_enroll_leg_is_retired=true` by name so the playbook gate
/// and this one read as a single fence. An env var rather than a clap flag
/// deliberately: a flag lands in shell history and in copy-pasted runbooks, and
/// this is meant to be typed on purpose each time.
/// Whether this invocation is the onboard-runner executing an ADMIN-APPROVED
/// proposal — the three-stage console path, which is the very path the enroll
/// fence's own error text tells people to use instead.
///
/// WHY THIS EXISTS. The fence refuses `--native` onboarding because it mints
/// from a HAND-AUTHORED manifest and writes a bundle carrying private key
/// material for an operator to carry to the person. The runner does neither: the
/// candidate came through invite -> sponsor vouch -> admin approve, and the
/// manifest at `ops/operators/<env>/<handle>.yml` is an OUTPUT the console just
/// wrote — exactly as the fence's own message says it should be. Without this,
/// the fence refuses the runner and onboarding stops.
///
/// Any goatnet running a `goat` that contains the fence refuses its runner
/// without this, so deploying that `goat` anywhere stops onboarding there.
///
/// NOT A SECURITY BOUNDARY, and deliberately not dressed as one. This is an
/// ordinary env var, and anyone who can set it can already set
/// GOAT_I_UNDERSTAND_THE_ENROLL_LEG_IS_RETIRED. The fence is a footgun guard —
/// it stops someone reaching for a retired door out of habit — and this teaches
/// it to recognise the one caller that is not doing that. The runner already
/// sets this variable for audit attribution, so no new surface is introduced.
fn driven_by_onboard_runner() -> bool {
    std::env::var("GOAT_OPERATOR_HANDLE")
        .map(|h| h.starts_with("onboard-runner-"))
        .unwrap_or(false)
}

/// The goatnet an operator action is RECORDED against.
///
/// `--env` when given, otherwise the profile's active context — which is the
/// goatnet selector (`--context` / `$GOAT_CONTEXT` / `current_context`). Never a
/// hardcoded goatnet.
///
/// An admin action run without `--env` must not name a default goatnet. An
/// audit record that names the wrong goatnet is worse than one that is
/// missing: the missing one is visibly absent.
///
/// A context still unset resolves to `"default"` upstream, which is honest —
/// it names no goatnet and is not another goatnet's name.
fn audit_goatnet<'a>(env: Option<&'a str>, profile: &'a crate::profile::Profile) -> &'a str {
    match env {
        Some(e) if !e.trim().is_empty() => e,
        _ => profile.context.as_str(),
    }
}

fn enroll_leg_override() -> bool {
    matches!(
        std::env::var("GOAT_I_UNDERSTAND_THE_ENROLL_LEG_IS_RETIRED").as_deref(),
        Ok("true") | Ok("1")
    )
}

/// `goat admin onboard <handle> --native` — native onboard (P2.1 Zenoh + P2.2
/// NetBird + P2.4 Ory), all three identity surfaces with no Ansible. Provisions
/// the Zenoh slot + cert + key + portal api_key via the portal, mints the NetBird
/// mesh setup key, and creates the Ory Kratos login identity over SSH to the site
/// Thor — writing one onboarding bundle into `ops/operators/out/`. Ordering:
/// NetBird FIRST (a mesh failure leaves nothing Zenoh-side consumed, cleanly
/// retryable), then the Zenoh mint, then Ory LAST (an Ory failure leaves the
/// Zenoh/NetBird bundle usable — surfaced loudly, marked partial, never silently
/// dropped). See docs/design/onboard-rotate-revoke-streamlining.md §"Phase 2".
async fn onboard_native(
    ctx: &super::Ctx,
    handle: &str,
    env: Option<&str>,
    repo_root: Option<&Path>,
    dry_run: bool,
    defer_credential: bool,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!(
                "invalid --env `{env}`: must be a kebab-case goatnet id \
                 (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `zagros-prod`)"
            );
        }
    }

    let cwd = std::env::current_dir()?;
    let repo_root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    let operators_dir = repo_root.join("ops/operators");
    let manifest_path = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest_path.exists() {
        anyhow::bail!(
            "no operator manifest at {} — author it first per docs/design/operator-onboard-utilities.md",
            manifest_path.display()
        );
    }
    let mut manifest: OnboardManifest = serde_yaml::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;

    // Resolve local affiliation inputs before any credential is minted, including
    // dry-run. Ory's existing feature and site gates still control emission.
    let aff_enabled = std::env::var("GOAT_ORY_AFF_TRAIT_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let ory_applies = manifest.ory_site.enabled
        && native_ory_config_for(env, &ctx.profile).is_some()
        && (manifest.ory_site.sites.is_empty()
            || manifest
                .ory_site
                .sites
                .iter()
                .any(|s| Some(s.as_str()) == env));
    if let Some(aff) = affiliation::resolve(
        &repo_root,
        env.unwrap_or("(none)"),
        &manifest.org,
        &manifest.affiliation,
        aff_enabled && ory_applies,
    )? {
        manifest.affiliation = aff;
    }

    // Resolve the Zenoh publisher prefix — ONLY when the manifest enables a Zenoh
    // surface. A browse / web-only tier (zenoh.enabled=false, NetBird + Ory on) is
    // valid: --native provisions its mesh + login legs and skips Zenoh, rather than
    // bailing (which stranded browse-tier onboards through the runner — the runner
    // forces --native, so the Ansible fallback isn't available to it). observer/prod
    // are still bailed with a precise pointer when Zenoh IS enabled.
    let prefix: Option<String> = if manifest.zenoh.enabled {
        match manifest.zenoh.trial.as_ref().map(|t| t.org.trim()) {
            Some(v) if !v.is_empty() => Some(v.to_string()),
            _ => {
                if manifest.zenoh.observer.is_some() {
                    anyhow::bail!(
                        "manifest {} is a read-only observer (zenoh.observer) — --native onboards \
                         a portal-minted publisher only; use the Ansible path `goat admin onboard {handle}`",
                        manifest_path.display()
                    );
                }
                if manifest.zenoh.prod.is_some() {
                    anyhow::bail!(
                        "manifest {} targets zenoh.prod — the ADR 0844 prod portal isn't built; \
                         --native onboards trial-fabric publishers only",
                        manifest_path.display()
                    );
                }
                anyhow::bail!(
                    "manifest {} has zenoh.enabled but no zenoh.trial.org (or legacy \
                     zenoh.trial.vendor) — set it to the publisher prefix/handle",
                    manifest_path.display()
                );
            }
        }
    } else {
        None
    };
    let display_name = {
        let dn = manifest.org.display_name.trim();
        if !dn.is_empty() {
            dn.to_string()
        } else if !manifest.identity.name.trim().is_empty() {
            manifest.identity.name.trim().to_string()
        } else {
            prefix.clone().unwrap_or_else(|| handle.to_string())
        }
    };

    // FAIL CLOSED on an env this binary has no native config for.
    // Each per-env table below returns Option, and every
    // leg used to read `None` as "skip me" — so onboarding a person on a goatnet
    // absent from the tables silently skipped EVERY leg, wrote a stub bundle with
    // no mesh key and no login, and exited 0. The onboard-runner reads that exit
    // code as success and the console sends the welcome email: a real person is
    // told they are provisioned when nothing was minted. Refuse instead, and name
    // both remedies (add the arms, or use the playbook that does support the env).
    //
    // Only ENABLED surfaces are checked: a browse-tier manifest with zenoh off is
    // legitimately mesh+login only, and must not be blocked by a missing router
    // endpoint it never uses.
    {
        // Profile-first: a context declaring its own endpoints satisfies these
        // checks without the built-in table knowing the goatnet at all.
        let envname = env.unwrap_or(ctx.profile.context.as_str());
        let mut unsupported: Vec<&str> = Vec::new();
        if manifest.netbird.enabled && native_netbird_config_for(env, &ctx.profile).is_none() {
            unsupported.push("NetBird mesh key (native_netbird_config)");
        }
        if manifest.ory_site.enabled && native_ory_config_for(env, &ctx.profile).is_none() {
            unsupported.push("Ory site login (native_ory_config)");
        }
        if manifest.zenoh.enabled && native_router_endpoint_for(env, &ctx.profile).is_none() {
            unsupported.push("Zenoh router endpoint (native_router_endpoint)");
        }
        if !unsupported.is_empty() {
            anyhow::bail!(
                "--native has no configuration for goatnet `{envname}`, so these enabled \
                 surface(s) would be SILENTLY SKIPPED: {}. Refusing: a partial mint reports \
                 success and delivers a bundle with nothing in it.\n\
                 Fix one of two ways:\n  \
                 1. add `{envname}` arms to the named function(s) in \
                 crates/goat-cli/src/cmd/admin.rs (one match arm each), rebuild, redeploy the \
                 host binary the onboard-runner bind-mounts; or\n  \
                 2. run the playbook path instead: `goat admin onboard {handle} --ansible \
                 --env {envname}`.",
                unsupported.join(", "),
            );
        }
    }

    // Honesty: --native provisions Zenoh + NetBird + the Ory *site* login (P2.4),
    // but NOT the Ory operator-IdP. Name the surface it is genuinely leaving to
    // Ansible. (Ory-site-without-native-config is no longer a soft skip — the
    // fail-closed gate above refuses that case outright.)
    let ory_site_native = manifest.ory_site.enabled;
    if manifest.ory_idp.enabled {
        eprintln!(
            "note: --native does not provision: Ory operator-IdP. Run \
             `goat admin onboard {handle} --ansible{}` to provision it.",
            env.map(|e| format!(" --env {e}")).unwrap_or_default(),
        );
    }

    // Validate ory_site.groups against the service-permission enum BEFORE minting
    // anything — a typo'd or prefixed group would mint an identity whose `groups`
    // claim matches no policy (a silent authz failure, F-177-adjacent). The Ansible
    // path gets this from validate.py; native was lenient, so add the same check.
    if ory_site_native && !manifest.ory_site.groups.is_empty() {
        let valid = valid_ory_groups(&repo_root)?;
        let unknown: Vec<&String> = manifest
            .ory_site
            .groups
            .iter()
            .filter(|g| !valid.contains(g))
            .collect();
        if !unknown.is_empty() {
            anyhow::bail!(
                "manifest ory_site.groups has unknown value(s) {unknown:?}. Valid Ory \
                 service-permission groups: {valid:?} (SoT ops/permissions/group-permissions.yml). \
                 These are the SERVICE-PERMISSION axis (dashboard / console / MinIO access), NOT \
                 the mesh role (that's netbird.auto_groups) — and they are NOT site-prefixed."
            );
        }
    }

    let portal = ctx.profile.portal_client()?;
    let portal_url = portal.base_url().unwrap_or("<portal>").to_string();
    let router_endpoint_owned = native_router_endpoint_for(env, &ctx.profile);
    let router_endpoint = router_endpoint_owned.as_deref();
    let out_dir = operators_dir.join("out");
    let bundle_md_path = out_dir.join(format!("{handle}-bundle.md"));
    let tarball_path = out_dir.join(format!("{handle}-zenoh-bundle.tar.gz"));

    if dry_run {
        // NetBird leg first (matches real ordering); prints its plan, mints nothing.
        provision_netbird_native(
            env,
            &ctx.profile,
            handle,
            &manifest.netbird,
            &manifest.org.short_tag,
            true,
        )
        .await?;
        if let Some(prefix) = &prefix {
            eprintln!(
                "dry-run: POST {portal_url}/api/orgs  body: prefix={prefix} \
                 display_name={display_name:?} synthetic=false auto_approve=true"
            );
            eprintln!(
                "dry-run: would download the one-time tarball → {}",
                tarball_path.display()
            );
        } else {
            eprintln!("dry-run: zenoh.enabled=false — skipping the Zenoh slot mint (browse/web-only tier)");
        }
        // Ory leg last (matches real ordering); prints its plan, creates nothing.
        provision_ory_native(env, &ctx.profile, handle, &manifest, true).await?;
        eprintln!(
            "dry-run: would render the bundle → {}",
            bundle_md_path.display()
        );
        return Ok(());
    }

    // Operator-action audit: started → ok/failed, mirroring onboard().
    let audit_env = audit_goatnet(env, &ctx.profile);
    let audit_actor = crate::cmd::audit::actor_handle(&ctx.profile);
    let audit_digest = format!(
        "onboard-native handle={handle} env={audit_env} prefix={}",
        prefix.as_deref().unwrap_or("(none — browse tier)")
    );
    crate::cmd::audit::emit(
        &ctx.profile,
        crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            "admin.onboard-native",
            handle,
            audit_env,
            "started",
        )
        .with_args_digest(&audit_digest),
    )
    .await;
    let finish_audit = |outcome: &'static str, err: Option<String>| {
        let mut a = crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            "admin.onboard-native",
            handle,
            audit_env,
            outcome,
        )
        .with_args_digest(&audit_digest);
        if let Some(e) = err {
            a = a.with_error(e);
        }
        crate::cmd::audit::emit(&ctx.profile, a)
    };

    // NetBird mesh leg FIRST (P2.2) — so a mesh failure leaves nothing Zenoh-side
    // consumed and the onboard is cleanly retryable. Hard failures bail here.
    let nb_prov = match provision_netbird_native(
        env,
        &ctx.profile,
        handle,
        &manifest.netbird,
        &manifest.org.short_tag,
        false,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            finish_audit("failed", Some(format!("netbird: {e}"))).await;
            return Err(e);
        }
    };

    // Zenoh slot mint — only for a manifest with a Zenoh surface. A browse /
    // web-only tier (prefix=None) skips it; its NetBird + Ory legs still run.
    let onboarded: Option<OnboardedOrg> = if let Some(prefix) = &prefix {
        let req = NewVendor {
            prefix: prefix.clone(),
            display_name: display_name.clone(),
            synthetic: false,
            auto_approve: true,
            // Sponsorship annotation (ADR 1071) from the manifest's
            // `sponsorship:` block — mirrored by the ansible path's Phase 4.0.
            sponsor_authorized: manifest.sponsorship.authorized,
            sponsor_max_tier: manifest.sponsorship.max_tier.clone(),
            sponsor_email: manifest.sponsorship.email.clone(),
            // Seed the machine-enrollment grant (ADR 1096) so a person onboarded
            // with a machine_quota can enroll machines with no separate operator
            // grant. The unit machines inherit is affiliation.id (the same real-org
            // value the identity trait now carries). The portal skips the seed when
            // quota is 0 or the affiliation is empty.
            machine_quota: manifest.machine_quota,
            affiliation: manifest.affiliation.id.trim().to_string(),
            site: String::new(),
            // Mint-on-collect: create the identity, leave the credential for its
            // owner to collect. Off unless the operator (or the runner, per
            // goatnet) asks for it — see the flag's own doc comment for why the
            // default must not flip before the collection page is deployed.
            defer_credential,
            // ADR 1138 partner block. Empty for an ordinary peer (the portal
            // then takes the lowest free slot, unchanged). Non-empty makes the
            // portal allocate from the matching pre-cut block and REFUSE if no
            // block declares that lane-set — deliberately, because falling back
            // to an ordinary slot is what leaves a partner reading everything.
            partner_lanes: manifest
                .zenoh
                .trial
                .as_ref()
                .map(|t| t.partner_lanes.clone())
                .unwrap_or_default(),
        };
        match portal.onboard_org(req).await {
            Ok(o) => Some(o),
            Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
                finish_audit("failed", Some("409 already onboarded".into())).await;
                anyhow::bail!(
                    "portal: `{prefix}` is already onboarded (HTTP 409) — its api_key + bundle are \
                     NOT regenerable. To re-key in place use `goat admin rotate {prefix}`; to start \
                     over, `goat admin onboard {handle}{} --revoke` then re-run. \
                     (Any NetBird setup key this run minted was revoked; a re-run mints a fresh one.)",
                    env.map(|e| format!(" --env {e}")).unwrap_or_default(),
                );
            }
            Err(e) => {
                revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
                finish_audit("failed", Some(e.to_string())).await;
                return Err(super::vendor::map_portal_err(e));
            }
        }
    } else {
        None
    };

    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
        anyhow::bail!("create {}: {e}", out_dir.display());
    }

    // Write the bundle markdown FIRST (crash insurance): it carries the one-time
    // credentials (setup key, api_key, bundle URL), so even a download failure
    // leaves them on disk. The Ory leg runs after, then the md is rewritten to
    // fold in the OIDC login credential. For a browse tier the md still captures
    // the NetBird setup key (its own one-time credential); the Zenoh section is
    // simply absent.
    // BELT (the fail-closed gate above is the suspenders): never write a bundle
    // that provisioned nothing. The gate catches the known cause — an env absent
    // from the config tables — but any future path that leaves every leg empty
    // would again produce a ~124-byte "bundle" the runner reports as success and
    // the console emails to a real person. Whatever the cause, an empty mint is
    // an error, not a deliverable.
    if manifest.netbird.enabled && nb_prov.is_none() && onboarded.is_none() {
        anyhow::bail!(
            "refusing to write a bundle for `{handle}`: the manifest enables the NetBird mesh \
             surface but no leg provisioned anything, so the bundle would carry no credentials. \
             This is the stub-mint shape — nothing was delivered and \
             nothing should be reported as delivered. Check the leg warnings above, then re-run."
        );
    }
    let md = render_bundle_md(
        handle,
        env,
        onboarded.as_ref(),
        &portal_url,
        router_endpoint,
        nb_prov.as_ref(),
        None,
    );
    if let Err(e) = std::fs::write(&bundle_md_path, &md) {
        revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
        anyhow::bail!("write {}: {e}", bundle_md_path.display());
    }

    // Then fetch the cert tarball (Zenoh tiers only). A download failure is
    // non-fatal — the md holds the one-time URL + api_key — so we warn.
    let mut tarball_note = match &onboarded {
        Some(_) => format!("{}", tarball_path.display()),
        None => "(none — browse/web-only tier, no Zenoh cert)".to_string(),
    };
    if let Some(o) = &onboarded {
        if let Err(e) = portal.download_bundle(&o.bundle_url, &tarball_path).await {
            tarball_note = format!(
                "DOWNLOAD FAILED ({e}) — fetch manually from {} (one-time, ~24 h)",
                full_bundle_url(Some(&portal_url), &o.bundle_url)
            );
            eprintln!("warning: bundle tarball download failed: {e}");
            eprintln!(
                "  the one-time URL + api_key are preserved in {} — fetch the tarball manually soon",
                bundle_md_path.display()
            );
        }
    }

    // Ory OIDC leg LAST (P2.4). For a Zenoh tier, a failure here USED to leave the
    // already-minted slot in place ("the bundle is still usable"). But an onboard
    // with no login is not a usable onboard, and the orphaned slot makes every
    // retry 409 ("already onboarded") — the person is stranded with a slot, no
    // Kratos identity, and never appears in the console user list (the stuck-
    // onboarding class). So the Zenoh-tier
    // onboard is now ATOMIC: if the login leg fails, roll the slot back (via the
    // portal delete the same revoke path uses) so the re-run is clean — no 409.
    // The browse/web-only tier (no slot minted) keeps the prior partial behavior:
    // there is nothing to orphan and the NetBird bundle is genuinely usable.
    let (ory_prov, ory_error) = match provision_ory_native(
        env,
        &ctx.profile,
        handle,
        &manifest,
        false,
    )
    .await
    {
        Ok(p) => (p, None),
        Err(e) => {
            // Every exit below fails the run, so nobody is sent the setup keys
            // this run minted; the browse-tier arm at the end does NOT fail, and
            // keeps them (#1587).
            if onboarded.is_some() {
                revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
            }
            if let Some(o) = onboarded.as_ref().filter(|o| o.reused) {
                // A reused slot (#1576) existed before this onboard — it is the
                // person's slot from their first onboarding, possibly holding a
                // credential they already collected. Rolling it back would revoke
                // that, so fail without touching it; a re-run reuses it again.
                finish_audit(
                    "failed",
                    Some(format!("ory: {e}; reused slot {} left in place", o.prefix)),
                )
                .await;
                anyhow::bail!(
                    "the Ory login leg failed ({e}). The fabric slot `{}` pre-dated this onboard \
                     (reused), so it was NOT rolled back. Re-run once Kratos/the site Thor is \
                     reachable; the retry reuses the same slot.",
                    o.prefix
                );
            }
            if let Some(o) = &onboarded {
                let slot_prefix = o.prefix.clone();
                let env_flag = env.map(|x| format!(" --env {x}")).unwrap_or_default();
                eprintln!("warning: the Ory OIDC login leg FAILED: {e}");
                eprintln!(
                    "  rolling back the just-minted Zenoh slot `{slot_prefix}` so the onboard is \
                     atomic and a re-run starts clean (no 409)..."
                );
                match portal.delete_vendor(&slot_prefix).await {
                    Ok(()) => {
                        finish_audit(
                            "failed",
                            Some(format!(
                                "ory: {e}; Zenoh slot rolled back (atomic) — re-run to retry"
                            )),
                        )
                        .await;
                        anyhow::bail!(
                            "the Ory login leg failed ({e}); rolled back the Zenoh slot so state is \
                             clean. Re-run `goat admin onboard {handle}{env_flag}` once Kratos/the \
                             site Thor is reachable — the retry will not 409."
                        );
                    }
                    Err(re) => {
                        // Rollback itself failed — the slot is now genuinely orphaned; point
                        // the operator at the supported repair rather than a silent partial.
                        finish_audit(
                            "failed",
                            Some(format!("ory: {e}; slot rollback FAILED: {re}")),
                        )
                        .await;
                        anyhow::bail!(
                            "the Ory login leg failed ({e}) AND rolling back the Zenoh slot \
                             `{slot_prefix}` failed ({re}); the slot is orphaned. Recover with \
                             `goat admin onboard {handle}{env_flag} --revoke` then re-run."
                        );
                    }
                }
            }
            // Browse / web-only tier: no slot to orphan; keep the NetBird bundle usable.
            eprintln!("warning: the Ory OIDC login leg FAILED: {e}");
            eprintln!(
                "  the NetBird bundle at {} is still usable. Provision the login \
                 separately via `goat admin onboard {handle}{}` (Ansible), or re-run once the \
                 site Thor / Kratos is reachable.",
                bundle_md_path.display(),
                env.map(|e| format!(" --env {e}")).unwrap_or_default(),
            );
            (None, Some(e.to_string()))
        }
    };

    // Record the person->slot linkage now that BOTH halves exist.
    //
    // This is the second call rather than a field on the slot-allocation
    // request because the legs run slot-then-Ory: at allocation time the
    // Kratos identity does not exist yet, so there is no UUID to send. That
    // ordering is deliberate (a failed login leg rolls the slot back, which
    // is what makes the onboard atomic), so the linkage is stamped here
    // instead of reordering and putting the rollback at risk.
    //
    // NON-FATAL by design. The person is onboarded and their credentials
    // work; a missing linkage degrades ownership to the alias-handle name
    // join — bad, but not worth failing a completed onboard over, and the warning
    // names the exact repair. A silent failure here ships NULL linkage on every slot.
    if let (Some(o), Some(id)) = (
        onboarded.as_ref(),
        ory_prov.as_ref().and_then(|o| o.identity_id.as_deref()),
    ) {
        // A reused slot already carries a linkage; it must be this person's.
        // Never relink one person's slot to another (#1576).
        if let Some(msg) = reused_slot_identity_conflict(o, id) {
            revoke_minted_setup_keys(env, &ctx.profile, nb_prov.as_ref()).await;
            finish_audit("failed", Some(msg.clone())).await;
            anyhow::bail!("{msg}");
        }
        match portal.stamp_slot_identity(&o.prefix, id).await {
            Ok(()) => eprintln!(
                "recorded person->slot linkage: slot {} -> identity {id}",
                o.prefix
            ),
            Err(e) => eprintln!(
                "warning: could not record the person->slot linkage for slot {} ({e}). The \
                 onboard SUCCEEDED and the credentials work, but ownership of this namespace \
                 will be derived by name join (alias<->handle) instead of the recorded \
                 identity. Repair with: PUT /api/slots/{}/identity {{\"identity_id\":\"{id}\"}}",
                o.prefix, o.prefix
            ),
        }
    } else if onboarded.is_some() && ory_prov.is_some() {
        eprintln!(
            "warning: the Ory leg returned no identity id, so the person->slot linkage was not \
             recorded; ownership will fall back to the alias<->handle name join."
        );
    }

    // Rewrite the bundle md to fold in the Ory login credential (if any). The
    // first write already captured the NetBird + Zenoh crash-insurance creds.
    if ory_prov.is_some() {
        let md2 = render_bundle_md(
            handle,
            env,
            onboarded.as_ref(),
            &portal_url,
            router_endpoint,
            nb_prov.as_ref(),
            ory_prov.as_ref(),
        );
        if let Err(e) = std::fs::write(&bundle_md_path, &md2) {
            eprintln!(
                "warning: could not rewrite bundle with the Ory section ({e}); the login \
                 credential is still printed below — capture it now."
            );
        }
    }

    // Audit outcome: "ok" when everything asked-for succeeded; "partial" when the
    // Zenoh/NetBird mint landed but the Ory leg failed (the error is recorded).
    match &ory_error {
        Some(e) => finish_audit("partial", Some(format!("ory: {e}"))).await,
        None => finish_audit("ok", None).await,
    };

    let mut surfaces: Vec<&str> = Vec::new();
    if nb_prov.is_some() {
        surfaces.push("NetBird");
    }
    if ory_prov.is_some() {
        surfaces.push("Ory");
    }
    if onboarded.is_some() {
        surfaces.push("Zenoh");
    }
    println!(
        "onboarded `{handle}` (native, no Ansible) — {}",
        surfaces.join(" + ")
    );
    if let Some(o) = &onboarded {
        let anchor = if o.slot_id.is_empty() {
            &o.prefix
        } else {
            &o.slot_id
        };
        println!("  write anchor: {anchor}");
        if o.reused {
            println!(
                "  fabric slot:  REUSED — this handle already held a live slot; nothing was minted. \
                 A credential already collected still works; an uncollected one is collected from \
                 the console."
            );
        }
        println!("  uri_san:      {}", o.uri_san);
        println!("  api_key:      {}", o.api_key);
    }
    if let Some(nb) = &nb_prov {
        println!("  setup key:    {}  (key `{}`)", nb.setup_key, nb.key_name);
        if let (Some(hk), Some(hn)) = (&nb.handset_key, &nb.handset_key_name) {
            println!("{}", handset_key_line(hk, hn));
        }
    }
    if let Some(ory) = &ory_prov {
        if let Some(pw) = &ory.password {
            println!("  ory login:    {}  (password `{}`)", ory.email, pw);
        } else if let Some(link) = &ory.recovery_link {
            println!(
                "  ory login:    {}  (passkey link, 15 min: {})",
                ory.email, link
            );
        } else {
            println!("  ory login:    {}  (identity already existed)", ory.email);
        }
    } else if ory_error.is_some() {
        println!("  ory login:    NOT PROVISIONED — see the warning above");
    }
    // Record the mint in the console store. Without this the CLI path writes an
    // operator manifest and nothing the console can see, so every lifecycle lane —
    // the dormancy sweep, the archive pass, the usage check, all of which enumerate
    // that store — is blind to anyone onboarded this way.
    //
    // BEST-EFFORT ON PURPOSE, AND LOUD. The credential already exists by the
    // time we get here; failing the command now would report a failure for work
    // that succeeded and invite a re-run that mints a second set. So a failure
    // warns with the exact command to repair it rather than aborting — but it
    // does warn, because a silent miss here is precisely how the gap opened.
    match portal
        .register_minted_proposal(
            handle,
            ory_prov.as_ref().map(|o| o.email.as_str()).unwrap_or(""),
            env.unwrap_or(""),
            &audit_actor,
            "minted by `goat admin onboard --native`; no review gate applies",
        )
        .await
    {
        Ok(true) => println!("  console row:  registered"),
        Ok(false) => println!("  console row:  already present (nothing written)"),
        Err(e) => eprintln!(
            "WARNING: the mint succeeded but the console row was NOT registered ({e}). \n         \
             This person is invisible to the dormancy sweep, the archive pass and the usage \n         \
             check until it is. Repair with:\n           \
             ops/identity/backfill-console-rows.py --env {} --handle {handle} --apply",
            env.unwrap_or("<env>")
        ),
    }

    println!("  bundle (md):  {}", bundle_md_path.display());
    println!("  cert tarball: {tarball_note}");
    eprintln!(
        "Distribute the tarball + bundle OOB; share the api_key + setup key{} separately. The \
         tarball URL is one-time (~24 h).",
        if ory_prov.is_some() {
            " + login password/link"
        } else {
            ""
        }
    );
    Ok(())
}

/// `goat admin operator onboard <handle>` — backend-OPERATOR tier wrapper
/// around `ops/ansible/operator-tier-onboard.yml`. Distinct from `onboard()`
/// (the participant tier): mints the operator's `operator-trial-peer` leaf,
/// installs their host SSH key, grants the Ory admin group on the EXISTING
/// identity, and prints the blast-radius additions. Self-contained — it
/// mirrors `onboard()`'s child-process streaming + SIGINT + audit rather than
/// sharing a runner, so the tested participant path is untouched. Pre-flight:
/// the manifest must exist (the playbook's O0.1 asserts the `operator:` block).
#[allow(clippy::disallowed_methods)]
pub async fn operator_onboard(
    profile: &crate::profile::Profile,
    handle: &str,
    env: Option<&str>,
    repo_root: Option<&Path>,
    dry_run: bool,
) -> anyhow::Result<()> {
    if let Some(env) = env {
        if !is_valid_env(env) {
            anyhow::bail!(
                "invalid --env `{env}`: must be a kebab-case goatnet id \
                 (^[a-z][a-z0-9-]*[a-z0-9]$, e.g. `zagros`, `zagros-prod`)"
            );
        }
    }

    let cwd = std::env::current_dir()?;
    let repo_root = match repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); \
                 pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    let operators_dir = repo_root.join("ops/operators");
    let manifest = match env {
        Some(env) => operators_dir.join(env).join(format!("{handle}.yml")),
        None => operators_dir.join(format!("{handle}.yml")),
    };
    if !manifest.exists() {
        anyhow::bail!(
            "no operator manifest at {} — author it (a participant manifest \
             PLUS an `operator:` block) per docs/operations/admin-operator-guide.md",
            manifest.display()
        );
    }

    let ansible_dir = repo_root.join("ops/ansible");
    let preflight = repo_root.join("ops/scripts/ansible-preflight.sh");
    if !preflight.exists() {
        anyhow::bail!(
            "ansible-preflight.sh not found at {} — repo layout looks wrong",
            preflight.display()
        );
    }

    let args = build_operator_tier_args(handle, env);

    if dry_run {
        eprintln!(
            "dry-run: (cwd {}) {} {}",
            ansible_dir.display(),
            preflight.display(),
            args.join(" ")
        );
        return Ok(());
    }

    // Operator-action audit: started -> ok/failed, action
    // `admin.operator-onboard`. The goatnet is --env, else the active context.
    let audit_env = audit_goatnet(env, profile);
    let audit_actor = crate::cmd::audit::actor_handle(profile);
    let audit_digest = format!("operator-tier onboard handle={handle} env={audit_env}");
    crate::cmd::audit::emit(
        profile,
        crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            "admin.operator-onboard",
            handle,
            audit_env,
            "started",
        )
        .with_args_digest(&audit_digest),
    )
    .await;

    let mut child = Command::new(&preflight)
        .args(&args)
        .current_dir(&ansible_dir)
        .env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn ansible-preflight.sh: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("{line}");
        }
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(());
    });

    let interrupted: bool;
    let wait_result: std::io::Result<std::process::ExitStatus> = tokio::select! {
        s = child.wait() => { interrupted = false; s }
        _ = shutdown_rx => {
            eprintln!("\ngoat admin operator onboard: SIGINT received, killing ansible-preflight.sh");
            interrupted = true;
            let _ = child.kill().await;
            child.wait().await
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let finish_audit = |outcome: &'static str, err: Option<String>| {
        let mut a = crate::cmd::audit::OperatorActionV1::new(
            &audit_actor,
            "goat-cli",
            "admin.operator-onboard",
            handle,
            audit_env,
            outcome,
        )
        .with_args_digest(&audit_digest);
        if let Some(e) = err {
            a = a.with_error(e);
        }
        crate::cmd::audit::emit(profile, a)
    };

    if interrupted {
        let msg = format!(
            "aborted by Ctrl-C (child exit: {})",
            wait_result
                .map(|st| format!("{st}"))
                .unwrap_or_else(|e| format!("wait-failed: {e}")),
        );
        finish_audit("failed", Some(msg.clone())).await;
        return Err(anyhow::anyhow!("{msg}"));
    }
    let status = match wait_result {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("child wait failed: {e}");
            finish_audit("failed", Some(msg.clone())).await;
            return Err(anyhow::anyhow!("{msg}"));
        }
    };

    if !status.success() {
        let msg = format!(
            "ansible-preflight.sh exited {} for operator-tier onboard operator={}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            handle,
        );
        finish_audit("failed", Some(msg.clone())).await;
        anyhow::bail!("{msg}");
    }
    finish_audit("ok", None).await;
    eprintln!("goat admin operator onboard: {handle} complete");
    Ok(())
}

// ---- Phase F.4: admin client onboard ----------------------------------

/// Minimal device manifest shape — mirrors
/// ops/devices/schema/device-manifest.schema.json. We only deserialize
/// the fields we hand to bundle-create; serde ignores the rest.
#[derive(Debug, serde::Deserialize)]
struct DeviceManifest {
    identity: DeviceIdentity,
    device: DeviceBlock,
    bundle: DeviceBundle,
}

#[derive(Debug, serde::Deserialize)]
struct DeviceIdentity {
    handle: String,
    #[serde(default)]
    status: String,
}

#[derive(Debug, serde::Deserialize)]
struct DeviceBlock {
    device_id: String,
    site: String,
    #[serde(default)]
    peer_pubkey: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DeviceBundle {
    acl_groups: Vec<String>,
    #[serde(default = "default_ttl_days")]
    ttl_days: u32,
    #[serde(default = "default_activation_days")]
    activation_days: u32,
    #[serde(default)]
    mesh_route_subnet: Option<String>,
    #[serde(default)]
    snitch: bool,
}

fn default_ttl_days() -> u32 {
    365
}
fn default_activation_days() -> u32 {
    14
}

async fn run_client(profile: &crate::profile::Profile, op: &AdminClientCmd) -> anyhow::Result<()> {
    match op {
        AdminClientCmd::Onboard {
            handle,
            ca_id,
            mode,
            age_identity,
            ca_key,
            tls_trust_anchor,
            snitch_collector,
            repo_root,
            dry_run,
        } => {
            client_onboard(
                profile,
                ClientOnboardArgs {
                    handle,
                    ca_id: ca_id.as_deref(),
                    mode,
                    age_identity: age_identity.as_deref(),
                    ca_key: ca_key.as_deref(),
                    tls_trust_anchor: tls_trust_anchor.as_deref(),
                    snitch_collector: snitch_collector.as_deref(),
                    repo_root: repo_root.as_deref(),
                    dry_run: *dry_run,
                },
            )
            .await
        }
    }
}

struct ClientOnboardArgs<'a> {
    handle: &'a str,
    ca_id: Option<&'a str>,
    mode: &'a str,
    age_identity: Option<&'a Path>,
    ca_key: Option<&'a Path>,
    tls_trust_anchor: Option<&'a Path>,
    snitch_collector: Option<&'a str>,
    repo_root: Option<&'a Path>,
    dry_run: bool,
}

// Spawns stream child stdout/stderr (see `onboard`); the disallowed-methods
// lint is goat-relay-hot-path-only, so the explicit allow is the
// clippy.toml-prescribed escape hatch for this CLI orchestration path.
#[allow(clippy::disallowed_methods)]
async fn client_onboard(
    profile: &crate::profile::Profile,
    args: ClientOnboardArgs<'_>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = match args.repo_root {
        Some(p) => p.to_path_buf(),
        None => discover_repo_root(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate repo root from {} (no .git ancestor); pass --repo-root <path>",
                cwd.display()
            )
        })?,
    };

    let manifest_path = repo_root
        .join("ops/devices")
        .join(format!("{}.yml", args.handle));
    if !manifest_path.exists() {
        anyhow::bail!(
            "no device manifest at {} — author it from ops/devices/example.yml.example first",
            manifest_path.display()
        );
    }

    let body = std::fs::read_to_string(&manifest_path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", manifest_path.display()))?;
    let manifest: DeviceManifest = serde_yaml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", manifest_path.display()))?;

    if manifest.identity.handle != args.handle {
        anyhow::bail!(
            "manifest handle `{}` does not match filename `{}.yml`",
            manifest.identity.handle,
            args.handle
        );
    }
    if manifest.identity.status == "revoked" {
        anyhow::bail!(
            "manifest {} is status=revoked — refusing to mint a new bundle for a revoked device",
            manifest_path.display()
        );
    }

    let ca_id = args.ca_id.ok_or_else(|| {
        anyhow::anyhow!(
            "--ca-id (or $GOAT_DEV_CA_ID) is required (e.g. dev-goat-ca-2026-04-20)"
        )
    })?;

    let out_dir = repo_root.join("ops/devices/out");
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| anyhow::anyhow!("mkdir {}: {e}", out_dir.display()))?;
    let out_path = out_dir.join(format!("{}-device.cbor", args.handle));

    // Per CLAUDE.md §"Running Ansible" + bundle-create's design, we run
    // bundle-create from `ops/enrollment/` via `go run ./cmd/bundle-
    // create` so it picks up the right go.mod. This mirrors
    // operator-onboard.yml Phase 4.5.4.
    let enrollment_dir = repo_root.join("ops/enrollment");
    if !enrollment_dir.join("go.mod").exists() {
        anyhow::bail!(
            "{}/go.mod not found — repo layout looks wrong",
            enrollment_dir.display()
        );
    }

    let mut argv: Vec<String> = vec![
        "run".into(),
        "./cmd/bundle-create".into(),
        "-mode".into(),
        args.mode.to_string(),
        "-ca-id".into(),
        ca_id.to_string(),
        "-site".into(),
        manifest.device.site.clone(),
        "-device-id".into(),
        manifest.device.device_id.clone(),
        "-acl-groups".into(),
        manifest.bundle.acl_groups.join(","),
        "-ttl-days".into(),
        manifest.bundle.ttl_days.to_string(),
        "-activation-days".into(),
        manifest.bundle.activation_days.to_string(),
        "-out".into(),
        out_path.to_string_lossy().to_string(),
    ];

    match args.mode {
        "dev" => {
            let ai = args.age_identity.ok_or_else(|| {
                anyhow::anyhow!("--mode=dev requires --age-identity (or $GOAT_DEV_CA_AGE_IDENTITY)")
            })?;
            let ck = args.ca_key.ok_or_else(|| {
                anyhow::anyhow!("--mode=dev requires --ca-key (or $GOAT_DEV_CA_KEY)")
            })?;
            argv.push("-age-identity".into());
            argv.push(ai.to_string_lossy().to_string());
            argv.push("-ca-key".into());
            argv.push(ck.to_string_lossy().to_string());
        }
        "prod" => {
            // YubiKey PIV: bundle-create prompts for PIN on stdin if
            // $GOAT_CA_PIN not set. The wrapper doesn't touch the PIN.
        }
        other => anyhow::bail!("--mode must be `dev` or `prod`, got `{other}`"),
    }

    if let Some(p) = args.tls_trust_anchor {
        argv.push("-tls-trust-anchor".into());
        argv.push(p.to_string_lossy().to_string());
    }
    if let Some(pk) = &manifest.device.peer_pubkey {
        argv.push("-peer-pubkey".into());
        argv.push(pk.clone());
    }
    if let Some(subnet) = &manifest.bundle.mesh_route_subnet {
        argv.push("-mesh-route-subnet".into());
        argv.push(subnet.clone());
    }
    if manifest.bundle.snitch {
        argv.push("-snitch".into());
        if let Some(collector) = args.snitch_collector {
            argv.push("-snitch-collector".into());
            argv.push(collector.to_string());
        }
    }

    if args.dry_run {
        eprintln!(
            "dry-run: (cwd {}) go {}",
            enrollment_dir.display(),
            argv.join(" "),
        );
        eprintln!("  output bundle would land at: {}", out_path.display());
        return Ok(());
    }

    let mut child = Command::new("go")
        .args(&argv)
        .current_dir(&enrollment_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn `go run ./cmd/bundle-create`: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("{line}");
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| anyhow::anyhow!("bundle-create wait failed: {e}"))?;

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    // Operator-action audit: a cert.mint event after bundle-create
    // returns. target = the device handle/CN; surface = "bundle-create".
    // Best-effort — never changes this command's own exit. (Dry-run already
    // returned above, so this only fires on a real mint.)
    {
        let actor = crate::cmd::audit::actor_handle(profile);
        let outcome = if status.success() { "ok" } else { "failed" };
        let mut a = crate::cmd::audit::OperatorActionV1::new(
            &actor,
            "bundle-create",
            "cert.mint",
            args.handle,
            manifest.device.site.clone(),
            outcome,
        )
        .with_args_digest(&format!(
            "cert.mint ca_id={ca_id} mode={} device_id={}",
            args.mode, manifest.device.device_id
        ));
        if !status.success() {
            a = a.with_error(format!(
                "bundle-create exited {}",
                status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
            ));
        }
        crate::cmd::audit::emit(profile, a).await;
    }

    if !status.success() {
        anyhow::bail!(
            "bundle-create exited {} for device {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            args.handle,
        );
    }
    eprintln!(
        "goat admin client onboard: minted bundle at {} (hand to device-side operator for `goat client bundle import`)",
        out_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn onboard_args_default_is_enroll() {
        let args = build_onboard_ansible_args("maintainer-perry", None, false);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "localhost,");
        assert_eq!(args[2], "--connection");
        assert_eq!(args[3], "local");
        assert_eq!(args[4], "operator-onboard.yml");
        assert_eq!(args[5], "-e");
        assert_eq!(args[6], "operator=maintainer-perry");
        // No env marker without --env, no revoke marker on enroll.
        assert!(!args.iter().any(|a| a.starts_with("operator_env=")));
        assert!(!args.iter().any(|a| a == "action=revoke"));
    }

    #[test]
    fn onboard_args_with_revoke_adds_action_marker() {
        let args = build_onboard_ansible_args("maintainer-perry", None, true);
        // The action marker must be `-e action=revoke` as two adjacent
        // tokens — ansible's extra-vars parser refuses `--action=...`
        // and won't see the value if the `-e` precedes a different `-e`.
        let idx = args.iter().position(|a| a == "action=revoke").unwrap();
        assert!(idx >= 2, "action=revoke must follow a -e token");
        assert_eq!(args[idx - 1], "-e");
    }

    #[test]
    fn onboard_args_handle_is_quoted_in_extra_var() {
        // Handles with hyphens flow through verbatim. The receiving
        // ansible playbook treats `operator=<handle>` as a single
        // extra-var; hyphens don't need shell-quoting at this layer.
        let args = build_onboard_ansible_args("multi-word-handle", None, false);
        assert!(args.iter().any(|a| a == "operator=multi-word-handle"));
    }

    #[test]
    fn resolve_org_members_filters_by_affiliation_and_active() {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("docs/operators/zagros");
        std::fs::create_dir_all(&ops).unwrap();
        std::fs::write(
            ops.join("roster.json"),
            r#"{
              "orgs": [],
              "members": [
                {"handle":"org63x-bob","type":"person","org":"org63x","vendor_tag":"org63x-bob","status":"active"},
                {"handle":"org63x-sensor","type":"system","org":"org63x","vendor_tag":null,"status":"active"},
                {"handle":"org63x-old","type":"person","org":"org63x","vendor_tag":"org63x-old","status":"revoked"},
                {"handle":"acme-jane","type":"person","org":"acme","vendor_tag":"acme-jane","status":"active"}
              ]
            }"#,
        )
        .unwrap();

        // Only ACTIVE members of org `org63x`, sorted by handle; the revoked one
        // and the other org are excluded; the system sibling IS included. The
        // per-member vendor_tag rides along (null -> None) for rotate's
        // anchor-resolution key.
        let members = resolve_org_members(dir.path(), "org63x", Some("zagros")).unwrap();
        assert_eq!(
            members,
            vec![
                OrgMember {
                    handle: "org63x-bob".into(),
                    kind: "person".into(),
                    vendor_tag: Some("org63x-bob".into()),
                },
                OrgMember {
                    handle: "org63x-sensor".into(),
                    kind: "system".into(),
                    vendor_tag: None,
                },
            ]
        );
        // Unknown org -> empty (caller treats as "nothing to revoke").
        assert!(resolve_org_members(dir.path(), "nope", Some("zagros")).unwrap().is_empty());
    }

    // --- console proposal offboard leg (revoke_console_proposal) -----------------

    async fn mock_list_proposals(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .and(path("/api/onboard/proposals"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn revoke_console_proposal_none_when_no_matching_row() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "none");
    }

    #[tokio::test]
    async fn revoke_console_proposal_none_for_terminal_statuses() {
        async fn run(status: &str) {
            let server = MockServer::start().await;
            let body = serde_json::json!([
                {
                    "id": "p1",
                    "env": "zagros",
                    "handle": "alice",
                    "email": "alice@example.test",
                    "status": status,
                    "escalated": false,
                    "complete": true,
                    "petitions": "[]"
                }
            ])
            .to_string();
            mock_list_proposals(&server, &body).await;

            // Terminal proposals must not trigger an offboard decision.
            Mock::given(method("POST"))
                .and(path("/api/onboard/proposals/p1/offboard"))
                .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
                .expect(0)
                .mount(&server)
                .await;

            let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
            let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
                .await
                .unwrap();
            assert_eq!(outcome, "none");
        }

        // A rejected, failed, or already-offboarded proposal must be treated as
        // non-actionable by the revoke path.
        run("rejected").await;
        run("offboarded").await;
        run("failed").await;
    }

    #[tokio::test]
    async fn revoke_console_proposal_dry_run_does_not_call_offboard() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        // If the implementation ever called decide_proposal in dry-run mode,
        // this POST stub (500) would surface as an error instead of the
        // expected Ok("offboarded").
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", true)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    #[tokio::test]
    async fn revoke_console_proposal_conflict_yields_failed() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    // #579: an older console refuses an archived (or enrolled) row that a
    // current CLI lists as eligible. The row is still live, so the leg still
    // reads failed; only the note changes.
    #[tokio::test]
    async fn revoke_console_proposal_conflict_on_archived_row_yields_failed() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "note": "key expired unredeemed",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    #[test]
    fn offboard_conflict_note_names_the_console_release_for_newer_states() {
        for status in ["archived", "enrolled", "ENROLLED"] {
            let note = offboard_conflict_note("p1", "alice", "zagros", status);
            assert!(note.contains("before #456"), "{status}: {note}");
            assert!(
                note.contains("the zagros console predates it, deploy its current release"),
                "{status}: {note}"
            );
            assert!(
                note.contains(&format!("listed as {status}")),
                "{status}: {note}"
            );
            assert!(
                note.contains("still live and blocks a fresh invite"),
                "{status}: {note}"
            );
            assert!(
                !note.contains("offboarded in the identity console"),
                "{status}: {note}"
            );
        }
        let onboarded = offboard_conflict_note("p1", "alice", "zagros", "onboarded");
        assert!(
            onboarded.contains("offboarded in the identity console"),
            "{onboarded}"
        );
        assert!(!onboarded.contains("#456"), "{onboarded}");
    }

    // A refused key (401/403) is not a per-row condition: every later row
    // would be refused the same way. The loop stops after the first refusal,
    // the second row is never posted, and the outcome still reads "failed".
    #[tokio::test]
    async fn revoke_console_proposal_refused_key_stops_after_first_row() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "first",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            },
            {
                "id": "second",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/first/offboard"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/second/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(0)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    #[tokio::test]
    async fn revoke_console_proposal_not_found_yields_none() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "none");
    }

    // A non-409 failure on one row is that row's failure, not the leg's: the
    // outcome is "failed" and the loop moves on (#578). Returning the error
    // here skipped every later eligible row after one transient 500.
    #[tokio::test]
    async fn revoke_console_proposal_server_error_yields_failed() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    // The mixed case from #578: an archived row that answers 500, followed by a
    // live onboarded row. The live row is still decided, and the receipt still
    // reads "failed" because one eligible row is uncut.
    #[tokio::test]
    async fn revoke_console_proposal_server_error_does_not_skip_later_rows() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "flaky",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            },
            {
                "id": "live",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/flaky/offboard"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/live/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    #[tokio::test]
    async fn revoke_console_proposal_matches_handle_and_env_case_insensitively() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "p1",
                "env": "ZAGROS",
                "handle": "Alice-Op",
                "email": "alice@example.test",
                "status": "ONBOARDED",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice-op", "zagros", true)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    // Regression: when several rows share a handle+env — exactly what the
    // re-enrollment path (offboard -> re-invite -> re-onboard) produces — the
    // leg must offboard the live row and leave the terminal one alone. The
    // terminal row's offboard endpoint is mounted with expect(0), so deciding
    // it would fail the mock verification.
    #[tokio::test]
    async fn revoke_console_proposal_skips_terminal_row_among_multiple() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "old",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "offboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            },
            {
                "id": "live",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/live/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/old/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(0)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    #[tokio::test]
    async fn revoke_console_proposal_enrolled_row_is_offboarded() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "enrolled",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "enrolled",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/enrolled/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    #[tokio::test]
    async fn revoke_console_proposal_archived_row_is_offboarded() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "archived",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "escalated": false,
                "complete": true,
                "petitions": "[]",
                "note": "dormant"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        // The console refuses an archived-row offboard without a note, so the
        // leg must send one — and it must carry the old archive reason
        // forward, not replace it (#1283); an unmatched body leaves this
        // mock unmet (404).
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/archived/offboard"))
            .and(body_partial_json(serde_json::json!({
                "note": "offboarded by goat-cli revoke; archived because: dormant"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    // An onboarded row's note belongs to the admin who wrote it. The leg
    // sends a blank note, which the console's blank-keeps rule leaves alone;
    // any non-blank body fails to match this mock (404 → "failed"), which is
    // exactly the #1283 regression.
    #[tokio::test]
    async fn revoke_console_proposal_onboarded_row_keeps_admin_note() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "onboarded",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]",
                "note": "on leave until October"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/onboarded/offboard"))
            .and(body_partial_json(serde_json::json!({ "note": "" })))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    #[test]
    fn revoke_offboard_note_per_status() {
        // Reversible states: blank, so the console keeps the existing note.
        assert_eq!(revoke_offboard_note("onboarded", "on leave"), "");
        assert_eq!(revoke_offboard_note("enrolled", "on leave"), "");
        assert_eq!(revoke_offboard_note("ENROLLED", ""), "");
        // Archived: the old reason travels with the offboard.
        assert_eq!(
            revoke_offboard_note("archived", "  left the unit "),
            "offboarded by goat-cli revoke; archived because: left the unit"
        );
        // Archived with no reason on file: still non-blank, so the console's
        // one-way gate accepts it.
        assert_eq!(
            revoke_offboard_note("Archived", "  "),
            "offboarded by goat-cli revoke"
        );
    }

    // A long archive reason is bounded on this side, with a visible marker,
    // so the console's silent tail-truncation never eats the reason.
    #[test]
    fn revoke_offboard_note_bounds_a_long_reason() {
        let prefix = "offboarded by goat-cli revoke; archived because: ";
        // Exactly at budget: untouched.
        let fits = "x".repeat(REVOKE_NOTE_MAX_BYTES - prefix.len());
        let note = revoke_offboard_note("archived", &fits);
        assert_eq!(note.len(), REVOKE_NOTE_MAX_BYTES);
        assert!(!note.ends_with("[…]"));
        // One over: cut, marked, still within the console's cap.
        let long = "x".repeat(REVOKE_NOTE_MAX_BYTES - prefix.len() + 1);
        let note = revoke_offboard_note("archived", &long);
        assert!(note.len() <= REVOKE_NOTE_MAX_BYTES, "{}", note.len());
        assert!(note.starts_with(prefix));
        assert!(note.ends_with(" […]"));
        // Multi-byte text: the cut lands on a char boundary, never mid-rune.
        let wide = "é".repeat(REVOKE_NOTE_MAX_BYTES);
        let note = revoke_offboard_note("archived", &wide);
        assert!(note.len() <= REVOKE_NOTE_MAX_BYTES);
        assert!(note.ends_with(" […]"));
        let body = note
            .strip_prefix(prefix)
            .and_then(|b| b.strip_suffix(" […]"))
            .expect("prefix and marker present");
        assert!(!body.is_empty() && body.chars().all(|c| c == 'é'));
    }

    // A revoke must cut EVERY offboard-eligible row, not a preferred one: a
    // leftover `archived` row still blocks a fresh invite on the same email
    // (the #414 duplicate-email guard treats it as active), so leaving it
    // standing while reporting "offboarded" would dead-end the re-invite.
    #[tokio::test]
    async fn revoke_console_proposal_offboards_all_eligible_rows() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "old",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            },
            {
                "id": "live",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/live/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/old/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "offboarded");
    }

    // A per-row 409 must surface as "failed" even when another row was cut —
    // the receipt may never report success while an eligible row is still
    // live. The remaining rows are still attempted (best-effort per row).
    #[tokio::test]
    async fn revoke_console_proposal_partial_conflict_yields_failed() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "stuck",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "archived",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            },
            {
                "id": "live",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/live/offboard"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/stuck/offboard"))
            .respond_with(ResponseTemplate::new(409).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let outcome = revoke_console_proposal(&portal, "alice", "zagros", false)
            .await
            .unwrap();
        assert_eq!(outcome, "failed");
    }

    // --- operator-key self-service door (#1373) ---------------------------------

    #[tokio::test]
    async fn operator_key_request_without_handle_files_for_self() {
        let server = MockServer::start().await;
        // No handle → an empty handle on the wire; the console binds it to the
        // person the token belongs to and answers with the roster handle.
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys"))
            .and(body_partial_json(serde_json::json!({ "handle": "", "reason": "first key" })))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "id": "r1", "handle": "bennett", "status": "requested", "env": "zagros"
                })
                .to_string(),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        run_operator_key(
            &portal,
            AdminOperatorKeyCmd::Request {
                handle: None,
                email: String::new(),
                env: String::new(),
                reason: "first key".into(),
                json: true,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn operator_key_self_request_without_reason_never_reaches_the_console() {
        let server = MockServer::start().await;
        // No mock is mounted: a request would 404 and fail differently.
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let err = run_operator_key(
            &portal,
            AdminOperatorKeyCmd::Request {
                handle: None,
                email: String::new(),
                env: String::new(),
                reason: "  ".into(),
                json: false,
            },
        )
        .await
        .unwrap_err();
        let rendered = format!("{err:#}");
        assert!(rendered.contains("--reason is required"), "{rendered}");
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn operator_key_collect_without_handle_sends_an_empty_handle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys/r1/collect"))
            .and(wiremock::matchers::query_param("handle", ""))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                serde_json::json!({ "handle": "bennett", "key": "op_x" }).to_string(),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        run_operator_key(
            &portal,
            AdminOperatorKeyCmd::Collect {
                id: "r1".into(),
                handle: None,
                json: true,
            },
        )
        .await
        .unwrap();
    }

    // The console answers 403 operator_only from the operator-key door AND from
    // unrelated gates (org registry, proposal decisions). Only the operator-key
    // verbs may render the operator-key wording.
    fn operator_only_refusal(message: &str) -> ResponseTemplate {
        ResponseTemplate::new(403).set_body_string(
            serde_json::json!({ "error": { "code": "operator_only", "message": message } })
                .to_string(),
        )
    }

    fn not_admin_hint(err: &anyhow::Error) -> String {
        err.downcast_ref::<crate::errors::GoatError>()
            .expect("a 403 stays E_NOT_ADMIN")
            .hint()
            .to_string()
    }

    #[tokio::test]
    async fn operator_only_on_operator_key_request_names_the_operator_key_verbs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys"))
            .respond_with(operator_only_refusal("operator-only action"))
            .expect(1)
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let err = run_operator_key(
            &portal,
            AdminOperatorKeyCmd::Request {
                handle: None,
                email: String::new(),
                env: String::new(),
                reason: "org creation".into(),
                json: false,
            },
        )
        .await
        .unwrap_err();
        let hint = not_admin_hint(&err);
        assert!(hint.contains("Which operator-key verbs take which token"), "{hint}");
        assert!(hint.contains("request (no handle), collect <id>"), "{hint}");
    }

    #[tokio::test]
    async fn operator_only_on_invite_keeps_the_shared_operator_hint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/invite"))
            .respond_with(operator_only_refusal("this action is operator-only"))
            .expect(1)
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let err = run_invite(
            &portal,
            None,
            false,
            Some("zagros".into()),
            Some("developer".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap_err();
        let hint = not_admin_hint(&err);
        assert!(hint.contains("needs your OPERATOR key"), "{hint}");
        assert!(!hint.contains("Which operator-key verbs"), "{hint}");
    }

    // --- portal-key offboard leg (revoke_operator_keys) --------------------------

    async fn mock_list_operator_keys(server: &MockServer, body: &str) {
        // Pinning `status=all` keeps the leg honest: the console defaults an
        // empty/missing status to REQUESTED only, so a caller that drops
        // `all` would silently see only pending rows in prod. Here it gets a
        // 404 from the mock instead.
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys"))
            .and(wiremock::matchers::query_param("status", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    fn ok_key_row(id: &str, handle: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(
            serde_json::json!({ "id": id, "handle": handle, "status": "revoked" }).to_string(),
        )
    }

    #[tokio::test]
    async fn revoke_operator_keys_none_when_person_holds_no_key() {
        let server = MockServer::start().await;
        mock_list_operator_keys(&server, "[]").await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "none");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_revokes_active_and_closes_pending() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            { "id": "a1", "handle": "alice", "env": "zagros", "status": "active" },
            { "id": "r1", "handle": "alice", "env": "zagros", "status": "requested" },
            // Terminal rows for the same person are skipped, not re-cut.
            { "id": "x1", "handle": "alice", "env": "zagros", "status": "revoked" },
            { "id": "x2", "handle": "alice", "env": "zagros", "status": "rejected" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/a1/revoke"))
            .respond_with(ok_key_row("a1", "alice"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/r1/reject"))
            .respond_with(ok_key_row("r1", "alice"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=1");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_ignores_other_handles() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            { "id": "a1", "handle": "alice", "env": "zagros", "status": "active" },
            { "id": "b1", "handle": "bob", "env": "zagros", "status": "active" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;

        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/a1/revoke"))
            .respond_with(ok_key_row("a1", "alice"))
            .expect(1)
            .mount(&server)
            .await;
        // Bob's key must never be touched when we revoke alice.
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/b1/revoke"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=0");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_spans_all_goatnets() {
        let server = MockServer::start().await;
        // Same person holds a key on two different goatnets — both must be cut.
        let body = serde_json::json!([
            { "id": "k-zagros", "handle": "alice", "env": "zagros", "status": "active" },
            { "id": "k-dev", "handle": "alice", "env": "zagros-dev", "status": "active" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/k-zagros/revoke"))
            .respond_with(ok_key_row("k-zagros", "alice"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/k-dev/revoke"))
            .respond_with(ok_key_row("k-dev", "alice"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=2 closed=0");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_matches_handle_case_insensitively() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            { "id": "a1", "handle": "Alice", "env": "zagros", "status": "active" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/a1/revoke"))
            .respond_with(ok_key_row("a1", "Alice"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=0");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_dry_run_does_not_decide() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            { "id": "a1", "handle": "alice", "env": "zagros", "status": "active" },
            { "id": "r1", "handle": "alice", "env": "zagros", "status": "requested" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        // If dry-run ever POSTed, this 500 would surface as a failure.
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/a1/revoke"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/r1/reject"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", true).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=1");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_records_per_key_failure_best_effort() {
        let server = MockServer::start().await;
        // Two active keys: one revoke succeeds, one fails. The failure must be
        // recorded (any_failed) but must not abort the other cut.
        let body = serde_json::json!([
            { "id": "ok", "handle": "alice", "env": "zagros", "status": "active" },
            { "id": "bad", "handle": "alice", "env": "zagros-dev", "status": "active" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/ok/revoke"))
            .respond_with(ok_key_row("ok", "alice"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/bad/revoke"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=0 failed=1");
        assert!(out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_404_on_decide_is_not_a_failure() {
        let server = MockServer::start().await;
        // The key went terminal between the list and the decide (someone else cut
        // it). Nothing is left live, so this is not our failure.
        let body = serde_json::json!([
            { "id": "gone", "handle": "alice", "env": "zagros", "status": "active" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/gone/revoke"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "none");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_409_after_concurrent_cut_is_not_a_failure() {
        let server = MockServer::start().await;
        // The real console never deletes key rows: a decide on a row that
        // already went terminal answers 409 wrong_state, not 404. The leg
        // re-reads the row, sees it terminal, and treats the race as benign.
        let active = serde_json::json!([
            { "id": "a1", "handle": "alice", "env": "zagros", "status": "active" },
        ])
        .to_string();
        let terminal = serde_json::json!([
            { "id": "a1", "handle": "alice", "env": "zagros", "status": "revoked" },
        ])
        .to_string();
        // Insertion order is load-bearing: wiremock serves the first matching
        // mock, and up_to_n_times(1) expires this one after the initial list,
        // so the post-409 re-read sees the terminal row.
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys"))
            .and(wiremock::matchers::query_param("status", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_string(active))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mock_list_operator_keys(&server, &terminal).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/a1/revoke"))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "none");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_409_reject_of_freshly_approved_request_revokes_instead() {
        let server = MockServer::start().await;
        // The dangerous half of the wrong-state race: the pending request was
        // APPROVED between the list and the reject — a live key. The leg must
        // re-read the row and REVOKE it, not shrug the 409 off as benign.
        let requested = serde_json::json!([
            { "id": "r1", "handle": "alice", "env": "zagros", "status": "requested" },
        ])
        .to_string();
        let now_active = serde_json::json!([
            { "id": "r1", "handle": "alice", "env": "zagros", "status": "active" },
        ])
        .to_string();
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys"))
            .and(wiremock::matchers::query_param("status", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_string(requested))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mock_list_operator_keys(&server, &now_active).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/r1/reject"))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/r1/revoke"))
            .respond_with(ok_key_row("r1", "alice"))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=1 closed=0");
        assert!(!out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_two_person_refusal_on_reject_is_recorded() {
        let server = MockServer::start().await;
        // The console's requester ≠ decider rule (two-person, #210) gates the
        // reject arm: a self-offboard — the decider IS the departing owner —
        // is refused 403. That pending request stays live-approvable, so the
        // leg must land it as failed → partial, never a silent success.
        let body = serde_json::json!([
            { "id": "r1", "handle": "alice", "env": "zagros", "status": "requested" },
        ])
        .to_string();
        mock_list_operator_keys(&server, &body).await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/operator-keys/r1/reject"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                serde_json::json!({
                    "error": "self_decision",
                    "message": "this is your own request — a different operator principal must decide it",
                })
                .to_string(),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let out = revoke_operator_keys(&portal, "alice", false).await.unwrap();
        assert_eq!(out.summary, "revoked=0 closed=0 failed=1");
        assert!(out.any_failed);
    }

    #[tokio::test]
    async fn revoke_operator_keys_list_error_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        // The caller (revoke_one) turns this Err into a recorded "failed" leg.
        assert!(revoke_operator_keys(&portal, "alice", false).await.is_err());
    }

    #[tokio::test]
    async fn revoke_operator_keys_disabled_portal_yields_disabled() {
        // An empty endpoint/key yields the Disabled client. The caller maps
        // this to "skipped" — no portal keys can exist where no portal is
        // configured — never a spurious `partial`.
        let portal = panoscope_core::portal::PortalClient::new(String::new(), String::new());
        let err = revoke_operator_keys(&portal, "alice", false)
            .await
            .unwrap_err();
        assert!(matches!(err, PortalError::Disabled));
    }

    #[test]
    fn resolve_org_members_missing_roster_is_error() {
        let dir = TempDir::new().unwrap();
        // No roster.json -> hard error, never a silent empty (no under-cut).
        assert!(resolve_org_members(dir.path(), "org63x", Some("zagros")).is_err());
        assert!(resolve_org_members(dir.path(), "org63x", None).is_err());
    }

    #[test]
    fn resolve_org_members_reads_the_named_goatnets_roster_not_the_union() {
        // A cascade against a goatnet must span that goatnet's declared members.
        // The union carries every environment's; resolving from it
        // prints people the operator is not cutting here as if they were.
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("docs/operators");
        std::fs::create_dir_all(ops.join("zagros-prod")).unwrap();
        std::fs::write(
            ops.join("roster.json"),
            r#"{"orgs":[],"members":[
                {"handle":"union-only","type":"person","org":"org63x","vendor_tag":null,"status":"active"},
                {"handle":"prod-alice","type":"person","org":"org63x","vendor_tag":null,"status":"active"}
              ]}"#,
        )
        .unwrap();
        std::fs::write(
            ops.join("zagros-prod/roster.json"),
            r#"{"orgs":[],"members":[
                {"handle":"prod-alice","type":"person","org":"org63x","vendor_tag":null,"status":"active"}
              ]}"#,
        )
        .unwrap();

        let scoped = resolve_org_members(dir.path(), "org63x", Some("zagros-prod")).unwrap();
        assert_eq!(
            scoped.iter().map(|m| m.handle.as_str()).collect::<Vec<_>>(),
            vec!["prod-alice"],
            "a named goatnet resolves its own roster only"
        );

        let unscoped = resolve_org_members(dir.path(), "org63x", None).unwrap();
        assert_eq!(
            unscoped.len(),
            2,
            "with no goatnet named the union is the documented fallback"
        );
    }

    #[test]
    fn resolve_member_anchors_keys_on_vendor_tag_only_no_handle_false_match() {
        use panoscope_core::alias::AliasRegistry;
        use panoscope_core::portal::AliasRow;
        // The alias map keys on the slot alias = the member's vendor_tag (by
        // onboard construction). `org63x-sensor` is a slot alias here.
        let reg = AliasRegistry::from_rows(vec![
            AliasRow {
                slot_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                alias: "org63x-bob".into(),
            },
            AliasRow {
                slot_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                alias: "org63x-sensor".into(),
            },
        ]);
        let members = vec![
            // Resolves: vendor_tag matches its slot alias.
            OrgMember {
                handle: "org63x-bob-host".into(),
                kind: "person".into(),
                vendor_tag: Some("org63x-bob".into()),
            },
            // Tag-less: must SKIP, even though its HANDLE (`org63x-sensor`) equals
            // another slot's alias — a handle fallback would mis-rotate that
            // other peer. Authoritative key only -> None.
            OrgMember {
                handle: "org63x-sensor".into(),
                kind: "system".into(),
                vendor_tag: None,
            },
            // Has a tag but no matching slot (not minted / revoked) -> None.
            OrgMember {
                handle: "org63x-ghost".into(),
                kind: "system".into(),
                vendor_tag: Some("org63x-ghost".into()),
            },
        ];
        let resolved = resolve_member_anchors(&reg, &members);
        assert_eq!(
            resolved[0].1.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(resolved[1].1, None); // no false handle match
        assert_eq!(resolved[2].1, None);
        assert_eq!(resolved.iter().filter(|(_, a)| a.is_some()).count(), 1);
    }

    #[test]
    fn onboard_args_with_env_adds_operator_env_marker() {
        let args = build_onboard_ansible_args("user147x", Some("zagros"), false);
        // operator_env must ride as `-e operator_env=zagros` (two adjacent
        // tokens), same parser constraint as action=revoke.
        let idx = args
            .iter()
            .position(|a| a == "operator_env=zagros")
            .expect("operator_env extra-var present");
        assert_eq!(args[idx - 1], "-e");
        // operator= still present and precedes the env marker.
        assert!(args.iter().any(|a| a == "operator=user147x"));
    }

    #[test]
    fn onboard_args_env_and_revoke_compose() {
        let args = build_onboard_ansible_args("user144x", Some("zagros"), true);
        assert!(args.iter().any(|a| a == "operator_env=zagros"));
        assert!(args.iter().any(|a| a == "action=revoke"));
    }

    #[test]
    fn operator_tier_args_target_the_operator_tier_playbook() {
        let args = build_operator_tier_args("contributora", None);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "localhost,");
        assert_eq!(args[2], "--connection");
        assert_eq!(args[3], "local");
        // The distinguishing bit vs the participant onboard: the playbook name.
        assert_eq!(args[4], "operator-tier-onboard.yml");
        assert_eq!(args[5], "-e");
        assert_eq!(args[6], "operator=contributora");
        // No revoke/action concept on the operator tier; no env marker without --env.
        assert!(!args.iter().any(|a| a == "action=revoke"));
        assert!(!args.iter().any(|a| a.starts_with("operator_env=")));
    }

    #[test]
    fn operator_tier_args_with_env_adds_operator_env_marker() {
        let args = build_operator_tier_args("contributora", Some("zagros"));
        assert!(args.iter().any(|a| a == "operator-tier-onboard.yml"));
        assert!(args.iter().any(|a| a == "operator=contributora"));
        assert!(args.iter().any(|a| a == "operator_env=zagros"));
    }

    #[test]
    fn ssh_key_install_args_carry_env_and_base_without_key_on_argv() {
        let args =
            build_ssh_key_install_args("zagros-prod", false, "https://onboard.example", "/repo");
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "localhost,");
        assert_eq!(args[2], "--connection");
        assert_eq!(args[3], "local");
        assert_eq!(args[4], "operator-ssh-key-install.yml");
        assert!(args.iter().any(|a| a == "onboard_env=zagros-prod"));
        assert!(args.iter().any(|a| a == "onboard_api_base=https://onboard.example"));
        assert!(
            args.iter().any(|a| a == r#"{"onboard_runner_repo":"/repo"}"#),
            "manifest root must point at the local checkout, not the mgmt-host default",
        );
        assert!(
            !args.iter().any(|a| a.contains("onboard_api_key")),
            "API key must not ride on argv",
        );
    }

    #[test]
    fn ssh_key_install_args_survive_a_repo_path_with_spaces() {
        // key=value extra-vars go through Ansible's parse_kv, which splits on
        // whitespace INSIDE the argv element — a space in the checkout path
        // would truncate the manifest root and skip-report every key as
        // no_manifest. The JSON extra-var form is immune.
        let args = build_ssh_key_install_args(
            "zagros",
            false,
            "https://onboard.example",
            "/Users/John Smith/dev/goat",
        );
        assert!(
            args.iter()
                .any(|a| a == r#"{"onboard_runner_repo":"/Users/John Smith/dev/goat"}"#),
            "repo root must ride as a single JSON extra-var, never key=value",
        );
        assert!(!args.iter().any(|a| a.starts_with("onboard_runner_repo=")));
    }

    #[test]
    fn ssh_key_install_args_add_check_flag_when_requested() {
        let args =
            build_ssh_key_install_args("zagros-prod", true, "https://onboard.example", "/repo");
        let idx = args
            .iter()
            .position(|a| a == "ssh_key_install_check=true")
            .expect("ssh_key_install_check marker present");
        assert_eq!(args[idx - 1], "-e");
    }

    #[test]
    fn is_valid_env_accepts_goatnet_ids() {
        assert!(is_valid_env("zagros"));
        assert!(is_valid_env("zagros-dev"));
        assert!(is_valid_env("vega"));
        assert!(is_valid_env("nova-twin"));
    }

    #[test]
    fn is_valid_env_rejects_path_traversal_and_bad_shapes() {
        assert!(!is_valid_env(".."));
        assert!(!is_valid_env("../etc"));
        assert!(!is_valid_env("a/b"));
        assert!(!is_valid_env("ZAGROS")); // uppercase
        assert!(!is_valid_env("-zagros")); // leading hyphen
        assert!(!is_valid_env("zagros-")); // trailing hyphen
        assert!(!is_valid_env("e")); // too short (min 2)
        assert!(!is_valid_env("")); // empty
    }

    #[test]
    fn revoke_peer_matches_is_handle_anchored_no_orgmate_falsepositive() {
        // Own device — untagged form.
        assert!(revoke_peer_matches(
            "peer-pat-example-zagros-2026-06-10-mac",
            "pat-example",
            "",
            "zagros"
        ));
        // Own device — tagged form (the goatnet mints the vendor tag into the name).
        assert!(revoke_peer_matches(
            "peer-user105x-org67x-zagros-2026-06-14-atomaionedge",
            "user105x",
            "org67x",
            "zagros"
        ));
        // The bug the sweep hit: an org-mate's peer carries the SHARED org tag
        // mid-name but a DIFFERENT handle — must NOT match.
        assert!(!revoke_peer_matches(
            "peer-user192x-org112x-naomi-zagros-2026-06-14-okroop759-mac",
            "user194x",
            "org112x",
            "zagros"
        ));
        // A handle that is a prefix of another must not cross-match.
        assert!(!revoke_peer_matches(
            "peer-user318x-2-zagros-2026-06-15-mac",
            "user318x",
            "",
            "zagros"
        ));
        // Wrong env doesn't match.
        assert!(!revoke_peer_matches(
            "peer-pat-example-vega-2026-06-10-mac",
            "pat-example",
            "",
            "zagros"
        ));
    }

    #[test]
    fn parse_handle_list_ignores_comments_and_blanks() {
        let text = "# cohort\npat-example\n\n  lee-example  # keep\n#skip\nsam-example\n";
        assert_eq!(
            parse_handle_list(text),
            vec!["pat-example", "lee-example", "sam-example"]
        );
        assert!(parse_handle_list("# only comments\n\n").is_empty());
    }

    #[test]
    fn revoke_outcome_fails_only_when_a_leg_failed_and_it_was_not_a_dry_run() {
        let outcome = |any_failed: bool| RevokeOutcome {
            handle: "some-handle".to_string(),
            peers_cut: 1,
            ory: "deactivated",
            zenoh: "none",
            manifest: "revoked",
            proposal: "none",
            collab: "n/a",
            operator_keys: "none".to_string(),
            ssh_keys: "none".to_string(),
            netbird_user: "blocked",
            any_failed,
        };
        assert!(outcome(true).should_fail(false));
        assert!(!outcome(true).should_fail(true), "a dry run attempted nothing");
        assert!(!outcome(false).should_fail(false));
        assert!(!outcome(false).should_fail(true));
    }

    #[test]
    fn flip_manifest_status_is_idempotent_and_targets_identity_block() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("m.yml");
        std::fs::write(
            &p,
            "identity:\n  handle: x\n  status: active\nzenoh:\n  enabled: true\n",
        )
        .unwrap();
        assert_eq!(flip_manifest_status_to_revoked(&p).unwrap(), "revoked");
        assert!(std::fs::read_to_string(&p)
            .unwrap()
            .contains("\n  status: revoked"));
        // Idempotent: a second flip is a no-op.
        assert_eq!(
            flip_manifest_status_to_revoked(&p).unwrap(),
            "already-revoked"
        );
        // No active status line at all.
        let p2 = td.path().join("n.yml");
        std::fs::write(&p2, "identity:\n  handle: y\n").unwrap();
        assert_eq!(
            flip_manifest_status_to_revoked(&p2).unwrap(),
            "no-status-line"
        );
    }

    #[test]
    fn census_active_tag_handles_is_env_scoped_and_active_only() {
        let td = TempDir::new().unwrap();
        let zagros = td.path().join("ops/operators/zagros");
        std::fs::create_dir_all(&zagros).unwrap();
        let mk = |dir: &Path, name: &str, tag: &str, status: &str| {
            std::fs::write(
                dir.join(name),
                format!("identity:\n  handle: h\n  status: {status}\norg:\n  short_tag: {tag}\n"),
            )
            .unwrap();
        };
        // Two active members share a tag; one revoked doesn't count.
        mk(&zagros, "alex.yml", "org63x-shared", "active");
        mk(&zagros, "john.yml", "org63x-shared", "active");
        mk(&zagros, "vince.yml", "org70x", "revoked");
        // A person's LEGACY flat manifest with the same tag must NOT count as a
        // teammate in this goatnet (env-scoped, non-recursive).
        let flat = td.path().join("ops/operators");
        mk(&flat, "alex.yml", "org63x-shared", "active");

        let c = census_active_tag_handles(&zagros);
        // handles are file stems; both actives present, revoked absent.
        let mut shared = c.get("org63x-shared").cloned().unwrap_or_default();
        shared.sort();
        assert_eq!(shared, vec!["alex".to_string(), "john".to_string()]);
        assert!(!c.contains_key("org70x"));
        // The flat dir's copy is out of scope: census unaffected (still 2).
        assert_eq!(c.get("org63x-shared").map(|v| v.len()), Some(2));
    }

    #[test]
    fn discover_repo_root_finds_dot_git_ancestor() {
        let td = TempDir::new().unwrap();
        let root = td.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        let nested = root.join("crates/foo/src");
        std::fs::create_dir_all(&nested).unwrap();
        let found = discover_repo_root(&nested).unwrap();
        // Canonicalize to defeat macOS' `/private/var/...` symlink shim
        // so the equality holds for tempdirs rooted under /var/folders.
        assert_eq!(found.canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn discover_repo_root_returns_none_when_no_dot_git() {
        let td = TempDir::new().unwrap();
        // No .git anywhere.
        assert!(discover_repo_root(td.path()).is_none());
    }

    #[test]
    fn discover_repo_root_at_root_itself() {
        let td = TempDir::new().unwrap();
        std::fs::create_dir(td.path().join(".git")).unwrap();
        let found = discover_repo_root(td.path()).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            td.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn onboard_endpoint_hint_present_on_fallback_admin() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        );
        let err = crate::cmd::vendor::map_onboard_err(
            &portal,
            PortalError::Transient("temporary outage".to_string()),
        );
        let goat = err
            .downcast_ref::<crate::errors::GoatError>()
            .expect("map_onboard_err must yield GoatError");
        let rendered = format!("{}", crate::errors::DisplayHuman(goat));
        assert!(
            rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "hint missing on fallback portal: {rendered}"
        );
    }

    #[test]
    fn onboard_endpoint_hint_absent_when_configured_admin() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        )
        .with_onboard_endpoint("https://onboard.example");
        let err = crate::cmd::vendor::map_onboard_err(
            &portal,
            PortalError::Transient("temporary outage".to_string()),
        );
        let goat = err
            .downcast_ref::<crate::errors::GoatError>()
            .expect("map_onboard_err must yield GoatError");
        let rendered = format!("{}", crate::errors::DisplayHuman(goat));
        assert!(
            !rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "hint must be absent when onboard endpoint is configured: {rendered}"
        );
    }

    #[test]
    fn onboard_endpoint_hint_list_ssh_keys_keeps_raw_error() {
        let portal = panoscope_core::portal::PortalClient::new(
            "https://portal.example".to_string(),
            "k".to_string(),
        );
        let err = crate::cmd::vendor::append_onboard_hint(
            &portal,
            PortalError::Transient("temporary outage".to_string()).into(),
        );
        assert!(
            err.downcast_ref::<crate::errors::GoatError>().is_none(),
            "list_ssh_keys path must not promote errors to GoatError"
        );
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "hint missing on raw PortalError: {rendered}"
        );
    }

    #[tokio::test]
    async fn org_registry_suggest_404_onboard_endpoint_hint_present() {
        use crate::output::{ColorChoice, OutputContext};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/org-registry/suggest"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let profile = crate::profile::Profile {
            dir: tmp.path().join("profile"),
            context: "default".into(),
        };
        std::fs::create_dir_all(profile.context_dir()).unwrap();
        std::fs::write(
            profile.profile_toml_path(),
            format!("portal_endpoint = \"{}\"\n", server.uri()),
        )
        .unwrap();
        std::fs::write(profile.portal_token_path(), "test-token").unwrap();

        // Ensure the onboard endpoint falls back to the portal base URL.
        let _guard = crate::cmd::access::OnboardEndpointEnvGuard::set_for_test(None);

        let ctx = crate::cmd::Ctx {
            profile,
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };

        let err = super::run(
            &ctx,
            AdminCmd::Vendor {
                op: VendorCmd::Suggest {
                    name: "acme".to_string(),
                    nation: None,
                },
            },
        )
        .await
        .expect_err("org-registry suggest 404 must surface an error");

        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("org-registry lookup returned 404"),
            "must report the 404 without asserting a single cause: {rendered}"
        );
        assert!(
            rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "must add onboard-endpoint configuration hint on 404 with fallback onboard endpoint: {rendered}"
        );
    }

    // --- revoke_one integration: proposal leg error handling -----------------

    #[tokio::test]
    async fn revoke_one_marks_proposal_failure_and_audit_on_console_error() {
        use crate::output::{ColorChoice, OutputContext};

        // Wire a mock console endpoint that lists an onboarded proposal and
        // returns HTTP 500 on the offboard POST.
        let server = MockServer::start().await;
        let proposals_body = serde_json::json!([
            {
                "id": "p1",
                "env": "zagros",
                "handle": "alice",
                "email": "alice@example.test",
                "status": "onboarded",
                "escalated": false,
                "complete": true,
                "petitions": "[]"
            }
        ])
        .to_string();
        mock_list_proposals(&server, &proposals_body).await;
        // Empty key list keeps the portal-key leg (R3) at "none" so the
        // partial outcome asserted below is attributable to the proposal leg.
        mock_list_operator_keys(&server, "[]").await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/proposals/p1/offboard"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        // Minimal profile rooted in a temp dir so the audit fallback log has
        // somewhere to write. We only care that emit() runs and records the
        // partial outcome.
        let tmp = TempDir::new().unwrap();
        let profile_dir = tmp.path().join("profile");
        let profile = crate::profile::Profile {
            dir: profile_dir.clone(),
            context: "default".into(),
        };
        std::fs::create_dir_all(profile.context_dir()).unwrap();
        let toml_body = format!(
            "portal_endpoint = \"{url}\"\nonboard_endpoint = \"{url}\"\n",
            url = server.uri()
        );
        std::fs::write(profile.profile_toml_path(), toml_body).unwrap();
        std::fs::write(profile.portal_token_path(), "test-token").unwrap();

        // Ensure the audit emit path does NOT try to open a Zenoh session by
        // leaving GOAT_OPERATOR_AGE_RECIPIENTS empty: encrypt_to_roster then
        // fails fast and emit() degrades to the local fallback log.
        std::env::set_var("GOAT_OPERATOR_AGE_RECIPIENTS", "");

        let ctx = crate::cmd::Ctx {
            profile: profile.clone(),
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };

        // Repo root with a minimal manifest that disables NetBird/Zenoh/Ory so
        // the revoke surfaces reduce to the manifest flip + console leg.
        let repo_root = tmp.path().join("repo");
        let ops_env_dir = repo_root.join("ops/operators/zagros");
        std::fs::create_dir_all(&ops_env_dir).unwrap();
        std::fs::write(
            ops_env_dir.join("alice.yml"),
            "identity: { email: alice@example.test, status: active }\n\
org: { short_tag: acme }\n\
netbird: { enabled: false }\n\
ory_site: { enabled: false }\n\
zenoh: { enabled: false }\n\
collab: { enabled: false }\n",
        )
        .unwrap();

        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .expect("build_revoke_shared must succeed");

        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .expect("revoke_one must succeed despite console leg error");
        assert_eq!(outcome.proposal, "failed");

        // The audit fallback log must record a partial outcome with an
        // error message that includes proposal=failed, proving the proposal
        // leg is included in the any_failed aggregate.
        let log_path = profile.dir.join("audit/operator-actions.log");
        let body = std::fs::read_to_string(&log_path).expect("audit fallback log must exist");
        let last = body
            .lines()
            .last()
            .expect("audit log must contain at least one line");
        let v: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(v["action"], "admin.revoke-native");
        assert_eq!(v["outcome"], "partial");
        let err = v["error_msg"].as_str().unwrap_or("");
        assert!(
            err.contains("proposal=failed"),
            "error_msg missing proposal leg: {err}"
        );
    }

    /// A console failure on the portal-key leg marks the receipt `failed` and
    /// the audit outcome `partial`, but never aborts the revoke — the mesh cut
    /// must not be hostage to one unreachable surface.
    #[tokio::test]
    async fn revoke_one_marks_operator_keys_failed_and_audit_partial_on_console_error() {
        use crate::output::{ColorChoice, OutputContext};

        // The console lists no proposals (R0 = none) and 500s the
        // operator-key list, so the only failing leg is the portal-key one.
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/operator-keys"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        // Minimal profile rooted in a temp dir so the audit fallback log has
        // somewhere to write.
        let tmp = TempDir::new().unwrap();
        let profile_dir = tmp.path().join("profile");
        let profile = crate::profile::Profile {
            dir: profile_dir.clone(),
            context: "default".into(),
        };
        std::fs::create_dir_all(profile.context_dir()).unwrap();
        let toml_body = format!(
            "portal_endpoint = \"{url}\"\nonboard_endpoint = \"{url}\"\n",
            url = server.uri()
        );
        std::fs::write(profile.profile_toml_path(), toml_body).unwrap();
        std::fs::write(profile.portal_token_path(), "test-token").unwrap();

        // Leave GOAT_OPERATOR_AGE_RECIPIENTS empty so emit() degrades to the
        // local fallback log instead of opening a Zenoh session.
        std::env::set_var("GOAT_OPERATOR_AGE_RECIPIENTS", "");

        let ctx = crate::cmd::Ctx {
            profile: profile.clone(),
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };

        // Repo root with a minimal manifest that disables NetBird/Zenoh/Ory so
        // the revoke surfaces reduce to the manifest flip + console legs.
        let repo_root = tmp.path().join("repo");
        let ops_env_dir = repo_root.join("ops/operators/zagros");
        std::fs::create_dir_all(&ops_env_dir).unwrap();
        std::fs::write(
            ops_env_dir.join("alice.yml"),
            "identity: { email: alice@example.test, status: active }\n\
org: { short_tag: acme }\n\
netbird: { enabled: false }\n\
ory_site: { enabled: false }\n\
zenoh: { enabled: false }\n\
collab: { enabled: false }\n",
        )
        .unwrap();

        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .expect("build_revoke_shared must succeed");

        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .expect("revoke_one must succeed despite portal-key leg error");
        assert_eq!(outcome.operator_keys, "failed");

        // The audit fallback log must record a partial outcome whose error
        // message includes the portal-key leg, proving it feeds any_failed.
        let log_path = profile.dir.join("audit/operator-actions.log");
        let body = std::fs::read_to_string(&log_path).expect("audit fallback log must exist");
        let last = body
            .lines()
            .last()
            .expect("audit log must contain at least one line");
        let v: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(v["action"], "admin.revoke-native");
        assert_eq!(v["outcome"], "partial");
        let err = v["error_msg"].as_str().unwrap_or("");
        assert!(
            err.contains("operator_keys=failed"),
            "error_msg missing portal-key leg: {err}"
        );
    }

    #[tokio::test]
    async fn revoke_one_retains_independent_setup_key_and_user_cut_failures() {
        for (setup_failed, user_failed) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let server = MockServer::start().await;
            mock_list_proposals(&server, "[]").await;
            mock_list_operator_keys(&server, "[]").await;
            mock_list_ssh_keys(&server, "[]").await;
            Mock::given(method("GET"))
                .and(path("/api/users"))
                .respond_with(if user_failed {
                    ResponseTemplate::new(500)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
                })
                .expect(1)
                .mount(&server)
                .await;

            let tmp = TempDir::new().unwrap();
            let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
            let manifest_path = repo_root.join("ops/operators/zagros/alice.yml");
            let mut manifest: serde_yaml::Value =
                serde_yaml::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
            manifest["netbird"]["enabled"] = serde_yaml::Value::Bool(true);
            manifest["ory_site"]["enabled"] = serde_yaml::Value::Bool(true);
            std::fs::write(&manifest_path, serde_yaml::to_string(&manifest).unwrap()).unwrap();
            // Isolate the two mesh cuts: no Kratos call, but a durable target
            // from an earlier attempt still requires a verified account cut.
            let targets =
                revoke_target_directory(&ctx.profile, "zagros", "alice", "alice@example.test", None);
            record_revoke_target(&targets, "00000000-0000-4000-8000-000000000001").unwrap();
            let shared = RevokeShared {
                portal: ctx.profile.portal_client().unwrap(),
                nb: Some(panoscope_core::netbird::NetbirdClient::new(
                    server.uri(),
                    "fixture".into(),
                )),
                peers: vec![],
                setup_keys: if setup_failed {
                    Err("fixture snapshot failure".into())
                } else {
                    Ok(vec![])
                },
                active_tag_handles: Default::default(),
                ory: None,
                env_label: "zagros".into(),
                repo_root,
            };
            let outcome = revoke_one(
                &ctx,
                &shared,
                "alice",
                Some("zagros"),
                false,
                false,
                false,
                None,
            )
            .await
            .unwrap();
            assert_eq!(outcome.should_fail(false), setup_failed || user_failed);
            assert_eq!(
                outcome.netbird_user,
                if user_failed { "failed" } else { "verified" }
            );
            assert_eq!(outcome.manifest, "revoked", "other legs must still run");
            let audit = std::fs::read_to_string(ctx.profile.dir.join("audit/operator-actions.log"))
                .unwrap();
            let event: serde_json::Value =
                serde_json::from_str(audit.lines().last().unwrap()).unwrap();
            assert_eq!(
                event["outcome"],
                if setup_failed || user_failed {
                    "partial"
                } else {
                    "ok"
                }
            );
            if setup_failed || user_failed {
                let message = event["error_msg"].as_str().unwrap();
                assert!(message.contains(if setup_failed {
                    "setup_keys=failed"
                } else {
                    "setup_keys=ok"
                }));
                assert!(message.contains(if user_failed {
                    "netbird_user=failed"
                } else {
                    "netbird_user=verified"
                }));
            }
        }
    }

    // --- revoke_one: SSH-key residual leg, show half (goat#371) --------------

    /// Common scaffolding for the revoke_one console-leg tests: a profile
    /// pointed at the mock console, and a repo with one minimal manifest (all
    /// other surfaces disabled) so revoke_one reduces to the console legs.
    async fn console_leg_test_fixture(
        server: &MockServer,
        tmp: &TempDir,
    ) -> (crate::cmd::Ctx, std::path::PathBuf) {
        use crate::output::{ColorChoice, OutputContext};
        let profile_dir = tmp.path().join("profile");
        let profile = crate::profile::Profile {
            dir: profile_dir.clone(),
            context: "default".into(),
        };
        std::fs::create_dir_all(profile.context_dir()).unwrap();
        let toml_body = format!(
            "portal_endpoint = \"{url}\"\nonboard_endpoint = \"{url}\"\n",
            url = server.uri()
        );
        std::fs::write(profile.profile_toml_path(), toml_body).unwrap();
        std::fs::write(profile.portal_token_path(), "test-token").unwrap();
        std::env::set_var("GOAT_OPERATOR_AGE_RECIPIENTS", "");
        let ctx = crate::cmd::Ctx {
            profile,
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };
        let repo_root = tmp.path().join("repo");
        let ops_env_dir = repo_root.join("ops/operators/zagros");
        std::fs::create_dir_all(&ops_env_dir).unwrap();
        std::fs::write(
            ops_env_dir.join("alice.yml"),
            "identity: { email: alice@example.test, status: active }\n\
org: { short_tag: acme }\n\
netbird: { enabled: false }\n\
ory_site: { enabled: false }\n\
zenoh: { enabled: false }\n\
collab: { enabled: false }\n",
        )
        .unwrap();
        (ctx, repo_root)
    }

    async fn mock_list_ssh_keys(server: &MockServer, body: &str) {
        // Same status-default trap as operator keys: empty = SUBMITTED only,
        // `all` = every state. The matcher keeps the leg honest.
        Mock::given(method("GET"))
            .and(path("/api/onboard/ssh-keys"))
            .and(wiremock::matchers::query_param("status", "all"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// Installed keys are REPORTED, never record-flipped: the console's revoke
    /// is a record-only state change, and flipping a row whose file is still
    /// on the hosts would hide the residual instead of surfacing it. Rows that
    /// have not reached a host yet (submitted/approved) are closed so nothing
    /// can be installed after the person is gone. Other handles untouched.
    #[tokio::test]
    async fn revoke_one_reports_installed_ssh_keys_and_closes_pending_grants() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        mock_list_ssh_keys(
            &server,
            &serde_json::json!([
                {"id": "sk-1", "handle": "alice", "env": "zagros", "status": "installed"},
                {"id": "sk-2", "handle": "alice", "env": "nova", "status": "approved"},
                {"id": "sk-3", "handle": "alice", "env": "zagros", "status": "submitted"},
                {"id": "sk-4", "handle": "bob",   "env": "zagros", "status": "installed"},
                {"id": "sk-5", "handle": "alice", "env": "zagros", "status": "revoked"}
            ])
            .to_string(),
        )
        .await;
        // The two pre-install rows are closed; the installed row must see NO
        // decision call at all.
        Mock::given(method("POST"))
            .and(path("/api/onboard/ssh-keys/sk-2/revoke"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"sk-2","handle":"alice","status":"revoked"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/ssh-keys/sk-3/revoke"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"sk-3","handle":"alice","status":"revoked"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/ssh-keys/sk-1/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.ssh_keys, "still-installed:1 closed:2");
    }

    /// A still-installed shell key makes the WHOLE revoke report partial —
    /// the person keeps a working login, and "ok" would say otherwise.
    #[tokio::test]
    async fn revoke_one_still_installed_ssh_key_marks_audit_partial() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        mock_list_ssh_keys(
            &server,
            &serde_json::json!([
                {"id": "sk-1", "handle": "alice", "env": "zagros", "status": "installed"}
            ])
            .to_string(),
        )
        .await;

        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.ssh_keys, "still-installed:1");

        let log_path = ctx.profile.dir.join("audit/operator-actions.log");
        let body = std::fs::read_to_string(&log_path).expect("audit fallback log must exist");
        let v: serde_json::Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
        assert_eq!(v["outcome"], "partial");
        let err = v["error_msg"].as_str().unwrap_or("");
        assert!(
            err.contains("ssh_keys=still-installed:1"),
            "error_msg missing ssh-key residual: {err}"
        );
    }

    /// No rows for the handle → "none", and that is success.
    #[tokio::test]
    async fn revoke_one_reports_none_when_no_ssh_keys() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        mock_list_ssh_keys(&server, "[]").await;

        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.ssh_keys, "none");
    }

    /// Console unreachable → the leg is "failed" (audit partial), never an
    /// abort; and dry-run closes nothing while still reporting the counts.
    #[tokio::test]
    async fn revoke_one_ssh_key_leg_failure_and_dry_run_contracts() {
        // Failure half.
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/ssh-keys"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .expect("revoke_one must succeed despite ssh-key leg error");
        assert_eq!(outcome.ssh_keys, "failed");

        // Dry-run half: pending rows counted, zero decision POSTs.
        let server2 = MockServer::start().await;
        mock_list_proposals(&server2, "[]").await;
        mock_list_operator_keys(&server2, "[]").await;
        mock_list_ssh_keys(
            &server2,
            &serde_json::json!([
                {"id": "sk-1", "handle": "alice", "env": "zagros", "status": "installed"},
                {"id": "sk-2", "handle": "alice", "env": "zagros", "status": "approved"}
            ])
            .to_string(),
        )
        .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::path_regex(
                r"^/api/onboard/ssh-keys/.+/revoke$",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server2)
            .await;
        let tmp2 = TempDir::new().unwrap();
        let (ctx2, repo_root2) = console_leg_test_fixture(&server2, &tmp2).await;
        let shared2 = build_revoke_shared(&ctx2, Some("zagros"), &repo_root2)
            .await
            .unwrap();
        let outcome2 = revoke_one(
            &ctx2,
            &shared2,
            "alice",
            Some("zagros"),
            false,
            false,
            true,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome2.ssh_keys, "still-installed:1 closed:1");
    }

    /// A failed pending close keeps the residual count: the receipt and the
    /// audit must still say a key is installed when something else went wrong.
    #[tokio::test]
    async fn revoke_one_ssh_key_close_failure_keeps_the_residual_count() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        mock_list_ssh_keys(
            &server,
            &serde_json::json!([
                {"id": "sk-1", "handle": "alice", "env": "zagros", "status": "installed"},
                {"id": "sk-2", "handle": "alice", "env": "zagros", "status": "approved"}
            ])
            .to_string(),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/ssh-keys/sk-2/revoke"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.ssh_keys, "still-installed:1 failed:1");
        assert!(outcome.any_failed);

        let log_path = ctx.profile.dir.join("audit/operator-actions.log");
        let body = std::fs::read_to_string(&log_path).expect("audit fallback log must exist");
        let v: serde_json::Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
        assert_eq!(v["outcome"], "partial");
        let err = v["error_msg"].as_str().unwrap_or("");
        assert!(
            err.contains("ssh_keys=still-installed:1 failed:1"),
            "error_msg lost the ssh-key residual: {err}"
        );
    }

    /// A 409 on the close means the row went rejected or revoked between the
    /// list and the decide: already cut, so neither counted nor a failure.
    #[tokio::test]
    async fn revoke_one_ssh_key_close_conflict_is_already_cut() {
        let server = MockServer::start().await;
        mock_list_proposals(&server, "[]").await;
        mock_list_operator_keys(&server, "[]").await;
        mock_list_ssh_keys(
            &server,
            &serde_json::json!([
                {"id": "sk-2", "handle": "alice", "env": "zagros", "status": "approved"}
            ])
            .to_string(),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/api/onboard/ssh-keys/sk-2/revoke"))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let (ctx, repo_root) = console_leg_test_fixture(&server, &tmp).await;
        let shared = build_revoke_shared(&ctx, Some("zagros"), &repo_root)
            .await
            .unwrap();
        let outcome = revoke_one(
            &ctx,
            &shared,
            "alice",
            Some("zagros"),
            false,
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.ssh_keys, "none");
        assert!(!outcome.any_failed);
    }

    /// The still-installed warning names the key, its fingerprint, and a
    /// cleanup that works for a console-queue key — not R1.6 alone, which
    /// removes only the manifest's key.
    #[test]
    fn still_installed_warning_names_a_working_cleanup() {
        let key: panoscope_core::portal::SshKey = serde_json::from_value(serde_json::json!({
            "id": "sk-1", "handle": "alice", "env": "zagros", "status": "installed",
            "key_type": "ssh-ed25519", "fingerprint": "SHA256:abc123"
        }))
        .unwrap();
        let w = still_installed_warning(&key);
        assert!(w.contains("sk-1"), "{w}");
        assert!(w.contains("SHA256:abc123"), "{w}");
        assert!(w.contains("operator.ssh_pubkey"), "{w}");
        assert!(w.contains("ssh-key revoke sk-1"), "{w}");

        let bare: panoscope_core::portal::SshKey = serde_json::from_value(serde_json::json!({
            "id": "sk-9", "handle": "alice", "env": "zagros", "status": "installed"
        }))
        .unwrap();
        assert!(still_installed_warning(&bare).contains("no fingerprint recorded"));
    }

    /// The operator-only gate's full truth table. Only the all-false row —
    /// not lan, uninitialized profile, no operator env — is refused; every
    /// other row proceeds to its authoritative downstream check. This is the
    /// "no false block of a configured admin" contract in one place.
    #[test]
    fn admin_may_proceed_truth_table() {
        // lan is always exempt (break-glass diagnostic), regardless of creds.
        assert!(admin_may_proceed(true, false, false));
        assert!(admin_may_proceed(true, false, true));
        // An initialized profile proceeds — authority is decided server-side.
        assert!(admin_may_proceed(false, true, false));
        // An operator env signal proceeds even on an uninitialized profile.
        assert!(admin_may_proceed(false, false, true));
        // The only refusal: a non-operator (uninitialized, no signal) who is
        // not running lan. This is the inert, fail-closed E_NOT_ADMIN case.
        assert!(!admin_may_proceed(false, false, false));
    }

    // ---- P2.1 native Zenoh onboard ----

    fn sample_onboarded() -> OnboardedOrg {
        serde_json::from_str(
            r#"{
                "prefix":"e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
                "api_key":"sk-xyz",
                "bundle_url":"/api/vendor-bundle/e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b/tok.tar.gz",
                "uri_san":"urn:goat:zagros:org=e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
                "slot_id":"e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
                "system_handle":"fra-land-bde"
            }"#,
        )
        .unwrap()
    }

    // ---- #1576: a deferred onboard may hand back the person's existing slot ----

    #[test]
    fn onboarded_org_reused_and_identity_default_off() {
        let o = sample_onboarded();
        assert!(!o.reused, "a portal that never sends `reused` means a fresh slot");
        assert!(o.identity_id.is_empty());
        let r: OnboardedOrg = serde_json::from_str(
            r#"{"prefix":"a","api_key":"","bundle_url":"","reused":true,"identity_id":"id-1"}"#,
        )
        .unwrap();
        assert!(r.reused);
        assert_eq!(r.identity_id, "id-1");
    }

    #[test]
    fn reused_slot_identity_conflict_only_on_a_different_recorded_identity() {
        let mut o = sample_onboarded();
        // A fresh slot never conflicts, whatever it says.
        o.identity_id = "someone-else".into();
        assert!(reused_slot_identity_conflict(&o, "me").is_none());
        o.reused = true;
        // Reused and linked to someone else: refuse, naming both.
        let msg = reused_slot_identity_conflict(&o, "me").expect("must refuse a relink");
        assert!(msg.contains("someone-else") && msg.contains("me"), "{msg}");
        // Reused and linked to the same person, or not linked: safe to stamp.
        o.identity_id = "me".into();
        assert!(reused_slot_identity_conflict(&o, "me").is_none());
        o.identity_id = String::new();
        assert!(reused_slot_identity_conflict(&o, "me").is_none());
    }

    // ---- re-onboard: an existing Ory identity's org and name follow the manifest ----

    fn org(st: &str, dn: &str) -> MfOrg {
        MfOrg {
            short_tag: st.into(),
            display_name: dn.into(),
        }
    }

    #[test]
    fn identity_resync_patch_moves_a_stale_legacy_traits_org() {
        // Org from an earlier onboarding.
        let existing = serde_json::json!({
            "id": "42a0",
            "traits": {"email": "m@x", "name": "Second Person",
                       "org": {"short_tag": "org131x", "display_name": "SVN CRC"}},
            "metadata_public": null
        });
        let (ops, changes) = identity_resync_patch(
            &existing,
            "Second Person",
            &org("org123x", "org123x"),
        );
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!(ops[0]["path"], "/traits/org");
        assert_eq!(ops[0]["value"]["short_tag"], "org123x");
        assert!(changes[0].contains("org131x") && changes[0].contains("org123x"));
    }

    #[test]
    fn identity_resync_patch_uses_metadata_public_when_present_and_never_creates_it() {
        let existing = serde_json::json!({
            "traits": {"name": "Old Name"},
            "metadata_public": {"groups": ["DEVELOPER"], "org": {"short_tag": "old"}}
        });
        let (ops, _) = identity_resync_patch(&existing, "New Name", &org("new", ""));
        let paths: Vec<&str> = ops.iter().map(|o| o["path"].as_str().unwrap()).collect();
        assert_eq!(paths, vec!["/traits/name", "/metadata_public/org"]);
        // Nothing for groups, credentials or state, ever.
        assert!(ops.iter().all(|o| {
            let p = o["path"].as_str().unwrap();
            !p.contains("groups") && !p.contains("credentials") && !p.contains("state")
        }));

        // No metadata_public and no legacy traits.org: nothing to move, and the
        // block is NOT created.
        let bare = serde_json::json!({"traits": {"name": "N"}, "metadata_public": null});
        let (ops, _) = identity_resync_patch(&bare, "N", &org("new", "New"));
        assert!(ops.is_empty(), "{ops:?}");
    }

    #[test]
    fn identity_resync_patch_is_empty_when_already_in_line() {
        let existing = serde_json::json!({
            "traits": {"name": "N", "org": {"short_tag": "a", "display_name": "A"}},
            "metadata_public": {"org": {"short_tag": "a", "display_name": "A"}}
        });
        let (ops, changes) = identity_resync_patch(&existing, "N", &org("a", "A"));
        assert!(ops.is_empty() && changes.is_empty());
        // An empty manifest name never blanks the stored one.
        let (ops, _) = identity_resync_patch(&existing, "  ", &org("a", "A"));
        assert!(ops.is_empty());
    }

    #[test]
    fn manifest_projects_zenoh_trial_vendor_and_surfaces() {
        let yaml = r#"
identity: { name: Acme Corporation }
vendor: { display_name: Acme Corp }
netbird: { enabled: true }
ory_site: { enabled: false }
zenoh:
  enabled: true
  trial:
    vendor: acme-corp-jane
unknown_future_field: { x: 1 }
"#;
        let m: OnboardManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(m.zenoh.enabled);
        assert_eq!(m.zenoh.trial.unwrap().org, "acme-corp-jane"); // legacy `vendor:` key → org field
        assert_eq!(m.org.display_name, "Acme Corp"); // legacy `vendor:` block → org field
        assert!(m.netbird.enabled); // surfaces --native must warn about
        assert!(!m.ory_site.enabled);
    }

    // F-274: the collab decision recorded on a revoke. goat-cli performs no
    // collab call — these values ARE the deliverable (receipt line + audit
    // digest), so they are pinned.
    #[test]
    fn collab_decision_records_the_operators_choice() {
        let with_collab: OnboardManifest = serde_yaml::from_str(
            "identity: { name: A }\ncollab: { enabled: true, account: jane-doe }\n",
        )
        .unwrap();
        let without: OnboardManifest = serde_yaml::from_str("identity: { name: A }\n").unwrap();

        // No collab on this plane ⇒ no decision is implied.
        assert_eq!(collab_decision_outcome(&without, None), "n/a");
        // Collab present, no --collab passed ⇒ the open question stays visible.
        assert_eq!(collab_decision_outcome(&with_collab, None), "undecided");
        assert_eq!(
            collab_decision_outcome(&with_collab, Some(CollabDecision::Keep)),
            "keep"
        );
        // `revoke` is a REQUEST: the cut is onboard-runner's, not this command's.
        assert_eq!(
            collab_decision_outcome(&with_collab, Some(CollabDecision::Revoke)),
            "revoke-requested"
        );
        // An explicit choice outranks a manifest that has no collab record yet —
        // that gap is the finding itself.
        assert_eq!(
            collab_decision_outcome(&without, Some(CollabDecision::Revoke)),
            "revoke-requested"
        );
    }

    // A hand-edited block that records the account but drops `enabled:` still
    // counts as having collab — the receipt must not read `n/a` for someone who
    // demonstrably holds an account.
    #[test]
    fn collab_decision_sees_an_account_only_block() {
        let m: OnboardManifest =
            serde_yaml::from_str("identity: { name: A }\ncollab: { account: jane-doe }\n").unwrap();
        assert_eq!(collab_decision_outcome(&m, None), "undecided");
    }

    // ADR 1052 additive org alias: a manifest authored with the new `org:` /
    // `zenoh.trial.org:` nouns must project to the SAME onboard inputs (prefix +
    // display_name) as the legacy `vendor:` shape — so it onboards identically.
    #[test]
    fn manifest_org_alias_projects_identically_to_vendor() {
        let vendor_yaml = r#"
identity: { name: Acme }
vendor: { display_name: Acme Corp }
zenoh: { enabled: true, trial: { vendor: acme-corp-jane } }
"#;
        let org_yaml = r#"
identity: { name: Acme }
org: { display_name: Acme Corp }
zenoh: { enabled: true, trial: { org: acme-corp-jane } }
"#;
        let v: OnboardManifest = serde_yaml::from_str(vendor_yaml).unwrap();
        let o: OnboardManifest = serde_yaml::from_str(org_yaml).unwrap();
        // Identical projection — the two onboard surfaces (publisher prefix +
        // portal display_name) come out the same regardless of which noun was authored.
        assert_eq!(
            v.zenoh.trial.as_ref().unwrap().org,
            o.zenoh.trial.as_ref().unwrap().org
        );
        assert_eq!(v.org.display_name, o.org.display_name);
        assert_eq!(o.zenoh.trial.unwrap().org, "acme-corp-jane");
        assert_eq!(o.org.display_name, "Acme Corp");
    }

    /// A profile helper for the overlay tests: a context dir with a profile.toml
    /// carrying whatever keys the case needs.
    fn profile_with(body: &str) -> (tempfile::TempDir, crate::profile::Profile) {
        let td = tempfile::tempdir().expect("tempdir");
        let ctx = td.path().join("contexts").join("default");
        std::fs::create_dir_all(&ctx).expect("mkdir");
        std::fs::write(ctx.join("profile.toml"), body).expect("write");
        let p = crate::profile::Profile {
            dir: td.path().to_path_buf(),
            context: "default".to_string(),
        };
        (td, p)
    }

    /// Three goatnets' facts as context fixtures, read the way production reads
    /// them: through a profile (`goatnet_endpoints_for`). goat itself holds no
    /// goatnet; these exist only so the onboard tests have facts to render.
    fn fixture_profile() -> &'static crate::profile::Profile {
        use std::sync::OnceLock;
        static P: OnceLock<(tempfile::TempDir, crate::profile::Profile)> = OnceLock::new();
        &P.get_or_init(|| {
            let td = tempfile::tempdir().expect("tempdir");
            let facts = [
                ("zagros", "tls/zenoh.zagros.netbird.zagros.example:7447", "https://netbird.zagros.example",
                 "ZAGROS_NB_API_TOKEN", "ZAGROS", "https://auth.zagros.netbird.zagros.example", "192.0.2.10", "goat"),
                ("zagros-prod", "tls/zenoh.zagros-prod.examplebird.zagros-prod.example:7447", "https://netbird.zagros-prod.example",
                 "ZAGROS_PROD_NB_API_TOKEN", "ZAGROS-PROD", "https://auth.zagros-prod.examplebird.zagros-prod.example", "192.0.2.20", "goat"),
                ("zagros-dev", "tls/zenoh.zagros-dev.examplebird.zagros-dev.example:7447", "https://netbird.zagros-dev.example",
                 "ZAGROS_DEV_NB_API_TOKEN", "ZAGROS-DEV", "https://auth.zagros-dev.examplebird.zagros-dev.example", "192.0.2.30", "goat"),
            ];
            for (g, router, nb, tok, prefix, auth, host, user) in facts {
                let d = td.path().join("contexts").join(g);
                std::fs::create_dir_all(&d).expect("mkdir");
                std::fs::write(
                    d.join("profile.toml"),
                    format!(
                        "goatnet = \"{g}\"\nrouter_endpoint = \"{router}\"\nnetbird_mgmt_url = \"{nb}\"\n\
                         netbird_token_env = \"{tok}\"\nnetbird_site_prefix = \"{prefix}\"\n\
                         netbird_rosenpass = \"false\"\nory_auth_url = \"{auth}\"\n\
                         ory_ssh_host = \"{host}\"\nory_ssh_user = \"{user}\"\n"
                    ),
                )
                .expect("write");
            }
            let p = crate::profile::Profile { dir: td.path().to_path_buf(), context: "zagros".to_string() };
            (td, p)
        })
        .1
    }

    fn native_router_endpoint(env: Option<&str>) -> Option<&'static str> {
        env?;
        native_router_endpoint_for(env, fixture_profile()).map(|s| &*Box::leak(s.into_boxed_str()))
    }

    fn native_netbird_config(env: Option<&str>) -> Option<NbEnvConfig> {
        env?;
        native_netbird_config_for(env, fixture_profile())
    }

    fn native_ory_config(env: Option<&str>) -> Option<OryEnvConfig> {
        env?;
        native_ory_config_for(env, fixture_profile())
    }

    #[test]
    fn a_context_can_serve_a_goatnet_the_table_never_heard_of() {
        // The F-485 case, inverted: no table row exists for this name, and the
        // onboard must still work because the context declares the facts.
        let (_td, p) = profile_with(
            r#"
portal_endpoint = "http://portal:8090"
router_endpoint = "tls/zenoh.newnet.example:7447"
netbird_mgmt_url = "https://netbird.newnet.example"
netbird_token_env = "NEWNET_NB_API_TOKEN"
netbird_site_prefix = "NEWNET"
ory_auth_url = "https://auth.newnet.example"
ory_ssh_host = "100.127.0.9"
ory_ssh_user = "operator"
"#,
        );
        let env = Some("newnet");
        assert!(native_router_endpoint(env).is_none(), "precondition: table has no row");

        assert_eq!(
            native_router_endpoint_for(env, &p).as_deref(),
            Some("tls/zenoh.newnet.example:7447")
        );
        let nb = native_netbird_config_for(env, &p).expect("profile alone must satisfy the mesh leg");
        assert_eq!(nb.mgmt_url, "https://netbird.newnet.example");
        assert_eq!(nb.token_env_var, "NEWNET_NB_API_TOKEN");
        assert_eq!(nb.site_prefix, "NEWNET");
        let ory = native_ory_config_for(env, &p).expect("profile alone must satisfy the login leg");
        assert_eq!(ory.auth_url, "https://auth.newnet.example");
        assert_eq!(ory.ssh_host, "100.127.0.9");
        assert_eq!(ory.ssh_user, "operator");
    }

    #[test]
    fn a_profile_without_facts_resolves_no_native_leg() {
        // goat holds no goatnet: a context that has not been refreshed knows none,
        // and every leg says so instead of guessing.
        let (_td, p) = profile_with("portal_endpoint = \"http://portal:8090\"\n");
        assert!(native_router_endpoint_for(Some("zagros"), &p).is_none());
        assert!(native_netbird_config_for(Some("zagros"), &p).is_none());
        assert!(native_ory_config_for(Some("zagros"), &p).is_none());
    }

    #[test]
    fn rosenpass_is_on_only_when_the_goatnet_says_true() {
        // Turning an intra-mesh transport on by omission is the one mistake this
        // must not make.
        let base = "netbird_mgmt_url = \"https://x\"\nnetbird_token_env = \"T\"\n";
        for (extra, on) in [
            ("", false),
            ("netbird_rosenpass = \"perhaps\"\n", false),
            ("netbird_rosenpass = \"false\"\n", false),
            ("netbird_rosenpass = \"true\"\n", true),
        ] {
            let (_td, p) = profile_with(&format!("{base}{extra}"));
            assert_eq!(native_netbird_config_for(Some("zagros"), &p).unwrap().rosenpass, on, "{extra:?}");
        }
    }

    #[test]
    fn audit_goatnet_never_names_a_goatnet_nobody_chose() {
        let (_td, p) = profile_with("portal_endpoint = \"http://portal:8090\"\n");
        assert_eq!(audit_goatnet(Some("zagros-prod"), &p), "zagros-prod", "--env wins");
        assert_eq!(audit_goatnet(None, &p), "default", "falls back to the CONTEXT");
        assert_eq!(audit_goatnet(Some("  "), &p), "default", "blank is not a goatnet");
        assert_ne!(audit_goatnet(None, &p), "nova", "never a hardcoded goatnet");
    }

    #[test]
    fn a_context_carrying_goatnet_toml_gives_every_native_leg_its_config() {
        // Every leg reads the same profile, so a goatnet whose context carries the
        // published facts is known to all three legs or to none.
        for env in ["zagros", "zagros-prod", "zagros-dev"] {
            assert!(
                native_router_endpoint(Some(env)).is_some(),
                "{env}: fabric leg"
            );
            assert!(
                native_netbird_config(Some(env)).is_some(),
                "{env}: mesh leg"
            );
            assert!(native_ory_config(Some(env)).is_some(), "{env}: login leg");
        }
    }

    #[test]
    fn unknown_goatnet_still_resolves_to_none() {
        // The fence above must not become a reason to make the tables permissive.
        // An undeclared goatnet stays None so the onboard refuses rather than
        // minting a partial identity.
        assert!(native_router_endpoint(Some("not-a-goatnet")).is_none());
        assert!(native_netbird_config(Some("not-a-goatnet")).is_none());
        assert!(native_ory_config(Some("not-a-goatnet")).is_none());
    }

    #[test]
    fn render_bundle_md_carries_credentials_and_anchor() {
        let o = sample_onboarded();
        let md = render_bundle_md(
            "fra-land-bde",
            Some("zagros"),
            Some(&o),
            "https://portal.zagros.netbird.zagros.example",
            native_router_endpoint(Some("zagros")),
            None, // no NetBird leg
            None, // no Ory leg — Zenoh-only bundle
        );
        // One-time credentials present (crash insurance).
        assert!(md.contains("sk-xyz"));
        assert!(md.contains(
            "https://portal.zagros.netbird.zagros.example/api/vendor-bundle/e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b/tok.tar.gz"
        ));
        // Slot anchor (not the handle) is the write namespace + topic path.
        assert!(md.contains("e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b/foo/v1"));
        assert!(md.contains("urn:goat:zagros:org="));
        // Known-env router endpoint pre-filled.
        assert!(md.contains("tls/zenoh.zagros.netbird.zagros.example:7447"));
        // No NetBird leg → no mesh section.
        assert!(!md.contains("## NetBird mesh access"));
    }

    #[test]
    fn render_bundle_md_unknown_env_points_at_tarball_for_endpoint() {
        let o = sample_onboarded();
        let md = render_bundle_md(
            "x",
            Some("newsite"),
            Some(&o),
            "https://p",
            native_router_endpoint(Some("newsite")),
            None,
            None,
        );
        assert!(md.contains("NOTES.md")); // no fabricated endpoint for unknown env
        assert!(!md.contains("tls/100.64")); // not the nova default
    }

    #[test]
    fn render_bundle_md_includes_netbird_section_when_provisioned() {
        let o = sample_onboarded();
        let nb = NbProvision {
            setup_key: "K-SECRET-123".into(),
            mgmt_url: "https://netbird.zagros.example".into(),
            key_name: "peer-fra-land-bde-zagros-2026-06-25".into(),
            expires_days: 30,
            usage_limit: 5,
            group_full_names: vec!["ZAGROS-OPERATOR".into(), "ZAGROS-VENDOR".into()],
            rosenpass: true,
            handset_key: None,
            handset_key_name: None,
            minted: Vec::new(),
        };
        let md = render_bundle_md(
            "fra-land-bde",
            Some("zagros"),
            Some(&o),
            "https://portal.zagros.netbird.zagros.example",
            native_router_endpoint(Some("zagros")),
            Some(&nb),
            None,
        );
        assert!(md.contains("## NetBird mesh access"));
        assert!(md.contains("K-SECRET-123")); // the setup key
        assert!(md.contains("sudo netbird up --management-url https://netbird.zagros.example --setup-key K-SECRET-123"));
        // Rosenpass on → the join command carries the PQC flags + a note.
        assert!(md.contains("--enable-rosenpass --rosenpass-permissive"));
        assert!(md.contains("post-quantum protection (Rosenpass)"));
        assert!(md.contains("ZAGROS-OPERATOR, ZAGROS-VENDOR")); // resolved group names
        assert!(md.contains("NetBird mesh access + Zenoh wire-layer")); // intro reflects both
        assert!(!md.contains("## Ory login")); // no Ory leg
    }

    #[test]
    fn render_netbird_section_empty_when_absent() {
        assert_eq!(render_netbird_section(None), "");
    }

    // --- the enroll fence ---------------------------------------------

    #[test]
    fn onboard_runner_handle_is_recognised_as_the_console_path() {
        // Serialised with the other env-var tests by construction.
        let prev = std::env::var("GOAT_OPERATOR_HANDLE").ok();
        std::env::set_var("GOAT_OPERATOR_HANDLE", "onboard-runner-zagros-dev");
        assert!(driven_by_onboard_runner(), "the runner must clear the enroll fence");
        std::env::set_var("GOAT_OPERATOR_HANDLE", "person-900@example.org");
        assert!(!driven_by_onboard_runner(), "a person must NOT clear the enroll fence");
        std::env::remove_var("GOAT_OPERATOR_HANDLE");
        assert!(!driven_by_onboard_runner(), "an unset handle must NOT clear the fence");
        match prev {
            Some(v) => std::env::set_var("GOAT_OPERATOR_HANDLE", v),
            None => std::env::remove_var("GOAT_OPERATOR_HANDLE"),
        }
    }

    #[test]
    fn enroll_leg_override_needs_an_explicit_true() {
        // Serialised with the other env-var test below by construction: both
        // set and clear the SAME key, and Rust runs tests in one process with
        // threads, so nothing else may touch this var.
        const K: &str = "GOAT_I_UNDERSTAND_THE_ENROLL_LEG_IS_RETIRED";
        std::env::remove_var(K);
        assert!(!enroll_leg_override(), "absent must not open the fence");

        // The two accepted spellings.
        std::env::set_var(K, "true");
        assert!(enroll_leg_override());
        std::env::set_var(K, "1");
        assert!(enroll_leg_override());

        // Everything else stays closed. `false` and `0` are the ones that
        // matter: an operator who exports the var to remind themselves it
        // exists must not thereby disarm the gate.
        for v in ["false", "0", "", "TRUE", "yes", "please"] {
            std::env::set_var(K, v);
            assert!(
                !enroll_leg_override(),
                "value {v:?} must not open the fence"
            );
        }
        std::env::remove_var(K);
    }

    // --- `goat admin pqc` -----------------------------------------------------

    #[test]
    fn native_netbird_config_carries_rosenpass_flag() {
        // Rosenpass is off on every live goatnet, matching each
        // goatnet's rosenpass_enabled; unknown env → None; no env → None.
        assert!(!native_netbird_config(Some("zagros")).unwrap().rosenpass);
        assert!(!native_netbird_config(Some("zagros-prod")).unwrap().rosenpass);
        assert!(native_netbird_config(None).is_none());
        assert!(native_netbird_config(Some("vega")).is_none());
    }

    #[test]
    fn quantum_line_extracts_report() {
        let status = "Peers count: 1/1 Connected\nQuantum resistance: true (permissive)\nManagement: Connected";
        assert_eq!(
            quantum_line(status).as_deref(),
            Some("Quantum resistance: true (permissive)")
        );
        // Case-insensitive match, trims indentation.
        assert_eq!(
            quantum_line("  quantum resistance: false").as_deref(),
            Some("quantum resistance: false")
        );
        // Absent (older client) → None, so the caller reports honestly.
        assert_eq!(
            quantum_line("Peers count: 0/0\nManagement: Connected"),
            None
        );
    }

    #[test]
    fn pqc_state_helpers() {
        assert!(PqcState::On.is_on());
        assert!(!PqcState::Off.is_on());
        assert_eq!(PqcState::On.label(), "on");
        assert_eq!(PqcState::Off.label(), "off");
    }

    #[test]
    fn rosenpass_flags_off_explicitly_disables() {
        // on = enable + permissive.
        assert_eq!(
            rosenpass_flags(PqcState::On),
            &["--enable-rosenpass", "--rosenpass-permissive"]
        );
        // off MUST carry --enable-rosenpass=false, not be empty — a bare `up`
        // persists the prior setting, so an empty off
        // would be a silent no-op.
        assert_eq!(
            rosenpass_flags(PqcState::Off),
            &["--enable-rosenpass=false"]
        );
        assert!(rosenpass_flags(PqcState::Off)
            .iter()
            .any(|f| f.contains("=false")));
    }

    #[test]
    fn render_netbird_section_rosenpass_off_omits_flags() {
        // rosenpass off (a not-yet-flipped goatnet) → bare join command, no note.
        let nb = NbProvision {
            setup_key: "K-OFF-1".into(),
            mgmt_url: "https://netbird.zagros.example".into(),
            key_name: "peer-x-zagros-2026-07-14".into(),
            expires_days: 30,
            usage_limit: 1,
            group_full_names: vec![],
            rosenpass: false,
            handset_key: None,
            handset_key_name: None,
            minted: Vec::new(),
        };
        let sec = render_netbird_section(Some(&nb));
        assert!(sec.contains("sudo netbird up --management-url https://netbird.zagros.example --setup-key K-OFF-1"));
        assert!(!sec.contains("--enable-rosenpass"));
        assert!(!sec.contains("Rosenpass"));
    }

    // --- P2.4 native Ory leg -------------------------------------------------

    #[test]
    fn native_ory_config_known_and_unknown() {
        let zagros = native_ory_config(Some("zagros")).unwrap();
        assert_eq!(zagros.ssh_host, "192.0.2.10");
        assert_eq!(zagros.kratos_admin_url, "http://127.0.0.1:4434");
        assert!(native_ory_config(Some("vega")).is_none());
        // No --env resolves to no config: an unset env never means a goatnet.
        assert!(native_ory_config(None).is_none());
    }

    #[test]
    fn pct_encode_query_encodes_email_specials() {
        assert_eq!(
            pct_encode_query("jane.smith@example.org"),
            "jane.smith%40example.org"
        );
        assert_eq!(pct_encode_query("a+b@example.org"), "a%2Bb%40example.org");
        // Unreserved set passes through untouched.
        assert_eq!(pct_encode_query("aZ9-._~"), "aZ9-._~");
    }

    #[test]
    fn generate_password_length_and_charset() {
        const ALLOWED: &[u8] =
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#%^*-_=+";
        let pw = generate_password(24);
        assert_eq!(pw.chars().count(), 24);
        assert!(
            pw.bytes().all(|b| ALLOWED.contains(&b)),
            "unexpected char in {pw}"
        );
        // Two draws differ (CSPRNG) — not a false-constant.
        assert_ne!(generate_password(24), generate_password(24));
    }

    #[test]
    fn parse_kratos_local_truthiness() {
        assert!(parse_kratos_local(Some("1".into())));
        assert!(parse_kratos_local(Some("true".into())));
        assert!(parse_kratos_local(Some("TRUE".into())));
        assert!(!parse_kratos_local(Some("0".into())));
        assert!(!parse_kratos_local(Some("no".into())));
        assert!(!parse_kratos_local(Some(String::new())));
        assert!(!parse_kratos_local(None));
    }

    #[test]
    fn ssh_kratos_argv_has_batchmode_and_no_secrets() {
        let cfg = native_ory_config(Some("zagros")).unwrap();
        let args = ssh_kratos_argv(&cfg, "curl -sS http://127.0.0.1:4434/admin/identities");
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "ConnectTimeout=15"));
        assert_eq!(args[args.len() - 2], "goat@192.0.2.10");
        // The remote command is the last word; the body (secrets) is never here.
        assert_eq!(
            args[args.len() - 1],
            "curl -sS http://127.0.0.1:4434/admin/identities"
        );
    }

    #[test]
    fn split_curl_status_splits_body_and_code() {
        let (body, code) = split_curl_status("{\"id\":\"abc\"}\n201").unwrap();
        assert_eq!(body, "{\"id\":\"abc\"}");
        assert_eq!(code, 201);
        // Empty body (e.g. 204).
        let (body, code) = split_curl_status("\n204").unwrap();
        assert_eq!(body, "");
        assert_eq!(code, 204);
        // Pretty-printed body with internal newlines — last newline is the -w sep.
        let (body, code) = split_curl_status("{\n  \"id\": \"x\"\n}\n200").unwrap();
        assert!(body.contains("\"id\""));
        assert_eq!(code, 200);
        assert!(split_curl_status("no-status-line").is_err());
    }

    #[test]
    fn org_trait_short_tag_prefers_affiliation_over_handle() {
        // The bug: on per-uuid goatnets org.short_tag is forced = handle. The org
        // identity trait must instead carry the real org from affiliation.id.
        let org = MfOrg {
            display_name: "USAREUR-AF".into(),
            short_tag: "contributorb".into(), // = handle (the Zenoh prefix)
        };
        let aff = MfAffiliation {
            sector: String::new(),
            id: "usareur-af".into(),
            kind: "military".into(),
            nation: Some("USA".into()),
        };
        assert_eq!(org_trait_short_tag(&org, &aff), "usareur-af");

        // No affiliation → fall back to the manifest org.short_tag (pre-affiliation
        // manifests keep working).
        let no_aff = MfAffiliation::default();
        assert_eq!(org_trait_short_tag(&org, &no_aff), "contributorb");
    }

    #[test]
    fn build_identity_body_shape_groups_and_utf8() {
        let org = MfOrg {
            display_name: "Acme Corp".into(),
            short_tag: "acme".into(),
        };
        let groups = vec!["ZAGROS-OPERATOR".to_string(), "ZAGROS-OBSERVER".to_string()];
        // Non-ASCII name must cross the wire as literal UTF-8, NOT \uXXXX
        // (the SQLSTATE-22P05 gotcha the Ansible path hand-serialized around).
        let raw = build_identity_body(
            "person-52@example.org",
            "Jánoš Novák",
            &groups,
            &org,
            None,
            Some("pw-123"),
        );
        assert!(
            raw.contains("Jánoš Novák"),
            "name must be literal UTF-8: {raw}"
        );
        assert!(
            !raw.contains("\\u00"),
            "must not \\u-escape non-ASCII: {raw}"
        );
        // Parse back and assert the structural shape.
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["schema_id"], "default");
        assert_eq!(v["state"], "active");
        assert_eq!(v["traits"]["email"], "person-52@example.org");
        // Authorization fields land in admin-managed metadata_public, not traits
        //.
        assert_eq!(v["metadata_public"]["groups"][0], "ZAGROS-OPERATOR");
        assert_eq!(v["metadata_public"]["org"]["short_tag"], "acme");
        assert_eq!(v["metadata_public"]["org"]["display_name"], "Acme Corp");
        assert!(v["traits"].get("groups").is_none(), "groups must not be in traits");
        assert_eq!(v["credentials"]["password"]["config"]["password"], "pw-123");
        assert_eq!(v["verifiable_addresses"][0]["value"], "person-52@example.org");
        assert_eq!(v["recovery_addresses"][0]["value"], "person-52@example.org");
        // `aff` is omitted when the caller passes None (Phase D gate off).
        assert!(v["metadata_public"].get("aff").is_none());
    }

    #[test]
    fn build_identity_body_omits_password_and_org_when_absent() {
        let org = MfOrg::default(); // no display_name / short_tag
        let raw = build_identity_body("x@example.net", "X", &[], &org, None, None);
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(
            v.get("credentials").is_none(),
            "no password → no credentials block"
        );
        assert!(
            v["metadata_public"].get("org").is_none(),
            "no org data → no org block"
        );
        assert_eq!(v["metadata_public"]["groups"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn build_identity_body_emits_aff_when_enabled() {
        let org = MfOrg::default();
        let aff = MfAffiliation {
            sector: String::new(),
            id: "demo-org".into(),
            kind: "partner".into(),
            nation: Some("POL".into()),
        };
        // Gate on (Some) → aff trait present with id/kind/nation.
        let raw = build_identity_body("person-138@example.org", "X", &[], &org, Some(&aff), None);
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["metadata_public"]["aff"]["id"], "demo-org");
        assert_eq!(v["metadata_public"]["aff"]["kind"], "partner");
        assert_eq!(v["metadata_public"]["aff"]["nation"], "POL");
        // Gate on but empty kind → still omitted (no half-formed claim).
        let empty = MfAffiliation::default();
        let raw2 = build_identity_body("person-138@example.org", "X", &[], &org, Some(&empty), None);
        let v2: serde_json::Value = serde_json::from_str(&raw2).unwrap();
        assert!(v2["metadata_public"].get("aff").is_none());
    }

    #[test]
    fn render_ory_section_variants() {
        assert_eq!(render_ory_section(None), "");
        let pw = OryProvision {
            auth_url: "https://auth.zagros.example".into(),
            email: "person-55@example.org".into(),
            password: Some("secretpw".into()),
            recovery_link: None,
            already_existed: false,
            identity_id: None,
        };
        let md = render_ory_section(Some(&pw));
        assert!(md.contains("## Ory login"));
        assert!(md.contains("https://auth.zagros.example"));
        assert!(md.contains("person-55@example.org"));
        assert!(md.contains("secretpw"));
        assert!(md.contains("Change the temporary password"));

        let link = OryProvision {
            password: None,
            recovery_link: Some("https://auth.zagros.example/recovery?token=abc".into()),
            already_existed: false,
            identity_id: None,
            ..pw
        };
        let md = render_ory_section(Some(&link));
        assert!(md.contains("Passkey-enrolment link"));
        assert!(md.contains("token=abc"));
        assert!(md.contains("within 15 minutes"));

        let existed = OryProvision {
            auth_url: "https://auth.zagros.example".into(),
            email: "person-55@example.org".into(),
            password: None,
            recovery_link: None,
            already_existed: true,
            identity_id: None,
        };
        let md = render_ory_section(Some(&existed));
        assert!(md.contains("already existed"));
    }

    #[test]
    fn render_bundle_md_includes_ory_section_when_provisioned() {
        let o = sample_onboarded();
        let ory = OryProvision {
            auth_url: "https://auth.zagros.netbird.zagros.example".into(),
            email: "person-43@example.org".into(),
            password: Some("temp-pw-xyz".into()),
            recovery_link: None,
            already_existed: false,
            identity_id: None,
        };
        let md = render_bundle_md(
            "fra-land-bde",
            Some("zagros"),
            Some(&o),
            "https://portal.zagros.netbird.zagros.example",
            native_router_endpoint(Some("zagros")),
            None,
            Some(&ory),
        );
        assert!(md.contains("## Ory login"));
        assert!(md.contains("temp-pw-xyz"));
        assert!(md.contains("Ory OIDC login + Zenoh wire-layer")); // intro reflects Ory
    }

    #[test]
    fn render_bundle_md_browse_tier_has_no_zenoh_section() {
        // A browse / web-only tier (o=None): NetBird + Ory sections render, the
        // Zenoh block is absent, and the intro names only the provisioned surfaces.
        let nb = NbProvision {
            setup_key: "K-BROWSE-9".into(),
            mgmt_url: "https://netbird.zagros.example".into(),
            key_name: "peer-browse-user-zagros-2026-07-02".into(),
            expires_days: 30,
            usage_limit: 1,
            group_full_names: vec!["ZAGROS-OBSERVER".into()],
            rosenpass: true,
            handset_key: None,
            handset_key_name: None,
            minted: Vec::new(),
        };
        let ory = OryProvision {
            auth_url: "https://auth.zagros.netbird.zagros.example".into(),
            email: "person-25@example.org".into(),
            password: Some("pw-browse".into()),
            recovery_link: None,
            already_existed: false,
            identity_id: None,
        };
        let md = render_bundle_md(
            "browse-user",
            Some("zagros"),
            None,
            "https://portal.zagros.netbird.zagros.example",
            native_router_endpoint(Some("zagros")),
            Some(&nb),
            Some(&ory),
        );
        assert!(md.contains("## NetBird mesh access"));
        assert!(md.contains("K-BROWSE-9"));
        assert!(md.contains("## Ory login"));
        assert!(md.contains("pw-browse"));
        // No Zenoh surface for a browse tier.
        assert!(!md.contains("## Zenoh trial fabric"));
        assert!(!md.contains("Portal API key"));
        // Intro names only NetBird + Ory (no Zenoh wire-layer).
        assert!(md.contains("the **NetBird mesh access + Ory OIDC login**."));
        assert!(!md.contains("Zenoh wire-layer"));
    }

    // --- 0.4.13: verbatim Ory groups + enum validation + native-default dispatch ---

    #[test]
    fn build_identity_body_groups_are_verbatim_not_prefixed() {
        // The service-permission groups go into the claim exactly as authored —
        // no site prefix (that axis is netbird.auto_groups). Regression guard
        // for the 0.4.13 fix.
        let raw = build_identity_body(
            "x@example.net",
            "X",
            &["PLATFORM-ADMIN".into(), "IAM-ADMIN".into()],
            &MfOrg::default(),
            None,
            None,
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["metadata_public"]["groups"][0], "PLATFORM-ADMIN");
        assert_eq!(v["metadata_public"]["groups"][1], "IAM-ADMIN");
        assert!(
            !raw.contains("ZAGROS-"),
            "groups must not be site-prefixed: {raw}"
        );
    }

    #[test]
    fn valid_ory_groups_reads_schema_enum() {
        use tempfile::TempDir;
        let td = TempDir::new().unwrap();
        let schema_dir = td.path().join("ops/operators/schema");
        std::fs::create_dir_all(&schema_dir).unwrap();
        std::fs::write(
            schema_dir.join("operator-manifest.schema.json"),
            r#"{"properties":{"ory_site":{"properties":{"groups":{"items":{"enum":["ADMINS","IAM-ADMIN","OBS-ADMIN"]}}}}}}"#,
        )
        .unwrap();
        let g = valid_ory_groups(td.path()).unwrap();
        assert_eq!(g.len(), 3);
        assert!(g.contains(&"IAM-ADMIN".to_string()));
        assert!(g.contains(&"ADMINS".to_string()));
        // missing schema → error (not a silent empty list)
        assert!(valid_ory_groups(&td.path().join("nope")).is_err());
    }

    #[test]
    fn native_eligibility_gates_by_manifest_shape() {
        use tempfile::TempDir;
        let td = TempDir::new().unwrap();
        let dir = td.path().join("ops/operators/zagros");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            std::fs::write(dir.join(format!("{name}.yml")), body).unwrap();
        };
        // Native-serviceable: a trial publisher, no operator-IdP.
        write(
            "ok",
            "identity: {handle: ok}\nzenoh: {enabled: true, trial: {org: acme}}\nory_idp: {enabled: false}\n",
        );
        assert_eq!(
            native_eligibility(Some(td.path()), "ok", Some("zagros")).unwrap(),
            None
        );
        // zenoh disabled → Ansible.
        write("nz", "identity: {handle: nz}\nzenoh: {enabled: false}\n");
        assert!(native_eligibility(Some(td.path()), "nz", Some("zagros"))
            .unwrap()
            .unwrap()
            .contains("zenoh.enabled=false"));
        // operator-IdP → Ansible (native doesn't provision it).
        write(
            "idp",
            "identity: {handle: idp}\nzenoh: {enabled: true, trial: {org: acme}}\nory_idp: {enabled: true}\n",
        );
        assert!(native_eligibility(Some(td.path()), "idp", Some("zagros"))
            .unwrap()
            .unwrap()
            .contains("ory_idp"));
        // no trial prefix → Ansible.
        write(
            "notrial",
            "identity: {handle: notrial}\nzenoh: {enabled: true}\n",
        );
        assert!(native_eligibility(Some(td.path()), "notrial", Some("zagros"))
            .unwrap()
            .unwrap()
            .contains("no zenoh.trial.org"));
        // missing manifest → Err (caller falls back to Ansible).
        assert!(native_eligibility(Some(td.path()), "ghost", Some("zagros")).is_err());
    }

    /// An env absent from the tables must be recognizable as unsupported by
    /// EVERY leg — never partially known, which is what let a mint half-run and
    /// still report success.
    #[test]
    fn unknown_env_is_unsupported_by_every_native_leg() {
        let ghost = Some("no-such-goatnet");
        assert!(native_netbird_config(ghost).is_none());
        assert!(native_ory_config(ghost).is_none());
        assert!(native_router_endpoint(ghost).is_none());
    }

    // The 7-day cut is only a control if widening it costs something. Manifests
    // are gitignored, so no CI job can ever lint one — the mint is the only
    // place that reads the file and therefore the only place this can be
    // enforced. These pin that it is.
    #[test]
    fn ttl_override_requires_a_stated_reason() {
        let bare = MfNetbird {
            role: "peer".into(),
            ttl_days: Some(90),
            ..Default::default()
        };
        let err = check_ttl_override(&bare).unwrap_err().to_string();
        assert!(err.contains("ttl_reason"), "the refusal must name the field: {err}");
        assert!(err.contains("90"), "the refusal must quote what was asked: {err}");

        // A reason passes.
        let justified = MfNetbird {
            role: "peer".into(),
            ttl_days: Some(90),
            ttl_reason: Some("issued ahead of the Rota movement; enrolled on arrival".into()),
            ..Default::default()
        };
        assert!(check_ttl_override(&justified).is_ok());

        // A token non-answer does not. The bar is deliberately low but nonzero:
        // "x" or "n/a" defeats the purpose while looking like compliance.
        let hollow = MfNetbird {
            role: "peer".into(),
            ttl_days: Some(90),
            ttl_reason: Some("n/a".into()),
            ..Default::default()
        };
        assert!(check_ttl_override(&hollow).is_err(), "a non-answer must not pass");

        // At or under the default needs no justification — TIGHTENING a
        // credential is never something anyone should have to argue for.
        for days in [1_u64, 7] {
            let tight = MfNetbird {
                role: "peer".into(),
                ttl_days: Some(days),
                ..Default::default()
            };
            assert!(check_ttl_override(&tight).is_ok(), "{days}d must pass unjustified");
        }

        // An omitted ttl_days takes the default and is never questioned.
        let silent = MfNetbird { role: "peer".into(), ..Default::default() };
        assert!(check_ttl_override(&silent).is_ok());
    }

    // #1587: a failed run revokes exactly the keys it minted — never one it did
    // not mint, never an id-less one it cannot address, never one twice.
    #[test]
    fn keys_to_revoke_after_failure_takes_only_addressable_minted_keys() {
        use panoscope_core::netbird::SetupKeyMeta;
        let k = |id: &str, name: &str, revoked: bool| SetupKeyMeta {
            id: id.into(),
            name: name.into(),
            auto_groups: vec!["g1".into()],
            revoked,
        };
        let minted = vec![
            k("dev1", "peer-x-zagros-2026-10-05", false),
            k("hs1", "peer-x-zagros-2026-10-05-handset", false),
            k("", "peer-x-noid", false),
            k("dev1", "peer-x-zagros-2026-10-05", false),
            k("old", "peer-x-already-revoked", true),
        ];
        let ids: Vec<&str> = keys_to_revoke_after_failure(&minted)
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, vec!["dev1", "hs1"]);
        assert!(keys_to_revoke_after_failure(&[]).is_empty());
    }

    #[test]
    fn minted_key_meta_carries_id_groups_and_falls_back_to_the_requested_name() {
        let echoed = panoscope_core::netbird::SetupKeyResult {
            id: "abc".into(),
            key: "SECRET".into(),
            name: String::new(),
            state: "valid".into(),
        };
        let m = minted_key_meta(&echoed, "peer-x-zagros-2026-10-05", &["g1".into(), "g2".into()]);
        assert_eq!(m.id, "abc");
        assert_eq!(m.name, "peer-x-zagros-2026-10-05");
        assert_eq!(m.auto_groups, vec!["g1".to_string(), "g2".to_string()]);
        assert!(!m.revoked);
        let named = panoscope_core::netbird::SetupKeyResult { name: "from-netbird".into(), ..echoed };
        assert_eq!(minted_key_meta(&named, "requested", &[]).name, "from-netbird");
    }

    #[test]
    fn role_ttl_days_matches_playbook_defaults() {
        assert_eq!(role_ttl_days("bootstrap"), 1);
        assert_eq!(role_ttl_days("admin"), 7);
        assert_eq!(role_ttl_days("operator"), 30);
        assert_eq!(role_ttl_days("peer"), 7);
        assert_eq!(role_ttl_days(""), 7); // unknown/blank → conservative peer default
    }

    // The same number lives in three languages with no shared source: here, the
    // console's onboard-intake-form.json `defaults` block, and
    // `netbird_setup_key_usage_limit_default` in operator-onboard.yml. This
    // pins the Rust copy and says where the other two are.
    //
    // The value that matters is "not 1". A manifest omitting usage_limit used to
    // fall through to a single-use key, consumed by the first successful
    // enrolment, with every attempt after it refused as "setup key is invalid" —
    // and nothing in any surface showed the limit.
    #[test]
    fn default_setup_key_usage_limit_matches_the_other_two_homes() {
        // The "never 1" property is fenced at compile time beside the constant.
        // This pins the exact value, which is the half that drifts.
        assert_eq!(DEFAULT_SETUP_KEY_USAGE_LIMIT, 5);
    }

    #[test]
    fn native_netbird_config_known_and_unknown() {
        assert!(native_netbird_config(None).is_none());
        let zagros = native_netbird_config(Some("zagros")).unwrap();
        assert_eq!(zagros.site_prefix, "ZAGROS");
        assert_eq!(zagros.token_env_var, "ZAGROS_NB_API_TOKEN");
        assert!(zagros.mgmt_url.contains("zagros.example"));
        assert!(native_netbird_config(Some("vega")).is_none());
    }

    #[test]
    fn manifest_projects_netbird_block() {
        let yaml = r#"
identity: { name: Jane, handle: jane }
zenoh: { enabled: true, trial: { org: jane-pub } }
netbird:
  enabled: true
  role: operator
  usage_limit: 5
  auto_groups: [OPERATOR, VENDOR]
"#;
        let m: OnboardManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(m.netbird.enabled);
        assert_eq!(m.netbird.role, "operator");
        assert_eq!(m.netbird.usage_limit, Some(5));
        assert_eq!(m.netbird.auto_groups, vec!["OPERATOR", "VENDOR"]);
        assert_eq!(m.netbird.ttl_days, None); // falls back to role_ttl_days(operator)=30
    }

    #[test]
    fn native_router_endpoint_known_and_unknown() {
        assert_eq!(
            native_router_endpoint(None),
            None
        );
        assert!(native_router_endpoint(Some("zagros"))
            .unwrap()
            .contains("zagros.example"));
        assert_eq!(native_router_endpoint(Some("vega")), None);
    }

    #[test]
    fn full_bundle_url_joins_portal_origin_with_relative_path() {
        let rel = "/api/org-bundle/32fd719f/b6aa9799.tar.gz";
        assert_eq!(
            full_bundle_url(Some("https://portal.zagros.netbird.zagros.example"), rel),
            "https://portal.zagros.netbird.zagros.example/api/org-bundle/32fd719f/b6aa9799.tar.gz"
        );
    }

    #[test]
    fn full_bundle_url_tolerates_trailing_slash_on_base() {
        assert_eq!(
            full_bundle_url(Some("https://portal.x/"), "/api/org-bundle/a/t.tar.gz"),
            "https://portal.x/api/org-bundle/a/t.tar.gz"
        );
    }

    #[test]
    fn full_bundle_url_passes_absolute_url_through() {
        let abs = "https://portal.x/api/org-bundle/a/t.tar.gz";
        assert_eq!(full_bundle_url(Some("https://portal.y"), abs), abs);
    }

    #[test]
    fn full_bundle_url_falls_back_to_relative_when_no_base() {
        assert_eq!(
            full_bundle_url(None, "/api/org-bundle/a/t.tar.gz"),
            "/api/org-bundle/a/t.tar.gz"
        );
    }

    // goat#1193 — `rotate --key-only` reissues ONLY the portal key. rekey_one
    // POSTs to /{anchor}/rekey and returns the fresh key for the caller to
    // print; no bundle is involved.
    #[tokio::test]
    async fn rekey_one_returns_fresh_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/orgs/deadbeef/rekey"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "prefix": "deadbeef",
                "api_key": "vp_deadbeef_fresh",
                "note_to_share": "Portal key reissued in place."
            })))
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let key = rekey_one(&portal, "deadbeef", "").await.unwrap();
        assert_eq!(key, "vp_deadbeef_fresh");
    }

    // A 404 means the peer doesn't exist — rekey reissues, it never creates.
    // The error must point the operator at onboard rather than leak a raw 404,
    // AND name the version-skew case: a 404 also comes back when the portal
    // image predates the rekey route, so the message must not tell the operator
    // to onboard (and burn a slot) for a peer that is actually live.
    #[tokio::test]
    async fn rekey_one_unknown_peer_is_helpful_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/orgs/ghost/rekey"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let err = rekey_one(&portal, "ghost", "").await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("onboard"),
            "unknown-peer error should point at onboard, was: {msg}"
        );
        assert!(
            msg.contains("predates rekey") && msg.contains("upgrade the portal image"),
            "404 error must name the version-skew case (old portal image), was: {msg}"
        );
    }
}

/// `goat admin operator-key …` — request, decide, collect, reconcile.
///
/// The verbs mirror the console pages exactly, over the same machine API, so an
/// operator who learns one has learned the other. The wording differs from
/// `goat admin ssh-key` in one important place and it is not an oversight:
/// approving an SSH key still needs an installer run against hosts, while
/// approving an operator key IS the grant.
#[derive(Subcommand, Debug)]
pub enum AdminOperatorKeyCmd {
    /// Record a request. Grants nothing — a DIFFERENT admin still approves.
    ///
    /// With no handle the request is for YOU: the console binds it to the person
    /// this profile's token belongs to, so a first-time admin needs no operator
    /// key to ask for one (#1373). Only a platform admin holds an operator key
    /// (ADR 1182), and a self request needs --reason naming the admin task. With
    /// a handle it is filed on that person's behalf, which takes your operator
    /// key (--context <env>-op).
    Request {
        /// The person's handle, as the onboarding roster knows them. Omit it to
        /// file for yourself.
        handle: Option<String>,
        /// Their email, so the queue shows who the request is for.
        #[arg(long, default_value = "")]
        email: String,
        /// Target goatnet. Defaults to the console's own.
        #[arg(long, default_value = "")]
        env: String,
        /// What they need it for. The admin decides on the person and the work.
        #[arg(long, default_value = "")]
        reason: String,
        #[arg(long)]
        json: bool,
    },
    /// List requests: requested (default) | active | revoked | rejected | all.
    List {
        #[arg(long, default_value = "requested")]
        status: String,
        #[arg(long)]
        json: bool,
    },
    /// Approve a request — mints the key AND writes the portal's key file.
    Approve {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Decline a request without minting anything.
    Reject {
        id: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        json: bool,
    },
    /// Revoke a live key — removes the file first, then the record.
    Revoke {
        id: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        json: bool,
    },
    /// Collect the key value. Works EXACTLY ONCE, for its owner.
    Collect {
        id: String,
        /// The handle the key belongs to; defaults to the person this profile's
        /// token belongs to. Collection is bound to the owner either way.
        #[arg(long)]
        handle: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compare the record against the directory the portal actually reads.
    /// Dry by default.
    Reconcile {
        #[arg(long, default_value = "")]
        env: String,
        /// Remove grants no record explains. Never invents a MISSING key: the
        /// secret is deliberately not retained, and manufacturing one would
        /// hand somebody a credential nobody approved.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
}

async fn run_operator_key(
    portal: &panoscope_core::portal::PortalClient,
    op: AdminOperatorKeyCmd,
) -> anyhow::Result<()> {
    match op {
        AdminOperatorKeyCmd::Request {
            handle,
            email,
            env,
            reason,
            json,
        } => {
            // No handle on the wire means "bind it to me": the console resolves
            // the person from the token, never from anything typed here.
            let for_self = handle.is_none();
            if for_self && reason.trim().is_empty() {
                // The console refuses a self request with no reason
                // (missing_reason); say so before spending a round trip.
                anyhow::bail!(
                    "--reason is required when filing for yourself: name the admin task that \
                     needs the key (a change to another organisation, another person's slot, \
                     or someone else's machines). Reading the portal or your own slot is \
                     covered by your slot key, and is not a reason."
                );
            }
            let handle = handle.unwrap_or_default();
            let row = portal
                .request_operator_key(&handle, &email, &env, &reason)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": row.id, "handle": row.handle, "status": row.status, "env": row.env,
                        "for_self": for_self,
                    }))?
                );
            } else if for_self {
                println!(
                    "{} — request {} recorded for you ({}). It grants nothing until a different \
                     admin approves it; once they do, collect it once with `goat admin \
                     operator-key collect {}` on this same profile.",
                    row.status, row.id, row.handle, row.id
                );
            } else {
                println!(
                    "{} — request {} recorded for {}. It grants nothing until an admin approves it.",
                    row.status, row.id, row.handle
                );
            }
            Ok(())
        }
        AdminOperatorKeyCmd::List { status, json } => {
            let rows = portal
                .list_operator_keys(&status)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                let out: Vec<_> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.id, "handle": r.handle, "env": r.env, "status": r.status,
                            "fingerprint": r.fingerprint, "decided_by": r.decided_by,
                            "reason": r.reason, "email": r.email,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if rows.is_empty() {
                println!("nothing in state {status}");
            } else {
                for r in &rows {
                    let fp = if r.fingerprint.is_empty() {
                        "—"
                    } else {
                        &r.fingerprint
                    };
                    println!(
                        "{:<14}  {:<10}  {:<10}  {:<26}  {}",
                        r.handle, r.env, r.status, fp, r.id
                    );
                }
            }
            Ok(())
        }
        AdminOperatorKeyCmd::Approve { id, json } => {
            let row = portal
                .decide_operator_key(&id, "approve", "")
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": row.id, "handle": row.handle, "status": row.status,
                        "fingerprint": row.fingerprint,
                    }))?
                );
            } else {
                // Say that this IS the grant. Borrowing the SSH flow's "not
                // installed yet" would leave people waiting for a step that
                // does not exist here.
                println!(
                    "approved {} ({}). The key file is written; the portal accepts it within one \
                     reload interval — there is no install step. {} collects it once with \
                     `goat admin operator-key collect {} --handle {}`.",
                    row.handle, row.fingerprint, row.handle, row.id, row.handle
                );
            }
            Ok(())
        }
        AdminOperatorKeyCmd::Reject { id, note, json } => {
            let row = portal
                .decide_operator_key(&id, "reject", &note)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": row.id, "handle": row.handle, "status": row.status,
                    }))?
                );
            } else {
                println!("rejected {} — nothing was minted.", row.handle);
            }
            Ok(())
        }
        AdminOperatorKeyCmd::Revoke { id, note, json } => {
            let row = portal
                .decide_operator_key(&id, "revoke", &note)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": row.id, "handle": row.handle, "status": row.status,
                    }))?
                );
            } else {
                println!(
                    "revoked {} — the key file is gone; the portal stops accepting it on its \
                     next reload.",
                    row.handle
                );
            }
            Ok(())
        }
        AdminOperatorKeyCmd::Collect { id, handle, json } => {
            // An empty handle on the wire binds the collection to the person
            // this token belongs to (the console refuses any other handle).
            let handle = handle.unwrap_or_default();
            let out = portal
                .collect_operator_key(&id, &handle)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "handle": out.handle, "key": out.key,
                    }))?
                );
            } else {
                // The value goes to stdout ALONE so it can be piped or captured
                // without the surrounding prose; the warning goes to stderr so
                // a person sees it and a variable does not swallow it.
                eprintln!(
                    "This is shown once and cannot be shown again. If it is lost, revoke and \
                     request a new one — a key nobody holds still works, and nobody can tell \
                     you who is using it."
                );
                println!("{}", out.key);
            }
            Ok(())
        }
        AdminOperatorKeyCmd::Reconcile { env, apply, json } => {
            let out = portal
                .reconcile_operator_keys(&env, apply)
                .await
                .map_err(|e| super::vendor::map_operator_key_err(portal, e))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "dir": out.dir, "applied": out.applied,
                        "revoked_but_still_granted": out.revoked_but_still_granted,
                        "granted_but_not_ours": out.granted_but_not_ours,
                        "recorded_but_not_granted": out.recorded_but_not_granted,
                    }))?
                );
                return Ok(());
            }
            if out.revoked_but_still_granted.is_empty()
                && out.recorded_but_not_granted.is_empty()
                && out.granted_but_not_ours.is_empty()
            {
                println!("the record and {} agree", out.dir);
                return Ok(());
            }
            if !out.revoked_but_still_granted.is_empty() {
                println!(
                    "REVOKED HERE, STILL GRANTED: {} — the record says this access was taken \
                     away and it has not been{}",
                    out.revoked_but_still_granted.join(", "),
                    if out.applied {
                        " (removed)"
                    } else {
                        " (run with --apply to remove)"
                    }
                );
            }
            if !out.granted_but_not_ours.is_empty() {
                // Printed calmly and last-but-one on purpose. These are normal:
                // service keys written by their own deploy, and keys issued
                // before this queue existed. An earlier version called them
                // unexplained and offered to delete them, which would have cut
                // the onboard-runner off from the portal.
                println!(
                    "not issued here (left alone): {} — grants with no request behind them, \
                     typically a service's own deploy",
                    out.granted_but_not_ours.join(", ")
                );
            }
            if !out.recorded_but_not_granted.is_empty() {
                println!(
                    "RECORDED BUT NOT GRANTED: {} — these people hold a key the portal refuses. \
                     Revoke and re-issue; the secret cannot be recovered.",
                    out.recorded_but_not_granted.join(", ")
                );
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod ssh_key_pubkey_vetting {
    use super::vet_pubkey_before_send;

    // The one input that must never leave the machine.
    #[test]
    fn refuses_a_private_key_by_name() {
        let err = vet_pubkey_before_send(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
        )
        .unwrap_err();
        assert!(err.contains("PRIVATE"), "must name the problem: {err}");
        assert!(
            err.contains("Nothing was sent"),
            "must say nothing left: {err}"
        );
    }

    // Private keys are multi-line; a pubkey never is. This catches a private key
    // whose header was stripped, which the PRIVATE-KEY string check would miss.
    #[test]
    fn refuses_multi_line_input() {
        assert!(vet_pubkey_before_send("ssh-ed25519 AAAA\nmore").is_err());
    }

    #[test]
    fn refuses_empty_and_untyped() {
        assert!(vet_pubkey_before_send("   ").is_err());
        assert!(vet_pubkey_before_send("AAAAC3NzaC1lZDI1NTE5 nope").is_err());
    }

    #[test]
    fn accepts_a_well_formed_public_key() {
        assert!(
            vet_pubkey_before_send("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINELAyKa6q1WM0G+f4hHPlfGzP0YEwvoTMpkMu//phb7 person@laptop").is_ok() // opsec-allow: synthetic throwaway test key; oss-allow: same
        );
        // trailing newline from `cat *.pub` is normal, not an error
        assert!(vet_pubkey_before_send("ssh-rsa AAAAB3Nza x@y\n").is_ok());
    }
}

#[cfg(test)]
mod handset_key_tests {
    #[test]
    fn handset_group_suffixes_adds_handset_only_when_the_group_exists() {
        use panoscope_core::netbird::NbGroup;
        let g = |name: &str, id: &str| NbGroup {
            id: id.to_string(),
            name: name.to_string(),
            ..Default::default()
        };
        let without = vec![g("ZAGROS-OBSERVER", "g1"), g("ZAGROS-DEVELOPER", "g2")];
        assert_eq!(super::handset_group_suffixes(&without, "ZAGROS"), vec!["OBSERVER"]);
        let with = vec![g("ZAGROS-OBSERVER", "g1"), g("ZAGROS-HANDSET", "g3")];
        assert_eq!(
            super::handset_group_suffixes(&with, "ZAGROS"),
            vec!["OBSERVER", "HANDSET"]
        );
        // Another goatnet's HANDSET group does not count, nor does an id-less row.
        let foreign = vec![g("ZAGROS-PROD-HANDSET", "g9"), g("ZAGROS-HANDSET", "")];
        assert_eq!(super::handset_group_suffixes(&foreign, "ZAGROS"), vec!["OBSERVER"]);
    }

    use super::*;

    // ---- handset key (ADR 1175) ----

    #[test]
    fn manifest_handset_limit_deserializes_and_defaults() {
        let with: OnboardManifest = serde_yaml::from_str(
            "netbird:\n  enabled: true\n  usage_limit: 3\n  handset_limit: 2\n",
        )
        .unwrap();
        assert_eq!(with.netbird.handset_limit, Some(2));
        assert_eq!(effective_handset_limit(&with.netbird), 2);

        let without: OnboardManifest =
            serde_yaml::from_str("netbird:\n  enabled: true\n  usage_limit: 3\n").unwrap();
        assert_eq!(without.netbird.handset_limit, None);
        assert_eq!(
            effective_handset_limit(&without.netbird),
            DEFAULT_HANDSET_LIMIT
        );

        let zero: OnboardManifest =
            serde_yaml::from_str("netbird:\n  enabled: true\n  handset_limit: 0\n").unwrap();
        assert_eq!(
            effective_handset_limit(&zero.netbird),
            0,
            "0 is the explicit no-key value"
        );

        let high: OnboardManifest =
            serde_yaml::from_str("netbird:\n  enabled: true\n  handset_limit: 9\n").unwrap();
        assert_eq!(effective_handset_limit(&high.netbird), MAX_HANDSET_LIMIT);
    }

    #[test]
    fn handset_key_line_is_the_scraped_format() {
        // The onboard-runner scrapes this line by regex; the shape is a contract.
        assert_eq!(
            handset_key_line("HK-1", "peer-x-zagros-2026-09-10-handset"),
            "  handset key:  HK-1  (key `peer-x-zagros-2026-09-10-handset`)"
        );
        assert_eq!(
            handset_key_name("peer-x-zagros-2026-09-10"),
            "peer-x-zagros-2026-09-10-handset"
        );
        assert_eq!(setup_key_type(1), "one-off");
        assert_eq!(setup_key_type(2), "reusable");
    }

    #[test]
    fn handset_key_name_belongs_to_its_person_for_the_revoke_sweep() {
        // R1.5 sweeps keys by `belongs_to`; the `-handset` suffix must not escape it.
        let device = peername::mint("ana-lopez", "acme", "zagros", "2026-09-10");
        let handset = handset_key_name(&device);
        assert!(revoke_peer_matches(&handset, "ana-lopez", "acme", "zagros"));
        // untagged (tag == handle is omitted by mint)
        let device2 = peername::mint("bo", "bo", "zagros", "2026-09-10");
        assert!(revoke_peer_matches(
            &handset_key_name(&device2),
            "bo",
            "bo",
            "zagros"
        ));
        // never another person's, never another goatnet's
        assert!(!revoke_peer_matches(&handset, "ana", "acme", "zagros"));
        assert!(!revoke_peer_matches(
            &handset,
            "ana-lopez",
            "acme",
            "zagros-prod"
        ));
    }

    #[test]
    fn handset_keys_to_revoke_selects_only_live_handset_keys_of_that_person() {
        use panoscope_core::netbird::SetupKeyMeta;
        let mk = |name: &str, revoked: bool| SetupKeyMeta {
            id: name.to_string(),
            name: name.to_string(),
            auto_groups: vec![],
            revoked,
        };
        let keys = vec![
            mk("peer-ana-lopez-acme-zagros-2026-09-01-handset", false), // old handset: yes
            mk("peer-ana-lopez-acme-zagros-2026-09-05-handset", true),  // already revoked: no
            mk("peer-ana-lopez-acme-zagros-2026-09-01", false),         // device key: no
            mk("peer-ana-acme-zagros-2026-09-01-handset", false),       // other person: no
        ];
        // (Keys from another goatnet never appear here: each management server
        // lists only its own keys, so `env` narrows the name, not the source.)
        let picked: Vec<&str> = handset_keys_to_revoke(&keys, "ana-lopez", "acme", "zagros")
            .into_iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(picked, vec!["peer-ana-lopez-acme-zagros-2026-09-01-handset"]);
    }

    #[test]
    fn device_keys_of_excludes_handset_and_other_people_newest_last() {
        use panoscope_core::netbird::SetupKeyMeta;
        let mk = |name: &str, revoked: bool| SetupKeyMeta {
            id: name.to_string(),
            name: name.to_string(),
            auto_groups: vec![],
            revoked,
        };
        let keys = vec![
            mk("peer-user239x-zagros-2026-09-09", false),
            mk("peer-user239x-zagros-2026-08-01", true),
            mk("peer-user239x-zagros-2026-09-09-handset", false), // phone: no
            mk("peer-sandeep-zagros-2026-09-20", false),                  // other person: no
        ];
        // (No other-goatnet case: each management server lists only its own
        // keys, so `env` narrows the name, not the source.)
        let picked: Vec<&str> = device_keys_of(&keys, "user239x", "", "zagros")
            .into_iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(
            picked,
            vec!["peer-user239x-zagros-2026-08-01", "peer-user239x-zagros-2026-09-09"],
            "oldest first, so .last() is the grant the person holds now"
        );
        // The handle "sandeep" never picks up "user239x".
        let other: Vec<&str> = device_keys_of(&keys, "sandeep", "", "zagros")
            .into_iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(other, vec!["peer-sandeep-zagros-2026-09-20"]);
    }

    #[test]
    fn legacy_device_key_selection_keeps_the_newest_grants() {
        use panoscope_core::netbird::SetupKeyMeta;
        let mk = |name: &str, group: &str| SetupKeyMeta {
            id: name.to_string(),
            name: name.to_string(),
            auto_groups: vec![group.to_string()],
            revoked: false,
        };
        for (newest, oldest) in [
            (
                "peer-ana-lopez-zagros-2026-09-20-rosenpass-reissue",
                "peer-ana-lopez-zagros-2026-09-01-rosenpass-reissue",
            ),
            (
                "peer-ana-lopez-zagros-2026-09-20-rosenpass-reissue",
                "peer-ana-lopez-zagros-2026-09-01",
            ),
            (
                "peer-ana-lopez-zagros-2026-09-20",
                "peer-ana-lopez-zagros-2026-09-01-rosenpass-reissue",
            ),
        ] {
            let keys = vec![
                mk(newest, "current-grant"),
                mk(oldest, "old-grant"),
                mk("peer-ana-lopez-zagros-2026-09-21-handset", "phone-grant"),
                mk("peer-ana-zagros-2026-09-21-rosenpass-reissue", "foreign-grant"),
            ];
            let prior = device_keys_of(&keys, "ana-lopez", "", "zagros");
            assert_eq!(prior.len(), 2);
            let last = prior.last().unwrap();
            assert_eq!(last.name, newest);
            assert_eq!(last.auto_groups, vec!["current-grant"]);
            assert_eq!(
                redate_key_name(&last.name, "2026-09-22").as_deref(),
                Some("peer-ana-lopez-zagros-2026-09-22")
            );
        }
    }

    #[test]
    fn redate_key_name_keeps_the_grammar_and_swaps_only_the_date() {
        assert_eq!(
            redate_key_name("peer-ana-lopez-acme-zagros-2026-09-01", "2026-09-22").as_deref(),
            Some("peer-ana-lopez-acme-zagros-2026-09-22")
        );
        assert_eq!(redate_key_name("peer-ana-zagros", "2026-09-22"), None);
        assert_eq!(redate_key_name("peer-ana-zagros-2026-09-01-handset", "2026-09-22"), None);
        assert_eq!(key_name_date("2026-09-01"), "", "a bare date is not a key name");
    }

    #[test]
    fn redate_key_name_accepts_legacy_rosenpass_reissue_names() {
        for (name, expected) in [
            (
                "peer-ana-lopez-zagros-2026-09-01-rosenpass-reissue",
                "peer-ana-lopez-zagros-2026-09-22",
            ),
            (
                "peer-ana-lopez-acme-zagros-2026-09-01-rosenpass-reissue",
                "peer-ana-lopez-acme-zagros-2026-09-22",
            ),
        ] {
            assert_eq!(redate_key_name(name, "2026-09-22").as_deref(), Some(expected));
        }
    }

    #[test]
    fn redate_key_name_still_refuses_unsupported_names() {
        for name in [
            "",
            "-rosenpass-reissue",
            "2026-09-01-rosenpass-reissue",
            "peer-ana-zagros-rosenpass-reissue",
            "peer-ana-zagros-2026-09-xx-rosenpass-reissue",
            "peer-ana-zagros-2026-09-01-unknown",
            "peer-ana-zagros-2026-09-01-handset-rosenpass-reissue",
            "peer-ana-zagros-2026-09-01-rosenpass-reissue-handset",
            "peer-ana-zagros-2026-09-01-rosenpass-reissue-rosenpass-reissue",
        ] {
            assert_eq!(redate_key_name(name, "2026-09-22"), None, "{name}");
        }
    }

    #[test]
    fn reissue_audit_digest_names_key_and_requester_never_the_secret() {
        let groups = ["ZAGROS-DEVELOPER".to_string(), "ZAGROS-OBSERVER".to_string()];
        let d = ReissueDigest {
            kind: "device",
            handle: "user239x",
            env: "zagros",
            key_name: "peer-user239x-zagros-2026-09-22",
            uses: 5,
            ttl_days: 7,
            groups: &groups,
            requested_by: Some("admin@example.org"),
        }
        .render();
        assert!(d.contains("key=peer-user239x-zagros-2026-09-22"));
        assert!(d.contains("uses=5"));
        assert!(d.contains("requested_by=admin@example.org"));
        let d2 = ReissueDigest {
            kind: "handset",
            handle: "a",
            env: "zagros",
            key_name: "k",
            uses: 1,
            ttl_days: 7,
            groups: &[],
            requested_by: Some("  "),
        }
        .render();
        assert!(d2.contains("requested_by=(operator)"));
    }

    #[test]
    fn render_netbird_section_phone_subsection_only_with_a_handset_key() {
        let mut nb = NbProvision {
            setup_key: "K-DEV".into(),
            mgmt_url: "https://netbird.zagros.example".into(),
            key_name: "peer-x-zagros-2026-09-10".into(),
            expires_days: 30,
            usage_limit: 5,
            group_full_names: vec!["ZAGROS-PLATFORM-ADMIN".into()],
            rosenpass: true,
            handset_key: None,
            handset_key_name: None,
            minted: Vec::new(),
        };
        assert!(!render_netbird_section(Some(&nb)).contains("Phone or tablet"));
        nb.handset_key = Some("K-PHONE".into());
        nb.handset_key_name = Some("peer-x-zagros-2026-09-10-handset".into());
        let sec = render_netbird_section(Some(&nb));
        assert!(sec.contains("### Phone or tablet"));
        assert!(sec.contains("| Handset key | `K-PHONE` |"));
        assert!(sec.contains("Private DNS"));
        assert!(sec.contains("one VPN at a time"));
        // The device key section is unchanged and precedes the phone one.
        assert!(sec.find("--setup-key K-DEV").unwrap() < sec.find("### Phone or tablet").unwrap());
    }
}

#[cfg(test)]
mod reissue_offboard_tests {
    #[test]
    fn replacement_key_names_are_included_in_the_final_cut() {
        for tag in ["", "fixture", "example-org"] {
            let device = super::peername::mint("fixture", tag, "zagros", "2026-10-11");
            let handset = super::handset_key_name(&device);
            for name in [&device, &handset] {
                assert!(super::revoke_peer_matches(name, "fixture", tag, "zagros"));
                assert!(!super::revoke_peer_matches(name, "other", tag, "zagros"));
                assert!(!super::revoke_peer_matches(name, "fixture", tag, "other"));
            }
        }
        let prior = "peer-fixture-zagros-2026-09-01";
        let fallback = super::redate_key_name(prior, "2026-10-11").unwrap();
        assert!(super::revoke_peer_matches(&fallback, "fixture", "", "zagros"));
    }
}

// --- queue ls --email completer (goat#334) ----------------------------------
#[cfg(test)]
mod queue_ls_completer {
    #[tokio::test]
    async fn queue_ls_sends_email_and_preserves_explicit_status() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for (status, email, expected_status) in [
            (None, Some("person+tag&other=#@example.test"), None),
            (Some("archived"), Some("person@example.test"), Some("archived")),
            (None, None, Some("sponsor_approved")),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/onboard/proposals"))
                .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
                .expect(1)
                .mount(&server)
                .await;
            let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
            super::run_queue(&portal, super::QueueAction::Ls {
                status: status.map(str::to_string),
                env: None,
                email: email.map(str::to_string),
                json: true,
            }).await.unwrap();
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let pairs: std::collections::HashMap<_, _> = requests[0].url.query_pairs().into_owned().collect();
            assert_eq!(pairs.get("status").map(String::as_str), expected_status);
            assert_eq!(pairs.get("email").map(String::as_str), email);
            server.verify().await;
        }
    }

    #[test]
    fn email_lookup_warns_when_filters_can_hide_a_blocker() {
        assert!(super::queue_ls_lookup_hint("all", None, Some("a@b.test")).is_none());
        assert!(super::queue_ls_lookup_hint("ALL", None, Some("a@b.test")).is_none());
        assert!(super::queue_ls_lookup_hint("proposed", None, Some("a@b.test")).is_some());
        assert!(super::queue_ls_lookup_hint("all", Some("zagros"), Some("a@b.test")).is_some());
        assert!(super::queue_ls_lookup_hint("proposed", Some("zagros"), None).is_none());
    }

    use super::{
        queue_ls_effective_status, queue_ls_filter_desc, queue_ls_flag_value, queue_ls_retain,
    };
    use panoscope_core::portal::Proposal;

    fn ls_prop(env: &str, email: &str, status: &str) -> Proposal {
        serde_json::from_value(serde_json::json!({
            "id": "prop-0000000000000000",
            "env": env,
            "handle": "h",
            "email": email,
            "status": status,
            "escalated": false,
            "complete": true,
            "petitions": "[]"
        }))
        .unwrap()
    }

    #[test]
    fn queue_ls_default_status_stays_sponsor_approved() {
        assert_eq!(queue_ls_effective_status(None, false), "sponsor_approved");
    }

    #[test]
    fn queue_ls_email_widens_default_status_to_all() {
        // The blocking proposal a refused submit names can sit in ANY state;
        // an email lookup that silently kept the sponsor_approved default
        // would usually print nothing — the exact defect goat#334 is about.
        assert_eq!(queue_ls_effective_status(None, true), "all");
    }

    #[test]
    fn queue_ls_explicit_status_wins_over_email_widening() {
        assert_eq!(
            queue_ls_effective_status(Some("proposed"), true),
            "proposed"
        );
    }

    #[test]
    fn queue_ls_flag_value_trims_and_passes_through() {
        assert_eq!(
            queue_ls_flag_value("email", "hint", Some(" A@b.test "))
                .unwrap()
                .as_deref(),
            Some("A@b.test")
        );
        assert_eq!(
            queue_ls_flag_value("env", "hint", Some(" zagros "))
                .unwrap()
                .as_deref(),
            Some("zagros")
        );
        assert_eq!(queue_ls_flag_value("email", "hint", None).unwrap(), None);
    }

    #[test]
    fn queue_ls_flag_value_refuses_blank_naming_the_flag() {
        // The `--flag "$UNSET_VAR"` accident: --email was refused, but a blank
        // --status listed everything and a blank --env listed nothing. All
        // three ls filters refuse blank the same way, naming the flag.
        let err = queue_ls_flag_value("env", "hint", Some("   ")).unwrap_err();
        assert!(err.to_string().contains("--env is blank"));
        let err = queue_ls_flag_value("status", "hint", Some("")).unwrap_err();
        assert!(err.to_string().contains("--status is blank"));
    }

    #[test]
    fn queue_ls_retain_matches_email_exactly_case_insensitive() {
        let mut rows = vec![
            ls_prop("zagros", "alice@example.test", "onboarded"),
            // near-miss rows: a superstring and a suffix — an exact match
            // must keep neither (substring matching acts on the wrong row)
            ls_prop("zagros", "alice@example.test.evil", "proposed"),
            ls_prop("zagros", "malice@example.test", "proposed"),
        ];
        queue_ls_retain(&mut rows, None, Some("ALICE@EXAMPLE.TEST"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].email, "alice@example.test");
    }

    #[test]
    fn queue_ls_retain_ands_env_and_email() {
        let mut rows = vec![
            ls_prop("zagros", "alice@example.test", "onboarded"),
            ls_prop("nova", "alice@example.test", "proposed"),
        ];
        queue_ls_retain(&mut rows, Some("zagros"), Some("alice@example.test"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].env, "zagros");
    }

    #[test]
    fn queue_ls_retain_without_filters_keeps_everything() {
        let mut rows = vec![
            ls_prop("zagros", "alice@example.test", "onboarded"),
            ls_prop("nova", "bob@example.test", "proposed"),
        ];
        queue_ls_retain(&mut rows, None, None);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn queue_ls_status_param_maps_all_to_empty_any_case() {
        // The wire contract: `all` (any case) means NO ?status= param —
        // a case-sensitive comparison here would silently send ?status=ALL
        // and return nothing while every unit test stays green.
        assert_eq!(super::queue_ls_status_param("all"), "");
        assert_eq!(super::queue_ls_status_param("ALL"), "");
        assert_eq!(super::queue_ls_status_param("proposed"), "proposed");
        assert_eq!(
            super::queue_ls_status_param("sponsor_approved"),
            "sponsor_approved"
        );
    }

    #[test]
    fn queue_ls_retain_email_fold_is_ascii_only_like_the_store() {
        // Documents intent, not an accident: the store's F-239 collision key
        // folds with SQLite lower(), which is ASCII-only — so ü and Ü are
        // DIFFERENT emails on both sides of the wire, by design.
        let mut rows = vec![ls_prop("zagros", "\u{fc}ser@example.test", "proposed")];
        queue_ls_retain(&mut rows, None, Some("\u{dc}SER@EXAMPLE.TEST"));
        assert!(rows.is_empty());
        let mut rows = vec![ls_prop("zagros", "\u{fc}ser@example.test", "proposed")];
        queue_ls_retain(&mut rows, None, Some("\u{fc}ser@EXAMPLE.TEST"));
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn queue_ls_blank_email_refused_before_any_request() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer};
        let server = MockServer::start().await;
        // Zero requests allowed: the refusal must precede the network call.
        Mock::given(method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("[]"))
            .expect(0)
            .mount(&server)
            .await;
        let portal = panoscope_core::portal::PortalClient::new(server.uri(), "k".into());
        let err = super::run_queue(
            &portal,
            super::QueueAction::Ls {
                status: None,
                env: None,
                email: Some("   ".into()),
                json: false,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--email is blank"));
        server.verify().await;
    }

    #[test]
    fn queue_ls_flag_surface_parses() {
        use clap::Parser;
        #[derive(Parser)]
        struct Probe {
            #[command(subcommand)]
            action: super::QueueAction,
        }
        // Bare `ls`: clap injects NO status default — the sponsor_approved /
        // all resolution lives in queue_ls_effective_status alone, so the
        // default cannot fork between the parser and the logic.
        let bare = Probe::parse_from(["probe", "ls"]);
        let super::QueueAction::Ls {
            status,
            env,
            email,
            json,
        } = bare.action
        else {
            panic!("bare `ls` parsed to the wrong variant");
        };
        assert_eq!(status, None);
        assert_eq!(env, None);
        assert_eq!(email, None);
        assert!(!json);

        let filtered = Probe::parse_from(["probe", "ls", "--email", "a@b.test"]);
        let super::QueueAction::Ls { status, email, .. } = filtered.action else {
            panic!("`ls --email` parsed to the wrong variant");
        };
        assert_eq!(status, None);
        assert_eq!(email.as_deref(), Some("a@b.test"));
    }

    #[test]
    fn queue_ls_empty_message_names_every_active_filter() {
        assert_eq!(
            queue_ls_filter_desc("sponsor_approved", None, None),
            "status=sponsor_approved"
        );
        assert_eq!(
            queue_ls_filter_desc("all", None, Some("a@b.test")),
            "status=all, email=a@b.test"
        );
        assert_eq!(
            queue_ls_filter_desc("all", Some("zagros"), Some("a@b.test")),
            "status=all, env=zagros, email=a@b.test"
        );
    }
}

#[cfg(test)]
mod revocation_retry_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000002";

    struct LocalCurl {
        old: Option<std::ffi::OsString>,
        _guard: tokio::sync::MutexGuard<'static, ()>,
    }
    impl LocalCurl {
        async fn enable() -> Self {
            static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
            let guard = LOCK.lock().await;
            let old = std::env::var_os("GOAT_ONBOARD_KRATOS_LOCAL");
            std::env::set_var("GOAT_ONBOARD_KRATOS_LOCAL", "1");
            Self { old, _guard: guard }
        }
    }
    impl Drop for LocalCurl {
        fn drop(&mut self) {
            match &self.old {
                Some(value) => std::env::set_var("GOAT_ONBOARD_KRATOS_LOCAL", value),
                None => std::env::remove_var("GOAT_ONBOARD_KRATOS_LOCAL"),
            }
        }
    }

    #[test]
    fn revocation_checkpoint_survives_restart_and_retains_prior_targets() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("revoke-targets").join("fixture");
        record_revoke_target(&directory, ID).unwrap();
        record_revoke_target(&directory, OTHER).unwrap();
        record_revoke_target(&directory, ID).unwrap();
        assert_eq!(
            read_revoke_targets(&directory).unwrap(),
            vec![ID.to_string(), OTHER.to_string()]
        );
        std::fs::write(directory.join(format!("{ID}.json")), b"truncated").unwrap();
        assert!(read_revoke_targets(&directory).is_err());
        assert!(record_revoke_target(&directory, ID).is_err());
        assert!(record_revoke_target(&directory, "../not-an-identity").is_err());
    }

    // The guard serializes the only tests that enable local curl mode;
    // every HTTP endpoint is a loopback fixture.
    #[tokio::test]
    async fn revocation_purge_retry_blocks_blank_oidc_user_after_kratos_deletion() {
        let _local = LocalCurl::enable().await;
        let server = MockServer::start().await;
        let deleted = Arc::new(AtomicBool::new(false));
        let blocked = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let state = deleted.clone();
        Mock::given(method("GET"))
            .and(path("/admin/identities"))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(if state.load(Ordering::SeqCst) {
                    serde_json::json!([])
                } else {
                    serde_json::json!([{"id":ID}])
                })
            })
            .mount(&server)
            .await;
        let state = deleted.clone();
        Mock::given(method("DELETE"))
            .and(path(format!("/admin/identities/{ID}")))
            .respond_with(move |_: &wiremock::Request| {
                state.store(true, Ordering::SeqCst);
                ResponseTemplate::new(204)
            })
            .expect(1)
            .mount(&server)
            .await;
        let state = blocked.clone();
        Mock::given(method("GET")).and(path("/api/users"))
            .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":ID,"email":"","name":"","role":"user","auto_groups":[],"is_blocked":state.load(Ordering::SeqCst)}
            ]))).mount(&server).await;
        let state = blocked.clone();
        let count = attempts.clone();
        Mock::given(method("PUT"))
            .and(path(format!("/api/users/{ID}")))
            .respond_with(move |_: &wiremock::Request| {
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(503)
                } else {
                    state.store(true, Ordering::SeqCst);
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":ID}))
                }
            })
            .mount(&server)
            .await;
        let cfg = OryEnvConfig {
            ssh_host: "fixture.invalid",
            ssh_user: "fixture",
            kratos_admin_url: server.uri(),
            auth_url: "https://auth.example.test",
            ssh_proxy: None,
        };
        let root = tempfile::tempdir().unwrap();
        let targets = root.path().join("revoke-targets").join("fixture");
        let nb = panoscope_core::netbird::NetbirdClient::new(server.uri(), "fixture".into());
        revoke_ory_identity(&cfg, "person@example.test", true, false, &targets)
            .await
            .unwrap();
        assert!(deleted.load(Ordering::SeqCst));
        assert!(block_netbird_user(&nb, ID, false).await.is_err());
        // Restart: no Kratos row and no email/name fields in the NetBird row.
        assert_eq!(
            revoke_ory_identity(&cfg, "person@example.test", true, false, &targets)
                .await
                .unwrap()
                .0,
            "already-absent"
        );
        for id in read_revoke_targets(&targets).unwrap() {
            block_netbird_user(&nb, &id, false).await.unwrap();
        }
        assert!(blocked.load(Ordering::SeqCst));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn revocation_checkpoint_failure_prevents_kratos_delete() {
        let _local = LocalCurl::enable().await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/admin/identities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":ID}])))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(204))
            .expect(0)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let targets = root.path().join("not-a-directory");
        std::fs::write(&targets, b"occupied").unwrap();
        let cfg = OryEnvConfig {
            ssh_host: "fixture.invalid",
            ssh_user: "fixture",
            kratos_admin_url: server.uri(),
            auth_url: "https://auth.example.test",
            ssh_proxy: None,
        };
        assert!(
            revoke_ory_identity(&cfg, "person@example.test", true, false, &targets)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn revocation_netbird_block_requires_persisted_readback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id":ID,"email":"","name":"","role":"user","auto_groups":[],"is_blocked":false}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/users/{ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":ID})))
            .mount(&server)
            .await;
        let nb = panoscope_core::netbird::NetbirdClient::new(server.uri(), "fixture".into());
        assert!(block_netbird_user(&nb, ID, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("did not persist"));
    }
}

#[cfg(test)]
mod reinstatement_command_tests {
    use super::*;
    #[test]
    fn reinstatement_command_quotes_actor_and_reason_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("must-not-exist");
        let actor = format!("operator'$(touch {})", marker.display());
        let reason = "owner's explicit return; $(false)";
        let command =
            reinstate_remote_command("00000000-0000-4000-8000-000000000001", &actor, reason)
                .unwrap();
        let args = command
            .strip_prefix("sudo -n /usr/local/libexec/goat-reinstate-identity ")
            .unwrap();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("set -- {args}; printf '%s\\n' \"$@\" "))
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                "--identity",
                "00000000-0000-4000-8000-000000000001",
                "--actor",
                actor.as_str(),
                "--reason",
                reason
            ]
        );
        assert!(!marker.exists());
        assert!(reinstate_remote_command("../wrong", "operator", "reason").is_err());
        assert!(
            reinstate_remote_command("00000000-0000-4000-8000-000000000001", "operator", "")
                .is_err()
        );
    }
}

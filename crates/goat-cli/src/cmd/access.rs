// `goat access` — the CLI half of the Data-Access COP (ADR 1026, goat-cli
// Phase G.4 PR-C). Thin client over the portal's read API:
//   who   <id>        forward — what may this identity write/read, and why
//   topic <key>       reverse — who may write/read this topic (operator-only)
//   why   <id> <key>  trace   — one allow/deny across the four layers
//
// `who`/`why` are self-or-operator scoped server-side: a vendor token may
// query its OWN prefix without admin (the catch-22 fix — a denied vendor
// can self-diagnose). `topic` is operator-only. The `doctor --topic`
// write-precheck is a separate, probe-based path (PR-C2) that needs no COP
// access at all.

use crate::output::{with_spinner, OutputContext};
use clap::Subcommand;
use panoscope_core::portal::{
    AccessTopic, AccessWho, AccessWhy, PortalClient, PortalError, Probation, ProbationHold,
    RecipientCert, RepoGrantRef, RosterFreshness,
};

#[derive(Subcommand, Debug)]
pub enum AccessCmd {
    /// Forward query: what may this identity write and read, with the
    /// reason at each layer. An org may ask about its own prefix; an
    /// operator may ask about any identity.
    Who {
        /// Identity selector: an org prefix, person email, peer name, or slot id.
        id: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reverse query: who may write and read this topic, and which partners
    /// may receive it if released. Operator-only.
    Topic {
        /// Topic key (e.g. `acme/observations/v1`).
        key: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Who administers an org's data governance, nominate an admin, or
    /// remove one (ADR 1102 / U9). An org starts in BOOTSTRAP (every member
    /// governs) and NARROWS itself with its first nomination — from then on
    /// only nominated admins govern, and the last one cannot be removed.
    #[command(subcommand)]
    Govern(GovernCmd),
    /// Grant-encryption leaf-key operations. With no subcommand: is this
    /// anchor's decrypt key published to the recipient-cert store (U7
    /// publish-my-key)? Defaults to the anchor in your own profile cert; a miss
    /// names the remedy. The `rotate` subcommand rotates the member leaf key
    /// used for grant-encryption unwrapping (a convenience over the grant-crypto
    /// SDK's `rotate`; distinct from `goat admin rotate` = org cert reissue).
    Key {
        /// (status) Anchor to check (default: your own profile cert's anchor).
        #[arg(long)]
        anchor: Option<String>,
        /// (status) Print the full PEM instead of the summary line.
        #[arg(long)]
        pem: bool,
        /// `rotate` the leaf key; omit for the U7 publish-status check.
        #[command(subcommand)]
        action: Option<KeyCmd>,
    },
    /// Trace one allow/deny decision for `<id>` against `<key>` across
    /// mesh → identity → Zenoh ACL → release.
    Why {
        /// Identity selector (an org may pass its own prefix).
        id: String,
        /// Topic key.
        key: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Collect every stray (live, no declared match) and bucket it by WHY it
    /// didn't reconcile — the safe first step of a stray sweep. Read-only:
    /// builds the cut-candidate + verify-first lists; it never cuts. The cut
    /// stays the audited `goat admin onboard --revoke`, one at a time.
    Strays {
        /// Emit the raw JSON (the frozen stray set, bucketed).
        #[arg(long)]
        json: bool,
    },
    /// Anomaly probation (ADR 1068): hold something that doesn't reconcile
    /// instead of leaving or deleting it — verify, then repair or purge. The
    /// hold/decide are recorded + audited; the actual repair/purge runs through
    /// the existing `goat admin` paths. Operator-only.
    #[command(subcommand)]
    Probation(ProbationCmd),
    /// Per-onboardee GitHub repo-access audibles (docs/design/onboarding-github-repo-grants.md
    /// §2): preview the effective set (on-rails policy ⊕ overrides) for a proposal,
    /// or add/remove a repo override on top of the policy at review time.
    #[command(subcommand)]
    Repo(RepoCmd),
    /// Preemptively GRANT a producer's data to a grantee (ADR 1075 §12) — the
    /// mutation half of the access language. Operator or the producer's
    /// custodian only; the same grant row the request-approve path writes.
    Grant {
        /// Producer key whose data is being shared (the write anchor / org prefix).
        producer: String,
        /// Grantee: an org prefix (default) or a principal id with `--principal`.
        grantee: String,
        /// Grant to a principal (person/machine id) instead of an org.
        #[arg(long)]
        principal: bool,
        /// Why — recorded on the grant row (audited).
        #[arg(long)]
        justification: String,
        /// Optional expiry as a unix timestamp (seconds); omit for open-ended.
        #[arg(long)]
        expires: Option<i64>,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// AUDIENCE — who can actually read a producer's stream: the machine anchors
    /// its grant-keyed DEK is wrapped to (the derived audience, WS4). Shows the
    /// members plus any honesty notes (browser-only readers, grantee-less orgs).
    Audience {
        /// Producer key / release-set (rs ≡ producer).
        producer: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// REVOKE a grant by id (ADR 1031 reversibility) — end-dates it, keeping the
    /// row for audit history. Operator or the producer's custodian only.
    Revoke {
        /// Grant id (from `goat access who` / the grant row).
        id: String,
        /// Confirm on the fabric that the revoke took effect, by reading the
        /// release-set's key bundle back and reporting who can still decrypt.
        /// Revoking end-dates a grant row; the read cut only lands once the key
        /// service publishes a new generation excluding the member, and a portal
        /// call returning success says nothing about that.
        #[arg(long, value_name = "RELEASE_SET")]
        confirm: Option<String>,
        /// With `--confirm`, assert this member anchor holds no slot in the new
        /// generation. Without it the command reports the resulting audience but
        /// asserts nothing about who left.
        #[arg(long, value_name = "ANCHOR")]
        expect_excluded: Option<String>,
        /// How long to wait for the new generation (default 90s — three of the
        /// key service's 30s poll intervals).
        #[arg(long, default_value_t = 90)]
        confirm_timeout_secs: u64,
        /// Signer-trust inputs for the bundle verification, as `goat pub --sealed`.
        #[arg(long)]
        key_service_bundle: Option<String>,
        #[arg(long)]
        trust_domain: Option<String>,
        #[arg(long)]
        key_service_id: Option<String>,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Co-publish onto multi-publisher topics (ADR 1105, PUBSUB-ORG-ADMIN):
    /// single-owner roots are inviolable; create a marked multi-publisher
    /// topic and add
    /// your own org's publishers to it.
    #[command(subcommand)]
    CoPublish(CoPublishCmd),
}

#[derive(Subcommand, Debug)]
pub enum KeyCmd {
    /// Rotate this member's grant-encryption LEAF KEY — the key used to unwrap
    /// the DEKs that open sealed streams (`goat sub --open`). A convenience over
    /// the grant-crypto SDK's `rotate` (ADR 1106); it computes the rotation
    /// locally (the platform holds no keys and never re-wraps) and prints a
    /// structured RotationReceipt.
    ///
    /// Two policies: `--destroy` (DEFAULT) drops the retiring key and re-wraps
    /// nothing — full forward secrecy for pre-rotation generations; `--keep-access`
    /// re-wraps this member's still-live DEKs to the new key, which REDUCES
    /// forward secrecy for those generations.
    ///
    /// This is DISTINCT from `goat admin rotate` (org cert reissue). Because a
    /// member's grant-encryption leaf key IS its mTLS profile key — whose
    /// re-issue is a CA operation (ADR 1044) — this verb NEVER overwrites your
    /// profile: it writes the freshly generated key to `--out` for you to install
    /// via the CA-coupled leaf re-issue so your cert continues to match. Until
    /// then your live key is unchanged.
    Rotate {
        /// Release-set (producer anchor) whose visible generations the rotation
        /// is computed against. Default: your own producer anchor.
        #[arg(long)]
        release_set: Option<String>,
        /// Re-wrap this member's still-live DEKs to the new key before retiring
        /// the old one. REDUCES forward secrecy for those generations. Mutually
        /// exclusive with `--destroy`.
        #[arg(long, conflicts_with = "destroy")]
        keep_access: bool,
        /// Drop the retiring key, re-wrap nothing (the DEFAULT — full forward
        /// secrecy). Explicit here only for symmetry / clarity.
        #[arg(long)]
        destroy: bool,
        /// Write the freshly generated new private key (PKCS#8 PEM) here. Never
        /// your profile key — installing it is the CA-coupled leaf re-issue.
        #[arg(long)]
        out: std::path::PathBuf,
        /// (with --keep-access) Write the re-wrapped DEK sidecar (JSON) here.
        /// Default: `<out>.rewrap.json`. These stay LOCAL — they are not
        /// published to the shared `keys/**` bundle tree.
        #[arg(long)]
        rewrap_out: Option<std::path::PathBuf>,
        /// The key service's SPIRE trust bundle PEM (fetch/verify the bundles).
        #[arg(long)]
        key_service_bundle: Option<String>,
        /// The goatnet's SPIFFE trust domain (e.g. zagros.zagros.example).
        #[arg(long)]
        trust_domain: Option<String>,
        /// The key service's SPIFFE id (default spiffe://<trust-domain>/grant-keyd).
        #[arg(long)]
        key_service_id: Option<String>,
        /// Emit the raw JSON (the receipt plus the emitted paths).
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CoPublishCmd {
    /// CREATE a multi-publisher topic for an org (a deliberate act).
    /// Single-owner roots are never involved — this is a separate, marked class.
    CreateTopic {
        /// Owning org id (the naming org — you must govern it).
        org: String,
        /// Human display name (what a discoverer sees).
        #[arg(long)]
        name: String,
        /// Why (recorded, audited).
        #[arg(long)]
        justification: String,
        /// Routed-only: do NOT render a history store for it.
        #[arg(long)]
        no_persist: bool,
        #[arg(long)]
        json: bool,
    },
    /// LIST/discover multi-publisher topics (the MULTI-PUBLISHER marker,
    /// owning org, and declared publishers).
    ListTopics {
        /// Narrow to one org.
        #[arg(long)]
        org: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// ADD a publisher to a multi-publisher topic (at-most-own-org, enforced
    /// server-side).
    Grant {
        /// The publisher's anchor (grantee) — the slot uuid that gains write.
        writer: String,
        /// The target REGISTERED multi-publisher topic id (from `list-topics`).
        #[arg(long = "to")]
        multipub_topic: String,
        /// Why (required, audited).
        #[arg(long)]
        justification: String,
        /// Optional expiry as a unix timestamp (seconds).
        #[arg(long)]
        expires: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// REVOKE a co-publish grant by id.
    Revoke {
        /// Grant id.
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// EXPORT the materialized zenoh_copublish_grants group_var (operator-only).
    Export {
        /// Emit the raw JSON (the export is always JSON).
        #[arg(long)]
        json: bool,
    },
}

/// `goat access repo <subcmd>` — the repo-access audible layer from the CLI.
#[derive(Subcommand, Debug)]
pub enum RepoCmd {
    /// Show the loaded on-rails repo-access policy (default owner + rules) — the
    /// default set a proposal's effective grants start from.
    Policy {
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Preview the effective repo-access set (policy ⊕ overrides) for a proposal,
    /// each repo tagged with its source (policy / audible).
    Preview {
        /// Proposal id (`prop-…`).
        id: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a repo override onto a proposal (or raise its permission). GETs the
    /// current overrides, applies this one add, POSTs them back.
    Add {
        /// Proposal id (`prop-…`).
        id: String,
        /// Repo as `owner/name` (or a bare `name` for the policy default owner),
        /// optionally `owner/name:perm` (perm ∈ pull|triage|push|maintain|admin;
        /// default pull).
        repo: String,
    },
    /// Remove a repo override from a proposal (drops it from the effective set —
    /// wins over the policy AND any add). GETs the current overrides, applies this
    /// one remove, POSTs them back.
    Remove {
        /// Proposal id (`prop-…`).
        id: String,
        /// Repo as `owner/name` (or a bare `name`).
        repo: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProbationCmd {
    /// List entities on probation (open by default; `--all` for history), with
    /// the cure-by countdown. Expired entries are flagged, never auto-cut.
    List {
        /// Include the decided (repaired/purged) history, not just open holds.
        #[arg(long)]
        all: bool,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Hold an entity on probation. `<ref>` is its stable id (a stray's
    /// identity id / slot anchor / recon key).
    Hold {
        /// The entity to hold.
        ref_: String,
        /// Why it's held (e.g. `stray:unregistered-key`).
        #[arg(long, default_value = "")]
        reason: String,
        /// Human label for display (email / alias / name).
        #[arg(long, default_value = "")]
        label: String,
        /// Anomaly kind (v1: identity).
        #[arg(long, default_value = "identity")]
        kind: String,
        /// Cure-by window in days (default = the portal policy default, 14d).
        #[arg(long)]
        cure_days: Option<i64>,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Record a terminal disposition for a held entry. `repair` regularizes it
    /// (via `goat admin onboard`); `purge` removes + cleanses it (via
    /// `goat admin onboard --revoke --purge`). Prints the follow-up command.
    Decide {
        /// The probation entry id (`prob-…`).
        id: String,
        /// `repair` (keep + regularize) or `purge` (remove + cleanse).
        #[arg(value_parser = ["repair", "purge"])]
        verdict: String,
        /// A note recorded with the decision.
        #[arg(long, default_value = "")]
        note: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum GovernCmd {
    /// List an org's governance admins and which mode it is in
    /// (bootstrap vs nominated/narrowed).
    List {
        /// Org id (kebab prefix).
        org: String,
        /// Include removed (end-dated) rows — the audit history.
        #[arg(long)]
        all: bool,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Nominate a governance admin (at-most-own-scope; nominee must be an
    /// active member of the org). THE FIRST NOMINATION NARROWS THE ORG.
    Nominate {
        /// Org id.
        org: String,
        /// Nominee: their login email, or an identity UUID via --identity.
        #[arg(long, conflicts_with = "identity")]
        email: Option<String>,
        /// Nominee identity UUID (alternative to --email).
        #[arg(long)]
        identity: Option<String>,
        /// The recorded why.
        #[arg(long, default_value = "")]
        justification: String,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove a nomination by id (end-dated, kept for audit). Removing the
    /// org's LAST active admin is refused — nominate a replacement first.
    Remove {
        /// Nomination id (gadm-…), from `goat access govern list`.
        id: String,
    },
}

pub async fn run(ctx: &super::Ctx, op: AccessCmd) -> anyhow::Result<()> {
    let portal = ctx.profile.portal_client()?;
    let out = &ctx.output;
    match op {
        AccessCmd::Who { id, json } => {
            let id = resolve_me(out, &portal, &id).await?;
            who(out, &portal, &id, json).await
        }
        AccessCmd::Topic { key, json } => topic(out, &portal, &key, json).await,
        AccessCmd::Why { id, key, json } => {
            let id = resolve_me(out, &portal, &id).await?;
            why(out, &portal, &id, &key, json).await
        }
        AccessCmd::Strays { json } => {
            let goatnet = ctx.profile.goatnet_endpoints().goatnet;
            strays(out, &portal, json, goatnet.as_deref()).await
        }
        AccessCmd::Probation(op) => probation(out, &portal, op).await,
        AccessCmd::Repo(op) => repo(out, &portal, op).await,
        AccessCmd::Grant {
            producer,
            grantee,
            principal,
            justification,
            expires,
            json,
        } => {
            grant(
                out,
                &portal,
                &producer,
                &grantee,
                principal,
                &justification,
                expires,
                json,
            )
            .await
        }
        AccessCmd::Audience { producer, json } => audience(out, &portal, &producer, json).await,
        AccessCmd::Revoke {
            id,
            confirm,
            expect_excluded,
            confirm_timeout_secs,
            key_service_bundle,
            trust_domain,
            key_service_id,
            json,
        } => {
            revoke(
                ctx,
                out,
                &portal,
                &id,
                confirm,
                expect_excluded,
                confirm_timeout_secs,
                key_service_bundle,
                trust_domain,
                key_service_id,
                json,
            )
            .await
        }
        AccessCmd::Govern(op) => govern(out, &portal, op).await,
        AccessCmd::Key {
            anchor,
            pem,
            action,
        } => match action {
            None => key_status(ctx, &portal, anchor, pem).await,
            Some(KeyCmd::Rotate {
                release_set,
                keep_access,
                destroy,
                out,
                rewrap_out,
                key_service_bundle,
                trust_domain,
                key_service_id,
                json,
            }) => {
                let _ = destroy; // destroy is the default; the flag is symmetry-only
                key_rotate(
                    ctx,
                    release_set,
                    keep_access,
                    out,
                    rewrap_out,
                    key_service_bundle,
                    trust_domain,
                    key_service_id,
                    json,
                )
                .await
            }
        },
        AccessCmd::CoPublish(op) => copublish(ctx, out, &portal, op).await,
    }
}

/// `goat access govern …` — the U9 surface behind the audited CLI chokepoint.
async fn govern(out: &OutputContext, portal: &PortalClient, op: GovernCmd) -> anyhow::Result<()> {
    match op {
        GovernCmd::List { org, all, json } => {
            let res = with_spinner(
                out,
                "reading governance admins",
                portal.govern_admins(Some(&org), all),
            )
            .await
            .map_err(map_access_err)?;
            if json {
                println!("{}", serde_json::to_string(&res).unwrap_or_default());
                return Ok(());
            }
            println!("governance of {} — {}", res.org, res.mode);
            if res.admins.is_empty() {
                println!("  (no nomination rows)");
            }
            for a in &res.admins {
                let why = if a.justification.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", a.justification)
                };
                println!(
                    "  {}  {}  [{}] by {}{}",
                    a.id, a.identity_id, a.status, a.nominated_by, why
                );
            }
            Ok(())
        }
        GovernCmd::Nominate {
            org,
            email,
            identity,
            justification,
            json,
        } => {
            if email.is_none() && identity.is_none() {
                anyhow::bail!("pass the nominee via --email <addr> or --identity <uuid>");
            }
            let row = with_spinner(
                out,
                "nominating governance admin",
                portal.nominate_govern_admin(
                    &org,
                    email.as_deref(),
                    identity.as_deref(),
                    &justification,
                ),
            )
            .await
            .map_err(map_access_err)?;
            if json {
                println!("{}", serde_json::to_string(&row).unwrap_or_default());
                return Ok(());
            }
            println!(
                "✓ nominated {} as a governance admin of {} (id {})",
                row.identity_id, row.org, row.id
            );
            println!(
                "  note: the org is now NARROWED — only nominated admins govern it (ADR 1102)"
            );
            Ok(())
        }
        GovernCmd::Remove { id } => {
            match with_spinner(out, "removing nomination", portal.remove_govern_admin(&id)).await {
                Ok(()) => {
                    println!("✓ nomination {id} removed (end-dated; kept for audit)");
                    Ok(())
                }
                Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {
                    anyhow::bail!(
                        "refused: {id} is the org's last active governance admin — narrowing is one-way; \
                         nominate a replacement first (recovery for a locked-out org is operator proxy stewardship)"
                    )
                }
                Err(e) => Err(map_access_err(e)),
            }
        }
    }
}

/// `goat access key` — is this anchor's decrypt key published (U7)?
async fn key_status(
    ctx: &super::Ctx,
    portal: &PortalClient,
    anchor: Option<String>,
    pem: bool,
) -> anyhow::Result<()> {
    let anchor = match anchor {
        Some(a) => a,
        None => crate::cmd::pubsub::own_anchor(&ctx.profile).ok_or_else(|| {
            anyhow::anyhow!(
                "your profile cert carries no slot anchor (32-hex CN) — pass --anchor <a> explicitly"
            )
        })?,
    };
    let res = with_spinner(
        out_of(ctx),
        "checking recipient-cert store",
        portal.recipient_cert(&anchor),
    )
    .await
    .map_err(map_access_err)?;
    match res {
        RecipientCert::Published { pem: body } => {
            println!("✓ recipient key for {anchor} is PUBLISHED — the key service can wrap to it with no staging");
            if pem {
                print!("{body}");
            }
            Ok(())
        }
        RecipientCert::Missing { message } => {
            anyhow::bail!("recipient key for {anchor} is NOT published — {message}")
        }
    }
}

fn out_of(ctx: &super::Ctx) -> &OutputContext {
    &ctx.output
}

/// `goat access key rotate` — the member-leaf grant-encryption key rotation
/// convenience, backed 1:1 by the grant-crypto SDK's `rotate` (ADR 1106).
///
/// It reuses the sealed data path's identity + fetch/verify helpers
/// (`SelfMember`/`SealTrust`/`fetch_verified_bundles`) so the member id, unwrap
/// key, and signer-trust discipline are exactly those of `goat pub --sealed` /
/// `goat sub --open` — no second crypto surface.
///
/// Safety scope (see the subcommand `--help`): the new key is EMITTED to `--out`,
/// never written into the profile. A member's grant-encryption leaf key is its
/// mTLS profile key, whose re-issue is a CA operation (ADR 1044); silently
/// swapping a locally generated key would break the cert↔key match and the mTLS
/// identity. So this verb computes the rotation + receipt and hands the operator
/// the new key to install through the CA-coupled leaf re-issue.
#[allow(clippy::too_many_arguments)]
async fn key_rotate(
    ctx: &super::Ctx,
    release_set: Option<String>,
    keep_access: bool,
    out: std::path::PathBuf,
    rewrap_out: Option<std::path::PathBuf>,
    key_service_bundle: Option<String>,
    trust_domain: Option<String>,
    key_service_id: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    use grant_crypto::rotate::{rotate, Identity, RotationPolicy};
    use p256::elliptic_curve::Generate;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};
    use p256::SecretKey;

    // Refuse to overwrite the live profile mTLS key — installing the new key is
    // the CA-coupled leaf re-issue, not a local file swap.
    if out == ctx.profile.key_path() {
        anyhow::bail!(
            "--out must not be your profile key ({}) — a member's grant-encryption leaf key IS \
             its mTLS key, whose re-issue is a CA operation (`goat admin rotate`, ADR 1044). \
             Emit the new key to a separate path and install it via the leaf re-issue.",
            ctx.profile.key_path().display()
        );
    }

    let trust = crate::cmd::sealed::SealTrust::from_flags(
        key_service_bundle,
        trust_domain,
        key_service_id,
    )?;
    let me = crate::cmd::sealed::SelfMember::from_profile(&ctx.profile)?;
    let rs = release_set.unwrap_or_else(|| me.anchor.clone());

    // Fetch + verify the release-set's key bundles over a read-only fabric
    // session (the same signer-trust cut sealed pub/sub uses).
    let session = crate::cmd::pubsub::open_session_readonly(&ctx.profile).await?;
    let bundles = crate::cmd::sealed::fetch_verified_bundles(
        &session,
        &rs,
        &trust,
        &ctx.profile.router_endpoint_label(),
    )
    .await;
    let _ = session.close().await;
    let bundles = bundles?;

    let policy = if keep_access {
        RotationPolicy::KeepAccess
    } else {
        RotationPolicy::Destroy
    };

    // The old identity is the profile's own member id + enrollment key; rotate
    // consumes it (zeroizing the retiring secret) and returns the receipt.
    let old = Identity::new(me.anchor.clone(), me.secret);
    let new_secret =
        SecretKey::try_generate().map_err(|e| anyhow::anyhow!("generate new private key: {e}"))?;
    let new_pem = new_secret
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("encode new private key: {e}"))?;
    let (_new_identity, receipt, rewrapped) = rotate(old, new_secret, policy, &bundles);

    // Emit the new private key (0600). NEVER the profile.
    std::fs::write(&out, new_pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("write new key to {}: {e}", out.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600));
    }

    // Under keep-access, persist the re-wrapped DEKs LOCALLY (a member-side
    // sidecar), NOT to the shared `keys/**` tree: that tree carries only
    // grant-keyd-SIGNED bundles, so a member-authored PUT would be dropped by
    // the same signer verification `fetch_verified_bundles` enforces.
    let rewrap_path = if keep_access {
        let p = rewrap_out.unwrap_or_else(|| {
            let mut p = out.clone().into_os_string();
            p.push(".rewrap.json");
            std::path::PathBuf::from(p)
        });
        let body = serde_json::to_string_pretty(&rewrapped).unwrap_or_default();
        std::fs::write(&p, body)
            .map_err(|e| anyhow::anyhow!("write re-wrap sidecar to {}: {e}", p.display()))?;
        Some(p)
    } else {
        None
    };

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "receipt": receipt,
                "release_set": rs,
                "new_key_path": out.display().to_string(),
                "rewrap_path": rewrap_path.as_ref().map(|p| p.display().to_string()),
                "installed": false,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }

    println!(
        "member-leaf rotation computed for {} — policy {} at {}",
        receipt.member_id, receipt.policy, receipt.timestamp_rfc3339
    );
    println!("  release-set:              {rs}");
    println!(
        "  generations affected:     {}",
        receipt.now_unreadable_generations
    );
    if keep_access {
        println!(
            "  re-wrapped (keep-access): {}",
            receipt.rewrapped_generations
        );
    }
    println!("  new private key WRITTEN:  {}", out.display());
    if let Some(p) = &rewrap_path {
        println!("  re-wrap material (local): {}", p.display());
    }
    println!();
    println!(
        "NOT INSTALLED: your live mTLS identity is unchanged. Your grant-encryption leaf key IS"
    );
    println!("your mTLS leaf key, whose re-issue is a CA operation — install this new key via the");
    println!("CA-coupled leaf re-issue (`goat admin rotate`, ADR 1044) so your cert matches. The");
    println!(
        "forward-secrecy benefit of --destroy is realized only once the old key is retired then."
    );
    Ok(())
}

async fn copublish(
    ctx: &super::Ctx,
    out: &OutputContext,
    portal: &PortalClient,
    op: CoPublishCmd,
) -> anyhow::Result<()> {
    match op {
        CoPublishCmd::CreateTopic {
            org,
            name,
            justification,
            no_persist,
            json,
        } => {
            let spec = panoscope_core::portal::MultipubTopicSpec {
                org: org.clone(),
                display_name: name.clone(),
                justification: justification.clone(),
                persist: Some(!no_persist),
            };
            let res = with_spinner(
                out,
                "creating multi-publisher topic",
                portal.create_multipub_topic(&spec),
            )
            .await;
            let row = match res {
                Ok(row) => {
                    emit_copublish_audit(
                        ctx,
                        "pubsub.multipub-topic.create",
                        &row.key_base,
                        "",
                        &org,
                        true,
                    )
                    .await;
                    row
                }
                Err(e) => {
                    emit_copublish_audit(
                        ctx,
                        "pubsub.multipub-topic.create",
                        &org,
                        "",
                        &org,
                        false,
                    )
                    .await;
                    return Err(map_access_err(e));
                }
            };
            if json {
                println!("{}", serde_json::to_string(&row).unwrap_or_default());
                return Ok(());
            }
            println!(
                "✓ multi-publisher topic created: {} ({}, id {}) — owned by {}",
                row.display_name, row.key_base, row.topic_id, row.org
            );
            Ok(())
        }
        CoPublishCmd::ListTopics { org, json } => {
            let rows = with_spinner(
                out,
                "listing multi-publisher topics",
                portal.list_multipub_topics(org.as_deref()),
            )
            .await
            .map_err(map_access_err)?;
            if json {
                println!("{}", serde_json::to_string(&rows).unwrap_or_default());
                return Ok(());
            }
            println!("MULTI-PUBLISHER TOPICS — {}", rows.len());
            if rows.is_empty() {
                println!("  (none)");
            }
            for r in &rows {
                println!(
                    "  {}  [MULTI-PUBLISHER]  org={}  id={}  key={}  publishers={}",
                    r.display_name,
                    r.org,
                    r.topic_id,
                    r.key_base,
                    r.publishers.len()
                );
                for p in &r.publishers {
                    println!("      {p}");
                }
            }
            Ok(())
        }
        CoPublishCmd::Grant {
            writer,
            multipub_topic,
            justification,
            expires,
            json,
        } => {
            let spec = panoscope_core::portal::CoPublishSpec {
                writer: writer.clone(),
                multipub_topic_id: multipub_topic.clone(),
                justification: justification.clone(),
                effective_to: expires,
            };
            let res = with_spinner(
                out,
                "adding publisher to multi-publisher topic",
                portal.create_copublish_grant(&spec),
            )
            .await;
            let row = match res {
                Ok(row) => {
                    emit_copublish_audit(
                        ctx,
                        "pubsub.write-scope.set",
                        &writer,
                        &writer,
                        &multipub_topic,
                        true,
                    )
                    .await;
                    row
                }
                Err(e) => {
                    emit_copublish_audit(
                        ctx,
                        "pubsub.write-scope.set",
                        &writer,
                        &writer,
                        &multipub_topic,
                        false,
                    )
                    .await;
                    return Err(map_access_err(e));
                }
            };
            if json {
                println!("{}", serde_json::to_string(&row).unwrap_or_default());
                return Ok(());
            }
            println!(
                "✓ publisher added: {} may publish topic {} (grant {}, status {})",
                writer, multipub_topic, row.id, row.status
            );
            Ok(())
        }
        CoPublishCmd::Revoke { id, json } => {
            let res =
                with_spinner(out, "revoking co-publish grant", portal.revoke_grant(&id)).await;
            match res {
                Ok(()) => {
                    emit_copublish_audit(ctx, "pubsub.write-scope.revoke", &id, "", "", true).await;
                }
                Err(e) => {
                    emit_copublish_audit(ctx, "pubsub.write-scope.revoke", &id, "", "", false)
                        .await;
                    return Err(map_access_err(e));
                }
            }
            if json {
                println!("{{\"grant\":\"{id}\",\"status\":\"revoked\"}}");
                return Ok(());
            }
            println!("✓ revoked co-publish grant {id}");
            Ok(())
        }
        CoPublishCmd::Export { json } => {
            let v = with_spinner(
                out,
                "exporting co-publish grants",
                portal.copublish_grants_export(),
            )
            .await
            .map_err(map_access_err)?;
            let _ = json; // the export is always the raw group_var JSON.
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            Ok(())
        }
    }
}

/// Best-effort operator-action audit for a co-publish mutation. `action`
/// is `pubsub.write-scope.set` / `pubsub.write-scope.revoke`; `target` is the
/// writer (grant) or the grant id (revoke). Never affects the caller's exit.
async fn emit_copublish_audit(
    ctx: &super::Ctx,
    action: &str,
    target: &str,
    writer: &str,
    producer: &str,
    ok: bool,
) {
    let actor = crate::cmd::audit::actor_handle(&ctx.profile);
    let outcome = if ok { "ok" } else { "failed" };
    crate::cmd::audit::emit(
        &ctx.profile,
        crate::cmd::audit::OperatorActionV1::new(
            &actor,
            "portal",
            action,
            target,
            ctx.profile.context.clone(),
            outcome,
        )
        .with_args_digest(&format!("co-publish writer={writer} target={producer}")),
    )
    .await;
}

/// `goat access audience <producer>` — the owner-facing read of the derived
/// machine-plane audience (WS4/U3). Renders the wrap-target anchors and any
/// honesty notes verbatim (browser-only readers, grantee-less orgs).
async fn audience(
    out: &OutputContext,
    portal: &PortalClient,
    producer: &str,
    json: bool,
) -> anyhow::Result<()> {
    let row = with_spinner(out, "deriving audience", portal.audience(producer))
        .await
        .map_err(map_access_err)?;
    if json {
        println!("{}", serde_json::to_string(&row).unwrap_or_default());
        return Ok(());
    }
    println!(
        "audience for {} — {} member(s) wrapped:",
        row.release_set,
        row.members.len()
    );
    for m in &row.members {
        println!("  {m}");
    }
    for n in &row.notes {
        println!("  note: {n}");
    }
    Ok(())
}

/// `goat access grant <producer> <grantee>` — the preemptive-grant mutation
/// verb (ADR 1075 §12). Wraps the portal `POST /api/access/grants`; the server
/// enforces operator-or-custodian and records the audit event.
#[allow(clippy::too_many_arguments)]
async fn grant(
    out: &OutputContext,
    portal: &PortalClient,
    producer: &str,
    grantee: &str,
    principal: bool,
    justification: &str,
    expires: Option<i64>,
    json: bool,
) -> anyhow::Result<()> {
    let spec = panoscope_core::portal::GrantSpec {
        producer: producer.to_string(),
        grantee_kind: if principal { "principal" } else { "org" }.to_string(),
        grantee_id: grantee.to_string(),
        justification: justification.to_string(),
        effective_to: expires,
    };
    let row = with_spinner(out, "creating grant", portal.create_grant(&spec))
        .await
        .map_err(map_access_err)?;
    if json {
        println!("{}", serde_json::to_string(&row).unwrap_or_default());
        return Ok(());
    }
    let exp = row
        .effective_to
        .map(|t| format!(", expires {t}"))
        .unwrap_or_default();
    println!(
        "✓ granted {} → {} {} (id {}, status {}{})",
        row.producer, row.grantee_kind, row.grantee_id, row.id, row.status, exp
    );
    Ok(())
}

/// `goat access revoke <id>` — end-date a grant (ADR 1031 reversibility). The
/// row is kept for audit history; a second revoke is the server's concern.
/// `goat access revoke <id> [--confirm <release-set>]`.
///
/// Revoking end-dates a grant row, which is a portal write. The read cut only
/// lands when the key service resolves the smaller audience and publishes a new
/// generation excluding the member — and a portal call returning success says
/// nothing about whether that happened. Worse, a publish can be accepted by the
/// router and silently discarded, so no error appears on the writing side either
/// `--confirm` therefore reads the bundle BACK off the fabric and
/// reports who can still decrypt, which is the only evidence that means anything.
///
/// Without `--confirm` the behaviour is unchanged, because the fabric is not
/// always reachable from where a revoke is issued.
#[allow(clippy::too_many_arguments)]
async fn revoke(
    ctx: &super::Ctx,
    out: &OutputContext,
    portal: &PortalClient,
    id: &str,
    confirm: Option<String>,
    expect_excluded: Option<String>,
    confirm_timeout_secs: u64,
    key_service_bundle: Option<String>,
    trust_domain: Option<String>,
    key_service_id: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    with_spinner(out, "revoking grant", portal.revoke_grant(id))
        .await
        .map_err(map_access_err)?;

    let Some(rs) = confirm else {
        if json {
            println!("{{\"grant\":\"{id}\",\"status\":\"revoked\",\"confirmed\":false}}");
            return Ok(());
        }
        println!("✓ revoked grant {id}");
        println!(
            "  note: the grant row is end-dated. Whether the read cut has landed on the fabric \n\
             \x20       is a separate fact — re-run with `--confirm <release-set>` to read it back."
        );
        return Ok(());
    };

    let outcome = confirm_revoke_on_fabric(
        ctx,
        &rs,
        expect_excluded.as_deref(),
        confirm_timeout_secs,
        key_service_bundle,
        trust_domain,
        key_service_id,
    )
    .await;

    match outcome {
        Ok(c) => {
            if json {
                let members = serde_json::to_string(&c.members).unwrap_or_default();
                println!(
                    "{{\"grant\":\"{id}\",\"status\":\"revoked\",\"confirmed\":true,\
                     \"release_set\":\"{rs}\",\"generation\":{},\"members\":{members}}}",
                    c.generation
                );
                return Ok(());
            }
            println!("✓ revoked grant {id}");
            println!(
                "✓ confirmed on the fabric — {rs} is now on generation {}, decryptable by {} member(s):",
                c.generation,
                c.members.len()
            );
            for m in &c.members {
                println!("    {m}");
            }
            Ok(())
        }
        Err(e) => {
            if json {
                println!(
                    "{{\"grant\":\"{id}\",\"status\":\"revoked\",\"confirmed\":false,\
                     \"release_set\":\"{rs}\",\"error\":{}}}",
                    serde_json::to_string(&e.to_string()).unwrap_or_default()
                );
            } else {
                println!("✓ revoked grant {id}");
                eprintln!("✗ NOT CONFIRMED on the fabric: {e}");
                eprintln!(
                    "  The grant row is end-dated, so the portal and every human-plane surface \n\
                     \x20 already show the member as removed. The KEY they hold has not changed, \n\
                     \x20 so they can still decrypt this stream. Check the key service is \n\
                     \x20 publishing: `goat health --domain fabric`, and its audience-drift \n\
                     \x20 metric grant_keyd_release_set_audience_drift_seconds."
                );
            }
            // Exit non-zero: an unconfirmed revoke is the case the operator most
            // needs to notice, and a zero exit is how it gets missed in a script.
            std::process::exit(3);
        }
    }
}

/// Poll `keys/<rs>/**` until the audience reflects the change, or give up.
///
/// The key service polls the grant rows on its own interval (about 30s), so a
/// revoke is not instantaneous on the fabric; the default wait is three of those.
/// Fail-closed on every path: an unreadable fabric, no bundle, or an expectation
/// that does not hold are all "not confirmed", never "probably fine".
#[allow(clippy::too_many_arguments)]
async fn confirm_revoke_on_fabric(
    ctx: &super::Ctx,
    rs: &str,
    expect_excluded: Option<&str>,
    timeout_secs: u64,
    key_service_bundle: Option<String>,
    trust_domain: Option<String>,
    key_service_id: Option<String>,
) -> anyhow::Result<grant_crypto::datapath::AudienceConfirmation> {
    use grant_crypto::datapath::{confirm_audience, AudienceExpectation};

    let trust = crate::cmd::sealed::SealTrust::from_flags(
        key_service_bundle,
        trust_domain,
        key_service_id,
    )?;
    let expectation = AudienceExpectation {
        excluded: expect_excluded
            .map(|a| vec![a.to_string()])
            .unwrap_or_default(),
        included: Vec::new(),
    };

    let router = ctx.profile.router_endpoint_label();
    let session = crate::cmd::pubsub::open_session_readonly(&ctx.profile).await?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    let result = loop {
        // Why the attempt is carried out of the match: the reason the last try
        // failed is what an operator needs when this times out. "Not confirmed"
        // alone does not distinguish an unreachable fabric from a member that is
        // still in the audience.
        let attempt =
            match crate::cmd::sealed::fetch_verified_bundles(&session, rs, &trust, &router).await {
                Ok(bundles) => match confirm_audience(rs, &bundles, &expectation) {
                    Ok(c) => break Ok(c),
                    Err(e) => format!("{e:?}"),
                },
                Err(e) => e.to_string(),
            };
        if std::time::Instant::now() >= deadline {
            break Err(anyhow::anyhow!(
                "the audience for {rs} still does not reflect the revoke after {timeout_secs}s ({attempt})"
            ));
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    };

    let _ = session.close().await;
    result
}

/// `goat access repo <policy|preview|add|remove>` — the repo-access audible layer
/// (docs/design/onboarding-github-repo-grants.md §2/§3). `add`/`remove` GET the
/// current overrides, apply the one change, and POST the full desired set back
/// (the API replaces the stored overrides with the posted set).
async fn repo(out: &OutputContext, portal: &PortalClient, op: RepoCmd) -> anyhow::Result<()> {
    match op {
        RepoCmd::Policy { json } => {
            let v = with_spinner(
                out,
                "loading repo-access policy",
                portal.repo_access_policy(),
            )
            .await
            .map_err(|e| super::vendor::append_onboard_hint(portal, map_access_err(e)))?;
            if json {
                println!("{}", serde_json::to_string(&v).unwrap_or_default());
                return Ok(());
            }
            render_repo_policy(&v);
            Ok(())
        }
        RepoCmd::Preview { id, json } => {
            let v = with_spinner(
                out,
                "computing effective repo access",
                portal.repo_access_get(&id),
            )
            .await
            .map_err(|e| super::vendor::append_onboard_hint(portal, map_repo_err(e)))?;
            if json {
                println!("{}", serde_json::to_string(&v).unwrap_or_default());
                return Ok(());
            }
            render_repo_access(&id, &v);
            Ok(())
        }
        RepoCmd::Add { id, repo } => {
            let (repo, perm) = split_repo_perm(&repo);
            // GET current → apply this add → POST the full set back.
            let cur = with_spinner(out, "loading overrides", portal.repo_access_get(&id))
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, map_repo_err(e)))?;
            let (mut add, remove) = (cur.overrides.add, cur.overrides.remove);
            // Replace an existing add for the same repo (idempotent), else append.
            add.retain(|g| !g.repo.eq_ignore_ascii_case(&repo));
            add.push(RepoGrantRef {
                repo: repo.clone(),
                permission: perm.clone(),
                owner: String::new(),
            });
            let v = with_spinner(
                out,
                "setting repo override",
                portal.repo_access_set(&id, add, remove),
            )
            .await
            .map_err(|e| super::vendor::append_onboard_hint(portal, map_repo_err(e)))?;
            println!("added override: {repo} ({perm}) on {id}\n");
            render_repo_access(&id, &v);
            Ok(())
        }
        RepoCmd::Remove { id, repo } => {
            let (repo, _) = split_repo_perm(&repo);
            let cur = with_spinner(out, "loading overrides", portal.repo_access_get(&id))
                .await
                .map_err(|e| super::vendor::append_onboard_hint(portal, map_repo_err(e)))?;
            let (mut add, mut remove) = (cur.overrides.add, cur.overrides.remove);
            // Drop any matching add, and ensure a remove is recorded so it also
            // wins over a policy grant.
            add.retain(|g| !g.repo.eq_ignore_ascii_case(&repo));
            if !remove.iter().any(|g| g.repo.eq_ignore_ascii_case(&repo)) {
                remove.push(RepoGrantRef {
                    repo: repo.clone(),
                    permission: String::new(),
                    owner: String::new(),
                });
            }
            let v = with_spinner(
                out,
                "setting repo override",
                portal.repo_access_set(&id, add, remove),
            )
            .await
            .map_err(|e| super::vendor::append_onboard_hint(portal, map_repo_err(e)))?;
            println!("removed override: {repo} on {id}\n");
            render_repo_access(&id, &v);
            Ok(())
        }
    }
}

/// Split a `owner/name:perm` (or `name`, `owner/name`) selector into (repo, perm).
/// A missing perm defaults to `pull`. The `:perm` is only split off the last
/// segment so an `owner/name` with no perm is untouched.
fn split_repo_perm(s: &str) -> (String, String) {
    match s.rsplit_once(':') {
        Some((repo, perm)) if !perm.is_empty() && !perm.contains('/') => {
            (repo.to_string(), perm.to_string())
        }
        _ => (s.to_string(), "pull".to_string()),
    }
}

fn render_repo_access(id: &str, v: &panoscope_core::portal::RepoAccessView) {
    println!("repo access — proposal {id}");
    if !v.github.is_empty() {
        println!("github:    {}", v.github);
    } else {
        println!("github:    (none captured — grants can't target a login)");
    }
    if !v.has_token {
        println!("⚠ no onboarding token configured — these grants will be SKIPPED at provision");
    }
    println!("\neffective ({}):", v.effective.len());
    if v.effective.is_empty() {
        println!("  (none)");
    }
    for g in &v.effective {
        println!(
            "  {}/{:<28} {:<10} [{}]",
            g.owner, g.repo, g.permission, g.source
        );
    }
    let ov = &v.overrides;
    if !ov.add.is_empty() || !ov.remove.is_empty() {
        println!("\noverrides:");
        for a in &ov.add {
            let perm = if a.permission.is_empty() {
                "pull"
            } else {
                &a.permission
            };
            println!("  + {} ({})", a.repo, perm);
        }
        for r in &ov.remove {
            println!("  - {}", r.repo);
        }
    }
}

fn render_repo_policy(v: &serde_json::Value) {
    if v.get("configured").and_then(|c| c.as_bool()) != Some(true) {
        println!("repo-access policy: not configured (no policy file — feature off; audibles still apply)");
        return;
    }
    let owner = v
        .get("default_owner")
        .and_then(|o| o.as_str())
        .unwrap_or("");
    println!("repo-access policy — default owner: {owner}");
    let empty = vec![];
    let rules = v.get("rules").and_then(|r| r.as_array()).unwrap_or(&empty);
    println!("\nrules ({}):", rules.len());
    for rule in rules {
        let name = rule.get("Name").and_then(|n| n.as_str()).unwrap_or("");
        println!("  {name}");
        if let Some(grants) = rule.get("Grant").and_then(|g| g.as_array()) {
            for g in grants {
                let repo = g.get("Repo").and_then(|r| r.as_str()).unwrap_or("");
                let perm = g.get("Permission").and_then(|p| p.as_str()).unwrap_or("");
                println!("    → {repo} ({perm})");
            }
        }
    }
}

/// Map repo-access portal errors (404 = unknown/out-of-scope proposal, 409 =
/// already onboarded) to operator-friendly messages.
fn map_repo_err(e: PortalError) -> anyhow::Error {
    match e {
        PortalError::NotFound { .. } => anyhow::anyhow!(
            "proposal not found (or outside your sponsorship scope) — `goat admin queue ls --status all`"
        ),
        PortalError::Http { status, .. } if status.as_u16() == 409 => anyhow::anyhow!(
            "this person is already onboarded (409): a repo can be added, but one cannot be removed here; remove them as a collaborator on GitHub"
        ),
        other => map_access_err(other),
    }
}

/// Resolve the `me` selector to the caller's own principal via the
/// portal's `GET /api/whoami` (the self tier the design doc promises —
/// `access who me` / `why me <key>`). Preference order mirrors what the
/// access graph can match: the org prefix (vendor/org keys), then the
/// live slot alias, then the per-person operator actor handle.
/// Fail-closed with a named reason — never a guessed identity.
async fn resolve_me(
    out: &OutputContext,
    portal: &PortalClient,
    id: &str,
) -> anyhow::Result<String> {
    if id != "me" {
        return Ok(id.to_string());
    }
    let w = with_spinner(out, "resolving `me` via /api/whoami", portal.whoami())
        .await
        .map_err(|e| anyhow::anyhow!("could not resolve `me` (portal whoami): {e}"))?;
    let resolved = if !w.vendor.is_empty() {
        w.vendor
    } else if !w.alias.is_empty() {
        w.alias
    } else if w.actor == "operator" {
        // The legacy SHARED operator key (pre-F-207): a pool, not a
        // person — nothing in the access graph to resolve to. Name it.
        anyhow::bail!(
            "your context holds the shared `operator` key — a pool, not an \
             identity, so `me` cannot resolve. Use a per-person operator key \
             or pass an explicit selector (org prefix, email, peer \
             name, or slot id)"
        );
    } else if !w.actor.is_empty() {
        w.actor
    } else {
        anyhow::bail!(
            "your key resolved to no queryable principal (operator pool key \
             with no actor handle) — pass an explicit selector: an org prefix, \
             person email, peer name, or slot id"
        );
    };
    // stderr so `--json` stdout stays parseable.
    let _ = out;
    eprintln!("`me` = {resolved} (via /api/whoami)");
    Ok(resolved)
}

async fn probation(
    out: &OutputContext,
    portal: &PortalClient,
    op: ProbationCmd,
) -> anyhow::Result<()> {
    match op {
        ProbationCmd::List { all, json } => {
            let rows = with_spinner(out, "loading probation", portal.probation_list(all))
                .await
                .map_err(map_access_err)?;
            if json {
                println!("{}", probation_json(&rows));
                return Ok(());
            }
            render_probation(&rows, all);
            Ok(())
        }
        ProbationCmd::Hold {
            ref_,
            reason,
            label,
            kind,
            cure_days,
            json,
        } => {
            let req = ProbationHold {
                ref_: ref_.clone(),
                kind,
                label,
                reason,
                cure_by: cure_days.map(cure_by_from_days).unwrap_or(0),
                note: String::new(),
            };
            let held = with_spinner(out, "holding on probation", portal.probation_hold(&req))
                .await
                .map_err(map_probation_err)?;
            if json {
                println!("{}", probation_json(std::slice::from_ref(&held)));
                return Ok(());
            }
            println!("held on probation: {} ({})", held.ref_, held.id);
            println!(
                "  cure by {} — verify, then `decide`; expiry only flags, never cuts",
                fmt_unix(held.cure_by)
            );
            Ok(())
        }
        ProbationCmd::Decide { id, verdict, note } => {
            let disposition = match verdict.as_str() {
                "repair" => "repaired",
                "purge" => "purged",
                other => other,
            };
            let r = with_spinner(
                out,
                "recording decision",
                portal.probation_decide(&id, disposition, &note),
            )
            .await
            .map_err(map_probation_err)?;
            println!("recorded: {} → {}", id, r.disposition);
            if !r.next.is_empty() {
                println!("\nnow run the actual {}:", verdict);
                println!("  {}", r.next);
            }
            Ok(())
        }
    }
}

/// Seconds-since-epoch for `now + days`, for the `--cure-days` override.
fn cure_by_from_days(days: i64) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now + days * 86400
}

/// Absolute date for a unix timestamp (`unknown` when 0/unparseable).
fn fmt_unix(unix: i64) -> String {
    if unix <= 0 {
        return "unknown".to_string();
    }
    match chrono::DateTime::from_timestamp(unix, 0) {
        Some(t) => t.format("%Y-%m-%d").to_string(),
        None => "unknown".to_string(),
    }
}

fn render_probation(rows: &[Probation], all: bool) {
    let scope = if all { "all" } else { "open" };
    println!("PROBATION — {} {} held\n", rows.len(), scope);
    if rows.is_empty() {
        println!("  (none)");
        return;
    }
    for p in rows {
        let flag = if p.expired {
            " ⚠ EXPIRED — verify + decide"
        } else {
            ""
        };
        let state = if p.disposition == "probation" {
            format!("cure-by {}{}", fmt_unix(p.cure_by), flag)
        } else {
            format!("{} by {}", p.disposition, p.decided_by)
        };
        let label = if p.label.is_empty() {
            &p.ref_
        } else {
            &p.label
        };
        println!(
            "  {:<30} {:<24} {:<20} {}",
            trunc(label, 30),
            p.id,
            p.reason,
            state
        );
    }
    println!("\ndecide: goat access probation decide <id> repair|purge");
}

fn probation_json(rows: &[Probation]) -> String {
    let v = rows
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id, "ref": p.ref_, "kind": p.kind, "label": p.label,
                "reason": p.reason, "held_at": p.held_at, "held_by": p.held_by,
                "cure_by": p.cure_by, "disposition": p.disposition,
                "decided_by": p.decided_by, "expired": p.expired,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({ "probation": v })).unwrap_or_default()
}

/// Map probation-specific portal errors to operator-friendly messages (409 =
/// already held / already decided).
fn map_probation_err(e: PortalError) -> anyhow::Error {
    match e {
        PortalError::Http { status, .. } if status.as_u16() == 409 => anyhow::anyhow!(
            "already held or already decided (409) — `goat access probation list --all` to see it"
        ),
        other => map_access_err(other),
    }
}

/// Whether a stray's reconcile reason is the VERIFY-first class (must not be
/// blind-cut): `unregistered-key`, or any reason we don't recognize as a clean
/// cut candidate. `untracked` + `no-declared-member` are cut candidates.
fn is_verify_class(reason: &str) -> bool {
    !matches!(reason, "untracked" | "no-declared-member")
}

/// Human age of a unix `last_seen` (0/None = unknown). Corroboration signal:
/// a stale last-seen strengthens a cut candidate.
fn last_seen_age(unix: i64) -> String {
    if unix <= 0 {
        return "unknown".to_string();
    }
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(now) => {
            let secs = now.as_secs() as i64 - unix;
            if secs < 0 {
                "just now".to_string()
            } else if secs < 3600 {
                format!("{}m", secs / 60)
            } else if secs < 86400 {
                format!("{}h", secs / 3600)
            } else {
                format!("{}d", secs / 86400)
            }
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Human "3d ago" for an RFC-3339 `generated_at`. `None` if it doesn't parse
/// (a portal predating the stamp, or a malformed field) — the caller then omits
/// the age rather than inventing one.
fn iso_age(generated_at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(generated_at).ok()?;
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds();
    Some(if secs < 0 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    })
}

/// The `source_digest` of THIS GOATNET's roster in the caller's checkout —
/// `docs/operators/<goatnet>/roster.json` — found by walking up from CWD.
/// `None` when `goat` runs outside the repo (the common case for an on-mesh
/// operator), or when the context names no goatnet — the caller then degrades
/// to a generic freshness note instead of a checkout-vs-portal verdict.
///
/// It must be that goatnet's artifact and not `docs/operators/roster.json`.
/// Since ADR 1172 each portal is served its own roster, so comparing against
/// the controller-side union would report drift on every run — a warning that
/// is always on is a warning nobody reads.
fn local_roster_digest(goatnet: Option<&str>) -> Option<String> {
    let goatnet = goatnet?;
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(format!("docs/operators/{goatnet}/roster.json"));
        if candidate.is_file() {
            let raw = std::fs::read_to_string(&candidate).ok()?;
            let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
            return v
                .get("source_digest")
                .and_then(|d| d.as_str())
                .map(String::from);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Turn the meaningless blanket "strays are suspect — regenerate" caveat into a
/// concrete freshness verdict: how old the served roster is, and — when `goat`
/// runs inside the repo — whether the portal is serving the same roster your
/// checkout holds (the drift the caveat was gesturing at). Honest degradation:
/// no stamp → say so; no local checkout → give the command instead of a verdict.
fn freshness_report(fr: Option<&RosterFreshness>, goatnet: Option<&str>) -> Vec<String> {
    let Some(fr) = fr else {
        return vec![
            "  roster: freshness unknown — the portal predates the provenance stamp.".to_string(),
            "  before acting, regenerate + redeploy: python3 ops/operators/gen-roster.py"
                .to_string(),
        ];
    };

    let digest8 = |d: &str| d.chars().take(8).collect::<String>();
    let mut head = String::from("  roster:");
    if !fr.generated_at.is_empty() {
        head.push_str(&format!(" generated {}", fr.generated_at));
        if let Some(age) = iso_age(&fr.generated_at) {
            head.push_str(&format!(" ({age})"));
        }
    }
    if fr.member_count > 0 {
        head.push_str(&format!(" · {} members", fr.member_count));
    }
    if !fr.source_digest.is_empty() {
        head.push_str(&format!(" · source {}", digest8(&fr.source_digest)));
    }

    let mut lines = vec![head];
    match (local_roster_digest(goatnet), fr.source_digest.is_empty()) {
        (Some(local), false) if local == fr.source_digest => {
            lines.push(
                "  ✓ the portal is serving the roster in your checkout — strays reflect these manifests"
                    .to_string(),
            );
        }
        (Some(local), false) => {
            lines.push(format!(
                "  ⚠ portal roster ≠ your checkout (portal {}, local {}) — one is behind:",
                digest8(&fr.source_digest),
                digest8(&local)
            ));
            lines.push(
                "    if you regenerated + committed, redeploy the portal; else pull/regenerate first."
                    .to_string(),
            );
        }
        _ => {
            lines.push(format!(
                "  to confirm it's current: run `python3 ops/operators/gen-roster.py{} --check`",
                goatnet.map(|g| format!(" --env {g}")).unwrap_or_default()
            ));
            lines.push(
                "  in the repo, then redeploy the portal if that goatnet's roster.json changed."
                    .to_string(),
            );
        }
    }
    lines
}

async fn strays(
    out: &OutputContext,
    portal: &PortalClient,
    json: bool,
    goatnet: Option<&str>,
) -> anyhow::Result<()> {
    // The roster call fans out to NetBird/Ory/Zenoh server-side and can take
    // 30–60s on a lossy mesh — spin so slow ≠ hung (the operator report).
    let roster = with_spinner(out, "collecting access roster", portal.access_roster())
        .await
        .map_err(map_access_err)?;
    // Freeze the stray set: every row the graph reconciled to out_of_band.
    let mut cut: Vec<&panoscope_core::portal::AccessRosterEntry> = Vec::new();
    let mut verify: Vec<&panoscope_core::portal::AccessRosterEntry> = Vec::new();
    for r in &roster.rows {
        if r.state != "out_of_band" {
            continue;
        }
        if is_verify_class(&r.recon_reason) {
            verify.push(r);
        } else {
            cut.push(r);
        }
    }

    // Cross-reference the probation list so a stray already being held shows it
    // inline (the two commands compose). Best-effort — if the probation read
    // fails, the sweep still works; absence of probation data never breaks it.
    let held: std::collections::HashMap<String, Probation> = portal
        .probation_list(false)
        .await
        .map(|rows| rows.into_iter().map(|p| (p.ref_.clone(), p)).collect())
        .unwrap_or_default();

    if json {
        let map = |v: &[&panoscope_core::portal::AccessRosterEntry]| {
            v.iter()
                .map(|r| {
                    // A row with no attributable device carries NO mesh verdict:
                    // the portal never looked, so false/0 would read as "known
                    // absent" while the row may be connected. Null says "not known".
                    let known = r.on_mesh;
                    serde_json::json!({
                        "id": r.id, "label": r.label, "kind": r.kind,
                        "recon_reason": r.recon_reason,
                        "mesh_attribution": r.mesh_attribution,
                        "last_seen_unix": known.then_some(r.last_seen_unix),
                        "on_mesh": known.then_some(true),
                        "connected": known.then_some(r.connected),
                        "on_probation": held.contains_key(&r.id),
                    })
                })
                .collect::<Vec<_>>()
        };
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "cut_candidates": map(&cut),
                "verify_first": map(&verify),
                "freshness": roster.freshness.as_ref().map(|fr| serde_json::json!({
                    "generated_at": fr.generated_at, "source_digest": fr.source_digest,
                    "member_count": fr.member_count, "loaded_unix": fr.loaded_unix,
                })),
                "notes": roster.notes,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }

    let total = cut.len() + verify.len();
    println!("STRAYS — {total} live logins with no onboarding record (no live slot recorded to");
    println!("  the login, and no roster entry)\n");
    println!("  A row's mesh column only reports a device the portal could attribute to the");
    println!("  person; \"device unattributed\" means it did not look, not that there is none.\n");
    for line in freshness_report(roster.freshness.as_ref(), goatnet) {
        println!("{line}");
    }
    println!();

    println!(
        "CUT CANDIDATES ({}) — untracked / offboarding residue; corroborate (stale last-seen, no",
        cut.len()
    );
    println!("  live cert) before cutting:");
    if cut.is_empty() {
        println!("  (none)");
    }
    for r in &cut {
        println!(
            "  {:<34} {:<18} last-seen {:<8} {}{}",
            trunc(&r.label, 34),
            r.recon_reason,
            last_seen_age(r.last_seen_unix),
            mesh_cell(r),
            probation_marker(&held, &r.id)
        );
    }

    println!(
        "\nVERIFY FIRST ({}) — carries an org key that matches no declared org and holds no",
        verify.len()
    );
    println!(
        "  slot the portal onboarded; do NOT blind-cut — fix the key or confirm the org left:"
    );
    if verify.is_empty() {
        println!("  (none)");
    }
    for r in &verify {
        println!(
            "  {:<34} {:<18} last-seen {:<8} {}{}",
            trunc(&r.label, 34),
            if r.recon_reason.is_empty() {
                "unclassified"
            } else {
                &r.recon_reason
            },
            last_seen_age(r.last_seen_unix),
            mesh_cell(r),
            probation_marker(&held, &r.id)
        );
    }

    if !held.is_empty() {
        println!(
            "\n{} of these are already on probation (held, not cut).",
            held.len()
        );
    }
    println!(
        "\nNext: hold undecided ones (`goat access probation hold <id>`), or cut one at a time:"
    );
    println!("  goat admin onboard --revoke <id>");
    print_notes(&roster.notes);
    Ok(())
}

/// The mesh column of a stray row. `on_mesh` is only ever a positive finding:
/// the portal attributes a device to a person by peer name, and a row it could
/// not attribute carries false without any lookup behind it.
fn mesh_cell(r: &panoscope_core::portal::AccessRosterEntry) -> &'static str {
    if r.on_mesh {
        "on-mesh"
    } else {
        "device unattributed"
    }
}

/// A probation marker for a stray row whose `id` is currently held. Empty when
/// the row isn't on probation. Expired holds are flagged (the deadline flags,
/// never cuts — ADR 1068).
fn probation_marker(held: &std::collections::HashMap<String, Probation>, id: &str) -> String {
    match held.get(id) {
        Some(p) if p.expired => format!(" · PROBATION ⚠ expired {}", fmt_unix(p.cure_by)),
        Some(p) => format!(" · PROBATION cure-by {}", fmt_unix(p.cure_by)),
        None => String::new(),
    }
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

async fn who(
    out: &OutputContext,
    portal: &PortalClient,
    id: &str,
    json: bool,
) -> anyhow::Result<()> {
    let r: AccessWho = with_spinner(out, "querying access", portal.access_who(id))
        .await
        .map_err(map_access_err)?;
    // Custody fold-in (ADR 1096): the machines this identity has enrolled. Purely
    // additive to the identity COP and BEST-EFFORT — a cross-custodian query (a
    // non-operator asking about someone else, which the server 403s) or a portal
    // without the endpoint degrades to no custody section, never a hard failure.
    let custody = if r.resolved.is_empty() {
        None
    } else {
        portal.list_machines(Some(&r.resolved)).await.ok()
    };
    if json {
        let machines_json = custody
            .as_ref()
            .map(|ml| {
                ml.machines.iter().map(|m| serde_json::json!({
                "handle": m.system_handle, "node_class": m.node_class,
                "autonomy": m.autonomy, "affiliation": m.affiliation, "slot_id": m.slot_id,
            })).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        println!("{}", serde_json::to_string(&serde_json::json!({
            "query": r.query, "resolved": r.resolved, "kind": r.kind, "vendor": r.vendor,
            "affiliation": r.affiliation, "nation": r.nation,
            "state": r.state, "on_mesh": r.on_mesh, "mesh_attribution": r.mesh_attribution,
            "writes": r.writes.iter().map(|a| serde_json::json!({"topic": a.topic, "layer": a.layer, "reason": a.reason})).collect::<Vec<_>>(),
            "reads": r.reads.iter().map(|a| serde_json::json!({"topic": a.topic, "layer": a.layer, "reason": a.reason})).collect::<Vec<_>>(),
            "custody": machines_json,
            "notes": r.notes,
        })).unwrap_or_default());
        return Ok(());
    }
    if r.resolved.is_empty() {
        println!("no identity matched `{id}`");
        print_notes(&r.notes);
        return Ok(());
    }
    println!("identity:  {} ({})", r.resolved, r.kind);
    if !r.vendor.is_empty() {
        println!("org:       {}", r.vendor);
    }
    if !r.affiliation.is_empty() {
        let nation = if r.nation.is_empty() {
            String::new()
        } else {
            format!(" ({})", r.nation)
        };
        println!("affiliation: {}{nation}", r.affiliation);
    }
    if !r.state.is_empty() {
        println!("state:     {}", r.state);
    }
    // Honesty: "no" is only honest when the graph could have known. A person
    // with no attributable device (mesh_attribution=none) gets "unattributed"
    // — a device enrolled under a non-conventional name is invisible to the
    // peer-name custody link, which is not the same as being off-mesh.
    let mesh_line = match (r.on_mesh, r.mesh_attribution.as_str()) {
        (true, "peer") => "yes (own device)",
        (true, "org") => "yes (via org peers)",
        (true, _) => "yes",
        (false, "none") => "unattributed (no enrolled device matches this identity)",
        (false, _) => "no",
    };
    println!("on mesh:   {mesh_line}");
    // Custody (ADR 1096): machines enrolled under this identity, if any.
    if let Some(ml) = custody.as_ref() {
        if !ml.machines.is_empty() {
            let n = ml.machines.len();
            println!("\ncustody ({n} machine{}):", if n == 1 { "" } else { "s" });
            for m in &ml.machines {
                println!(
                    "  {}   {} / {}   ({})",
                    m.system_handle, m.node_class, m.autonomy, m.affiliation
                );
            }
            if let Some(q) = &ml.quota {
                println!(
                    "  — quota: {}/{} used, {} remaining",
                    q.used, q.quota, q.remaining
                );
            }
        }
    }
    println!("\nmay write ({}):", r.writes.len());
    if r.writes.is_empty() {
        println!("  (nothing — no rendered write rule)");
    }
    for a in &r.writes {
        println!("  {}   [{}] {}", a.topic, a.layer, a.reason);
    }
    println!("\nmay read ({}):", r.reads.len());
    if r.reads.is_empty() {
        println!("  (no explicit read-tier edges — see notes)");
    }
    for a in &r.reads {
        println!("  {}   [{}] {}", a.topic, a.layer, a.reason);
    }
    print_notes(&r.notes);
    Ok(())
}

async fn topic(
    out: &OutputContext,
    portal: &PortalClient,
    key: &str,
    json: bool,
) -> anyhow::Result<()> {
    let r: AccessTopic = with_spinner(out, "querying topic access", portal.access_topic(key))
        .await
        .map_err(map_access_err)?;
    if json {
        println!("{}", serde_json::to_string(&serde_json::json!({
            "topic": r.topic, "found": r.found,
            "writers": r.writers.iter().map(|w| serde_json::json!({"identity": w.identity, "kind": w.kind, "layer": w.layer, "reason": w.reason})).collect::<Vec<_>>(),
            "readers": r.readers.iter().map(|w| serde_json::json!({"identity": w.identity, "layer": w.layer, "reason": w.reason})).collect::<Vec<_>>(),
            "receivers": r.receivers, "notes": r.notes,
        })).unwrap_or_default());
        return Ok(());
    }
    println!("topic:   {}", r.topic);
    if !r.found {
        println!("  (not registered)");
        print_notes(&r.notes);
        return Ok(());
    }
    println!("\nwriters ({}):", r.writers.len());
    if r.writers.is_empty() {
        println!("  (none — a publish here is silently denied at the router, F-177)");
    }
    for w in &r.writers {
        println!("  {} ({})   [{}] {}", w.identity, w.kind, w.layer, w.reason);
    }
    println!("\nreaders ({}):", r.readers.len());
    for w in &r.readers {
        println!("  {}   [{}] {}", w.identity, w.layer, w.reason);
    }
    if !r.receivers.is_empty() {
        println!("\nrelease receivers: {}", r.receivers.join(", "));
    }
    print_notes(&r.notes);
    Ok(())
}

async fn why(
    out: &OutputContext,
    portal: &PortalClient,
    id: &str,
    key: &str,
    json: bool,
) -> anyhow::Result<()> {
    let r: AccessWhy = with_spinner(out, "tracing access", portal.access_why(id, key))
        .await
        .map_err(map_access_err)?;
    if json {
        println!("{}", serde_json::to_string(&serde_json::json!({
            "query": r.query, "topic": r.topic, "resolved": r.resolved, "vendor": r.vendor,
            "write_verdict": r.write_verdict, "read_verdict": r.read_verdict,
            "steps": r.steps.iter().map(|s| serde_json::json!({"layer": s.layer, "verdict": s.verdict, "reason": s.reason})).collect::<Vec<_>>(),
            "notes": r.notes,
        })).unwrap_or_default());
        return Ok(());
    }
    if r.resolved.is_empty() {
        println!("no identity matched `{id}`");
        print_notes(&r.notes);
        return Ok(());
    }
    println!("can `{}` access `{}`?", id, key);
    println!("  write: {}", verdict_glyph(&r.write_verdict));
    println!("  read:  {}", verdict_glyph(&r.read_verdict));
    println!("\ntrace:");
    for s in &r.steps {
        println!("  [{:<8}] {:<5}  {}", s.layer, s.verdict, s.reason);
    }
    print_notes(&r.notes);
    Ok(())
}

fn verdict_glyph(v: &str) -> String {
    match v {
        "allow" => "allow ✓".to_string(),
        "deny" => "DENY ✗".to_string(),
        other => other.to_string(),
    }
}

fn print_notes(notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    eprintln!("\nnotes:");
    for n in notes {
        eprintln!("  • {n}");
    }
}

/// Map the portal error to an operator-friendly message. The COP's 403 is
/// the self-scope boundary — name it so an org knows to query their own
/// prefix instead of guessing.
fn map_access_err(e: PortalError) -> anyhow::Error {
    match e {
        PortalError::Http { status, .. } if status.as_u16() == 403 => anyhow::anyhow!(
            "access denied (403): cross-identity queries need an operator token; \
             as an org you can query your own prefix — `goat access who <your-prefix>`"
        ),
        other => super::vendor::map_portal_err(other),
    }
}

#[cfg(test)]
static GOAT_ONBOARD_ENDPOINT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) struct OnboardEndpointEnvGuard {
    prev: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl OnboardEndpointEnvGuard {
    pub(crate) fn set_for_test(value: Option<&str>) -> Self {
        let lock = GOAT_ONBOARD_ENDPOINT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let prev = std::env::var("GOAT_ONBOARD_ENDPOINT").ok();

        match value {
            Some(v) => std::env::set_var("GOAT_ONBOARD_ENDPOINT", v),
            None => std::env::remove_var("GOAT_ONBOARD_ENDPOINT"),
        }

        OnboardEndpointEnvGuard { prev, _lock: lock }
    }
}

#[cfg(test)]
impl Drop for OnboardEndpointEnvGuard {
    fn drop(&mut self) {
        if let Some(prev) = &self.prev {
            std::env::set_var("GOAT_ONBOARD_ENDPOINT", prev);
        } else {
            std::env::remove_var("GOAT_ONBOARD_ENDPOINT");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_cell_never_reports_absence() {
        let mut r = panoscope_core::portal::AccessRosterEntry {
            mesh_attribution: "none".into(),
            ..Default::default()
        };
        assert_eq!(mesh_cell(&r), "device unattributed");
        r.on_mesh = true;
        r.mesh_attribution = "peer".into();
        assert_eq!(mesh_cell(&r), "on-mesh");
    }

    #[test]
    fn verdict_glyph_renders() {
        assert!(verdict_glyph("allow").contains("allow"));
        assert!(verdict_glyph("deny").contains("DENY"));
        assert_eq!(verdict_glyph("n/a"), "n/a");
    }

    #[test]
    fn http_403_maps_to_self_scope_hint() {
        let err = map_access_err(PortalError::status(reqwest::StatusCode::FORBIDDEN));
        let s = err.to_string();
        assert!(s.contains("operator") && s.contains("your own prefix"));
    }

    #[test]
    fn stray_bucketing_is_fail_safe() {
        // Cut candidates: only the two recognized clean-cut reasons.
        assert!(!is_verify_class("untracked"));
        assert!(!is_verify_class("no-declared-member"));
        // Verify-first: the reconcilable class AND — crucially — anything we
        // can't classify (empty reason from an un-upgraded portal) fails safe
        // to verify, never to a blind cut.
        assert!(is_verify_class("unregistered-key"));
        assert!(is_verify_class(""));
        assert!(is_verify_class("something-new"));
    }

    #[test]
    fn last_seen_age_handles_unknown() {
        assert_eq!(last_seen_age(0), "unknown");
        assert_eq!(last_seen_age(-1), "unknown");
    }

    #[test]
    fn freshness_report_none_is_honest_about_missing_stamp() {
        let lines = freshness_report(None, None);
        assert!(lines[0].contains("freshness unknown"));
        // Never a fabricated verdict; it gives the remediation command instead.
        assert!(lines.iter().any(|l| l.contains("gen-roster.py")));
    }

    #[test]
    fn freshness_report_renders_age_and_digest() {
        let fr = RosterFreshness {
            generated_at: "2026-06-28T14:00:00Z".to_string(),
            source_digest: "28de1d5f5b7e3c70e8119a47".to_string(),
            member_count: 270,
            loaded_unix: 0,
        };
        let head = &freshness_report(Some(&fr), None)[0];
        assert!(head.contains("270 members"));
        assert!(head.contains("source 28de1d5f")); // truncated to 8
        assert!(head.contains("generated 2026-06-28T14:00:00Z"));
    }

    #[test]
    fn iso_age_rejects_garbage() {
        assert!(iso_age("not-a-date").is_none());
        assert!(iso_age("").is_none());
        assert!(iso_age("2026-06-28T14:00:00Z").is_some());
    }

    #[test]
    fn fmt_unix_handles_unknown_and_dates() {
        assert_eq!(fmt_unix(0), "unknown");
        assert_eq!(fmt_unix(-5), "unknown");
        // A real timestamp renders a YYYY-MM-DD date, not "unknown".
        let d = fmt_unix(1_781_000_000);
        assert_ne!(d, "unknown");
        assert_eq!(d.len(), 10);
        assert_eq!(d.matches('-').count(), 2);
    }

    #[test]
    fn cure_by_from_days_is_in_the_future() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(cure_by_from_days(14) >= now + 14 * 86400 - 5);
    }

    #[test]
    fn probation_json_maps_ref_field() {
        let p = Probation {
            id: "prob-1".into(),
            ref_: "kid-x".into(),
            reason: "stray:untracked".into(),
            disposition: "probation".into(),
            ..Default::default()
        };
        let s = probation_json(std::slice::from_ref(&p));
        // The wire key is `ref`, not `ref_` (the Rust-keyword workaround).
        assert!(s.contains("\"ref\":\"kid-x\""));
        assert!(!s.contains("ref_"));
        assert!(s.contains("stray:untracked"));
    }

    #[test]
    fn probation_marker_reflects_hold_state() {
        use std::collections::HashMap;
        let mut held: HashMap<String, Probation> = HashMap::new();
        // Not held → empty marker.
        assert_eq!(probation_marker(&held, "kid-x"), "");
        // Held, open → cure-by marker.
        held.insert(
            "kid-x".into(),
            Probation {
                ref_: "kid-x".into(),
                cure_by: 1_781_000_000,
                disposition: "probation".into(),
                ..Default::default()
            },
        );
        assert!(probation_marker(&held, "kid-x").contains("PROBATION cure-by"));
        // Held + expired → flagged, never silently dropped.
        held.insert(
            "kid-y".into(),
            Probation {
                ref_: "kid-y".into(),
                cure_by: 1_700_000_000,
                disposition: "probation".into(),
                expired: true,
                ..Default::default()
            },
        );
        assert!(probation_marker(&held, "kid-y").contains("expired"));
    }

    #[test]
    fn split_repo_perm_parses() {
        // owner/name → default pull.
        assert_eq!(
            split_repo_perm("acme/thing"),
            ("acme/thing".into(), "pull".into())
        );
        // owner/name:perm → split off the perm.
        assert_eq!(
            split_repo_perm("acme/thing:push"),
            ("acme/thing".into(), "push".into())
        );
        // bare name → default pull.
        assert_eq!(split_repo_perm("thing"), ("thing".into(), "pull".into()));
        // bare name:perm.
        assert_eq!(
            split_repo_perm("thing:admin"),
            ("thing".into(), "admin".into())
        );
    }

    #[test]
    fn repo_404_maps_to_scope_hint() {
        let err = map_repo_err(PortalError::NotFound { path: String::new() });
        assert!(err.to_string().contains("proposal not found"));
    }

    #[test]
    fn repo_409_maps_to_onboarded_hint() {
        let err = map_repo_err(PortalError::status(reqwest::StatusCode::CONFLICT));
        assert!(err.to_string().contains("already onboarded"));
    }

    #[test]
    fn onboard_endpoint_hint_present_on_fallback_repo_access() {
        let portal = PortalClient::new("https://portal.example".to_string(), "k".to_string());
        let base = map_repo_err(PortalError::NotFound { path: String::new() });
        let err = crate::cmd::vendor::append_onboard_hint(&portal, base);
        let rendered = err.to_string();
        assert!(rendered.contains("proposal not found"));
        assert!(rendered.contains("GOAT_ONBOARD_ENDPOINT"));
    }

    #[test]
    fn onboard_endpoint_hint_absent_when_configured_repo_access() {
        let portal = PortalClient::new("https://portal.example".to_string(), "k".to_string())
            .with_onboard_endpoint("https://onboard.example");
        let base = map_repo_err(PortalError::NotFound { path: String::new() });
        let err = crate::cmd::vendor::append_onboard_hint(&portal, base);
        let rendered = err.to_string();
        assert!(rendered.contains("proposal not found"));
        assert!(!rendered.contains("GOAT_ONBOARD_ENDPOINT"));
    }

    #[test]
    fn probation_409_maps_to_already_held() {
        let err = map_probation_err(PortalError::status(reqwest::StatusCode::CONFLICT));
        assert!(err.to_string().contains("already held"));
    }

    #[tokio::test]
    async fn repo_policy_onboard_endpoint_hint_added_on_fallback() {
        use crate::output::{ColorChoice, OutputContext};
        use tempfile::TempDir;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/repo-access/policy"))
            .respond_with(ResponseTemplate::new(500))
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
        let _guard = super::OnboardEndpointEnvGuard::set_for_test(None);

        let ctx = crate::cmd::Ctx {
            profile,
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };

        let err = crate::cmd::access::run(&ctx, AccessCmd::Repo(RepoCmd::Policy { json: false }))
            .await
            .expect_err("HTTP error must surface from repo policy");

        // Mirror main.rs's report_anyhow: GoatError → DisplayHuman, else {err:#}.
        let rendered = if let Some(g) = err.downcast_ref::<crate::errors::GoatError>() {
            format!("{}", crate::errors::DisplayHuman(g))
        } else {
            format!("{err:#}")
        };
        assert!(
            rendered.contains("HTTP 500") && rendered.contains("E_PORTAL_SERVER_ERROR"),
            "must include underlying portal error: {rendered}"
        );
        assert!(
            rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "must include onboard-endpoint configuration hint when falling back: {rendered}"
        );
    }

    #[tokio::test]
    async fn repo_policy_onboard_endpoint_hint_suppressed_when_configured() {
        use crate::output::{ColorChoice, OutputContext};
        use tempfile::TempDir;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/onboard/repo-access/policy"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let profile = crate::profile::Profile {
            dir: tmp.path().join("profile"),
            context: "default".into(),
        };
        std::fs::create_dir_all(profile.context_dir()).unwrap();
        let toml_body = format!(
            "portal_endpoint = \"{url}\"\nonboard_endpoint = \"{url}\"\n",
            url = server.uri()
        );
        std::fs::write(profile.profile_toml_path(), toml_body).unwrap();
        std::fs::write(profile.portal_token_path(), "test-token").unwrap();

        let ctx = crate::cmd::Ctx {
            profile,
            output: OutputContext::from_env(ColorChoice::Never),
            narrate: false,
            narrate_json: false,
            uuid: false,
        };

        let err = crate::cmd::access::run(&ctx, AccessCmd::Repo(RepoCmd::Policy { json: false }))
            .await
            .expect_err("HTTP error must surface from repo policy");

        let rendered = if let Some(g) = err.downcast_ref::<crate::errors::GoatError>() {
            format!("{}", crate::errors::DisplayHuman(g))
        } else {
            format!("{err:#}")
        };
        assert!(
            rendered.contains("HTTP 500") && rendered.contains("E_PORTAL_SERVER_ERROR"),
            "must include underlying portal error: {rendered}"
        );
        assert!(
            !rendered.contains("GOAT_ONBOARD_ENDPOINT"),
            "must NOT include onboard-endpoint hint when override is configured: {rendered}"
        );
    }
}

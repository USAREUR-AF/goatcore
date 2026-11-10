// `goat machine` — ADR 1096 stage 2b: the CLI face of delegated machine
// enrollment (docs/design/machine-enrollment.md). Thin client over the portal
// `POST /api/machines/enroll`: a custodian mints one machine (system principal)
// against their standing enrollment_grant. The SERVER applies the
// infra-vs-self-service gate and enforces the quota; this wrapper only builds the
// request and renders the returned bundle. The person-flow shape (goat CLI →
// portal) mirrors `goat admin onboard`.

use crate::output::{with_spinner, OutputContext};
use clap::Subcommand;
use panoscope_core::portal::{
    MachineBulkEnrollSpec, MachineBulkSpec, MachineEnrollSpec, MachineGrantSpec, MachineListItem,
    PortalClient, PortalError,
};

#[derive(Subcommand, Debug)]
pub enum MachineCmd {
    /// Preview or repair verified legacy affiliation spellings. Operator-only.
    Affiliation {
        #[command(subcommand)]
        op: super::machine_affiliation::AffiliationCmd,
    },
    /// Enroll one machine (system principal) against a custodian's grant: mints
    /// its OWN slot + cert + api_key and records the person→system custody edge.
    /// Gear node-classes (ADR 1116) are custodian self-service; infrastructure
    /// classes are operator-only (the server enforces both). With `--bulk <file>`, N
    /// machines under ONE custodian in one act.
    Enroll {
        /// The machine's handle — its slot alias (lowercase `[a-z0-9-]`).
        /// Omitted in `--bulk` mode (each spec carries its own).
        #[arg(required_unless_present = "bulk", conflicts_with = "bulk")]
        handle: Option<String>,
        /// Node-class — the function the box performs (ADR 1116), or a
        /// standalone appliance boundary; enroll once per function. Custodian self-service: SENSOR-FEED /
        /// SENSOR-TELEMETRY / SENSOR-ARBITER, EFFECTOR-CUE / EFFECTOR-TRACK /
        /// EFFECTOR-TELEMETRY / EFFECTOR-ARBITER, POD / MOON / KID /
        /// EXTERNAL-KID / EXTERNAL-INTERMEDIARY, FUSION, CONTROL; or PLATFORM, a composition
        /// that needs `--workloads`. NETWORK (bearer gear — routers,
        /// switches, radios) and EDGE-SERVER (a server that supports edge
        /// devices): operator, or a custodian holding the network-admin
        /// entitlement for its own site (ADR 1113, ADR 1216). Operator-only
        /// infrastructure: MGMT-HOST / ZENOH / SNITCH / SERVICES / BREAKGLASS /
        /// NETWORK-JUMP. Refused at the mint: SENSOR and EFFECTOR (reserved),
        /// SENSOR-TASKING and EFFECTOR-COMMAND (intake-only, never a primary
        /// class), PROCESSOR (legacy, no mesh group — use FUSION).
        /// Case-insensitive; normalized to uppercase.
        #[arg(long, required_unless_present = "bulk", conflicts_with = "bulk")]
        node_class: Option<String>,
        /// Autonomy: manual / remote / supervised / autonomous. Case-insensitive.
        #[arg(long, default_value = "manual", conflicts_with = "bulk")]
        autonomy: String,
        /// Custodian handle. Omit for self-service (your own identity is used);
        /// required when an operator enrolls on someone's behalf, or for any
        /// infrastructure node-class. In `--bulk` mode it names the ONE
        /// custodian the whole act draws down.
        #[arg(long)]
        custodian: Option<String>,
        /// Optional display name (defaults to the handle).
        #[arg(long, conflicts_with = "bulk")]
        display_name: Option<String>,
        /// PLATFORM sub-workload composition: a comma-separated list of the
        /// composable gear functions the box performs (e.g.
        /// `effector-cue,effector-arbiter`); its mesh groups are their union.
        /// POD and MOON are standalone classes, not PLATFORM workloads. Valid
        /// only when `--node-class PLATFORM`. Case-insensitive.
        #[arg(long, value_delimiter = ',', conflicts_with = "bulk")]
        workloads: Vec<String>,
        /// Bulk mode: read a YAML or JSON LIST of machine specs
        /// (`machine_handle`, `node_class`, `autonomy`, optional
        /// `display_name` / `workloads`) from a file — or stdin with `-` —
        /// and enroll them all in one act (`POST /api/machines/enroll-bulk`):
        /// N independent mints, one custodian, one single-use mesh key each.
        #[arg(long, value_name = "FILE")]
        bulk: Option<String>,
        /// Scope every minted MESH KEY to an exercise's synthetic-site groups
        /// (`EX-<PREFIX>-<class>`) instead of the goatnet site scope. The name
        /// must be in the portal's exercise map; identity is untouched — the
        /// exercise is a key scope, not a cert attribute.
        #[arg(long)]
        exercise: Option<String>,
        /// Emit the raw JSON bundle instead of the human summary.
        #[arg(long)]
        json: bool,
    },
    /// Re-mint the one-off mesh setup key for an ALREADY-ENROLLED machine —
    /// quota-neutral (the enrollment already consumed its unit; a join window
    /// is not a second machine). The recovery when the enroll-time key expired
    /// before the box joined: groups are re-derived from the machine's BORN-AS
    /// node-class, nothing else changes.
    MeshKey {
        /// The machine: its 32-hex slot anchor, or its handle (resolved via
        /// the machine list; an ambiguous handle is refused — pass the anchor).
        machine: String,
        /// Scope the key to an exercise's synthetic-site groups instead of the
        /// goatnet site scope (portal exercise map; same contract as enroll).
        #[arg(long)]
        exercise: Option<String>,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Retire an enrolled machine: cut its peer and any UNCONSUMED setup key
    /// from the mesh, then free its slot and delete its anchor row. Mesh-first
    /// (ADR 1031) — if the mesh cut fails nothing is changed, so a retry is a
    /// retry. Quota returns by construction. Re-enrolling the handle later
    /// mints a NEW anchor; the retired cert is not restorable.
    Revoke {
        /// The machine: its 32-hex slot anchor, or its handle (resolved via
        /// the machine list; an ambiguous handle is refused — pass the anchor).
        machine: String,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// List enrolled machines and their custody — a custodian's fleet, or (as an
    /// operator, with no `--custodian`) every enrolled machine. A non-operator
    /// always sees only its own machines.
    Ls {
        /// Custodian handle to scope to. Omit as operator for the whole fleet.
        #[arg(long)]
        custodian: Option<String>,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// NetBird SSH on an enrolled machine's mesh peer (ADR 1216 `device-admin`,
    /// tcp 22022). Needs the NetBird admin token; recorded in the
    /// operator-action audit.
    Ssh {
        #[command(subcommand)]
        op: super::machine_ssh::MachineSshCmd,
    },
    /// Set (create or adjust) a custodian's machine enrollment quota. Operator-only.
    /// This is the management path for the enrollment grant that onboarding seeds:
    /// `goat machine grant <custodian> --quota 5`. Include `--affiliation` the FIRST
    /// time a custodian is granted (a machine inherits its custodian's unit); omit it
    /// to only change the quota of a grant that already exists.
    Grant {
        /// Custodian handle (or slot uuid) to grant quota to.
        custodian: String,
        /// How many machines the custodian may have enrolled at once.
        #[arg(long)]
        quota: i64,
        /// Affiliation/unit stamped on every machine the custodian enrolls —
        /// required to seed a brand-new custodian's grant; omit to adjust quota only.
        #[arg(long)]
        affiliation: Option<String>,
        /// Optional site stamped on enrolled machines (only meaningful with --affiliation).
        #[arg(long)]
        site: Option<String>,
        /// Grant (or, with --no-network-admin, revoke) the delegated network-admin
        /// entitlement: lets this custodian enroll NETWORK-class bearer gear
        /// (routers/switches/radios) for their OWN site without an operator
        /// (ADR 1113). Requires the grant to carry a site. Omit both flags to
        /// leave the entitlement exactly as it is.
        #[arg(long, overrides_with = "no_network_admin")]
        network_admin: bool,
        /// Revoke the delegated network-admin entitlement (ADR 1113).
        #[arg(long)]
        no_network_admin: bool,
        /// Emit the raw JSON.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(ctx: &super::Ctx, op: MachineCmd) -> anyhow::Result<()> {
    let portal = ctx.profile.portal_client()?;
    let out = &ctx.output;
    match op {
        MachineCmd::Affiliation { op } => super::machine_affiliation::run(out, &portal, op).await,
        MachineCmd::Enroll {
            handle,
            node_class,
            autonomy,
            custodian,
            display_name,
            workloads,
            bulk,
            exercise,
            json,
        } => {
            if let Some(file) = bulk {
                enroll_bulk(out, &portal, &file, custodian, exercise, json).await
            } else {
                // clap's required_unless_present guarantees both in single mode.
                let handle = handle.expect("clap: handle required without --bulk");
                let node_class = node_class.expect("clap: --node-class required without --bulk");
                enroll(
                    out,
                    &portal,
                    handle,
                    node_class,
                    autonomy,
                    custodian,
                    display_name,
                    workloads,
                    exercise,
                    json,
                )
                .await
            }
        }
        MachineCmd::MeshKey {
            machine,
            exercise,
            json,
        } => mesh_key(out, &portal, machine, exercise, json).await,
        MachineCmd::Revoke { machine, yes, json } => {
            revoke(out, &portal, machine, yes, json).await
        }
        MachineCmd::Ls { custodian, json } => list(out, &portal, custodian, json).await,
        MachineCmd::Ssh { op } => super::machine_ssh::run(ctx, &portal, op).await,
        MachineCmd::Grant {
            custodian,
            quota,
            affiliation,
            site,
            network_admin,
            no_network_admin,
            json,
        } => {
            // Tri-state: --network-admin => Some(true), --no-network-admin =>
            // Some(false), neither => None ("leave it alone").
            let net_admin = if network_admin {
                Some(true)
            } else if no_network_admin {
                Some(false)
            } else {
                None
            };
            grant(out, &portal, custodian, quota, affiliation, site, net_admin, json).await
        }
    }
}

/// Render the custody surface (`GET /api/machines`).
async fn list(
    out: &OutputContext,
    portal: &PortalClient,
    custodian: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let res = with_spinner(out, "listing machines", portal.list_machines(custodian.as_deref()))
        .await
        .map_err(map_enroll_err)?;
    if json {
        println!("{}", serde_json::to_string(&res).unwrap_or_default());
        return Ok(());
    }
    if res.machines.is_empty() {
        println!("no enrolled machines");
        return Ok(());
    }
    for m in &res.machines {
        let comp = if m.workloads.is_empty() {
            String::new()
        } else {
            format!(" [{}]", m.workloads.join(","))
        };
        println!(
            "{:<24} {:<10} {:<11} {}  ({}){comp}",
            m.system_handle, m.node_class, m.autonomy, m.custodian, m.affiliation
        );
    }
    if let Some(q) = &res.quota {
        println!("— quota: {}/{} used, {} remaining", q.used, q.quota, q.remaining);
    }
    Ok(())
}

/// Set (create or adjust) a custodian's machine enrollment quota
/// (`POST /api/machines/grant`, operator-only).
#[allow(clippy::too_many_arguments)]
async fn grant(
    out: &OutputContext,
    portal: &PortalClient,
    custodian: String,
    quota: i64,
    affiliation: Option<String>,
    site: Option<String>,
    network_admin: Option<bool>,
    json: bool,
) -> anyhow::Result<()> {
    let spec = MachineGrantSpec {
        custodian: custodian.clone(),
        quota,
        network_admin,
        // Normalize the unit to the grammar's lowercase-kebab form (matches the
        // affiliation the enroll flow stamps); the server treats it as an opaque
        // string, but keeping the case consistent avoids two spellings of one unit.
        affiliation: affiliation.map(|a| a.to_lowercase()),
        site,
    };
    let seeding_affiliation = spec.affiliation.is_some();
    let q = with_spinner(
        out,
        "setting machine quota",
        portal.set_machine_grant(&spec),
    )
    .await
    .map_err(|e| map_grant_err(e, seeding_affiliation))?;
    if json {
        println!("{}", serde_json::to_string(&q).unwrap_or_default());
        return Ok(());
    }
    println!(
        "✓ {} quota set to {} (used {}, {} remaining)",
        q.custodian, q.quota, q.used, q.remaining
    );
    match network_admin {
        Some(true) => println!(
            "✓ {} may now enroll NETWORK-class bearer gear for its own site without an operator (ADR 1113)",
            q.custodian
        ),
        Some(false) => println!("✓ {} network-admin entitlement revoked", q.custodian),
        None => {}
    }
    Ok(())
}

/// Map the grant endpoint's refusals (machine_grant.go): 403 non-operator, and
/// the 400s. A bare refusal carries the status and not the body, so the exact
/// code may not be visible here — but the request shape narrows it, since
/// `affiliation_required` can only answer the quota-only path. Without that
/// split an operator who just hit the affiliation spelling guard is told to
/// pass the flag they already passed.
fn map_grant_err(e: PortalError, seeding_affiliation: bool) -> anyhow::Error {
    // Status keyed off `http_status()` so the bare and body-carrying refusal
    // shapes hit the same curated arms; the server's own message (when the
    // body carried one) rides underneath instead of replacing the guidance.
    let server = server_line(&e);
    if e.server_message()
        .is_some_and(|message| message.contains("legacy affiliation spellings"))
    {
        return anyhow::anyhow!("affiliation conflict: an operator must preview goat machine affiliation repair after verifying the organization.{server}");
    }
    match e.http_status().map(|s| s.as_u16()) {
        // Name the REMEDY, not just the requirement. The overwhelmingly common
        // cause is not "you lack authority" but "your profile holds the wrong
        // one of your two credentials": `goat profile init` writes the vp_
        // participant token out of the bundle, while this endpoint is
        // operator-gated and wants the op_ key, which is collected separately
        // from the console and is deliberately never in a bundle. Told only
        // that an operator token is required, people re-request the bundle —
        // which cannot help, because the bundle is where the wrong key came
        // from. Cost real time three times over .
        Some(403) => anyhow::anyhow!(
            "refused (403): setting a machine quota needs your OPERATOR key (op_…), \
             and this profile is using a different token.\n\
             \n\
             `goat profile init` installs the vp_ participant token from your bundle. \
             The op_ key is a separate credential you collect once from the identity \
             console's /operator-key page; it is never inside a bundle.\n\
             \n\
             Point this command at it, either way:\n\
             \x20 GOAT_PORTAL_TOKEN=op_… goat machine grant …\n\
             \x20 or put it in a paired operator context and use --context <env>-op:\n\
             \x20   cp -r ~/.goat/contexts/<env> ~/.goat/contexts/<env>-op\n\
             \x20   printf %s 'op_…' > ~/.goat/contexts/<env>-op/portal.token\n\
             \n\
             Do not overwrite the token in your main context — the vp_ key there is \
             what the participant-side calls use.{server}"
        ),
        Some(400) if seeding_affiliation => anyhow::anyhow!(
            "refused (400): the portal would not take that grant — either \
             --quota is negative, or the affiliation respells a unit already \
             on file (two spellings of one unit fork the fleet). Run \
             `goat machine ls` and pass the existing spelling exactly.{server}"
        ),
        Some(400) => anyhow::anyhow!(
            "refused (400): this custodian has no grant yet — pass --affiliation \
             <unit> to seed one (a machine inherits its custodian's unit){server}"
        ),
        _ => super::vendor::map_portal_err(e),
    }
}

/// The portal's own message as an indented follow-on stanza, or empty. Every
/// machine-verb mapper appends this to its curated arm: the arm explains the
/// refusal class, the server names the concrete instance. Shared with the
/// vendor mappers so one concept keeps one rendering.
use super::vendor::server_line;

#[cfg(test)]
mod grant_err_tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn seeding_400_points_at_the_spelling_guard_not_the_missing_flag() {
        let e = map_grant_err(
            PortalError::status(StatusCode::BAD_REQUEST),
            true,
        );
        let msg = e.to_string();
        assert!(msg.contains("goat machine ls"), "{msg}");
        assert!(!msg.contains("pass --affiliation"), "{msg}");
    }

    #[test]
    fn quota_only_400_still_asks_for_the_affiliation() {
        let e = map_grant_err(
            PortalError::status(StatusCode::BAD_REQUEST),
            false,
        );
        assert!(e.to_string().contains("--affiliation"), "{e}");
    }
}

#[allow(clippy::too_many_arguments)]
async fn enroll(
    out: &OutputContext,
    portal: &PortalClient,
    handle: String,
    node_class: String,
    autonomy: String,
    custodian: Option<String>,
    display_name: Option<String>,
    workloads: Vec<String>,
    exercise: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    // Normalize to the grammar's canonical case (node-classes uppercase, autonomy
    // lowercase) so an operator needn't remember the casing; the server validates
    // the closed set either way.
    let spec = MachineEnrollSpec {
        machine_handle: handle,
        node_class: node_class.to_uppercase(),
        autonomy: autonomy.to_lowercase(),
        custodian,
        display_name,
        workloads: (!workloads.is_empty())
            .then(|| workloads.iter().map(|w| w.to_uppercase()).collect()),
        exercise,
    };
    let res = with_spinner(out, "enrolling machine", portal.enroll_machine(&spec))
        .await
        .map_err(map_enroll_err)?;

    if json {
        println!("{}", serde_json::to_string(&res).unwrap_or_default());
        return Ok(());
    }
    println!(
        "✓ enrolled {} — {} (autonomy {}) under custodian {} ({})",
        res.system_handle, res.node_class, res.autonomy, res.custodian, res.affiliation
    );
    println!("  slot     {}", res.slot_id);
    if !res.workloads.is_empty() {
        println!("  workloads {}", res.workloads.join(", "));
    }
    println!("  api_key  {}", res.api_key);
    if !res.bundle_url.is_empty() {
        println!("  bundle   {}", res.bundle_url);
    }
    // ADR 1116 §9 — the mesh half. Printed BEFORE the note so the groups are
    // the thing an operator's eye lands on: the class chose them, not a person.
    if !res.mesh_groups.is_empty() {
        println!("  groups   {}", res.mesh_groups.join(" + "));
    }
    if !res.mesh_setup_key.is_empty() {
        println!("  mesh key {}", res.mesh_setup_key);
    }
    if !res.mesh_scope.is_empty() {
        println!("  scope    {} (exercise-scoped key)", res.mesh_scope);
    }
    if !res.mesh_note.is_empty() {
        println!("  mesh     {}", res.mesh_note);
    }
    println!("  quota    {} remaining", res.quota_remaining);
    if !res.note_to_share.is_empty() {
        println!("  note     {}", res.note_to_share);
    }
    Ok(())
}

/// Map the portal's refusals to actionable messages. The server returns a small,
/// stable set of statuses for this endpoint (machine_enroll.go): 403 for the
/// grant/gate refusals, 409 for quota/alias, 400 for bad grammar.
fn map_enroll_err(e: PortalError) -> anyhow::Error {
    let server = server_line(&e);
    // The server names the refusal; when it does, say THAT one plainly instead
    // of listing every 403 cause. `no_enrollment_grant` is by far the common
    // one, and a person told "one of three things" cannot tell that the fix is
    // an admin's `goat machine grant`.
    match e.error_code() {
        Some("no_enrollment_grant") => {
            return anyhow::anyhow!(
                "enrollment refused (403): you hold no machine-enrollment grant on this \
                 goatnet, so your allowance is 0 machines.\n\
                 \n\
                 A grant belongs to the PERSON who enrolls, not to their organisation. \
                 Ask a goatnet admin to run, with their operator key:\n\
                 \x20 goat machine grant <your-handle> --quota <n> --affiliation <your-org>\n\
                 then check it with `goat machine ls`, which prints your quota.{server}"
            );
        }
        Some("quota_exhausted") => {
            return anyhow::anyhow!(
                "enrollment refused (409): every machine your grant allows is already \
                 enrolled. Revoke one you no longer use (`goat machine revoke <machine>`) \
                 or ask a goatnet admin to raise your quota \
                 (`goat machine grant <your-handle> --quota <n>`).{server}"
            );
        }
        _ => {}
    }
    match e.http_status().map(|s| s.as_u16()) {
        Some(403) => anyhow::anyhow!(
            "enrollment refused (403): no enrollment grant for this custodian, an \
             infrastructure node-class needs an operator token, or a non-operator \
             named a custodian other than itself{server}"
        ),
        Some(409) => anyhow::anyhow!(
            "enrollment refused (409): the custodian's machine quota is exhausted, \
             or that machine handle already holds a live slot (revoke it or pick \
             another handle){server}"
        ),
        Some(400) => anyhow::anyhow!(
            "enrollment refused (400): invalid node-class or autonomy, a malformed \
             machine handle, or an ADR 1116 mint-time refusal — SENSOR and EFFECTOR \
             are RESERVED (class the box by the function it performs: SENSOR-FEED, \
             EFFECTOR-CUE, KID, FUSION …); SENSOR-TASKING and EFFECTOR-COMMAND are \
             intake-only and never a primary class; a PLATFORM needs --workloads so \
             its groups can be derived from the functions it performs. The server's \
             message names which one{server}"
        ),
        Some(502) => anyhow::anyhow!(
            "enrollment rolled back (502): the mesh setup key could not be minted, so \
             the machine was not left with a fabric identity and no mesh membership. \
             Quota is returned. Usually the derived group does not exist yet on this \
             goatnet — run netbird-acl-bootstrap.yml for the site{server}"
        ),
        _ => super::vendor::map_portal_err(e),
    }
}

/// Re-mint the one-off mesh setup key for an already-enrolled machine
/// (`POST /api/machines/{anchor}/mesh-key`, quota-neutral).
async fn mesh_key(
    out: &OutputContext,
    portal: &PortalClient,
    machine: String,
    exercise: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let anchor = if panoscope_core::alias::is_anchor_shaped(&machine) {
        machine
    } else {
        // Not anchor-shaped: treat it as a handle and resolve it against the
        // machine list (the fleet for an operator; own machines for a
        // vendor-scoped caller — the server scopes the list either way).
        let list = with_spinner(out, "resolving machine handle", portal.list_machines(None))
            .await
            .map_err(map_enroll_err)?;
        resolve_handle_to_anchor(&list.machines, &machine)?
    };
    let res = with_spinner(
        out,
        "re-minting mesh key",
        portal.machine_mesh_key(&anchor, exercise.as_deref()),
    )
    .await
    .map_err(map_mesh_key_err)?;

    if json {
        println!("{}", serde_json::to_string(&res).unwrap_or_default());
        return Ok(());
    }
    println!(
        "✓ re-minted mesh key for {} — {} (quota-neutral)",
        res.system_handle, res.node_class
    );
    println!("  slot     {}", res.slot_id);
    if !res.mesh_groups.is_empty() {
        println!("  groups   {}", res.mesh_groups.join(" + "));
    }
    if !res.mesh_scope.is_empty() {
        println!("  scope    {} (exercise-scoped key)", res.mesh_scope);
    }
    // The key IS the payload of this verb — set it off on its own block so it
    // cannot be lost among the detail lines (it is single-use; losing it means
    // running this verb again).
    if res.mesh_setup_key.is_empty() {
        println!();
        println!("  NO KEY MINTED — {}", res.mesh_note);
    } else {
        println!();
        println!("  MESH SETUP KEY   {}", res.mesh_setup_key);
        println!();
        if !res.mesh_note.is_empty() {
            println!("  mesh     {}", res.mesh_note);
        }
    }
    Ok(())
}

/// Retire an enrolled machine (`DELETE /api/machines/{anchor}`).
///
/// The confirmation is not ceremony: unlike the re-mint next door, this one is
/// not undoable. Freeing the slot returns its index to the pool, so the anchor
/// the retired cert was minted for can later belong to a different tenant
/// — re-enrolling the same handle gives a NEW anchor, never the old one
/// back.
async fn revoke(
    out: &OutputContext,
    portal: &PortalClient,
    machine: String,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let anchor = if panoscope_core::alias::is_anchor_shaped(&machine) {
        machine.clone()
    } else {
        let list = with_spinner(out, "resolving machine handle", portal.list_machines(None))
            .await
            .map_err(map_enroll_err)?;
        resolve_handle_to_anchor(&list.machines, &machine)?
    };
    if !yes {
        eprint!(
            "Retire machine `{machine}` (anchor {anchor})? Its peer and any unconsumed \
             setup key are cut from the mesh, its slot is freed and re-enrolling mints a \
             NEW anchor. [y/N] "
        );
        std::io::Write::flush(&mut std::io::stderr()).ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("aborted (no confirmation)");
        }
    }
    let res = with_spinner(out, "revoking machine", portal.revoke_machine(&anchor))
        .await
        .map_err(map_revoke_err)?;

    if json {
        println!("{}", serde_json::to_string(&res).unwrap_or_default());
        return Ok(());
    }
    println!("✓ retired {} — {}", res.system_handle, res.node_class);
    println!("  slot     {}", res.slot_id);
    if res.mesh_cut {
        println!(
            "  mesh     {} peer(s), {} unconsumed key(s) deleted",
            res.peers_deleted.len(),
            res.keys_deleted.len()
        );
        if !res.keys_already_spent.is_empty() {
            println!(
                "           {} already-spent key(s) left as the record of how it joined",
                res.keys_already_spent.len()
            );
        }
        // A machine that joined but whose peer was NOT found is the
        // name/handle drift case: the peer was renamed off its handle, so the
        // provenance join key no longer matches and the cut missed it.
        if res.peers_deleted.is_empty() && !res.keys_already_spent.is_empty() {
            println!();
            println!("  WARNING: a setup key for this machine was SPENT but no peer named");
            println!(
                "  {:?} exists — the peer was very likely renamed off its handle, and",
                res.system_handle
            );
            println!("  it is STILL ON THE MESH. Find it and delete it by hand.");
        }
    } else {
        println!("  mesh     NOT CUT — {}", res.mesh_note);
    }
    if res.quota_remaining >= 0 {
        println!("  quota    {} remaining for {}", res.quota_remaining, res.custodian);
    }
    Ok(())
}

/// Map the revoke endpoint's refusals (machine_revoke.go).
fn map_revoke_err(e: PortalError) -> anyhow::Error {
    let server = server_line(&e);
    match e.http_status().map(|s| s.as_u16()) {
        // The route does not exist on this portal at all. Go's ServeMux answers
        // 405 (not 404) when a path is registered for other methods but not
        // DELETE — which is exactly the shape of a portal built before this
        // endpoint. Worth its own message: without it the operator reads a bare
        // "405" and reasonably concludes the machine is un-revokable, when the
        // real answer is that this goatnet has not taken the image yet.
        Some(405) => anyhow::anyhow!(
            "this goatnet's portal has no machine-revoke endpoint (405) — it predates \
             it. Nothing was changed. The endpoint reaches a goatnet by the ordinary \
             graduation ladder; until it lands, retire the registry side with \
             `DELETE /api/vendors/<anchor>` and cut the peer + any unconsumed setup \
             key on the mgmt server BY HAND, in that order.{server}"
        ),
        Some(404) => anyhow::anyhow!(
            "refused (404): no live custody-enrolled machine there — it is a person's \
             slot, a free slot, or already revoked. `goat machine ls` shows what is \
             live.{server}"
        ),
        Some(403) => anyhow::anyhow!(
            "refused (403): a non-operator may revoke only machines under its own \
             custody edge{server}"
        ),
        Some(409) => anyhow::anyhow!(
            "refused (409): the mesh cut already happened, but this anchor still owns \
             non-retired topics — retire them and re-run to finish the registry side{server}"
        ),
        Some(502) => anyhow::anyhow!(
            "refused (502): the mesh cut failed, so NOTHING was changed — the machine, \
             its slot and its cert are exactly as they were. Retry once the netbird \
             mgmt server answers; the portal will not retire a registry row while the \
             peer is still live.{server}"
        ),
        // Was `anyhow::Error::new(e)` — the raw PortalError, unlike
        // every sibling map_*_err in this file, which all delegate their
        // catch-all to the shared classifier (e.g. map_mesh_key_err just
        // below). That meant a revoke hitting any status code other than
        // the five named above (401 in particular — a bad/expired token,
        // not just "wrong kind of token") surfaced as an unclassified
        // PortalError Display instead of a proper GoatError.
        _ => super::vendor::map_portal_err(e),
    }
}

/// Pure half of the handle→anchor resolution, split out so the ambiguity
/// refusal is unit-testable without a portal.
fn resolve_handle_to_anchor(machines: &[MachineListItem], handle: &str) -> anyhow::Result<String> {
    let matches: Vec<&MachineListItem> =
        machines.iter().filter(|m| m.system_handle == handle).collect();
    match matches.len() {
        0 => anyhow::bail!(
            "no enrolled machine with handle {handle:?} — `goat machine ls` shows the \
             machines you can see; or pass the 32-hex slot anchor directly"
        ),
        1 => Ok(matches[0].slot_id.clone()),
        n => anyhow::bail!(
            "handle {handle:?} matches {n} machines (custodians: {}) — ambiguous; pass \
             the 32-hex slot anchor instead (`goat machine ls` shows both)",
            matches.iter().map(|m| m.custodian.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Map the mesh-key endpoint's refusals (machine_mesh_key.go): 404 not a
/// machine, 403 custodian mismatch, 409 legacy born-as class with no mesh
/// derivation, 400 unknown exercise, 502 mint failure.
fn map_mesh_key_err(e: PortalError) -> anyhow::Error {
    let server = server_line(&e);
    match e.http_status().map(|s| s.as_u16()) {
        Some(404) => anyhow::anyhow!(
            "refused (404): no live custody-enrolled machine at this anchor — the \
             re-mint is for machines the mint already enrolled; a person's slot or a \
             free slot has no derived mesh reach{server}"
        ),
        Some(403) => anyhow::anyhow!(
            "refused (403): a non-operator may re-mint a mesh key only for machines \
             under its own custody edge{server}"
        ),
        Some(409) => anyhow::anyhow!(
            "refused (409): this machine's born-as node-class has no mesh derivation \
             (legacy whole-system class) — re-stamp it with a gear class and re-mint{server}"
        ),
        Some(400) => anyhow::anyhow!(
            "refused (400): unknown or unconfigured exercise — the name must be in this \
             goatnet's exercise map (the server's message lists the known names){server}"
        ),
        Some(502) => anyhow::anyhow!(
            "re-mint failed (502): the mesh setup key could not be minted. Usually the \
             derived group does not exist yet on this goatnet — run \
             netbird-acl-bootstrap.yml for the site{server}"
        ),
        _ => super::vendor::map_portal_err(e),
    }
}

/// Bulk enrollment (`POST /api/machines/enroll-bulk`): read a YAML/JSON list
/// of machine specs from a file (or stdin with `-`), fire ONE act, render the
/// per-entry outcomes. Partial failure exits non-zero after rendering — the
/// crate convention (exit 0 only when everything minted).
async fn enroll_bulk(
    out: &OutputContext,
    portal: &PortalClient,
    file: &str,
    custodian: Option<String>,
    exercise: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let machines = read_bulk_specs(file)?;
    let req = MachineBulkEnrollSpec {
        custodian,
        exercise,
        machines,
    };
    let res = with_spinner(
        out,
        "enrolling machines (bulk)",
        portal.enroll_machines_bulk(&req),
    )
    .await
    .map_err(map_enroll_err)?;

    if json {
        println!("{}", serde_json::to_string(&res).unwrap_or_default());
    } else {
        println!(
            "{} bulk enroll under custodian {} — {} minted, {} failed, {} remaining",
            if res.failed == 0 { "✓" } else { "!" },
            res.custodian,
            res.minted,
            res.failed,
            if res.quota_remaining < 0 {
                "quota unknown".to_string()
            } else {
                format!("quota {}", res.quota_remaining)
            }
        );
        for e in &res.entries {
            let detail = match &e.result {
                Some(r) if !r.mesh_setup_key.is_empty() => format!("mesh key {}", r.mesh_setup_key),
                Some(r) if !r.mesh_note.is_empty() => format!("no mesh key — {}", r.mesh_note),
                Some(_) => String::new(),
                None if !e.error_code.is_empty() => format!("{}: {}", e.error_code, e.error),
                None => e.error.clone(),
            };
            println!("{:<24} {:<8} {detail}", e.machine_handle, e.status);
        }
        if res.minted > 0 {
            println!(
                "— full bundles (api_key, bundle, mesh groups) per machine: re-run with --json"
            );
        }
    }
    if res.failed > 0 {
        anyhow::bail!(
            "{} of {} entries failed ({} minted) — each failed entry rolled back only \
             itself; fix the spec and re-run with the failed machines",
            res.failed,
            res.entries.len(),
            res.minted
        );
    }
    Ok(())
}

/// Read the `--bulk` spec file (or stdin for `-`): a YAML or JSON LIST of
/// machine specs, or the wire-shaped `machines:` wrapper. Normalizes grammar
/// case the same way the single door does (node-classes/workloads uppercase,
/// autonomy lowercase).
fn read_bulk_specs(path: &str) -> anyhow::Result<Vec<MachineBulkSpec>> {
    let raw = if path == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| anyhow::anyhow!("read machine specs from stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read machine spec file {path}: {e}"))?
    };
    parse_bulk_specs(&raw).map_err(|e| {
        anyhow::anyhow!(
            "parse {}: {e} — expected a YAML/JSON LIST of machine specs, each with \
             machine_handle + node_class + autonomy (optional display_name, workloads)",
            if path == "-" { "stdin" } else { path }
        )
    })
}

/// Parse + normalize the bulk spec body. Split from the I/O so it is
/// unit-testable. serde_yaml also parses JSON (a YAML subset), so one decode
/// path covers both formats.
fn parse_bulk_specs(raw: &str) -> anyhow::Result<Vec<MachineBulkSpec>> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum BulkFile {
        List(Vec<MachineBulkSpec>),
        Wrapped { machines: Vec<MachineBulkSpec> },
    }
    let parsed: BulkFile = serde_yaml::from_str(raw)?;
    let mut specs = match parsed {
        BulkFile::List(v) | BulkFile::Wrapped { machines: v } => v,
    };
    if specs.is_empty() {
        anyhow::bail!("the spec list is empty");
    }
    for s in &mut specs {
        s.node_class = s.node_class.to_uppercase();
        s.autonomy = s.autonomy.to_lowercase();
        if let Some(w) = &mut s.workloads {
            for x in w.iter_mut() {
                *x = x.to_uppercase();
            }
        }
    }
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(handle: &str, slot: &str, custodian: &str) -> MachineListItem {
        MachineListItem {
            slot_id: slot.into(),
            system_handle: handle.into(),
            custodian: custodian.into(),
            node_class: "SENSOR-FEED".into(),
            autonomy: "supervised".into(),
            affiliation: "usareur-af".into(),
            workloads: vec![],
            enrolled_at: 0,
        }
    }

    #[test]
    fn resolve_handle_unique_match() {
        let ms = vec![item("fra-sensor-01", "aa", "alice"), item("fra-eff-01", "bb", "bob")];
        assert_eq!(resolve_handle_to_anchor(&ms, "fra-sensor-01").unwrap(), "aa");
    }

    #[test]
    fn resolve_handle_refuses_unknown_and_ambiguous() {
        let ms = vec![item("dup", "aa", "alice"), item("dup", "bb", "bob")];
        let missing = resolve_handle_to_anchor(&ms, "nope").unwrap_err().to_string();
        assert!(missing.contains("no enrolled machine"), "{missing}");
        let ambiguous = resolve_handle_to_anchor(&ms, "dup").unwrap_err().to_string();
        assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
        assert!(ambiguous.contains("alice, bob"), "{ambiguous}");
    }

    fn body_err(status: u16, message: &str) -> PortalError {
        PortalError::HttpWithBody {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            code: None,
            message: message.into(),
            path: String::new(),
            retry_after: None,
        }
    }

    // A body-carrying refusal must hit the same curated arm as the bare
    // status AND carry the server's message — composed, never replaced.
    #[test]
    fn map_enroll_err_with_body_composes_server_line() {
        let msg = map_enroll_err(body_err(400, "workload KID has no mesh mapping")).to_string();
        assert!(msg.contains("enrollment refused (400)"), "{msg}");
        assert!(msg.contains("ADR 1116"), "{msg}");
        assert!(msg.contains("server: workload KID has no mesh mapping"), "{msg}");
    }

    fn coded_err(status: u16, code: &str, message: &str) -> PortalError {
        PortalError::HttpWithBody {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            code: Some(code.into()),
            message: message.into(),
            path: String::new(),
            retry_after: None,
        }
    }

    #[test]
    fn no_enrollment_grant_names_the_grant_and_the_admin_command() {
        let msg = map_enroll_err(coded_err(
            403,
            "no_enrollment_grant",
            "custodian \"x\" holds no machine enrollment grant",
        ))
        .to_string();
        assert!(msg.contains("no machine-enrollment grant"), "{msg}");
        assert!(msg.contains("goat machine grant <your-handle>"), "{msg}");
        assert!(!msg.contains("infrastructure node-class"), "{msg}");
    }

    #[test]
    fn quota_exhausted_names_revoke_and_raise() {
        let msg = map_enroll_err(coded_err(409, "quota_exhausted", "5/5 used")).to_string();
        assert!(msg.contains("goat machine revoke"), "{msg}");
        assert!(msg.contains("--quota"), "{msg}");
        assert!(msg.contains("5/5 used"), "{msg}");
    }

    #[test]
    fn other_403_codes_keep_the_generic_arm() {
        let msg = map_enroll_err(coded_err(403, "infra_requires_operator", "x")).to_string();
        assert!(msg.contains("infrastructure node-class"), "{msg}");
    }

    #[test]
    fn map_grant_err_with_body_composes_server_line() {
        let msg = map_grant_err(body_err(400, "custodian has no affiliation"), false).to_string();
        assert!(msg.contains("--affiliation"), "{msg}");
        assert!(msg.contains("server: custodian has no affiliation"), "{msg}");
    }

    #[test]
    fn map_mesh_key_err_with_body_composes_server_line() {
        let msg = map_mesh_key_err(body_err(409, "born-as class LEGACY")).to_string();
        assert!(msg.contains("no mesh derivation"), "{msg}");
        assert!(msg.contains("server: born-as class LEGACY"), "{msg}");
    }

    #[test]
    fn map_revoke_err_with_body_composes_server_line() {
        let msg = map_revoke_err(body_err(502, "mgmt unreachable")).to_string();
        assert!(msg.contains("NOTHING was changed"), "{msg}");
        assert!(msg.contains("server: mgmt unreachable"), "{msg}");
    }

    // An uncurated status still routes through the shared portal mapper
    // without losing the body; a 5xx the portal answered is its own fault,
    // not "unreachable" (#2087).
    #[test]
    fn map_enroll_err_with_body_uncurated_status_falls_through() {
        let err = map_enroll_err(body_err(500, "panic in mint"));
        let g = err
            .downcast_ref::<crate::errors::GoatError>()
            .expect("downcast");
        assert_eq!(g.code(), "E_PORTAL_SERVER_ERROR");
        let dbg = format!("{g:?}");
        assert!(dbg.contains("panic in mint"), "{dbg}");
    }

    // The --bulk file: a YAML list, case-normalized the same way the single
    // door normalizes flags.
    #[test]
    fn parse_bulk_specs_yaml_list_and_normalization() {
        let specs = parse_bulk_specs(
            r#"
- machine_handle: fra-sensor-01
  node_class: sensor-feed
  autonomy: SUPERVISED
- machine_handle: fra-platform-1
  node_class: platform
  autonomy: autonomous
  display_name: Fra Platform One
  workloads: [sensor, effector]
"#,
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].node_class, "SENSOR-FEED");
        assert_eq!(specs[0].autonomy, "supervised");
        assert!(specs[0].workloads.is_none());
        assert_eq!(
            specs[1].workloads.as_deref(),
            Some(&["SENSOR".to_string(), "EFFECTOR".to_string()][..])
        );
    }

    // JSON is a YAML subset — the same decode path takes a JSON list, and the
    // wire-shaped `machines:` wrapper is accepted for symmetry with the API.
    #[test]
    fn parse_bulk_specs_json_and_wrapped() {
        let specs = parse_bulk_specs(
            r#"[{"machine_handle":"kid-1","node_class":"KID","autonomy":"manual"}]"#,
        )
        .unwrap();
        assert_eq!(specs[0].machine_handle, "kid-1");

        let wrapped = parse_bulk_specs(
            "machines:\n  - machine_handle: kid-2\n    node_class: kid\n    autonomy: manual\n",
        )
        .unwrap();
        assert_eq!(wrapped[0].machine_handle, "kid-2");
        assert_eq!(wrapped[0].node_class, "KID");
    }

    #[test]
    fn parse_bulk_specs_refuses_empty_and_malformed() {
        assert!(parse_bulk_specs("[]").is_err());
        assert!(parse_bulk_specs("- machine_handle: x\n").is_err()); // missing required keys
        assert!(parse_bulk_specs("not a list").is_err());
    }

    // The revoke refusals an operator will actually hit. 405 is the one worth
    // pinning: Go's ServeMux answers it (not 404) for a path registered with
    // other methods but no DELETE, which is exactly a portal predating this
    // endpoint. A bare "405" reads as "this machine cannot be revoked"; the
    // message has to say the goatnet has not taken the image yet.
    #[test]
    fn revoke_err_405_names_the_undeployed_endpoint_not_a_bad_machine() {
        let msg = map_revoke_err(PortalError::status(reqwest::StatusCode::METHOD_NOT_ALLOWED))
        .to_string();
        assert!(msg.contains("no machine-revoke endpoint"), "{msg}");
        assert!(msg.contains("Nothing was changed"), "{msg}");
        assert!(msg.contains("/api/vendors/"), "names the interim door: {msg}");
    }

    // 502 means the mesh cut failed, and the load-bearing fact is that the
    // registry is untouched — otherwise an operator retries against a state
    // they think is half-cut.
    #[test]
    fn revoke_err_502_says_nothing_was_changed() {
        let msg = map_revoke_err(PortalError::status(reqwest::StatusCode::BAD_GATEWAY))
        .to_string();
        assert!(msg.contains("NOTHING was changed"), "{msg}");
        assert!(msg.contains("Retry"), "{msg}");
    }
}

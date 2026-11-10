// `goat machine ssh enable` — turn on NetBird SSH on an enrolled machine's
// mesh peer (ADR 1216: a site network engineer reaches NETWORK, CONTROL and
// EDGE-SERVER devices on port-set `device-admin`, which carries NetBird SSH on
// tcp 22022; the port is open only once the peer's SSH server is on).
//
// The machine is resolved through the portal machine list, so a non-operator
// can name only machines under its own custody. Its peer is found by the name
// the enrollment gave it (the portal's `ownedPeerName` rule). The change is one
// NetBird `PUT /api/peers/{id}` with every other setting read back unchanged
// (`NetbirdClient::set_peer_ssh`), and it needs the NetBird admin token, so in
// practice an operator runs it. It is recorded in the operator-action audit
// (`ops/audit/operator/<actor>`, docs/design/operator-action-audit.md) as
// `machine.ssh.enable`: `started` before the write, `ok` or `failed` after.
//
// Every error this verb returns is a `GoatError` whose code is in
// ops/errors/catalog.yaml (ADR 1199).

use crate::errors::GoatError;
use crate::output::{with_spinner, OutputContext};
use clap::Subcommand;
use panoscope_core::netbird::{NbPeer, NetbirdClient, NetbirdError, PeerSshChange};
use panoscope_core::portal::{MachineListItem, PortalClient, PortalError};

/// The audit action name for this verb.
pub(crate) const ACTION: &str = "machine.ssh.enable";

#[derive(Subcommand, Debug)]
pub enum MachineSshCmd {
    /// Turn on NetBird SSH (tcp 22022) on an enrolled machine's mesh peer.
    /// Reads the peer, then writes only `ssh_enabled`: every other setting is
    /// sent back as read. Needs the NetBird admin token
    /// (`<CONTEXT>_NB_API_TOKEN` or `GOAT_NB_API_TOKEN`). Recorded in the
    /// operator-action audit. The device's NetBird client must also allow an
    /// SSH server (`netbird up --allow-server-ssh` on the device).
    Enable {
        /// The machine: its handle or its 32-hex slot anchor (`goat machine ls`).
        machine: String,
        /// Resolve the machine and its peer and print the change, without writing.
        #[arg(long)]
        dry_run: bool,
        /// Override the NetBird mgmt URL (else env / derived from the profile).
        #[arg(long)]
        mgmt_url: Option<String>,
    },
}

pub async fn run(ctx: &super::Ctx, portal: &PortalClient, op: MachineSshCmd) -> anyhow::Result<()> {
    match op {
        MachineSshCmd::Enable {
            machine,
            dry_run,
            mgmt_url,
        } => enable(ctx, portal, &machine, dry_run, mgmt_url.as_deref()).await,
    }
}

async fn enable(
    ctx: &super::Ctx,
    portal: &PortalClient,
    machine: &str,
    dry_run: bool,
    mgmt_url: Option<&str>,
) -> anyhow::Result<()> {
    let out = &ctx.output;
    let nb = ctx.profile.netbird_client(mgmt_url)?;
    let (m, peer) = resolve(out, portal, &nb, machine).await?;

    println!(
        "machine  {} ({}, custodian {})",
        m.system_handle, m.node_class, m.custodian
    );
    println!(
        "peer     {} ({})  mesh_ip={}  connected={}",
        peer.name, peer.id, peer.ip, peer.connected
    );
    if peer.ssh_enabled {
        println!("NetBird SSH is already on for this peer — nothing to change");
        return Ok(());
    }
    if dry_run {
        println!(
            "dry-run: PUT <netbird>/api/peers/{} ssh_enabled=true, every other setting as read (not executed)",
            peer.id
        );
        return Ok(());
    }

    let actor = crate::cmd::audit::actor_handle(&ctx.profile);
    let env = ctx.profile.context.clone();
    crate::cmd::audit::emit(&ctx.profile, audit_record(&actor, &env, "started", &m, &peer, None)).await;
    let res = with_spinner(out, "turning on NetBird SSH", nb.set_peer_ssh(&peer.id, true))
        .await
        .map_err(|e| map_nb_err(e, &m.system_handle));
    let change = match res {
        Ok(c) => c,
        Err(e) => {
            let msg = crate::errors::DisplayHuman(&e).to_string();
            crate::cmd::audit::emit(&ctx.profile, audit_record(&actor, &env, "failed", &m, &peer, Some(msg)))
                .await;
            return Err(e.into());
        }
    };
    crate::cmd::audit::emit(&ctx.profile, audit_record(&actor, &env, "ok", &m, &peer, None)).await;
    for line in result_lines(&m, &change) {
        println!("{line}");
    }
    Ok(())
}

/// Resolve the machine through the portal and its peer through NetBird.
pub(crate) async fn resolve(
    out: &OutputContext,
    portal: &PortalClient,
    nb: &NetbirdClient,
    machine: &str,
) -> Result<(MachineListItem, NbPeer), GoatError> {
    let list = with_spinner(out, "resolving machine", portal.list_machines(None))
        .await
        .map_err(map_portal_err)?;
    let m = resolve_machine(&list.machines, machine)?.clone();
    let peers = with_spinner(out, "finding its mesh peer", nb.list_peers())
        .await
        .map_err(|e| map_nb_err(e, &m.system_handle))?;
    let peer = resolve_peer(&peers, &m.system_handle)?.clone();
    Ok((m, peer))
}

/// One machine for a handle or a 32-hex slot anchor.
pub(crate) fn resolve_machine<'a>(
    machines: &'a [MachineListItem],
    input: &str,
) -> Result<&'a MachineListItem, GoatError> {
    let by_anchor = panoscope_core::alias::is_anchor_shaped(input);
    let matches: Vec<&MachineListItem> = machines
        .iter()
        .filter(|m| if by_anchor { m.slot_id == input } else { m.system_handle == input })
        .collect();
    match matches.as_slice() {
        [m] => Ok(m),
        [] => Err(GoatError::MachineUnresolved {
            machine: input.to_string(),
            hint: format!(
                "no enrolled machine this profile can see has {} {input:?}. `goat machine ls` \
                 lists them; a non-operator sees only machines under its own custody.",
                if by_anchor { "slot anchor" } else { "handle" }
            ),
        }),
        many => Err(GoatError::MachineUnresolved {
            machine: input.to_string(),
            hint: format!(
                "handle {input:?} matches {} machines (custodians: {}). Pass the 32-hex slot \
                 anchor instead; `goat machine ls` shows both.",
                many.len(),
                many.iter().map(|m| m.custodian.as_str()).collect::<Vec<_>>().join(", ")
            ),
        }),
    }
}

/// Whether a peer name belongs to the machine with this handle. The same rule
/// as the portal's `ownedPeerName` (internal/netbirdkeys): the setup key is
/// named `machine-<handle>`, NetBird names the peer `<key>-<hostname>`, and the
/// documented hand rename is the bare handle. The trailing hyphen keeps
/// `machine-sw-1` from claiming `machine-sw-11-host`.
pub(crate) fn owned_peer_name(name: &str, handle: &str) -> bool {
    let key = format!("machine-{handle}");
    name == handle || name == key || name.starts_with(&format!("{key}-"))
}

/// The one NetBird peer that belongs to the machine.
pub(crate) fn resolve_peer<'a>(peers: &'a [NbPeer], handle: &str) -> Result<&'a NbPeer, GoatError> {
    let matches: Vec<&NbPeer> = peers.iter().filter(|p| owned_peer_name(&p.name, handle)).collect();
    match matches.as_slice() {
        [p] => Ok(p),
        [] => Err(GoatError::MeshPeerUnresolved {
            machine: handle.to_string(),
            hint: format!(
                "no NetBird peer is named `machine-{handle}-<host>` or `{handle}`. Either the \
                 machine never joined (`goat machine mesh-key {handle}` re-mints its key) or its \
                 peer was renamed; `goat admin peer ls` shows every peer."
            ),
        }),
        many => Err(GoatError::MeshPeerUnresolved {
            machine: handle.to_string(),
            hint: format!(
                "{} NetBird peers carry this machine's name: {}. Remove the stale one with \
                 `goat admin peer cut <peer-id>` and run this again.",
                many.len(),
                many.iter()
                    .map(|p| format!("{} ({}, connected={})", p.name, p.id, p.connected))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }),
    }
}

/// A NetBird client error as a catalog-coded `GoatError`.
pub(crate) fn map_nb_err(e: NetbirdError, handle: &str) -> GoatError {
    match e {
        NetbirdError::Disabled => GoatError::NetbirdFailed {
            hint: "no NetBird mgmt URL or admin token is configured — export \
                   <CONTEXT>_NB_API_TOKEN (or GOAT_NB_API_TOKEN) and, if the URL is not derived \
                   from the profile, pass --mgmt-url."
                .into(),
        },
        NetbirdError::NotFound => GoatError::MeshPeerUnresolved {
            machine: handle.to_string(),
            hint: "the peer was found in the peer list and then answered 404: it was removed \
                   in between. Nothing was changed; `goat admin peer ls` shows what is there now."
                .into(),
        },
        NetbirdError::Http { status } if status.as_u16() == 403 => GoatError::NotAdmin {
            hint: format!(
                "NetBird mgmt API denied this request (HTTP {status}). The token is valid but \
                 not authorized to change peers. Nothing was changed. Use an admin-scoped \
                 NetBird token."
            ),
        },
        NetbirdError::Http { status } if status.as_u16() == 401 => GoatError::NetbirdFailed {
            hint: format!(
                "NetBird mgmt API rejected the token (HTTP {status}). Nothing was changed. \
                 Export a current admin token as <CONTEXT>_NB_API_TOKEN or GOAT_NB_API_TOKEN."
            ),
        },
        NetbirdError::Http { status } => GoatError::NetbirdFailed {
            hint: format!(
                "NetBird mgmt API answered HTTP {status}. Nothing was changed. Check --mgmt-url \
                 and retry."
            ),
        },
        NetbirdError::Transient(s) => GoatError::NetbirdFailed {
            hint: format!("{s}. Nothing was changed; retry once the management server answers."),
        },
    }
}

/// A portal error as a catalog-coded `GoatError`. The shared classifier
/// already returns one for every status a list read can meet; the fallback
/// keeps that true if it ever returns plain text.
fn map_portal_err(e: PortalError) -> GoatError {
    let status = e.http_status().map(|s| s.as_u16());
    let path = e.path().to_string();
    let err = super::vendor::map_portal_err(e);
    match err.downcast::<GoatError>() {
        Ok(g) => g,
        Err(other) => crate::errors::portal_answer(
            status.unwrap_or(400),
            &path,
            None,
            &format!("\n  server: {other}"),
        ),
    }
}

/// The operator-action record for one step of this verb.
pub(crate) fn audit_record(
    actor: &str,
    env: &str,
    outcome: &str,
    m: &MachineListItem,
    peer: &NbPeer,
    err: Option<String>,
) -> crate::cmd::audit::OperatorActionV1 {
    let digest = format!(
        "{ACTION} handle={} anchor={} node_class={} custodian={} peer_id={} peer={}",
        m.system_handle, m.slot_id, m.node_class, m.custodian, peer.id, peer.name
    );
    let mut a = crate::cmd::audit::OperatorActionV1::new(actor, "goat-cli", ACTION, &m.system_handle, env, outcome)
        .with_args_digest(&digest);
    if let Some(e) = err {
        a = a.with_error(e);
    }
    a
}

/// What the operator reads after the write.
pub(crate) fn result_lines(m: &MachineListItem, c: &PeerSshChange) -> Vec<String> {
    let mut lines = vec![format!("✓ NetBird SSH on for {} (peer {})", m.system_handle, c.name)];
    lines.push(
        "  the device's NetBird client must also allow it: `netbird up --allow-server-ssh` on the device"
            .to_string(),
    );
    let device_class = matches!(m.node_class.as_str(), "NETWORK" | "CONTROL" | "EDGE-SERVER");
    let expiring: Vec<&str> = [
        (c.login_expiration_enabled, "login"),
        (c.inactivity_expiration_enabled, "inactivity"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, k)| *k)
    .collect();
    if device_class && !expiring.is_empty() {
        lines.push(format!(
            "  note: {} expiration is on for this {} peer and was left as it was; it drops off the \
             mesh when that runs out (ADR 1216, alert DevicePeerCanExpire)",
            expiring.join(" and "),
            m.node_class
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputContext;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// No spinner, no colour: the tests read return values, not the terminal.
    const QUIET: OutputContext =
        OutputContext { color: false, stdout_is_tty: false, stderr_is_tty: false, ci: true };

    fn machine(handle: &str, slot: &str, custodian: &str, class: &str) -> MachineListItem {
        MachineListItem {
            slot_id: slot.into(),
            system_handle: handle.into(),
            custodian: custodian.into(),
            node_class: class.into(),
            autonomy: "manual".into(),
            affiliation: "unit-a".into(),
            workloads: vec![],
            enrolled_at: 0,
        }
    }

    fn peer(id: &str, name: &str) -> NbPeer {
        NbPeer { id: id.into(), name: name.into(), ..Default::default() }
    }

    const A1: &str = "0123456789abcdef0123456789abcdef";
    const A2: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn machine_resolves_by_handle_or_anchor() {
        let ms = vec![machine("switch-a", A1, "eng-a", "NETWORK"), machine("switch-b", A2, "eng-a", "NETWORK")];
        assert_eq!(resolve_machine(&ms, "switch-a").unwrap().slot_id, A1);
        assert_eq!(resolve_machine(&ms, A2).unwrap().system_handle, "switch-b");
    }

    #[test]
    fn unknown_and_ambiguous_machines_are_coded() {
        let ms = vec![machine("switch-a", A1, "eng-a", "NETWORK"), machine("switch-a", A2, "eng-b", "NETWORK")];
        let e = resolve_machine(&ms, "switch-z").unwrap_err();
        assert_eq!(e.code(), "E_MACHINE_UNRESOLVED");
        let e = resolve_machine(&ms, "switch-a").unwrap_err();
        assert_eq!(e.code(), "E_MACHINE_UNRESOLVED");
        assert!(e.hint().contains("eng-a, eng-b"), "{}", e.hint());
        // The ambiguous handle still resolves by anchor.
        assert_eq!(resolve_machine(&ms, A2).unwrap().custodian, "eng-b");
    }

    #[test]
    fn peer_name_follows_the_portal_rule() {
        assert!(owned_peer_name("machine-sw-1-host", "sw-1"));
        assert!(owned_peer_name("machine-sw-1", "sw-1"));
        assert!(owned_peer_name("sw-1", "sw-1"));
        assert!(!owned_peer_name("machine-sw-11-host", "sw-1"));
        assert!(!owned_peer_name("sw-1-laptop", "sw-1"));
        assert!(!owned_peer_name("machine-sw-1x", "sw-1"));
    }

    #[test]
    fn peer_resolution_refuses_none_and_several() {
        let peers = vec![peer("p1", "machine-sw-1-host"), peer("p2", "machine-sw-11-host"), peer("p3", "laptop")];
        assert_eq!(resolve_peer(&peers, "sw-1").unwrap().id, "p1");
        assert_eq!(resolve_peer(&peers, "sw-2").unwrap_err().code(), "E_MESH_PEER_UNRESOLVED");
        let dup = vec![peer("p1", "machine-sw-1-host"), peer("p4", "sw-1")];
        let e = resolve_peer(&dup, "sw-1").unwrap_err();
        assert_eq!(e.code(), "E_MESH_PEER_UNRESOLVED");
        assert!(e.hint().contains("p1") && e.hint().contains("p4"), "{}", e.hint());
    }

    #[test]
    fn every_netbird_error_maps_to_a_catalog_code() {
        use reqwest::StatusCode;
        let cases = [
            (NetbirdError::Disabled, "E_NETBIRD_FAILED"),
            (NetbirdError::NotFound, "E_MESH_PEER_UNRESOLVED"),
            (NetbirdError::Http { status: StatusCode::FORBIDDEN }, "E_NOT_ADMIN"),
            (NetbirdError::Http { status: StatusCode::UNAUTHORIZED }, "E_NETBIRD_FAILED"),
            (NetbirdError::Http { status: StatusCode::BAD_GATEWAY }, "E_NETBIRD_FAILED"),
            (NetbirdError::Transient("connection refused".into()), "E_NETBIRD_FAILED"),
        ];
        for (e, code) in cases {
            let g = map_nb_err(e, "sw-1");
            assert_eq!(g.code(), code);
            assert!(goat_errors::explain(code).is_some(), "{code} is not in ops/errors/catalog.yaml");
        }
    }

    #[test]
    fn the_new_codes_are_in_the_catalog_with_their_exit_codes() {
        for g in [
            GoatError::MachineUnresolved { machine: "x".into(), hint: String::new() },
            GoatError::MeshPeerUnresolved { machine: "x".into(), hint: String::new() },
            GoatError::NetbirdFailed { hint: String::new() },
        ] {
            let e = goat_errors::explain(g.code()).unwrap_or_else(|| panic!("{} missing", g.code()));
            assert_eq!(e.exit, Some(g.exit_code()), "{}", g.code());
        }
    }

    #[test]
    fn portal_refusals_stay_coded() {
        for status in [401u16, 403, 404, 409, 500] {
            let e = PortalError::status(reqwest::StatusCode::from_u16(status).unwrap());
            let g = map_portal_err(e);
            assert!(goat_errors::explain(g.code()).is_some(), "{status} -> {}", g.code());
        }
    }

    #[test]
    fn the_audit_record_names_the_person_the_machine_and_the_peer() {
        let m = machine("switch-a", A1, "eng-a", "NETWORK");
        let p = peer("p1", "machine-switch-a-host");
        let a = audit_record("maintainer-operator", "zagros-prod", "started", &m, &p, None);
        assert_eq!(a.actor, "maintainer-operator");
        assert_eq!(a.surface, "goat-cli");
        assert_eq!(a.action, "machine.ssh.enable");
        assert_eq!(a.target, "switch-a");
        assert_eq!(a.env, "zagros-prod");
        assert_eq!(a.outcome, "started");
        assert!(a.error_msg.is_empty());
        assert_eq!(a.args_digest.len(), 64, "a sha-256 hex digest");
        let f = audit_record("maintainer-operator", "zagros-prod", "failed", &m, &p, Some("boom".into()));
        assert_eq!(f.error_msg, "boom");
        assert_eq!(f.args_digest, a.args_digest, "same act, same digest");
    }

    #[test]
    fn result_warns_when_a_device_peer_can_expire() {
        let m = machine("switch-a", A1, "eng-a", "NETWORK");
        let c = PeerSshChange {
            name: "machine-switch-a-host".into(),
            changed: true,
            login_expiration_enabled: true,
            inactivity_expiration_enabled: false,
        };
        let lines = result_lines(&m, &c).join("\n");
        assert!(lines.contains("--allow-server-ssh"), "{lines}");
        assert!(lines.contains("login expiration is on"), "{lines}");
        let quiet = PeerSshChange { login_expiration_enabled: false, ..c.clone() };
        assert!(!result_lines(&m, &quiet).join("\n").contains("expiration"));
        let sensor = machine("cam-a", A2, "eng-a", "SENSOR-FEED");
        assert!(!result_lines(&sensor, &c).join("\n").contains("expiration"));
    }

    /// The whole read path against fixture servers: the portal's machine list
    /// and NetBird's peer list. No live call.
    #[tokio::test]
    async fn resolve_reads_the_portal_then_netbird() {
        let portal_srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/machines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "machines": [{
                    "slot_id": A1, "system_handle": "switch-a", "custodian": "eng-a",
                    "node_class": "NETWORK", "autonomy": "manual", "affiliation": "unit-a"
                }]
            })))
            .mount(&portal_srv)
            .await;
        let nb_srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/peers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "p1", "name": "machine-switch-a-host", "ip": "100.127.0.45", "ssh_enabled": false},
                {"id": "p2", "name": "machine-switch-ab-host", "ip": "100.127.0.46"}
            ])))
            .mount(&nb_srv)
            .await;
        let portal = PortalClient::new(portal_srv.uri(), "k".into());
        let nb = NetbirdClient::new(nb_srv.uri(), "t".into());
        let out = QUIET;
        let (m, p) = resolve(&out, &portal, &nb, "switch-a").await.unwrap();
        assert_eq!(m.slot_id, A1);
        assert_eq!(p.id, "p1");
        assert!(!p.ssh_enabled);

        let e = resolve(&out, &portal, &nb, "switch-z").await.unwrap_err();
        assert_eq!(e.code(), "E_MACHINE_UNRESOLVED");
    }

    #[tokio::test]
    async fn resolve_codes_a_netbird_403() {
        let portal_srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/machines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "machines": [{
                    "slot_id": A1, "system_handle": "switch-a", "custodian": "eng-a",
                    "node_class": "NETWORK", "autonomy": "manual", "affiliation": "unit-a"
                }]
            })))
            .mount(&portal_srv)
            .await;
        let nb_srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/peers"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&nb_srv)
            .await;
        let portal = PortalClient::new(portal_srv.uri(), "k".into());
        let nb = NetbirdClient::new(nb_srv.uri(), "t".into());
        let e = resolve(&QUIET, &portal, &nb, "switch-a").await.unwrap_err();
        assert_eq!(e.code(), "E_NOT_ADMIN");
    }
}

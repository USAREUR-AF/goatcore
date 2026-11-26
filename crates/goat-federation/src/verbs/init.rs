// init: our half of a federation, read from the running system. Reads
// local state; writes only the package file it is told to, and, once, on the
// upgrade path from builds before phase 6 kept it, <work>/ours.composed.json
// (agreed_offer).

use super::{
    emit_json, print_outcomes, refusal, sign_with, FedOutcome, Site, Verdict, EXIT_FED_UNDERIVED,
    EXIT_OK, FILE_UNWRITABLE,
};
use crate::derive;
use crate::json::{canonical, JVal};
use clap::Args;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Also derive `transport_offer` — that we are willing to connect.
    ///
    /// Omitted, the package carries no transport offer, which is a complete
    /// and valid declaration: presence is the declaration, and a party willing
    /// to be recognised and nothing more says so by carrying neither optional
    /// block.
    #[arg(long)]
    pub with_transport: bool,

    /// Also derive `data_offer` — that we are willing to exchange.
    #[arg(long)]
    pub with_data: bool,

    /// The package version to emit: v1 (the default) or v2. A peering selects
    /// it; both parties select the same one.
    #[arg(long, value_enum, default_value_t = super::PackageVersion::V1)]
    pub package_version: super::PackageVersion,

    /// Our Ed25519 signing key: 64 hex characters (the RFC 8032 seed) or 44
    /// base64 characters. Without it this prints the exact bytes to sign and
    /// stops — it will not invent a key.
    #[arg(long, value_name = "FILE")]
    pub signing_key: Option<PathBuf>,

    /// Write the package's canonical bytes here.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// One JSON object instead of the glyph lines.
    #[arg(long)]
    pub json: bool,

    /// Read the declaration from this file instead of federation.toml in the
    /// context directory: a peering's own federation.toml, which goatbuilder
    /// generates from the environment's. Identity is still read from the
    /// context directory.
    #[arg(long, value_name = "FILE")]
    pub declaration: Option<PathBuf>,
}

#[derive(Serialize)]
struct InitReport {
    verb: &'static str,
    verdict: Verdict,
    /// Members that could not be derived. The list IS the output of a failed
    /// init: it is what has to be true before this package can exist.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    underived: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    outcomes: Vec<FedOutcome>,
}

/// The declared half: organisational identity and the link parameters two
/// parties agree rather than one party observes. Read from a file that is
/// reviewed the way production configuration is — never prompted for.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct Declaration {
    pub(crate) sovereign_id: String,
    /// How long our half is offered for, in days from today.
    term_days: i64,
    cut_contact: String,
    #[serde(default)]
    transport: Option<DeclaredTransport>,
    #[serde(default)]
    data: Option<DeclaredData>,
}

#[derive(Debug, serde::Deserialize)]
struct DeclaredTransport {
    /// The federation interface. Its public key is READ from it, never set here.
    interface: String,
    endpoint: String,
    listener: bool,
    proposed_link_subnet: String,
    /// A measured path MTU. An interface MTU is not one and is never
    /// substituted for one — but it is cross-checked against the interface,
    /// because a path MTU larger than the interface carrying it cannot be right.
    measured_mtu: i64,
}

#[derive(Debug, serde::Deserialize)]
struct DeclaredData {
    #[serde(default)]
    release: Option<Vec<String>>,
    #[serde(default)]
    accept_hz: Option<i64>,
    #[serde(default)]
    accept_bytes_per_message: Option<i64>,
    #[serde(default)]
    accept_distinct_keys: Option<i64>,
}

const DECLARATION_TEMPLATE: &str = r#"# Our half of a federation: the part no probe of a running system can produce.
#
# `sovereign_id` is the organisation's name, from ISO 3166-1 alpha-3 lowercased
# (usa, isl, svk, svn), an established short name for a multinational body
# (nato), or a parent plus a local suffix (nato-jfcnp). It is NOT one of the
# organisation's environments: a sandbox and a production fabric share one
# sovereign_id, and putting an environment name here produces peering names that
# cannot survive that environment being retired.
#
# NOTHING in this file may name a capability. `goat federate init` measures
# capabilities.transports, capabilities.fabric_fips_image,
# capabilities.fabric_version, capabilities.pqc_capable and the transport public
# key from the running system, and refuses to read any of them from here — a
# capability typed by an operator is a claim, and a wrong claim stays wrong
# until the link fails in front of a partner.

sovereign_id = "xaa"
term_days    = 91
cut_contact  = "fed-ops@example"

# Optional. Present means "willing to connect"; --with-transport reads it.
# [transport]
# interface            = "wg-fed0"        # its public key is READ from it
# endpoint             = "_fed._udp.example"
# listener             = true
# proposed_link_subnet = "169.254.10.0/31"
# measured_mtu         = 1380             # from an actual path-MTU measurement

# Optional. Present means "willing to exchange"; --with-data reads it.
# Each half is complete once present, independently of the other: `release`
# alone is send-only, all three accept_* alone is receive-only.
# [data]
# release                  = ["release/weather/**"]
# accept_hz                = 100
# accept_bytes_per_message = 1048576
# accept_distinct_keys     = 5000
"#;

pub fn run_init(site: &Site, a: &InitArgs, out: &mut dyn Write) -> anyhow::Result<i32> {
    run_init_measured(site, a, out, &measure_here)
}

/// What `goat federate init` has always measured: this machine, and the core
/// router through this machine's container runtime (`DOCKER_HOST`,
/// `FED_ROUTER_CONTAINER`, read from the process environment).
pub fn measure_here(interface: Option<&str>) -> anyhow::Result<derive::Measured> {
    measure_here_with(&derive::RouterEnv::from_process(), interface)
}

/// [`measure_here`], with the core router's `DOCKER_HOST` and container given
/// as values. A driver that runs `init` in its own process passes them here
/// (to [`run_init_measured`]) instead of writing them into its environment.
pub fn measure_here_with(
    router: &derive::RouterEnv,
    interface: Option<&str>,
) -> anyhow::Result<derive::Measured> {
    let (fabric_fips_image, fabric_version) = derive::fabric_capabilities_with(router);
    Ok(derive::Measured {
        transports: derive::transports(),
        fabric_fips_image,
        fabric_version,
        pqc_capable: derive::pqc_capable(),
        public_key: interface.map(derive::transport_public_key),
        interface_mtu: interface.and_then(derive::interface_mtu),
        applied_mtu: interface.and_then(derive::applied_mtu),
        // Run on the core, this reads the SPIRE server's own config; anywhere
        // else it is absent and the profile alone must name the trust domain.
        spire_trust_domain: Some(derive::spire_trust_domain_on(
            &derive::Local,
            derive::SPIRE_SERVER_CONF,
        )),
    })
}

/// [`run_init`], with the measured half supplied by the caller. `measure` is
/// called once, after the declaration is read, with the federation interface
/// when the package offers a transport; goatbuilder passes one that reads the
/// edge and the core over SSH. Identity and the declaration are still read
/// from `site`, and signing still happens here. An `Err` from `measure` (a
/// host that cannot be reached) is returned as is, for the caller to print.
pub fn run_init_measured(
    site: &Site,
    a: &InitArgs,
    out: &mut dyn Write,
    measure: &dyn Fn(Option<&str>) -> anyhow::Result<derive::Measured>,
) -> anyhow::Result<i32> {
    let mut outcomes = Vec::new();
    let mut underived: Vec<String> = Vec::new();

    let decl_path = a.declaration.clone().unwrap_or_else(|| site.declaration_path());
    // Captured here and reused below instead of re-reading the same file a
    // second time — the second read used `.unwrap_or_default()`, so on ANY
    // failure of that redundant read (a TOCTOU remove between the two reads,
    // an unusual FS state) `decl_text` silently became empty, which switched
    // off the "measured-only field claimed" refusal check entirely rather
    // than erroring — a security-relevant check disabled by a swallowed
    // error, not a deliberate skip.
    let raw_decl_text = std::fs::read_to_string(&decl_path);
    let decl = match &raw_decl_text {
        Ok(s) => match toml::from_str::<Declaration>(s) {
            Ok(d) => d,
            Err(e) => {
                outcomes.push(FedOutcome::fail(
                    "declaration",
                    format!("{}: {e}", decl_path.display()),
                    "Fix the file. It is not prompted for and its values are not guessed at.",
                ));
                finish_init(
                    out,
                    a,
                    Verdict::Fail,
                    vec!["declaration".into()],
                    None,
                    outcomes,
                )?;
                return Ok(EXIT_FED_UNDERIVED);
            }
        },
        Err(_) => {
            let mut o = FedOutcome::fail(
                "declaration",
                format!("no {} at {}", derive::DECLARATION_FILE, decl_path.display()),
                "Write it, review it the way you would any other production configuration, and \
                 re-run. It holds only what no probe can produce — the organisation's name, the \
                 term, the cut contact, and the link parameters two parties agree rather than \
                 one observes. A template follows.",
            );
            o.block = Some(DECLARATION_TEMPLATE.to_string());
            outcomes.push(o);
            finish_init(
                out,
                a,
                Verdict::Fail,
                vec!["declaration".into()],
                None,
                outcomes,
            )?;
            return Ok(EXIT_FED_UNDERIVED);
        }
    };

    // A declaration that names a measured member is an error rather than an
    // override. This is the rule the whole verb turns on, so it fires before
    // anything is read.
    // Reuse the read from above (the `decl` match already proved this file
    // is present and parses) rather than re-reading and silently defaulting
    // to empty on any failure of that redundant read.
    let decl_text = raw_decl_text.unwrap_or_default();
    let claimed: Vec<&str> = derive::MEASURED_ONLY
        .iter()
        .copied()
        .filter(|m| {
            let leaf = m.rsplit('.').next().unwrap_or(m);
            decl_text.lines().any(|l| {
                let l = l.trim();
                !l.starts_with('#') && l.starts_with(leaf) && l.contains('=')
            })
        })
        .collect();
    if !claimed.is_empty() {
        outcomes.push(FedOutcome::fail(
            "measured-not-claimed",
            format!("{} names {claimed:?}", decl_path.display()),
            "These are measured from the running system, never read from configuration. A \
             capability typed by an operator is a claim; a capability read from the running \
             system is a fact. Remove them.",
        ));
        finish_init(
            out,
            a,
            Verdict::Fail,
            claimed.iter().map(|s| s.to_string()).collect(),
            None,
            outcomes,
        )?;
        return Ok(EXIT_FED_UNDERIVED);
    }
    outcomes.push(FedOutcome::ok(
        "declaration",
        format!(
            "{} — sovereign_id `{}`, {} day term, cut contact recorded",
            decl_path.display(),
            decl.sovereign_id,
            decl.term_days
        ),
    ));

    // --- capabilities, measured ---
    let interface = match (&decl.transport, a.with_transport) {
        (Some(t), true) => Some(t.interface.as_str()),
        _ => None,
    };
    let measured = measure(interface)?;
    // --- identity, from our own credentials, and the core's SPIRE server ---
    let td = derive::resolve_trust_domain(&site.cert_path, measured.spire_trust_domain.as_ref());
    let cafp = derive::ca_fingerprint(&site.ca_roots_path, &site.cert_path);
    let transports = measured.transports;
    let (fips, version) = (measured.fabric_fips_image, measured.fabric_version);
    let pqc = measured.pqc_capable;

    let mut members: Vec<(String, JVal)> = Vec::new();
    members.push((
        "spec".into(),
        JVal::Str(a.package_version.package_spec().into()),
    ));
    members.push(("sovereign_id".into(), JVal::Str(decl.sovereign_id.clone())));

    let today = chrono::Utc::now().date_naive();
    members.push(("issued".into(), JVal::Str(today.to_string())));
    let expires = today + chrono::Duration::days(decl.term_days);
    members.push(("expires".into(), JVal::Str(expires.to_string())));

    let td_v = take(&td, &mut outcomes, &mut underived, JVal::Str);
    let cafp_v = take(&cafp, &mut outcomes, &mut underived, JVal::Str);
    if let (Some(t), Some(c)) = (td_v, cafp_v) {
        members.push((
            "identity".into(),
            JVal::Obj(vec![
                ("ca_fingerprint".into(), c),
                ("trust_domain".into(), t),
            ]),
        ));
    }

    let tr_v = take(
        &transports,
        &mut outcomes,
        &mut underived,
        |v: Vec<String>| JVal::Arr(v.into_iter().map(JVal::Str).collect()),
    );
    let fips_v = take(&fips, &mut outcomes, &mut underived, JVal::Bool);
    let ver_v = take(&version, &mut outcomes, &mut underived, JVal::Str);
    let pqc_v = take(&pqc, &mut outcomes, &mut underived, JVal::Bool);
    if let (Some(tr), Some(fi), Some(ve), Some(pq)) = (tr_v, fips_v, ver_v, pqc_v) {
        members.push((
            "capabilities".into(),
            JVal::Obj(vec![
                ("fabric_fips_image".into(), fi),
                ("fabric_version".into(), ve),
                ("pqc_capable".into(), pq),
                ("transports".into(), tr),
            ]),
        ));
    }

    if a.with_transport {
        match &decl.transport {
            None => {
                outcomes.push(FedOutcome::fail(
                    "transport_offer",
                    format!(
                        "--with-transport, but {} has no [transport] section",
                        decl_path.display()
                    ),
                    "A transport offer needs the endpoint, the link subnet and a measured path \
                     MTU, none of which a probe of one host produces. Declare them, or drop \
                     --with-transport: a package with no transport offer is complete and valid.",
                ));
                underived.push("transport_offer".into());
            }
            Some(t) => {
                // `measure` was given this interface above, so it answered
                // for it; a measurer that did not is refused, not guessed.
                let pk = measured.public_key.unwrap_or_else(|| derive::Derived {
                    field: "transport_offer.public_key",
                    outcome: Err(format!(
                        "the measurement returned no public key for interface `{}`",
                        t.interface
                    )),
                    source: "the caller's measurement".into(),
                });
                match measured.interface_mtu {
                    // Phase 11 set the interface to the agreed MTU, min(both
                    // offers); below our offer it only says the partner offered
                    // less. The offer was cross-checked when it was first made,
                    // so it is skipped only while it is still that offer: our
                    // offer in the agreement the interface carries, read from
                    // phase 6's copy in <work> (where --out writes our package).
                    // A changed offer could be checked against nothing here,
                    // and is blocked.
                    Some(mtu) if measured.applied_mtu == Some(mtu) && t.measured_mtu > mtu => {
                        match agreed_offer(a.out.as_deref(), mtu) {
                            Ok(ours) if ours == t.measured_mtu => outcomes.push(FedOutcome::skip(
                                "transport_offer.measured_mtu",
                                format!(
                                    "interface `{}` carries the agreed MTU {mtu} an applied agreement set (min of both offers), and the declared {} is our offer in that agreement; not cross-checked again",
                                    t.interface, t.measured_mtu
                                ),
                            )),
                            found => {
                                let why = match found {
                                    Ok(ours) => format!("our offer in the agreement it carries was {ours}, and the declaration now says {}", t.measured_mtu),
                                    Err(e) => format!("whether the declared {} is our offer in the agreement it carries cannot be read: {e}", t.measured_mtu),
                                };
                                outcomes.push(FedOutcome::fail(
                                    "transport_offer.measured_mtu",
                                    format!(
                                        "blocked: interface `{}` holds MTU {mtu}, set by an applied agreement, so it cannot bound a new offer; {why}",
                                        t.interface
                                    ),
                                    "re-measure the path MTU off the federation interface (across the path itself, not \
                                     over the tunnel the agreement already sized) and declare that measured_mtu. An \
                                     offer above the real path goes into the next agreement as min(both sides), and \
                                     a link sized above its path silently drops large packets.",
                                ));
                                underived.push("transport_offer.measured_mtu".into());
                            }
                        }
                    }
                    Some(mtu) => {
                        if t.measured_mtu > mtu {
                            outcomes.push(FedOutcome::fail(
                                "transport_offer.measured_mtu",
                                format!(
                                    "declared {} exceeds interface `{}` MTU {mtu}",
                                    t.measured_mtu, t.interface
                                ),
                                "A path MTU cannot be larger than the interface carrying it, so this \
                                 number was not measured across the path. It goes into the agreement \
                                 as min(both sides), and a wrong one there is a silently broken link.",
                            ));
                            underived.push("transport_offer.measured_mtu".into());
                        }
                    }
                    // `interface_mtu` reads `/sys/class/net/<if>/mtu` — absent on
                    // any non-Linux host, or if the interface doesn't exist, or
                    // on a permission-denied read. This used to just skip the
                    // whole cross-check with no trace, which is indistinguishable
                    // from "checked and it passed" in the outcome list.
                    None => outcomes.push(FedOutcome::skip(
                        "transport_offer.measured_mtu",
                        format!(
                            "could not read the MTU of interface `{}` (non-Linux host, missing \
                             interface, or unreadable /sys) — the declared measured_mtu was NOT \
                             cross-checked against it",
                            t.interface
                        ),
                    )),
                }
                if let Some(k) = take(&pk, &mut outcomes, &mut underived, JVal::Str) {
                    members.push((
                        "transport_offer".into(),
                        JVal::Obj(vec![
                            ("endpoint".into(), JVal::Str(t.endpoint.clone())),
                            ("listener".into(), JVal::Bool(t.listener)),
                            ("measured_mtu".into(), JVal::Int(t.measured_mtu)),
                            (
                                "proposed_link_subnet".into(),
                                JVal::Str(t.proposed_link_subnet.clone()),
                            ),
                            ("public_key".into(), k),
                        ]),
                    ));
                }
            }
        }
    } else {
        outcomes.push(FedOutcome::skip(
            "transport_offer",
            "absent — this half is not offering to connect. Presence is the declaration, so \
             nothing needs to be claimed",
        ));
    }

    if a.with_data {
        match &decl.data {
            None => {
                outcomes.push(FedOutcome::fail(
                    "data_offer",
                    format!(
                        "--with-data, but {} has no [data] section",
                        decl_path.display()
                    ),
                    "A data offer needs a release set, or all three accept limits, or both. A \
                     limit that is not stated is a limit that does not exist, so none of them is \
                     defaulted.",
                ));
                underived.push("data_offer".into());
            }
            Some(d) => match build_data_offer(d) {
                Ok(v) => {
                    members.push(("data_offer".into(), v));
                    members.push((
                        "cut".into(),
                        JVal::Obj(vec![(
                            "contact".into(),
                            JVal::Str(decl.cut_contact.clone()),
                        )]),
                    ));
                    outcomes.push(FedOutcome::ok("data_offer", describe_declared_data(d)));
                }
                Err(why) => {
                    outcomes.push(FedOutcome::fail(
                        "data_offer",
                        why,
                        "Fix the [data] section.",
                    ));
                    underived.push("data_offer".into());
                }
            },
        }
    } else {
        outcomes.push(FedOutcome::skip(
            "data_offer",
            "absent — this half is not offering to exchange",
        ));
    }

    if !underived.is_empty() {
        outcomes.push(FedOutcome::fail(
            "package",
            format!(
                "{} value{} could not be derived",
                underived.len(),
                if underived.len() == 1 { "" } else { "s" }
            ),
            "Nothing is prompted for and nothing is filled in. Each line above says what is \
             missing and where it would have come from; make it true on this host, or run this \
             on the host where it is.",
        ));
        finish_init(out, a, Verdict::Fail, underived, None, outcomes)?;
        return Ok(EXIT_FED_UNDERIVED);
    }

    // Sign, or hand over the exact bytes to sign. The one thing not done here
    // is invent a key.
    let unsigned = JVal::Obj(members);
    let signing_input = canonical(&unsigned);
    match &a.signing_key {
        None => {
            let mut o = FedOutcome::fail(
                "signature",
                "no --signing-key, so the package is incomplete",
                "The exact bytes to sign with Ed25519 follow — the canonical form with no \
                 `signature` member, and no trailing newline. Put the base64 of the 64-byte \
                 result in a top-level `signature` member.",
            );
            o.block = Some(signing_input.clone());
            outcomes.push(o);
            finish_init(
                out,
                a,
                Verdict::Fail,
                vec!["signature".into()],
                None,
                outcomes,
            )?;
            Ok(EXIT_FED_UNDERIVED)
        }
        Some(path) => {
            let sig = sign_with(path, signing_input.as_bytes())?;
            let mut signed = match unsigned {
                JVal::Obj(m) => m,
                _ => unreachable!("built as an object"),
            };
            signed.push(("signature".into(), JVal::Str(sig)));
            let pkg = JVal::Obj(signed);
            let c = canonical(&pkg);
            outcomes.push(FedOutcome::ok(
                "signature",
                format!(
                    "Ed25519 over the canonical form ex-signature, from {}",
                    path.display()
                ),
            ));
            outcomes.push(FedOutcome::ok(
                "package",
                format!(
                    "`{}`, issued {today}, expires {expires} — {} bytes of canonical form",
                    decl.sovereign_id,
                    c.len()
                ),
            ));
            if let Some(out) = &a.out {
                std::fs::write(out, c.as_bytes())
                    .map_err(|e| refusal(FILE_UNWRITABLE, format!("{}: {e}", out.display())))?;
                outcomes.push(FedOutcome::ok(
                    "write",
                    format!("{} — canonical bytes, no trailing newline", out.display()),
                ));
            }
            finish_init(out, a, Verdict::Ok, Vec::new(), Some(c), outcomes)?;
            Ok(EXIT_OK)
        }
    }
}

fn build_data_offer(d: &DeclaredData) -> Result<JVal, String> {
    let mut m: Vec<(String, JVal)> = Vec::new();
    let accepts = [
        ("accept_bytes_per_message", d.accept_bytes_per_message),
        ("accept_distinct_keys", d.accept_distinct_keys),
        ("accept_hz", d.accept_hz),
    ];
    let present = accepts.iter().filter(|(_, v)| v.is_some()).count();
    if present != 0 && present != 3 {
        let missing: Vec<&str> = accepts
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        return Err(format!(
            "the receive half is all three accept values or none; [data] is missing {missing:?}. A \
             limit that is not stated is a limit that does not exist"
        ));
    }
    for (name, v) in accepts {
        if let Some(v) = v {
            m.push((name.into(), JVal::Int(v)));
        }
    }
    match &d.release {
        Some(r) if r.is_empty() => {
            return Err(
                "`release = []` is not a send half that carries nothing — an empty array \
                        is not a declaration. Remove the key, or list what you will publish"
                    .into(),
            )
        }
        Some(r) => m.push((
            "release".into(),
            JVal::Arr(
                crate::json::normalise_set(r)
                    .into_iter()
                    .map(JVal::Str)
                    .collect(),
            ),
        )),
        None => {}
    }
    if m.is_empty() {
        return Err(
            "[data] declares neither half. Give it a release set, or all three accept \
                    limits, or both"
                .into(),
        );
    }
    Ok(JVal::Obj(m))
}

fn describe_declared_data(d: &DeclaredData) -> String {
    match (&d.release, d.accept_hz) {
        (Some(r), Some(_)) => format!("sends {r:?}, and accepts within the declared limits"),
        (Some(r), None) => format!("send-only: {r:?}"),
        (None, Some(_)) => "receive-only: the three accept limits, and nothing released".into(),
        (None, None) => "declares neither half".into(),
    }
}

/// Move a derivation's value into the package, or record why it could not be.
fn take<T, F>(
    d: &derive::Derived<T>,
    outcomes: &mut Vec<FedOutcome>,
    underived: &mut Vec<String>,
    f: F,
) -> Option<JVal>
where
    T: Clone,
    F: FnOnce(T) -> JVal,
{
    match &d.outcome {
        Ok(v) => {
            outcomes.push(FedOutcome::ok(d.field, format!("read from {}", d.source)));
            Some(f(v.clone()))
        }
        Err(why) => {
            outcomes.push(FedOutcome::fail(
                d.field,
                format!("cannot derive from {}", d.source),
                why.clone(),
            ));
            underived.push(d.field.to_string());
            None
        }
    }
}

/// Our offer in the agreement the federation interface carries: `<work>` is
/// where `--out` writes our package. `<work>/agreement.json` must name the MTU
/// the interface was given (`applied`), and `<work>/ours.composed.json` is
/// phase 6's copy of the package that agreement was composed from, whose
/// `transport_offer.measured_mtu` is returned. ours.canonical.json is not
/// read for this: phase 3 rewrites it on every run, so after a retry that
/// stopped before compose it holds an offer no agreement used. A missing copy
/// is an error (blocked, fail closed), except on the upgrade path from builds
/// before the copy existed ([`upgrade_to_composed`]).
fn agreed_offer(out: Option<&std::path::Path>, applied: i64) -> Result<i64, String> {
    let work = out
        .and_then(|o| o.parent())
        .ok_or_else(|| "no --out, so no <work> holding the agreement".to_string())?;
    let read = |name: &str| -> Result<JVal, String> {
        let p = work.join(name);
        let text = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        crate::json::parse_document(&text).map_err(|e| format!("{}: {e:?}", p.display()))
    };
    let agreement = read("agreement.json")?;
    match agreement.get("transport").and_then(|t| t.get("mtu")).and_then(JVal::as_int) {
        Some(m) if m == applied => {}
        Some(m) => {
            return Err(format!(
                "{} names MTU {m}, not the {applied} the interface carries",
                work.join("agreement.json").display()
            ))
        }
        None => return Err(format!("{} has no transport.mtu", work.join("agreement.json").display())),
    }
    if !work.join(COMPOSED).exists() {
        upgrade_to_composed(work)?;
    }
    read(COMPOSED)
        .map_err(|e| format!("{e} (phase 6 keeps it beside the agreement; run phase 6 again to make it)"))?
        .get("transport_offer")
        .and_then(|t| t.get("measured_mtu"))
        .and_then(JVal::as_int)
        .ok_or_else(|| format!("{} offers no transport_offer.measured_mtu", work.join(COMPOSED).display()))
}

/// The upgrade path from builds before phase 6 kept ours.composed.json: a
/// peering they stood up has only ours.canonical.json and agreement.json, and
/// its first rotation re-runs phase 3 before any phase 6 could write the copy,
/// after phase 2 has already restarted the link. The agreement records neither
/// our offer nor our package's digest (it holds min(both offers) and no
/// digest), so the package is identified by order: phase 3 writes
/// ours.canonical.json and compose writes agreement.json after it, so a
/// package no newer than the agreement is the one it was composed from. That
/// package stands in once and the copy is written from it, so every later
/// phase 3 reads the copy. A package newer than the agreement was written by a
/// later phase 3 no agreement has used: nothing is written, and the caller
/// blocks.
fn upgrade_to_composed(work: &std::path::Path) -> Result<(), String> {
    let (ours, agreement) = (work.join("ours.canonical.json"), work.join("agreement.json"));
    let modified = |p: &std::path::Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let missing = format!("no {}", work.join(COMPOSED).display());
    if modified(&ours).map_err(|e| format!("{missing}, and {e}"))?
        > modified(&agreement).map_err(|e| format!("{missing}, and {e}"))?
    {
        return Err(format!(
            "{missing} (phase 6 keeps it beside the agreement), and {} is newer than {}: a later phase 3 wrote it, and no agreement has been composed from it; run phase 6 again to make the copy",
            ours.display(),
            agreement.display()
        ));
    }
    let tmp = work.join(".ours.composed.json.tmp");
    std::fs::copy(&ours, &tmp)
        .and_then(|_| std::fs::rename(&tmp, work.join(COMPOSED)))
        .map_err(|e| format!("{missing}, and it could not be written from {}: {e}", ours.display()))
}

/// Phase 6's copy of our package as composed into the agreement, in `<work>`.
const COMPOSED: &str = "ours.composed.json";

fn finish_init(
    out: &mut dyn Write,
    a: &InitArgs,
    verdict: Verdict,
    underived: Vec<String>,
    package: Option<String>,
    outcomes: Vec<FedOutcome>,
) -> std::io::Result<()> {
    if a.json {
        emit_json(
            out,
            &InitReport {
                verb: "init",
                verdict,
                underived,
                package,
                outcomes,
            },
        )
    } else {
        writeln!(out, "init — our half, read from the running system")?;
        print_outcomes(out, &outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(
        release: Option<Vec<&str>>,
        hz: Option<i64>,
        bpm: Option<i64>,
        keys: Option<i64>,
    ) -> DeclaredData {
        DeclaredData {
            release: release.map(|r| r.into_iter().map(str::to_string).collect()),
            accept_hz: hz,
            accept_bytes_per_message: bpm,
            accept_distinct_keys: keys,
        }
    }

    fn site_with(decl: &str) -> (tempfile::TempDir, Site) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(derive::DECLARATION_FILE), decl).unwrap();
        let site = Site {
            context_dir: dir.path().to_path_buf(),
            cert_path: dir.path().join("cert.pem"),
            ca_roots_path: dir.path().join("ca-roots.pem"),
        };
        (dir, site)
    }

    fn args(with_transport: bool) -> InitArgs {
        InitArgs {
            with_transport,
            with_data: false,
            package_version: super::super::PackageVersion::V1,
            signing_key: None,
            out: None,
            json: true,
            declaration: None,
        }
    }

    const DECL: &str = "sovereign_id = \"xaa\"\nterm_days = 30\ncut_contact = \"fed-ops@example.org\"\n\n[transport]\ninterface = \"wg-fed0\"\nendpoint = \"192.0.2.10:51820\"\nlistener = true\nproposed_link_subnet = \"169.254.77.0/31\"\nmeasured_mtu = 1380\n";

    fn fixed(interface: Option<&str>) -> derive::Measured {
        derive::Measured {
            transports: derive::Derived {
                field: "capabilities.transports",
                outcome: Ok(vec!["wireguard".into()]),
                source: "on edge-a".into(),
            },
            fabric_fips_image: derive::Derived {
                field: "capabilities.fabric_fips_image",
                outcome: Ok(false),
                source: "on core-a".into(),
            },
            fabric_version: derive::Derived {
                field: "capabilities.fabric_version",
                outcome: Ok("1.9.0".into()),
                source: "on core-a".into(),
            },
            pqc_capable: derive::Derived {
                field: "capabilities.pqc_capable",
                outcome: Ok(false),
                source: "on edge-a".into(),
            },
            public_key: interface.map(|_| derive::Derived {
                field: "transport_offer.public_key",
                outcome: Ok("PUBKEYfixture=".into()),
                source: "on edge-a".into(),
            }),
            interface_mtu: interface.map(|_| 1420),
            applied_mtu: None,
            spire_trust_domain: None,
        }
    }

    /// The caller's measurement is asked once, for the declared interface
    /// only when a transport is offered, and its values reach the report.
    #[test]
    fn init_takes_its_measured_half_from_the_caller() {
        let (_d, site) = site_with(DECL);
        for (with_transport, want) in [(true, Some("wg-fed0")), (false, None)] {
            let asked = std::cell::RefCell::new(Vec::new());
            let measure = |i: Option<&str>| {
                asked.borrow_mut().push(i.map(str::to_string));
                Ok(fixed(i))
            };
            let mut out = Vec::new();
            // No profile certificate in the fixture: identity is underived, so
            // the run ends at exit 23 after measuring, with the sources shown.
            let code = run_init_measured(&site, &args(with_transport), &mut out, &measure).unwrap();
            assert_eq!(code, super::super::EXIT_FED_UNDERIVED);
            assert_eq!(*asked.borrow(), vec![want.map(str::to_string)]);
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.contains("on core-a") && text.contains("on edge-a"),
                "{text}"
            );
        }
    }

    /// `--declaration` names the file the declared half is read from; the
    /// context directory's federation.toml is then not read at all.
    #[test]
    fn init_reads_the_declaration_it_is_named() {
        // The context's own file would fail to parse, so reading it fails the run.
        let (_d, site) = site_with("not toml [");
        let peering = tempfile::tempdir().unwrap();
        let named = peering.path().join("federation.toml");
        std::fs::write(&named, DECL.replace("wg-fed0", "wg-fed3")).unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        let measure = |i: Option<&str>| {
            asked.borrow_mut().push(i.map(str::to_string));
            Ok(fixed(i))
        };
        let a = InitArgs {
            declaration: Some(named.clone()),
            ..args(true)
        };
        let mut out = Vec::new();
        let code = run_init_measured(&site, &a, &mut out, &measure).unwrap();
        // Identity is underived (no certificate in the fixture), the declaration is not.
        assert_eq!(code, super::super::EXIT_FED_UNDERIVED);
        assert_eq!(*asked.borrow(), vec![Some("wg-fed3".to_string())]);
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("not toml"), "{text}");

        // Without it, the context's file is the declaration, as before.
        let mut out = Vec::new();
        run_init_measured(&site, &args(true), &mut out, &measure).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(&site.declaration_path().display().to_string()), "{text}");

        // A named file that is missing says which file.
        let gone = peering.path().join("missing.toml");
        let a = InitArgs {
            declaration: Some(gone.clone()),
            ..args(true)
        };
        let mut out = Vec::new();
        run_init_measured(&site, &a, &mut out, &measure).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(&gone.display().to_string()), "{text}");
    }

    /// One phase-3 re-run after phase 11: the interface MTU is `interface`,
    /// the role's state file says an agreement applied `applied`, the
    /// declaration offers `declared`, and `<work>` (where `--out` writes our
    /// package) holds the agreement the interface carries (`agreed_mtu`) and
    /// phase 6's copy of the package it was composed from,
    /// `ours.composed.json` (`our_offer`). Returns init's output.
    fn rerun(
        declared: i64,
        interface: i64,
        applied: Option<i64>,
        agreed_mtu: Option<i64>,
        our_offer: Option<i64>,
    ) -> String {
        rerun_at(declared, interface, applied, agreed_mtu, our_offer, our_offer)
    }

    /// [`rerun`], with `ours.canonical.json` (what phase 3 last wrote, on
    /// every run, after the agreement) offering `canonical`.
    fn rerun_at(
        declared: i64,
        interface: i64,
        applied: Option<i64>,
        agreed_mtu: Option<i64>,
        our_offer: Option<i64>,
        canonical: Option<i64>,
    ) -> String {
        let (_d, site) = site_with(&DECL.replace("measured_mtu = 1380", &format!("measured_mtu = {declared}")));
        let work = tempfile::tempdir().unwrap();
        if let Some(m) = agreed_mtu {
            std::fs::write(
                work.path().join("agreement.json"),
                format!(r#"{{"transport": {{"kind": "wireguard", "listener": "xaa", "mtu": {m}}}}}"#),
            )
            .unwrap();
        }
        let package = |o: i64| {
            format!(r#"{{"sovereign_id": "xaa", "transport_offer": {{"endpoint": "192.0.2.10:51820", "listener": true, "measured_mtu": {o}}}}}"#)
        };
        if let Some(o) = our_offer {
            std::fs::write(work.path().join("ours.composed.json"), package(o)).unwrap();
        }
        // Phase 3 rewrites our package on every run, so it is newer than the
        // agreement after any retry; its age says nothing.
        if let Some(o) = canonical {
            std::fs::write(work.path().join("ours.canonical.json"), package(o)).unwrap();
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
            std::fs::File::options()
                .write(true)
                .open(work.path().join("ours.canonical.json"))
                .unwrap()
                .set_modified(later)
                .unwrap();
        }
        let measure = |i: Option<&str>| {
            Ok(derive::Measured {
                interface_mtu: i.map(|_| interface),
                applied_mtu: applied,
                ..fixed(i)
            })
        };
        let mut a = args(true);
        a.out = Some(work.path().join("ours.canonical.json"));
        let mut out = Vec::new();
        run_init_measured(&site, &a, &mut out, &measure).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// After phase 11 the federation interface carries the agreed MTU,
    /// min(both offers). A re-run of phase 3 after a carrier rotation reads an
    /// interface MTU below our offer whenever the partner offered less: that
    /// bounds nothing about our offer, so while our declared offer is the one
    /// we made in the agreement the interface carries, it is a SKIP naming the
    /// agreement. With no applied agreement, an interface below the offer is
    /// still refused, and an applied MTU that is not the interface's says
    /// nothing about it.
    #[test]
    fn an_unchanged_offer_over_the_agreed_mtu_is_skipped_after_a_carrier_rotation() {
        let before = rerun(1380, 1300, None, Some(1300), Some(1380));
        assert!(before.contains("declared 1380 exceeds interface `wg-fed0` MTU 1300"), "{before}");
        let after = rerun(1380, 1300, Some(1300), Some(1300), Some(1380));
        assert!(!after.contains("exceeds interface"), "{after}");
        assert!(after.contains("carries the agreed MTU 1300"), "{after}");
        let other = rerun(1380, 1300, Some(1340), Some(1300), Some(1380));
        assert!(other.contains("declared 1380 exceeds interface `wg-fed0` MTU 1300"), "{other}");
    }

    /// The same re-run after `rotate <partner> transport`: on the side behind
    /// NAT, phase 2 restarts the link onto the new key, and phase 3 then reads
    /// the agreed 1300 on wg-fed0 under our unchanged offer of 1380. A
    /// rotation changes a key, not the offer, so it is the same SKIP.
    #[test]
    fn an_unchanged_offer_over_the_agreed_mtu_is_skipped_after_a_transport_rotation() {
        let out = rerun(1380, 1300, Some(1300), Some(1300), Some(1380));
        assert!(!out.contains("exceeds interface"), "{out}");
        assert!(out.contains("carries the agreed MTU 1300"), "{out}");
        assert!(out.contains("1380"), "{out}");
    }

    /// The interface carries an agreed MTU, so it cannot bound a NEW offer:
    /// applied 1300, our offer in that agreement 1380, and the declaration now
    /// raised to 1500 on a path that is really 1380. Skipping would publish
    /// 1500 unchecked, and the next agreement could black-hole the link; it is
    /// refused as blocked, asking for a measurement off the federation
    /// interface. The same when the agreement or our package in <work> cannot
    /// be read, or names another MTU than the one applied.
    #[test]
    fn a_changed_offer_over_the_agreed_mtu_is_blocked() {
        let raised = rerun(1500, 1300, Some(1300), Some(1300), Some(1380));
        assert!(!raised.contains("carries the agreed MTU"), "{raised}");
        assert!(raised.contains("blocked"), "{raised}");
        assert!(raised.contains("re-measure"), "{raised}");
        assert!(raised.contains("1500") && raised.contains("1380"), "{raised}");
        for (agreed, ours) in [(None, Some(1380)), (Some(1300), None), (Some(1280), Some(1380))] {
            let out = rerun(1380, 1300, Some(1300), agreed, ours);
            assert!(!out.contains("carries the agreed MTU"), "{agreed:?} {ours:?}: {out}");
            assert!(out.contains("blocked") && out.contains("re-measure"), "{agreed:?} {ours:?}: {out}");
        }
    }

    /// Phase 3 on `work` as it stands: the interface MTU is `interface`, the
    /// state file's applied MTU `applied`, the declaration offers `declared`.
    fn phase3_in(work: &std::path::Path, declared: i64, interface: i64, applied: Option<i64>) -> String {
        let (_d, site) = site_with(&DECL.replace("measured_mtu = 1380", &format!("measured_mtu = {declared}")));
        let measure = |i: Option<&str>| {
            Ok(derive::Measured {
                interface_mtu: i.map(|_| interface),
                applied_mtu: applied,
                ..fixed(i)
            })
        };
        let mut a = args(true);
        a.out = Some(work.join("ours.canonical.json"));
        let mut out = Vec::new();
        run_init_measured(&site, &a, &mut out, &measure).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// A peering stood up by a build before phase 6 kept its copy:
    /// agreement.json (MTU `agreed`) and ours.canonical.json (offering
    /// `canonical`, written `canonical_age` seconds before the agreement when
    /// positive, after it when negative), and no ours.composed.json.
    fn stood_up_before_the_copy(agreed: i64, canonical: i64, canonical_age: i64) -> tempfile::TempDir {
        let work = tempfile::tempdir().unwrap();
        std::fs::write(
            work.path().join("agreement.json"),
            format!(r#"{{"transport": {{"kind": "wireguard", "listener": "xaa", "mtu": {agreed}}}}}"#),
        )
        .unwrap();
        std::fs::write(
            work.path().join("ours.canonical.json"),
            format!(r#"{{"sovereign_id": "xaa", "transport_offer": {{"endpoint": "192.0.2.10:51820", "listener": true, "measured_mtu": {canonical}}}}}"#),
        )
        .unwrap();
        let agreement_at = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        let canonical_at = if canonical_age >= 0 {
            agreement_at - std::time::Duration::from_secs(canonical_age as u64)
        } else {
            agreement_at + std::time::Duration::from_secs((-canonical_age) as u64)
        };
        for (name, at) in [("agreement.json", agreement_at), ("ours.canonical.json", canonical_at)] {
            std::fs::File::options().write(true).open(work.path().join(name)).unwrap().set_modified(at).unwrap();
        }
        work
    }

    /// The upgrade path from builds before phase 6 kept its copy: with no
    /// ours.composed.json, an ours.canonical.json no newer than the agreement
    /// is the package the agreement was composed from (phase 3 wrote it, then
    /// compose ran), so it stands in once, and the copy is written from it.
    /// The same time (written in the same second) counts as not newer.
    #[test]
    fn with_no_copy_a_package_no_newer_than_the_agreement_stands_in_once() {
        for age in [60, 0] {
            let work = stood_up_before_the_copy(1300, 1380, age);
            let before = std::fs::read(work.path().join("ours.canonical.json")).unwrap();
            let out = phase3_in(work.path(), 1380, 1300, Some(1300));
            assert!(out.contains("carries the agreed MTU 1300"), "age {age}: {out}");
            assert!(!out.contains("blocked"), "age {age}: {out}");
            assert_eq!(
                std::fs::read(work.path().join("ours.composed.json")).ok(),
                Some(before),
                "age {age}: the copy was not written from the package"
            );
        }
    }

    /// With no copy, a package newer than the agreement was written by a
    /// later phase 3 no agreement has been composed from: blocked, and no copy
    /// is written from it.
    #[test]
    fn with_no_copy_a_package_newer_than_the_agreement_is_blocked() {
        let work = stood_up_before_the_copy(1300, 1380, -60);
        let out = phase3_in(work.path(), 1380, 1300, Some(1300));
        assert!(!out.contains("carries the agreed MTU"), "{out}");
        assert!(out.contains("blocked") && out.contains("re-measure"), "{out}");
        assert!(!work.path().join("ours.composed.json").exists(), "a copy was written from a newer package");
    }

    /// `rotate <partner> carrier` (or transport) on a peering stood up before
    /// the copy existed: phase 2 has rotated the key and restarted the link,
    /// and phase 3 re-runs with the interface at the agreed 1300 under our
    /// unchanged 1380. It passes on the stand-in package and writes the copy.
    /// Phase 3 then writes our new package (newer than the agreement); a
    /// second phase 3 before phase 6 (the run stopped at 4 or 5) reads the
    /// copy and passes again, where the fallback alone would block it.
    #[test]
    fn a_rotation_on_a_peering_without_the_copy_passes_phase_3_and_keeps_passing() {
        let work = stood_up_before_the_copy(1300, 1380, 60);
        let first = phase3_in(work.path(), 1380, 1300, Some(1300));
        assert!(first.contains("carries the agreed MTU 1300"), "{first}");
        assert!(work.path().join("ours.composed.json").is_file(), "no copy after the first phase 3");
        // Phase 3 rewrote our package (its --out), after the agreement.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::write(
            work.path().join("ours.canonical.json"),
            r#"{"sovereign_id": "xaa", "transport_offer": {"endpoint": "192.0.2.10:51820", "listener": true, "measured_mtu": 1380}}"#,
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(work.path().join("ours.canonical.json"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        let second = phase3_in(work.path(), 1380, 1300, Some(1300));
        assert!(second.contains("carries the agreed MTU 1300"), "{second}");
        assert!(!second.contains("blocked"), "{second}");
    }

    /// An honest retry: a run passed phase 3, stopped before phase 6 (a lint
    /// failure at 4, phase 5 waiting, a kill), and runs again. Phase 3 rewrote
    /// ours.canonical.json each time, so it is newer than the agreement, and
    /// may hold another offer; the offer the interface carries is phase 6's
    /// copy beside the agreement. Declared equal to that copy: a SKIP.
    #[test]
    fn an_honest_retry_after_a_stop_before_compose_is_skipped() {
        for canonical in [Some(1380), Some(1500), None] {
            let out = rerun_at(1380, 1300, Some(1300), Some(1300), Some(1380), canonical);
            assert!(out.contains("carries the agreed MTU 1300"), "{canonical:?}: {out}");
            assert!(!out.contains("blocked"), "{canonical:?}: {out}");
        }
    }

    /// The offer is read from phase 6's copy only: with no
    /// ours.composed.json beside the agreement, the cross-check is blocked
    /// (fail closed), even when ours.canonical.json offers the declared value;
    /// and a raised offer is blocked whatever ours.canonical.json says.
    #[test]
    fn the_offer_is_read_from_phase_6s_copy_only() {
        let missing = rerun_at(1380, 1300, Some(1300), Some(1300), None, Some(1380));
        assert!(!missing.contains("carries the agreed MTU"), "{missing}");
        assert!(missing.contains("blocked") && missing.contains("re-measure"), "{missing}");
        assert!(missing.contains("ours.composed.json"), "{missing}");
        let raised = rerun_at(1500, 1300, Some(1300), Some(1300), Some(1380), Some(1500));
        assert!(!raised.contains("carries the agreed MTU"), "{raised}");
        assert!(raised.contains("blocked") && raised.contains("re-measure"), "{raised}");
    }

    /// A host that cannot be reached is the caller's error, returned as is.
    #[test]
    fn a_measurement_that_fails_is_returned_not_reported_as_underived() {
        let (_d, site) = site_with(DECL);
        let measure = |_: Option<&str>| Err(anyhow::anyhow!("GB-111 could not reach xaa-edge"));
        let e = run_init_measured(&site, &args(true), &mut Vec::new(), &measure).unwrap_err();
        assert!(e.to_string().contains("could not reach xaa-edge"), "{e}");
    }

    /// A limit that is not stated is a limit that does not exist, so a
    /// half-stated receive half is refused rather than completed.
    #[test]
    fn a_partial_receive_half_is_refused() {
        let err = build_data_offer(&declared(None, Some(100), Some(1024), None)).unwrap_err();
        assert!(err.contains("accept_distinct_keys"), "{err}");
    }

    /// One-way federation is a real case, not a degenerate one.
    #[test]
    fn each_half_is_complete_on_its_own() {
        let send = build_data_offer(&declared(
            Some(vec!["release/weather/**"]),
            None,
            None,
            None,
        ))
        .unwrap();
        assert!(send.get("release").is_some());
        assert!(send.get("accept_hz").is_none());

        let recv = build_data_offer(&declared(None, Some(100), Some(1024), Some(5))).unwrap();
        assert!(recv.get("release").is_none());
        assert!(recv.get("accept_hz").is_some());
    }

    /// An empty array is not a declaration — not a send half that happens to
    /// carry nothing.
    #[test]
    fn an_empty_release_is_refused_rather_than_read_as_send_only() {
        let err = build_data_offer(&declared(Some(vec![]), None, None, None)).unwrap_err();
        assert!(err.contains("not a send half"), "{err}");
    }

    #[test]
    fn a_data_offer_with_neither_half_is_refused() {
        let err = build_data_offer(&declared(None, None, None, None)).unwrap_err();
        assert!(err.contains("neither half"), "{err}");
    }

    /// The declared release set is normalised on the way in, so a package this
    /// tool signs never carries the unsorted form the signer is obliged to
    /// avoid.
    #[test]
    fn a_declared_release_set_is_normalised_before_signing() {
        let v = build_data_offer(&declared(
            Some(vec!["b/**", "a/**", "b/**"]),
            None,
            None,
            None,
        ))
        .unwrap();
        let items: Vec<&str> = v
            .get("release")
            .and_then(JVal::as_arr)
            .unwrap()
            .iter()
            .filter_map(JVal::as_str)
            .collect();
        assert_eq!(items, vec!["a/**", "b/**"]);
    }
}

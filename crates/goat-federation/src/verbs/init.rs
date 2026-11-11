// init: our half of a federation, read from the running system. Reads
// local state; writes only the package file it is told to.

use super::{
    emit_json, print_outcomes, sign_with, FedOutcome, Site, Verdict, EXIT_FED_UNDERIVED, EXIT_OK,
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
/// `FED_ROUTER_CONTAINER`).
pub fn measure_here(interface: Option<&str>) -> anyhow::Result<derive::Measured> {
    let (fabric_fips_image, fabric_version) = derive::fabric_capabilities();
    Ok(derive::Measured {
        transports: derive::transports(),
        fabric_fips_image,
        fabric_version,
        pqc_capable: derive::pqc_capable(),
        public_key: interface.map(derive::transport_public_key),
        interface_mtu: interface.and_then(derive::interface_mtu),
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
        JVal::Str(crate::package::PACKAGE_SPEC.into()),
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
                    .map_err(|e| anyhow::anyhow!("{}: {e}", out.display()))?;
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

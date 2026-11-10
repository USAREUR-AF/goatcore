// `init`'s derivations — read the running system, never ask.
//
// THE RULE THIS MODULE EXISTS TO ENFORCE. A capability typed by an operator is
// a claim; a capability read from the running system is a fact. A package is
// reviewed by people who cannot check it against the machine it describes, so
// a claim that is wrong stays wrong until the link fails. Every
// `capabilities.*` member below is therefore MEASURED, and this module refuses
// to read any of them from configuration — a declaration file that sets one is
// an error, not an override.
//
// WHERE A VALUE GENUINELY CANNOT BE DERIVED, THE DERIVATION FAILS AND NAMES
// WHAT IS MISSING. It is never prompted for and never defaulted: a silent
// default is how a link ends up with no rate limit (§3).
//
// The line between the two halves:
//
//   MEASURED, and refused from the file:
//     capabilities.transports        what is installed and configured
//     capabilities.fabric_fips_image the running router's image
//     capabilities.fabric_version    the running router
//     capabilities.pqc_capable       whether Rosenpass is actually running
//     transport_offer.public_key     the federation interface's own key
//
//   DECLARED, in a file that is reviewed the way production configuration is:
//     sovereign_id, the term, the cut contact, and the link parameters that
//     are agreed between two parties rather than observed on one — an
//     endpoint, a link subnet, a measured path MTU, the release set and the
//     accept limits.
//
// `sovereign_id` is in the second list because it is not a capability. It is
// the organisation's name, taken from ISO 3166-1 (§3), and no probe of a
// running system produces it.

use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Where the reads happen
// ---------------------------------------------------------------------------

/// The one way `derive` touches a host: run a read-only command, or read a
/// file. [`Local`] is this machine; a caller supplies another implementation to
/// measure a host it reaches some other way (goatbuilder measures the edge and
/// the core over SSH, `crates/goatbuilder/src/federate/measure.rs`). The
/// derivations below are written once, against this trait, so a package
/// measured from the operator's machine and one measured on the edge itself
/// come from the same code.
///
/// The privilege boundary stays where it was: every command `derive` issues is
/// listed by [`is_read_only_probe`], and an implementation that reaches another
/// host is expected to refuse anything else.
pub trait Probe {
    /// Run `bin args…`. A non-zero exit is an error carrying stderr, never an
    /// empty answer.
    fn run(&self, bin: &str, args: &[&str]) -> Result<String, String>;
    /// Run `bin args…` whose answer is on stdout even when it exits non-zero
    /// (`systemctl is-active`). Still an error when nothing is on stdout.
    fn run_reading_output(&self, bin: &str, args: &[&str]) -> Result<String, String>;
    /// Read a file. `NotFound` and `PermissionDenied` are kept apart: they are
    /// different facts (see [`transports_on`]).
    fn read_file(&self, path: &str) -> std::io::Result<String>;
    /// Where the reads happen, appended to each `source` so a reviewer can
    /// re-run the same read in the same place. Empty for this machine.
    fn place(&self) -> String {
        String::new()
    }
}

/// This machine: the reads `goat federate init` has always made.
pub struct Local;

impl Probe for Local {
    fn run(&self, bin: &str, args: &[&str]) -> Result<String, String> {
        run(bin, args)
    }
    fn run_reading_output(&self, bin: &str, args: &[&str]) -> Result<String, String> {
        run_reading_output(bin, args)
    }
    fn read_file(&self, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// Every command `derive` issues, and nothing else. A [`Probe`] that reaches
/// another host checks each command against this before sending it, so the
/// derivations cannot be turned into a way to change a host: they read.
///
/// `docker exec <id> zenohd --version` is the one `exec`, and it is allowed
/// only in exactly that shape. `cat` is how a remote [`Probe::read_file`] reads,
/// and only under the two trees `derive` reads.
pub fn is_read_only_probe(bin: &str, args: &[&str]) -> bool {
    match (bin, args) {
        // Exactly the two `wg show` reads: `wg show all private-key` is a read
        // too, and it is the one read that must never leave the host.
        ("wg", ["show", "interfaces"]) => true,
        ("wg", ["show", interface, "public-key"]) => interface_name(interface),
        ("swanctl" | "ipsec", ["--version"]) => true,
        ("systemctl", ["is-active", "rosenpass"]) => true,
        ("systemctl", ["list-unit-files", "rosenpass.service"]) => true,
        // The listing and the inspection in the formats derive asks for, so a
        // container's environment (its secrets) is never printed.
        ("docker" | "podman", ["ps", "--no-trunc", "--format", ROUTER_LIST_FORMAT]) => true,
        ("docker" | "podman", ["inspect", "--type", "container", "--format", format, name]) => {
            (*format == ROUTER_INSPECT_FORMAT || *format == PODMAN_INSPECT_FORMAT)
                && container_name(name)
        }
        ("docker" | "podman", ["exec", id, "zenohd" | "/zenohd", "--version"]) => {
            container_name(id)
        }
        ("cat", ["--", path]) => {
            (path.starts_with("/proc/sys/crypto/") || path.starts_with("/sys/class/net/"))
                && !path.contains("..")
        }
        _ => false,
    }
}

fn at(probe: &dyn Probe) -> String {
    let place = probe.place();
    if place.is_empty() {
        place
    } else {
        format!(" {place}")
    }
}

/// The declaration file, beside the profile it belongs to.
pub const DECLARATION_FILE: &str = "federation.toml";

/// The measured members. Naming one of these in the declaration file is an
/// error: it would turn a fact into a claim, silently.
pub const MEASURED_ONLY: [&str; 5] = [
    "capabilities.transports",
    "capabilities.fabric_fips_image",
    "capabilities.fabric_version",
    "capabilities.pqc_capable",
    "transport.public_key",
];

/// Stable role label; names remain deployment configuration.
const CORE_ROLE: &str = "zenoh-core";
const ROUTER_LIST_FORMAT: &str = "{{.ID}}\t{{.Names}}\t{{.Label \"org.goat.component\"}}";
const ROUTER_INSPECT_FORMAT: &str = "{{.Id}}\t{{.State.Running}}\t{{.Config.Image}}";
/// Podman's template struct spells the field ID; Docker spells it Id.
const PODMAN_INSPECT_FORMAT: &str = "{{.ID}}\t{{.State.Running}}\t{{.Config.Image}}";

#[derive(Debug)]
struct Router {
    tool: &'static str,
    id: String,
    image: String,
}

/// One derived value, with the source it came from — or the reason it could
/// not be derived. `source` is printed on success so a reviewer can re-run the
/// same read; `hint` is printed on failure and says what is missing.
#[derive(Debug, Clone)]
pub struct Derived<T> {
    pub field: &'static str,
    pub outcome: Result<T, String>,
    pub source: String,
}

impl<T> Derived<T> {
    fn ok(field: &'static str, value: T, source: impl Into<String>) -> Self {
        Derived {
            field,
            outcome: Ok(value),
            source: source.into(),
        }
    }
    fn fail(field: &'static str, why: impl Into<String>, source: impl Into<String>) -> Self {
        Derived {
            field,
            outcome: Err(why.into()),
            source: source.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Identity — from the profile's own credentials
// ---------------------------------------------------------------------------

/// `identity.trust_domain` — the SPIFFE trust domain out of the profile leaf
/// certificate's URI subject-alternative name. Derived rather than declared
/// because the certificate is what a counterparty's edge will actually see.
pub fn trust_domain(cert_pem: &Path) -> Derived<String> {
    const FIELD: &str = "identity.trust_domain";
    let src = cert_pem.display().to_string();
    // NotFound/IsADirectory/PermissionDenied were all collapsed into one
    // "run profile init" remedy — correct for the first, wrong for the
    // other two (a permission problem needs a chown/ownership fix, not a
    // re-init, and re-init would just fail the same way again).
    let bytes = match std::fs::read(cert_pem) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Derived::fail(
                FIELD,
                format!(
                    "profile certificate at {src} exists but is not readable by this user \
                     (permission denied) — fix ownership/permissions, not `profile init`"
                ),
                src,
            );
        }
        Err(_) => {
            return Derived::fail(
                FIELD,
                format!(
                    "no profile certificate at {src}. Run `goat profile init <bundle.cbor>` first"
                ),
                src,
            );
        }
    };
    let Ok(pem) = pem::parse(&bytes) else {
        return Derived::fail(FIELD, "the profile certificate is not readable PEM", src);
    };
    let der = pem.contents();
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(der) else {
        return Derived::fail(FIELD, "the profile certificate is not readable X.509", src);
    };
    let mut uris: Vec<String> = Vec::new();
    if let Ok(Some(san)) = cert.tbs_certificate.subject_alternative_name() {
        for name in &san.value.general_names {
            if let x509_parser::extensions::GeneralName::URI(u) = name {
                uris.push((*u).to_string());
            }
        }
    }
    let spiffe = uris.iter().find(|u| u.starts_with("spiffe://"));
    match spiffe {
        Some(u) => {
            // spiffe://<trust-domain>/<path…> — the trust domain is the
            // authority component and nothing below it.
            let rest = &u["spiffe://".len()..];
            let domain = rest.split('/').next().unwrap_or_default();
            if domain.is_empty() {
                return Derived::fail(
                    FIELD,
                    format!("the URI-SAN `{u}` names no trust domain"),
                    src,
                );
            }
            Derived::ok(
                FIELD,
                format!("spiffe://{domain}"),
                format!("{src} URI-SAN"),
            )
        }
        None => Derived::fail(
            FIELD,
            format!(
                "the profile certificate carries no `spiffe://` URI-SAN (it has {uris:?}). The \
                 trust domain is read from the certificate a counterparty's edge will actually \
                 see, so there is nowhere else to take it from"
            ),
            src,
        ),
    }
}

/// `identity.ca_fingerprint` — `sha256:` over the DER of the trust anchor in
/// the profile's root bundle (§5.1): the root whose key signed the profile
/// certificate's chain.
///
/// The bundle is never read by position, and a one-certificate bundle is
/// checked like any other. An zagros operator bundle after the CA separation
/// carries the zagros CA that signs the leaf AND the shared root the router's own
/// server certificate still chains to (#1115), and the wrong anchor here is the
/// wrong trust anchor for every peering certificate the counterparty will
/// issue. The selection is [`super::anchor::issuing_root`].
pub fn ca_fingerprint(ca_roots_pem: &Path, leaf_pem: &Path) -> Derived<String> {
    const FIELD: &str = "identity.ca_fingerprint";
    let src = ca_roots_pem.display().to_string();
    let Ok(bytes) = std::fs::read(ca_roots_pem) else {
        return Derived::fail(FIELD, format!("no CA root bundle at {src}"), src);
    };
    let Ok(items) = pem::parse_many(&bytes) else {
        return Derived::fail(FIELD, "the CA root bundle is not readable PEM", src);
    };
    let certs: Vec<&pem::Pem> = items.iter().filter(|p| p.tag() == "CERTIFICATE").collect();
    if certs.is_empty() {
        return Derived::fail(FIELD, "the CA root bundle contains no certificate", src);
    }
    let leaf_src = leaf_pem.display().to_string();
    let Ok(presented) = std::fs::read(leaf_pem) else {
        return Derived::fail(
            FIELD,
            format!(
                "the profile certificate at {leaf_src}, which says which bundle certificate is \
                 the anchor, could not be read"
            ),
            src,
        );
    };
    let (anchor, subject) = match super::anchor::issuing_root(&certs, &presented, &leaf_src) {
        Ok(found) => found,
        Err(why) => return Derived::fail(FIELD, why, src),
    };
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(anchor.contents());
    Derived::ok(
        FIELD,
        format!("sha256:{:x}", h.finalize()),
        format!(
            "sha256 over the DER in {src} ({subject}, the root that issued the profile \
             certificate)"
        ),
    )
}

// ---------------------------------------------------------------------------
// Capabilities — measured, every one
// ---------------------------------------------------------------------------

/// `capabilities.transports` — what is actually installed and configured, not
/// what could be.
///
/// An empty result is a FAILURE, not `[]`: an empty set-array is not a
/// declaration (§3), and a package that offered one would refuse at
/// composition anyway with nothing to point at.
pub fn transports() -> Derived<Vec<String>> {
    transports_on(&Local)
}

/// [`transports`], read through `probe`.
pub fn transports_on(probe: &dyn Probe) -> Derived<Vec<String>> {
    const FIELD: &str = "capabilities.transports";
    let mut found = Vec::new();
    let mut notes = Vec::new();

    // WireGuard: an interface has to exist. The tool being installed says
    // nothing about whether this host can carry a link.
    match probe.run("wg", &["show", "interfaces"]) {
        Ok(out) if !out.trim().is_empty() => {
            found.push("wireguard".to_string());
            notes.push(format!("`wg show interfaces` → {}", out.trim()));
        }
        Ok(_) => notes.push(
            "`wg show interfaces` is empty — WireGuard is installed with no interface up".into(),
        ),
        Err(e) => notes.push(format!("`wg show interfaces` — {e}")),
    }

    // ipsec-fips is two facts, and both are required: an IPsec implementation,
    // AND the host actually running in FIPS mode. Reporting `ipsec-fips`
    // because strongSwan is installed would be the exact claim-for-a-fact
    // substitution this module exists to prevent.
    let ipsec =
        probe.run("swanctl", &["--version"]).is_ok() || probe.run("ipsec", &["--version"]).is_ok();
    // Absent (ENOENT — no such sysctl, e.g. not Linux or a non-FIPS kernel)
    // IS a readable "not FIPS". PermissionDenied is a DIFFERENT fact: the
    // file exists and this host may well be in FIPS mode, but this caller
    // cannot see it — collapsing that into `false` is the exact
    // claim-for-a-fact substitution `pqc_capable()` (below) was written to
    // avoid, and this check didn't copy that discipline.
    //
    // Only an absent file is a "no". Through a remote probe, a read can also
    // fail because the connection dropped or sudo would not run `cat`; those
    // say nothing about the host's FIPS mode either, so they are unknowns too.
    let fips = match probe.read_file("/proc/sys/crypto/fips_enabled") {
        Ok(s) => Ok(s.trim() == "1"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err("permission denied".to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    match (ipsec, fips) {
        (true, Ok(true)) => {
            found.push("ipsec-fips".to_string());
            notes.push("strongSwan present and /proc/sys/crypto/fips_enabled is 1".into());
        }
        (true, Ok(false)) => {
            notes.push("strongSwan present but /proc/sys/crypto/fips_enabled is not 1 — that is `ipsec`, not `ipsec-fips`".into());
        }
        (true, Err(why)) => {
            notes.push(format!(
                "strongSwan present but /proc/sys/crypto/fips_enabled could not be read \
                 ({why}) — FIPS status on this host is UNKNOWN, not false; \
                 `ipsec-fips` is withheld rather than guessed."
            ));
        }
        (false, _) => {
            notes.push("no strongSwan (`swanctl`/`ipsec`) on PATH".into());
        }
    }

    if found.is_empty() {
        Derived::fail(
            FIELD,
            format!(
                "no transport is installed and configured on this host{}. An empty set-array is \
                 not a declaration, so there is nothing to emit. What was read: {}",
                at(probe),
                notes.join("; ")
            ),
            format!("local system probe{}", at(probe)),
        )
    } else {
        Derived::ok(
            FIELD,
            super::json::normalise_set(&found),
            format!("{}{}", notes.join("; "), at(probe)),
        )
    }
}

/// Derive both fabric capabilities from one running core container. A pin
/// says what should be running; the container and its binary say what is.
pub fn fabric_capabilities() -> (Derived<bool>, Derived<String>) {
    let selected = std::env::var("FED_ROUTER_CONTAINER")
        .ok()
        .filter(|v| !v.is_empty());
    let target = match selected.as_deref() {
        Some(name) => RouterTarget::Named {
            name,
            from: "FED_ROUTER_CONTAINER",
        },
        None => RouterTarget::Discover,
    };
    // DOCKER_HOST explicitly selects a daemon, often remote. Never replace a
    // failed remote Docker read with an unrelated local Podman observation.
    let tools = router_tools(std::env::var_os("DOCKER_HOST").is_some());
    fabric_capabilities_on(&Local, target, tools, "FED_ROUTER_CONTAINER")
}

/// Which core router container to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterTarget<'a> {
    /// Find the one running core: the role label, the standard
    /// `*-zenoh-router` names, or the legacy `goat-zenoh`.
    Discover,
    /// A container name or ID, and where that name came from (an operator's
    /// selector, or the deployment's own variables). Never falls back to
    /// discovery: a named router that is absent or stopped is a refusal, not a
    /// reason to measure some other container.
    Named { name: &'a str, from: &'a str },
}

/// [`fabric_capabilities`], read through `probe`. `tools` are the container
/// runtimes to ask, in order; `knob` is how the operator names a router by
/// hand, quoted in the refusals (`FED_ROUTER_CONTAINER` for `goat`).
pub fn fabric_capabilities_on(
    probe: &dyn Probe,
    target: RouterTarget,
    tools: &[&'static str],
    knob: &str,
) -> (Derived<bool>, Derived<String>) {
    let mut run = |bin: &str, args: &[&str]| probe.run(bin, args);
    let router = match target {
        RouterTarget::Discover => select_router(None, tools, &mut run, knob),
        // An operator's own selector reads as it always has; a name the
        // deployment supplied says where it came from.
        RouterTarget::Named { name, from } if from == knob => {
            select_router(Some(name), tools, &mut run, knob)
        }
        RouterTarget::Named { name, from } => select_router(Some(name), tools, &mut run, knob)
            .map_err(|why| format!("{from} names the core router `{name}`: {why}")),
    };
    let (mut fips, mut version) = fabric_from_router(router, &mut run);
    let place = at(probe);
    fips.source.push_str(&place);
    version.source.push_str(&place);
    (fips, version)
}

fn router_tools(docker_host_set: bool) -> &'static [&'static str] {
    if docker_host_set {
        &["docker"]
    } else {
        &["docker", "podman"]
    }
}

fn fabric_from_router(
    router: Result<Router, String>,
    run: &mut impl FnMut(&str, &[&str]) -> Result<String, String>,
) -> (Derived<bool>, Derived<String>) {
    const FIPS: &str = "capabilities.fabric_fips_image";
    const VERSION: &str = "capabilities.fabric_version";
    let router = match router {
        Ok(router) => router,
        Err(why) => {
            return (
                Derived::fail(FIPS, &why, "running core router discovery"),
                Derived::fail(VERSION, why, "running core router discovery"),
            )
        }
    };
    let fips = Derived::ok(
        FIPS,
        router.image.contains("zenoh-pqc-fips"),
        format!("`{} inspect {}` → {}", router.tool, router.id, router.image),
    );
    let mut attempts = Vec::new();
    // Published images install /zenohd without a PATH entry. Both probes use
    // the same daemon and immutable container ID as the image observation.
    for executable in ["zenohd", "/zenohd"] {
        match run(router.tool, &["exec", &router.id, executable, "--version"]) {
            Ok(out) => {
                if let Some(v) = parse_version(&out) {
                    return (
                        fips,
                        Derived::ok(
                            VERSION,
                            v,
                            format!(
                                "`{} exec {} {executable} --version`",
                                router.tool, router.id
                            ),
                        ),
                    );
                }
                attempts.push(format!("{executable}: ran, but no version in output"));
            }
            Err(e) => attempts.push(format!("{executable}: {e}")),
        }
    }
    let version = Derived::fail(
        VERSION,
        format!(
            "could not read a version from the selected running core router {}. Attempts: {}",
            router.id,
            attempts.join(" | ")
        ),
        format!("{} container {}", router.tool, router.id),
    );
    (fips, version)
}

/// `capabilities.pqc_capable` — whether the intra-mesh PQC layer is RUNNING.
///
/// A host with the software installed and the unit stopped is not PQC-capable
/// in any sense a counterparty cares about, and `false` is not defaulted to
/// either: an unreadable probe is an unknown, and an unknown is a failure.
pub fn pqc_capable() -> Derived<bool> {
    pqc_capable_on(&Local)
}

/// [`pqc_capable`], read through `probe`.
pub fn pqc_capable_on(probe: &dyn Probe) -> Derived<bool> {
    const FIELD: &str = "capabilities.pqc_capable";
    // `systemctl is-active` EXITS NON-ZERO FOR EVERY STATE THAT IS NOT ACTIVE
    // and prints the state on stdout regardless — `inactive` with exit 3 is a
    // complete, readable answer, not a probe that failed. Reading only the exit
    // status threw that answer away and made the field underivable on any host
    // where Rosenpass is simply not installed, which is the default: it is
    // opt-in per goatnet (`rosenpass_enabled`). The result was that `init`
    // refused to produce a package on a host whose systemd had just answered
    // the question.
    match probe.run_reading_output("systemctl", &["is-active", "rosenpass"]) {
        Ok(out) => {
            let s = out.trim().to_string();
            Derived::ok(
                FIELD,
                s == "active",
                format!("`systemctl is-active rosenpass` → {s}{}", at(probe)),
            )
        }
        Err(_) => match probe.run("systemctl", &["list-unit-files", "rosenpass.service"]) {
            // The unit exists and is not active — that is a readable `false`.
            Ok(out) if out.contains("rosenpass.service") => Derived::ok(
                FIELD,
                false,
                format!(
                    "`systemctl list-unit-files rosenpass.service` — present, not active{}",
                    at(probe)
                ),
            ),
            _ => Derived::fail(
                FIELD,
                "no `rosenpass` unit and no systemd to ask. `false` is not defaulted here: an \
                 unreadable probe is an unknown, and emitting an unknown as a capability is the \
                 substitution this verb exists to prevent",
                format!("systemd{}", at(probe)),
            ),
        },
    }
}

/// `transport_offer.public_key` — the federation interface's own public key,
/// read out of the running interface.
pub fn transport_public_key(interface: &str) -> Derived<String> {
    transport_public_key_on(&Local, interface)
}

/// [`transport_public_key`], read through `probe`.
pub fn transport_public_key_on(probe: &dyn Probe, interface: &str) -> Derived<String> {
    const FIELD: &str = "transport_offer.public_key";
    if !interface_name(interface) {
        return Derived::fail(
            FIELD,
            format!("`{interface}` is not an interface name"),
            "the declaration's [transport] interface",
        );
    }
    match probe.run("wg", &["show", interface, "public-key"]) {
        Ok(out) if !out.trim().is_empty() => Derived::ok(
            FIELD,
            out.trim().to_string(),
            format!("`wg show {interface} public-key`{}", at(probe)),
        ),
        Ok(_) => Derived::fail(FIELD, format!("interface `{interface}` reported no public key"), "wg"),
        Err(e) => Derived::fail(
            FIELD,
            format!("`wg show {interface} public-key` — {e}. The declared federation interface has to exist before its key can be read"),
            "wg",
        ),
    }
}

/// The interface MTU, used only to CROSS-CHECK a declared `measured_mtu`.
///
/// An interface MTU is not a measured path MTU and is never substituted for
/// one — but a declared path MTU larger than the interface that carries it
/// cannot be right, and that is worth saying before the package is signed.
pub fn interface_mtu(interface: &str) -> Option<i64> {
    interface_mtu_on(&Local, interface)
}

/// [`interface_mtu`], read through `probe`.
pub fn interface_mtu_on(probe: &dyn Probe, interface: &str) -> Option<i64> {
    if !interface_name(interface) {
        return None;
    }
    probe
        .read_file(&format!("/sys/class/net/{interface}/mtu"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ---------------------------------------------------------------------------
// Everything `init` measures, in one place
// ---------------------------------------------------------------------------

/// The measured half of a package: what the edge host and the core host say
/// about themselves. Identity (`trust_domain`, `ca_fingerprint`) is not here:
/// it is read from the operator's own profile, wherever `init` runs.
#[derive(Debug, Clone)]
pub struct Measured {
    pub transports: Derived<Vec<String>>,
    pub fabric_fips_image: Derived<bool>,
    pub fabric_version: Derived<String>,
    pub pqc_capable: Derived<bool>,
    /// Present when a federation interface was named.
    pub public_key: Option<Derived<String>>,
    /// The interface MTU, for the `measured_mtu` cross-check only.
    pub interface_mtu: Option<i64>,
}

/// Measure the edge and the core. The edge carries the transport and the PQC
/// layer; the core runs the router. On the edge host itself, with the core's
/// runtime reachable from it, both are [`Local`] — the shape `goat federate
/// init` has always had.
pub fn measure(
    edge: &dyn Probe,
    core: &dyn Probe,
    router: RouterTarget,
    tools: &[&'static str],
    knob: &str,
    interface: Option<&str>,
) -> Measured {
    let (fabric_fips_image, fabric_version) = fabric_capabilities_on(core, router, tools, knob);
    Measured {
        transports: transports_on(edge),
        fabric_fips_image,
        fabric_version,
        pqc_capable: pqc_capable_on(edge),
        public_key: interface.map(|i| transport_public_key_on(edge, i)),
        interface_mtu: interface.and_then(|i| interface_mtu_on(edge, i)),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A Linux interface name: what `wg` and `/sys/class/net/` accept, and
/// nothing that walks out of `/sys/class/net/`.
fn interface_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 15
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

fn container_name(value: &str) -> bool {
    !value.is_empty()
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

fn inspect_router(
    tool: &'static str,
    name: &str,
    run: &mut impl FnMut(&str, &[&str]) -> Result<String, String>,
) -> Result<Router, String> {
    let format = if tool == "podman" {
        PODMAN_INSPECT_FORMAT
    } else {
        ROUTER_INSPECT_FORMAT
    };
    let out = run(
        tool,
        &["inspect", "--type", "container", "--format", format, name],
    )?;
    let parts: Vec<_> = out.trim().split('\t').collect();
    if parts.len() != 3 || !container_name(parts[0]) || parts[1] != "true" || parts[2].is_empty() {
        return Err(format!(
            "{tool} container {name}: no complete running-container observation"
        ));
    }
    Ok(Router {
        tool,
        id: parts[0].to_string(),
        image: parts[2].to_string(),
    })
}

fn select_router(
    selected: Option<&str>,
    tools: &[&'static str],
    run: &mut impl FnMut(&str, &[&str]) -> Result<String, String>,
    knob: &str,
) -> Result<Router, String> {
    if selected.is_some_and(|name| !container_name(name)) {
        return Err(format!(
            "{knob} must be a container name or ID, not an option or expression"
        ));
    }
    let mut found: Vec<Router> = Vec::new();
    let mut attempts = Vec::new();
    for &tool in tools {
        let candidates = match selected {
            Some(name) => vec![name.to_string()],
            None => match run(tool, &["ps", "--no-trunc", "--format", ROUTER_LIST_FORMAT]) {
                Ok(out) => {
                    let mut ids = Vec::new();
                    for line in out.lines().filter(|line| !line.is_empty()) {
                        let parts: Vec<_> = line.split('\t').collect();
                        if parts.len() != 3
                            || !container_name(parts[0])
                            || !container_name(parts[1])
                        {
                            return Err(format!(
                                "{tool}: could not parse the running container listing"
                            ));
                        }
                        if parts[2] == CORE_ROLE
                            || parts[1] == "goat-zenoh"
                            || parts[1].ends_with("-zenoh-router")
                        {
                            ids.push(parts[0].to_string());
                        }
                    }
                    ids
                }
                Err(e) => {
                    attempts.push(format!("{tool}: {e}"));
                    continue;
                }
            },
        };
        for name in candidates {
            match inspect_router(tool, &name, run) {
                Ok(router) => {
                    // Docker may be an alias of Podman: one immutable ID is
                    // one observation, not two different router candidates.
                    if !found.iter().any(|old| old.id == router.id) {
                        found.push(router);
                    }
                }
                Err(e) => attempts.push(format!("{tool}: {e}")),
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("no running core router found. Set {knob} to its name or ID; \
            discovery accepts org.goat.component=zenoh-core, standard *-zenoh-router names and legacy goat-zenoh. \
            Attempts: {}", attempts.join(" | "))),
        _ => Err(format!("multiple running core routers found ({}); set {knob} to select one",
            found.iter().map(|r| format!("{}:{}", r.tool, r.id)).collect::<Vec<_>>().join(", "))),
    }
}

/// `zenohd 1.9.0 …` → `1.9.0`. First dotted-numeric token, so a build suffix
/// or a git hash on the line does not become part of the version.
fn parse_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|t| {
            let core = t.trim_start_matches('v');
            core.split('.').count() >= 2 && core.starts_with(|c: char| c.is_ascii_digit())
        })
        .map(|t| t.trim_start_matches('v').to_string())
}

/// Run a read-only probe. Anything that is not a clean exit is an error rather
/// than an empty answer — "the command failed" and "the answer is nothing" are
/// different facts and only one of them is a derivation.
/// Like [`run`], but for the commands whose ANSWER is on stdout even when they
/// exit non-zero — `systemctl is-active` is the case that matters here. Kept
/// separate and used deliberately: for every other probe a non-zero exit means
/// the probe could not be made, and treating that as data is exactly the
/// claim-for-a-fact substitution this module exists to prevent.
///
/// Still an error when the binary cannot be executed at all, or when it exits
/// non-zero with nothing on stdout — there is no answer to read in either case.
fn run_reading_output(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("{bin} exited {} with no output", out.status)
        } else {
            err
        });
    }
    Ok(stdout)
}

fn run(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("{bin} exited {}", out.status)
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::anchor::test_pki::{leaf_signed_by, root, Ca};

    /// Writes the bundle and, when given, the profile certificate; returns the
    /// derivation. The selection itself is tested in `anchor`.
    fn derive_with(roots: &[&Ca], leaf_pem: Option<&str>) -> Derived<String> {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("ca-roots.pem");
        let leaf = dir.path().join("cert.pem");
        let joined: String = roots.iter().map(|c| c.cert.pem()).collect();
        std::fs::write(&bundle, joined).unwrap();
        if let Some(p) = leaf_pem {
            std::fs::write(&leaf, p).unwrap();
        }
        ca_fingerprint(&bundle, &leaf)
    }

    #[test]
    fn the_fingerprint_is_over_the_signing_root_and_the_source_names_it() {
        use sha2::{Digest, Sha256};
        let a = root("root-a");
        let b = root("root-b");
        let d = derive_with(&[&b, &a], Some(&leaf_signed_by(&a)));
        let want = format!("sha256:{:x}", Sha256::digest(a.cert.der()));
        assert_eq!(d.outcome.as_deref(), Ok(want.as_str()), "{d:?}");
        assert!(d.source.contains("root-a"), "{}", d.source);
    }

    #[test]
    fn no_readable_profile_certificate_is_refused_and_says_so() {
        // One root or two: the anchor is read from the leaf either way.
        let a = root("root-a");
        let b = root("root-b");
        for roots in [vec![&a], vec![&a, &b]] {
            let why = derive_with(&roots, None).outcome.unwrap_err();
            assert!(
                why.contains("profile certificate") && why.contains("could not be read"),
                "{why}"
            );
        }
    }

    fn runtime(rows: &str) -> impl FnMut(&str, &[&str]) -> Result<String, String> + '_ {
        move |tool, args| {
            if tool != "docker" {
                return Err("runtime unavailable".into());
            }
            match args[0] {
                "ps" => Ok(rows.into()),
                "inspect" => Ok(format!(
                    "{}\ttrue\tregistry.example.org/zenoh-pqc-fips@sha256:fixture",
                    args.last().unwrap()
                )),
                "exec" if args[2] == "/zenohd" => Ok("zenohd 1.9.0".into()),
                _ => Err("executable not found".into()),
            }
        }
    }

    #[test]
    fn discovers_the_standard_role_name_and_reads_both_values_from_its_id() {
        let mut probe = runtime("coreid\tzagros-zenoh-router\t\nedgeid\tgoat-zenoh-edge-xab\t\n");
        let router = select_router(
            None,
            &["docker", "podman"],
            &mut probe,
            "FED_ROUTER_CONTAINER",
        )
        .unwrap();
        assert_eq!(router.id, "coreid");
        let (fips, version) = fabric_from_router(Ok(router), &mut probe);
        assert_eq!(fips.outcome, Ok(true));
        assert_eq!(version.outcome.as_deref(), Ok("1.9.0"));
        assert!(fips.source.contains("docker inspect coreid"));
        assert!(version
            .source
            .contains("docker exec coreid /zenohd --version"));
    }

    #[test]
    fn role_label_supports_a_custom_stack_without_matching_unrelated_images() {
        let mut probe =
            runtime("customid\tcustom-router\tzenoh-core\nunrelated\tother-service\t\n");
        assert_eq!(
            select_router(None, &["docker"], &mut probe, "FED_ROUTER_CONTAINER")
                .unwrap()
                .id,
            "customid"
        );
    }

    #[test]
    fn existing_legacy_name_is_compatible() {
        let mut probe = runtime("legacyid\tgoat-zenoh\t\n");
        assert_eq!(
            select_router(None, &["docker"], &mut probe, "FED_ROUTER_CONTAINER")
                .unwrap()
                .id,
            "legacyid"
        );
    }

    #[test]
    fn multiple_cores_are_refused_instead_of_picking_the_first() {
        let mut probe =
            runtime("first\tzagros-zenoh-router\t\nsecond\tcustom-router\tzenoh-core\n");
        let why = select_router(None, &["docker"], &mut probe, "FED_ROUTER_CONTAINER").unwrap_err();
        assert!(why.contains("multiple") && why.contains("FED_ROUTER_CONTAINER"));
    }

    #[test]
    fn explicit_selector_never_falls_back_to_discovery() {
        let mut calls = Vec::new();
        let mut probe = |tool: &str, args: &[&str]| {
            calls.push(format!("{tool} {}", args.join(" ")));
            Err("no such container".into())
        };
        assert!(select_router(
            Some("missing"),
            &["docker"],
            &mut probe,
            "FED_ROUTER_CONTAINER"
        )
        .is_err());
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("inspect") && calls[0].ends_with(" missing"));
    }

    #[test]
    fn explicit_selector_works_without_a_role_label() {
        let mut probe = runtime("");
        assert_eq!(
            select_router(
                Some("custom-router"),
                &["docker"],
                &mut probe,
                "FED_ROUTER_CONTAINER"
            )
            .unwrap()
            .id,
            "custom-router"
        );
    }

    #[test]
    fn selector_options_and_expressions_are_refused_before_any_command() {
        for name in ["--help", "x y", "x;echo", "$(id)", "", "/router"] {
            let mut probe = |_: &str, _: &[&str]| panic!("invalid selector executed");
            assert!(
                select_router(Some(name), &["docker"], &mut probe, "FED_ROUTER_CONTAINER").is_err()
            );
        }
    }

    #[test]
    fn missing_and_stopped_routers_are_not_measured() {
        let mut empty = runtime("");
        let why = select_router(None, &["docker"], &mut empty, "FED_ROUTER_CONTAINER").unwrap_err();
        assert!(why.contains("no running core router") && why.contains("FED_ROUTER_CONTAINER"));
        let mut stopped = |_: &str, _: &[&str]| Ok("id\tfalse\timage".into());
        assert!(select_router(
            Some("stopped"),
            &["docker"],
            &mut stopped,
            "FED_ROUTER_CONTAINER"
        )
        .is_err());
    }

    #[test]
    fn failed_discovery_fails_both_fields_without_running_version_probes() {
        let mut probe = |_: &str, _: &[&str]| panic!("unselected router executed");
        let (fips, version) = fabric_from_router(Err("ambiguous".into()), &mut probe);
        assert_eq!(fips.outcome.unwrap_err(), "ambiguous");
        assert_eq!(version.outcome.unwrap_err(), "ambiguous");
    }

    #[test]
    fn version_failure_never_switches_daemon_or_container() {
        let router = Router {
            tool: "docker",
            id: "coreid".into(),
            image: "eclipse/zenoh:1.9.0".into(),
        };
        let mut probe = |tool: &str, args: &[&str]| {
            assert_eq!(tool, "docker");
            assert_eq!(args[1], "coreid");
            Err("container stopped during observation".into())
        };
        let (fips, version) = fabric_from_router(Ok(router), &mut probe);
        assert_eq!(fips.outcome, Ok(false));
        assert!(version.outcome.unwrap_err().contains("container stopped"));
    }

    #[test]
    fn podman_is_supported_and_runtime_aliases_do_not_create_ambiguity() {
        let mut probe = |_: &str, args: &[&str]| match args[0] {
            "ps" => Ok("sameid\tgoat-zenoh\t\n".into()),
            "inspect" => Ok("sameid\ttrue\timage".into()),
            _ => unreachable!(),
        };
        let router = select_router(
            None,
            &["docker", "podman"],
            &mut probe,
            "FED_ROUTER_CONTAINER",
        )
        .unwrap();
        assert_eq!(router.id, "sameid");
        assert_eq!(
            select_router(None, &["podman"], &mut probe, "FED_ROUTER_CONTAINER")
                .unwrap()
                .tool,
            "podman"
        );
    }

    #[test]
    fn an_explicit_docker_host_never_probes_local_podman() {
        let mut probe = |tool: &str, _: &[&str]| {
            assert_eq!(tool, "docker", "an unrelated local runtime was probed");
            Err("remote daemon unreachable".into())
        };
        let why = select_router(None, router_tools(true), &mut probe, "FED_ROUTER_CONTAINER")
            .unwrap_err();
        assert!(why.contains("remote daemon unreachable"));
    }

    #[test]
    fn malformed_runtime_output_and_permission_errors_are_not_capabilities() {
        let mut malformed = |_: &str, _: &[&str]| Ok("not a container listing".into());
        assert!(
            select_router(None, &["docker"], &mut malformed, "FED_ROUTER_CONTAINER")
                .unwrap_err()
                .contains("could not parse")
        );
        let mut denied = |_: &str, _: &[&str]| Err("permission denied".into());
        assert!(
            select_router(None, &["docker"], &mut denied, "FED_ROUTER_CONTAINER")
                .unwrap_err()
                .contains("permission denied")
        );
    }

    /// A host made of recorded outputs: `(command line, answer)`. Every command
    /// it is asked is recorded, and anything not recorded fails the way a
    /// missing binary does.
    struct Recorded {
        place: &'static str,
        answers: Vec<(&'static str, Result<&'static str, &'static str>)>,
        files: Vec<(&'static str, std::io::Result<&'static str>)>,
        asked: std::cell::RefCell<Vec<String>>,
    }

    impl Recorded {
        fn new(place: &'static str) -> Self {
            Recorded {
                place,
                answers: Vec::new(),
                files: Vec::new(),
                asked: Default::default(),
            }
        }
        fn answer(mut self, cmd: &'static str, out: Result<&'static str, &'static str>) -> Self {
            self.answers.push((cmd, out));
            self
        }
        fn file(mut self, path: &'static str, out: std::io::Result<&'static str>) -> Self {
            self.files.push((path, out));
            self
        }
        fn lookup(&self, bin: &str, args: &[&str]) -> Result<String, String> {
            assert!(
                is_read_only_probe(bin, args),
                "derive issued a command outside the read-only list: {bin} {args:?}"
            );
            let line = std::iter::once(bin)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.asked.borrow_mut().push(line.clone());
            match self.answers.iter().find(|(c, _)| *c == line) {
                Some((_, Ok(out))) => Ok(out.to_string()),
                Some((_, Err(e))) => Err(e.to_string()),
                None => Err(format!("{bin}: No such file or directory")),
            }
        }
    }

    impl Probe for Recorded {
        fn run(&self, bin: &str, args: &[&str]) -> Result<String, String> {
            self.lookup(bin, args)
        }
        fn run_reading_output(&self, bin: &str, args: &[&str]) -> Result<String, String> {
            self.lookup(bin, args)
        }
        fn read_file(&self, path: &str) -> std::io::Result<String> {
            assert!(
                is_read_only_probe("cat", &["--", path]),
                "read outside the list: {path}"
            );
            self.asked.borrow_mut().push(format!("read {path}"));
            match self.files.iter().find(|(p, _)| *p == path) {
                Some((_, Ok(s))) => Ok(s.to_string()),
                Some((_, Err(e))) => Err(std::io::Error::new(e.kind(), e.to_string())),
                None => Err(std::io::ErrorKind::NotFound.into()),
            }
        }
        fn place(&self) -> String {
            self.place.to_string()
        }
    }

    fn edge_host() -> Recorded {
        Recorded::new("on edge-a")
            .answer("wg show interfaces", Ok("wg-fed0 wt0\n"))
            .answer("wg show wg-fed0 public-key", Ok("PUBKEYfixture=\n"))
            .answer("systemctl is-active rosenpass", Ok("inactive\n"))
            .file("/proc/sys/crypto/fips_enabled", Ok("0\n"))
            .file("/sys/class/net/wg-fed0/mtu", Ok("1420\n"))
    }

    fn core_host() -> Recorded {
        Recorded::new("on core-a")
            .answer(
                "docker ps --no-trunc --format {{.ID}}\t{{.Names}}\t{{.Label \"org.goat.component\"}}",
                Ok("coreid\tzagros-zenoh-router\tzenoh-core\n"),
            )
            .answer(
                "docker inspect --type container --format {{.Id}}\t{{.State.Running}}\t{{.Config.Image}} coreid",
                Ok("coreid\ttrue\tregistry.example.org/zenoh-pqc-fips@sha256:fixture\n"),
            )
            .answer("docker exec coreid /zenohd --version", Ok("zenohd v1.9.0\n"))
    }

    #[test]
    fn measure_reads_the_edge_and_the_core_each_where_it_runs() {
        let (edge, core) = (edge_host(), core_host());
        let m = measure(
            &edge,
            &core,
            RouterTarget::Discover,
            &["docker"],
            "--router-container",
            Some("wg-fed0"),
        );
        assert_eq!(m.transports.outcome, Ok(vec!["wireguard".to_string()]));
        assert!(
            m.transports.source.ends_with(" on edge-a"),
            "{}",
            m.transports.source
        );
        assert_eq!(m.fabric_fips_image.outcome, Ok(true));
        assert_eq!(m.fabric_version.outcome.as_deref(), Ok("1.9.0"));
        assert!(
            m.fabric_version.source.ends_with(" on core-a"),
            "{}",
            m.fabric_version.source
        );
        assert_eq!(m.pqc_capable.outcome, Ok(false));
        assert_eq!(
            m.public_key.unwrap().outcome.as_deref(),
            Ok("PUBKEYfixture=")
        );
        assert_eq!(m.interface_mtu, Some(1420));
        // No transport or PQC read goes to the core, no router read to the edge.
        assert!(
            core.asked.borrow().iter().all(|c| c.starts_with("docker ")),
            "{:?}",
            core.asked.borrow()
        );
        assert!(
            edge.asked
                .borrow()
                .iter()
                .all(|c| !c.starts_with("docker ")),
            "{:?}",
            edge.asked.borrow()
        );
    }

    #[test]
    fn an_unreadable_fips_flag_withholds_ipsec_fips_on_a_remote_host_too() {
        let edge = Recorded::new("on edge-a")
            .answer("swanctl --version", Ok("swanctl 6.0"))
            .file(
                "/proc/sys/crypto/fips_enabled",
                Err(std::io::ErrorKind::PermissionDenied.into()),
            );
        let d = transports_on(&edge);
        let why = d.outcome.unwrap_err();
        assert!(
            why.contains("UNKNOWN, not false") && why.contains("on edge-a"),
            "{why}"
        );
    }

    #[test]
    fn a_declared_router_is_measured_by_name_and_never_rediscovered() {
        let core = core_host();
        let (fips, version) = fabric_capabilities_on(
            &core,
            RouterTarget::Named {
                name: "xaa-zenoh-router",
                from: "zenoh_stack (inventory host core-a)",
            },
            &["docker"],
            "--router-container",
        );
        let why = version.outcome.unwrap_err();
        assert!(
            why.starts_with(
                "zenoh_stack (inventory host core-a) names the core router `xaa-zenoh-router`"
            ),
            "{why}"
        );
        assert!(fips.outcome.is_err());
        assert!(
            core.asked.borrow().iter().all(|c| !c.contains(" ps ")),
            "discovery ran: {:?}",
            core.asked.borrow()
        );
    }

    #[test]
    fn a_remote_read_that_failed_for_another_reason_is_unknown_not_false() {
        let edge = Recorded::new("on edge-a")
            .answer("swanctl --version", Ok("swanctl 6.0"))
            .file(
                "/proc/sys/crypto/fips_enabled",
                Err(std::io::Error::other(
                    "ssh to edge-a failed: connection reset",
                )),
            );
        let why = transports_on(&edge).outcome.unwrap_err();
        assert!(
            why.contains("UNKNOWN, not false") && why.contains("connection reset"),
            "{why}"
        );
        // An absent file is still a readable "no".
        let absent = Recorded::new("on edge-a").answer("swanctl --version", Ok("swanctl 6.0"));
        let why = transports_on(&absent).outcome.unwrap_err();
        assert!(why.contains("that is `ipsec`, not `ipsec-fips`"), "{why}");
    }

    #[test]
    fn an_operator_selector_reads_as_before_and_a_deployment_name_says_where_it_came_from() {
        let core = core_host();
        let named = |from| {
            fabric_capabilities_on(
                &core,
                RouterTarget::Named { name: "gone", from },
                &["docker"],
                "FED_ROUTER_CONTAINER",
            )
            .1
            .outcome
            .unwrap_err()
        };
        assert!(!named("FED_ROUTER_CONTAINER").contains("names the core router"));
        assert!(named("site_id").starts_with("site_id names the core router `gone`"));
    }

    #[test]
    fn an_interface_name_that_leaves_sys_class_net_is_not_read() {
        let edge = edge_host();
        assert_eq!(interface_mtu_on(&edge, "../../../etc/shadow"), None);
        assert!(transport_public_key_on(&edge, "-x").outcome.is_err());
        assert!(edge.asked.borrow().is_empty(), "{:?}", edge.asked.borrow());
    }

    #[test]
    fn the_read_only_list_refuses_writes_and_shells() {
        for (bin, args) in [
            ("wg", vec!["set", "wg-fed0", "private-key", "/k"]),
            ("docker", vec!["exec", "coreid", "sh", "-c", "id"]),
            ("docker", vec!["rm", "-f", "coreid"]),
            (
                "docker",
                vec!["exec", "--privileged", "zenohd", "--version"],
            ),
            ("systemctl", vec!["restart", "rosenpass"]),
            ("cat", vec!["--", "/etc/shadow"]),
            ("cat", vec!["--", "/sys/class/net/../../../etc/shadow"]),
            ("wg", vec!["show", "all", "private-key"]),
            ("wg", vec!["show", "wg-fed0", "private-key"]),
            ("wg", vec!["showconf", "wg-fed0"]),
            ("docker", vec!["inspect", "coreid"]),
            (
                "docker",
                vec![
                    "inspect",
                    "--type",
                    "container",
                    "--format",
                    "{{json .Config.Env}}",
                    "coreid",
                ],
            ),
            ("docker", vec!["ps", "--format", "{{.Command}}"]),
            ("sh", vec!["-c", "true"]),
        ] {
            assert!(!is_read_only_probe(bin, &args), "{bin} {args:?}");
        }
        assert!(is_read_only_probe(
            "docker",
            &["exec", "coreid", "/zenohd", "--version"]
        ));
    }

    #[test]
    fn version_token_survives_a_noisy_line() {
        assert_eq!(
            parse_version("zenohd v1.9.0 built with rustc"),
            Some("1.9.0".into())
        );
        assert_eq!(parse_version("zenohd 1.9.0"), Some("1.9.0".into()));
        assert_eq!(parse_version("no version here"), None);
    }

    /// The selection contract has two implementations: this one (phase 3) and
    /// ops/federation/standup/preflight_live.py (phase 1). Both read the same
    /// cases, so a change to one that the other does not make fails here or there.
    #[test]
    fn the_shared_router_discovery_cases_hold() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ops/federation/standup/fixtures/router-discovery.tsv");
        let text = std::fs::read_to_string(&path).unwrap();
        let unescape = |s: &str| {
            s.replace("\\t", "\t")
                .replace("\\n", "\n")
                .replace("\\\\", "\\")
        };
        type Case = (
            String,
            String,
            Vec<&'static str>,
            String,
            String,
            Vec<(String, String, bool, String)>,
        );
        let mut cases: Vec<Case> = Vec::new();
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let f: Vec<&str> = line.split('\t').collect();
            match f[0] {
                "case" => {
                    let tools = f[3]
                        .split(',')
                        .map(|t| if t == "docker" { "docker" } else { "podman" })
                        .collect();
                    cases.push((
                        f[1].into(),
                        f[2].into(),
                        tools,
                        f[4].into(),
                        f.get(5).copied().unwrap_or("").into(),
                        Vec::new(),
                    ));
                }
                "reply" => cases.last_mut().unwrap().5.push((
                    f[1].into(),
                    f[2].into(),
                    f[3] == "ok",
                    unescape(f.get(4).copied().unwrap_or("")),
                )),
                other => panic!("unknown row {other}"),
            }
        }
        assert!(cases.len() >= 10, "{} cases", cases.len());
        for (name, selected, tools, expect, id, replies) in &cases {
            let mut probe = |tool: &str, args: &[&str]| -> Result<String, String> {
                let req = if args[0] == "ps" {
                    "ps".to_string()
                } else {
                    format!("inspect:{}", args.last().unwrap())
                };
                match replies.iter().find(|(t, r, _, _)| t == tool && *r == req) {
                    Some((_, _, true, out)) => Ok(out.clone()),
                    Some((_, _, false, err)) => Err(err.clone()),
                    None => Err(format!("{tool}: no reply for {req}")),
                }
            };
            let sel = if selected == "-" {
                None
            } else {
                Some(selected.as_str())
            };
            let got = select_router(sel, tools, &mut probe, "FED_ROUTER_CONTAINER");
            let verdict = match &got {
                Ok(_) => "ok",
                Err(e) if e.starts_with("multiple") => "many",
                Err(e) if e.starts_with("no running core router") => "none",
                Err(_) => "invalid",
            };
            assert_eq!(verdict, expect, "{name}: {got:?}");
            if let Ok(r) = got {
                assert_eq!(&r.id, id, "{name}");
            }
        }
    }
}

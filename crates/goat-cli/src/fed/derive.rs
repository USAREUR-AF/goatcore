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
    const FIELD: &str = "capabilities.transports";
    let mut found = Vec::new();
    let mut notes = Vec::new();

    // WireGuard: an interface has to exist. The tool being installed says
    // nothing about whether this host can carry a link.
    match run("wg", &["show", "interfaces"]) {
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
    let ipsec = run("swanctl", &["--version"]).is_ok() || run("ipsec", &["--version"]).is_ok();
    // Absent (ENOENT — no such sysctl, e.g. not Linux or a non-FIPS kernel)
    // IS a readable "not FIPS". PermissionDenied is a DIFFERENT fact: the
    // file exists and this host may well be in FIPS mode, but this caller
    // cannot see it — collapsing that into `false` is the exact
    // claim-for-a-fact substitution `pqc_capable()` (below) was written to
    // avoid, and this check didn't copy that discipline.
    let fips = match std::fs::read_to_string("/proc/sys/crypto/fips_enabled") {
        Ok(s) => Some(s.trim() == "1"),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(_) => Some(false),
    };
    match (ipsec, fips) {
        (true, Some(true)) => {
            found.push("ipsec-fips".to_string());
            notes.push("strongSwan present and /proc/sys/crypto/fips_enabled is 1".into());
        }
        (true, Some(false)) => {
            notes.push("strongSwan present but /proc/sys/crypto/fips_enabled is not 1 — that is `ipsec`, not `ipsec-fips`".into());
        }
        (true, None) => {
            notes.push(
                "strongSwan present but /proc/sys/crypto/fips_enabled could not be read \
                 (permission denied) — FIPS status on this host is UNKNOWN, not false; \
                 `ipsec-fips` is withheld rather than guessed."
                    .into(),
            );
        }
        (false, _) => {
            notes.push("no strongSwan (`swanctl`/`ipsec`) on PATH".into());
        }
    }

    if found.is_empty() {
        Derived::fail(
            FIELD,
            format!(
                "no transport is installed and configured on this host. An empty set-array is not \
                 a declaration, so there is nothing to emit. What was read: {}",
                notes.join("; ")
            ),
            "local system probe",
        )
    } else {
        Derived::ok(FIELD, super::json::normalise_set(&found), notes.join("; "))
    }
}

/// Derive both fabric capabilities from one running core container. A pin
/// says what should be running; the container and its binary say what is.
pub fn fabric_capabilities() -> (Derived<bool>, Derived<String>) {
    let selected = std::env::var("FED_ROUTER_CONTAINER")
        .ok()
        .filter(|v| !v.is_empty());
    // DOCKER_HOST explicitly selects a daemon, often remote. Never replace a
    // failed remote Docker read with an unrelated local Podman observation.
    let tools = router_tools(std::env::var_os("DOCKER_HOST").is_some());
    let router = select_router(selected.as_deref(), tools, &mut run);
    fabric_from_router(router, &mut run)
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
    const FIELD: &str = "capabilities.pqc_capable";
    // `systemctl is-active` EXITS NON-ZERO FOR EVERY STATE THAT IS NOT ACTIVE
    // and prints the state on stdout regardless — `inactive` with exit 3 is a
    // complete, readable answer, not a probe that failed. Reading only the exit
    // status threw that answer away and made the field underivable on any host
    // where Rosenpass is simply not installed, which is the default: it is
    // opt-in per goatnet (`rosenpass_enabled`). The result was that `init`
    // refused to produce a package on a host whose systemd had just answered
    // the question.
    match run_reading_output("systemctl", &["is-active", "rosenpass"]) {
        Ok(out) => {
            let s = out.trim().to_string();
            Derived::ok(
                FIELD,
                s == "active",
                format!("`systemctl is-active rosenpass` → {s}"),
            )
        }
        Err(_) => match run("systemctl", &["list-unit-files", "rosenpass.service"]) {
            // The unit exists and is not active — that is a readable `false`.
            Ok(out) if out.contains("rosenpass.service") => Derived::ok(
                FIELD,
                false,
                "`systemctl list-unit-files rosenpass.service` — present, not active",
            ),
            _ => Derived::fail(
                FIELD,
                "no `rosenpass` unit and no systemd to ask. `false` is not defaulted here: an \
                 unreadable probe is an unknown, and emitting an unknown as a capability is the \
                 substitution this verb exists to prevent",
                "systemd",
            ),
        },
    }
}

/// `transport_offer.public_key` — the federation interface's own public key,
/// read out of the running interface.
pub fn transport_public_key(interface: &str) -> Derived<String> {
    const FIELD: &str = "transport_offer.public_key";
    match run("wg", &["show", interface, "public-key"]) {
        Ok(out) if !out.trim().is_empty() => {
            Derived::ok(FIELD, out.trim().to_string(), format!("`wg show {interface} public-key`"))
        }
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
    std::fs::read_to_string(format!("/sys/class/net/{interface}/mtu"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    // Podman's template struct spells the field ID; Docker spells it Id.
    let format = if tool == "podman" {
        "{{.ID}}\t{{.State.Running}}\t{{.Config.Image}}"
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
) -> Result<Router, String> {
    if selected.is_some_and(|name| !container_name(name)) {
        return Err(
            "FED_ROUTER_CONTAINER must be a container name or ID, not an option or expression"
                .into(),
        );
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
        0 => Err(format!("no running core router found. Set FED_ROUTER_CONTAINER to its name or ID; \
            discovery accepts org.goat.component=zenoh-core, standard *-zenoh-router names and legacy goat-zenoh. \
            Attempts: {}", attempts.join(" | "))),
        _ => Err(format!("multiple running core routers found ({}); set FED_ROUTER_CONTAINER to select one",
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

    use crate::fed::anchor::test_pki::{leaf_signed_by, root, Ca};

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
        let router = select_router(None, &["docker", "podman"], &mut probe).unwrap();
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
            select_router(None, &["docker"], &mut probe).unwrap().id,
            "customid"
        );
    }

    #[test]
    fn existing_legacy_name_is_compatible() {
        let mut probe = runtime("legacyid\tgoat-zenoh\t\n");
        assert_eq!(
            select_router(None, &["docker"], &mut probe).unwrap().id,
            "legacyid"
        );
    }

    #[test]
    fn multiple_cores_are_refused_instead_of_picking_the_first() {
        let mut probe =
            runtime("first\tzagros-zenoh-router\t\nsecond\tcustom-router\tzenoh-core\n");
        let why = select_router(None, &["docker"], &mut probe).unwrap_err();
        assert!(why.contains("multiple") && why.contains("FED_ROUTER_CONTAINER"));
    }

    #[test]
    fn explicit_selector_never_falls_back_to_discovery() {
        let mut calls = Vec::new();
        let mut probe = |tool: &str, args: &[&str]| {
            calls.push(format!("{tool} {}", args.join(" ")));
            Err("no such container".into())
        };
        assert!(select_router(Some("missing"), &["docker"], &mut probe).is_err());
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("inspect") && calls[0].ends_with(" missing"));
    }

    #[test]
    fn explicit_selector_works_without_a_role_label() {
        let mut probe = runtime("");
        assert_eq!(
            select_router(Some("custom-router"), &["docker"], &mut probe)
                .unwrap()
                .id,
            "custom-router"
        );
    }

    #[test]
    fn selector_options_and_expressions_are_refused_before_any_command() {
        for name in ["--help", "x y", "x;echo", "$(id)", "", "/router"] {
            let mut probe = |_: &str, _: &[&str]| panic!("invalid selector executed");
            assert!(select_router(Some(name), &["docker"], &mut probe).is_err());
        }
    }

    #[test]
    fn missing_and_stopped_routers_are_not_measured() {
        let mut empty = runtime("");
        let why = select_router(None, &["docker"], &mut empty).unwrap_err();
        assert!(why.contains("no running core router") && why.contains("FED_ROUTER_CONTAINER"));
        let mut stopped = |_: &str, _: &[&str]| Ok("id\tfalse\timage".into());
        assert!(select_router(Some("stopped"), &["docker"], &mut stopped).is_err());
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
        let router = select_router(None, &["docker", "podman"], &mut probe).unwrap();
        assert_eq!(router.id, "sameid");
        assert_eq!(
            select_router(None, &["podman"], &mut probe).unwrap().tool,
            "podman"
        );
    }

    #[test]
    fn an_explicit_docker_host_never_probes_local_podman() {
        let mut probe = |tool: &str, _: &[&str]| {
            assert_eq!(tool, "docker", "an unrelated local runtime was probed");
            Err("remote daemon unreachable".into())
        };
        let why = select_router(None, router_tools(true), &mut probe).unwrap_err();
        assert!(why.contains("remote daemon unreachable"));
    }

    #[test]
    fn malformed_runtime_output_and_permission_errors_are_not_capabilities() {
        let mut malformed = |_: &str, _: &[&str]| Ok("not a container listing".into());
        assert!(select_router(None, &["docker"], &mut malformed)
            .unwrap_err()
            .contains("could not parse"));
        let mut denied = |_: &str, _: &[&str]| Err("permission denied".into());
        assert!(select_router(None, &["docker"], &mut denied)
            .unwrap_err()
            .contains("permission denied"));
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
}

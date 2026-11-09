// The package model, and the structural rules of spec §3 and §4.
//
// These refusals are properties of ONE package, so they are not among §6's
// seven composition refusals and are reported with their own `M_*` codes. The
// distinction matters and the conformance vectors test it directly: a
// malformed input reported as a composition refusal is the same verdict from
// the wrong place, which `ops/federation/vectors/malformed/README.md` calls
// out as a passing test hiding a broken implementation.
//
// Nothing here verifies a signature. The verifying key is bound to a
// `sovereign_id` over the authenticated channel of §5.1 and is never taken
// from the package, so a lone package cannot be authenticated by looking at
// it — `lint` says so rather than implying a check it did not run.

use super::json::{canonical, parse_document, Finding, JVal};

/// The one package version this build implements. Note that a package
/// declaring something else is NOT malformed: the version test is a literal
/// pin applied at composition step 1, deliberately, so that a mismatched pair
/// and a both-on-v2 pair produce the same named refusal (§6 step 1).
pub const PACKAGE_SPEC: &str = "federation-package/v1";
pub const AGREEMENT_SPEC: &str = "federation-agreement/v1";

/// The transport preference order. Part of the specification, not a local
/// setting — a local preference would break determinism (§7).
///
/// WireGuard is first, decided. It is what we run, what our
/// waiver covers, and what we can cut unilaterally; `transport_offer.public_key`
/// is WireGuard-shaped and has no defined IPsec derivation, so ranking
/// IPsec first preferred a transport this format cannot describe an offer for.
/// A party that needs IPsec gets it by declaring `ipsec-fips` and not
/// `wireguard`, which makes the intersection decide and never reaches this
/// order — so the exception costs a declaration, not a code change.
pub const TRANSPORT_PREFERENCE: [&str; 2] = ["wireguard", "ipsec-fips"];

#[derive(Debug, Clone)]
pub struct Identity {
    pub trust_domain: String,
    pub ca_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub transports: Vec<String>,
    pub fabric_fips_image: bool,
    pub pqc_capable: bool,
    pub fabric_version: String,
}

#[derive(Debug, Clone)]
pub struct TransportOffer {
    pub endpoint: String,
    pub listener: bool,
    pub public_key: String,
    pub proposed_link_subnet: String,
    pub measured_mtu: i64,
}

/// `data_offer` carries two separable halves, and each is complete once
/// present, independently of the other (§3). One-way federation is a real
/// case: a partner who will only receive has nothing to release, and forcing
/// it to invent a plausible-looking key expression would put a fiction in the
/// package that is later read as an intention.
#[derive(Debug, Clone)]
pub struct DataOffer {
    /// The send half. `None` when absent; never `Some(empty)` — an empty array
    /// is not a declaration.
    pub release: Option<Vec<String>>,
    /// The receive half: all three or none.
    pub accept: Option<Accept>,
}

#[derive(Debug, Clone, Copy)]
pub struct Accept {
    pub hz: i64,
    pub bytes_per_message: i64,
    pub distinct_keys: i64,
}

#[derive(Debug, Clone)]
pub struct Package {
    /// The document exactly as parsed. The signing input is derived from this,
    /// so nothing downstream may substitute a re-built object for it.
    pub raw: JVal,
    pub spec: String,
    pub sovereign_id: String,
    pub issued: String,
    pub expires: String,
    pub identity: Identity,
    pub capabilities: Capabilities,
    pub transport_offer: Option<TransportOffer>,
    pub data_offer: Option<DataOffer>,
    /// Signature bytes, already length- and alphabet-checked (§5). The base64
    /// text is not kept: it lives in `raw`, and a second copy is a second
    /// thing that can go stale.
    pub signature_bytes: [u8; 64],
}

/// A lint observation that is not a refusal.
///
/// The obligation to sort set-arrays sits on the SIGNER; a composer sorts
/// whatever arrives regardless (§4). So an unsorted package composes correctly
/// and is still a package its producer should fix — which is a warning, not a
/// refusal. The signature covers the bytes as signed, so a composer handed an
/// unsorted package cannot repair it and must not try.
#[derive(Debug, Clone)]
pub struct Warning {
    pub rule: &'static str,
    pub spec: &'static str,
    pub detail: String,
}

impl Package {
    /// The exact input to Ed25519: the canonical form with `signature`
    /// removed (§5).
    pub fn signing_input(&self) -> String {
        canonical(&self.raw.without_member("signature"))
    }

    /// The canonical form of the complete signed package.
    pub fn canonical(&self) -> String {
        canonical(&self.raw)
    }

    pub fn accept(&self) -> Option<Accept> {
        self.data_offer.as_ref().and_then(|d| d.accept)
    }

    pub fn release(&self) -> Option<&Vec<String>> {
        self.data_offer.as_ref().and_then(|d| d.release.as_ref())
    }
}

/// Parse and structurally validate one package. The first rule that fires is
/// the finding — every fixture in the vector set is built so that only its own
/// rule can fire, so a single finding is the right shape for the answer.
pub fn parse_package(bytes: &[u8]) -> Result<Package, Finding> {
    let raw = parse_document(bytes)?;
    build(raw)
}

fn build(raw: JVal) -> Result<Package, Finding> {
    // Always-required members first, in the order §3 lists them, so a package
    // missing several is reported against the first one a reader would notice.
    let spec = req_str(&raw, "spec")?;
    let sovereign_id = req_str(&raw, "sovereign_id")?;
    check_sovereign_id(&sovereign_id, "sovereign_id")?;
    let issued = req_str(&raw, "issued")?;
    check_date(&issued, "issued")?;
    let expires = req_str(&raw, "expires")?;
    check_date(&expires, "expires")?;

    let identity_obj = req_obj(&raw, "identity")?;
    let trust_domain = req_str_at(identity_obj, "identity.trust_domain", "trust_domain")?;
    if trust_domain.is_empty() {
        return Err(Finding::new(
            "M_MISSING_MEMBER",
            "§3",
            "identity.trust_domain",
            "an empty trust domain",
        ));
    }
    let ca_fingerprint = req_str_at(identity_obj, "identity.ca_fingerprint", "ca_fingerprint")?;
    check_ca_fingerprint(&ca_fingerprint)?;

    let caps_obj = req_obj(&raw, "capabilities")?;
    let transports = req_set_array(caps_obj, "capabilities.transports", "transports")?;
    let fabric_fips_image = req_bool_at(
        caps_obj,
        "capabilities.fabric_fips_image",
        "fabric_fips_image",
    )?;
    let pqc_capable = req_bool_at(caps_obj, "capabilities.pqc_capable", "pqc_capable")?;
    let fabric_version = req_str_at(caps_obj, "capabilities.fabric_version", "fabric_version")?;

    // A block that is present but incomplete is refused, never defaulted —
    // silent defaults are how a link ends up with no rate limit (§3).
    let transport_offer = match raw.get("transport_offer") {
        None => None,
        Some(v) => {
            let o = as_obj(v, "transport_offer")?;
            Some(TransportOffer {
                endpoint: req_str_at(o, "transport_offer.endpoint", "endpoint")?,
                listener: req_bool_at(o, "transport_offer.listener", "listener")?,
                public_key: req_str_at(o, "transport_offer.public_key", "public_key")?,
                proposed_link_subnet: req_str_at(
                    o,
                    "transport_offer.proposed_link_subnet",
                    "proposed_link_subnet",
                )?,
                measured_mtu: req_int_at(o, "transport_offer.measured_mtu", "measured_mtu")?,
            })
        }
    };

    let data_offer = match raw.get("data_offer") {
        None => None,
        Some(v) => Some(parse_data_offer(as_obj(v, "data_offer")?)?),
    };

    // `cut` is required alongside `data_offer` whichever halves it carries: a
    // peering with no agreed cut procedure gets severed badly, at speed, by
    // someone guessing (§3).
    if data_offer.is_some() && raw.get("cut").is_none() {
        return Err(Finding::new(
            "M_CUT_REQUIRED",
            "§3",
            "cut",
            "a package carrying `data_offer` must carry `cut`. A peering with no agreed cut \
             procedure gets severed badly, at speed, by someone guessing",
        ));
    }
    if let Some(c) = raw.get("cut") {
        as_obj(c, "cut")?;
    }

    let signature_bytes = decode_signature(&req_str(&raw, "signature")?)?;

    Ok(Package {
        raw,
        spec,
        sovereign_id,
        issued,
        expires,
        identity: Identity {
            trust_domain,
            ca_fingerprint,
        },
        capabilities: Capabilities {
            transports,
            fabric_fips_image,
            pqc_capable,
            fabric_version,
        },
        transport_offer,
        data_offer,
        signature_bytes,
    })
}

fn parse_data_offer(o: &[(String, JVal)]) -> Result<DataOffer, Finding> {
    let release = match find(o, "release") {
        None => None,
        Some(v) => {
            let items = set_array(v, "data_offer.release")?;
            Some(items)
        }
    };

    // The receive half is all three or none. A PARTIAL receive half is refused,
    // because a limit that is not stated is a limit that does not exist — §7's
    // rule that a flow is never emitted with a limit it does not have, seen
    // from the other end (§3).
    let names = [
        "accept_hz",
        "accept_bytes_per_message",
        "accept_distinct_keys",
    ];
    let present: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| find(o, n).is_some())
        .collect();
    let accept = match present.len() {
        0 => None,
        3 => Some(Accept {
            hz: req_int_at(o, "data_offer.accept_hz", "accept_hz")?,
            bytes_per_message: req_int_at(
                o,
                "data_offer.accept_bytes_per_message",
                "accept_bytes_per_message",
            )?,
            distinct_keys: req_int_at(
                o,
                "data_offer.accept_distinct_keys",
                "accept_distinct_keys",
            )?,
        }),
        _ => {
            let missing: Vec<&str> = names
                .iter()
                .copied()
                .filter(|n| find(o, n).is_none())
                .collect();
            return Err(Finding::new(
                "M_PARTIAL_DATA_OFFER",
                "§3",
                "data_offer",
                format!(
                    "the receive half is all three `accept_*` values or none; this one is \
                     missing {}. A limit that is not stated is a limit that does not exist",
                    missing.join(", ")
                ),
            ));
        }
    };

    if release.is_none() && accept.is_none() {
        return Err(Finding::new(
            "M_DATA_OFFER_NO_HALF",
            "§3",
            "data_offer",
            "a `data_offer` must carry at least one half — `release` to send, or all three \
             `accept_*` to receive",
        ));
    }
    Ok(DataOffer { release, accept })
}

// ---------------------------------------------------------------------------
// Field rules
// ---------------------------------------------------------------------------

/// 2–16 characters, lowercase `a-z0-9-` (§3). The value is not freely chosen —
/// it comes from ISO 3166-1 alpha-3 or an established short name — but that is
/// a naming rule a validator cannot enforce, so only the shape is checked.
fn check_sovereign_id(v: &str, path: &str) -> Result<(), Finding> {
    let ok = (2..=16).contains(&v.len())
        && v.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
    if ok {
        Ok(())
    } else {
        Err(Finding::new(
            "M_SOVEREIGN_ID",
            "§3",
            path,
            format!("`{v}` is not 2–16 characters of lowercase a-z0-9-"),
        ))
    }
}

/// `YYYY-MM-DD`, no times and no zones, so there is no clock skew between the
/// parties (§4). Checked on the shape as well as the value: chrono's parser
/// accepts `2026-9-5`, which is a different string for the same day and would
/// canonicalise to bytes the other side never produced.
fn check_date(v: &str, path: &str) -> Result<(), Finding> {
    let shaped = v.len() == 10
        && v.as_bytes()[4] == b'-'
        && v.as_bytes()[7] == b'-'
        && v.bytes()
            .enumerate()
            .all(|(n, c)| n == 4 || n == 7 || c.is_ascii_digit());
    if shaped && chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok() {
        Ok(())
    } else {
        Err(Finding::new(
            "M_DATE_FORMAT",
            "§4",
            path,
            format!("`{v}` is not a `YYYY-MM-DD` date"),
        ))
    }
}

/// `sha256:` followed by exactly 64 lowercase hex characters (§5.1). The
/// encoding is pinned because this value is read aloud over a telephone call —
/// there is no file extension to infer it from.
fn check_ca_fingerprint(v: &str) -> Result<(), Finding> {
    let hex = v.strip_prefix("sha256:");
    let ok = matches!(hex, Some(h) if h.len() == 64
        && h.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    if ok {
        Ok(())
    } else {
        Err(Finding::new(
            "M_CA_FINGERPRINT",
            "§5.1",
            "identity.ca_fingerprint",
            format!("`{v}` is not `sha256:` followed by exactly 64 lowercase hex characters"),
        ))
    }
}

/// Ed25519 signatures are exactly 64 bytes, which is exactly 88 base64
/// characters ending in one `=`. Any other length is refused BEFORE
/// verification is attempted, so a wrong-shaped value fails as a malformed
/// package rather than as a bad signature — two different findings that would
/// otherwise look alike (§5).
pub fn decode_signature(v: &str) -> Result<[u8; 64], Finding> {
    let bad = |detail: String| Finding::new("M_SIGNATURE_ENCODING", "§5", "signature", detail);
    if v.contains('-') || v.contains('_') {
        return Err(bad(
            "the URL-safe base64 alphabet. §5 pins the standard alphabet of RFC 4648 §4; a value \
             containing `-` or `_` is malformed, not something to be decoded a second way"
                .to_string(),
        ));
    }
    // 64 bytes is 88 base64 characters ending in TWO `=`. §5's prose says "one
    // `=`", which is arithmetically impossible for a 64-byte value — 64 mod 3
    // is 1, so the final quantum carries one byte and takes two pad
    // characters. Every signature in the conformance vectors ends `==`, and
    // the vectors are normative, so the length rule §5 is reaching for is
    // implemented here and its example count is not. Reported upstream rather
    // than worked around silently. The §5.1 key rule — 32 bytes, 44
    // characters, one `=` — is correct and is enforced as written.
    if v.len() != 88 || !v.ends_with("==") || v.ends_with("===") {
        return Err(bad(format!(
            "{} base64 characters ending `{}`; an Ed25519 signature is exactly 64 bytes, which is \
             exactly 88 characters ending in `==`",
            v.len(),
            &v[v.len().saturating_sub(2)..]
        )));
    }
    let raw = decode_b64_strict(v).map_err(bad)?;
    raw.try_into()
        .map_err(|_| bad("did not decode to exactly 64 bytes".to_string()))
}

/// The 32-byte raw Ed25519 public key, base64, exactly 44 characters ending in
/// one `=` (§5.1). Not PEM, not DER, not OpenSSH — and the check is worth
/// making because PEM is what most libraries hand you: a `SubjectPublicKeyInfo`
/// is the same 32 bytes behind a fixed 12-byte DER prefix, 60 base64
/// characters always beginning `MCowBQYDK2Vw`.
pub fn decode_verifying_key(v: &str) -> Result<[u8; 32], String> {
    let v = v.trim();
    if v.starts_with("MCowBQYDK2Vw") {
        return Err(format!(
            "`{}…` is a PEM/DER SubjectPublicKeyInfo, not a raw key. §5.1 pins the 32-byte raw \
             Ed25519 public key: exactly 44 base64 characters ending in one `=`. Every Ed25519 \
             library has a raw public-key entry point; that is the one to use",
            &v[..12.min(v.len())]
        ));
    }
    if v.len() != 44 || !v.ends_with('=') || v.ends_with("==") {
        return Err(format!(
            "{} base64 characters; §5.1 pins exactly 44, ending in one `=`",
            v.len()
        ));
    }
    let raw = decode_b64_strict(v)?;
    raw.try_into()
        .map_err(|_| "did not decode to exactly 32 bytes".to_string())
}

/// Strict base64: standard alphabet, canonical padding, and a final quantum
/// whose unused bits are zero. `base64::engine::general_purpose::STANDARD`
/// already refuses trailing bits and non-canonical padding; the alphabet and
/// length are checked by the callers above.
fn decode_b64_strict(v: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    if v.bytes().any(|c| c.is_ascii_whitespace()) {
        return Err("whitespace inside the base64 value; decoding is strict (§5)".to_string());
    }
    base64::engine::general_purpose::STANDARD
        .decode(v)
        .map_err(|e| format!("strict base64 decode failed: {e}"))
}

// ---------------------------------------------------------------------------
// Lint warnings — obligations on the signer that a composer normalises anyway
// ---------------------------------------------------------------------------

pub fn warnings(pkg: &Package, source_bytes: &[u8]) -> Vec<Warning> {
    let mut out = Vec::new();

    for (name, items) in [
        (
            "capabilities.transports",
            Some(&pkg.capabilities.transports),
        ),
        ("data_offer.release", pkg.release()),
    ] {
        let Some(items) = items else { continue };
        let normalised = super::json::normalise_set(items);
        if &normalised != items {
            out.push(Warning {
                rule: "W_SET_NOT_NORMALISED",
                spec: "§4",
                detail: format!(
                    "`{name}` is not de-duplicated and sorted by UTF-16 code unit. A signer is \
                     obliged to emit it normalised; a composer normalises it anyway, so this \
                     composes correctly — but the signature covers the bytes as signed, so no \
                     composer can repair the package itself"
                ),
            });
        }
    }

    if pkg.canonical().as_bytes() != source_bytes {
        out.push(Warning {
            rule: "W_NOT_CANONICAL",
            spec: "§4",
            detail: "this file is not byte-identical to its own canonical form. That is fine for \
                     a reading aid and wrong for anything hashed or signed — the signing input is \
                     the canonical form, so a signer working from these bytes signs the wrong \
                     thing"
                .to_string(),
        });
    }

    if pkg.spec != PACKAGE_SPEC {
        out.push(Warning {
            rule: "W_SPEC_NOT_V1",
            spec: "§6 step 1",
            detail: format!(
                "`spec` is `{}`, not `{PACKAGE_SPEC}`. This is not a structural defect and is \
                 deliberately not refused here: the version test is a literal pin applied at \
                 composition step 1, so `goat federate compose` will refuse this package with \
                 C_SPEC_PIN",
                pkg.spec
            ),
        });
    }

    if pkg.transport_offer.is_none() && pkg.data_offer.is_none() {
        out.push(Warning {
            rule: "W_RECOGNITION_ONLY",
            spec: "§3",
            detail: "this package carries neither `transport_offer` nor `data_offer`. That is a \
                     valid package — presence is the declaration — and it says this party is \
                     willing to be recognised and nothing more. No link and no flow can compose \
                     from it"
                .to_string(),
        });
    } else if pkg.transport_offer.is_some()
        && !pkg
            .capabilities
            .transports
            .iter()
            .any(|t| TRANSPORT_PREFERENCE.contains(&t.as_str()))
    {
        out.push(Warning {
            rule: "W_NO_PREFERRED_TRANSPORT",
            spec: "§6 step 4a",
            detail: format!(
                "`capabilities.transports` names none of {:?}. The list is open, so this is a \
                 valid package — but composition tests the intersection against that preference \
                 order, so no counterparty can form a link with it",
                TRANSPORT_PREFERENCE
            ),
        });
    }

    out
}

// ---------------------------------------------------------------------------
// Small accessors
// ---------------------------------------------------------------------------

fn find<'a>(o: &'a [(String, JVal)], name: &str) -> Option<&'a JVal> {
    o.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn missing(path: &str) -> Finding {
    Finding::new("M_MISSING_MEMBER", "§3", path, "required member is absent")
}

fn wrong_type(path: &str, want: &str, got: &JVal) -> Finding {
    Finding::new(
        "M_WRONG_TYPE",
        "§3",
        path,
        format!("expected a {want}, found a {}", got.type_name()),
    )
}

fn as_obj<'a>(v: &'a JVal, path: &str) -> Result<&'a [(String, JVal)], Finding> {
    v.as_obj().ok_or_else(|| wrong_type(path, "object", v))
}

fn req_str(raw: &JVal, name: &str) -> Result<String, Finding> {
    let v = raw.get(name).ok_or_else(|| missing(name))?;
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| wrong_type(name, "string", v))
}

fn req_obj<'a>(raw: &'a JVal, name: &str) -> Result<&'a [(String, JVal)], Finding> {
    let v = raw.get(name).ok_or_else(|| missing(name))?;
    as_obj(v, name)
}

fn req_str_at(o: &[(String, JVal)], path: &str, name: &str) -> Result<String, Finding> {
    let v = find(o, name).ok_or_else(|| missing(path))?;
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| wrong_type(path, "string", v))
}

fn req_bool_at(o: &[(String, JVal)], path: &str, name: &str) -> Result<bool, Finding> {
    let v = find(o, name).ok_or_else(|| missing(path))?;
    v.as_bool().ok_or_else(|| wrong_type(path, "boolean", v))
}

fn req_int_at(o: &[(String, JVal)], path: &str, name: &str) -> Result<i64, Finding> {
    let v = find(o, name).ok_or_else(|| missing(path))?;
    v.as_int().ok_or_else(|| wrong_type(path, "integer", v))
}

fn req_set_array(o: &[(String, JVal)], path: &str, name: &str) -> Result<Vec<String>, Finding> {
    let v = find(o, name).ok_or_else(|| missing(path))?;
    set_array(v, path)
}

/// A set-array: non-empty, every element a string. **An empty array is not a
/// declaration** (§3) — `"release": []` is a malformed `data_offer` and the
/// package is refused, not a send half that happens to carry nothing.
fn set_array(v: &JVal, path: &str) -> Result<Vec<String>, Finding> {
    let items = v.as_arr().ok_or_else(|| wrong_type(path, "array", v))?;
    if items.is_empty() {
        return Err(Finding::new(
            "M_EMPTY_SET_ARRAY",
            "§3",
            path,
            "an empty array is not a declaration. It is refused, not read as a half that happens \
             to carry nothing",
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (n, item) in items.iter().enumerate() {
        let s = item
            .as_str()
            .ok_or_else(|| wrong_type(&format!("{path}[{n}]"), "string", item))?;
        out.push(s.to_string());
    }
    Ok(out)
}

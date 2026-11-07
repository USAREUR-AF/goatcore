// Composition — spec §6, §7, §8, §9.
//
// A PURE FUNCTION over two parsed packages, an evaluation date and a set of
// bound verifying keys. No network, no credentials, no host access, nothing
// read from the environment. That is not an implementation detail: a partner
// has to be able to trust that composing their package cannot touch anything,
// and the way to make that checkable is for the code that composes to have no
// way to reach anything.
//
// The crux (§2): the composed agreement has NO PERSPECTIVE. The two halves are
// sorted by `sovereign_id` into parties[0] and parties[1], and every derived
// field is expressed in terms of those positions, so both sides compute
// byte-identical output. If this code ever grows an `our_*`/`their_*`
// distinction, the format is broken.

use super::json::{canonical, normalise_set, utf16_cmp, Finding, JVal};
use super::package::{Accept, Package, AGREEMENT_SPEC, PACKAGE_SPEC, TRANSPORT_PREFERENCE};
use chrono::NaiveDate;
use std::collections::BTreeMap;

/// Verifying keys bound to a `sovereign_id` over the authenticated channel of
/// §5.1. **Never taken from a package** — a package carrying its own key
/// authenticates nothing.
#[derive(Debug, Default, Clone)]
pub struct KeyRing {
    keys: BTreeMap<String, [u8; 32]>,
}

impl KeyRing {
    pub fn insert(&mut self, sovereign_id: &str, key: [u8; 32]) {
        self.keys.insert(sovereign_id.to_string(), key);
    }
    pub fn get(&self, sovereign_id: &str) -> Option<&[u8; 32]> {
        self.keys.get(sovereign_id)
    }
}

#[derive(Debug)]
pub enum Composed {
    /// A complete agreement. Refusal produces no agreement and no digest, so
    /// there is no shape here that carries a partial one.
    Agreement {
        value: JVal,
        canonical: String,
        digest: String,
    },
    /// One of §6's seven refusals, naming which rule fired.
    Refused(Finding),
}

pub fn compose(p: &Package, q: &Package, now: NaiveDate, keys: &KeyRing) -> Composed {
    match refuse(p, q, now, keys) {
        Some(f) => Composed::Refused(f),
        None => {
            // Step 5: sort into positions. Everything below is expressed
            // against them, never against "us" and "them".
            let (a, b) = if utf16_cmp(&p.sovereign_id, &q.sovereign_id).is_lt() {
                (p, q)
            } else {
                (q, p)
            };
            let value = emit(a, b);
            let canonical = canonical(&value);
            let digest = digest_of(canonical.as_bytes());
            Composed::Agreement {
                value,
                canonical,
                digest,
            }
        }
    }
}

/// §6's seven ways to refuse, in the order the specification numbers them.
fn refuse(p: &Package, q: &Package, now: NaiveDate, keys: &KeyRing) -> Option<Finding> {
    // Step 0 — nothing below means anything on an unauthenticated package, so
    // this is first, and there is no partial result before it.
    for pkg in [p, q] {
        if let Some(f) = verify_signature(pkg, keys) {
            return Some(f);
        }
    }

    // Step 1 — a LITERAL pin, not a comparison of the two packages with one
    // another. Two packages both declaring v2 agree with each other, so an
    // implementation that only compares them composes under v1 rules and emits
    // a v1 agreement — verifying under Ed25519 a signature a later version may
    // have specified some other algorithm for. The attack moves both sides at
    // once, so nothing looks irregular.
    for pkg in [p, q] {
        if pkg.spec != PACKAGE_SPEC {
            return Some(Finding::new(
                "C_SPEC_PIN",
                "§6 step 1",
                format!("{}.spec", pkg.sovereign_id),
                format!(
                    "`{}` declares spec `{}`; this implementation composes `{PACKAGE_SPEC}` and \
                     nothing else. The pin is literal, not an equality test between the two \
                     packages: a pair both declaring another version agrees with itself and is \
                     refused here just the same",
                    pkg.sovereign_id, pkg.spec
                ),
            ));
        }
    }

    // Step 2 — two parties must be distinguishable at every layer.
    if p.sovereign_id == q.sovereign_id {
        return Some(Finding::new(
            "C_SAME_SOVEREIGN_ID",
            "§6 step 2",
            "sovereign_id",
            format!("both packages declare `{}`", p.sovereign_id),
        ));
    }
    if p.identity.trust_domain == q.identity.trust_domain {
        return Some(Finding::new(
            "C_SAME_TRUST_DOMAIN",
            "§6 step 2",
            "identity.trust_domain",
            format!("both packages declare `{}`", p.identity.trust_domain),
        ));
    }

    // Step 3 — the boundary is INCLUSIVE: a package whose `expires` equals
    // `now` is valid that day and composes. A term runs through the end of its
    // `expires` date, not up to its start.
    for pkg in [p, q] {
        let e = NaiveDate::parse_from_str(&pkg.expires, "%Y-%m-%d")
            .expect("expires shape checked at parse");
        if e < now {
            return Some(Finding::new(
                "C_EXPIRED",
                "§6 step 3",
                format!("{}.expires", pkg.sovereign_id),
                format!(
                    "`{}` expired on {} and `now` is {now}. `now` is the current date in UTC, \
                     never the local date of whoever happens to be composing",
                    pkg.sovereign_id, pkg.expires
                ),
            ));
        }
    }

    // Step 4 — applies ONLY when both packages carry `transport_offer`. When
    // they do not, no link was asked for and there is nothing to fail.
    if let (Some(pt), Some(qt)) = (&p.transport_offer, &q.transport_offer) {
        // 4(a) is stated against the PREFERENCE ORDER, not the raw
        // intersection. `capabilities.transports` is an open list, so two
        // packages each offering `quic` do intersect — and a bare intersection
        // test passes them, arriving at §7's selection with a shared entry the
        // order does not name and so nothing to choose. Testing against the
        // same ordered list selection walks means the pair either refuses here
        // or selects there, and never neither.
        if select_transport(p, q).is_none() {
            let shared: Vec<&str> = p
                .capabilities
                .transports
                .iter()
                .filter(|t| q.capabilities.transports.contains(t))
                .map(String::as_str)
                .collect();
            let detail = if shared.is_empty() {
                format!(
                    "`{}` offers {:?} and `{}` offers {:?}; the sets are disjoint",
                    p.sovereign_id,
                    p.capabilities.transports,
                    q.sovereign_id,
                    q.capabilities.transports
                )
            } else {
                format!(
                    "both parties offer {shared:?}, but the preference order is {:?} and names \
                     none of them. The list is open by design — a later version adds a transport \
                     by extending that one ordered list — so a shared entry outside it refuses \
                     here rather than reaching selection with nothing to choose",
                    TRANSPORT_PREFERENCE
                )
            };
            return Some(Finding::new(
                "C_NO_TRANSPORT_IN_PREFERENCE",
                "§6 step 4a",
                "capabilities.transports",
                detail,
            ));
        }
        if !pt.listener && !qt.listener {
            return Some(Finding::new(
                "C_NEITHER_LISTENER",
                "§6 step 4b",
                "transport_offer.listener",
                "neither party will accept a connection. Both have said they want a link and the \
                 pair cannot produce one",
            ));
        }
    }

    None
}

fn verify_signature(pkg: &Package, keys: &KeyRing) -> Option<Finding> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let Some(raw) = keys.get(&pkg.sovereign_id) else {
        return Some(Finding::new(
            "C_SIGNATURE",
            "§6 step 0",
            format!("{}.signature", pkg.sovereign_id),
            format!(
                "no verifying key is bound to `{}`. The key is never taken from the package — a \
                 package carrying its own key authenticates nothing — so it must be bound over a \
                 channel authenticated independently of the relationship being established \
                 (§5.1). Pass it with --key {}=<44 base64 characters>",
                pkg.sovereign_id, pkg.sovereign_id
            ),
        ));
    };
    let Ok(vk) = VerifyingKey::from_bytes(raw) else {
        return Some(Finding::new(
            "C_SIGNATURE",
            "§6 step 0",
            format!("{}.signature", pkg.sovereign_id),
            format!(
                "the key bound to `{}` is not a valid Ed25519 public key",
                pkg.sovereign_id
            ),
        ));
    };
    let sig = Signature::from_bytes(&pkg.signature_bytes);
    // The signature covers the canonical form with `signature` removed (§5) —
    // not the file on disk, which may carry any whitespace at all.
    if vk.verify(pkg.signing_input().as_bytes(), &sig).is_err() {
        return Some(Finding::new(
            "C_SIGNATURE",
            "§6 step 0",
            format!("{}.signature", pkg.sovereign_id),
            format!(
                "the signature on `{}` does not verify under the key bound to it. The signed \
                 bytes are the canonical form with `signature` removed, so a package that is not \
                 in canonical form fails here even when its content is right",
                pkg.sovereign_id
            ),
        ));
    }
    None
}

/// Walk the fixed preference order and take the first entry present in BOTH
/// `capabilities.transports`. Entries either party names that are not in the
/// order are ignored (§7).
fn select_transport(p: &Package, q: &Package) -> Option<&'static str> {
    TRANSPORT_PREFERENCE.into_iter().find(|kind| {
        p.capabilities.transports.iter().any(|t| t == kind)
            && q.capabilities.transports.iter().any(|t| t == kind)
    })
}

/// Build the agreement object of §8.1. `a` is parties[0], `b` is parties[1].
fn emit(a: &Package, b: &Package) -> JVal {
    let mut top: Vec<(String, JVal)> = Vec::new();

    top.push(("spec".into(), JVal::Str(AGREEMENT_SPEC.into())));

    // The EARLIER of the two, so a composed link inherits the shorter term and
    // neither party's term is silently extended by the other's (§7). Both are
    // `YYYY-MM-DD`, which orders correctly as a string.
    let expires = if a.expires <= b.expires {
        &a.expires
    } else {
        &b.expires
    };
    top.push(("expires".into(), JVal::Str(expires.clone())));

    top.push(("parties".into(), JVal::Arr(vec![party(a, b), party(b, a)])));

    // `transport` is emitted ONLY if both packages carry `transport_offer`.
    // Otherwise the whole member is absent — not null, not empty. One party
    // willing to connect is not a link (§7, §8.1).
    if let (Some(at), Some(bt)) = (&a.transport_offer, &b.transport_offer) {
        let kind = select_transport(a, b).expect("step 4a already found one");
        // Exactly one true → that party. Both true → parties[0]. Neither →
        // already refused at step 4b. The member holds the party that LISTENS
        // and is named for what it holds.
        let listener = if at.listener {
            &a.sovereign_id
        } else {
            &b.sovereign_id
        };
        let transport = JVal::Obj(vec![
            ("kind".into(), JVal::Str(kind.into())),
            // Position decides, so both sides agree without negotiating.
            (
                "link_subnet".into(),
                JVal::Str(at.proposed_link_subnet.clone()),
            ),
            ("listener".into(), JVal::Str(listener.clone())),
            // Never an average, never a default.
            (
                "mtu".into(),
                JVal::Int(at.measured_mtu.min(bt.measured_mtu)),
            ),
        ]);
        top.push(("transport".into(), transport));
    }

    // One entry per direction that is fully specified, ordered parties[0]
    // first. A flow is never emitted with a limit it does not have (§7).
    let mut flows = Vec::new();
    for (from, to) in [(a, b), (b, a)] {
        if let (Some(release), Some(accept)) = (from.release(), to.accept()) {
            flows.push(flow(from, to, release, accept));
        }
    }
    top.push(("flows".into(), JVal::Arr(flows)));

    JVal::Obj(top)
}

fn party(this: &Package, other: &Package) -> JVal {
    JVal::Obj(vec![
        (
            "ca_fingerprint".into(),
            JVal::Str(this.identity.ca_fingerprint.clone()),
        ),
        // Carried VERBATIM, per party. Never combined into one value: both
        // false and one-of-each are different situations and a single boolean
        // cannot tell them apart. What to do about a difference is a local
        // decision, rendered locally, and never inside the digest (§7).
        (
            "fabric_fips_image".into(),
            JVal::Bool(this.capabilities.fabric_fips_image),
        ),
        // Derived, never declared. A name that is never written cannot be
        // mistyped (§3.1).
        (
            "peering_cn".into(),
            JVal::Str(peering_cn(&this.sovereign_id, &other.sovereign_id)),
        ),
        ("sovereign_id".into(), JVal::Str(this.sovereign_id.clone())),
        (
            "trust_domain".into(),
            JVal::Str(this.identity.trust_domain.clone()),
        ),
    ])
}

fn flow(from: &Package, to: &Package, release: &[String], accept: Accept) -> JVal {
    let envelope = JVal::Obj(vec![
        (
            "bytes_per_message".into(),
            JVal::Int(accept.bytes_per_message),
        ),
        ("distinct_keys".into(), JVal::Int(accept.distinct_keys)),
        ("hz".into(), JVal::Int(accept.hz)),
    ]);
    JVal::Obj(vec![
        // The RECEIVER's declared values. You cannot be obliged to accept more
        // than you declared, and the sender cannot raise your ceiling (§7).
        ("envelope".into(), envelope),
        ("from".into(), JVal::Str(from.sovereign_id.clone())),
        (
            "publish_under".into(),
            JVal::Str(publish_under(&from.sovereign_id)),
        ),
        // The originating party's own release set, normalised per §4. The
        // agreement never inherits an ordering from a package.
        (
            "release".into(),
            JVal::Arr(normalise_set(release).into_iter().map(JVal::Str).collect()),
        ),
        ("to".into(), JVal::Str(to.sovereign_id.clone())),
    ])
}

/// `fed.<issuer>.<counterparty>.<ordinal>` (§3.1). The separator is a full
/// stop and this is load-bearing: a `sovereign_id` may itself contain a hyphen
/// (`nato-jfcnp`), so a hyphen-separated name would not parse unambiguously.
pub fn peering_cn(issuer: &str, counterparty: &str) -> String {
    format!("fed.{issuer}.{counterparty}.01")
}

/// The outbound republisher's name on OUR core, for the peering with
/// `counterparty`: `fed.<us>.<counterparty>.republisher.01`. Never presented to
/// the counterparty, and never agreed with them: it is our own client of our own
/// core, minted by our CA in peering phase 8 and matched by the subject `apply`
/// renders into the core's access-control fragment.
pub fn republisher_cn(us: &str, counterparty: &str) -> String {
    format!("fed.{us}.{counterparty}.republisher.01")
}

/// The wire name. Every party publishes across the link under
/// `from/<its own sovereign_id>/` — one rule, the same rule on both sides
/// (§3.2). Not the same thing as a release set, which is the local staging
/// name inside the publishing party.
pub fn publish_under(sovereign_id: &str) -> String {
    format!("from/{sovereign_id}/**")
}

/// §9: sha256 over the canonical form encoded UTF-8 and NOTHING ELSE — no
/// trailing newline, no byte-order mark, no leading or trailing whitespace.
/// The ordinary way to hash a canonical form is to write it to a file first,
/// at which point most editors, `echo`, here-documents and several standard
/// libraries add a newline on your behalf, and the digest changes over an
/// identical agreement.
pub fn digest_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

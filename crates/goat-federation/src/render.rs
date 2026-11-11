// Reading a composed agreement, and rendering our own configuration from it.
//
// The agreement is the ONLY input. Everything that crosses is derived from it
// exactly once, so the release set the republisher forwards and the prefix the
// far edge admits cannot drift apart by being typed twice.
//
// Two things this module deliberately does NOT do:
//
//   * It never renders a value the agreement does not determine. Paths, image
//     digests and endpoints are properties of our own deployment, not of the
//     peering, and the edge role refuses to run with any of them unset. Emitting
//     a plausible default here would turn its loud refusal into a
//     half-configured edge that looks deployed — which is the failure that
//     role's own comments were written against.
//   * It never writes anything. The caller decides that, and only `apply`
//     does.

use super::compose::{peering_cn, publish_under, republisher_cn};
use super::json::{parse_document, Finding, JVal};
use super::package::AGREEMENT_SPEC;

#[derive(Debug, Clone)]
pub struct Agreement {
    pub expires: String,
    pub parties: [Party; 2],
    pub transport: Option<Transport>,
    pub flows: Vec<Flow>,
}

#[derive(Debug, Clone)]
pub struct Party {
    pub sovereign_id: String,
    pub trust_domain: String,
    pub ca_fingerprint: String,
    pub fabric_fips_image: bool,
    pub peering_cn: String,
}

#[derive(Debug, Clone)]
pub struct Transport {
    pub kind: String,
    pub link_subnet: String,
    pub listener: String,
    pub mtu: i64,
}

#[derive(Debug, Clone)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub publish_under: String,
    pub release: Vec<String>,
    pub hz: i64,
    pub bytes_per_message: i64,
    pub distinct_keys: i64,
}

pub fn parse_agreement(bytes: &[u8]) -> Result<Agreement, Finding> {
    let raw = parse_document(bytes)?;
    let f =
        |rule: &'static str, path: &str, detail: String| Finding::new(rule, "§8.1", path, detail);

    let spec = raw
        .get("spec")
        .and_then(JVal::as_str)
        .ok_or_else(|| f("A_MISSING_MEMBER", "spec", "absent".into()))?;
    if spec != AGREEMENT_SPEC {
        return Err(f(
            "A_SPEC",
            "spec",
            format!("`{spec}`; this build reads `{AGREEMENT_SPEC}` and nothing else"),
        ));
    }
    let expires = raw
        .get("expires")
        .and_then(JVal::as_str)
        .ok_or_else(|| f("A_MISSING_MEMBER", "expires", "absent".into()))?
        .to_string();

    let parties_arr = raw.get("parties").and_then(JVal::as_arr).ok_or_else(|| {
        f(
            "A_MISSING_MEMBER",
            "parties",
            "absent or not an array".into(),
        )
    })?;
    if parties_arr.len() != 2 {
        return Err(f(
            "A_PARTIES",
            "parties",
            format!(
                "{} entries; an agreement carries exactly two",
                parties_arr.len()
            ),
        ));
    }
    let mut parties = Vec::with_capacity(2);
    for (n, p) in parties_arr.iter().enumerate() {
        let s = |name: &str| -> Result<String, Finding> {
            p.get(name)
                .and_then(JVal::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    f(
                        "A_MISSING_MEMBER",
                        &format!("parties[{n}].{name}"),
                        "absent".into(),
                    )
                })
        };
        parties.push(Party {
            sovereign_id: s("sovereign_id")?,
            trust_domain: s("trust_domain")?,
            ca_fingerprint: s("ca_fingerprint")?,
            fabric_fips_image: p
                .get("fabric_fips_image")
                .and_then(JVal::as_bool)
                .ok_or_else(|| {
                    f(
                        "A_MISSING_MEMBER",
                        &format!("parties[{n}].fabric_fips_image"),
                        "absent".into(),
                    )
                })?,
            peering_cn: s("peering_cn")?,
        });
    }

    let transport = match raw.get("transport") {
        None => None,
        Some(t) => {
            let s = |name: &str| -> Result<String, Finding> {
                t.get(name)
                    .and_then(JVal::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        f(
                            "A_MISSING_MEMBER",
                            &format!("transport.{name}"),
                            "absent".into(),
                        )
                    })
            };
            Some(Transport {
                kind: s("kind")?,
                link_subnet: s("link_subnet")?,
                listener: s("listener")?,
                mtu: t
                    .get("mtu")
                    .and_then(JVal::as_int)
                    .ok_or_else(|| f("A_MISSING_MEMBER", "transport.mtu", "absent".into()))?,
            })
        }
    };

    let mut flows = Vec::new();
    let flows_arr = raw
        .get("flows")
        .and_then(JVal::as_arr)
        .ok_or_else(|| f("A_MISSING_MEMBER", "flows", "absent or not an array".into()))?;
    for (n, fl) in flows_arr.iter().enumerate() {
        let s = |name: &str| -> Result<String, Finding> {
            fl.get(name)
                .and_then(JVal::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    f(
                        "A_MISSING_MEMBER",
                        &format!("flows[{n}].{name}"),
                        "absent".into(),
                    )
                })
        };
        let env = fl.get("envelope").ok_or_else(|| {
            f(
                "A_MISSING_MEMBER",
                &format!("flows[{n}].envelope"),
                "absent".into(),
            )
        })?;
        let i = |name: &str| -> Result<i64, Finding> {
            env.get(name).and_then(JVal::as_int).ok_or_else(|| {
                f(
                    "A_MISSING_MEMBER",
                    &format!("flows[{n}].envelope.{name}"),
                    "absent".into(),
                )
            })
        };
        let release: Vec<String> = fl
            .get("release")
            .and_then(JVal::as_arr)
            .ok_or_else(|| {
                f(
                    "A_MISSING_MEMBER",
                    &format!("flows[{n}].release"),
                    "absent".into(),
                )
            })?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        flows.push(Flow {
            from: s("from")?,
            to: s("to")?,
            publish_under: s("publish_under")?,
            release,
            hz: i("hz")?,
            bytes_per_message: i("bytes_per_message")?,
            distinct_keys: i("distinct_keys")?,
        });
    }

    let parties: [Party; 2] = parties.try_into().expect("length checked above");
    Ok(Agreement {
        expires,
        parties,
        transport,
        flows,
    })
}

impl Agreement {
    pub fn party(&self, sovereign_id: &str) -> Option<&Party> {
        self.parties.iter().find(|p| p.sovereign_id == sovereign_id)
    }
    pub fn counterparty(&self, ours: &str) -> Option<&Party> {
        self.parties.iter().find(|p| p.sovereign_id != ours)
    }
    /// The flow that originates with us — what we publish across the link.
    pub fn outbound(&self, ours: &str) -> Option<&Flow> {
        self.flows.iter().find(|f| f.from == ours)
    }
    /// The flow that arrives — what we must admit, and the limits we declared.
    pub fn inbound(&self, ours: &str) -> Option<&Flow> {
        self.flows.iter().find(|f| f.to == ours)
    }
}

/// A check `verify` runs against our own systems, without applying anything
/// (§10). "Agreeing is not the same as being able to deliver."
#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    /// Non-empty when `ok` is false.
    pub hint: String,
}

fn pass(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        detail: detail.into(),
        hint: String::new(),
    }
}
fn fail(name: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
        detail: detail.into(),
        hint: hint.into(),
    }
}

/// Run the agreement against our own configuration generator and assert the
/// output would be valid here. Reads local state; changes nothing.
pub fn verify_locally(ag: &Agreement, ours: &str, today: chrono::NaiveDate) -> Vec<Check> {
    let mut checks = Vec::new();

    let Some(us) = ag.party(ours) else {
        checks.push(fail(
            "party-present",
            format!(
                "this agreement is between `{}` and `{}`; neither is `{ours}`",
                ag.parties[0].sovereign_id, ag.parties[1].sovereign_id
            ),
            "Pass the sovereign_id this party actually holds. Conformance is a statement about \
             one party's own systems, so composing it for a party we are not is meaningless.",
        ));
        return checks;
    };
    let them = ag.counterparty(ours).expect("two distinct parties");
    checks.push(pass(
        "party-present",
        format!(
            "we are `{ours}`; the counterparty is `{}`",
            them.sovereign_id
        ),
    ));

    // The derived names are recomputed rather than trusted. A value that can
    // only be derived cannot be disagreed about — so a mismatch here means the
    // agreement was hand-edited after composition.
    let expect_ours = peering_cn(ours, &them.sovereign_id);
    let expect_theirs = peering_cn(&them.sovereign_id, ours);
    if us.peering_cn == expect_ours && them.peering_cn == expect_theirs {
        checks.push(pass(
            "derived-names",
            format!("`{expect_ours}` and `{expect_theirs}` re-derive from the two sovereign ids"),
        ));
    } else {
        checks.push(fail(
            "derived-names",
            format!(
                "the agreement carries `{}` and `{}`; the derived values are `{expect_ours}` and \
                 `{expect_theirs}`",
                us.peering_cn, them.peering_cn
            ),
            "A peering name is derived, never declared, so these cannot disagree unless the file \
             was edited after composition. Re-compose from the two packages.",
        ));
    }

    // An expired agreement is not a conformance failure — it is a link that
    // has run its term — but attesting to one is meaningless, so it stops here.
    match chrono::NaiveDate::parse_from_str(&ag.expires, "%Y-%m-%d") {
        Ok(e) if e >= today => checks.push(pass(
            "term",
            format!("runs through {} (today is {today})", ag.expires),
        )),
        Ok(e) => checks.push(fail(
            "term",
            format!("expired on {e}; today is {today}"),
            "Compose a fresh pair of packages. A conformance attestation over an expired \
             agreement asserts an ability to deliver something that may no longer be offered.",
        )),
        Err(_) => checks.push(fail(
            "term",
            format!("`{}` is not a YYYY-MM-DD date", ag.expires),
            "The agreement is malformed; re-compose it.",
        )),
    }

    // What we would have to admit, and where.
    match ag.inbound(ours) {
        Some(f) => {
            let derived = publish_under(&f.from);
            if f.publish_under == derived {
                checks.push(pass(
                    "ingress-prefix",
                    format!(
                        "admit `{derived}` on the edge, at most {} Hz, {} bytes per message, {} \
                         distinct keys — the limits we ourselves declared",
                        f.hz, f.bytes_per_message, f.distinct_keys
                    ),
                ));
            } else {
                checks.push(fail(
                    "ingress-prefix",
                    format!(
                        "flow declares `{}` but the derived prefix is `{derived}`",
                        f.publish_under
                    ),
                    "The publishing prefix is derived from the sovereign id and never negotiated, \
                     so this agreement was edited. Re-compose it.",
                ));
            }
        }
        None => checks.push(pass(
            "ingress-prefix",
            "nothing arrives under this agreement, so no ingress rule is rendered",
        )),
    }

    // What we would publish, and the republishing step it needs. This is the
    // one that catches an agreement nobody can actually deliver: the fabric
    // cannot rename key expressions in transit, so a release set has to be
    // lifted onto the wire name by a process on our own side.
    match ag.outbound(ours) {
        Some(f) => checks.push(pass(
            "egress-republish",
            format!(
                "republish {:?} onto `{}` — a process on our own side, over our own data; the \
                 fabric cannot rename key expressions in transit",
                f.release, f.publish_under
            ),
        )),
        None => checks.push(pass(
            "egress-republish",
            "we publish nothing under this agreement",
        )),
    }

    // A policy difference is a fact in the agreement and a decision here. The
    // agreement stays identical on both sides while two parties legitimately
    // reach different conclusions from it (§7).
    if us.fabric_fips_image != them.fabric_fips_image {
        checks.push(pass(
            "fips-parity",
            format!(
                "`{ours}` runs a validated build ({}) and `{}` runs one ({}) — a difference the \
                 agreement records per party and does not resolve. Whether it blocks is a local \
                 policy decision, applied here and never inside the digest",
                us.fabric_fips_image, them.sovereign_id, them.fabric_fips_image
            ),
        ));
    }

    match &ag.transport {
        Some(t) if link_addresses(&t.link_subnet).is_none() => checks.push(fail(
            "transport",
            format!("link subnet {} is not a /31 or a /30", t.link_subnet),
            "A peering link carries two hosts and nothing else, and each party's address is \
             derived from its position in the agreement. Have the party in parties[0] propose \
             a /31 or /30, and re-compose.",
        )),
        Some(t) => checks.push(pass(
            "transport",
            format!(
                "{} link on {}, MTU {}, {} listens",
                t.kind, t.link_subnet, t.mtu, t.listener
            ),
        )),
        None => checks.push(pass(
            "transport",
            "no transport member: the parties have agreed data terms and no path. Valid, and not \
             yet a working link — nothing can cross",
        )),
    }
    if ag.flows.is_empty() {
        checks.push(pass(
            "flows",
            "`flows: []` — the parties have agreed a path that carries nothing. Valid, and not an \
             error",
        ));
    }

    checks
}

/// The two usable addresses of an agreed link subnet, in party order:
/// `parties[0]` holds the first and `parties[1]` the second (spec §8.1: position
/// decides, so both sides agree without negotiating). A /31 uses both of its
/// addresses; a /30 skips its network and broadcast address. Anything else
/// carries more than two hosts, which a peering link never does, and is `None`.
pub fn link_addresses(subnet: &str) -> Option<(String, String, u8)> {
    let (ip, len) = subnet.split_once('/')?;
    let len: u8 = len.parse().ok()?;
    let base = u32::from(ip.parse::<std::net::Ipv4Addr>().ok()?);
    let first = match len {
        31 => base,
        30 => base.checked_add(1)?,
        _ => return None,
    };
    let second = first.checked_add(1)?;
    Some((
        std::net::Ipv4Addr::from(first).to_string(),
        std::net::Ipv4Addr::from(second).to_string(),
        len,
    ))
}

/// The words every generated file's first line carries after its comment
/// marker: the header this module writes today. A tool that must recognize a
/// generated file (goatbuilder's back-out deletes a generated partner record
/// and removes a spliced core fragment) matches [`GENERATED_MARKERS`], so the
/// file stays recognized when the wording names `goatbuilder` instead.
pub const GENERATED_BY: &str = "GENERATED by `goat federate apply`";

/// Every wording a generated file's header has carried or will carry:
/// [`GENERATED_BY`] and its goatbuilder form.
pub const GENERATED_MARKERS: &[&str] = &[GENERATED_BY, "GENERATED by `goatbuilder federate apply`"];

/// One rendered artifact: where it goes, and what is in it.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub path: String,
    pub body: String,
    /// What a reviewer should look at before this is merged or applied.
    pub note: String,
}

/// Render every artifact the agreement determines.
/// `edge_group` is the ansible group the edge runs in. `None` keeps the literal
/// `EDGE_HOST` placeholder, which is honest about being unset but leaves the
/// operator to type a value into a flow that exists so values are not typed —
/// so the driver always passes it.
pub fn render_all(
    ag: &Agreement,
    ours: &str,
    digest: &str,
    edge_group: Option<&str>,
    key_sets: &[String],
) -> Vec<Rendered> {
    let group = edge_group.unwrap_or("EDGE_HOST");
    let them = ag.counterparty(ours).expect("checked by verify_locally");
    let us = ag.party(ours).expect("checked by verify_locally");
    let inbound = ag.inbound(ours);
    let outbound = ag.outbound(ours);

    let level = match (&ag.transport, inbound.is_some() || outbound.is_some()) {
        (Some(_), true) => "exchanging",
        (Some(_), false) => "connected",
        (None, _) => "recognised",
    };

    let ingress: Vec<String> = inbound
        .map(|f| vec![f.publish_under.clone()])
        .unwrap_or_default();
    // THE EGRESS KEY SET IS OUR WIRE NAME, NOT THE RELEASE SET'S STAGING NAME.
    //
    // Two names, two jobs (`ops/federation/republisher/README.md`):
    //   release/weather/**   the STAGING name — internal, never on the wire
    //   from/<us>/weather/** the WIRE name    — what actually crosses
    //
    // The fabric cannot rename a key expression in transit, so the
    // republisher lifts the first onto the second BEFORE anything reaches the
    // edge. An egress rule naming the staging name therefore permits a key that
    // never appears on the link, and forbids the one that does.
    //
    // Rendering the staging name here made the two artifacts this command writes
    // in the same breath contradict each other: the republisher published under
    // `from/<us>/**` while the edge permitted only `release/**`, and the
    // counterparty's ingress — correctly derived from the same agreement — admitted
    // `from/<us>/**`. NEITHER NAME COULD CROSS: ours refused to send the wire name,
    // theirs refused to accept the staging name.
    //
    // `release` is still the republisher's SOURCE list — that is the one place the
    // staging name belongs, and it is rendered as such below.
    //
    // ONE WIRE KEY PER OFFERED SET, NEVER THE WHOLE WIRE NAME. A party with two
    // peerings has two republishers writing onto one core, each under
    // `from/<us>/<set>/**`. An edge granted `from/<us>/**` would carry every set
    // written there, so partner B would receive the set agreed only with partner C.
    // Each set is the agreement's own release entry put through the republisher's
    // mapping (the first segment replaced by `from/<us>`), so this is a narrowing of
    // what the agreement permits and never a list typed beside it.
    let egress: Vec<String> = outbound
        .map(|f| wire_keys(&f.publish_under, &f.release))
        .unwrap_or_default();
    // INTEREST STAYS ON THE WHOLE WIRE NAME. A receiver subscribes to all of
    // `from/<us>/**` (its landing store does), and zenoh refuses a subscriber
    // declaration wider than the rule that would admit it: with interest narrowed
    // to the sets, the declaration is refused, nothing is ever asked for, and
    // nothing crosses, with every link up. A declaration carries no data, so the
    // DATA rules are what is narrowed (put, delete, reply) and the INTEREST rules
    // keep the whole wire name.
    let egress_interest: Vec<String> = outbound
        .map(|f| vec![f.publish_under.clone()])
        .unwrap_or_default();

    let mut out = Vec::new();

    // 1 — the reviewed partner declaration, when the agreement grants anything.
    //
    // A recognition-only agreement grants no access, so there is no declaration
    // to write: `ops/federation/validate-partners.py` requires a non-empty
    // `ingress` and `egress`, and rendering empty lists would produce a file
    // that this repository's own validator rejects. Saying so is better than
    // emitting one and better than silently skipping it.
    if ingress.is_empty() && egress.is_empty() {
        out.push(Rendered {
            path: "(none) partner declaration".to_string(),
            body: format!(
                "This agreement grants {} no access in either direction, so there are no access\n\
                 rules to render. That is a valid agreement — the parties have recognised one\n\
                 another and agreed nothing crosses — and it is not an error.\n\n\
                 ops/federation/partners/ has no form for it today: validate-partners.py\n\
                 requires a non-empty `ingress` and `egress`, so a recognition-only peering\n\
                 cannot be declared there.\n",
                them.sovereign_id
            ),
            note: "Nothing to write. Recorded so the absence is deliberate rather than a gap."
                .to_string(),
        });
    } else {
        let mut y = String::new();
        y.push_str(&format!(
            "# GENERATED by `goat federate apply` from the composed agreement\n\
         # {digest}\n\
         # Do not hand-edit: every value below is derived from the agreement, and the\n\
         # agreement is what both parties hold identically. Editing here makes this side\n\
         # disagree with the other with nothing to compare.\n\n\
         sovereign_id: {}\n\
         level: {level}\n\n\
         # The Subject CN their peering router presents. Derived, never agreed by hand:\n\
         # fed.<issuer>.<counterparty>.<ordinal>, and a name that is never written cannot\n\
         # be mistyped.\n\
         partner_cn: {}\n\n",
            them.sovereign_id, them.peering_cn
        ));
        y.push_str("# Where their data may land on our side. The wire name, enforced.\ningress:\n");
        for k in &ingress {
            y.push_str(&format!("  - \"{k}\"\n"));
        }
        if ingress.is_empty() {
            y.push_str("  []   # nothing arrives under this agreement\n");
        }
        y.push_str("\n# What they may receive from us: each offered set under THE WIRE NAME it\n\
         # actually crosses as, `from/<us>/<set>/**`. Never the `release/**` staging name,\n\
         # which the republisher lifts off before anything reaches the edge.\negress:\n");
        for k in &egress {
            y.push_str(&format!("  - \"{k}\"\n"));
        }
        if egress.is_empty() {
            y.push_str("  []   # we publish nothing under this agreement\n");
        }
        y.push_str(&format!(
            "\nrationale: >-\n  Federation agreement {digest}, term through {}.\n  \
         Trust domain {}; CA fingerprint {}.\n",
            ag.expires, them.trust_domain, them.ca_fingerprint
        ));
        out.push(Rendered {
            path: format!("ops/federation/partners/{}.yaml", them.sovereign_id),
            body: y,
            note: "Open this as a pull request. The review is the control: widening a partner's \
               access should be as visible as any other change to production configuration."
                .to_string(),
        });
    }

    // 2 — the edge router's peering parameters.
    let mut g = String::new();
    g.push_str(&format!(
        "# GENERATED by `goat federate apply` from the composed agreement\n\
         # {digest}\n\
         #\n\
         # ONLY the values the agreement determines are set here. Everything the edge\n\
         # role also requires — trust paths, both certificate pairs, the inward dial\n\
         # target and the digest-pinned image — is a property of THIS deployment, not of\n\
         # the peering, and is deliberately left unset. The role refuses to run with any\n\
         # of them empty, and that refusal is the point: a half-configured edge looks\n\
         # deployed and is not.\n\n\
         goat_zenoh_edge_enabled: true\n\
         goat_zenoh_edge_partner_id: \"{}\"\n\
         goat_zenoh_edge_partner_cn: \"{}\"\n",
        them.sovereign_id, them.peering_cn
    ));
    g.push_str("\n# Where they may write. One prefix, derived from their sovereign id.\ngoat_zenoh_edge_ingress_key_exprs:\n");
    if ingress.is_empty() {
        g.push_str("  []\n");
    } else {
        for k in &ingress {
            g.push_str(&format!("  - \"{k}\"\n"));
        }
    }
    g.push_str("\n# What they may receive: the wire key of each offered set.\ngoat_zenoh_edge_egress_key_exprs:\n");
    if egress.is_empty() {
        g.push_str("  []\n");
    } else {
        for k in &egress {
            g.push_str(&format!("  - \"{k}\"\n"));
        }
    }
    g.push_str("\n# Where their interest may reach: our whole wire name. A declaration carries\n# no data; the sets above decide what is sent.\ngoat_zenoh_edge_egress_interest_key_exprs:\n");
    if egress_interest.is_empty() {
        g.push_str("  []\n");
    } else {
        for k in &egress_interest {
            g.push_str(&format!("  - \"{k}\"\n"));
        }
    }
    if let Some(f) = inbound {
        g.push_str(&format!(
            "\n# Ingress limits — the ceilings WE declared, so they bound what arrives here and\n\
             # nothing we send. A partner link is the one place load arrives that we do not\n\
             # control and cannot ask to slow down.\n\
             goat_zenoh_edge_downsampling_rules:\n  \
             - {{ key_expr: \"{}\", freq: {}.0 }}\n\
             goat_zenoh_edge_low_pass_key_exprs:\n  - \"{}\"\n\
             goat_zenoh_edge_low_pass_size_limit: {}\n\
             # Agreed ceiling on distinct keys across this link: {}. The edge has no\n\
             # interceptor for cardinality, so this one is not rendered as a rule — it is a\n\
             # number to watch, not a limit that enforces itself.\n",
            f.publish_under, f.hz, f.publish_under, f.bytes_per_message, f.distinct_keys
        ));
    }
    // ADR 1169 decision 3 — the partner's read of the key topic, when it holds
    // an audience slot.
    //
    // ABSENT BY DEFAULT, and that is the safe configuration rather than a gap:
    // "Unsealed exchange remains the default. With no key scope configured the
    // rule is not rendered at all" (ADR 1169, Consequences). Audience membership
    // is OUR governance decision and is not carried in the agreement — the
    // agreement grants a data lane — so this comes from an explicit operator
    // input and is never derived.
    //
    // Scoped per release set and NEVER `keys/**`: a member belongs to the
    // audiences it produces to or reads from and to no others, and a broad grant
    // would hand it every audience's bundle.
    if !key_sets.is_empty() {
        g.push_str(
            "\n# The partner's READ of the key topic, because it holds a slot in these\n\
             # release-set audiences (ADR 1169 decision 3). READ ONLY — `put` and `delete`\n\
             # are absent from the role's rule and must stay absent: anyone able to publish\n\
             # on keys/** can publish a bundle wrapping a DEK of their own choosing.\n\
             goat_zenoh_edge_key_bundle_key_exprs:\n",
        );
        for rs in key_sets {
            g.push_str(&format!("  - \"keys/{rs}/**\"\n"));
        }
    }

    out.push(Rendered {
        path: format!(
            "ops/ansible/group_vars/{}/federation-{}.yml",
            group,
            them.sovereign_id
        ),
        body: g,
        note: "Merge this into the edge group's vars. The role \
               asserts every remaining parameter is set and fails loud if one is not — leave \
               it that way."
            .to_string(),
    });

    // 3 — the outbound republisher, if anything leaves.
    if let Some(f) = outbound {
        let mut r = String::new();
        r.push_str(&format!(
            "# GENERATED by `goat federate apply` from the composed agreement\n\
             # {digest}\n\
             #\n\
             # The outbound half of the symmetric publishing rule. Our data is staged under\n\
             # a local `release/…` name; the wire name is `from/{}/`. The fabric cannot\n\
             # rename key expressions in transit, so attaching the wire name is a small\n\
             # process on our own side, over our own data.\n\
             #\n\
             # It runs beside our CORE router and both of its sessions dial that core: it\n\
             # reads the release sets below and writes the wire name, and the edge carries\n\
             # the wire name out. The core admits it by this CN (the core fragment's\n\
             # `fed-rp-<them>` subject); phase 8 mints its certificate as republisher.pem.\n\n\
             goat_fed_republisher_sovereign_id: \"{}\"\n\
             goat_fed_republisher_partner_id: \"{}\"\n\
             goat_fed_republisher_cn: \"{}\"\n\
             goat_fed_republisher_publish_under: \"{}\"\n\
             goat_fed_republisher_release:\n",
            ours,
            ours,
            them.sovereign_id,
            republisher_cn(ours, &them.sovereign_id),
            f.publish_under
        ));
        for k in &f.release {
            r.push_str(&format!("  - \"{k}\"\n"));
        }
        out.push(Rendered {
            path: format!(
                "ops/ansible/group_vars/{}/federation-{}-republish.yml",
                group,
                them.sovereign_id
            ),
            body: r,
            note: "The release set here and the far side's ingress prefix come from the same \
                   agreement, so they cannot drift apart by being typed twice."
                .to_string(),
        });
    }

    // 3a — the agreed link, for whichever transport play phase 11 runs. Every value
    // is the agreement's: the two addresses by party position, the MTU, who listens.
    // The far edge's WireGuard key and endpoint are NOT in the agreement (they are
    // the counterparty package's `transport_offer`), and the transport role reads
    // them from that package.
    if let Some(t) = &ag.transport {
        let position = ag
            .parties
            .iter()
            .position(|p| p.sovereign_id == ours)
            .expect("checked by verify_locally");
        match link_addresses(&t.link_subnet) {
            Some((first, second, len)) => {
                let (local, remote) = if position == 0 {
                    (first, second)
                } else {
                    (second, first)
                };
                let we_listen = t.listener == ours;
                let dial = if we_listen { "" } else { remote.as_str() };
                let mut l = String::new();
                l.push_str(&format!(
                    "# GENERATED by `goat federate apply` from the composed agreement\n\
                     # {digest}\n\
                     #\n\
                     # The agreed link. Do not hand-edit: the far side derives the mirror of\n\
                     # these values from the same bytes.\n\n\
                     goat_peering_transport_kind: \"{}\"\n\
                     goat_peering_our_sovereign: \"{ours}\"\n\
                     goat_peering_partner_sovereign: \"{}\"\n\
                     goat_peering_our_trust_domain: \"{}\"\n\
                     goat_peering_partner_trust_domain: \"{}\"\n\
                     goat_peering_local_link_ip: \"{local}\"\n\
                     goat_peering_remote_link_ip: \"{remote}\"\n\
                     goat_peering_link_prefix_len: {len}\n\
                     goat_peering_mtu: {}\n\
                     goat_peering_we_listen: {we_listen}\n\n\
                     # The edge faces the partner on the link address and nothing else. The\n\
                     # party that does not listen dials the far link address; the party that\n\
                     # listens dials nothing.\n\
                     goat_zenoh_edge_bind_address: \"{local}\"\n\
                     goat_zenoh_edge_partner_dial: \"{dial}\"\n",
                    t.kind, them.sovereign_id, us.trust_domain, them.trust_domain, t.mtu
                ));
                out.push(Rendered {
                    path: format!(
                        "ops/ansible/group_vars/{}/federation-{}-transport.yml",
                        group, them.sovereign_id
                    ),
                    body: l,
                    note: "What phases 11 and 12 read: the transport play and the edge's bind \
                           address both come from this one file."
                        .to_string(),
                });
            }
            // `verify` refuses such a subnet before anything reaches `apply`.
            None => out.push(Rendered {
                path: format!("(refused) the link subnet {}", t.link_subnet),
                body: String::new(),
                note: "Not a /31 or /30, so no transport values are rendered.".to_string(),
            }),
        }
    }

    // 4 — the trust anchor to place, stated rather than written: this tool has
    // no copy of the counterparty's CA, and inventing a path to it would be
    // the single most damaging mistake available in this procedure.
    out.push(Rendered {
        path: format!("(manual) the trust anchor for {}", them.sovereign_id),
        body: format!(
            "The counterparty's CA must fingerprint to:\n\n    {}\n\n\
             Place it at a path used by NOTHING else, and point\n\
             goat_zenoh_edge_partner_ca_path at that file. Never at a core router's trust\n\
             file: the edge's global trust anchor is the PARTNER's CA, and pointing it at\n\
             the core's file puts a partner's authority on the machine that holds\n\
             everything.\n\n\
             This tool holds no copy of their CA and will not guess a path to it. The\n\
             fingerprint above was established over a channel authenticated independently\n\
             of this link (§5.1) — check the file against it before it is placed.\n",
            them.ca_fingerprint
        ),
        note: "Not a file this command writes. The fingerprint is the check to run on \
               whatever you place."
            .to_string(),
    });

    // 5 — the zenoh access-control fragment, in the region format
    // `acl-assemble.py` splices.
    //
    // WHY THIS EXISTS: cross-sovereign-peering-transport.md listed, as a real gap,
    // that "the declaration is not yet generated from — `ops/federation/` validates
    // the shape; nothing yet renders ACL subjects and rules from it, so the
    // agreement and the configuration are still two artifacts that can disagree."
    // They can only disagree while the second one is typed.
    //
    // THREE OBJECTS, NOT ONE. Zenoh's access control separates them and a rule
    // carrying its own `subjects` does not load:
    //   rule    { id, messages, flows, permission, key_exprs }
    //   subject { id, cert_common_names }
    //   policy  { rules: [ids], subjects: [ids] }   — the binding
    //
    // `query`, never `get`: the older keyword is rejected by zenoh-config 1.9.0
    // (config.json5.j2 records that it surfaced at first re-fire).
    //
    // Emitted as a REVIEWABLE FRAGMENT under the region headers. `acl-assemble.py`
    // owns the splice and the portal owns the org fragment; a tool that edited a
    // running router's access control from an agreement would be exactly the
    // central override the brokerless design refuses.
    {
        let sub_id = format!("fed-{}", them.sovereign_id);
        let w_id = format!("fed-w-{}", them.sovereign_id);
        let r_id = format!("fed-r-{}", them.sovereign_id);
        let wi_id = format!("fed-w-interest-{}", them.sovereign_id);
        let ri_id = format!("fed-r-interest-{}", them.sovereign_id);

        let mut a = String::new();
        a.push_str("// GENERATED by `goat federate apply` from the composed agreement\n");
        a.push_str(&format!("// {digest}\n//\n"));
        a.push_str(&format!(
            "// Subject is the peering CN, DERIVED (spec section 3.1) and never agreed in\n// prose: {}. The access layer matches this name and cannot see which\n// authority issued the certificate, which is why the far side's CA\n// belongs only in the edge's own trust store and never in a core router's.\n",
            them.peering_cn
        ));

        a.push_str("##ACL-ORG-REGION storages\n");
        a.push_str("##ACL-ORG-REGION write-rules\n");
        // DATA FLOWS ONE WAY PER KEY SET AND INTEREST FLOWS THE OTHER. The
        // core has to send the edge its subscriber declaration for the landing
        // prefix or the edge never learns anyone wants the partner's data, and it
        // has to accept the edge's declaration for our wire name and then send the
        // data. Each rule below names exactly one declared key set.
        let mut rule = |id: &str, messages: &str, flow: &str, keys: &[String]| {
            a.push_str("      {\n");
            a.push_str(&format!("        id: \"{id}\",\n"));
            a.push_str(&format!("        messages: [{messages}],\n"));
            a.push_str(&format!("        flows: [\"{flow}\"],\n"));
            a.push_str("        permission: \"allow\",\n");
            a.push_str("        key_exprs: [\n");
            for k in keys {
                a.push_str(&format!("          \"{k}\",\n"));
            }
            a.push_str("        ],\n      },\n");
        };
        if !ingress.is_empty() {
            rule(&w_id, "\"put\", \"delete\"", "ingress", &ingress);
            rule(
                &wi_id,
                "\"declare_subscriber\", \"query\", \"declare_queryable\"",
                "egress",
                &ingress,
            );
        }
        if !egress.is_empty() {
            rule(
                &r_id,
                "\"put\", \"delete\", \"declare_subscriber\", \"query\", \"reply\"",
                "egress",
                &egress,
            );
            rule(
                &ri_id,
                "\"declare_subscriber\", \"query\", \"declare_queryable\"",
                "ingress",
                &egress_interest,
            );
        }

        // THE OUTBOUND REPUBLISHER, our own client of our own core. It reads the
        // offered release sets and writes our wire name, and nothing else: the
        // edge's `fed-r-<them>` rule above then carries the wire name out. It is
        // the only writer of `from/<us>/**` this fragment admits.
        let rp_id = format!("fed-rp-{}", them.sovereign_id);
        let rp_rules = [
            format!("fed-rp-r-{}", them.sovereign_id),
            format!("fed-rp-r-interest-{}", them.sovereign_id),
            format!("fed-rp-w-{}", them.sovereign_id),
            format!("fed-rp-w-interest-{}", them.sovereign_id),
        ];
        let rp_release: Vec<String> = outbound.map(|f| f.release.clone()).unwrap_or_default();
        let republishes = !egress.is_empty() && !rp_release.is_empty();
        if republishes {
            rule(
                &rp_rules[0],
                "\"put\", \"delete\", \"declare_subscriber\", \"query\", \"reply\"",
                "egress",
                &rp_release,
            );
            rule(
                &rp_rules[1],
                "\"declare_subscriber\", \"query\", \"declare_queryable\"",
                "ingress",
                &rp_release,
            );
            rule(&rp_rules[2], "\"put\", \"delete\"", "ingress", &egress);
            rule(
                &rp_rules[3],
                "\"declare_subscriber\", \"query\", \"declare_queryable\"",
                "egress",
                &egress_interest,
            );
        }

        a.push_str("##ACL-ORG-REGION subjects\n");
        if !ingress.is_empty() || !egress.is_empty() {
            a.push_str("      {\n");
            a.push_str(&format!("        id: \"{sub_id}\",\n"));
            a.push_str(&format!(
                "        cert_common_names: [\"{}\"],\n      }},\n",
                them.peering_cn
            ));
        }
        if republishes {
            a.push_str("      {\n");
            a.push_str(&format!("        id: \"{rp_id}\",\n"));
            a.push_str(&format!(
                "        cert_common_names: [\"{}\"],\n      }},\n",
                republisher_cn(ours, &them.sovereign_id)
            ));
        }

        a.push_str("##ACL-ORG-REGION policies\n");
        let mut bound: Vec<&str> = Vec::new();
        if !ingress.is_empty() {
            bound.push(&w_id);
            bound.push(&wi_id);
        }
        if !egress.is_empty() {
            bound.push(&r_id);
            bound.push(&ri_id);
        }
        if bound.is_empty() {
            // A recognition-only agreement. Every region stays empty and that is the
            // honest output: the tunnel may come up and NOTHING will match. Rendering
            // a subject with no policy would look like a grant that failed to apply.
            a.push_str(
                "      // NO RULES, NO SUBJECT, NO POLICY. This agreement grants no access in\n                 \u{20}     // either direction. A tunnel may still come up; nothing will match it.\n                 \u{20}     // That is the difference between reachable and authorized.\n",
            );
        } else {
            let quoted: Vec<String> = bound.iter().map(|r| format!("\"{r}\"")).collect();
            a.push_str(&format!(
                "      {{ rules: [{}], subjects: [\"{sub_id}\"] }},\n",
                quoted.join(", ")
            ));
        }
        if republishes {
            let quoted: Vec<String> = rp_rules.iter().map(|r| format!("\"{r}\"")).collect();
            a.push_str(&format!(
                "      {{ rules: [{}], subjects: [\"{rp_id}\"] }},\n",
                quoted.join(", ")
            ));
        }

        out.push(Rendered {
            path: format!("ops/federation/acl/{}-fragment.json5", them.sovereign_id),
            body: a,
            note: "Region headers are what acl-assemble.py splices on. Review the key \
                   expressions against the agreement's flows before splicing: a prefix \
                   widened here is a grant, and the access layer matches the name only."
                .to_string(),
        });
    }

    let _ = us;
    out
}

/// The conformance attestation of §10: one line, over the digest, signed with
/// the same key that signed the package.
///
/// "At that point, and only then, both properties hold: the two sides provably
/// hold the same agreement, and each has proved it can implement it."
pub fn attestation_line(digest: &str) -> String {
    format!("conformance ok for {digest}")
}

/// The wire keys an agreement's outbound flow can produce: each release entry put
/// through the republisher's mapping (`ops/federation/republisher/src/mapping.rs`:
/// the staging root, the first segment, becomes `from/<us>`). `release/weather/**`
/// becomes `from/<us>/weather/**`. An entry with nothing below its root, or an empty
/// release list, yields the whole wire name, as before: nothing narrower is derivable.
pub fn wire_keys(publish_under: &str, release: &[String]) -> Vec<String> {
    let root = publish_under.strip_suffix("/**").unwrap_or(publish_under);
    let mut out: Vec<String> = Vec::new();
    for r in release {
        let k = match r.split_once('/') {
            Some((_, rest)) if !rest.is_empty() && rest != "**" => format!("{root}/{rest}"),
            _ => publish_under.to_string(),
        };
        if !out.contains(&k) {
            out.push(k);
        }
    }
    if out.is_empty() || out.iter().any(|k| k == publish_under) {
        return vec![publish_under.to_string()];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file this module writes opens with the words [`GENERATED_BY`]
    /// holds, so a reader matching [`GENERATED_MARKERS`] recognizes it.
    #[test]
    fn every_generated_file_opens_with_the_shared_header() {
        let ag = agreement();
        let files: Vec<Rendered> = render_all(&ag, "xaa", "sha256:test", Some("edge"), &[])
            .into_iter()
            .filter(|r| !r.path.starts_with('('))
            .collect();
        assert!(files.iter().any(|r| r.path.starts_with("ops/federation/partners/")));
        assert!(files.iter().any(|r| r.path.ends_with("-fragment.json5")));
        for r in &files {
            let first = r.body.lines().next().unwrap_or_default();
            assert!(first.contains(GENERATED_BY), "{}: {first}", r.path);
        }
        assert!(GENERATED_MARKERS.contains(&GENERATED_BY));
    }

    fn agreement() -> Agreement {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ops/federation/vectors/compose-full/agreement.canonical.json"),
        )
        .expect("the compose-full vector");
        parse_agreement(&bytes).expect("a readable agreement")
    }

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
    }

    /// Every rendered ingress prefix must sit under `from/<their id>/`, which
    /// is `validate-partners.py`'s rule and the only authenticated attribution
    /// available across a boundary.
    #[test]
    fn no_key_scope_is_rendered_unless_the_partner_holds_an_audience_slot() {
        let ag = agreement();
        let body = render_all(&ag, "xaa", "sha256:test", Some("edge"), &[])
            .into_iter()
            .find(|r| r.path.ends_with("federation-xab.yml"))
            .expect("edge group_vars")
            .body;
        // ADR 1169: "Unsealed exchange remains the default. With no key scope
        // configured the rule is not rendered at all, which is a safe and
        // deliberate configuration rather than a broken one."
        assert!(!body.contains("key_bundle_key_exprs"), "{body}");
    }

    #[test]
    fn a_partner_audience_renders_a_read_scoped_per_release_set() {
        let ag = agreement();
        let sets = vec!["weather".to_string(), "imagery".to_string()];
        let body = render_all(&ag, "xaa", "sha256:test", Some("edge"), &sets)
            .into_iter()
            .find(|r| r.path.ends_with("federation-xab.yml"))
            .expect("edge group_vars")
            .body;
        assert!(body.contains("goat_zenoh_edge_key_bundle_key_exprs:"), "{body}");
        assert!(body.contains("\"keys/weather/**\""), "{body}");
        assert!(body.contains("\"keys/imagery/**\""), "{body}");
        // NEVER the blanket scope — that would hand the partner every audience's
        // bundle, which is the one thing ADR 1169 decision 3 forbids by name.
        assert!(!body.contains("\"keys/**\""), "{body}");
    }

    #[test]
    fn rendered_ingress_is_always_the_counterpartys_own_prefix() {
        let ag = agreement();
        for (ours, theirs) in [("xaa", "xab"), ("xab", "xaa")] {
            let body = render_all(&ag, ours, "sha256:test", None, &[])
                .into_iter()
                .find(|r| r.path.ends_with(&format!("partners/{theirs}.yaml")))
                .expect("a partner declaration")
                .body;
            // SCOPED TO THE INGRESS BLOCK, not the whole file. A body-wide
            // assertion was only ever correct while `egress` carried the
            // `release/**` staging name; now that egress is our own wire name
            // `from/<ours>/**` legitimately appears further down, and a
            // whole-body check would forbid the correct output.
            let ingress_block = body
                .split("ingress:\n")
                .nth(1)
                .expect("an ingress block")
                .split("\negress:")
                .next()
                .expect("an egress block after it");
            assert!(
                ingress_block.contains(&format!("- \"from/{theirs}/**\"")),
                "ingress must admit the counterparty's own prefix\n{body}"
            );
            // And never our own prefix, which would admit their traffic under
            // an origin claim that is ours.
            assert!(
                !ingress_block.contains(&format!("- \"from/{ours}/**\"")),
                "ingress must never name our own prefix\n{body}"
            );
            // The other half of the same rule: egress is under OUR wire name, because
            // that is the only name our data ever carries on the link, one key per
            // offered set and never the whole prefix.
            let egress_block = body.split("\negress:\n").nth(1).expect("an egress block");
            let egress_keys: Vec<&str> = egress_block
                .lines()
                .take_while(|l| l.starts_with("  - "))
                .map(|l| l.trim_start_matches("  - ").trim_matches('"'))
                .collect();
            assert!(!egress_keys.is_empty(), "an egress key\n{body}");
            for k in &egress_keys {
                assert!(
                    k.starts_with(&format!("from/{ours}/")) && *k != format!("from/{ours}/**"),
                    "egress must be one of our own set's wire keys, never the whole prefix: {k}\n{body}"
                );
            }
            assert!(
                !egress_block.contains("release/"),
                "egress must never carry the staging name\n{body}"
            );
        }
    }

    /// A derived name is recomputed rather than trusted, so an agreement edited
    /// after composition is caught before anything is rendered from it.
    #[test]
    fn an_edited_peering_name_fails_verification() {
        let mut ag = agreement();
        ag.parties[0].peering_cn = "fed.xaa.xac.01".to_string();
        let checks = verify_locally(&ag, "xaa", today());
        let names = checks
            .iter()
            .find(|c| c.name == "derived-names")
            .expect("the check");
        assert!(!names.ok, "an edited peering name must not pass");
        assert!(names.hint.contains("Re-compose"), "{}", names.hint);
    }

    /// Conformance is a statement about one party's own systems.
    #[test]
    fn verifying_as_a_party_we_are_not_is_refused() {
        let checks = verify_locally(&agreement(), "usa", today());
        assert!(!checks[0].ok);
        assert_eq!(
            checks.len(),
            1,
            "nothing is checked once the party is wrong"
        );
    }

    /// An expired agreement stops before the attestation: attesting to one
    /// asserts an ability to deliver something that may no longer be offered.
    #[test]
    fn an_expired_agreement_is_not_attested() {
        let past = chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        let checks = verify_locally(&agreement(), "xaa", past);
        let term = checks.iter().find(|c| c.name == "term").expect("the check");
        assert!(!term.ok);
    }

    #[test]
    fn a_link_subnet_names_two_addresses_in_party_order() {
        assert_eq!(
            link_addresses("169.254.10.0/31"),
            Some(("169.254.10.0".into(), "169.254.10.1".into(), 31))
        );
        assert_eq!(
            link_addresses("169.254.10.0/30"),
            Some(("169.254.10.1".into(), "169.254.10.2".into(), 30))
        );
        // A subnet with room for a third host is not a peering link.
        assert_eq!(link_addresses("169.254.10.0/29"), None);
        assert_eq!(link_addresses("not-a-subnet"), None);
    }

    /// The two parties render mirrors of one link: each holds the address the other
    /// dials, and exactly one of them dials.
    #[test]
    fn both_parties_render_mirrors_of_one_link() {
        let ag = agreement();
        let t = ag.transport.as_ref().expect("compose-full carries a transport");
        let (first, second, _) = link_addresses(&t.link_subnet).expect("a /31 or /30");
        let body = |ours: &str, theirs: &str| {
            render_all(&ag, ours, "sha256:test", Some("edge"), &[])
                .into_iter()
                .find(|r| r.path.ends_with(&format!("federation-{theirs}-transport.yml")))
                .expect("a transport fragment")
                .body
        };
        let a = body("xaa", "xab");
        let b = body("xab", "xaa");
        assert!(a.contains(&format!("goat_peering_local_link_ip: \"{first}\"")), "{a}");
        assert!(a.contains(&format!("goat_peering_remote_link_ip: \"{second}\"")), "{a}");
        assert!(b.contains(&format!("goat_peering_local_link_ip: \"{second}\"")), "{b}");
        assert!(b.contains(&format!("goat_peering_remote_link_ip: \"{first}\"")), "{b}");
        assert!(a.contains(&format!("goat_peering_transport_kind: \"{}\"", t.kind)), "{a}");
        let dials = [&a, &b]
            .iter()
            .filter(|f| !f.contains("goat_zenoh_edge_partner_dial: \"\""))
            .count();
        assert_eq!(dials, 1, "exactly one party dials\n{a}\n{b}");
    }

    /// The core fragment grants interest against data on each key set, and
    /// never a wildcard: a data rule alone leaves a link ESTABLISHED and carrying
    /// nothing, with no denial logged.
    #[test]
    fn the_core_fragment_grants_interest_against_data_on_each_key_set() {
        let ag = agreement();
        let body = render_all(&ag, "xaa", "sha256:test", Some("edge"), &[])
            .into_iter()
            .find(|r| r.path.ends_with("acl/xab-fragment.json5"))
            .expect("a core fragment")
            .body;
        for id in ["fed-w-xab", "fed-w-interest-xab", "fed-r-xab", "fed-r-interest-xab"] {
            assert!(body.contains(&format!("id: \"{id}\"")), "{id}\n{body}");
        }
        assert!(
            body.contains("rules: [\"fed-w-xab\", \"fed-w-interest-xab\", \"fed-r-xab\", \"fed-r-interest-xab\"]"),
            "{body}"
        );
        assert!(!body.contains("\"**\""), "no rule names the whole keyspace\n{body}");
    }

    /// The republisher config and the far side's ingress rule come from one
    /// agreement, so they cannot drift apart by being typed twice.
    #[test]
    fn the_release_set_reaches_the_republisher_unaltered() {
        let ag = agreement();
        let body = render_all(&ag, "xaa", "sha256:test", None, &[])
            .into_iter()
            .find(|r| r.path.ends_with("-republish.yml"))
            .expect("a republisher fragment")
            .body;
        assert!(
            body.contains("goat_fed_republisher_publish_under: \"from/xaa/**\""),
            "{body}"
        );
        assert!(body.contains("release/weather/**"), "{body}");
        assert!(
            body.contains("goat_fed_republisher_cn: \"fed.xaa.xab.republisher.01\""),
            "{body}"
        );
        assert!(body.contains("goat_fed_republisher_partner_id: \"xab\""), "{body}");
    }

    /// The republisher is admitted to OUR core by its own subject: it reads the
    /// offered release sets and writes our wire name, and nothing else.
    #[test]
    fn the_core_fragment_admits_the_republisher_to_read_the_release_set_and_write_the_wire_name() {
        let ag = agreement();
        let body = render_all(&ag, "xaa", "sha256:test", Some("edge"), &[])
            .into_iter()
            .find(|r| r.path.ends_with("acl/xab-fragment.json5"))
            .expect("a core fragment")
            .body;
        assert!(body.contains("cert_common_names: [\"fed.xaa.xab.republisher.01\"]"), "{body}");
        assert!(
            body.contains("rules: [\"fed-rp-r-xab\", \"fed-rp-r-interest-xab\", \"fed-rp-w-xab\", \"fed-rp-w-interest-xab\"], subjects: [\"fed-rp-xab\"]"),
            "{body}"
        );
        let rule_keys = |id: &str| -> String {
            body.split(&format!("id: \"{id}\""))
                .nth(1)
                .and_then(|r| r.split("},").next())
                .unwrap_or_default()
                .to_string()
        };
        assert!(rule_keys("fed-rp-r-xab").contains("\"release/weather/**\""), "{body}");
        assert!(!rule_keys("fed-rp-r-xab").contains("from/"), "{body}");
        assert!(rule_keys("fed-rp-w-xab").contains("\"from/xaa/weather/**\""), "{body}");
        assert!(!rule_keys("fed-rp-w-xab").contains("\"from/xaa/**\""), "{body}");
        assert!(rule_keys("fed-rp-w-interest-xab").contains("\"from/xaa/**\""), "{body}");
        assert!(!rule_keys("fed-rp-w-xab").contains("release/"), "{body}");
        assert!(!body.contains("\"**\""), "no rule names the whole keyspace\n{body}");
    }
    /// The republisher's mapping, applied to the agreement's release entries.
    #[test]
    fn wire_keys_follow_the_republisher_mapping() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            wire_keys("from/xaa/**", &s(&["release/all/**", "release/xab/**"])),
            s(&["from/xaa/all/**", "from/xaa/xab/**"])
        );
        assert_eq!(wire_keys("from/xaa/**", &s(&["release/weather/eu/**"])), s(&["from/xaa/weather/eu/**"]));
        // Nothing narrower is derivable: the whole wire name, as before.
        assert_eq!(wire_keys("from/xaa/**", &s(&["release/**"])), s(&["from/xaa/**"]));
        assert_eq!(wire_keys("from/xaa/**", &[]), s(&["from/xaa/**"]));
        assert_eq!(wire_keys("from/xaa/**", &s(&["release/all/**", "release/**"])), s(&["from/xaa/**"]));
    }

    /// Two peerings on one core: the set agreed only with one partner is never in
    /// the other partner's egress, and the set both agreements offer is in both.
    #[test]
    fn a_bilateral_set_is_never_in_another_peerings_egress() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let b = wire_keys("from/xaa/**", &s(&["release/all/**", "release/xab/**"]));
        let c = wire_keys("from/xaa/**", &s(&["release/all/**", "release/xac/**"]));
        assert!(b.contains(&"from/xaa/all/**".to_string()) && c.contains(&"from/xaa/all/**".to_string()));
        assert!(!b.iter().any(|k| k.starts_with("from/xaa/xac") || k == "from/xaa/**"), "{b:?}");
        assert!(!c.iter().any(|k| k.starts_with("from/xaa/xab") || k == "from/xaa/**"), "{c:?}");
    }

    /// Every rule that lets our DATA out names the offered sets, not the whole wire
    /// name: the partner declaration, the edge's egress and the core's fed-r (the
    /// edge reads) and fed-rp-w (the republisher writes) rules. The interest rules
    /// keep the whole wire name, which carries no data.
    #[test]
    fn no_rendered_artifact_grants_the_whole_wire_name_outward() {
        let ag = agreement();
        let all = render_all(&ag, "xaa", "sha256:test", Some("edge"), &[]);
        let find = |suffix: &str| {
            all.iter()
                .find(|r| r.path.ends_with(suffix))
                .unwrap_or_else(|| panic!("no {suffix}"))
                .body
                .clone()
        };
        let partner = find("partners/xab.yaml");
        let egress = partner.split("\negress:\n").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(egress.contains("from/xaa/weather/**") && !egress.contains("\"from/xaa/**\""), "{partner}");
        let edge = find("group_vars/edge/federation-xab.yml");
        let e = edge.split("goat_zenoh_edge_egress_key_exprs:\n").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(e.contains("from/xaa/weather/**") && !e.contains("\"from/xaa/**\""), "{edge}");
        let frag = find("acl/xab-fragment.json5");
        let keys = |id: &str| frag.split(&format!("id: \"{id}\"")).nth(1).and_then(|r| r.split("},").next()).unwrap().to_string();
        // Data: the sets only.
        for id in ["fed-r-xab", "fed-rp-w-xab"] {
            assert!(keys(id).contains("\"from/xaa/weather/**\""), "{id}\n{frag}");
            assert!(!keys(id).contains("\"from/xaa/**\""), "{id} grants the whole wire name\n{frag}");
        }
        // Interest: the whole wire name, or a receiver's subscription is refused.
        for id in ["fed-r-interest-xab", "fed-rp-w-interest-xab"] {
            assert!(keys(id).contains("\"from/xaa/**\""), "{id}\n{frag}");
        }
        let ei = edge.split("goat_zenoh_edge_egress_interest_key_exprs:\n").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(ei.contains("\"from/xaa/**\""), "{edge}");
    }
}


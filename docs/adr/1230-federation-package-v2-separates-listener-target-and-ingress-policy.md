# ADR 1230 — Federation package v2 signs the listener target and ingress policy separately

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-13 |
| Repo scope | goat core |
| Amends | — |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md), [ADR 1218](1218-peering-forwarder-when-neither-edge-accepts-inbound.md), [ADR 1225](1225-federation-belongs-to-goatbuilder-one-command-one-decision.md), [ADR 1229](1229-a-partners-new-package-is-confirmed-within-confirmation-1.md), [`federation-package-spec.md`](../design/federation-package-spec.md), [`federation-package-v2-spec.md`](../design/federation-package-v2-spec.md) |

## TL;DR (for a layperson)

Version 2 records where a listener can be called separately from which network addresses it
allows to call. Missing information always stops. Calls from anywhere remain unsupported
initially; future support would require both a signed declaration and digest-bound local
authorization for that one agreement. Version 1 remains unchanged, and both parties must
deliberately choose version 2 before using it.

## Context

V1 `transport_offer.endpoint` has two operational meanings: the listener address a dialer uses,
and, when a counterparty endpoint is a literal IPv4 address, a firewall source hint. Issue
#438 showed that a truthful non-listener can lack a dial target. Removing endpoint from v1 would
break old readers and may silently broaden a listener or forwarder's exposed UDP port.

The existing `refuse-both-v2` and `refuse-version-mismatch` vectors intentionally contain
v1-shaped packages with changed spec strings and v1 signatures. They pin the behavior of a v1
implementation; they are not examples of the new v2 body. Dual-version dispatch therefore needs
its own expectations without changing the v1 corpus.

## Decision

1. **Keep v1 frozen.** V1 parsing, emission, signature, composition order, agreement bytes, and
   normative vector bytes remain unchanged. V1 conformance runs through an explicitly
   version-pinned v1 entry point.
2. **Add exact v2 versions with explicit production dispatch.** V2 packages say
   `federation-package/v2`; v2 agreements say `federation-agreement/v2`. A `spec` value first is
   an untrusted routing hint. Production dispatch first applies the version-independent,
   whole-input safety scan in the v2 specification. A routing-eligible v1/v1 pair then enters the
   complete frozen v1 structural, signature, and composition path with its identifiers unchanged.
   An input rejected by the pre-routing scan can therefore receive a different diagnostic from
   the version-pinned v1 entry point. Owners must approve that production-dispatch distinction;
   it changes neither v1 conformance nor the frozen v1 corpus. Other supported pairs authenticate
   the common envelope before treating versions as claims. Structural lint dispatches without a
   key and never claims authentication. There is no retry or downgrade.
3. **Select v2 explicitly per peering.** V1 remains the emission default. Both parties opt into
   v2 in reviewed configuration before either emits it. #438 remains refused under the v1
   default when a truthful v1 endpoint is absent.
4. **Separate listener facts.** A v2 listener carries `listener_path`, structured `dial_target`,
   and `ingress_policy`; a non-listener carries none of them. The selected listener's values are
   copied into agreement/v2 and covered by its digest.
5. **Declare policy at its observation points.** Direct policy has a `partner` rule. Forwarder
   policy has both a public-side `partner` rule and an edge-side non-empty `carrier_sources`
   list. Empty sources never mean any.
6. **Authenticate is not authorize.** The listener package signature proves what that listener
   declared. Unrestricted is unsupported initially. If implemented after owner approval, local
   reviewed configuration must bind its explicit opt-in to the exact canonical agreement digest;
   a boolean or version-wide opt-in is insufficient. The signed reason documents the choice but
   does not authorize it. No new approver, ticket, or expiry service is created.
7. **Make any future authorization narrow.** If unrestricted is later approved and implemented,
   its digest-bound opt-in authorizes UDP source-any only on the declared ingress interface and
   local listen port for that agreement's lifetime. It authorizes no other port, interface,
   protocol, peering, forwarder, or service. WireGuard peer-key authentication remains the
   compensating gate.
8. **No local source override.** Apply requires local source declarations to equal the agreement.
   Local policy may refuse or take the link down, but cannot widen or silently narrow the agreed
   rule. Forwarder apply also verifies public-interface/public-port mapping, carrier sources,
   carrier interface, and translated edge port.
9. **Enforce only an exclusively owned socket.** The initial direct profile proves one namespace,
   one UDP port, one listener, no DNAT/redirect/published-port competitor, one target address on
   the declared ingress interface, and no listener-side translation. Dialer-side source NAT is
   permitted only when the signed exact post-NAT source set is complete, stable for the agreement
   term, and verified at the listener observation point. At prerouting/mangle an interface-scoped IPv4
   allow precedes a final owned-port drop, which denies unlisted sources, IPv6, and unintended
   interfaces. If exclusive ownership, managed host-firewall equality, or absence of an
   intersecting unknown chain cannot be proved, apply refuses. Semantic read-back—not hashes—must
   prove hook, priority, rule order, sources, interface, socket, listener-side translation
   absence, dialer post-NAT evidence, and WireGuard state.
10. **Start with the smallest useful apply profile.** The first profile is direct, restricted,
    IPv4-source WireGuard with an IPv4 target **and agreement/v2-capable republisher flow
    enforcement**. DNS, IPv6, forwarder, unrestricted, listener-side translation, unverified
    dialer post-NAT sources, shared-port, unmanaged-firewall, and
    `ipsec-fips` combinations have exact refusals before apply. If owners choose transport-only
    scope instead, it is a disclosed milestone that refuses non-empty flows, not a useful
    data-carrying profile.
11. **Both-listener tie stays deterministic.** When both offer to listen, `parties[0]` is selected.
    The other party behaves only as the dialer for that agreement: it uses the selected target,
    does not open an inbound rule, and its unused listener policy is not copied to the agreement.
    There is no failover; changing listener requires new signed packages and recomposition.
12. **Rollback is honest.** An endpointless v2 peer cannot roll back to a truthful v1 transport
    package. Rollback may require withdrawing the transport offer and taking the link down until
    both sides can resume v2; no endpoint is fabricated.
13. **Make parsing deterministic and closed.** Every v2 package/agreement object has an exhaustive
    member grammar. Findings use one step-major total order across stable raw-byte candidate keys,
    typed paths with numeric array indexes, and the authoritative category rank. Address classes
    are fixed numeric ranges, not library predicates; documentation ranges are valid only for
    protocol vectors and refuse production applicability.
14. **Keep the approval boundary visible.** Before this ADR can be accepted, owners must approve
    the production-dispatch diagnostic distinction, closed v2 grammar and refusal identifiers,
    initial direct restricted IPv4 WireGuard profile, agreement/v2-capable republisher scope,
    listener-translation and dialer-source-NAT applicability rules, and explicit refusal of every
    other profile. The existing Rust/Go disagreement about an empty v1 endpoint is a separate v1
    erratum and is not decided here.

## Alternatives considered

- **Change v1 conditionally.** Breaks old readers and changes the meaning of normative fixtures.
- **Placeholder endpoint.** Signs false data and may accidentally authorize firewall exposure.
- **Signature alone authorizes unrestricted.** Confuses authentication with local authority.
- **Local empty list means any.** Recreates the silent widening being corrected.
- **Allow source overrides.** Makes the deployed policy differ from the compared agreement.
- **Ship all profiles immediately.** Expands #438 into IPsec, IPv6, forwarder, and DNS behavior
  before focused enforcement evidence exists.

## Consequences

**Positive.** Endpointless dialers can be represented truthfully after coordinated v2 adoption.
Exposure is signed, compared, locally authorized where necessary, and semantically verified at a
specified observation point. The initial profile refuses listener-side translation and any
dialer post-NAT source policy that cannot be proved complete and stable.

**Negative.** Readers and consumers need real version dispatch. Some existing forwarder
configurations use empty `partner_sources`; they cannot claim v2 conformance without an explicit
restricted rule. A future unrestricted profile also requires the separately approved digest-bound
authorization and enforcement evidence. Endpointless peers may have no live v1 rollback. This
Proposed ADR supplies no implementation or deployment validation and does not close issue #438.

## Assessor view (SCA-R)

The decision supports SC-7 and AC-4 by making each boundary source rule explicit and fail-closed,
and CM-3/CM-6 by binding the intended rule to signed configuration and observed enforcement.
Unrestricted ingress is not in the initial supported profile. A future digest-bound local opt-in
would scope residual exposure to one agreement and socket, with WireGuard peer authentication as
a compensating gate; its signed reason remains evidence, not authorization. No new waiver or
approval system is asserted.

## Trigger conditions for revisit

- A v2 IPsec enforcement contract is approved.
- IPv6, DNS, or forwarder profiles have task-level and isolated firewall evidence sufficient to
  move them into the supported matrix.
- Exact source addresses cannot represent a required NAT pool; CIDR policy then needs a separate
  decision and vectors.
- The local authorization mechanism for unrestricted cannot identify one peering and agreement
  version without ambiguity.

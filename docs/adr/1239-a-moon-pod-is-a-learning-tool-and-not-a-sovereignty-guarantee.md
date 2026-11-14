# ADR 1239 — A moon pod is a learning tool and not a sovereignty guarantee

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-14 |
| Deciders | operator |
| Repo scope | goat core (documents only) |
| Amends | [ADR 0223](0223-goat-moon-pod-repo-containment.md), [ADR 1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md), [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md), [ADR 1164](1164-per-site-spire-trust-domains.md) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1092](1092-moon-pod-peer-classes.md) (the `POD` class: a pod inside the environment's own identity planes), [ADR 0821](0821-mission-partner-sharing-pattern.md), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) (sovereign environments) |

## TL;DR (for a layperson)

A moon pod is a small appliance that joins a goat environment. Today it is used only to learn
how goat works. It does not protect the sovereignty of whoever runs it: the environment it
joins belongs to someone else, and that owner decides who may enrol, what the rules are, and
what data may leave the pod. It could become a real capability one day, but only if the pod's
operator accepts the environment owner as trusted and as the authority over moving data out of
the pod. A party that needs to keep control of its own data runs its own environment instead,
and federates with others across a boundary.

## Context

The moon pod was first described as a pod a mission partner runs, under the partner's own
control, inside an environment someone else manages, and presented as useful in real operations
([ADR 0223](0223-goat-moon-pod-repo-containment.md), the Tier-0 section of
[`mission-partner-sharing.md`](../design/mission-partner-sharing.md)). In practice it is used only
as a learning tool: rung 2 of the learner ladder.

The documents never caught up. A banner added to four of them on 2026-11-10 says the pod is a
learning tool and that sovereigns federate their own environments, but the text beneath still
promises sovereignty, and no document says who controls a pod or on what condition it could be
used for real. ADR 1092's `POD` class already says the factual part: a pod is a node inside the
environment's own mesh and identity planes.

The operator, 2026-11-14:

> "originally it was intended as a soveigen managed pod but within a [another party's] managed env and it
> was prsented as somethign useful in real operations, but it's only used a s a learnign tool and
> I'm not sure that the documentation has been updated everywhere. it could some day be used as a
> realcapability but it would require that the operator of the moon pod treate the envrinoment
> owner as trusted and as authoratative to their own ability to move data outside of that pod."

## Decision

### D1 — The position, stated once

Every goatcore document that describes a moon pod points here.

1. **A moon pod is used today only to learn.** It is rung 2 of the learner ladder, and it is not
   in operational use.
2. **It is not a sovereignty guarantee.** A pod joins an environment someone else runs, and that
   environment's owner controls enrolment, the access rules, and what data may leave the pod
   (ADR 1092, `POD`).
3. **It could become a real capability on one condition:** the pod's operator treats the
   environment owner as trusted, and as the authority over the pod's ability to move data outside
   the pod.
4. **A party that needs sovereignty runs its own environment and federates across a boundary**
   ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md),
   [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)). That is a
   different thing from a moon pod.

**Reason.** One statement that every document points to cannot drift the way four copied banners
did. Point 2 is what the architecture already does (ADR 1092); point 3 is the only honest basis on
which a pod could carry real data.

### D2 — How the amended claims now read

| ADR | Claim | Now reads |
|---|---|---|
| [0223](0223-goat-moon-pod-repo-containment.md), TL;DR | a turnkey "easy button" partners "can deploy in 24 hours that still preserves their data sovereignty" | a learning appliance (D1.1). It does not preserve the sovereignty of its operator (D1.2), and it is not offered for operational deployment |
| 0223, naming | "`moon` for sovereign-orbital partner relationships" | `moon` names the relationship plane of a pod. It does not mean the pod is sovereign |
| 0223, deliverables | the conformance suite: "CI runs continuously against `goat-moon-open-pod`" | the suite is a skeleton (`tests/moon-pod-conformance/README.md`). No CI runs it |
| 0223, out of scope | cross-moon-pod federation is out of scope because "each moon is sovereign-orbital" | still out of scope; "sovereign" is withdrawn (D1.2) |
| 0223, hosted SaaS | requirement 6, the "sovereignty floor", "requires partner-controlled custody of data, identity, and audit" | the pod's data, identity and audit sit inside an environment its owner controls (D1.2). Requirement 6 does not make the pod sovereign |
| 0223, consequences | `USAREUR-AF/zagros-moon-pod` "private during MVP" | the repository is public. It holds the learning pod |
| [1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md), external partners | a "sovereign goat partner (runs the goat stack but in a separate sovereign trust root — moon-pod / standalone-goatnet)" | that case is a party running its own environment (D1.4). A moon pod is not it: it joins an environment someone else runs |
| [1157](1157-cross-sovereign-federation-edge-terminated.md), section 1 | "Steps 2 and 3 are the moon-pod federation process already in the repo" | steps 2 and 3 are the SPIRE federation mechanism in the repository, which the learning pod also exercises. Cross-sovereign federation is between environments (D1.4), not with a moon pod |
| [1164](1164-per-site-spire-trust-domains.md), the asymmetry | the count of parties to re-establish "rises with every org onboarded and every moon-pod that federates — including the partner-pod shape … where a partner runs its own domain" | the count rises with every party that runs its own environment and federates (D1.4), and with every learning pod that federates. A partner running its own domain is that first case, not a moon pod |

The amended ADRs are not edited. Each one's row in `docs/adr/INDEX.md` notes this amendment.

**Reason.** ADRs are never rewritten, they are superseded or amended (`CONTRIBUTING.md`). The table lets a reader of each
claim find its correction without reading the old ADR differently from how it was written.

### D3 — The copied banners become pointers

The banners in ADR 0223, ADR 0821, `mission-partner-sharing.md` and `partner-data-exchange.md`
become one line each that points to D1. The design documents and READMEs that describe the pod as
sovereign or as operational are corrected to agree with D1. ADR 1092 is unchanged: its `POD`
definition is the source of D1.2.

**Reason.** A pointer cannot disagree with the position it points to.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Rewrite the amended ADRs in place | ADRs are never rewritten; the history of what was claimed is part of the record |
| Keep the four banners and expand each | Five copies of one position drift, as the four did |
| Withdraw the moon pod | It is the learner ladder's rung 2 and works for that |
| Describe the pod as sovereign "with caveats" | The owner of the environment controls the pod. A caveat that large is the statement |

## Consequences

- Readers of ADRs 0223, 1025, 1157 and 1164 meet the correction through their INDEX notes and
  through this ADR, not in the ADRs' own text.
- The sibling repositories (goatstuff, goatdocs, `zagros-moon-pod`, `hello-galaxy-zagros-pod`)
  state the same four points in their own home documents. Each corrects its own text.
- No code, role or configuration changes.

## Assessor view (SCA-R)

The change removes a claim of data sovereignty (SC-7 boundary protection, AC-4 information flow
enforcement) that the architecture never provided: a pod's information flow is enforced by the
environment that owns it. An assessor reviewing a moon pod should read it as a component inside
that environment's authorization boundary, not as a separate boundary.

## Trigger conditions for revisit

- A pod is proposed for operational use. D1.3's condition is then stated in that deployment's
  agreement, and this ADR is amended or superseded.
- The learner ladder drops or moves the moon pod's rung.

# ADR 1247 — Decoys are a detection layer, generated on demand from a seed; the mechanism is public and each goatnet's placements are private

| Field | Value |
|---|---|
| Status | Proposed. Decisions 1 to 14 are built except the Zenoh surface of decision 6 and provider-issued credentials (decision 2), which are designed. |
| Date | 2026-11-15 |
| Repo scope | goat core (`ops/decoy/`, `ops/ansible/roles/decoy/`, `ops/ansible/decoy-deploy.yml`, the decoy stanza of `ops/ansible/roles/mesh-coredns`, `docs/design/deception-layer.md`, `docs/operations/decoy-layer-runbook.md`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1123](1123-client-demand-must-not-amplify.md) (client demand must not become fabric load) · [ADR 1168](1168-self-hosted-paging-and-automatic-oncall-enrolment.md) (paging) · [ADR 1178](1178-external-service-golden-path.md) (external service declaration) · [ADR 1186](1186-credentials-at-rest-are-references-only.md) (credentials at rest are references) · [ADR 1220](1220-public-base-images-by-role-overlay-supplies-its-own.md) (the overlay supplies its own) · [`deception-layer.md`](../design/deception-layer.md) |

## TL;DR (for a layperson)

A goatnet can contain fake keys, fake notes and fake services that nobody has a real reason to touch. If anyone touches one, that person or program is an intruder, and the operator on call is paged at once. The fake services are produced by one small program that invents a consistent answer for whatever it is asked, so an intruder, and especially an automated AI agent, keeps exploring a world that does not exist while its search is recorded. How this works is published. Where a particular goatnet has placed its fakes is kept private.

## Context

goat's defenses decide who may connect, read and write: machine identity on the mesh, generated access rules, and grant encryption for data. None of them reports an intruder who holds a valid identity, for example on a compromised host. On the Zenoh fabric a denied request is dropped without a record, so a probing intruder is not seen there either.

Intrusions are increasingly carried out by AI agents that read whatever a host presents and follow every reference they find. Such an agent pays for each step in requests, tokens and time. The defender knows which names and credentials are real; the agent does not.

goat's public repositories contain the full mechanism of every defense. A decoy described in a public tree, or stored in any git tree, identifies itself.

## Decision

1. **Decoys are a detection layer and are never a control.** No access decision depends on a decoy. Any use of a decoy is treated as hostile and pages the on-call operator.
2. **There are four decoy kinds:** false documentation (statements that name further decoys), decoy credentials (plausible, bound to nothing), the decoy responder, and a one-line stop notice addressed to AI agents.
3. **False documentation states; it does not command.** Text that tries to override an AI agent's instructions is not used. The stop notice is the only imperative text.
4. **A decoy name is a plausible body and a keyed tag,** `<body>-<tag>`, where the tag is 40 bits of `HMAC-SHA-256(key, "tag\0" || body)`. Decoy hosts are single tagged labels under the zone the goatnet's services already use, so the existing wildcard certificate covers them and no decoy name enters a certificate log.
5. **Two keys are derived from one seed.** The recognition key tags what is planted on hosts and is never on a goatnet host. The responder key tags what the responder serves and is on the responder host only.
6. **The decoy responder is one stateless process** (`ops/decoy/decoy.py serve`). It computes each answer as a template filled from an HMAC of the request under the responder key. Each answer names further decoy hosts that resolve to the same responder. It runs no language model.
7. **A request reaches the responder through what the goatnet already runs:** one CoreDNS `template` stanza and one Traefik router, both matching the shape of a tagged label. No port, certificate or access rule is added. A real service name with that shape is refused at deploy.
8. **The responder is isolated and the isolation is tested.** It runs confined under systemd as its own user; one nftables rule drops any connection that user opens; the deploy fails if a connection attempt as that user succeeds. Each source is rate-limited.
9. **A touch pages through Alertmanager.** The responder writes counters to a file node-exporter already reads; a rule installed by the role fires on a touch. Each touch is one JSON line in the journal.
10. **One file per goatnet declares the layer:** `group_vars/<site>_hosts/deception.yml`. The seed is resolved per run from the goatnet's secret store. Files are rendered on the control machine and written to hosts; no decoy file is committed to any git tree.
11. **A decoy is recognized by key, not by list.** `decoy.py check` recomputes tags from the seed; an identifier a provider issues is listed in the declaration as a keyed digest. The verdict `decoy` means never use and never trust.
12. **Reading a decoy is not a touch; using one is.** A planted file whose content differs from the content computed from the seed is a touch, and the deploy stops on it.
13. **The mechanism is public and its use is private.** This repository holds the responder, the role, the schema and example templates. A goatnet's overlay holds its seed, declaration and templates. This repository states that a goatnet may carry decoys and does not state which do.
14. **A touch by a goatnet's own tooling is a defect in the placement or the tooling.** Touches are never suppressed by source.

## Alternatives considered

- **Instruction-override text in decoy files.** Rejected: it marks the file as a decoy to anyone reading an agent's transcript, its effect on a given agent cannot be predicted, and a goatnet's own agents read its hosts far more often than an intruder's do.
- **A stored decoy environment (decoy hosts, databases and services).** Rejected: it has to be built, kept consistent and kept current. Computing answers from a seed gives the same consistency with no stored state.
- **Answers written by a language model at request time.** Rejected: the goatnet would pay per request, which removes the cost difference the design depends on, and the responder would need an outbound dependency.
- **A zone of its own for decoy hosts.** Rejected as the default: it needs its own wildcard certificate, which a public certificate log would publish, and its own DNS delegation. Tagged labels under the existing zone need neither. A goatnet may still declare a zone, and `check` honors it.
- **The responder paging the pager directly.** Rejected: it would have to open a connection, which is the one thing its isolation forbids. A metric that Prometheus collects needs none.
- **A container image for the responder.** Rejected for now: one standard-library Python file under systemd needs no image build, no pin and no registry, and a systemd user is simpler to deny all outbound connections than a container on a shared Docker network.
- **Keeping the mechanism private as well.** Rejected: other goatnets could not use it, and the protection comes from the private seed, templates and placements. Publishing that decoys may exist makes an intruder doubt real credentials too.
- **A stored registry of decoy names or content hashes.** Rejected: wherever the registry is readable it is the map of the decoys, a content hash stops matching when a file is edited, and a name-only list lets an intruder name an implant after a decoy. A keyed tag needs no stored list and cannot be forged without the key.
- **Decoy files committed to the overlay repository.** Rejected: the repository's secret and identifier gates refuse them, and a decoy in a tree is labeled by its path and history.

## Consequences

- A goatnet gains a signal for an intruder who holds a valid identity.
- Each goatnet that enables the layer owns one more secret (the seed) and its own templates.
- Mesh DNS and Traefik on the responder's host answer for any name with the shape of a tagged label. A goatnet that enables the layer must never give a real service a name of that shape; the deploy refuses if one exists.
- The responder's host holds the responder key. A reader of that host can recognize responder names and cannot recognize planted ones.
- Operators and their AI agents must stay out of declared entry points; a page caused by a goatnet's own tooling costs on-call attention until the placement or the tooling is changed.
- What is designed and not built is listed in [`deception-layer.md`](../design/deception-layer.md) §Designed, not built.

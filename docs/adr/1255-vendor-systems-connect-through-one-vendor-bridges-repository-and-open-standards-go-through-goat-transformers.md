# ADR 1255 — Vendor systems connect through one vendor-bridges repository, and open standards go through goat-transformers

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | Multi: `USAREUR-AF/open-entity-gateway`, `USAREUR-AF/goat-transformers`, and the repositories that point at them |
| Amends | [ADR 0217](0217-shim-repo-containment.md) (one repository per commercial vendor) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0209](0209-shim-pattern.md) (the shim pattern), [ADR 0112](0112-tech-strategy-workload-shape-language-boundary.md) (language by workload shape) |

## TL;DR (for a layperson)

goat has to exchange tracks with two kinds of outside system. Systems that speak an open standard, such as a TAK server
(Cursor on Target), a SAPIENT controller or an ASTERIX consumer, are reached through goat-transformers, the library that
converts every message format. Systems built by a vendor, reached through that vendor's own software kit, are reached through
one shared repository of vendor bridges, where each vendor's bridge is its own package. open-entity-gateway becomes that
repository. A receiver's format is written once, and nothing adds a hop that neither converts nor delivers.

## Context

`open-entity-gateway` was created on 2026-11-15 without a decision record. It holds three things:

| Part | What it is | Where the same job already exists |
|---|---|---|
| A track model, mapping files and its own CoT, SAPIENT and ASTERIX CAT062 encoders | the shared model and conversion | goat-transformers: the shared model and its packs, which the gateway already compares itself against |
| Senders to TAK, SAPIENT and ASTERIX receivers, and a destination that republishes encoded payloads for an octo-pod to relay | transports to open-standard receivers | goat-transformers `adapters/`: `goat-xf-pipe` over UDP, multicast, TCP, MQTT and the fabric |
| Two vendor bridges, each a separate package whose vendor SDK the user installs | vendor bridges | nothing else |

ADR 0217 gives each commercial vendor its own repository. It was written before goat-transformers existed and before any
vendor contributed a bridge; an engineer of one of the two vendors has since contributed to that vendor's bridge in this repository.

## Decision

1. **Open standards go through goat-transformers.** The shared model, the conversion of every format, and the transports to
   open-standard receivers (`adapters/`) live in goat-transformers. A transport that goat-transformers lacks and a receiver
   needs is added there: a TLS output for TAK servers, a SAPIENT session output, and the TAK presence event on connect.
2. **Vendor systems go through one vendor-bridges repository.** `open-entity-gateway` is that repository. Each vendor's bridge
   is its own package, with its own SDK, licence and tests, so that removing one vendor touches no other. This amends ADR 0217's
   one-repository-per-vendor rule for bridges, because one place lowers the cost for each new vendor while the package boundary
   keeps the separation ADR 0217 wanted.
3. **A bridge reads goat's shared model.** When a bridge needs a data model, it takes the shared model from goat-transformers;
   the repository defines none of its own.
4. **One conversion, one delivery.** A receiver's format is written once: by the producer (open-billie encodes at the
   node; first-bite's transforms write CoT), or by goat-transformers converting tracks on the fabric (`goat-xf-pipe-zenoh`).
   octo-pod delivers a key unchanged to a receiver off the mesh. No component adds a hop that neither converts nor delivers,
   and no second conversion path is built beside goat-transformers.
5. **Python by exception.** Both vendor SDKs are Python, so the bridges are Python, as an exception to ADR 0112, which gives
   long-running data-path programs to Rust. A bridge for a vendor whose SDK is in another language uses that language.
6. **Nothing is removed before its replacement passes the same test.** The gateway's open-standard senders and encoders leave
   the repository only after the goat-transformers path passes the end-to-end tests they pass now (OpenTAKServer for CoT,
   Dstl Apex for SAPIENT, an octo-pod egress run).

## Consequences

- The gateway's `fabric` destination, a second conversion path beside goat-transformers, and its
  `docs/design/track-egress.md` are withdrawn. Documentation elsewhere that
  describes tracks leaving goat through the gateway is corrected.
- goat-transformers gains the TLS, SAPIENT-session and presence-event transports. Until it has them, the gateway's own senders
  stay and keep working.
- The repository may be renamed to say what it is for; GitHub redirects the old name and its forks.
- The repository is a registered component with a stated purpose: this ADR.

## Alternatives

- **Remove the repository.** Rejected: the vendor bridges have no other home, and a vendor already contributes to one.
- **Keep it as a general gateway for every system.** Rejected: it duplicates goat-transformers' model, encoders and
  transports, and a second conversion path drifts from the first.
- **One repository per vendor, as ADR 0217 has it.** Rejected for bridges: each new vendor would have to stand up a
  repository, CI and contribution rules before writing a line of bridge code.

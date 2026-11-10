# ADR 0700 — Chainguard-first for third-party container images

| Field | Value |
|---|---|
| Status | Accepted. Amended by [ADR 1220](1220-public-base-images-by-role-overlay-supplies-its-own.md): base images are named by role; the public repositories default to Chainguard's free catalogue, and a private Chainguard catalogue is named only in an environment's overlay |
| Date | 2026-04-25 |
| Repo scope | Multi |
| Related | [`0701-iron-bank-not-trust-anchor.md`](0701-iron-bank-not-trust-anchor.md), [`0702-build-from-source-mgmt-plane.md`](0702-build-from-source-mgmt-plane.md), `../design/os-supply-chain-security.md`, `../project/chainguard-procurement.md` |

## TL;DR (for a layperson)

For any third-party software we run in a container (Postgres,
CoreDNS, Caddy, etc.), our default source is Chainguard's
hardened-image catalog. Chainguard images are built daily from
upstream sources by Chainguard's pipeline, scanned for
vulnerabilities, and signed end-to-end. This is meaningfully
better than pulling from Docker Hub or Iron Bank.

## Context

Container image provenance is the largest supply-chain risk
surface in modern systems. The default ("pull from Docker
Hub") is the worst-case path: unsigned, unscanned by default,
unowned by anyone after the image is uploaded.

Chainguard offers an enterprise-grade alternative:

- Daily rebuilds from upstream sources.
- SBOM published per image.
- Signed images (cosign).
- Distroless / minimal base images reduce CVE surface.
- Hardened-by-default configuration.

Iron Bank is *not* a sufficient substitute — see
[`0701`](0701-iron-bank-not-trust-anchor.md).

## Decision

For every third-party container image, our preference order is:

1. **Chainguard** (cgr.dev/chainguard/<image>) — first choice
   when available.
2. **Build from source** (the upstream project's source +
   our pipeline) — when the use case justifies it
   ([`0702`](0702-build-from-source-mgmt-plane.md)).
3. **Iron Bank** — *only* after per-image audit and only when
   Chainguard has no equivalent. Iron Bank is not a
   standalone trust signal ([`0701`](0701-iron-bank-not-trust-anchor.md)).
4. **Distroless / minimal upstream image** (e.g. official
   `distroless/static`) — when none of the above applies.
5. **Generic upstream image** — last resort, with explicit
   ADR documenting why.

Pinning by digest ([`0703`](0703-pinned-digests-and-pin-regression-ci.md))
is mandatory in all cases.

The procurement-ready image list (in-catalog vs
request-to-add) is at
`../project/chainguard-procurement.md`.
The supply-chain design is at
`../design/os-supply-chain-security.md`.

## Alternatives considered

- **Docker Hub default.** Worst-case provenance. Rejected.
- **Iron Bank as primary.** Per-image quality varies; Iron
  Bank presence is not a trust anchor
  ([`0701`](0701-iron-bank-not-trust-anchor.md)).
  Rejected as primary.
- **Roll our own image factory.** Reinvents Chainguard.
  Rejected at our scale.

## Consequences

**Positive.** Daily-rebuilt images, signed end-to-end, SBOM
per image. Reduced CVE surface from minimal base images.
Per-image accountability (Chainguard publishes provenance).

**Negative.** Chainguard requires authenticated pulls for
most images (cgr.dev requires credentials — see Block 13A
CoreDNS migration). We provision authentication via
secrets manager. Cost of access is small.

Some images we need are not in Chainguard's catalog yet.
The procurement document tracks "request-to-add" status.

## Assessor view (SCA-R)

Chainguard-first serves SR-3 (supply chain controls), SR-4
(provenance), SI-7 (software, firmware, integrity), CM-7
(least functionality — minimal base images), and SA-12
(supply chain protection). The signed-image + SBOM + daily-
rebuild combination is a stronger SR-3/SR-4 posture than
median DoD network practice — see
`../compliance/comparative-posture.md`.

## Trigger conditions for revisit

- Chainguard governance / pricing changes adversely.
  Investigate alternatives (build-from-source ourselves at
  scale).
- A specific authorization mandates Iron Bank exclusively
  for a defined image class. Negotiate; do not silently
  accept Iron Bank as universal.

---

*See `GLOSSARY.md` for terms and acronyms.*

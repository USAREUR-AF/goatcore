# ADR 1079 — Post-quantum hybrid key exchange (X25519MLKEM768) on Zenoh TLS links

| Field | Value |
|---|---|
| Status | Proposed (prototype built + wire-proven 2026-07-12; adoption gated on the trigger conditions below) |
| Date | 2026-07-12 |
| Repo scope | goat core |
| Amended by | [ADR 1265](1265-fips-router-moves-to-the-aws-lc-fips-4-module.md) — the FIPS router module moves to AWS-LC FIPS 4.0.0 (CMVP #5596/#5595) |
| Related | [`0602-rosenpass-hybrid-rekey.md`](0602-rosenpass-hybrid-rekey.md), [`0601-fips-pqc-boundary-node.md`](0601-fips-pqc-boundary-node.md), [`0603-ml-kem-at-boundary.md`](0603-ml-kem-at-boundary.md), [`0858-zenoh-image-upstream-pivot.md`](0858-zenoh-image-upstream-pivot.md), F-247, [`../design/zenoh-pqc-tls.md`](../design/zenoh-pqc-tls.md) |

## TL;DR (for a layperson)

Our data fabric's TLS connections can be upgraded so the session keys are
agreed with a hybrid of classical and post-quantum cryptography
(X25519MLKEM768 — the same scheme Chrome and Cloudflare deploy). We built
and proved this with a tiny, containable change: one 14-line patch to the
router's startup, no fork of the zenoh libraries, and old and new software
interoperate automatically. We are NOT switching the fleet on yet, because
the same traffic already gets its post-quantum protection from the planned
WireGuard/Rosenpass layer underneath — this becomes valuable the day a
fabric connection runs *outside* that tunnel, or the day the FIPS-validated
post-quantum library certificate lands.

## Context

zenoh 1.9.0 pins rustls's ring provider (no ML-KEM). F-247: the provider is
a first-installer-wins process default, so PQ needs no library fork — a
patched `zenohd` binary (routers) or one startup call (our own client
binaries). The prototype (`ops/zenoh/pqc-image/`) is the upstream
digest-pinned image with `/zenohd` swapped; plugin ABI is guarded by a
fail-closed CI smoke test and zenohd's own startup check. Negotiation,
overhead, and impaired-link behavior are measured, not estimated
(M-081,
M-082).

All fleet Zenoh TLS links currently run inside the NetBird WireGuard mesh,
whose PQ hardening is the committed Rosenpass path (ADR 0602, Block 19).
For today's topology, PQ-in-Zenoh-TLS duplicates that protection. On FIPS:
CMVP **#5298/#5314 issued 2026-06-03/05** validating AWS-LC FIPS 3.1.0 with
ML-KEM in-boundary (security policy read directly; the vendor FIPS.md lags) —
so the FIPS-variant trigger below has already FIRED. No FIPS claim attaches
to *this* prototype (it builds non-FIPS aws-lc-rs); the claim needs the
validated-module build described in the design doc §5.

## Decision

1. **Keep the capability built and CI-guarded, not fleet-deployed.** The
   image (`ghcr.io/usareur-af/zenoh-pqc`), its 14-line patch, and the ABI smoke
   gate live in-tree and build on every touching PR, so the seam stays
   proven against the pinned zenoh version.
2. **Standing canary on the nova trial fabric router** once the first
   post-merge image digest exists: real PQ exposure where breakage costs
   nothing, exercised on every zenoh bump. *(Done 2026-07-12: nova-a runs the
   `sha256:e74217f3…` list digest; 20/20 PQ + 20/20 classical handshakes
   live-verified; resident fleet carried unchanged — design doc §7.)*
3. **Fleet adoption is deferred** until a trigger fires (below). Rosenpass
   (ADR 0602) remains the committed intra-mesh PQ path — better
   security-per-effort for traffic that never leaves the mesh.
4. **Vocabulary discipline:** this is "hybrid PQ key exchange on fabric
   TLS" — never "FIPS PQC." The certificate condition is now met upstream
   (#5298/#5314), but condition 2 of fips-pqc-path.md §1 is satisfied only
   by a router actually built on the validated module — which this
   prototype deliberately is not. *(Addendum 2026-07-12: that router now
   exists — `ghcr.io/usareur-af/zenoh-pqc-fips`, aws-lc-fips-sys =0.13.11 =
   exactly the validated AWS-LC FIPS 3.1.0 source, X25519MLKEM768 verified
   fips=true + negotiating under a fail-closed probe gate. The permitted
   vocabulary steps up to "built on the CMVP #5298-validated module"; a
   FIPS 140-3 operational claim still awaits the compliance-package
   language and a deploy decision — design doc §5.)*

## Alternatives considered

- **Adopt fleet-wide now.** Redundant with the committed Rosenpass path for
  all current links; spends canary/soak attention without a distinct threat
  addressed. Rejected for now (trigger-gated).
- **Wait for upstream zenoh provider selection.** No upstream signal;
  leaves the off-mesh-link scenario unprotected when it arrives. Rejected.
- **Rosenpass-style outer wrapper for off-mesh Zenoh links.** Heavier than
  flipping the TLS group, and off-mesh links are exactly where we can't
  assume a WG substrate. Rejected.
- **Fork zenoh libraries for provider config.** Strictly worse than the
  14-line binary-crate patch. Rejected.

## Consequences

**Positive.** PQ capability held at one-image-pin distance; mixed-fleet
safety proven; fork cost bounded (14 lines, loud failure on bump); the
FIPS-in-software path stays warm pending the certificate.

**Negative.** A second router image to keep green per zenoh bump (CI does
the proving); the canary router diverges from the fleet pin while deployed;
prefer-post-quantum adds ~2.3 KB per full handshake on PQ↔PQ pairs
(measured; amortizes to noise on long-lived fabric sessions).

**Addendum 2026-07-17.** The client chokepoint gained a
runtime kill-switch: `panoscope-core/src/tls.rs` resolves a pqc-mode signal
(`GOAT_PQC` env, then `/var/lib/goat/pqc-mode`, then `~/.config/goat/pqc-mode`)
at first TLS session build and installs classical-only kx groups when it says
`classical`; `goat client pqc on|off` / `goat admin pqc --peer|--fleet` flip
this file and rosenpass in one action. Per-node, reversible, no redeploy —
complements (does not replace) this ADR's image-pin fleet brake and its
human/strategic triggers. A degradation-**driven** automatic trigger (Tier 2)
is a new trigger class this ADR does not cover — ratify via amendment when
built. Strategy: docs/design/pqc-kill-switch.md.

## Assessor view (SCA-R)

Serves SC-12 (key establishment — hybrid classical+PQ consistent with CNSA
2.0 direction) and SC-8 as defense-in-depth.
No FIPS 140-3 claim is made or implied (SC-13 unchanged) until a
CMVP certificate listing ML-KEM in-boundary is issued and the build pins
that validated module; at that point this ADR re-opens with the evidence
package.

## Trigger conditions for revisit

- **Any Zenoh TLS link scheduled to run outside the WG mesh** (federated
  peering ADR 1020 activation, public mTLS crutch tier enablement,
  relay-hosted fabric surface) → adopt PQ image for the routers terminating
  that link before it carries traffic.
- **AWS-LC-FIPS CMVP certificate with ML-KEM in-boundary** — **FIRED
  2026-06-03/05 (#5298/#5314, AWS-LC FIPS 3.1.0), discovered 2026-07-12** →
  the rustls `fips` build variant is now an actionable follow-on track
  (validated-module pin + glibc image shape + matched-pair plugins + the
  SP 800-56C rev2 hybrid-mode verification, design doc §5); the
  FIPS-in-software claim re-opens with fips-pqc-path.md discipline when
  that variant exists.
- **Rosenpass/Block 19 slips past its horizon or is descoped** → the
  fabric-layer PQ insurance argument strengthens; reconsider fleet adoption.
- **zenoh upstream ships provider selection or an aws-lc-rs default** →
  delete the patch; keep only the pin.

---

*See `GLOSSARY.md` for terms and acronyms.*

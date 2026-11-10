# ADR 1023 — REST→native-mTLS Zenoh publish bridge for non-Rust publishers

**Status:** Accepted
**Date:** 2026-06-03
**Repo scope:** goat core (`ops/zenoh-shims/rest-pub-bridge`, `airdefenders` role)
**Related:** [ADR 1020](1020-zenoh-federated-peering-trust-model.md) (ingress/egress ACL split) · [ADR 0844](0844-zenoh-deployment-topology-and-acl-v1.md) (ACL v1) · [ADR 1022](1022-zenoh-denial-observability.md) (denial observability) · [ADR 0112](0112-tech-strategy-workload-shape-language-boundary.md) (language-boundary) · design [`zenoh-rest-pub-bridge.md`](../design/zenoh-rest-pub-bridge.md) · `airdefenders-multiplayer.md` · `nova-x509-identity.md`

> **Update (2026-07) — the Go-binding premise changed; the decision stands.** Since this was accepted, an official cgo `zenoh-go` shipped in **Zenoh 1.9.0** (2026-04, SoftBank-sponsored). The "no maintained Go client for Zenoh 1.x" premise throughout the context below is now **historical**. This ADR is **not superseded**: the sidecar remains correct wherever a **CGO-free static image** or a **zero-code-change** publisher matters (airdefenders keeps it). Native `zenoh-go` reintroduces exactly the cost weighed under *Alternatives → "CGO-bind zenoh-c"* (a native library at build+runtime, no pure-static image), so it is now a real option for *new* Go publishers willing to pay it — evaluated for the data-access E1 key-service in grant-keyed-encryption.md §8 M2.1.

## TL;DR (for a layperson)

Our data fabric now refuses to accept a publish unless the publisher proves who it is with a certificate, presented during the encrypted handshake. Most of our publishers are written in Rust and do this natively. But the multiplayer game is written in Go, and there is no usable Go library for our fabric — so it has been publishing over a plain web request that carries no certificate. That worked while the fabric trusted anyone on the local network; once we tightened the rules, the game's data started getting silently dropped. Rewriting the game isn't worth it. Instead we add a tiny helper container next to the game (a "sidecar") that *does* hold the certificate and speaks the fabric natively. The game keeps making its simple web request — but now to the sidecar, which re-sends it with the proper identity. One small, reusable piece restores the game's data and gives us a clean pattern for any future non-Rust publisher.

## Context

The nova Zenoh fabric authenticates peers by client certificate at the TLS transport layer: a peer opens a native session, presents an X.509 leaf, and the router's `access_control` matches the leaf Common Name (`CN=<vendor>-trial-peer`) to a per-vendor write rule (`put` on `<vendor>/**`). Every first-party peer — synth-vendor, audit-subscriber, panoscope, the portal — works this way.

The airdefenders game server cannot. It is Go, and there is no maintained Go client for Zenoh 1.x (Eclipse's maintained 1.x bindings are zenoh-c / zenoh-pico / zenoh-python / zenoh-kotlin; the once-planned `zenoh-go v0.11.0` does not resolve on the Go proxy). The pre-existing fallback was to publish over the router's **REST plugin** — HTTP PUT to `:8000` — which has no client-cert requirement. The game was implicitly trusted by virtue of sharing the router's docker network.

[ADR 1020](1020-zenoh-federated-peering-trust-model.md)'s ingress/egress ACL split (landed #716/#763) removed the blanket ingress-write grant. Publish is now allowed **only** to identified per-vendor subjects. The game's identity-less REST PUTs match no subject and are dropped at ingress — silently ([ADR 1022](1022-zenoh-denial-observability.md) / F-177). The game's data stops appearing in panoscope.

The game already *has* a minted leaf (`CN=airdefenders-trial-peer`, from `bootstrap-airdefenders-portal.sh`) — it just has no transport on which to present it. Three ways to close the gap:

1. **mTLS sidecar** — a small native-Zenoh helper container holds the leaf and re-publishes the game's PUTs over mTLS.
2. **CGO-bind zenoh-c from Go** — gives the game a native session, but breaks the CGO-free Chainguard `static:latest` image the whole airdefenders build depends on.
3. **Rewrite the publisher** in Rust/Python — the eventual goat-net production target, but disproportionate for the trial fabric today.

A TLS-terminating reverse proxy in front of the REST plugin is *not* a fourth option: the REST plugin injects PUTs as the router's own local publications, so a client cert presented to it never reaches `access_control` as a peer subject. Only a real native session carries an ACL-matchable identity.

## Decision

**Adopt the mTLS sidecar (option 1) as the standard pattern for non-Rust Zenoh publishers, implemented as a generic, reusable `rest-pub-bridge`.**

The bridge mimics the Zenoh REST plugin's PUT contract exactly (`PUT /<keyexpr>`, body = value), so the publisher needs **zero code change** — it just repoints `ZENOH_REST_ENDPOINT` from the router to the sidecar. The bridge opens a native mTLS client session presenting the per-vendor leaf it mounts, and re-emits each sample. It is deployed as a sidecar in the publisher's compose project, on the shared router network.

Identity flows through the existing machinery unchanged: the leaf is minted by the same portal/bootstrap path, and the ACL grant comes from adding the vendor to `nova_seed_vendors` (rendering the subject + rule + storage). For airdefenders that is `CN=airdefenders-trial-peer` → `put airdefenders/**`.

A `--allow-prefix` guard restricts which keys the bridge will forward, so a compromised upstream cannot borrow the bridge's cert to publish outside the vendor namespace (defense in depth behind the router ACL).

This is the right fork because it (a) restores real wire identity rather than re-widening the ACL, (b) requires no change to the Go publisher, (c) keeps the Chainguard CGO-free image intact, and (d) generalizes — any future Go/non-Rust publisher reuses the same binary, closing the language-boundary gap [ADR 0112](0112-tech-strategy-workload-shape-language-boundary.md) anticipates rather than papering over it per-service.

## Alternatives considered

- **Re-add a blanket ingress-write ACL rule for the REST path.** Rejected — reopens exactly the over-grant ADR 1020 closed (any network-local actor publishes anywhere). The regression, not a fix.
- **CGO-bind zenoh-c from Go.** Rejected — breaks the CGO-free Chainguard static image; reintroduces the glibc-dynamic build-variant complexity the airdefenders build deliberately retired.
- **Rewrite the publisher in Rust/Python.** Deferred, not rejected — it is the goat-net production target. Disproportionate for the trial fabric, and the bridge is reusable there too as a transition.
- **TLS reverse-proxy in front of the REST plugin.** Rejected — the REST plugin does not propagate client-cert identity into `access_control`; no ACL subject results. Does not actually authenticate.
- **Leave it on REST + accept silent drops.** Rejected — the data simply doesn't flow; this is the bug.

## Consequences

- One more container per non-Rust publisher (small, distroless, no embedded secrets). The sidecar is on the publish hot path; a sidecar outage stops that publisher's Zenoh output (but not the app itself — airdefenders' publish loop is non-blocking and drops on error).
- The publisher↔sidecar hop is plaintext HTTP over the docker network (same trust boundary the direct-REST path used); the sidecar↔router hop is now mTLS (an improvement). The `--allow-prefix` guard bounds blast radius if the publisher is compromised.
- Establishes a reusable substrate piece. Future non-Rust publishers add the sidecar + a `nova_seed_vendors` entry; no new bespoke code.

## Implementation status

- ✅ **`rest-pub-bridge` binary + image** — `ops/zenoh-shims/rest-pub-bridge/` (axum + native mTLS Zenoh client, copying the synth-vendor session idiom); CI at `.github/workflows/rest-pub-bridge-image.yml`.
- ✅ **ACL grant** — `airdefenders` added to `nova_seed_vendors` (renders subject `vendor-airdefenders` + rule `trial-peer-w-airdefenders` + storage).
- ✅ **Deployment wiring** — sidecar service in `airdefenders-docker-compose.yml.j2`; game repointed at the sidecar; role pre-flights the cert material and pins the sidecar image in `.env`.
- 🔲 **nova-a live-fire** — build + push the image, re-render the router with the new seed vendor, deploy the sidecar, and confirm the four airdefenders topics flow in panoscope during a live game. Gates this ADR's first production proof.

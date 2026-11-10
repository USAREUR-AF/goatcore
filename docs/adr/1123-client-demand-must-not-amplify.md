# ADR 1123 — Client demand must not amplify into fabric load

- **Status:** Accepted
- **Date:** 2026-08-11
- **Amended:** 2026-09-11 — the entrypoint limiter is now two tiers: a
  3000 r/s `goat-entrypoint-ceiling` at the entrypoint and the 50 r/s
  `goat-ratelimit` attached per browsing router, after the single 50 r/s
  entrypoint limiter measurably refused the mgmt host's own netbird client,
  which reaches signal through the docker gateway as one source address
  (732 × 429 in 5 min on zagros). Relayed peers enter on the `enrollment`
  entrypoint and were never limited. Design and
  measurement: surface-overload-protection.md §3.1.
- **Deciders:** platform
- **Related:** ADR 1097 (zenoh router health bands), ADR 1101 (lazy per-slot
  storage), F-268 (fabric wedge), ADR 1073/1074 (viewer scoping),
  surface-overload-protection.md

## Context

Every operator-facing web surface on a goatnet — goat-map, panoscope,
flashlight, stethoscope, microscope, the portal — sits behind Traefik on the
mesh, gated by oauth2-proxy. None of them had any request-rate ceiling. An
audit on 2026-08-11 found no HTTP-layer rate limiting anywhere on the fleet;
what rate limiting exists (wg-cp0 nftables, the ntp-admit sidecar,
airdefenders' per-player input) sits below or beside the web tier, not in
front of it.

The audit also found that the surfaces are not equivalent in what client
demand costs:

| Surface | Client request triggers a fabric query? |
|---|---|
| goat-map | No — every read is served from an in-memory store |
| **panoscope** | **Yes** — hygiene retrievability probe + stale-recall |
| flashlight, stethoscope, microscope, portal | No |
| trashgoat | No — one `get` at cold start only |

Panoscope is the sole surface where an HTTP request from a browser becomes a
`session.get()` against a router's Zenoh storage: the hygiene probe issues one
`get` per fan-out template (3s timeout each), and `/api/samples` +
`/api/samples/decoded` fall through to a stale-recall `get` whenever a topic's
in-RAM ring is empty. A topic that never publishes has a permanently-empty
ring, so its recall fires on *every* view.

Router wedge is not hypothetical on this fleet — F-268 is an observed
deadlock, and ADR 1097 exists because router health needed its own alerting
band. Storage-query pressure driven by a browser is therefore a credible path
into a failure that takes a site's data plane with it.

What bounds the blast radius today: these surfaces are mesh-only and
authenticated. There is no anonymous client. The realistic threat is
unintentional over-demand — a reconnect loop, a wall-mounted dashboard, a
script left running — not a hostile flood.

## Decision

**A client-facing surface must not be able to convert HTTP request volume into
unbounded work on the fabric.** Where a surface genuinely needs to query the
fabric to answer a request, that path carries an explicit concurrency ceiling,
and saturation degrades into the surface's existing "could not determine"
state rather than queueing.

Enforced in three layers, each bounding something the others do not:

1. **Traefik `goat-ratelimit`, attached at the `websecure` entrypoint.** One
   definition, applied to every router on :443 including surfaces not yet
   built. Bounds request rate per source IP. *(Amended 2026-09-11: the
   entrypoint now carries `goat-entrypoint-ceiling`; `goat-ratelimit` is per
   browsing router — see the amendment note above.)*
2. **Panoscope's fabric-probe budget.** A process-wide semaphore
   (`MAX_INFLIGHT_FABRIC_PROBES`, currently 4) across all three
   client-triggered `get` call sites. `try_acquire`, never a wait.
3. **goat-map's stream-connection ceiling.** `GOATMAP_MAX_STREAM_CLIENTS`
   (default 64) bounding SSE fan-out cost, alongside the pre-existing
   per-client queue depth of 256.

Saturation is always an honest, visible state — a 503 with `Retry-After`, an
`Unconfirmed` retrievability verdict, an unchanged empty ring — never a
silent truncation that reads to an operator as absence of data.

## Consequences

**Good**

- The invariant is stated once, so the next surface inherits a rule rather
  than rediscovering the hazard. Panoscope violated it because nothing said
  it was a rule.
- The entrypoint attachment means a new vhost is covered the day it exists,
  with no per-service step to forget.
- Panoscope's degradation reuses states the endpoints already emit, so no new
  failure mode reaches the frontend.

**Costs and risks**

- The rate-limit numbers (50 r/s average, 100 burst) are set from the shape of
  operator use, **not** from measured load — no load measurement was taken.
  They are a ceiling against runaway demand, not a tuned quota, and should be
  revisited against real traffic.
- `--entrypoints.websecure.http.middlewares` is fail-closed: a router whose
  middleware cannot be resolved fails. The flag is therefore attached only in
  the ACME overlay, whose own playbook renders the definition first. It is
  deliberately absent from the base mgmt-host compose, which reaches hosts
  that never run that playbook.
- The middleware is defined twice (ACME path and traefik-role path) because
  the two config paths cannot reach each other. Divergence is a real risk;
  see the design doc §4.
- A busy site could see hygiene verdicts report `Unconfirmed` more often. That
  is the intended trade — an unconfirmed diagnostic beats a wedged router —
  but it does make the probe budget observable as reduced certainty.

## Alternatives considered

- **Per-router middleware attachment.** Rejected as the primary mechanism: it
  requires remembering sixteen files today and one more per future surface.
  Kept available for the traefik-role path, which does not centrally own its
  Traefik command set.
- **Queue the fabric probes instead of refusing.** Rejected: it converts
  fabric pressure into held HTTP connections and request latency, which is the
  same wedge one layer up.
- **Cache stale-recall results.** Complementary rather than alternative, and
  deferred: a cache changes freshness semantics on a diagnostic whose value is
  its liveness, and the budget is the part that bounds the router.
- **Remove the on-demand recall from panoscope entirely.** Rejected: it exists
  so a quiet topic's last message stays inspectable across restarts, which is
  a real operator need. Bound it, don't delete it.

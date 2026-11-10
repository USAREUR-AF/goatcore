# portal-static-delivery — how the portal frontend reaches a browser

Browser credential handling follows the [browser session authentication contract](browser-session-authentication.md), including the HTTPS deployment requirement.


| | |
|---|---|
| Scope | Delivery of `goat-zenoh-portal`'s embedded frontend and its JSON API to a browser: content encoding, cache revalidation, and the per-page request chain. Not the frontend's content or the API's semantics. |
| Related | [`portal-lexicon.md`](portal-lexicon.md) (the substitution that shares this path), [`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md) (the cross-surface links that land on these pages), [`minimalism-under-network-constraints.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/minimalism-under-network-constraints.md) |
| Code | `ops/goat-zenoh-portal/internal/api/static.go`, `internal/api/gzip.go`, `web/static/app.js` |
| Status | Built 2026-08-12, live on zagros (preprod) since 2026-08-13 as `sha-41a6895`. Measured live, before and after, on that goatnet. |

## Why this has a doc

The portal serves small static pages, so it was assumed to be fast, and the
server-side numbers agreed: on zagros every endpoint answered in ~200 ms. What
that missed is that a page load is not one request. Deep-linking into
`portal/schema.html?key=<topic>` from panoscope is a cold navigation, and a cold
navigation was fetching ~215 KB of uncompressed, uncacheable assets and then
running three API calls one after another before anything rendered.

None of that shows up in a per-endpoint latency check. It shows up as "the page
takes a few seconds", and it scales with how slow the mesh path is — which is
exactly the condition the edge surfaces are supposed to tolerate.

## The three defects

| | Symptom | Cause |
|---|---|---|
| No compression | `content-length: 124980` on `highlight.min.js` with `Accept-Encoding: gzip, br` on the request | Nothing in the chain negotiated an encoding |
| No cache validators | No `ETag`, no `Last-Modified`, no `Cache-Control` on any asset | Served by a stock `http.FileServer` over an `embed.FS`. embed reports a **zero modtime**, so `ServeContent` omits `Last-Modified`, and `FileServer` sets no `ETag` or `Cache-Control` of its own. A client had nothing to revalidate against, so every navigation refetched everything. |
| Serialized init | 3 blocking round trips before first paint | `hydrateAuth()` awaited `/api/orgs`, then awaited the lexicon, then the page awaited its own data |

The third had a specific cost: `/api/orgs` is ~63 KB and was fetched **on every
page** — `schema.js`, `vendor.js`, `progress.js`, `data-access.js` — purely to
render "N orgs visible" in the header.

## What it does now

**Static assets are precomputed at construction.** `buildStaticAssets` walks the
embed once and stores, per file: the exact bytes to serve (display-lexicon
already substituted for HTML), a strong ETag over those bytes, and a gzip
encoding where the type is compressible and the file clears a 512-byte floor.
The asset set cannot change without a new image, so there is nothing to
invalidate and no per-request compression cost.

**`Cache-Control: no-cache`, not a max-age.** The assets are unversioned
filenames — `/app.js` is `/app.js` across image bumps — so any max-age would
serve stale JS after a deploy. `no-cache` means *revalidate before use*, not *do
not store*: the client keeps the bytes and spends one conditional request, all
of them multiplexed on the same HTTP/2 connection, and a 304 carries no body.
Repeat navigation drops from ~215 KB to approximately nothing without ever
risking a stale frontend.

**Identity and gzip carry different ETags.** They are different representations
(RFC 9110 §8.8.1); `Vary: Accept-Encoding` is set so a shared cache cannot hand
a gzip body to a client that did not ask for one.

**API responses compress in middleware.** `gzipMiddleware` covers the generated
JSON, which the static table cannot precompute. It holds the first 512 bytes
back so a short error response is not inflated by a gzip frame, passes
`text/event-stream` straight through (`/api/topics/stream` is real SSE —
buffering it would turn a live tail into a hang), forwards `Flush`, and skips
anything that already carries a `Content-Encoding`, which is how the static
handler's precomputed gzip avoids being encoded twice.

**The header count comes from `/api/whoami`.** It now carries `visible_orgs`
(backed by `registry.CountVendors`, a `SELECT COUNT(*)`), so `hydrateAuth`
fetches ~60 bytes instead of ~63 KB. `index.html` is the one surface that
actually renders the org table, so it fetches the listing itself. `hydrateAuth`
also runs its identity probe and the lexicon fetch concurrently rather than in
sequence.

**Heavy third-party scripts are deferred.** `schema.html` loaded 125 KB of
highlight.js plus six language packs parser-blocking; `api-docs.html` loaded
1.7 MB of swagger-ui the same way. Both are `defer` now. Deferred classic
scripts still execute in document order ahead of module scripts, so
`window.hljs` and `SwaggerUIBundle` are in place when their consumers run.

## Measured

**On zagros (preprod), live**, with the before-numbers taken against the running
deploy immediately before the pin moved — so both sides are the same goatnet
over the same mesh path, not a bench estimate:

| Page | Before | After (gzip) | Saving |
|---|---|---|---|
| `schema.html` + subresources | 215,529 B | 74,098 B | 66% |
| `api-docs.html` + subresources | 1,845,600 B | 501,284 B | 73% |

Repeat navigation of the schema page: all 13 assets return `304`, **0 bytes**.

`/api/orgs` compresses 63,227 → 6,159 B, and is no longer fetched at all on any
page that does not render the org table.

The bench numbers this replaced (216,315 / 1,845,536 B) differed by well under
0.1% — the small delta is the display lexicon, which substitutes per-goatnet
words into the HTML at serve time.

Round trips before first paint drop from 3 to 2. It is not 1: the schema render
path reads the display lexicon (`LEX`) when labelling example badges, so
rendering genuinely waits on the lexicon. Collapsing the last hop means
splitting the fetch from the render, which was not worth the regression risk for
~200 ms — recorded here rather than done.

## What verifying this turned up: SSE was never streaming

Probing the live deploy found `/api/topics/stream` answering `200` with an empty
body — no event, no heartbeat, not even response headers, on a 20-second
subscribe past the endpoint's own 15-second heartbeat.

`statusWriter`, the access-log wrapper every request passes through, embeds the
`http.ResponseWriter` **interface**. Embedding an interface promotes exactly its
method set — `Header`, `Write`, `WriteHeader` — so `Flush` was never promoted and
`w.(http.Flusher)` was false however capable the real writer underneath was.
`StreamSSE` takes that assertion at its top and returns an error that
`handleStream` swallows, on the reasoning that it cannot switch to a JSON error
once SSE headers are out — except they never were.

This predates the delivery work and is independent of it. What the delivery work
changed is the failure mode: `gzipResponseWriter` **does** implement `Flusher`,
so `StreamSSE` began getting past the assertion and hanging until the client
timed out, rather than returning instantly. Worse to sit in front of, same
absence of streaming underneath. Fixed by forwarding `Flush` from
`statusWriter`; see F-325.

Verified live on zagros 2026-08-13 once the fix shipped: the same 20-second
subscribe that had returned 0 bytes on every prior image returned headers
immediately (`content-type: text/event-stream`, `x-accel-buffering: no`) and
delivered a `: heartbeat` event. The stream carries no `Content-Encoding` — the
middleware passes `text/event-stream` through as identity, as intended.

The reusable part is the shape, not the instance: **wrapping a
`http.ResponseWriter` by embedding the interface silently drops every optional
interface the real writer implements** — `Flusher`, `Hijacker`, `ReaderFrom`,
`Pusher`. Nothing fails to compile, nothing errors at runtime, and the loss has
no symptom until something streams. Any wrapper on a path carrying SSE,
websockets, or large file sends needs those forwarded explicitly.

## Caveats

- **The original "several seconds" report was not reproduced.** Measured from
  the operator laptop against zagros, a cold `schema.html` load was ~1.5 s, not
  several. The defects above are real and were fixed on their own merits, and
  they are the mechanism by which a slower path becomes several seconds — but
  the specific slow observation (which goatnet, from which machine) was never
  pinned down, so there may be a second cause still out there.
- **Memory cost.** The precomputed table holds the identity bytes plus the gzip
  bytes: ~2.5 MB + ~0.7 MB of heap, and a one-time startup cost to compress.
  Negligible for this service, but it is not free, and it scales with whatever
  gets added to `web/static/`.
- **BREACH.** Compressing authenticated responses that reflect attacker-supplied
  input is the precondition for BREACH-style attacks. The portal is mesh-only
  behind TLS with no cross-origin form posts, so this is noted rather than
  mitigated; a public-facing deployment of the same code should revisit it.

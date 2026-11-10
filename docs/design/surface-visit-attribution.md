# Surface visit attribution — who opened which page, how often

| Field | Value |
|---|---|
| Status | **Doors live 2026-09-05. Visit emitter built and tested, not deployed.** |
| Anchor ADR | [ADR 1163](../adr/1163-per-visit-attribution-for-fenced-surfaces.md) |
| Related | rbac-adherence (ADR 1073 — the door taxonomy), microscope (ADR 1090 — the reader), [operator-action-audit](operator-action-audit.md) (the envelope this copies), [audit-cold-storage-offload](audit-cold-storage-offload.md) (the retention this inherits), F-457 |

## What this is

Three web surfaces on each goatnet — a multiplayer demo game and two briefing
decks — were reachable by anyone on the mesh with no login, and nothing recorded
who opened them. They are now behind doors that still admit everyone but
authenticate first, so each visit is attributable. On top of that sits a
per-request record, so the question "how many times, and when" has an answer.

**Nobody lost access.** The doors carry an empty allowed-groups list: any
identity the site Hydra will issue a token for passes, and no group is refused.

## Why a door was the only way

A surface with no door never touches Hydra. It produces no consent grant and no
session, so it is invisible to the identity machinery regardless of how much of
that machinery is built — and a good deal is: microscope composes eight sources
into a per-entity timeline, and one of them reads exactly the consent grants a
door produces.

So the door is not the access control on these three. It is the instrument. The
access decision is "everyone", and that is expressed by the empty group list
rather than by having no door at all.

The cost to a visitor is one silent redirect per surface, not a login prompt.
Kratos holds a 24-hour SSO session, so the second surface someone opens
re-authenticates without showing them anything. Cookies stay locked to each
surface's own FQDN so a stolen one cannot be replayed against a sibling console.

## Two sources, and why neither is enough alone

**The consent grant** (Hydra, read by microscope's `hydra-consents` adapter) is
a standing record: one row per person per surface, with `handled_at` moving to
the last time it was exercised. It answers *has this person reached brief, and
when last*. Repeat visits collapse into that one row.

**The visit record** (`SurfaceVisitV1`, read by the `surface-visits` adapter) is
one row per authenticated request. Counts and a history fall out of it.

| | consent grant | visit record |
|---|---|---|
| Survives the emitter being down | yes | no |
| Survives the grant being revoked | **no — vanishes from the API** | yes |
| Answers "how many times" | no | yes |

Read together they separate *never came* from *came, and the grant was later
revoked*. Apart, neither can, because each has a failure mode that looks exactly
like absence. This is the F-177 silent-denial lesson applied to attribution, and
it is why the visit stream was added beside the grant rather than instead of it.

## Where the record comes from

Every field comes from the door's own request log — the only place that holds
both the identity oauth2-proxy validated and the request line.

The application cannot be the source: two of the three surfaces are static file
servers, so there is no code to instrument, and adding some would mean shipping
an application to serve a deck. The Traefik access log could carry the identity
in one config line, but that log deliberately excludes client identity — its
config says so, and notes User-Agent is withheld for the same reason — and
reversing that decision should not ride along inside a feature.

The log format is pinned to explicit JSON rather than parsed from oauth2-proxy's
positional default, which is ambiguous the moment a field is empty (an
unauthenticated request renders `- -`, and the user field is findable only by
counting separators) and which upstream may reorder without anything failing
loudly. A field that disappears now becomes a missing key.

## The path a visit takes

```
browser ─► Traefik ─► oauth2-proxy door ─► the surface
                          │
                          │ request log (JSON, identity included)
                          ▼
                  goat-surface-visit
                          │ age-encrypt to the operator roster
                          ▼
              ops/audit/surface/<surface>/<actor>   (Zenoh)
                          │
                          ▼
                  audit-subscriber ─► MinIO audit-integrity (30d) ─► cold storage
                          │
                          ▼
              microscope `surface-visits` adapter
```

Nothing in the storage or retention half was built for this. The
audit-subscriber already persists `ops/audit/**`, and microscope already reads
that tier for two other sources.

## What is deliberately not recorded

- **Unauthenticated requests.** oauth2-proxy renders those with no user, which
  is every prober, every scrape and every api-route 401. Traffic that never
  involved a person must not appear as a person visiting a page.
- **Query strings.** A query can carry the content of a search, which is a
  different and more sensitive thing than the fact of a visit.
- **Anything in plaintext.** Each record is age-encrypted to the operator roster
  before publish — the same envelope `OperatorActionV1` uses — so a mis-set read
  ACL leaks opaque ciphertext rather than a browsing trail. The emitter holds
  only public recipients and cannot read back what it writes. An empty roster
  file is a hard refusal, never a fallback.
- **An event time substituted by read time.** An unparseable timestamp yields no
  record. Silently dating a line "now" is the lie microscope's never-re-stamp
  rule exists to prevent.

Microscope does not decrypt. Counts and times come from the key path, which
carries surface and actor; only the per-request detail needs a roster key, and
the service holds none. Watching is itself audited, as every microscope look
already is — so a query about who visited what is visible to a later query about
the watcher.

## The capture gate

Microscope's design doc says it is "not surveillance infrastructure beyond what
already exists" and that new capture is "explicitly out of v1 and gated on its
own justification". Per-visit recording is new capture. The gate was opened
deliberately, by the operator, with the limits above as the conditions — see
ADR 1163. The doc did not forbid this; it required someone to own it.

## Operating it

**One role for all doors.** `ops/ansible/roles/oauth2-proxy-surface/` is
parameterised by surface name, hostname and upstream, and carries the
accumulated knobs once: F-342 (a polling page mints a CSRF cookie per poll until
the browser is locked out with `400 Request Header Or Cookie Too Large`), F-310
(the session dies ~86 s in without a minimal cookie), F-126 (the sidecar's
resolver cannot reach mesh CoreDNS), F-244 (only the `X-Forwarded-*` family is
unforgeable), F-457 (the cookie secret must be 32 raw characters).

The eight older per-surface sidecars are **not** migrated to it. Rewriting eight
working fences to prove a refactor is how a fence stops working; this role earns
that migration on the three surfaces it was built for first.

**Route classes are per surface.** The game serves a websocket, an SSE
stream and a polled leaderboard, so `^/api/` is declared as API routes: a
dropped connection then 401s like an XHR instead of starting an OIDC flow.
`/health` and `/metrics` skip auth, because a prober has no cookie jar and no
way to log in. The two decks are static and declare neither.

**The Hydra client must exist before the fence is live**, and nothing detects
the failure if it does not: the container is healthy, every probe is green, and
each real page load ends at `invalid_client`. The role therefore reads the registration back from Hydra and
asserts the redirect URI and scope match what the proxy will send.

**A door and its route deploy in order.** A door that is running but not
routed to changes nothing; a route pointed at a door that is not running takes
the surface down. `ops/ansible/authenticated-doors.yml` puts the doors up and
verifies each registration; `ops/ansible/authenticated-doors-routes.yml` then
swings the routes, and refuses a surface whose door is switched on but not
running.

**Rollback is one variable and no state.** Set
`<surface>_oauth2_proxy_enabled: false` and re-run that surface's playbook: its
Traefik route addresses the container directly again and the surface is
mesh-only. Nothing has been written that needs undoing.

## As-built

| Piece | State |
|---|---|
| `oauth2-proxy-surface` role | Built, merged, live |
| Doors on the game / brief / dataflows | Live |
| Router grant for `ops/audit/surface/**` | In the seed |
| `goat-surface-visit` emitter | Built, merged, live (`surface_visit_enabled: true`) |
| microscope `surface-visits` adapter | Built, merged, deployed |

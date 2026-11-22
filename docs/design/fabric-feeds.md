# Fabric feeds — self-service, approved delivery of fabric topics to mesh systems

**Status: WIP** — decision [ADR 1259](../adr/1259-fabric-feeds-self-service-approved-delivery-of-fabric-topics-to-mesh-systems.md).
The first build is `goat-feeds` in goatstuff (`ops/serve/goat-feeds/`, role
`ops/ansible/roles/goat-feeds/`), on trial. §9 says which parts are built and which
are designed. Expect the shape to change as people use it; change this doc with it.

## 1. What it does

A person signs in, picks a topic on the fabric, picks the mesh system that should
receive it, sets the port, transport and output format, and submits. A tactical admin
for the site, or a platform admin, switches it on, either directly as the requester or
by approving someone else's request. From then on the site's delivery service reads the
topic, converts each message and sends it. Everyone can see each feed's state, its
counters, its last error and its history.

## 2. Roles

| Who | Request | On directly | Approve or decline others' | Pause, resume, remove |
|---|---|---|---|---|
| Signed-in member | yes | synthetic topics only, when the site allows self-service | no | their own feeds |
| Tactical admin (sign-in group named in the site's config) | yes | yes, at this site | yes, at this site | any feed at this site |
| Platform admin (`PLATFORM-ADMIN`) | yes | yes | yes | any feed |

A delivery service serves one site, so a tactical admin group in that site's config
is a tactical admin for that site. Identity comes from the sign-in fence (oauth2-proxy
in reverse-proxy mode) as `X-Forwarded-Email` and `X-Forwarded-Groups`; the service
listens on loopback only, so nothing but the fence can set those headers.

## 3. A feed

| Field | Meaning |
|---|---|
| topic | A catalog entry: the Zenoh key expression, its source format, its data class (real or synthetic), raw or processed |
| destination | A mesh peer: name and mesh IP as resolved at request time; the peer's identity when a peer source is configured |
| port, transport | UDP, or TCP connect |
| output | An output form the catalog offers for that topic: target format, framing, fixed field values (for example ASTERIX `cat062.sac=38`, `cat062.sic=204`) |
| purpose, owner | Why, and the requester's sign-in email |
| end date | The feed stops on this date unless renewed |
| status | `pending`, `active`, `paused`, `declined`, `expired`, `removed` |

States: `pending → active | declined`; `active ⇄ paused`; `active | paused → removed`;
`active → expired` on its end date; `expired → active` on renewal by an admin. One
destination port takes one active feed.

## 4. The catalog

What can be requested is declared per site, in the site's group_vars: each topic's key,
label, kind (raw sensor or processed), data class, source format and the output forms
offered for it. The catalog is the place where a platform admin decides what is
offerable; adding a topic is one entry. Output forms are goat-transformers conversions
(the `--to-format`, `--out-frame` and `--set` of `goat-xf-pipe-zenoh`), so any format the
installed packs can produce can be offered.

## 5. Delivery

One supervisor in the service owns delivery for the site:

1. Every few seconds it reads the active feeds and groups them by topic and output form.
2. Each group runs one `goat-xf-pipe-zenoh` process: one subscription, one conversion,
   one `--out` per feed in the group. A thousand feeds of one topic and form are one
   subscription and one conversion.
3. When a group's set of outputs changes, its process restarts with the new set; other
   groups are not touched. A process that exits is restarted with back-off.
4. Each process's output is kept (the last lines), and its counter lines
   (`in= converted= failed=`) feed the per-feed counters.

The pipe reads the fabric with the site's client configuration (the same file the
processors pipeline uses, `/etc/goat-xf/zenoh-client.json5`), so what it may read is
bounded by that identity's grants at the router.

## 6. Seeing what is wrong

| Question | Where it is answered |
|---|---|
| Is the feed running? | Feed page: state, process state and uptime |
| Is data arriving from the fabric? | Messages in, last message time |
| Is it converting? | Converted and failed counts; the last error line from the pipe |
| Is it leaving for the destination? | Sent count. UDP shows "sending"; it cannot show "received" |
| Can the core node reach the destination? | "Check destination": ping, and a TCP connect for TCP feeds |
| What exactly runs? | The pipe command line, shown on the feed page |
| Who did what? | The feed's timeline and the site audit log |
| Is the service healthy? | `/healthz`; `/metrics` (Prometheus): feeds by state, groups running, per-feed counters |

## 7. Screens

- **Request a feed**: four steps on one page: topic (raw or processed, with data class
  shown), destination (search mesh peers by name), delivery (port, transport, output
  form), review (purpose, end date, and what happens next: on now, or waits for whom).
- **Feeds**: every feed with its state and a one-line health summary; filter by mine,
  waiting for approval, active, all.
- **Approvals** (tactical and platform admins): the waiting requests with approve and
  decline, a note on each.
- **Feed page**: state, counters, last error, destination check, the command, the
  timeline, and the actions the viewer may take.
- **Audit**: every request, decision and change at the site.

## 8. API

| Route | Does |
|---|---|
| `GET /api/me` | The viewer's email, groups and role at this site |
| `GET /api/catalog` | Offerable topics and output forms |
| `GET /api/peers?q=` | Mesh peers matching a name (when a peer source is configured) |
| `GET /api/feeds`, `POST /api/feeds` | List; request a feed |
| `GET /api/feeds/<id>` | One feed with counters, process state, last lines and timeline |
| `POST /api/feeds/<id>/<action>` | `approve`, `decline`, `pause`, `resume`, `remove`, `renew`, `check` |
| `GET /api/audit` | The site's audit log |
| `GET /healthz`, `GET /metrics` | Health and Prometheus metrics |

## 9. Built and designed

| Part | State |
|---|---|
| Registry (SQLite on the site's core node), states, audit | Built (first build) |
| Roles from sign-in groups; tactical admin per site; real data needs an admin | Built (first build) |
| Delivery supervisor around `goat-xf-pipe-zenoh`, grouped by topic and form | Built (first build) |
| Feed page diagnostics, destination check, `/healthz`, `/metrics` | Built (first build) |
| Screens in plain JavaScript | Built (first build) |
| Requester's data-access grants checked against the topic's producer (ADRs 1074–1076) | Designed |
| Per-feed port-scoped warrant rows, so approval also opens the mesh rule (ADR 1129) | Designed. Until then, feeds reach destinations the site's existing rules allow |
| Expiry reminders, automatic switch-off on lost access, removed peer or offboarding | Designed (expiry on the end date is built) |
| Registry replicated per site for island mode (ADR 1165) | Designed |
| Fleet component (CI image, pin, ledger) | Designed |
| Moving git-declared feeds (processors pipeline outputs) into the registry | Designed |
| Scale test at the target feed count | Designed |

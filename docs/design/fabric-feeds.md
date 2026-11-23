# Fabric feeds — self-service, approved delivery of fabric topics to mesh systems

**Status: WIP** — decision [ADR 1259](../adr/1259-fabric-feeds-self-service-approved-delivery-of-fabric-topics-to-mesh-systems.md).
The first build is `goat-feeds` in goatstuff (`ops/serve/goat-feeds/`, Go, with the page in
plain JavaScript; roles `ops/ansible/roles/goat-feeds/` and `goat-feeds-oidc-client`), on trial. §9 says which parts are built and which
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

## 5A. Receiving a system's data (IN feeds)

A system on the mesh can also send its own data in: a counter-UAS effector's position, its
tracks, its status. A tactical admin names what it sends, picks its standard and a free
receive port on the core node; once on, the core node publishes every message on the fabric.

1. The site declares its receive side in group_vars: the core node's mesh address, a list of
   receive ports, the standards it accepts (each a goat-transformers input format), the key
   prefix it publishes under, and the output forms a received feed is offered in.
2. Each received feed runs its own `goat-xf-pipe-zenoh`:
   `--in udp://<core mesh IP>:<port> --from-format <standard> --set track.source_id=<feed>
   --to-format goat-processors-system-track --out zenoh:<key_prefix>/<feed>`.
3. While the received feed is on or paused, it is offered in the catalog as a topic
   (`in-<feed>`), so anyone can request it out as CoT, CAT062 or SAPIENT like any other topic.
4. One receive port takes one live received feed. Data from a system is real data: a member's
   request waits for a tactical or platform admin.
5. The receive ports must be reachable from the sending systems: the warrant port set
   `feed-intake` (UDP 7100–7109), granted from the gear classes to the core node.

The receive process accepts datagrams from any address the mesh lets through to that port; the
mesh rule, not the process, limits who can send.

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
| Is a received feed arriving and readable? | Received, Published and Not readable counts; health `listening` (nothing yet), `receiving`, `quiet`, or `broken` (nothing reads as the chosen standard) |
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
| Registry in Postgres (the site's `postgres-a`, its own `goat_feeds` database), states, audit | Built (first build) |
| Roles from sign-in groups; tactical admin per site; real data needs an admin | Built (first build) |
| Delivery supervisor around `goat-xf-pipe-zenoh`, grouped by topic and form | Built (first build) |
| Feed page diagnostics, destination check, `/healthz`, `/metrics` | Built (first build) |
| Screens in plain JavaScript | Built (first build) |
| Receiving a system's data (IN feeds), published on the fabric and offered in the catalog | Built (goatstuff #134) |
| Receive ports reachable from gear (warrant port set `feed-intake`) | Built in the port-set vocabulary; granted per goatnet by its rule intents |
| Requester's data-access grants checked against the topic's producer (ADRs 1074–1076) | Designed |
| Per-feed port-scoped warrant rows, so approval also opens the mesh rule (ADR 1129) | Designed. Until then, feeds reach destinations the site's existing rules allow |
| Expiry reminders, automatic switch-off on lost access, removed peer or offboarding | Designed (expiry on the end date is built) |
| Registry replicated per site for island mode (ADR 1165) | Designed |
| Fleet component (CI image, pin, ledger) | Designed |
| Moving git-declared feeds (processors pipeline outputs) into the registry | Designed |
| Every tactical link in the registry with one health record each, drift states, Links view, `goat feeds` CLI (ADR 1260, §10) | Designed |
| Scale test at the target feed count | Designed |

## 10. Every tactical link in one place (ADR 1260)

A site's tactical links are not all made in the form. Some are declared in deployment
files (processors pipeline encodes and raw relays, boundary-node feeds) and some are
started by hand during an exercise. [ADR 1260](../adr/1260-every-tactical-link-in-one-registry-with-one-health-record.md)
brings all of them into this registry and gives each one health record on the fabric, so
Fieldkit Feeds answers "what links exist here, is each moving data, what data, and who
owns it" for every link, not only the requested ones. Status: designed, not built.

**A link and its row.** A link is one process that carries data between the fabric and an
address outside it, in either direction. Its row has an id, a kind (*requested* or
*declared*), a direction, the fabric key, each destination or receive address, the
format, the data class, the tier (raw or processed), the owner, and for declared rows the
file and switch that control it.

**Declared rows.** Each role that starts links renders a manifest of them on the host, for
example `/etc/goat-links/declared.d/processors-pipeline-rou-real.json`, one entry per
link. `goat-feeds` reads every manifest at start and on change and keeps the rows
read-only. A role change that removes a link removes its row.

**Health record.** Every link publishes on `ops/links/health/<site>/<link id>` every
10 seconds, as the environment health envelope (`EnvHealthV1`, ADR 1036):

| Field | Meaning |
|---|---|
| `state` | `sending`, `receiving`, `listening`, `quiet`, `broken` or `stopped`, with the same meanings as §6 |
| `in`, `converted`, `failed` | Counts since start, as in the pipe's status line |
| `sent` | Count per destination |
| `last_message_at`, `last_error` | Time of the last message; last error line |
| `summary` | Messages per second over the last minute, distinct track identities, source identifiers seen (ASTERIX SAC/SIC, MAVLink system id), data class |

`goat-xf-pipe-zenoh` publishes it with a new `--health-key` option. The registry is the
site's only subscriber to `ops/links/health/<site>/**`; at 10-second intervals a site
with 500 links adds 50 small messages per second.

**Drift.** The registry joins rows and records:

| Condition | Shown as | Typical cause |
|---|---|---|
| A health record with no row | `undeclared` | A link started by hand; its declaration has not merged |
| A row with no health record for 60 s | `silent` | Declared but not running, or the host is down |
| The destination check fails | `unreachable` | Mesh rule missing, or the destination is off |

**Links view.** One table per site: every row with kind, direction, owner, health and the
data summary; filters by owner, destination and state; a destination search answers
"what does this console receive?". Requested rows keep their actions. Declared rows show
their controlling switch, for example a destination's `enabled` switch in the pipeline's
group_vars file. CLI: `goat feeds ls [--dest <ip>] [--state <s>]` and `goat feeds show <id>`.

**Site roll-up.** `ops/links/health/<site>` counts links by state and is rendered by
`goat health`, the panoscope environment band and stethoscope, as a fourth domain beside
fabric, time and mesh links.

**Build order.**

| Step | Repo | Delivers |
|---|---|---|
| 1 | goat-transformers | `goat-xf-pipe-zenoh --health-key`: the record above, from the counters it already keeps |
| 2 | goatstuff, overlays | `goat-feeds` OUT and IN processes and the processors-pipeline units pass `--health-key`; the processors-pipeline and boundary-node roles render declared-link manifests |
| 3 | goatstuff | `goat-feeds` imports manifests, subscribes to the site's health records, shows the Links view and the drift states |
| 4 | goatstuff, goatcore | the site roll-up; `goat health` and panoscope render the domain |
| 5 | goatcore | `goat feeds ls` and `goat feeds show` |


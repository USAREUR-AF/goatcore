# External service golden path

| Field | Value |
|---|---|
| Status | Built; goat-map migrated onto it on every goatnet (render-identical, unmeasured live); first temporary consumer declared |
| Canon | [ADR 1178](../adr/1178-external-service-golden-path.md) |
| Related | mesh-and-ory-roles.md (the `EXTERNAL-INFRA` tier), machine-enrollment.md (how the host joins the mesh), rbac-adherence.md (the `door:` vocabulary), [name-fallback-resolver.md](name-fallback-resolver.md) (why the zone is a committed, signed file), goat-map.md (the first declaration); runnable walk-through with a simulated issuer and the relying-party rules: `hello-galaxy-external-service` |

## What this is

A goatnet lends its identity plane to a service it does not run. The service's owner writes one file and runs one command; the goatnet then signs its users in at the site Hydra, routes a name on the mgmt-host edge to the service's mesh address, resolves that name on the mesh and lists it on the status page. The same file, marked withdrawn, and the same command take all of it away. No core engineer is involved in either direction, and nothing in goat is named for the service.

The path exists because every earlier external service cost five hand edits in five places and had no removal. goat-map paid them once and is permanent, so that was acceptable. A service that lives for one event cannot pay them, and the same five places would have to be unpicked on the way out. [ADR 1178](../adr/1178-external-service-golden-path.md) records the decision; this page is the how.

## Step zero: the host is a mesh peer

The service runs on its own host, which is already a mesh peer when its owner writes the declaration — the declaration names that peer's address. For somebody else's hardware serving our users the tier is `EXTERNAL-INFRA`: every role group reaches it one-way on any port, and it initiates only toward `INFRA` (mesh-and-ory-roles.md §tiers). **No node class mints that tier** (F-531), so the host is moved there: `ops/scripts/external-service-finish.py` confirms the peer with the platform engineer, moves it, and records the move in `ops/ci/membership-declared-deviations.yaml`. The play's pre-flight asserts the group on the host and names what it found when it fails; fix the host's membership, never the participants'.

The service binds its mesh IP, never `0.0.0.0`. Mesh membership is the outer gate.

## The declaration

One file per service per goatnet, `ops/ansible/group_vars/<site>_hosts/external-service-<name>.yml`, holding one mapping `external_service_<name>` (hyphens become underscores). Ansible loads every file in the directory, so adding a service is adding a file.

```yaml
# ops/ansible/group_vars/zagros_hosts/external-service-example-registry.yml
external_service_example_registry:
  name: registry                        # -> registry.zagros.<zone>; one lowercase DNS label
  backend_url: "http://100.127.0.10:8081"   # the host's MESH IP + the port its fence binds
  health_path: /healthz                 # open, no auth; the pre-flight and the status page probe it
  door: authenticated                   # what the SERVICE'S OWN fence decides (closed set, below)
  client:                               # optional: goat registers this OAuth2 client at the site Hydra
    id: oauth2-proxy-example-registry
    secret_env: REGISTRY_OAUTH2_CLIENT_SECRET   # the SAME variable configures the service's fence
    scopes: [openid, profile, email, groups]    # add offline_access only if the fence refreshes
    redirect_path: /oauth2/callback
    token_lifespan: 8h                  # pin to the fence's cookie lifetime when it holds no refresh token
  status:                               # optional: the status-page row; defaults derive from the fields above
    name: "Example registry"
    desc: "System of record for the example programme's validation lanes."
    access: public                      # what the page shows: any authenticated identity gets past the door
    category: exercise
    icon: clipboard
  owner: example-office
  retire_after: "2026-10-15"            # a TEMPORARY service: the footprint guard is strict, the play refuses present after this date
  enabled: true                         # false = withdrawn; the play then accepts state=absent
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | The hostname label. The service is `https://<name>.<site_id>.<site_dns_zone>`. The edge SAN and route use `public_mgmt_fqdn`, like every other name on the mesh-only cert anchor; the play refuses a group where that differs from `site_dns_zone` |
| `backend_url` | yes | `http(s)://<mesh ip>:<port>`. The play refuses a non-mesh address |
| `health_path` | no, default `/` | Answers 200 without a session. The pre-flight, the status page and any blackbox probe read it |
| `door` | yes | One of `group-gated`, `authenticated`, `in-app`, `mesh-only` ([ADR 1073](../adr/1073-rbac-adherence-principle.md)). Recorded so a reader knows what the service's own fence decides; goat enforces nothing here |
| `client` | no | When present, goat upserts the Hydra client with these scopes and redirect, secret from `$secret_env`, lifespans pinned when `token_lifespan` is set. Absent means the service registers its own client, or has none |
| `status` | no | Status-page row fields (`id`, `name`, `desc`, `kind`, `access`, `category`, `icon`, `link_path`, `probe_front_door`, `expect_codes`), plus the listing controls in "On the status page" below: `list` (default `true`), `lifecycle` (`live` / `planned` / `retiring`) and `hostnames:` for per-hostname overrides. A row in `isitup_services` with the same hostname or id wins over the derived one |
| `hostnames` | no, default `[name]` | Several names from ONE declaration: a partner platform whose surfaces are one service with one withdrawal. Each entry is the service's own: `<name>`, `<name>-<surface>`, or nested one level as `<surface>.<name>`. Each gets a record, a status row, and (in `edge` mode) a place on the router rule. The certificate covers flat names with `*.<site>.<zone>` and a nesting service with one `*.<name>.<site>.<zone>` — see below. No wildcard DNS |
| `tls` | no, default `edge` | Who terminates. `edge`: goat's Traefik does, from `backend_url` over the mesh — the mesh-only allowlist is the gate and the partner holds no key for this zone. `partner`: the names resolve to `mesh_ip` and the partner terminates with their OWN certificate; goat renders no router, no backend and no SAN. That certificate still has to be trusted for a name in OUR zone, so `tls: partner` REQUIRES `acme_delegate` (below) — there is no lane for a partner to serve an untrusted or self-signed certificate on a declared name |
| `mesh_ip` | only when `tls: partner` | The partner host's 100.64/10 address. Every hostname in the declaration resolves here |
| `acme_delegate` | **required when `tls: partner`**; refused otherwise | A hostname in a zone the PARTNER controls, outside the site zone. goat publishes `_acme-challenge.<label>.<site>.<zone>` CNAME to it for each certificate name, so the partner gets a publicly trusted certificate with ACME DNS-01 and serves nothing publicly — see "How a partner gets a trusted certificate". The play refuses `tls: partner` without it: a partner certificate for a name in our zone is only trusted through this delegation, there is no own-certificate lane that skips it |
| `mesh_peer` | no | The NetBird peer name of the service host. When set, the play and the finish script refuse unless the peer at the declared address has this name, so a re-enrolled host with a new address is caught instead of named at the wrong IP |
| `traefik_name` | no, default `ext-<name>` | The Traefik router and service key. goat-map keeps `goat-map` so nothing keyed on it moved |
| `owner` | yes | Who to call. Free text; the play refuses a declaration without one |
| `retire_after` | no | `YYYY-MM-DD`. Marks a temporary service: the CI guard becomes strict and `state=present` is refused after the date |
| `guard_tokens` | no, and **set it explicitly on a multi-endpoint declaration** | Strings the footprint guard searches for. Defaults to the file stem (`example-registry`), its underscore form and `external_service_<stem>`. The `name` label is deliberately not a default token: a short label such as `registry` or `map` matches ordinary prose. Add it here if it is distinctive |
| `enabled` | no, default `true` | `false` keeps the service out of every render: the loops skip it and the play accepts `state=absent`. **A declaration merged before its backend is serving ships `enabled: false`**: an enabled declaration renders its route and cert SAN on any `traefik-acme.yml` run, not only on this play, so a dead backend would put a 502 route on the shared edge and re-mint the mesh certificate on an unrelated deploy. The cutover PR flips it to `true`; the play then applies it |

A word on `guard_tokens` for a platform. The default tokens are the file stem,
its underscore form and `external_service_<name>` — fine for `example-registry`,
useless for a one-word name where the first two collapse to the same string, and
actively dangerous for a name like `api` or `policy`, which matches half the tree
and would fail the guard on unrelated lines. A declaration carrying `hostnames:`
should name its own tokens: something unambiguous the service actually owns.

## Lane: a partner hosting a platform with several endpoints

A vendor rarely has one surface. A platform arrives wanting `api`, `audit`,
`authentication`, `provisioning`, `policy` and a console, all under a name of
its own. Three shapes carry that, and the choice is about who holds a key, not
about how many files get written.

### Where the names hang, and what certificate covers them

Every `hostnames` entry belongs to its own service, in one of three shapes:

| Entry | Publishes | Certificate name that covers it |
|---|---|---|
| `acme` (the `name`) | `acme.<site_id>.<site_dns_zone>` | `*.<site_id>.<zone>` |
| `acme-api` (`<name>-<surface>`) | `acme-api.<site_id>.<site_dns_zone>` | `*.<site_id>.<zone>` |
| `api.acme` (`<surface>.<name>`) | `api.acme.<site_id>.<site_dns_zone>` | `*.acme.<site_id>.<zone>`, once for the service |

Anything else is refused, by the play and the DNS generator alike: a bare
`api`, another service's prefix (`otherco-api`) or parent (`api.otherco`), and
nesting two levels deep (`demo.console.acme` — write `demo-console.acme`).
`api`, `policy`, `user` and `monitoring` are words every platform wants; the
rule is what stops one platform taking them from the goatnet-wide namespace.
`name` stays ONE label: it is the service's identity, its default status-row id
and its default guard token.

**Why the shapes are limited.** A certificate holds at most 100 names, and every
external service in `tls: edge` mode rides the mesh-only certificate that also
covers `auth` and `oauth2`. One name per hostname would have hit that limit with
a goatnet's hundredth hostname, and a refused order fails renewal for every name
on the certificate, the identity plane included. So the certificate carries:

- **`*.<site_id>.<zone>`** — one name covering every flat hostname of every
  service, present and future. A thousand flat services add nothing.
- **`*.<name>.<site_id>.<zone>`** — one name per `tls: edge` service that nests
  at least one hostname. A wildcard matches exactly one label, which is why
  nesting stops at one level: a second level would need a second wildcard.

The certificate grows with the number of services that nest, not with services
or hostnames. With the mesh-only certificate's ten fixed core names, that leaves
room for about ninety nesting services per goatnet. Prefer flat names; nest when
the separation is worth one certificate name.

**This reverses a choice, deliberately.** After a pen test the
mesh-only certificate listed internal names explicitly, so that at least the
public certificate would not enumerate them. The wildcard goes further on the
same concern: an individual service name no longer appears on any certificate,
nor in the public Certificate Transparency logs every Let's Encrypt certificate
is published to. Nothing new is exposed by it either. The private key stays on
the edge and already covers `auth` and `oauth2`. The public enrollment
certificate, `netbird.<zone>`, is one level up, is not matched, and is
unchanged. A `tls: partner` service's flat names do fall under the site
wildcard, but its records point at the partner's host, so goat never serves
them.

`external_service_wildcard_cert: false` in a goatnet's group_vars restores the
explicit per-hostname list for that goatnet alone. That is the rollback.

**There is no wildcard DNS, and none is possible on this path.** Mesh DNS is
served by CoreDNS's `hosts` plugin from a generated file of literal records
(`ops/dns/generate-mesh-hosts.py`, `roles/mesh-coredns`), so every name a
platform wants is still an entry somebody wrote, and the router still matches
listed names. The wildcard is on the certificate only. That is the trade this
path makes everywhere: nothing resolves that nobody declared, the footprint
guard can name every surface a teardown must visit, and the status page has a
row per surface.

Every label is at most 63 characters and the built name at most 253; both are
refused past the limit.

A nested entry shortens its own status row: `api.acme` reads
`Acme platform (api)` with id `acme-api`, not `acme-api-acme`.

### On the status page

Declared services are listed on the goatnet status page's **External
services** page (`https://isitup.<site>.<zone>/external`), not among the
goatnet's own services. The home page links to it with a one-line summary. Each
row is grouped under, and names, the declaration's `owner`, so nobody reads a
member-hosted service as one the goat core team runs (#1518).

A change to a declaration's `status:` block after the service is published
needs only the row list rendered again, not this play:
`ops/scripts/isitup-list.py apply <goatnet>` runs the list-only play
(`isitup-rows.yml`), which needs no secret
([isitup-status-page](isitup-status-page.md), "Keeping the list current").

The owner decides what is listed, in the declaration:

```yaml
  status:
    name: "Acme Platform"
    list: true                      # default for every hostname; false lists none
    lifecycle: live                 # live (default) | planned | retiring
    hostnames:                      # per-hostname overrides, keyed by the hostnames: entry
      api.acme: { list: false }                 # an API surface nobody browses
      heartbeat.acme: { health_path: /health }  # this surface's own probe path
      keycloak.acme: { lifecycle: planned }     # declared, not up yet
```

- `list: false` keeps a hostname off the page and out of the probe. Its DNS
  record, certificate name and route are untouched: `hostnames:` itself stays a
  plain list, and only the status page reads `status:`.
- `planned` shows the row with a "planned" pill and keeps it out of the status
  banner, so a surface that is not up yet does not turn the page red.
  `retiring` does the same for one on its way out.
- A per-hostname `health_path`, `name`, `desc`, `link_path`, `kind` or
  `expect_codes` overrides the declaration's for that row only. Without it
  every hostname is probed on the declaration's one `health_path`.
- Past `retire_after`, the rows drop off the page even before anyone re-runs
  the play: the deploy leaves them out, and the page hides a row whose date has
  passed.

**One name, paths behind it.** The plainest answer and the one to reach for
first. Declare `acme` alone; the partner's own reverse proxy splits `/api`,
`/audit` and the rest on their host. One record, one row, one certificate name,
one withdrawal. Nothing in goat grows.

**Several names, goat terminates** — `hostnames: [acme, acme-api, audit.acme, ...]` with
`tls: edge` (the default). Every label lands on the mesh-only certificate and on
one router rule; each gets its own status row so a reader checking "is audit up?"
sees that, not the platform's front door. The partner still holds no key for the
zone, the mesh-only allowlist still gates every name, and withdrawal is the same
single `enabled: false`.

**Several names, the partner terminates** — the same `hostnames:` with
`tls: partner` and `mesh_ip:`. The records point at the partner's host; goat
renders no router, no backend and no SAN. They serve TLS with their own
certificate, for names they choose, on a host they run — but that certificate
still has to be trusted, so the declaration also carries `acme_delegate`
(below); the goatnet never issues a certificate for a name the partner
serves, and never holds the partner's key, but it also never lets a partner
serve a name in our zone on an untrusted certificate.

### How a partner gets a trusted certificate

A partner that terminates TLS still needs a certificate browsers trust, for
names in OUR zone, on a host with no public address. `acme_delegate` is how
it gets one; the play refuses `tls: partner` without it. Of the ways Let's Encrypt
checks control of a name, HTTP-01 and TLS-ALPN-01 need the host reachable from
the public internet, on every renewal — a partner whose policy forbids serving
public traffic cannot do that, and a temporarily public name would be a
recurring exposure, not a one-off. DNS-01 needs only a TXT record, and a CNAME
may hand that record to a zone someone else controls. So:

    _acme-challenge.acme.zagros.<zone>          CNAME  acme.acme.partner.example
    _acme-challenge.acme-status.zagros.<zone>   CNAME  acme.acme.partner.example

One record per certificate name, named from each hostname's first label under
the site: `<name>` (whose challenge name also serves `*.<name>`, covering every
nested `<surface>.<name>`) and each flat `<name>-<surface>`. The partner's ACME
client writes the TXT at `acme_delegate` in their own DNS and follows the CNAME
(lego does by default; acme.sh with `--domain-alias <acme_delegate>`). Renewal is
unattended and never involves us.

Order matters here. The partner cannot hold a certificate until the delegation
record exists, and the play is what publishes it, so a partner's health probe
is reported, never required: an edge service still has to answer before
anything is published, a partner may serve no TLS at all until after the first
run. The status page carries the live answer from then on.

What is public, and what is not: the challenge names (which Certificate
Transparency logs publish for any trusted certificate anyway) and the
delegation target. No address, and no service name resolves publicly — the
names still exist only in mesh DNS. What the partner can do with the
delegation: obtain certificates for exactly the delegated names. Nothing else in
the zone, and not the site wildcard, whose challenge name is not delegated.

`external-service.yml` owns the records at Cloudflare, tagged with a comment
naming the service. It creates or repoints them on `present`, and removes every
record carrying the tag that the declaration no longer asks for — so `absent`,
a move to `tls: edge`, or a dropped hostname all converge, even when the
declaration file is already gone. A record at a delegation name WITHOUT the tag
belongs to someone else, and the play refuses to overwrite it.

What that last mode costs, stated plainly because it is the reason it is not the
default: goat's edge is no longer in the path, so the mesh-only allowlist is not
the gate — reachability control lives on the partner's host, and the status page
can only report what their health endpoint says. It also means goat never issues
a certificate for a name a partner serves, and never holds the partner's key:
goat's part is the public delegation record, which grants issuance for the
delegated names and nothing more.

All three leave the same way. The declaration is still the whole footprint, the
footprint guard still proves a temporary service left nothing behind, and
`state=absent` still drops every name in the declaration at once.

## Finishing a proposed service

A declaration usually arrives from `hello-galaxy-external-service`: its owner opens a pull request there, a code owner approves it, and the merge opens a pull request here carrying the declaration and a regenerated zone. Nothing in that pull request can finish it, because the remaining steps need a person's credentials. One command does them, from the platform engineer's laptop:

```bash
python3 ops/scripts/external-service-finish.py <PR>     # add --dry-run to see every step first
```

| Step | What happens | Why a person |
|---|---|---|
| peer | finds the mesh peer at the declared address and shows it; you confirm it is the requester's host and which groups it gives up | the address is the requester's claim |
| prepare | merges main in if the zone moved, regenerates the zone, signs the name snapshot, records the tier move, pushes | the goatnet CA key stays on the engineer's machine |
| review | you approve the pull request as it now stands; every check must pass | goat dismisses an approval given before a new commit, so the approval that counts comes after the signature |
| merge, pull | squash-merges, then applies from a fresh checkout of main | `ansible-preflight.sh` refuses a stale tree |
| tier | moves the peer into `<SITE>-EXTERNAL-INFRA` and out of the groups you agreed to remove | no node class mints the tier |
| apply | `external-service.yml state=present`, over the mesh or an env door; when the declaration is a re-declaration that dropped surfaces, exactly those names are passed to the mesh-DNS drift guard (`zone_record_removal_allow`) so they retire and every other record stays guarded | the play reaches the mgmt host over SSH |
| verify | each name resolves where it should; the service answers | |
| email | the requester's configuration, from the goatnet's onboarding address, to the goatnet identity whose onboarding record lists the PR author's GitHub login | when no identity lists that login, the terminal alerts and the engineer types a delivery address |

It runs on the laptop, not on the mgmt host, for the reasons every deploy does: the CA key, the ansible controller, and the engineer's own GitHub and credential logins live there. Re-running resumes at the step that stopped; `--from <step>` restarts from one.

## The command

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  ../scripts/ansible-preflight.sh \
    -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini \
    -e target=zagros_hosts -e service=example_registry -e state=present \
    external-service.yml
```

A declaration is merged with `enabled: false` while its backend is not yet serving; the cutover PR flips it. Before the play, one laptop step in either direction, because the mesh zone and its signed name snapshot are committed files ([ADR 1082](../adr/1082-name-fallback-resolver-signed-snapshots.md)):

```bash
python3 ops/dns/generate-mesh-hosts.py          # renders the record from the declaration
(cd ops/enrollment && go run ./cmd/name-snapshot-sign \
  --snapshot ../dns/zagros/name-snapshot.json --root-key <this goatnet's CA key> --age-identity ~/.config/age/keys.txt)
bash ops/ci/check-name-snapshot-sig.sh          # commit mesh-hosts, name-snapshot.json and the .sig
```

The play refuses to run against a zone that does not match the declaration, in either direction (F-197: a declared-but-unresolvable name deploys dark; a withdrawn name left in the zone answers for a service that is gone).

With `state=present`, in order:

1. The declaration exists, is enabled and is well-formed. A `retire_after` in the past is refused.
2. The committed zone carries every declared name — each entry of `hostnames` (default `[name]`), not only the first.
3. The host answers 200 on `backend_url` + `health_path`, from the mgmt host over the mesh.
4. The host is exactly one mesh peer and holds `<SITE>-EXTERNAL-INFRA` (needs `<SITE>_NB_API_TOKEN`; `-e skip_reach_check=true` skips this step only).
5. When `client:` is declared: `$secret_env` is set, Hydra is up, the client is created or re-PUT (Hydra v2 omits the secret from GET, so an existing client is always re-synced, which is also how a secret is rolled), lifespans pinned.
6. The play prints what the owner configures on their side: hostname, issuer with its load-bearing trailing slash, JWKS URL, the issuer pin for a host without mesh DNS, client id, redirect URI, the secret's variable name.
7. `traefik-acme.yml` renders the edge from the loop: router and backend, plus `*.<name>.<site>.<zone>` on the mesh-only cert if the service nests (flat names are already covered by `*.<site>.<zone>`).
8. The zone file is pushed to the site resolver (mesh-coredns, zone-only), and the new name is smoke-tested.
9. `isitup-deploy.yml` re-renders the status page with the derived rows: one per listed hostname, on the External services page.

With `state=absent`, after the owner has set `enabled: false` and regenerated the zone: the Hydra client is deleted, then steps 7 to 9 run and drop the route, the SAN, the record and the row. The play refuses `absent` while the declaration is still enabled. Order matters: `enabled: false`, regenerate and re-sign, run `absent`. **Keep the file** unless the owner has said the service will not return: a withdrawn declaration with `enabled: false` is the seed for bringing it back (next section) and costs nothing while it sits there. Delete it only on that confirmation; a file deleted first leaves the play no client id; `-e client_id=<id>` supplies it, and without it the play says a client may remain.

## Bringing a withdrawn service back

A service withdrawn with `enabled: false` comes back with the same file and the same play. Four steps, from a current checkout:

1. **Edit the declaration.** Set `enabled: true`. If the host re-enrolled, its mesh address changed: update `backend_url` (edge) or `mesh_ip` (partner) and `mesh_peer` to the peer the management API lists now (`goat --context <site> who --once`, or the NetBird dashboard). If `retire_after` is in the past the play refuses `present`; move the date or remove the field.
2. **Regenerate and re-sign the zone.** `python3 ops/dns/generate-mesh-hosts.py`, then `cd ops/enrollment && go run ./cmd/name-snapshot-sign --snapshot ../dns/<site>/name-snapshot.json --root-key <this goatnet's CA key> --age-identity ~/.config/age/keys.txt`, then `bash ops/ci/check-name-snapshot-sig.sh`. Records are being added, so the mesh-DNS drift guard has nothing to refuse; no `zone_record_removal_allow` is needed in this direction.
3. **Run `present`.** The command in "The command" above with `-e service=<name> -e state=present`. Its pre-flight proves the host answers on `health_path` over the mesh and holds `EXTERNAL-INFRA`, re-creates the Hydra client from the `client:` block when there is one (so the `secret_env` variable must still be exported), re-renders the edge, pushes the zone and re-renders the status page. Partner-mode services get their `_acme-challenge` delegation CNAMEs back in the same run.
4. **Commit the three files** (declaration, `mesh-hosts`, `name-snapshot.json` + `.sig`) in one PR.

What re-enabling does not need: a Traefik or certificate change by hand (the edge loop renders every enabled declaration), a status-page edit (the row derives from the declaration), or any core code. A withdrawal done this way leaves the file carrying the peer and date evidence it was withdrawn on, so the owner's return is a one-line edit and this sequence.

## What the service does on its side

The service verifies the goatnet's tokens itself and authorizes from its own records. The trust rule the first two consumers already follow, and the one this path assumes: the provider authenticates, the service authorizes, and a token's claims grant nothing on their own.

- Run an oauth2-proxy sidecar beside the app on the same host, or do the authorization-code flow natively. The proxy signs people in at `https://oauth2.<site>.<zone>/` and forwards the id_token as a bearer; the app verifies signature, issuer, audience (the client id) and expiry against the JWKS URL, then reads `email`, `email_verified` and, for organisation-scoped services, `aff.id`.
- Publish no host port except through the fence. The host is reachable from every role group, so a header-trusting app on an open port is a fence with a gap beside it. This is why the fence lives on the service's host and not on the mgmt host under goat's shared door role.
- Pin the issuer hostname to the mgmt host's mesh IP in the containers if the host does not use mesh DNS.
- Match the fence's cookie lifetime to the client's pinned token lifespan when the fence holds no refresh token, or the id_token expires under a live session and every request 401s.
- Keep the whole session out of the cookie when the token is large; a fence that must forward the id_token can do so from a server-side session store instead.

## What the identity plane guarantees, and what it does not

A relying party may assume the id_token carries `sub`, `email`, `groups` and, on goatnets where ADR 1043 Phase D is on, `aff` with the person's catalog organisation id. Two things are owed by core and are not a service's problem to solve:

- `email_verified` on console-minted identities: the console mints identities without it today, so a relying party that checks the claim refuses everyone the console minted.
- The `aff` claim: schema and consent-UI forwarder are live on zagros since 2026-07-13; the mint flag is off on every goatnet. Turning it on is the sandbox-first Phase D step and comes with a backfill.

A service that needs a claim the token does not carry is asking for a core identity change. That request is reviewed on its own merits, as core work, never as a per-service exception inside this path.

## The boundary, and how it is enforced

A temporary service's name may appear in goat only in its declaration files. `ops/ci/check-external-service-footprint.sh` (workflow `external-service-footprint`) fails a PR when a declaration carrying `retire_after` has any of its guard tokens under `ops/`, `.github/` or `terraform/` anywhere else. A permanent service is reported, not gated: goat-map's name legitimately appears in obs group_vars and in `zagros-prod-oidc-clients.yml`.

What this keeps out, by construction: a link in the identity console's welcome package (the status page already lists the service and the guide already links the status page), an operator key in the console for a service-side exporter, a role, a wrapper playbook, a template block. The first consumer's PR carried all of those before the path existed; under the path its goat-side footprint is one file.

## The first declaration: goat-map

goat-map (docs/design/goat-map.md) was `goat_map_enabled` + `goat_map_backend_url` in a group_vars file per goatnet plus three hand-written blocks in the Traefik template. On 2026-09-11 the four group_vars files became declarations and the blocks became the loop. Acceptance was a structured diff of the rendered template on zagros, zagros-dev and zagros-prod before and after: routers, services and certificate anchors identical; `generate-mesh-hosts.py --check` green with no file regenerated, because the `isitup_services` row for `map.<site>` stays hand-written and wins the dedup. goat-map declares no `client:` (its `oauth2-proxy-goat-map` client is registered by its own deploy) and no `retire_after`.

Not measured: a live `traefik-acme.yml` run on any goatnet after the change. The rendered file is identical, so the first unrelated run of that play is the live proof.

## Not built

- **A goat-hosted fence for an off-box service** (`oauth2-proxy-surface` on the mgmt host fronting a backend elsewhere). Rejected for hosts in `EXTERNAL-INFRA` for the reason above; an on-host service is not external and uses that role directly.
- **A machine lane.** A service whose API is called from code needs a client-credentials client or the caller's fabric bundle through rest-pub-bridge. The declaration reserves no field for it yet.
- **Blackbox probing from the declaration.** The obs host is its own group and cannot read the mgmt group's vars; a probe is one optional row in the obs group's `obs_blackbox_extra_targets`.
- **Migrating the other twenty template blocks.** Each can move onto a declaration when touched, proven the way goat-map was.

## Housekeeping this path exposes

Each goatnet's name snapshot signs under that goatnet's own CA; the finish script signs with it.

## Reversible archive

Retain the declaration with `enabled: false`; do not delete it. Validate the
owner proposal with `--state absent`. The finisher is for enabled services only:
withdraw through `external-service.yml` with `state=absent`, after generating and
signing the DNS snapshot. Pass `zone_record_removal_allow` with exactly the
removed fully qualified names in the generated revision, never the blanket override.
The play removes managed OAuth clients and partner certificate delegation; retain
credential references and client configuration for restoration. Mesh enrollment,
partner applications and their data are not removed by this path.

Restore by confirming the owner can serve again, reviewing `retire_after`, setting
`enabled: true`, regenerating and signing DNS, and applying `state=present`. Verify
TLS, authentication, application health and listings. Restoration recreates managed
clients; it does not restore expired user sessions.

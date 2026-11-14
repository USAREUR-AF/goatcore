# Standing up a GOAT-Net — a phase-by-phase walkthrough

A companion to the two canonical docs, written to be read two ways in the same
pass: each phase opens in plain language — what happens and why — and then gives
the platform engineer the exact tool, file, gate, and the **acceptance criterion
that proves the phase**. It covers both flows: a brand-new goatnet (**founder**)
and a new node joining an existing one (**joiner**).

- **The procedure** (the executable step list, gaps, decisions): [`goatnet-standup.md`](goatnet-standup.md).
- **The why** (the principles this path enforces): [`goatnet-deployment-doctrine.md`](../design/goatnet-deployment-doctrine.md).
- **The big-picture overview** (hardware, time expectations): [`goatnet-buildout-overview.md`](goatnet-buildout-overview.md).
- **Joiner mechanics** (why the passes cannot reorder): `node-addition.md`; model: ADR 1084.

> **Status — not yet cold-read certified.** This is the designed path the driver
> executes, and a joiner site was built through it. Full turnkey certification (a
> non-author standing a goatnet up from these docs alone) is still in progress.
> Treat any step that needs improvisation as a **defect in the path**, filed
> against the remediation plan — not operator error.

A GOAT-Net is one self-contained, self-healing network: a management host that
runs the mesh (NetBird management + signal + Postgres + Traefik), an identity
system that says who everyone is (Ory Kratos + Hydra), a Zenoh data-fabric router
that carries the traffic, and the services on top — plus, when declared, a
separate observability host built **in parallel**.

## Three engines, one driver

The path is not a checklist you hand-execute. It is three fail-closed engines
chained by one driver: one proves the plan is complete *before* touching a host,
one runs the work in order and stops at the genuinely manual steps, one proves
the result is actually *alive* — not merely that a process started.

| Engine | Answers | Command |
|---|---|---|
| **READY** | Is the config complete and consistent, before any host is touched? | `standup-config-preflight.py <group>` |
| **ORDER** | What runs, in what order — pausing only at the truly manual steps? | `goat-standup.sh <site>` |
| **DONE** | Is every provisioned aspect live and functional, proven not asserted? | `run-acceptance.py --env <site> --all-eligible` |

`goat-standup.sh <site>` **is** the phase list below, executable — resumable with
`--from <n>`, and it stops at the manual steps with the exact instruction.

## The pipeline at a glance

| Phase | What | Kind | lan | internet |
|---|---|---|:--:|:--:|
| **1** | config preflight → required inputs → declared ACL → env-door → host preflight | gate | ✓ | ✓ |
| **2** | host baseline + LUKS | auto | ✓ | ✓ |
| **3** | per-instance secrets | tooling | ✓ | ✓ |
| **4** | leaf certs + name-fallback snapshot | tooling | ✓ | ✓ |
| **5** | mgmt-stack (site-bootstrap)*, then relay-connect (internet) | auto | ✓ | ✓ |
| **6** | identity stack (Ory) → re-run site-bootstrap (completes 5) | auto | ✓ | ✓ |
| **7** | public TLS (traefik-acme) + **relay tier** | auto | — | ✓ |
| **8** | mesh-enroll host + re-mint router cert | **manual** | — | ✓ |
| **9** | Zenoh router + SPIRE | auto | ✓ | ✓ |
| **10** | LAN services  /  dashboard + OIDC | auto | ✓ | ✓ |
| **11** | first login + admin-promote + bootstrap NetBird token | **manual** | — | ✓ |
| **12** | declared ACL, NetBird service identities (mint + gate), account baseline, mesh CoreDNS, services, add-ons | auto | ✓ | ✓ |
| **13** | **acceptance gate** (`goatbuilder assay`) | gate | ✓ | ✓ |
| **14** | rescue nodes + cross-goatnet surfaces | auto | ✓ | ✓ |

\* First run **stops at the schema wait** — expected; phase 6 brings Ory up and
the re-run completes it.

**One number per phase, no gaps.** Phases run 1 to 14. The account-seed is not a
phase of its own: it runs *inside* site-bootstrap (phases 5 and 6). A LAN site
skips the internet-only phases 7, 8 and 11 — same order, a skipped phase runs
nothing. Older notes use the numbering from before 2026-10-29; the old-to-new
table is in [goatnet-standup.md §"Phase numbers"](goatnet-standup.md#phase-numbers).

**The obs leg** runs in **parallel** whenever the site declares an observability
host (`has_obs_host: true` + its config surface + inventory group + a measurable
host): baseline+LUKS and Forgejo start with phase 2; Prometheus/Alertmanager/Grafana
start once the router is up (phase 9) and join before the phase-13 gate.

---

## Before phase 1 — what must already exist

Before the driver runs its first line, the hardware, the accounts, and the plan
have to be in place. None of it touches a host; it is repo, cloud, and hardware.

| Before the run | What it is |
|---|---|
| **re-baseline** | Boxes off another GOAT-Net: de-enroll, drop Spock/pgBackRest membership, revoke bundles + leaf certs, `cryptsetup luksErase`, re-flash. |
| **credentials** | Cloudflare token, GHCR PAT, `aws-vault add`, and the age-encrypted dev-CA blob (out-of-band from the trunk owner — the one thing that cannot be self-served). |
| **terraform** (internet) | `terraform-<env>/` holds the public zone, relays, DNS. **The driver has no terraform phase** — you apply it. |
| **config surface** | An inventory group `[<site>_hosts]` *and* the group_vars **file inventory** (~30 files, not one `vars.yml`). Scaffold: `standup-config-preflight.py <group> --scaffold <site_id>`; every value naming another env becomes a `TBD_` sentinel the preflight refuses to pass. |
| **cert manifest** | `ops/enrollment/manifests/<site>-certs.yaml` from the TEMPLATE — never another goatnet's, its URI-SANs name that goatnet. |
| **fleet-pins row, env-door** | A row in `fleet-pins.yaml` (or it deploys unobserved) and an env-door entry in `doors.yaml` (or it is not deployable off-profile). |

**The topology is a choice, not a size.** `internet` (peers join over public
relays + a DNS zone; the host enrolls onto its own mesh at phase 8) is the real
system and most of the work. `lan` is a genuinely smaller, different topology —
not a warm-up. The driver auto-detects from config (`dashboard_cert_delivery_mode:
acme` ⇒ internet); `--profile` overrides.

---

## Phases 1–2 — gates & ground truth

**Plain:** The driver refuses to touch a host until it has proven the plan is
complete and measured what is actually out there. Then it lays the secure
foundation on this site's machines: a hardened base and full-disk encryption.

**Why the split:** two of the four steps change nothing at all — they *read*. A
plan can be complete and still describe a machine in a different state than
reality.

**For the platform engineer:**

- `step 1 GATE` — **config preflight (fail-closed)**: `standup-config-preflight.py <group>`. Reads group_vars, contacts no host. Blocks the run on any missing file, any remaining `TBD_` sentinel, or any line naming a foreign goatnet without a reason.
- `step 1 GATE` — **establish the env-door**: renders `doors.ini` from `doors.yaml` so the site is reachable off-profile.
- `step 1 GATE` — **host preflight**: measures the targets, changes nothing.
- `step 2 A` — **host baseline + LUKS** (this site's nodes only): `mgmt-host-baseline.yml` / `mgmt-host-luks.yml`.

**Done when:** the operator can log in over the mesh/door and the container data
root is on the encrypted mount. → `operator-ssh-login-core-a`,
`operator-ssh-login-core-b`, `docker-data-on-encrypted-mount`.

---

## Phases 3–4 — secrets & certificates

**Plain:** Every goatnet gets its own freshly-generated secrets and its own set of
cryptographic identities — the certificates that let each part of the system
prove, to every other part, exactly what it is.

**Why:** generated locally from the goatnet's own manifest, so no two goatnets
share a secret or a certificate identity. This is why you copy the template, never
another site's manifest.

**For the platform engineer:**

- `step 3 T` — **per-instance secrets**: Postgres / NB_ENCKEY / replicator / relay / MinIO / Ory, written to the gitignored env files, never committed.
- `step 4 T` — **mint leaf certs**: `mint-site-certs.sh <site>` reads `<site>-certs.yaml` and stages the router leaf into the zenoh read path. The router's mesh-IP SANs are placeholders here — re-minted at phase 8. The router cert CN is *per-goatnet* by design (`<site>-zenoh-router`); the site's own ACL matches its infra peers on that name.
- `step 4 T` — **name-fallback snapshot**: a signed, versioned name→IP snapshot so the mesh can still resolve itself if DNS is down.

**Done when:** the signed name-fallback snapshot serves; the certs themselves are
proven downstream, at the router (11) and the enrollment entrypoint (12). →
`name-snapshot-served` → (later) `router-tls-identity-mesh-ip`,
`enrollment-entrypoint-cert-identity`.

---

## Phases 5–6 — the management stack, identity & the schema-wait

**Plain:** The goatnet comes alive: the management host and its database, then the
identity system (who is who), and the mesh's first credential.

**The dance:** the first bootstrap run *stops on purpose*, because the database
schema does not exist until the identity system is up. Phase 6 brings Ory up; a
second bootstrap run completes what the first could not. Expected.

**For the platform engineer:**

- `step 5 A` — **mgmt-stack (site-bootstrap)**: first run **stops at the schema wait** — management + signal + Postgres + Traefik up, seed cannot run yet.
- `step 6 A` — **Ory** (Kratos / Hydra / UI): its migrations create the schema the seed needs.
- `step 6 A` — **re-run site-bootstrap**: now the schema exists, so this pass completes phase 5, seeds the account (founder), and prints + saves the one-time setup key.

**Founder vs joiner here:** a founder's re-run seeds a **new** account. A joiner
runs bootstrap with Spock off and takes its data by replication instead — see the
fork below.

**Done when:** the management API answers and refuses an unauthenticated caller,
signal speaks TLS not cleartext, the enrollment entrypoint presents this goatnet's
cert, and the OIDC dashboard login flow works. → `mgmt-api-answers`,
`mgmt-api-refuses-an-unauthenticated-caller`, `signal-tls-not-cleartext`,
`enrollment-entrypoint-cert-identity`, `oidc-dashboard-login-flow`.

---

## Founder vs joiner — the one real divergence

The entire path is identical for a founder and a joiner except for one idea: a
founder **creates** the identity plane; a joiner **attaches** to one that already
exists. A joiner sets `goatnet_founder: false` and `join_founder_site: <founder>`
in its config — read from config, never the command line, because setting it wrong
would seed a second account into a replicated schema.

| | **Founder** — originates | **Joiner** — attaches |
|---|---|---|
| Account | bootstrap seeds a **new** account + store | bootstrap runs Spock **off**; nothing seeded locally |
| Data | the setup key it prints is the genesis credential | data is **copied** from the founder, then **replicated** live |
| Plane | stands up its **own** relays, DNS, mesh | uses the founder's identity plane, DNS, relay ingress |
| Prereq | — | the founder's config must name this joiner as a peer first |

**The three-pass Spock join (joiner only).** The passes cannot be reordered: the
schema must exist before data is copied, and data must be copied before live
replication is wired — otherwise replication would truncate an empty table or
collide on a populated one.

1. **Pass 1 (phase 5)** — `site-bootstrap` with `participates_in_spock=false`. The account and schema are not seeded locally; they arrive by replication.
2. **Pass 2 (phase 6, joiner-only)** — `site-seed-from-peer.yml` (or the site's own `<site>-seed-from-peer.yml` when it keeps one), the guarded, data-only copy of identity + mesh state from the founder. `synchronize_data:=false` means no wipe, so the seed must precede wiring.
3. **Pass 3 (phase 6)** — re-run `site-bootstrap` with Spock enabled, creating the `netbird_mgmt` subscriptions and the joiner's `ory_narrow` node and repset (no Ory subscription yet).
4. **Founder-side mirror (phase 6, joiner-only)** — `spock-mesh-wire.yml` on the founder, scoped to this joiner only: Phase 7B and Phase 7D, so the subscriptions exist in **both** directions and an unrelated down peer cannot block the join.
5. **Joiner again (phase 6, joiner-only)** — `spock-mesh-wire.yml` on the joiner: its `ory_narrow` subscription to the founder and the two-way revocation-history reconcile.

The identity plane (Ory) federates the same way on a second subscription set
(`ory_narrow`, kratos + hydra), declared by `ory_spock_enabled: true` — which
**every** goatnet sets. A joiner is not finished until those subscriptions exist,
not just the substrate.

**Done when (joiner):** the seed completed, the founder's mgmt rows are present,
both the `netbird_mgmt` *and* the `ory_narrow` subscriptions are replicating, and
the identities landed. This is the exact set that catches a half-wired joiner —
substrate built but Ory subscriptions never created. →
`fed-seed-from-peer-completed`, `fed-founder-mgmt-rows-present-on-joiner`,
`fed-inter-site-netbird-subscriptions-replicating`,
`fed-ory-narrow-subscriptions-replicating`, `fed-ory-identities-replicated-to-joiner`.

---

## Phases 7–8 — reaching the internet

**Plain:** For a goatnet people join over the public internet, this is where it
grows its front gate: real TLS certificates, the cloud relays that let peers find
each other from anywhere, and — the one hands-on moment — enrolling the management
host onto its own mesh. A LAN-only goatnet skips this section entirely.

### The cloud relay buildout — where it actually happens

The relays are built in two places, on purpose — provisioning is Terraform's job,
configuration is the driver's:

1. **Provision (before the run, Terraform):** `terraform-<env>/` creates the relay VPSes, the public DNS zone, and the records. *The driver has no terraform phase* — you apply this before the run.
2. **Connect (phase 5, internet):** `site-rathole-client.yml` (or the site's own `<site>-rathole-client.yml`) links the management host out to the relays (rathole tunnel).
3. **Build the tier (phase 7, two steps):** `generate-relays.sh` reads Terraform's `relays` output → writes `inventory/hosts-<site>.ini`; then `relay-server.yml` installs **coturn + rathole + the certbot TLS-renewal path** onto those hosts. Two steps because a relay VPS that exists in Terraform but has no inventory row is a relay no playbook can target, and nothing reports it.

A **joiner** builds no relays of its own — phase 7 wires it into the *founder's*
relay ingress and re-renders the dispatch.

**Manual step (phase 8):** the operator enrolls the host with the printed
`netbird up` line (`preflight-enrollment.sh` prints it), confirms `netbird status`
shows *Management: Connected* and a `100.x` mesh IP, then fills that mesh IP into
the inventory and the cert manifest and re-mints:
`mint-site-certs.sh <site> --only router`. The router's identity is bound to the
address it will actually serve on.

**Done when:** the declared relay fleet answers with a system-trusted cert, the
public login domains are not flagged by Safe Browsing, and a factory-fresh peer can
enroll over the internet and reach the mgmt API. → `relay-fleet-answers-if-declared`,
`relay-cert-system-trust`, `public-auth-fqdns-not-flagged`, `fresh-peer-enrolls`,
`fresh-peer-mesh-reaches-mgmt`.

---

## Phases 9–11 — the data fabric & the front door

**Plain:** The Zenoh router — the data fabric that actually carries the goatnet's
traffic — comes up, bound to the address it enrolled on. Then the human front
door: the dashboard and single-sign-on, and the first operator login that turns a
fresh install into an administered one.

**The knot:** the goatnet's own service accounts are minted *through* the
dashboard, but the dashboard needs an admin to mint them, and the first admin does
not exist until a person logs in and is promoted. So one login is unavoidably by
hand.

**For the platform engineer:**

- `step 9 A` — **zenoh router**: binds the mesh IP (internet) or LAN IP (lan), cert from phase 4/8. "Up" is not enough — phase 13 checks it is *bound* and presenting the right identity.
- `step 10 A` — **dashboard** + **register OIDC clients** (internet); or the **LAN service surface** (dashboard + DNS + TLS routes + STUN) on lan.
- `step 11 MANUAL` — **first login, admin-promote, the bootstrap NetBird token**: enroll your laptop, read the one-time password from the host, log in, re-run the dashboard deploy *once* to promote yourself to admin (the UI 403s until then), then create the first service user (`svc-<site>-netbird-infra`), mint its token, and store it. Everything phase 13 automates reads from that token.

**Done when:** the router answers and its adminspace is not writable from the
fabric, the router presents the right mesh-IP identity, and a fenced surface's OIDC
client exists with its RBAC fences enforcing. → `zenoh-router-answers`,
`adminspace-is-not-writable-from-the-fabric`, `router-tls-identity-mesh-ip`,
`fenced-surface-oidc-client-exists`, `admin-fences-are-enforcing`.

---

## Phase 12 — identity, DNS & the services on top

**Plain:** With an admin token in hand, the driver mints exactly the service
identities this goatnet needs, sets the network's baseline access rules and
internal DNS, and stands up the on-top services — the state-of-record store, the
public intake front desk, the onboarding runner.

**Why:** this is the widest phase, and where a goatnet stops being infrastructure
and starts being usable. It mints only what is missing, so re-running is safe, and
a gate confirms every required identity is present before the services that depend
on them come up.

**For the platform engineer:**

- `step 12 T + GATE` — mint the NetBird service identities (creates only what is missing), then a **fail-closed gate** verifies all are present.
- `step 12 A` — **netbird account baseline** (mesh-DNS group + policy) and **mesh CoreDNS** (internet), after which the laptop's `/etc/hosts` pin is dropped. Mesh CoreDNS runs only for a site with a mesh zone. A joiner with no zone skips it and resolves through the founder's DNS today; that is a gap, not the target: every site is to run its own resolver ([ADR 1233](../adr/1233-a-joiner-runs-the-replication-core.md), gate row I3).
- `step 12 A` — **pubsub-state Repl mirror**, the **onboarding intake front desk** (public, ADR 1062 stage 1), and the **onboard-runner** (auto-mint on approval, stage 3).

**Done when:** every required service identity is present, *every declared service
is both running and resolvable in mesh DNS*, the state-of-record primary answers,
the intake front desk accepts a browser submit, and the onboard-runner knows this
goatnet and reaches the mgmt API. → `netbird-service-identities`,
`declared-service-actually-deployed`, `every-declared-service-resolves`,
`pubsub-state-forgejo-primary-up`, `intake-form-browser-submit`,
`onboard-runner-reaches-mgmt-api`.

---

## Phase 13 — the acceptance gate: done means green, proven

**Plain:** The goatnet is not "stood up" when the automation finishes. It is stood
up when an independent gate proves every piece it was supposed to build is actually
alive and doing its job — and names, precisely, anything that is not.

**For the platform engineer.** The gate resolves every probe against a value set
**derived from this goatnet's own config** — phase 13 runs `generate-values.py`
first, then the suite:

```bash
python3 ops/validation/generate-values.py <site>
python3 ops/validation/run-acceptance.py --env <site> \
  --values ops/validation/values-<site>.yaml --all-eligible
```

- **Every applicable row PASS** = complete. A probe that cannot run is `UNKNOWN`, *never* a pass — read the report, not the exit code.
- **Service coverage is exact**: `declared-service-actually-deployed` fails, by name, for any service you declared but did not deploy. "Stood up" means the gate is green, not that ansible finished.
- **The obs host is graded too** (`has_obs_host: true`): targets up, rules loaded, Alertmanager reached, deadman firing; with a `pubsub-state.yml` the state-of-record primary must *answer*, not merely have an active timer.
- **Honesty grammar**: nothing-heard is never green. Each row cites the defect it catches; the suite is both the phase-13 gate and the standing regression surface.

**This is the loop that makes it turnkey.** When a standup hits a gap, the fix is
not only the patch — it is a new preflight check or a new acceptance row, so the
*next* standup cannot hit it. Prove function, not process; never-exercised is
untested.

---

## Phase 14 — joining the cross-goatnet surfaces

**Plain:** The new goatnet takes its place among the others: the fleet-wide views
— like the relay-health overview — learn about it, so it appears everywhere the
fleet is watched from.

**For the platform engineer:** `step 14 A` — the relay-health overview is
(re)generated on every box so the new goatnet's relays are represented across the
fleet. Declared in `ops/goatnets/cross-env.yaml`, applied by the cross-env driver.

**Done when:** the new goatnet's relay-health targets are scraped by the fleet-wide
overview. → `relay-tls-targets-are-scraped`.

---

*One entrypoint: `goat-standup.sh <site>` — the phase list above, executable,
resumable with `--from <n>`. A step that needs improvisation is a defect in the
path, filed against the remediation plan — not operator error.*

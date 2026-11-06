# Capacity, scale envelope, and the austerity ladder

**Status:** measured baseline + derived model, 2026-08-20. The baseline numbers
are measurements on one management host; everything past §3 is arithmetic on
top of them and is labelled as such.

**Scope:** what the goatnet costs today, what 1,000 producing/consuming systems
would cost, and the order in which services are shed when a site cannot serve
everything.

Related: goat-net-scaling.md,
[ADR 1122](../adr/1122-scale-envelope-systems-vs-peers-pod-tier.md),
surface-overload-protection.md,
[ADR 1123](../adr/1123-client-demand-must-not-amplify.md),
[environment-health-surface.md](environment-health-surface.md).

---

## 1. What the hardening actually bought

(omitted)

---

## 2. Measured baseline

(omitted)

---

## 3. Extrapolation to 1,000 systems

(omitted)

---

## 4. The austerity ladder

For when a site cannot serve everything. Each rung states what is lost, what
is bought, and — the part that matters — **what breaks if you skip ahead**.

The ordering principle: **shed by blast radius, not by cost.** T3 is cheap to
shed and expensive to run, but it is shed late because losing observability
during a degradation is how a recoverable event becomes an unrecoverable one.

### Rung 1 — browse and demo (T5)

**Shed:** synth producers (22), airdefenders, gym, goat-docs, brief, herd,
zenoh-sandbox, goat-map-synth, isitup, selfservice-ui.
**Buys:** ~9.5 % CPU, 1.7 GiB RAM, and — larger than either — the fabric
message rate the synth producers generate, which is most of the current
persisted 46 msg/s.
**Cost:** demos stop. Onboarding walkthroughs stop. No operational capability
is lost.
**Reversible:** immediately, no state.

> The single highest-value rung, because it cuts *fabric* load, not just host
> load. Everything below cuts host resources only.

### Rung 2 — admin convenience (T4)

**Shed:** microscope, netvis, goat-map, stethoscope, flashlight, panoscope,
consistency-console, dashboard, crypto-posture and their oauth2-proxy sidecars.
**Buys:** ~2.6 % CPU, 2.0 GiB RAM.
**Cost:** operators lose *inspection*, not *control*. `goat` CLI, the portal
API and the fabric itself are untouched — every question these surfaces answer
is still answerable, more slowly, from the CLI.
**Keep panoscope longest:** it is the only surface that queries on request
(ADR 1123) and is the fastest path to "what is actually happening".
**Reversible:** immediately, no state.

### Rung 3 — observability convenience (T3)

**Shed:** Grafana, process-exporter, postgres-exporter, goat-logship.
**Buys:** **~11 % CPU** — the best CPU-per-container on the ladder — and
0.4 GiB.
**Cost:** no dashboards; Prometheus still scrapes and alerts still fire. Log
shipping stops, so off-box forensics degrade.
**Do not skip to here.** Shedding T3 before T5 means keeping the demo load that
is generating the pressure while losing the ability to see it.
**Reversible:** immediately; a gap in shipped logs is permanent.

### Rung 4 — observability core (T2)

**Shed:** Prometheus, Alertmanager, cadvisor, blackbox, node-exporter.
**Buys:** ~3.6 % CPU, 0.6 GiB.
**Cost:** **you are now flying blind.** No alerting, no history, no capacity to
diagnose what comes next. The environment-health surface (ADR 1036) still
publishes on the fabric, so `goat health` remains the last instrument standing
— it is fabric-native and does not depend on the obs stack.
**This rung is a decision to stop measuring in order to keep serving.** It
should be deliberate, time-boxed, and announced.
**Reversible:** immediately; the metric history for the outage window is gone
forever, which is exactly the window you will want later.

### Rung 5 — floor (T0 + T1 only)

**Remains:** Zenoh router, NetBird management + signal, Postgres, Traefik,
mesh-DNS, portal, SPIRE, MinIO + audit-subscriber, id-console, and the fabric
health publishers.
**Cost:** ~45 % of one core, ~6.1 GiB. That is the whole engineering platform:
**identity, fabric, mesh, ingress, and the compliance write path.**

**What must never be shed, and why:**

| Service | Why it is floor |
|---|---|
| Zenoh router | the fabric; without it there is no data plane |
| NetBird mgmt + signal | enrollment and mesh liveness; losing it strands peers |
| Portal | the identity/slot registry — the ACL is generated from it |
| MinIO + audit-subscriber | the WORM audit path; a compliance obligation, not a convenience |
| SPIRE | workload identity; shedding it fails workloads closed |
| Traefik + mesh-DNS | every mesh-internal service is addressed through them |
| Postgres | NetBird's state |

**The audit path is the one that will be argued about.** It is ~1 % CPU and
the largest disk consumer, so under storage pressure it looks like the obvious
cut. It is not shed, because ADR 0811 puts it on the substrate's critical-write
path and dropping audit is a compliance gap rather than a degradation. Under
storage pressure the correct lever is **retention and the `volatile/` class**,
not turning off the writer.

### The mirror: shed order is deploy order, reversed

Not an aesthetic choice — it falls out of dependency. `site-bootstrap.yml`
builds a site bottom-up, and contraction unwinds the same graph from the top:
**you can only shed a thing nothing still standing depends on.** Deploy-last is
shed-first because deploy-last has the most above it and the least below it.

| Deployed | | Shed |
|---|---|---|
| Phase 1 — NTP, health agent | -> | **floor**, never |
| Phase 2 — mgmt stack (Postgres, NetBird) | -> | **floor**, never |
| Phase 2.5 — Traefik | -> | **floor**, never |
| Phase 3 — identity seed | -> | **floor**, never |
| Phase 4 / 8 — observability | -> | rung **4**, then **3** |
| own plays — fabric, portal | -> | **floor**, never *(see below)* |
| own plays — inspection surfaces | -> | rung **2** |
| last — synth vendors, gym, demos | -> | rung **1** |

**One deliberate break, named rather than hidden.** The Zenoh router deploys
*late* — `site-bootstrap` Phase 7D explicitly fences it out and it arrives via
its own play — so a strict reversal would shed it early. It is in the floor and
never shed, because it is the reason the site exists: a goatnet without a
fabric is a host running management software. **Late-deployed does not mean
peripheral**, and this is the single place the two orderings genuinely diverge.

### Operating it

The ladder is executable, not just a document:

```sh
ops/scripts/austerity.sh status              # what is running, by tier
ops/scripts/austerity.sh contract 3          # dry run: shed 5, then 4, then 3
ops/scripts/austerity.sh contract 3 --apply
ops/scripts/austerity.sh restore --apply     # lowest tier first
```

Tier membership and its reasoning live in `ops/austerity/tiers.yaml`, because a
judgement about blast radius belongs where a reviewer can diff it, not in a
`case` statement.

Four properties chosen for the conditions it runs in:

- **`stop`/`start`, never `down`/`up`.** Stopping leaves the container, its
  config and its volumes intact, so restore is seconds with no image pull and
  no re-render — **it works when the registry is unreachable**, which is
  precisely when you may be contracting. `compose down` would make recovery
  depend on the network you are already short of.
- **Dry run is the default.** `--apply` is required, every time.
- **Tier 0 has no flag.** There is no way to ask this tool to shed the floor.
- **Unclassified containers are reported and never touched.** A container
  nobody has classified is exactly the one whose loss you cannot predict, so
  silence is not consent.

### Progression at a glance

| Rung | Sheds | CPU back | RAM back | Loses |
|---|---|---|---|---|
| 1 | T5 (38) | 9.5 % | 1.7 GiB | demos — **and fabric load** |
| 2 | T4 (22) | 2.6 % | 2.0 GiB | inspection, not control |
| 3 | T3 (4) | **11.2 %** | 0.4 GiB | dashboards, log shipping |
| 4 | T2 (5) | 3.6 % | 0.6 GiB | **alerting and history** |
| floor | T0+T1 (31) | — | — | nothing essential |

**Total recoverable: ~27 % of one core and 4.7 GiB** — which on this hardware
is not the point. The ladder's value is not the CPU it returns; it is that on
constrained hardware, or under a fabric-rate event, **the shedding order is
decided in advance rather than under pressure.**

---

## 5. Honest gaps

(omitted)

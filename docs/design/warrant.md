# warrant — scoped, evidenced, expiring network authorizations

Canonical detail for [ADR 1129](../adr/1129-warrant-scoped-expiring-network-authorizations.md).
The ADR holds the decision and its reasoning; this holds the shapes, the schema,
the surfaces and the migration.

**Status: built for access rules** — `ops/ansible/netbird_acl_matrix.yml` is generated from `ops/warrant/` and CI refuses a hand edit ([warrant-runbook](../operations/warrant-runbook.md)). **Extended to peer membership** by [ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md): design [`mesh-change-control.md`](mesh-change-control.md), live on zagros-dev.

**Where the CLI reads the declaration.** `goat acl test` and `goat warrant` compile in no deployment's declaration. They read it from `--declaration <dir>`, else `GOAT_ACL_DECLARATION_DIR`, else the active context (`acl_declaration_dir`, or its `acl/` directory that `goat context refresh` fills from `get.<site>/acl/`), else the `ops/ansible/` of the checkout they run in; none is `E_ACL_DECLARATION_MISSING`. The goat-get role serves the declaration at `get.<site>/acl/` when `goat_get_acl_declaration` is true.

---

## The thesis in one page

An access rule is an authorization. Every authorization should say **who may
reach what, on what**, be **held by someone**, and **expire**. A mesh ACL
normally does only the first, and often so loosely that a rule names no
protocol at all.

That looseness is not carelessness. `protocol: all` is what a rule says when
nobody can prove what narrowing it would break — an **information problem**
before a discipline one. The knowledge of which ports a service needs sits with
whoever runs the service; the rule is written by whoever administers the
network; nothing carries the first fact to the second person.

**One principle underpins all of it: git is the system of record for network
policy decisions.** The decision and the artifact are the same object — the
reviewed diff is what generates the ACL — so there is no gap between what was
agreed and what is deployed, attribution is permanent, review happens before
effect, and rollback is `git revert` rather than a special path. Everything
below follows from taking that seriously: a request is a pull request, routing
is `CODEOWNERS`, a decision is a review, and no bespoke approval service is left
with anything to do.

On top of that, four moving parts, in the order they depend on each other:

| | | |
|---|---|---|
| **1** | **the unit** | a *warrant*: one source class, one destination class, one named port-set, one owner, one expiry |
| **2** | **vocabulary** — what may be granted | **port-sets**: seven named sets, stored once, that *generate* the rules |
| **3** | **evidence** — what a grant would cost or enable | the **listening surface** joined against the declared grant |
| **4** | **authority** — who may issue one | a six-rung ladder where five rungs reach no queue, sitting on a **governance plane that only git can change** |

The last line is the load-bearing one. **The instance plane is fast because the
governance plane is slow**, and the two never share a path.

### Two phrases, in plain words

Repo shorthand, written out once so the rest of this document can use them
without hiding behind them.

**"Fail-closed" = when the system does not know something, it says no.** Three
specific behaviours, not a property:

| when this is unknown | what happens | what is ruled out |
|---|---|---|
| who should decide a request | it is **refused at submission**, reason shown | routing it to a catch-all queue that waits on nobody |
| what a node class listens on, none having reported | the join reads **`unknown`** and cannot justify a narrowing | assuming "listens on nothing" and cutting a live service |
| a request nobody decided in its window | it **closes as `expired`**, requester told | an ignored request quietly becoming a grant |

**"At-most-own-tier" = you cannot approve reach you do not hold yourself.** A
site's admin can approve warrants inside their site, not ones granting access to
the parent goatnet's substrate. Onboarding already works this way.

---

## 1. The measurement

Reproducible against `ops/ansible/netbird_acl_matrix.yml`; not to be quoted once
the declaration has moved on.

### 1.1 The vocabulary is already closed — and already named, in prose

118 rule-shapes carry a protocol; **13 distinct `(protocol, ports)` combinations
exist**, of which 11 carry ports. The other two are `all` (48 uses) and `icmp`
(12, which has no ports and is correct as written).

| uses | protocol · ports | the name already in the rule |
|---|---|---|
| 8 | tcp 443, 53, 5353, 7447 | `(web/dns/fabric tcp)` |
| 8 | udp 53, 5353, 7448 | `(dns/fabric udp)` |
| 7 | tcp 443, 53, 5353 | `(web/dns only)` |
| 7 | udp 53, 5353 | `(dns udp)` |
| 7 | tcp 22 | `(ssh only)` |
| 4 | tcp 7447 | `(fabric tcp)` |
| 4 | tcp 443, 53, 49, 6514 | `(management-plane tcp)` |
| 4 | udp 53, 5353, 123, 514, 162 | `(management-plane udp)` |
| 4 | udp 5353 | `(mesh dns only)` |
| 3 | tcp 443 | `(HTTPS :443)` |
| 2 | udp 7448 | `(fabric udp)` |

The right-hand column is copied out of the `name:` field of rules that already
use each set. **The declaration names its port-sets 118 times and stores them
zero times.**

Two costs follow. Changing the Zenoh TCP port means finding eight identical
literals and being sure there was no ninth. And the rationale has already
drifted: the three rules carrying `tcp 443,53,49,6514` describe themselves in
two different texts, same fact, nothing keeping them equal.

---

## 2. Vocabulary — port-sets

### 2.1 Seven names, extracted rather than designed

| name | protocol · ports | uses | purpose |
|---|---|---|---|
| `substrate` | tcp 443, 53, 5353, 7447 · udp 53, 5353, 7448 | 16 | reach the goatnet's own services: mesh web, DNS, the fabric |
| `substrate-restricted` | tcp 443, 53, 5353 · udp 53, 5353 | 14 | the same, minus the fabric — what probation and quarantine keep |
| `netadmin` | tcp 443, 53, 49, 6514 · udp 53, 5353, 123, 514, 162 | 8 | bearer-gear management plane: web, DNS, TACACS+, syslog-TLS, NTP, SNMP trap |
| `shell` | tcp 22 | 7 | interactive login — break-glass and network administration |
| `fabric` | tcp 7447 · udp 7448 | 6 | Zenoh router: a publish/subscribe leg and nothing else |
| `mesh-dns` | udp 5353 | 4 | name resolution alone |
| `web` | tcp 443 | 3 | an HTTPS service and nothing else |

A set spans protocols because the measurement shows the halves are never used
apart — `tcp 443,53,5353,7447` and `udp 53,5353,7448` appear 8 times each, in
the same rules. Modelling the pair as one named thing removes the class of
change where the TCP half is updated and the UDP half forgotten.

### 2.2 Schema

```yaml
port_sets:
  - name: substrate
    purpose: >
      Reach the goatnet's own services — mesh web and management (443),
      DNS (53) and mesh DNS (5353), and the Zenoh fabric (7447/7448).
    owner: platform-admin
    members:
      - { protocol: tcp, ports: ["443", "53", "5353", "7447"] }
      - { protocol: udp, ports: ["53", "5353", "7448"] }
    # Class pairs that MUST retain reach on this set. Generated into
    # netbird_acl_tests, so a narrowing that breaks one fails the gate.
    required_by:
      - { src: KID, dst: INFRA }
      - { src: POD, dst: INFRA }
```

`purpose` is prose and load-bearing — it is the thing that currently exists in
three drifting copies, and the registry's job is to make it singular.

**A rule names ONE arm, not the set.** A NetBird rule is single-protocol, so a
rule intent carries its own `protocol:` and a `port_set:` naming where the
ports come from; the generator resolves that set's arm for that protocol. The
alternative — one intent expanding to every arm — would emit a `udp 7448` rule
next to every `tcp 7447` one and change the declaration. `(protocol, ports)`
must therefore name exactly ONE set, which `generate.py` enforces: two names
for one shape is the re-fragmentation this registry exists to prevent.

**`expires:` and `owner:` are registry-only** and are stripped at generation,
so nothing downstream ever sees them and the matrix stays byte-identical.
That is what makes expiry cost one field rather than a service.

### 2.3 The registry generates the rules

```
port-sets.yaml ──┐
                 ├─→ generate ─→ netbird_acl_matrix.yml rules (expanded)
rule-intents.yaml┘             └─→ required_by ASSERTS INTO netbird_acl_tests
```

**`required_by` asserts into the tests; it does not generate them.** The
`netbird_acl_tests` block is hand-written under a stated discipline — it
carries `deny` rows, baselines and cross-scope targets that `required_by`
cannot express, and its own rule is that every assertion restates an intent
already written down. Generating the block would delete that. So the gate
checks that every `required_by` pair is granted its set somewhere and fails
when one is not, which is the property that matters — a narrowing that severs
a class fails loudly — without dropping assertions nobody agreed to drop.

The same shape as `ops/serve/services.yaml` → Caddyfile and `isitup_services` →
mesh DNS zones. The generated file stays fully expanded and human-readable:
**the indirection lives in the source, not in what is applied or reviewed.** An
operator reading the applied ACL sees literal ports, as today.

An advisory registry — one that describes what the rules should contain — is
the F-324
failure by construction. It generates or it is not worth building.

### 2.4 The lens on netvis — one surface, not a new one

`panoscope_core::portsets` is the single implementation of "which named set is
this rule's port list". `goat warrant why-blocked` and netvis both call it, for
the same reason `aclcheck` is shared: three readings of what a rule grants is
three chances to disagree about one goatnet.

netvis's existing pair view (`/api/pair/:a/:b`) gains four fields per rule —
`port_set`, `port_set_purpose`, `port_set_owner`, and `expires` / `grant_owner`
from the intent. The last two are the interesting ones: they are registry-only
metadata, stripped by `generate.py` before the ACL is applied, so they exist
**nowhere in the live plane**. A reader of the applied ACL cannot know a grant
is temporary. This is the only place that says so.

**It adds no load.** The vocabulary and the intents are `include_str!`-compiled
into the netvis binary and parsed once per process behind a `OnceLock`, so a
pair view costs what it cost before plus two map lookups — no NetBird call, no
fabric query, no I/O ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)).
Neither file is compiled into `panoscope-core` itself: that crate is linked by
microscope, stethoscope, panoscope and flashlight, and none of them renders a
port-set.

`resolve` is an **exact** shape match, not a subset test. A rule granting
tcp/443 is `web`; naming it `substrate` because substrate also contains 443
would label a rule after a set granting far more than the rule does. A shape
the vocabulary does not name renders `null`, which is a real answer — it means
that rule is not yet expressible in the registry — and a netvis test asserts
that every port-carrying permit in the shipped declaration does resolve, so
`null` cannot appear silently.

A malformed registry degrades to no annotation rather than an error page. CI
(`ops/ci/check-warrant-registry.py`) is what refuses a broken registry; a
rendering surface must not go dark over a defect it did not cause.

---

## 3. Evidence — the listening surface, and the join

### 3.1 A configuration fact, not telemetry

Each node publishes what it listens on: `ss -ltnp` reduced. A property of how
the machine is configured, readable without observing a single packet.

**This does not touch the brokerless no-telemetry position.** Nothing counts a
message, samples a flow, or sits in a data path. It is the reveal-only shape the
environment health surface already uses
([ADR 1036](../adr/1036-environment-health-surface.md)): each node self-publishes
a fact about itself, a passive reader rolls it up.

```yaml
node_class: INFRA
observed_at: 2026-08-13T14:22:01Z
listening:
  - { protocol: tcp, port: "443",  process: traefik }
  - { protocol: tcp, port: "7447", process: zenohd }
  - { protocol: udp, port: "7448", process: zenohd }
  - { protocol: tcp, port: "22",   process: sshd }
```

Rolled up **per class**, because the ACL grants to classes and the join must be
at the same altitude to be actionable.

Three honesty rules, carried from the environment-health discipline because the
failure mode is identical:

- a class with **no reporting node** reads `unknown`, and cannot license a
  narrowing;
- a class whose nodes disagree renders the **union** and says they disagree —
  narrowing to an intersection would sever whichever node was the odd one;
- every roll-up states its **horizon**: how many nodes of the class reported, of
  how many are enrolled.

### 3.1a Usage evidence — what already exists, and what is one expression away

The "brokerless means no traffic evidence" framing conflates two layers, and the
conflation costs us a real signal.

**Zenoh message-level observability is genuinely structural.** No central
subscriber sees every message; that is the brokerless root and it does not
change.

**The WireGuard data plane is a different layer, and it counts.** Two facts:

| signal | status | what it answers |
|---|---|---|
| per-peer `rx_bytes_delta` / `tx_bytes_delta`, handshake age, direct-vs-relay | **already published** — the mesh health domain ([ADR 1042](../adr/1042-mesh-link-health-domain.md), `node-net-health.md`) carries these per snapshot | *has this pair exchanged any bytes, and when* |
| per-rule packet/byte counters on the ACL itself | **not built** — the fork's `client/firewall/nftables/acl_linux.go` builds rule expressions with no `expr.Counter`, though the eBPF-proxy path uses one | *was this grant ever matched* |

The second is a small change: append `&expr.Counter{}` to the ACL rule
expressions, then read `nft list ruleset -j` **locally, on the peer that
enforces the rule.** nftables counters are per-rule and free to read.

**This is not a broker and does not become one.** The count is produced at the
enforcement point, self-published as a fact by the node that owns it, and rolled
up by a passive reader — the same reveal-only shape as the listening surface and
`EnvHealthV1`. Nothing sits in the data path, and no client demand amplifies
into fabric load ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)).

**P2P is not the obstacle.** Tailscale is also peer-to-peer WireGuard, and its
flow logs are generated *by each client* and reported to its coordination plane
— because in a P2P mesh there is no middlebox to tap, so every vendor must
source at the endpoint. The difference is that they built the endpoint
reporting and this substrate built half of it.

**And a counter is not a flow record**, which is an opsec advantage rather than
a limitation: it yields recertification evidence — *this grant matched zero
packets in 90 days* — without storing who talked to whom about what.

### 3.2 The join, in both directions

Declared grant against observed surface. Closing asks *what would this cost*:

```
p2p → gear                       grants: ALL ports
gear classes listen on:          tcp 7447, udp 7448, tcp 22, tcp 443
                                 (4 of 4 enrolled nodes reporting)
narrow to `fabric` + `shell`  →  removes no port any node is offering
narrow to `fabric` alone      →  would remove tcp 22, in use on 4 nodes
```

Opening asks *what else does this grant*:

```
requested   EX-DEMO-GEAR → ZAGROS-INFRA on `substrate`
also grants the same reach to 4 other peers already in EX-DEMO-GEAR
narrower?   `fabric` alone covers the stated need — offered as the default
```

That last line is the mechanism that keeps grants small: **the system proposes
the narrowest port-set satisfying the stated need, not the one asked for**, so
the path of least resistance produces the smallest grant.

**Evidence, never authority.** The listening surface informs a human or a
declared request. It never issues a grant — a node that opened a socket would
otherwise be authoring its own permissions.

---

## 4. Authority — the ladder

### 4.1 Both directions, six rungs, one queue

| the change | who decides | how long |
|---|---|---|
| **narrowing** anything | nobody — applied and logged | immediate |
| an opening **already permitted** | nobody — diagnosed, no warrant created | seconds |
| an opening **matching a standing template** | nobody — the decision was made once, over the class | seconds |
| an opening needing **relief now**, own scope | the site's own admin, **provisionally** | minutes; expires in 24h |
| a **novel** opening, own scope, existing port-set | the site's own admin | minutes |
| **new port-set · new class pair · cross-scope · `all` · `NETWORK`/`BREAKGLASS`** | platform admin | the only rung where anyone waits |

**Five of six rungs reach no queue.** The one that does is where the vocabulary
changes or a trust boundary is crossed — where waiting is correct behaviour.

**Narrowing being free is the mechanism, not a convenience.** It means the mesh
gets stricter by default rather than by campaign, and nobody is ever queued
behind an administrator in order to make the goatnet *safer*.

### 4.2 Most openings are not ACL problems

The common report — *"my sensor can't reach the fabric"* — is usually DNS, a
missing membership, a service not listening, or an offline peer. Routing that
into an approval queue produces a grant that fixes nothing and widens the ACL
permanently. So the entry point is a diagnosis:

```
goat warrant why-blocked GEAR → INFRA :7447

  ACL          PERMITTED — a rule already allows this (gear → infra, `substrate`)
  membership   this peer holds EX-DEMO-GEAR ✓
  listening    INFRA offers tcp/7447 on 3 of 3 nodes ✓
  verdict      NOT AN ACL PROBLEM. Look at DNS resolution or the peer's
               connection state.
```

Seconds, no warrant, nobody involved — and computable from the reach resolver
netvis already serves. **A request path whose first act is to rule itself out
removes most of the traffic before a human sees it.** When it *is* a gap, the
same command pre-fills the request.

### 4.3 Standing shapes — the openings nobody decides

A pre-approved *shape*, expressed where routing already lives: a `CODEOWNERS`
rule plus a CI check that a change matches the shape. When it does, CI
auto-approves.

```
# a GEAR gaining `fabric` to its own host-site INFRA is pre-authorized:
# CI matches the shape and approves; anything else waits for an owner.
```

Approved once, over a class, by the authority that would otherwise approve every
instance. **Novelty escalates; volume does not** — and it needs no engine,
because "a rule about which changes may merge unattended" is what CI and
CODEOWNERS already are.

### 4.4 Provisional openings — unblock now, decide after

`expires:` twenty-four hours out, in a merged PR. The site's own admin can merge
it within their tier; CI fails the day it lapses and the scheduled job opens the
removal PR.

That is the whole mechanism — **no state machine, no lifecycle service.** Risk
bounded by *time* rather than by ceremony, which is the opposite of the trade a
change board makes and is made deliberately: a system people route around
protects nothing.

### 4.5 Every warrant expires

One that cannot be narrowed today is still issuable, with an owner and a date.
On expiry it neither lapses nor renews automatically: it surfaces as expiring
**with the join already attached**, so narrow-or-extend is a two-minute read.
The same queue that opens paths is what keeps the ACL from accreting.

---

## 5. Two planes, which are two `CODEOWNERS` lines

Everything in §4 changes *warrants*. The things that govern warrants — the
vocabulary, the pre-approved shapes, who owns what — change a different way, and
giving them the same path would be this design's largest hole.

The separation is not a subsystem. It is which paths are owned by whom:

```
rule-intents/<scope>.yaml   @<scope>-admins     # instance plane — fast
port-sets.yaml              @user369x    # governance plane — reviewed
standing/                   @user369x    # governance plane — reviewed
```

**The instance plane is fast because the governance plane is slow**, and the
trade holds because they are different paths with different owners.

### 5.1 Why the vocabulary is guarded harder than the rules

**A pre-approved shape delegates issuing authority; a warrant only grants
access.** A warrant lets one class reach another until it expires. A shape lets
unbounded future changes merge unattended, indefinitely. If a scope admin could
edit `standing/`, they could promote themselves to auto-merge permanently.

Kubernetes RBAC calls the equivalent guard **escalation prevention** — you
cannot create a role carrying permissions you do not hold. Here it is a CI
check: a shape may not authorize reach broader than its owners hold.

And **vocabulary edits are reviewed in both directions**, the one place
narrowing-is-free does not apply. Narrowing one intent affects one pair;
narrowing `substrate` affects all sixteen rules referencing it. Blast radius,
not direction, is what makes vocabulary dangerous.

### 5.2 No emergency governance change

There is no expedited vocabulary edit and no emergency shape. The emergency path
is an intent with `expires:` 24 hours out, which lapses on its own — because if
"emergency" could reach the governance plane it would become the route by which
authority is permanently widened under time pressure, which is how every
exception process decays.

### 5.3 Governance has its own assertions

Checkable, and living in the same `netbird_acl_tests` mechanism
([ADR 1126](../adr/1126-acl-assertions-rechecked-when-nothing-is-written.md))
that guards the layer below: no shape may target `NETWORK` or `BREAKGLASS`,
carry `protocol: all`, or cross a scope; no port-set may be orphaned; every
`expires:` is bounded. A change breaching one cannot merge.

## 6. What mediates it — git, and four small things

### 6.1 The machinery already exists

| the need | what serves it | new code |
|---|---|---|
| a proposal | a **pull request** against the registry | none |
| routing to a decider | **`CODEOWNERS`** on the registry paths — fail-closed when no owner resolves | a CODEOWNERS file |
| the decision | a **review** | none |
| audit | **git history** — who, when, what, and the diff | none |
| the gate | **CI** — the assertions, the guards | one check |
| delivery | **fastlane** ([ADR 1127](../adr/1127-fastlane-urgent-fleet-wide-delivery.md)), ~30s to every goatnet | none |

So the build is **four things and no new services**:

1. **`port-sets.yaml` + generator** — the vocabulary, and byte-identical output.
2. **`goat warrant why-blocked`** — a read over the resolver netvis already
   serves, which returns a verdict and, when it is a real gap, the minimal
   port-set and the diff to put in a PR.
3. **`expires:` + `owner:` on a rule intent**, with a CI check that fails when
   one is past due and a scheduled job that opens the removal PR. That is the
   entire expiry mechanism — no store, no state machine.
4. **`CODEOWNERS`** over `port-sets.yaml` (platform admins),
   `rule-intents/<scope>.yaml` (that scope's admins) and `standing/` (platform
   admins only).

### 6.2 Narrowing merges unattended

The asymmetry that makes this fast is one CI check, not a queue: **the generator
can prove a change is a reduction** by diffing its own output. When it is, CI
auto-approves and the change merges without a human. Widening waits for its
owner.

### 6.3 What a change looks like end to end

```
$ goat warrant why-blocked GEAR → INFRA :7447
  ACL       DENIED — no enabled rule connects EX-DEMO-GEAR → ZAGROS-INFRA
  listening ZAGROS-INFRA offers tcp/443, tcp/7447, udp/7448 (3 of 3 reporting)
  minimal   port-set `fabric` covers the stated need
  → write this to rule-intents/ex-demo.yaml:

      - src: GEAR
        dst: HOST:INFRA
        port_set: fabric
        owner: kim@example.org
        expires: 2026-11-11

$ git commit && gh pr create      # CODEOWNERS routes it to ex-demo's admins
                                   # CI runs the assertions
                                   # merge → fastlane → live, ~30s
```

A provisional warrant is the same thing with `expires:` 24 hours out. A standing
template is a CODEOWNERS rule that auto-approves a matching shape. Neither needs
a subsystem.

### 6.4 The one thing git cannot serve

A forward-site admin on a tablet who cannot open a pull request. That is a real
person and a real need — and it is an **accessibility layer over this same
mechanism**, deferred until someone is actually blocked by it, not a precondition
for any of the above.

Because `CODEOWNERS` already routes to named people and is already fail-closed,
**the delegation this design exists to enable does not wait on the console.**

## 6A. The open-source seam — mechanism public, policy private

goat goes open (oss-strategy.md, repo split at
Block 29M): public upstream `goat-maintainer/goat`, private downstream overlay. Warrant
has to declare which side each of its files lives on, because **the governance
record is the most sensitive artifact this design creates.**

`git log -p rule-intents/ex-demo.yaml` is simultaneously a complete map of
what reaches what, and a list of who authorizes it. That is category 1
(deployment-specific) and category 3 (methodology that helps an attacker) in the
oss-strategy cleansing taxonomy, at once.

### 6A.1 The split

| public — `goat-maintainer/goat` | private overlay |
|---|---|
| the port-set **schema** and the generator | `rule-intents/<goatnet>/<scope>.yaml` — the actual grants |
| `goat warrant why-blocked` | `CODEOWNERS` entries naming real people |
| every CI check: byte-identical gate, narrowing-proof, expiry, governance assertions | deployment-specific assertions (*`KID → INFRA` must hold on ex-demo*) |
| the **reference vocabulary** — the seven port-set names | scope, site and class names |
| an example goatnet's intents, so `make demo` works | **the decision history for all of the above** |
| this doc, the ADR, the plan | |

**The seven port-set names ship public deliberately.** `shell` is tcp/22 and
`fabric` is a Zenoh router leg — that is generic infrastructure vocabulary, not
intelligence, and publishing it means an adopter gets a working default instead
of a blank file. It is the Move-1 `git clone && it runs` property the OSS
strategy asks for.

### 6A.2 Put the seam in before the split, not after

**History cannot be relocated after the fact, and history is the whole point of
the record.** If rule-intents accumulate in a substrate path and the split then
tries to separate them, it needs `filter-repo` surgery on precisely the file
whose history is load-bearing — and a leaked commit message is as exploitable as
a leaked working tree.

So policy files live in an **overlay-only path from their first commit**, even
while both sides still share one repo today. At split time that is a path that
simply does not travel, rather than an excision.

Two guards, both of which already exist in plan and need only to learn these
paths:

- `public-surface-guard.yml` (30D) — rejects deployment-specific tokens entering
  the public repo. Scope names, site names and owner identities go on its list.
- `overlay-content-guard.yml` (29M) — rejects overlay-only content appearing in
  substrate paths. A rule-intent authored into the public tree fails there.

### 6A.3 The generator takes policy as input

The seam is an argument, not a vendored directory:

```
goat-warrant generate --policy-dir <private>/rule-intents/zagros/ \
                      --port-sets <private>/port-sets.yaml
```

The public repo is runnable against **any** operator's policy, including an
adopter who has never seen a goatnet. That is the [ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md)
bar restated: the substrate works without the operator acquiring our
deployment's specifics.

A consequence worth stating: **an adopter's governance record is theirs.** The
public project ships the mechanism for a policy record; it never becomes a place
where someone else's network policy accumulates.

## 6B. The read-back — reconcile, and why write-only was the defect

Everything above describes one direction: registry → generated matrix → applied
policy. Until 2026-08-20 there was no other direction, and that turned out to
matter more than the elegance of this one.

**The only read-back that existed was self-referential.** The cutover playbook
computes orphans by filtering live policies whose `description` contains the tag
it stamps on its own output. So it can tell you which of *its own* policies the
registry has stopped declaring. It cannot see a policy it did not create — and a
policy created in the NetBird web console has an empty description, so it matches
no filter, appears in no report, is skipped by the orphan reaper, and survives
every deploy indefinitely.

Every check this repo had asks **"is what we declared applied correctly."** None
asked **"is what is applied something we declared."** That is a difference in
kind, not in coverage, and no amount of tightening the first question reaches the
second.

### What `reconcile.py` does

Reads the live policy set and classifies every row against the registry:

| bucket | meaning |
|---|---|
| `generated` | tagged by warrant **and** declared in the matrix — the healthy case |
| `orphaned` | tagged by warrant, no longer declared — the registry dropped the rule |
| `undeclared` | not tagged at all. **Nobody generated it.** The bucket that had no watcher |
| `default` | NetBird's built-in `Default` policy, reported by state, since disabled is the post-cutover expectation |

It **writes nothing**. Removing an undeclared policy is a human decision: some of
them are legitimate work that took the wrong road, and deleting those silently
teaches people that the console is where real work gets undone.

### Adoption — the way back in

`--adopt` prints each undeclared policy as a rule stanza shaped for
`rule-intents/`, with `description`, `justification` and `owner` left **empty and
required**. The person who made the change in the console says why; the stanza
goes through review like any other rule; the next generate makes it real.

The tool deliberately cannot fill the justification in. A machine-written reason
is precisely the thing review must not be handed — it gives a reviewer something
plausible to approve, which is worse than an empty field that stops them.

Two shapes it refuses to launder. A rule targeting a **peer id** is emitted
commented-out, because a peer id changes on re-enrolment and a rule pinned to one
cannot outlive it — the right adoption names the peer's class, and that is a
decision rather than a transform. A rule with **no protocol and no port limit** is
emitted with a note saying so, because narrowing it is the entire point of
bringing it back: go find what actually listens.

## 7. Prior art, and where this diverges

Almost nothing here is novel, which is the point — each mechanism has an
established name outside this repo, and using those names makes the design
reviewable by people who have never seen a goatnet.

| this design | established practice | taken / left |
|---|---|---|
| standing template | ITIL **standard change** — pre-authorized as a class, no per-instance board | take the model; leave the tooling — authorization lives in the registry as data |
| provisional warrant | **just-in-time access**, zero standing privilege — PIM activations, Vault leases | take time-bounding by construction; leave approval-to-activate, since a site admin acting in their own site is not escalation |
| narrowing is free | Netflix **Repokid** + Access Advisor — auto-removes unused IAM permissions | take the asymmetry; leave the automation — Repokid *acts*, here evidence only informs |
| the listening surface | **IAM Access Analyzer** policy generation; **Cilium/Calico audit mode** | take the intent, **leave the signal** — both read traffic, which a brokerless fabric cannot |
| expiry | **firewall rule recertification** — AlgoSec / Tufin, PCI's periodic review | take the goal; leave the cadence — periodic bulk review is why it gets skipped |
| `why-blocked` | AWS **Reachability Analyzer** / Network Access Analyzer | take it directly; ADR 1126 already cites NAA's missing `MustExistPaths` |
| escalation prevention | **Kubernetes RBAC** — no creating a role with permissions you lack | take it for standing templates, checked at generation |
| registry generates | **GitOps** | take it — already how this repo does hosted services and mesh DNS |

**Four deliberate divergences.**

*No change advisory board.* The industry default for network change is a review
board; it is rejected for the reason boards are usually adopted, because a queue
people route around protects nothing. Blast radius is bounded instead by the
port-set vocabulary and by expiry.

*No traffic-derived signal, permanently.* Every mature least-privilege tool above
reads flows. This one cannot and will not, and the consequence is stated rather
than hidden: **it can tell you nothing is listening on a granted port; it can
never tell you a granted port went unused.**

*No general policy engine.* OPA/Rego or a solver would be the conventional home
for rules about rules. ADR 1126 refused a solver — group-to-group reach with
ports is set intersection over bounded membership, not a routing fixed-point —
and ADR 1016's complexity budget makes a second policy language expensive. The
assertion block is the policy engine.

*Not per-request authorization.* BeyondCorp-style models authorize each request
at a proxy. This is an L3/L4 mesh ACL evaluated at enrollment and membership
time; a proxy in the path would re-introduce the broker the architecture
rejects.

---

## 8. Migration

Ordered so nothing reaches a live ACL until the generated output is proven
identical to what is already running.

| phase | what | gate to the next |
|---|---|---|
| 1 | write `port-sets.yaml` — the seven sets, extracted | they reproduce all 11 port-carrying combinations exactly |
| 2 | generate the rules; diff against the hand-written file | **byte-identical.** Not "equivalent" — identical, or phase 3 does not start |
| 3 | cut over; the registry becomes the source | CI green; a live re-read shows no policy change |
| 4 | listening-surface publisher on `INFRA`, then `POD` / `MOON` | a class reports, and an unreported class renders `unknown` |
| 5 | the join as a read-only surface, and `why-blocked` | a reviewer answers the cost question without writing a script |
| 6 | narrow the first `all` rules, evidence-led, one at a time | assertions still satisfied after each |
| 7 | request path, standing templates, provisional warrants, expiry | governance assertions (§5.3) green |
| 8 | gear and Jetson classes | last on purpose: hardest, highest consequence |

**Phase 2's byte-identical gate is the safety property of the whole migration.**
It means the cutover is structurally incapable of changing a goatnet, so any
later behaviour change is attributable to a deliberate narrowing rather than to
the refactor.

Risks worth naming: a subtly divergent generator (answered by phase 2); a class
that under-reports and licenses an unsafe narrowing (answered by `unknown`,
union-not-intersection, and the stated horizon); the registry drifting into
advisory (it generates, or it is F-324); and vocabulary re-fragmenting as
services register their own ports (answered by purpose-keyed sets and by nobody
being asked for a port list).

---

## 9. What this does not do

- **It does not observe Zenoh messages.** Which topic, which publisher, which
  payload stays outside its reach — that is the brokerless root, and the audit
  subscriber covers the governance-relevant part of it.
- **It does not decide.** The join informs; a human or a routed request approves.
  Nothing in the pipeline widens a grant automatically.
- **It does not replace the assertions.** ADR 1126 remains what proves the rules
  still mean what they are supposed to mean — including across membership
  changes, which write no file and so escape every write-time gate. On a goatnet governed by
  [ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md)
  membership does write a file (`ops/warrant/membership/`), and that gap closes there.
- **It does not cover `icmp`.** Twelve rule-shapes, no ports, correct as written.
- **It does not make governance fast.** That is the trade, not an omission.

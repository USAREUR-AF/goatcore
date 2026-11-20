# Enrolling gear — choosing the class, and what to do when the site is not ready

**Audience:** anyone who enrols a machine — a custodian doing it for their own
gear (the common case; function classes are self-service) or an operator doing
it on someone's behalf.

**Why this exists.** The decision record for the class vocabulary is
ADR 1116, and a decision record
is not a procedure. Until now the only operational guidance was the mint's own
refusal messages, which is learn-by-rejection: correct, but it teaches you the
answer only after you have guessed wrong. The second half of this page exists
because the failure it describes has happened twice on real goatnets.

Design canon stays machine-enrollment.md.
The RULE — the four doors, what counts as going around them, and what
break-glass costs — is
[onboarding-rails.md](onboarding-rails.md) §"Machines have their own rails".
This page is how to *do* it.

---

## 1. Choose the class

**A class is a FUNCTION, not a kind of box.** A machine holds one class per
function it performs — a box doing two jobs is enrolled twice, once per job.
This is the rule that stops a catch-all forming: `OPSYSTEM` became one because
"what is this box" has no policy answer, while "what does it do" does.

Work down these questions in order. The first one that fits is your class.

| Ask | If yes | Class |
|---|---|---|
| Does it front devices that do not speak the fabric — a gateway translating something else onto it? | | `KID` (`EXTERNAL-KID` when the far side is egress you do not control) |
| Does it emit sensor observations onto the fabric? | | `SENSOR-FEED` |
| Does it emit health/status about a sensor rather than the observations themselves? | | `SENSOR-TELEMETRY` |
| Does it arbitrate between sensor inputs — decide which track wins? | | `SENSOR-ARBITER` |
| Does it cue an effector? | | `EFFECTOR-CUE` |
| Does it carry effector track data? | | `EFFECTOR-TRACK` |
| Does it emit effector health/status? | | `EFFECTOR-TELEMETRY` |
| Does it arbitrate between effector inputs? | | `EFFECTOR-ARBITER` |
| Does it produce derived data from other people's data — fusion, analytics? | | `FUSION` |
| Is it a control station a person drives? | | `CONTROL` |
| Does it sit *between* two systems that speak to each other, owned by someone else — never originating and never terminating? | | `EXTERNAL-INTERMEDIARY` |
| Is it bearer gear — router, switch, radio? | | `NETWORK` (infra; operator, or a custodian holding the ADR 1113 network-admin entitlement for their own site) |
| Is it a moon-pod appliance inside our identity planes, with its own local router? | | `POD` |
| Is it the uplink peer of a foreign-stack goatnet? | | `MOON` |

`EXTERNAL-INTERMEDIARY` carries two properties and both are in the name
(ADR 1116 §5a): **EXTERNAL** — we do not own it, patch it, or control its
egress; **INTERMEDIARY** — it acts between things and is never a terminus.
That second half has an operational consequence worth knowing before you pick
it: when an intermediary is down, the systems on either side are still healthy
and simply cannot reach each other. That is a different failure signature from
a dead endpoint, and it should be read as one. Where the thing genuinely is a
broker, `broker` is carried as a **label** — a label is not a class.

**One box, several jobs → several enrolments.** A ground segment that both cues
an effector and arbitrates between effector inputs is a `PLATFORM` whose
`--workloads` declare both (`EFFECTOR-CUE`, `EFFECTOR-ARBITER`). A `PLATFORM` has no reach of its own; its
groups are the union of its declared workloads.

### POD and MOON appliances

`POD` and `MOON` are canonical node classes from ADR 1092. A pod is inside our
identity planes; a moon is the uplink peer of a foreign-stack goatnet. Enrol the
peer under the class that describes that boundary directly:

```bash
goat machine enroll <handle> --node-class POD \
  --autonomy remote --custodian <handle> [--exercise <site-scope>]
```

Use `--node-class MOON` for the foreign-stack uplink. The mint derives exactly
the matching scoped group (`POD` or `MOON`); do not add either group by hand.
Both are standalone trust-boundary classes, not composable functions: do not
declare `POD` or `MOON` in a `PLATFORM --workloads` list. The mint rejects that
shape as `invalid_workloads` rather than unioning either boundary group onto a
platform peer.
Neither class grants router peering. Peering into a pod's local router remains a
separate, audited `router-peer` capability under ADR 1087.

### Classes the mint will refuse, and what it means

These refusals are the vocabulary defending itself. Each names the right answer:

| You asked for | Refused because | Use instead |
|---|---|---|
| `SENSOR`, `EFFECTOR` | Reserved. They name the sensing or effecting element itself, which is not a mesh peer. | the function class — `SENSOR-FEED`, `EFFECTOR-CUE`, `KID` … |
| `SENSOR-TASKING`, `EFFECTOR-COMMAND` | Intake-only: they grant inbound permission and no outbound reach, so a peer holding one *alone* could not reach the router to receive the very tasking the class exists for. | the class carrying its outbound half; add the intake membership deliberately afterwards |
| `PROCESSOR` | Legacy whole-system class with no NetBird group — a peer enrolled as one holds nothing and is uncovered. | `FUSION` |
| `PLATFORM` with no `--workloads` | A composition that declares nothing composes nothing. | `--workloads <class,class>` |

Legacy whole-system classes stay valid to **read** — machines enrolled before
the gear vocabulary existed are not broken retroactively — but a machine
carrying one has no mesh derivation, so re-stamp it
(`PUT /api/slots/{anchor}/custody`) before re-minting its key.

### Two invariants worth knowing before you start

- **Affiliation is never a class and never a group.** Who a machine belongs to
  is identity, not reach.
- **The peer's mesh name must equal the machine handle.** That pair is the join
  key for the provenance audit *and* for `goat machine revoke` — a peer left on
  its appliance hostname reads as a violation and cannot be cut by the revoke
  path. Rename the peer, never allowlist it.

---

## 2. Gear has arrived and the site's groups do not exist yet

The mint **fails closed** here, on purpose: it refuses the whole enrolment
rather than mint a key carrying some of the derived groups, because an
under-enrolled peer looks configured while reaching nothing.

**The remediation has two halves, and doing only the first is the trap.**

### Half 1 — the groups must exist

```bash
cd ops/ansible
ansible-playbook -i localhost, --connection local \
  netbird-acl-bootstrap.yml -e target_site=<site>          # add --check to dry-run
```

Idempotent: it GETs what exists, POSTs only what is missing, never deletes or
renames. Requires `<SITE>_NB_API_TOKEN`.

### Half 2 — a policy has to USE them

**Creating the groups creates no policy.** A goatnet with the full taxonomy and
no matrix entry runs the Default `All→All`: every peer reaches everything, the
groups are labels, and the class you just chose grants nothing of its own. This
is F-283.

So also confirm:

1. the site has an entry in `ops/ansible/netbird_acl_matrix.yml` (alias the
   reference ruleset unless a deliberate delta is intended);
2. its permits have been **applied**, and the cutover from the Default has been
   fired — that step is deliberately operator-fired:
   review `netbird-acl-cutover.yml -e mode=propose` first, then
   `-e mode=enforce -e confirm_cutover=YES`. To undo, revert the rule PR and re-run `-e mode=apply` (ADR 1185; there is no rollback to a wide-open Default).

**`ops/ci/check-netbird-acl-parity.py` checks the FILES, so a green CI does not
mean the goatnet is segmented.** Read the live policy if you need to know, not
the repo.

> **Reading live NetBird policy: check `policy.enabled` FIRST.** A NetBird
> policy carries **two** enabled flags — one on the policy and one on each rule
> — and they can disagree. A goatnet's Default `All→All` can sit at
> `policy.enabled: false` with `rule.enabled: true`. Read the rule flag alone
> and you will conclude the goatnet is running flat when it is not. Anything
> auditing live policy has to look at the policy flag before the rule flag.

### If you cannot wait for half 2

Enrol anyway and know what you have: the machine gets its fabric identity, its
custody edge and a key carrying the right groups, and it reaches everything
because the mesh is still flat. That is a *recorded* state, not a hidden one —
the provenance guard will show the peer as `aligned` (it came through the mint)
while the mesh is undifferentiated. Do not compensate by hand-picking wider
groups on the key; that is the side door the mint exists to close.

---

## 3. Enrol

```bash
# operator, once per custodian — the standing allowance
goat machine grant <custodian> --quota 5 --affiliation <unit>

# the enrolment
goat machine enroll <handle> --node-class <CLASS> --autonomy <manual|remote|supervised|autonomous> \
  [--custodian <handle>] [--workloads a,b] [--exercise <name>]
```

You get back the machine's own slot anchor, its cert bundle URL, its API key,
the derived `mesh_groups` and a **one-off, single-use** setup key with a 30-day
join window. Deliver the tarball out of band; the box joins with
`netbird up --setup-key <key>`.

**Name the peer the machine handle.** See the invariant above.

### The allowance line is the one that stops people

The two commands above are one line each, but the first is an operator act with
a credential the operator probably does not have loaded — and every failure it
produces is a 403 or a 401 that names three possible causes without saying
which. Field account: enrolling a robot cost four refusals, none of
them the class vocabulary, all of them this line.

**The grant needs the operator's `op_` key, not the `vp_` one from their
bundle.** `goat profile init` installs the participant token; operator-gated
calls do not accept it. This is already canon in
goat-cli.md § Which token — *"using the participant
token against an operator endpoint answers 403, and that is the single most
common cause of one"* — but a custodian reading this SOP never gets there,
because the failure lands on the operator they asked, not on them.

```text
403  setting a machine quota needs your OPERATOR key (op_…), and this profile
     is using a different token.
```

The key is collected once, self-service, from the identity console's
`/operator-key` (an admin approves; `goat admin operator-key request` /
`collect` is the same door from the CLI). It is never inside a bundle. Point
the grant at it per goat-cli.md's paired-context convention, or inline:

```bash
GOAT_PORTAL_TOKEN=op_… goat machine grant <custodian> --quota 5 --affiliation <unit>
```

**`operator-<hex>` is an identifier, not a token.** An operator principal ID
looks enough like a credential to be pasted as one, and the portal answers:

```text
401  E_TOKEN_INVALID — token expired or revoked; re-onboard
```

Nothing is expired. The token starts `op_`; anything else is the wrong string.

**Grant against the custodian the portal actually keys, which may not be the
handle you know them by.** The custodian is authoritative in the quota view,
and a roster can hold several similar handles for one person — a grant that
lands on the wrong one succeeds and changes nothing for the person waiting.
Have the custodian read their own:

```bash
curl -s -H "Authorization: Bearer <their vp_ token>" "$PORTAL/api/machines"
# {"machines":[],"quota":{"custodian":"<this string>","quota":0,"used":0,"remaining":0}}
```

That is also how they confirm the grant landed: `remaining` non-zero. Until it
is, their enrolment answers

```text
403  no enrollment grant for this custodian, an infrastructure node-class needs
     an operator token, or a non-operator named a custodian other than itself
```

which is three causes in one body, and for a custodian with a fresh identity it
is always the first. A quota of zero is the documented starting state, not a
fault — machine-enrollment.md § Managing the quota
already records that a zero start "bought a round-trip, not a control" for admin
tiers. It buys the same round-trip for a vendor custodian, who additionally
cannot raise their own ceiling by design. **Ask for the allowance when the gear
is ordered, not when it is on the bench.**

Quota is a ceiling, not a budget: `revoke` returns the unit, and an unused
allowance costs nothing. Granting 5 or 10 to somebody who needs one is how you
stop having this conversation twice.

### The other doors, all through the same derivation

| Need | Door |
|---|---|
| the join window lapsed before the box joined | `goat machine mesh-key <handle>` — quota-neutral re-mint from the born-as class |
| N machines, one custodian, one act | `goat machine enroll --bulk <file>` — N one-off keys, never one reusable key |
| exercise-scoped gear | `--exercise <name>` — scopes the KEY, never the identity |
| the box is offline at issuance | `bundle-create --from-machine <anchor>` |
| retire it | `goat machine revoke <handle>` — cuts the peer and any unconsumed key, then frees the slot |

`goat machine revoke` needs goat-cli ≥ 0.4.40 **and** a portal carrying the
endpoint; a portal that predates it answers 405 and the CLI says so.

### Many machines with their topics, in one command

A bulk file can declare each machine's topics. After the mint, each new
machine registers its own topics with its own new key, in the same command,
so the catalog exists before anyone sets up a context for the machine. A key
still acts only for its own slot. Needs goat-cli ≥ 0.5.20.

```yaml
- machine_handle: moth-01
  node_class: KID
  autonomy: manual
  topics:
    - key: moth/cot            # relative to the machine's slot
      format: xml:event
      description: Cursor on Target events
- machine_handle: moth-02
  node_class: KID
  autonomy: manual
  topics:
    - key: moth/cot
      format: xml:event
```

`goat machine enroll --bulk machines.yaml` prints one line per topic:
`created`, `exists` (already registered, left as is), `skipped` (its machine
was not minted) or `failed`. With `--json` the outcomes are under `topics`.
The command exits non-zero when any topic failed; the machines stay enrolled,
and the rest are registered from each machine's own context.

---

## 4. Check your work

From a host that can reach the goatnet's portal — the operator laptop usually
cannot, and the guard grades `unknown` rather than pretending:

```bash
NB_API_URL=… NB_API_TOKEN=… PORTAL_URL=… PORTAL_OPERATOR_KEY=… \
  PROVENANCE_ENFORCE=1 ops/ci/check-node-class-membership.sh
```

Every gear peer grades `aligned` / `pending-reconcile` / `violation` /
`unknown`. A `violation` when a registry handle matches no peer is almost
always a renamed peer, and the guard says so — rename it rather than allowlist
it.

## Sources

- ADR 1116 — the class vocabulary and why `OPSYSTEM` was retired rather than renamed
- machine-enrollment.md — design canon, including the mint's mesh half and the revoke exit
- F-283 — groups without matrix policies ship flat
- goatnet-turnkey-standup.md — the standup half of the same trap
- goat-cli.md § Which token — `vp_` vs `op_`, the paired-context convention, and why a 403 is usually the wrong token
- [onboarding-notifications.md](../design/onboarding-notifications.md) — where the `op_` key is requested and collected

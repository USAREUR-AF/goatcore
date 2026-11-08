# Tutorial 3 — A boundary node for ally and partner sharing

You will stand up a **boundary node**: a dedicated router that lets your goatnet
release chosen data to an ally or partner who runs their own goatnet, under
their own authority, without either side joining the other's network. You
declare the peering, exchange packages with the counterparty through the
peering directory, and converge the edge, ending at an acceptance gate that proves data crosses in the agreed
direction and nothing else does. You rehearse the plan on your laptop first;
the live run needs the counterparty, so it is the one tutorial you cannot finish
alone.

| | |
|---|---|
| Audience | A platform admin who has finished tutorial 2, or who runs an existing goatnet, and has a counterparty ready to peer |
| Time | The rehearsal: a few minutes. The real run: days, most of it waiting on the counterparty to publish their package |
| You need | A goatnet whose gate is green, the `goatbuilder` binary, an edge host, and a counterparty running their own goatnet |
| You finish with | One peering live: your goatnet reads the partner's released topics and the partner reads yours, each refused everything else, proven by a round trip |
| Prior | [Tutorial 2 — the first federated node](02-first-federated-node.md) |

A joiner (tutorial 2) is a second site of **your own** goatnet: it receives your
account by replication and you vouch for its identities. A boundary node is the
opposite act. The party on the far side owns their goatnet, their certificate
authority and their identities; you recognise them, you do not vouch for them,
and no account replicates across. The two goatnets stay separate and exchange
only what each deliberately releases.

The supported path is `goatbuilder federate`. The script it stands beside,
`ops/scripts/peering-standup.sh`, runs the same fourteen phases and stays
documented; each step below names it. Partners are onboarded with this same
tooling and the onboarding guide, never with a one-off script
([partner-onboarding.md](../design/partner-onboarding.md)).

## The words used here

- **sovereign** — a party that runs its own goatnet under its own authority: its
  own certificate authority, management plane and identity domain. You are one
  sovereign; the counterparty is another.
- **boundary node** — one dedicated router process per partner, run on an edge
  host, that terminates the peering. It is deny-by-default and trusts only that
  partner's trust bundle. It is never your core router with the partner's trust
  added: a second certificate authority in the core router would let it mint any
  name your own services use, which is an authorization grant, not recognition
  (F-424).
- **peering** — the agreement and the link between two sovereigns, named after
  the counterparty. Everything about it is symmetric: each side does the same
  four things to the other — a reachability tunnel, a trust-bundle exchange, an
  agreed certificate name, and one access rule scoped to the release prefix.
- **release namespace** — the keys a sovereign deliberately shares. Data crosses
  only under these; a partner writes only under `from/<their-own-code>/` and
  reads only your declared release prefixes, enforced by the edge router's
  access rule (cross-sovereign-federation.md §5.6).
- **grant encryption** — the per-reader control. A message meant for certain
  readers is sealed to them when it is published, so a copy forwarded or stored
  by mistake can be opened only by the readers it named
  ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md),
  grant-keyed-encryption.md). The router's
  access rules are static and never carry a read grant.
- **phase** — one numbered, re-runnable step of the peering. A failed phase is
  fixed and resumed by number; phases 5 and 10 are manual, the exchange and the
  review of what was rendered, and each stops the run on purpose.

## Recognition is not access

Holding a partner's trust bundle grants them nothing. Access is three steps, each
a separate, deliberate act, and each with its own control that holds a partner
there until the next act is taken (cross-sovereign-federation.md §4):

| Level | What the partner can do | What stops them going further |
|---|---|---|
| **Recognised** | Nothing. You hold their bundle; no link exists. | No tunnel is up, and a CI guard keeps their bundle off every core host. |
| **Connected** | Complete a TLS session to your edge router. | No access rule names their certificate, and the edge default is deny. |
| **Exchanging** | Write under `from/<their-code>/`, read your declared release prefixes. | One access rule naming their agreed certificate, scoped to that prefix and nothing else. |

`goatbuilder federate` walks a peering from recognised to exchanging. Each phase
is one of those deliberate acts, and the last phase proves the partner reached
exactly "exchanging" and no further.

## Rehearse it on the fixture

You cannot rehearse a whole peering on a laptop — the far side is another
sovereign's goatnet. What you rehearse is the **plan**: `federate --dry-run`
prints every phase and the exact command it will run, and touches nothing. Read
it before you have a counterparty, so the live run holds no surprises.

The worked example declares the peering for you. The fixture
`ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node` is the goatnet from
tutorial 2 with one `peerings:` entry added — `hello-ally`, on the `edge` class
at `alpha`. `--mock` points `federate` at that flat declaration (the same
`--mock` tutorials 1 and 2 use for `standup`), so the plan is the one your
declared peering produces. A peering you have not declared is refused before any
phase runs — the "declare it first" check.

The agreement that phase 6 composes decides the transport, and phase 11 reads
that choice. So the dry run can show the transport phase the way a real plan
does, place a one-line stand-in for the agreement first — a real run writes this
file itself in phase 6.

```bash
WORK=/tmp/goat-peering-rehearsal
rm -rf "$WORK"; mkdir -p "$WORK"
printf '{"transport":{"kind":"ipsec-fips"}}' > "$WORK/agreement.json"
GOAT_BIN=goat goatbuilder -C . --mock ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node \
  federate hello-ally --as zagros --work "$WORK" --dry-run
```

**Checkpoint 1.** Your output matches
`ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/03-boundary-node.txt` line for
line. Read it before moving on:

- The fourteen phases run in the staged-delivery order: a fail-closed preflight
  (1), the federation interface and the bundle prepared so they can be measured
  (2), your half of the package read from your running system (3) and linted (4),
  the exchange through the peering directory and the key read-back (5), one agreement composed from both packages with no
  perspective (6), verified from your side (7), the peering certificates minted
  from it (8), the access rules and configuration rendered (9, the only writer),
  the manual review that admits the edge on your core (10), the agreed transport
  opened (11), the edge router deployed (12), the SPIRE trust domains federated
  (13), and acceptance (14).
- Phases 5 and 10 are `MANUAL`. The plan names them and, on a real run, the verb
  stops at each: phase 5 exchanges packages through the directory, you and the
  counterparty read your keys to each other and resume with `--from 6`, then review what was rendered and resume with `--from 11`.
- Phase 11 opens the transport the **agreement** chose, read from the agreement
  itself. Here that is `ipsec-fips`, so it runs `peering-edge.yml`; `wireguard`
  runs `peering-wg.yml`. If the two
  parties' packages do not intersect on a transport this repository can stand
  up, phase 11 refuses rather than substituting the other one — a tunnel that
  does not match the agreement is exactly the drift the flow exists to prevent
  (F-495).
- `--as zagros` names which sovereign you are. It is stamped onto the compose,
  verify, mint and apply commands so every artifact records the party that made
  it; a real run reads it from the declaration when you omit the flag.

## Before the real run

Everything the driver cannot create for you. The canonical procedure behind it
is the sovereign-federation runbook
(`docs/operations/sovereign-federation-runbook.md`).

1. **A counterparty running their own goatnet**, and an agreement on what data
   flows each way. The peering's names, prefixes, transport and certificate names
   are derived from the two packages, never agreed by hand. Without the far side
   there is no peering to complete; this is why the tutorial stops at the plan on
   a laptop.
2. **An edge host**, a peer of your own goatnet's mesh in node class `PEERING-EDGE` (it reaches
   only your core router's fabric port and mesh DNS; ADR 1219), in an ansible group
   named by `--edge-group` (default `<peering>_edge`). The boundary router runs
   here, not on a core host. Declare the group in
   `ops/ansible/inventory/peering-edges.ini` in your goatnet's own repository;
   goatcore ships none, because the hosts are yours. One group per peering,
   listed under `[peering_edges:children]`:

   ```ini
   [peering_edges:children]
   <peering>_edge

   [<peering>_edge]
   <edge-host> ansible_host=<address on the peering transport or the mesh> ansible_user=ubuntu
   ```
3. **The peering declared**, when you keep peerings in `goatbuilder.yaml`: a row
   under `peerings:` for this counterparty on this environment. With any peering
   declared, the one you name must be among them, so a typo is refused before
   anything runs.
4. **A peering directory both sides reach** (saltlick), named by
   `FED_DIRECTORY_URL`, with `FED_COUNTERPARTY_ID` and `FED_SIGNING_KEY`; phase 1
   refuses without them. Phase 5 uploads your package and CA, fetches theirs, and
   prints both verifying keys and CA fingerprints. You read yours to each other on
   one call and record both keys, one per line: a key taken from the package it
   verifies proves nothing, and phase 6 refuses to compose for any party whose key
   is not bound.
5. **A FIPS-mode fabric image on both edge routers**, which belongs in the
   agreement: the data's cryptography is carried by the fabric TLS session, not
   by the tunnel (cross-sovereign-federation.md §5.1).
   A partner who does not run one gives a one-sided claim, which is a decision to
   record.

## Run it on your hardware

The same verb, against your peering. `<peering>` is the counterparty's name;
`<ours>` is your sovereign code.

```text
goatbuilder federate <peering> --env <env> --as <ours> --dry-run
```

The plan is the command reference; there is no second list to drift. When it
reads right, run it for real up to the manual phase:

```text
ops/scripts/with-secrets \
  goatbuilder federate <peering> --env <env> --as <ours>
```

The run stops at **phase 5**, after the directory step has uploaded your package
and CA and fetched theirs. If their entry is not there yet, run phase 5 again
once they have run theirs. Read your verifying key and CA fingerprint to each
other on one call, record both keys where the plan names, then resume:

```text
ops/scripts/with-secrets \
  goatbuilder federate <peering> --env <env> --as <ours> --from 6
```

The run stops again at **phase 10**, because only you may change your own core.
Review what `apply` rendered, check their CA against the fingerprint it printed,
splice the access fragment into your core and give the core the certificate phase
8 minted, then resume with `--from 11`.

A phase that fails says which phase, what state the peering is in, and the resume
command. Every phase is idempotent: fix the cause, resume from that number. The
verb's exit codes are specific — 20 a composition refused, 21 a lint refused, 22
a diff refused, 23 a value that could not be derived.

**The scripts avenue.** The same run, unsupported but documented:
`ops/scripts/peering-standup.sh <peering> --plan`, then without `--plan`, and
`--from <phase>` to resume.

## Done means proven, not deployed

**Checkpoint 2.** Phase 14 is the acceptance gate, and it is the only check you
can make from your own side alone. It proves three things, in the words the gate
uses — **carries, refuses, and round-trips**:

- **Carries.** A sample you publish on your egress lane (`from/<ours>/**`)
  reaches the partner, and a sample they publish on theirs reaches you.
- **Refuses.** A write outside the agreed prefix is denied. Denial is silent on
  the fabric (F-177),
  so this is checked by live fire, never by reading the configuration.
- **Round-trips.** You emit a nonce under your wire name; a responder on the
  partner's goatnet answers under theirs; you match the nonce and the payload
  digest. It belongs last because it is the only stage that fails when an earlier
  one silently half-works — a peering can be established, mutually authenticated,
  and carrying nothing, with nothing logged at either end.

The access decision itself is a reviewable record: one file per partner under
`ops/federation/partners/` in your goatnet's own repository, changed by pull
request and gated by `ops/federation/validate-partners.py`. goatcore ships one
file there, `EXAMPLE-hello-ally.yaml`, as the shape to copy; the fields are
explained in `ops/federation/README.md`. Adding the file grants nothing; access
follows only when the edge runs with it.

## Undo

Cutting a peering is local, immediate and unilateral — you do it on your own edge
host, and it severs established sessions. On the IPsec edge `goatbuilder federate`
builds, remove the partner's certificate-authority file, reload, and terminate
the session; dropping the authority file alone does not tear down a session
already up. The partner's authority file is the revocation instrument: because it
is one per peering, rotating it retires that partner's certificate exactly and
nothing else (cross-sovereign-federation.md §7).

## What you have now, and what is still ahead

One peering, carrying released data in both directions under the agreed prefixes
and nothing else. This tutorial is the light, in-builder rehearsal of the
`federate` flow; to run cross-environment federation end to end against released
artefacts — a second goatnet, real package exchange, a live acceptance gate —
the standalone repository `hello-galaxy-cross-env-federation` does the
same flow in full. The deeper material lives in the canon this tutorial wraps:
cross-sovereign-federation.md for the
trust model and the edge profile, [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md)
for the one-forwarding-router-per-sovereign-root decision, and
[ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)
for release namespaces and grant encryption.

**Status, so the plan holds no surprises.** The flow has run through this driver on two real hosts
across the internet, both sovereigns ours, 13 of 13 acceptance checks from each side
(M-125). It has not yet run between two goatnets or with a real counterparty. Grant encryption
across a federated boundary is tracked under F-481.
To run both sides of a peering yourself, follow [tutorial 4](04-two-sovereigns.md).

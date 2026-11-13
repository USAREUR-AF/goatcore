# Tutorial 2 — The first federated node

You will attach a second site to the goatnet from tutorial 1. The new site is a
**joiner**: it does not originate an account, a certificate authority or an
access policy; it receives the founder's by replication and serves the same
goatnet from a second place. Its standup also changes the founder, once, so
replication runs in both directions. You rehearse it on the worked example,
starting exactly where tutorial 1 ended, then run it on your hardware.

| | |
|---|---|
| Audience | A platform admin who has finished tutorial 1, or who runs an existing goatnet |
| Time | The rehearsal: ten minutes. The real run: a working day the first time |
| You need | A founder site whose acceptance gate is green, the `goatbuilder` binary, and the joiner's hardware |
| You finish with | Two sites serving one goatnet, replication proven in both directions, every record verified |
| Next | Tutorial 3, the boundary node for ally and partner sharing (sequenced; see the tutorial program) |

A goatnet federating with **another** goatnet, owned by someone else, is a
different act with its own trust model; that is tutorial 3's ground, not this
one. Here both sites belong to the same goatnet and the same owner.

## The three things that make a joiner different

1. **It declares itself a joiner.** Its site configuration says
   `goatnet_founder: false` and names the founder it attaches to
   (`join_founder_site`). The driver reads both from configuration, never from
   the command line: declaring it wrong seeds a second account into a
   replicated database.
2. **Its management stack runs in three passes.** Pass 1 brings the stack up
   without seeding an account. Pass 2 copies identity and mesh state from the
   founder. Pass 3 turns replication on. The passes cannot be reordered.
3. **It changes the founder.** The founder-side mirror (phase 6) adds the
   founder's half of the replication subscriptions. Without it, writes flow one
   way only.
4. **Its signal federates with the founder's.** A peer holds its signalling
   stream on one signal server; the two forward offers to each other over the
   mesh, with a relay-tunneled backup. Both sides are turned on in the same
   change, after replication
   ([signal-federation-pair.md](../design/signal-federation-pair.md)).

## Rehearse it on the fixture

The worked example's second step declares a joiner site, `beta`, attached to
`alpha`. Its records start where tutorial 1 ended: the founder's four machines
already carry their converged records, and publication serials continue from 6.
From a clone of `USAREUR-AF/hello-galaxy-goatenv`, drop the
`ops/goatbuilder/fixtures/goatnet-from-scratch/` prefix from the paths below.

First, see exactly what tutorial 2 declares, as a difference from tutorial 1:

```bash
diff ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder/goatbuilder.yaml \
     ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node/goatbuilder.yaml || true
```

Then stand the joiner up and read the whole goatnet's state:

```bash
WORK="$(mktemp -d)/zagros"
cp -R ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node "$WORK"
goatbuilder standup zagros --site beta --mock "$WORK" --ack "A Platform Admin" --yes
goatbuilder status --env zagros --mock "$WORK"
goatbuilder assay zagros --values ops/goatbuilder/fixtures/goatnet-from-scratch/02-first-federated-node/assay-values.yaml --plan
```

**Checkpoint 1.** Your output matches
`ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/02-first-federated-node.txt`
apart from the milliseconds. Read it for three things:

- The difference adds one site (`role: joiner`, `joins: alpha`) and two
  components. That is the whole declaration.
- Phase 6 runs twice: once on the joiner (`[core] spock-subscriptions`) and once
  on the **founder's** machines (`[alpha/core] spock-mesh-wire`). The mirror is
  recorded where it happens, on `alpha-core-1` and `alpha-core-2`.
- `status` covers both sites: the founder's five components carried from
  tutorial 1 and the joiner's new ones, all converged. A joiner that broke the
  founder would show here.
- `assay --plan` for the joiner adds the federation layer: replication running
  both ways, the founder's rows present on the joiner, a sample published at
  the joiner received through the founder's router, and the one check that must
  be refused, site-local tables staying out of the replicated set.

The rehearsal proves the order of the phases, that the founder is touched, and
that every record is verified. It does not replicate a database; that is what
checkpoint 2 on hardware proves.

## Before the real run

1. **The joiner's declaration.** Its `group_vars/<joiner-site>_hosts/vars.yml`
   with `goatnet_founder: false` and `join_founder_site: <founder-site>`.
2. **The founder names the joiner.** The founder's configuration must list the
   joiner as a replication peer before the mirror runs. The driver cannot
   supply this; the configuration must. Both sites' groups are children of
   `[mgmt_hosts:children]` in `inventory/mgmt-hosts.ini`: replication finds its
   peers by walking that group, and the config preflight fails, naming the line
   to add, when a site running the management stack is missing from it.
3. **The pins.** The joiner's member group added to the goatnet's row in
   `ops/pins/fleet-pins.yaml`, with `absent: true` for components that are
   single-homed on the founder. The admin write components (portal, id console,
   onboarding runner) are single-homed only until
   [ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md) lifts
   their write class; their intended placement is every site.
4. **Signal federation, declared on both sides.** Each site's group_vars name
   the other's mesh address and federation hostname, the shared token by
   reference, the proxy and the ingress
   ([signal-federation-pair.md](../design/signal-federation-pair.md) §"What a
   federating pair declares"); the founder's relay inventory
   `inventory/<founder>-goat-relay-alias.ini` pins each federation hostname to
   its own node; the hostnames have A records over the relays, applied. Check
   with `ops/scripts/signal-federation-check.py <joiner-site> --resolve`. A
   joiner that stays off declares `signal_federation_absent_reason` instead.
5. **Reach.** The joiner's machines reachable for the first key seeding, and
   your laptop able to reach both sites: on the goatnet's mesh, or through its
   door (`join-node --via door`, below).

## Run it on your hardware

```text
goatbuilder doctor --env <env> --credentials
goatbuilder standup <env> --site <joiner-site> --dry-run
```

The plan shows the joiner's passes and the founder-side mirror by name. When it
reads right:

```text
ops/scripts/with-secrets \
  goatbuilder standup <env> --site <joiner-site> --ack "<your name>" --yes
```

Resume after a manual phase or a fixed failure with `--from <phase>`, as in
tutorial 1.

**Each further machine at a site** joins with its own verb, which runs the
node-addition sequence (reach, key seeding, the narrowed baseline, the class's
converge) in numbered phases:

```text
goatbuilder join-node <env> --site <joiner-site> --node <node-name> --class core --dry-run
goatbuilder join-node <env> --site <joiner-site> --node <node-name> --class core \
  --address <its LAN address> --yes
```

A laptop that is not on the goatnet's mesh adds `--via door`. Every play then
takes `inventory/mgmt-hosts.ini` with `inventory/doors-groupvars.ini`, never the
mesh inventory as well, and the plan's first line opens the door
(`ops/scripts/door-open.sh <env>`); a run opens it before phase 1.

Phase 2 seeds the SSH key only when the box needs it: it first tries
`ssh -o BatchMode=yes <user>@<address> sudo -n true`, and a box installed
unattended with your key and passwordless sudo passes that and skips the seed.
Phase 6, the first WireGuard pass, is expected to fail on one check before
enrolment, the one that looks for `wt0`; any other failure stops the run.

### What join-node runs for a joiner's core machine

Phase 13 is the joiner's three passes, the founder-side mirror and signal
federation, in this order. Each step depends on the one before.

| Step | Play | What it does |
|---|---|---|
| Pass 1 | `site-bootstrap-for.yml -e bootstrap_site=<joiner-site>` (or the site's own `<site>-bootstrap.yml`) with `participates_in_spock=false` | Brings the stack up with no account. Postgres initialises with the founder's cluster encoding, read from the founder; an existing cluster with a different encoding is refused |
| Ory | `ory-deploy.yml` | Creates the identity schemas the seed loads into |
| Pass 2 | `site-seed-from-peer.yml` | Installs the hub directive ledger on both ends, then copies identity and mesh data from the founder. Refuses an account that is already populated |
| Pass 3 | the same bootstrap play with `ory_spock_wire_subscriptions=false` | Turns on mesh replication both ways; for identity, creates the joiner's node and replication set only |
| Mirror | `spock-mesh-wire.yml --limit <founder group>` with `inter_site_spock_peers=[<joiner-site>]`, `ory_spock_reconcile_history=false` | The founder's half: its node, replication set and subscription to the joiner. No history reconcile yet |
| Rewire | `spock-mesh-wire.yml --limit <node>` | The joiner's identity subscription, then the history reconcile, which copies history both ways now that both halves exist |
| Federation gate | `ops/scripts/signal-federation-check.py <joiner-site> --peer <founder-site> --resolve --require-env` | Refuses a half-configured federation: one side on, a proxy with no peer address, an ingress without a token, crossed names, a missing relay pin or DNS record, the token not in the environment. Off with a declared reason passes, and the steps below are skipped |
| Federation, joiner | `signal-federation-transport.yml -e federation_site=<joiner-site>` | The token and the dual-path proxy; proves the running signal enforces the token |
| Federation, founder | the same with `-e federation_site=<founder-site>` | The founder's half, in the same run |
| Routers | `traefik-acme.yml`, joiner group then founder group | Each node's federation hostname router, and its own certificate served for that name (a stale DNS-01 record is cleared and Traefik restarted once before the step fails) |
| Tunnel | the founder's `<founder>-rathole-client.yml` (else `site-rathole-client.yml`) on the joiner group, with `inventory/hosts-<founder>.ini` | The joiner's tunnel to every founder relay |
| Relay pin | `relay-ingress-add-site.yml` with the founder's relay inventory and alias file | The joiner's backend and its federation-hostname pin on every relay; every dispatcher backend UP, a DOWN one's tunnel client restarted once before the step fails |
| Federation proof | `signal-federation-verify.yml -e federation_site=<joiner-site> -e federation_peer_site=<founder-site>` | Transport up from both nodes; an offer forwarded and delivered in at least one direction; a probe that could not run is named and fails the step |

`site_id` reaches every pass as a play variable, set by the wrapper. Never pass
`-e site_id=...`: an extra variable sets the site on every host in the
inventory, the peer walk then finds no other site, and a joiner that would wire
zero peers is refused rather than reported as passing.

An obs machine at a joiner site runs the baseline, LUKS and the observability
stack, and skips the utility leg (Forgejo, the pub/sub state-of-record
primary): the state of record stays with the founder, and the plan says so.

**A renamed machine keeps its backups.** Each machine's pgBackRest repository
passphrase is derived from the site's backup key and a per-machine seed. The
seed is the machine's inventory name the first time the backup play runs, and
it is recorded on the machine in `/etc/pgbackrest/cipher-seed` (root only; it
holds a name, not the key). Later runs read the recorded name, so renaming the
machine in the inventory leaves the passphrase, and the repository, as they
were. To re-key a machine on purpose, remove that file together with the
repository it keys.

**The scripts avenue.** `ops/scripts/goat-standup.sh <joiner-site> --plan`, then
without `--plan`. For each further machine, the `join-node --dry-run` plan above
is the by-hand sequence: every phase prints the command it runs and the gate
that proves it, so the same steps can be run one at a time.

## Done means proven, not deployed

**Checkpoint 2.** `goatbuilder assay <joiner-site>` is green for the joiner,
with the fresh-peer rows run from the joiner's network and a goat context for
each site on the machine that runs it (`goat_context` for the joiner,
`goat_context_peer` for the founder). **Checkpoint 3**, the federation
evidence: every replication subscription reports `replicating` in both
directions; a write on the founder appears in the joiner's database; the
same deletion replicates back; and `signal-federation-verify.yml` passes, naming
which directions a forward proved (a direction whose target peer is homed on
the sender's own signal reports UNPROVEN until peers home on the joiner). **Checkpoint 4:**

```text
goatbuilder status --env <env>
```

Both sites' machines name their declared refs, the founder's included.

## Undo

Re-run from the failed phase. Federation is turned off on both sides at
once (`federation_peers_for_a: ""` on each, then the transport play or
`mgmt-host.yml` on each node); one side alone leaves offers to the other's peers
undeliverable. Removing a joiner reverses the mirror on the
founder in the same change that removes the joiner from the pins and the
founder's peer list; never remove the joiner's machines while the founder still
subscribes to them.

## What you have now

One goatnet served from two places. Tutorial 3 adds a boundary node, which lets
the goatnet release data to an ally or partner who runs their own goatnet.

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
   supply this; the configuration must.
3. **The pins.** The joiner's member group added to the goatnet's row in
   `ops/pins/fleet-pins.yaml`, with `absent: true` for components that stay
   single-homed on the founder.
4. **Reach.** The joiner's machines reachable for the first key seeding, and
   your laptop able to reach both sites: on the goatnet's mesh, or through its
   door.

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
directions; a write on the founder appears in the joiner's database; and the
same deletion replicates back. **Checkpoint 4:**

```text
goatbuilder status --env <env>
```

Both sites' machines name their declared refs, the founder's included.

## Undo

Re-run from the failed phase. Removing a joiner reverses the mirror on the
founder in the same change that removes the joiner from the pins and the
founder's peer list; never remove the joiner's machines while the founder still
subscribes to them.

## What you have now

One goatnet served from two places. Tutorial 3 adds a boundary node, which lets
the goatnet release data to an ally or partner who runs their own goatnet.

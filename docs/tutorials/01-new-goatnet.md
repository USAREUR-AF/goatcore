# Tutorial 1 — A new goatnet, from nothing

You will stand up a goatnet's first site, its **founder**: the management stack,
the router, the observability host if you have one, and the rescue node, in the
order the doctrine fixes, ending at an acceptance gate that says green. You
rehearse the whole run on a worked example first, in under a minute and on your
laptop; then you run the same verbs against your own hardware.

| | |
|---|---|
| Audience | A platform admin standing up a goatnet for the first time |
| Time | The rehearsal: ten minutes. The real run: a working day the first time; the target is one hour |
| You need | A checkout of the goat repository on `main`, the `goatbuilder` binary (step 1), and for the real run the hardware and accounts in "Before the real run" |
| You finish with | A founder site whose acceptance gate is green and whose every component is proven from the hosts' own records |
| Next | [Tutorial 2 — the first federated node](02-first-federated-node.md) |

The supported path is `goatbuilder`. The script it stands beside,
`ops/scripts/goat-standup.sh`, runs the same phases and stays documented; each
step below names it.

## The words used here

- **goatnet** — one environment: its own mesh, identity, certificate authority and data fabric.
- **site** — one place's machines inside a goatnet. The first site is the **founder**: it originates the account, the certificate authority and the access policy. Every later site is a **joiner** (tutorial 2).
- **class** — what a machine is for: `core` (the management stack), `obs` (observability), `breakglass` (the rescue node).
- **phase** — one numbered, re-runnable step of the standup. A failed phase is fixed and resumed by number.
- **record** — the line each machine writes after it converges, naming exactly what it runs. "Done" means every record names what was declared, never that a play finished.

## 1. Get the binary

Download it from the release page, or from `get.<site>/goatbuilder/` inside any
goatnet that serves it, and verify before you run it:

```text
cosign verify-blob --bundle SHA256SUMS.cosign-bundle \
  --certificate-identity-regexp '^https://github\.com/USAREUR-AF/goatcore/\.github/workflows/_release-binary\.yml@refs/tags/goatbuilder/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
shasum -a 256 -c --ignore-missing SHA256SUMS
install -m 0755 goatbuilder-<os>-<arch> ~/.local/bin/goatbuilder
goatbuilder --version
```

From source instead: `cd crates && cargo build --release -p goatbuilder`.

## 2. Rehearse it on the fixture

The worked example is a goatnet called `zagros` with one founder site, `alpha`:
two core machines, one observability machine, one rescue node, five components.
A scripted converger stands in for the hosts, so every verb runs for real
against files, nothing touches a network, and the output is the same every
time. One core machine converges only on its second look, so you see the verb
keep watching a class until every record agrees.

The commands below run from the root of this repository. The worked example is
also a repository of its own, `USAREUR-AF/hello-galaxy-goatenv`:
from a clone of it, use `01-founder` in place of
`ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder`, or run `./run`, which does
both tutorials' rehearsals and the checkpoint comparison.

```bash
WORK="$(mktemp -d)/zagros"
cp -R ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder "$WORK"
goatbuilder standup zagros --site alpha --mock "$WORK" --ack "A Platform Admin" --yes
goatbuilder status --env zagros --mock "$WORK"
goatbuilder assay zagros --values ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder/assay-values.yaml --plan
```

**Checkpoint 1.** Your output matches
`ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/01-new-goatnet.txt` line for
line, apart from the milliseconds. Read it before moving on:

- The phases run in the doctrine's order: 6 the management stack, 11 the router
  and the observability stack, 13 the service surface, 15 the rescue node.
- Every component is **published** first with a serial number (1 to 5), then
  observed on each machine of its class until that machine's record names it.
- The last line of `standup` says `declared → verified: 5 components on 4
  machines … every record names its declared ref`. That sentence is the
  definition of done; on hardware it carries the time the one-hour target is
  measured against.
- `status` reads declared against recorded per machine and finds nothing pending.
- `assay --plan` lists, by layer, every acceptance check that applies to a
  founder site with an observability machine and a rescue node, and the ones
  that read N/A (the federation rows wait for tutorial 2). On hardware the same
  verb without `--plan` runs them; that is checkpoint 2.

Run it twice. The second run publishes again and every machine is already
converged: re-running a standup is always safe.

## 3. Before the real run

Everything the driver cannot create for you, in one list. The canonical
procedure behind it is the standup runbook (`docs/operations/goatnet-standup.md`).

1. **Hardware.** The host-site baseline is four nodes (a core pair and an
   observability pair); one core host is enough to start. Each runs the fleet OS
   with disk encryption.
2. **The site's declaration.** A `group_vars/<site>_hosts/vars.yml` written from
   the nearest existing site, with this site's own zone, addresses and flags. Do
   not copy another goatnet's addresses: a renamed copy keeps them in forms a
   rename does not fix.
3. **The goatnet's entries.** The goatnet's row in `ops/pins/fleet-pins.yaml`,
   its rung in `ops/release/ladder.yaml`, and its door in
   `ops/ansible/inventory/doors.yaml` if your laptop will not sit on its mesh.
   `ops/goatbuilder/goatbuilder.yaml` is generated from the first two.
4. **Secrets.** The variables the plays template live in the team password
   store; your laptop holds references only, one `op://` line each, in
   `.envrc.local.d/<file>.refs`. `goatbuilder doctor` names every one it needs.
5. **Tools.** `goatbuilder doctor` checks them; the runbook lists the rest
   (`yq`, `sshpass`, `go`, `age`, `aws-vault`).

## 4. Run it on your hardware

The same verbs, against your site. `<env>` is the goatnet's key in the pins;
`<founder-site>` is the site id.

```text
goatbuilder validate
goatbuilder doctor --env <env> --credentials
goatbuilder standup <env> --site <founder-site> --dry-run
```

`--dry-run` prints every phase and the exact command it will run, and touches
nothing. That printout is the command reference; there is no second list to
drift. When it reads right:

```text
ops/scripts/with-secrets \
  goatbuilder standup <env> --site <founder-site> --ack "<your name>" --yes
```

`with-secrets` supplies the secrets from whichever store `goatbuilder.yaml`
declares (`secrets: store:`): 1Password or OpenBao references, sops,
git-crypt, a gitignored env file, or variables already exported. With no store
set up yet, `goatbuilder secrets init --env <env>` writes a local env file of
generated values and names the few that only an outside service can issue
(ADR 1204).

Two phases wait for you and print exactly what to do: **8**, enrolling the
host into its own mesh, and **11**, the first browser login on an
internet-facing site. Do the step, then resume at the next phase (here, after
phase 8):

```text
goatbuilder standup <env> --site <founder-site> --from 9 --ack "<your name>" --yes
```

A phase that fails says which phase, what state the site is in, and the resume
command. Every phase is idempotent: fix the cause, resume from that number.

**The scripts avenue.** The same run, unsupported but documented:
`ops/scripts/goat-standup.sh <founder-site> --plan`, then without `--plan`, and
`--from <phase>` to resume.

## 5. Done means green, proven

**Checkpoint 2.** Phase 13 runs the acceptance assay against values generated
from this goatnet's own configuration, and you can run it again at any time:

```text
goatbuilder assay <founder-site>
goatbuilder assay <founder-site> --var goat_via=<user>@<host>   # where your goat context lives
```

It checks the goatnet layer by layer: the hosts, the management stack, TLS and
DNS, the mesh, identity, the fabric, time, the service surface, the deploy
ledgers, the rescue node. The goat CLI rows read it as an operator would:
`goat status`, a `goat hello` round trip, `goat health` for every domain, `goat
who`. Every applicable row is `PASS`, and the table at the end shows the count
per layer. A row that could not run is `UNKNOWN`, which is never a pass; the
command exits 3 until there are none (GB-048). A goat CLI row needs a goat
context with this goatnet's fabric identity on the machine that runs it. Then:

```text
goatbuilder status --env <env>
```

Every machine of every class names its declared ref. **Checkpoint 3.** The last
phase redeploys the cross-goatnet surfaces; open another goatnet's overview and
find the new goatnet's card there before you call it done.

## Undo

Re-running is the repair: fix the cause and resume from the failed phase.
Retiring a goatnet runs the last phase in reverse: remove it from every roster
in one change, merge it, redeploy every cross-goatnet surface, then destroy its
infrastructure. Merging the removal alone changes nothing live.

## What you have now

A goatnet with one site that originates everything. Tutorial 2 attaches a
second site to it, the first federated node.

# 8. Deploying and rolling back

`deploy` puts the ref `main` declares onto one environment and proves it from
the hosts' ledgers. `rollback` moves a component back to an earlier ref by the
rule for its artifact type. `plan` and `apply` let a second person review a
deploy or rollback before it runs. `adopt` turns what an environment already
runs into a declaration.

## The deploy ledger

Every component's role writes one JSON line per change to
`/var/lib/goat/<component>/deploy-history.jsonl` on each host
([ADR 1060](../../adr/1060-universal-deploy-rollback-ledger.md),
[universal-deploy-rollback.md](../../design/universal-deploy-rollback.md)). A
row is appended only when the ref changed.

| Field | Meaning |
|---|---|
| `component` | the ledger directory's name |
| `artifact_type` | `image`, `config`, `acl`, `data`, `cert` or `forward-only` |
| `ref` | the digest-pinned ref now running |
| `prior_ref` | the ref it replaced |
| `ts` | when the play started, in UTC |
| `promoted_at` | when the deploy finished: the host's UTC clock when the row became a deploy |
| `operator` | who ran it |
| `reversible` | `false` means a rollback is refused |
| `action` | `deploy` or `rollback` |
| `restore_pointer` | for data, cert and forward-only artifacts: where recovery starts |
| `source` | `offline` when the run was judged against the cached trunk |

## `deploy`

**Purpose.** Resolve the component and the environment's member groups from
the pins, find the play and its target mode, check every secret the play
templates, decide the transport, run the play through
`ops/scripts/ansible-preflight.sh` once per member group, then read the ledger
back from every host. Done only when each host's newest row names the declared
digest with `action: deploy`.

**Use it** to put a merged pin onto an environment, or to re-converge an
environment that drifted.

```bash
# decide everything, touch nothing; exit 1 when a real run would refuse
# (ladder, fresh source, a pin with no digest, an intent that does not verify,
# an unset secret), after the whole plan
goatbuilder deploy --env <goatnet> --component <component> --dry-run

# the run, with the secrets store's values for this one command
goatbuilder secrets exec -- \
  goatbuilder deploy --env <goatnet> --component <component> --ack "<your name>" --yes

# one member group only
goatbuilder deploy --env <goatnet> --component <component> --group <site>_hosts --ack "<your name>" --yes

# skip the play, only read the ledgers back
goatbuilder deploy --env <goatnet> --component <component> --verify-only

# a deliberate exception to the release ladder, recorded in the ledger row
goatbuilder deploy --env <goatnet> --component <component> --skip-rung --reason "<why>" --ack "<your name>" --yes
```

**Release ladder (ADR 1120).** Before the secrets, the transport or any host,
on a goatnet above the lowest live rung of `ops/release/ladder.yaml`, each
member group's declared digest must be what the rung below pins, and a real
run then reads that rung's ledgers (`--verify-only`) to confirm it converged
there; a dry run names that read. `ops/release/ladder_gate.py` decides, the
module `deploy-component.sh` and `ansible-preflight.sh` call. The only ways
past it are an open hotfix record for the digest and `--skip-rung` with both
`--reason` and `--ack`. A gate that cannot run refuses. `--verify-only` deploys
nothing and is not judged.

**What a dry run prints**, one row per decision (details shortened):

```text
ok  <goatnet>/<component>/secrets  templates 3 variable(s) from your environment
ok  <goatnet>/transport            groups [<site>_hosts] via mesh (this laptop reaches <host>:22)
ok  <goatnet>/playbook             <component>-deploy.yml -e target=<group>
ok  <goatnet>/<group>/deploy       DRY: deploying ghcr.io/<org>/<component>:<tag>@sha256:<digest>
ok  <goatnet>/<group>/verify       DRY: would read the ledger and require sha256:<digest>
ok  <goatnet>/<component>          every host's ledger names the declared ref
deploy: ok (6 checks)
```

**A release component** (`artifact_type: release`) is fetched and verified on
your machine before any host, in a dry run too, and `<goatnet>/<group>/release`
names each file, its SHA-256 and who signed it. `verify` then also reads the file
each host runs and requires the pin's. Verified files are kept under
`<deploy.state_dir>/releases`; with no internet, set `GOATBUILDER_RELEASE_MIRROR`
to a directory of `<owner>/<repository>/<tag>/<file>`.

**Success:** exit 0, every host converged. The converged row says whether the
ledger row was written by this run or was already there.

**Refusals.**

| Code | Meaning |
|---|---|
| GB-001, GB-002, GB-003 | unknown component, unknown goatnet, or a group the component does not deploy to (exit 2) |
| GB-005 | no deploy playbook resolves for the component |
| GB-006 | the door did not open |
| GB-007 | a secret the play needs is not available |
| GB-009 | not a current `main` checkout |
| GB-047 | no `--ack`: every declaration names the person who made it |
| GB-017 | not converged on every host (exit 3) |
| GB-064, GB-065 | the checkout is behind the trunk, or the trunk could not be fetched |
| GB-029 | the play failed on a group; the log path is in the row |
| GB-030 | no host of the group is in the loaded inventories |
| GB-003 | `--node` names something wider than one machine, or no `--group` |
| GB-381 | a release component's file was not fetched or did not verify, before any host (a dry run names it and exits 1) |
| GB-373 | the release ladder: the digest is not on the rung below, the rung below has not converged on it, or `--skip-rung` lacks `--reason` or `--ack` |

**Script:** `ops/scripts/deploy-component.sh <component> --goatnet <goatnet>
[--group G] [--dry-run] [--verify-only] [--skip-rung --reason "<why>" --ack "<operator>"]`. The deploy parity gate compares both
dry runs for every component on every environment.

## `plan` and `apply`: deploy what was reviewed

**Purpose.** `plan` writes every decision a deploy or rollback would make to a
JSON file (`goatbuilder.plan/v1`). `apply` recomputes those decisions and runs
only if none moved.

**Use it** when a second person reviews a change before it runs, or when the
run happens later than the decision.

```bash
goatbuilder plan deploy <component> --env <goatnet> --out deploy.plan.json
goatbuilder plan rollback <component> --n 1 --site <goatnet> --out rollback.plan.json
goatbuilder apply deploy.plan.json --yes
```

A deploy plan holds the member groups, the play, the target flag, the ledger,
the declared ref per group, the pins file's last commit and the transport. A
rollback plan holds the artifact type, reversibility, the target ref with its
source, the goatnet's hosts and the `--site` it was planned for; `apply` carries
that site through, so it rolls back the one goatnet the plan was reviewed for and
refuses (GB-053) if the host set moved under it. A `plan rollback` of a
multi-goatnet component with no `--site` is refused (GB-055), just as the roll
itself is, so a plan that would flip every goatnet can never be written.

**Refusals:** GB-051 (the plan request is incomplete), GB-052 (not a plan
file), GB-053 (the plan no longer matches the tree; the row names each fact
that moved), GB-055 (a `plan rollback` of a multi-goatnet component with no
`--site`). A changed transport is reported, not refused. **Script:** none.

## `adopt`: declare what is already running

**Purpose.** Read every host's ledgers and propose a pins deviation for each
component whose hosts agree on a running ref that differs from `main`. It never
deploys.

**Use it** when an environment was changed outside the ladder, or when taking
over an environment that was never declared.

```bash
goatbuilder adopt --env <goatnet>          # a report
goatbuilder adopt --env <goatnet> --yes    # write the edits into a fresh worktree of main for review
```

`adopt` refuses to choose between hosts that disagree, and names every host it
could not read (exit 1). **Refusals:** GB-050 (nothing declared on that
goatnet), GB-054 (could not create its worktree). **Script:** none.

## `rollback`

**Purpose.** Move a component back to its N-th most recent ref, reading the
host's ledger as the source of truth, by the rule for its artifact type.



| Artifact type | Rollback | Exit |
|---|---|---|
| `image` | automatic: re-pin and recreate the container, then write a `rollback` row | 0 |
| `release` | automatic, one step: every host of one goatnet goes back to the file it ran before, with no fetch, and the ledger gets a `rollback` row; a second rollback switches back; more than one step is refused (GB-379) | 0 |
| `config` | guided: prints the git commit to revert and the redeploy command | 4 |
| `acl` | guided: prints the commit to revert and warns that a full re-render overwrites rules added at runtime | 4 |
| `data` | refused: prints the `restore_pointer`. A ref swap cannot undo committed data | 3 |
| `cert` | refused: prints the rotation command, `goat admin rotate <anchor>` | 3 |
| `forward-only` | refused: `reversible: false` or a `do_not_roll_to` entry | 3 |

```bash
goatbuilder rollback <component> --list        # history, newest first
goatbuilder rollback <component> --status      # the running ref and artifact type per host
goatbuilder rollback <component> --dry-run     # the remote shell each host would run
goatbuilder rollback <component> 1 --site <site> --ack "<your name>" --yes   # one goatnet only
goatbuilder rollback <component> 1 --ack "<your name>" --yes   # one step back
goatbuilder rollback <component> 3 --ack "<your name>" --yes   # three steps back (1 to 7)
```

`--list` reads the first reachable host's ledger, else the in-repo
`deploy-history.yml`. A rollback with no `--site` is refused (GB-055) when the
component is deployed on more than one goatnet: with no site it would flip every
goatnet at once, prod included, so the error names the goatnets and you re-run
with `--site <goatnet>`. A component on a single goatnet still rolls back with no
`--site`. `--site <site>` narrows the roll to the hosts whose `site:` is that
goatnet — it flips every host of that one goatnet and no other; an unknown site
is refused. The host→goatnet fact is each host's `site:` in the ledger, the value
`generate-pins.py` resolves and records.

A release component's `--list`, `--status` and rollback read and act on every
host of one goatnet through `deploy`'s own transport; it needs no
`deploy-history.yml`. A rollback that a host did not take exits 3.

*Fixture output: a `data` artifact refused before anything changes.*

```text
ok    dev/db-schema/artifact  artifact_type=data reversible=true (declared/artifacts.yaml)
FAIL  dev/db-schema/rollback  REFUSING: 'db-schema' is artifact_type=data. A ref swap cannot undo committed data; restore from its backup instead.
rollback: refused as unsafe (2 checks)
```

**After a rollback,** the environment runs a ref that differs from `main`'s
declaration, and `diff` reports it as drift. Either fix forward and deploy the
new pin, or record the rollback as a declaration with `adopt --yes` so `main`
matches the hosts.

**Refusals:** GB-023 (refused or guided by artifact type), GB-039 (a fixture
rollback needs `--env`), GB-047 (no `--ack`), GB-055 (a multi-goatnet component
with no `--site`), GB-064 (a checkout behind the trunk). **Script:** `ops/scripts/rollback.sh <component> [N]
[--list|--status|--dry-run|--site <site>]`. The rollback parity gate compares `--list` and
`--status` line for line for every component with a `deploy-history.yml`, and
compares `--status --site` and an unknown `--site` for every multi-goatnet one.

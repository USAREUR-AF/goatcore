# 11. End-to-end workflows

Each workflow below strings the verbs of chapters 7 to 10 into one job, in the
order a person runs them. Every workflow ends with a read-only proof step.

## A. A laptop's first session

```bash
git checkout main && git pull --ff-only origin main
goatbuilder validate
goatbuilder doctor --env <goatnet>     # fix every FAIL row it names
goatbuilder tutor                      # optional: the eighteen lessons on fixtures
```

Done when `doctor` exits 0.

## B. Deploy a merged pin to one environment

A pull request changed a component's pin in `ops/pins/fleet-pins.yaml` and is
merged. The environment should now run it.

```bash
git pull --ff-only origin main
goatbuilder pins --check                                      # rendered files are current
goatbuilder diff --env <goatnet> <component>                  # shows drift: main moved
goatbuilder deploy --env <goatnet> --component <component> --dry-run
goatbuilder secrets exec -- \
  goatbuilder deploy --env <goatnet> --component <component> --ack "<your name>" --yes
goatbuilder verify <component> --env <goatnet>                # exit 0
goatbuilder history <component> --env <goatnet>               # the new row, with you as operator
```

With review: replace the deploy with `plan deploy … --out p.json`, have a
second person read the file, then `apply p.json --yes`.

## C. Release a component through the ladder

```bash
goatbuilder promote review --detail                           # what is already in flight
goatbuilder release <component> --version X.Y.Z --ack "<your name>" --dry-run
goatbuilder release <component> --version X.Y.Z --ack "<your name>" --yes
# dev deployed, rows passed, preprod deployed, soak line printed
goatbuilder promote status                                    # the candidate on preprod
# … after min_days on preprod …
goatbuilder promote review                                    # says it is ready
goatbuilder release <component> --tag <component>/vX.Y.Z-rc.N --from prod --yes
goatbuilder diff --env <prod goatnet> <component>             # same on every host
```

If a step stops, fix what the refusal names and resume with `--from` at that
step. Re-running a step that already completed changes nothing.

## D. Roll back after a bad deploy

```bash
goatbuilder history <component> --env <goatnet>               # what ran before
goatbuilder rollback <component> --status                     # running ref and artifact type per host
goatbuilder rollback <component> --dry-run                    # the shell each host would run
goatbuilder rollback <component> 1 --ack "<your name>" --yes
goatbuilder diff --env <goatnet> <component>                  # drift: hosts now differ from main
```

Then make `main` match the hosts: revert the pin by pull request and redeploy,
or record the rollback with `goatbuilder adopt --env <goatnet> --yes` and land
the proposed deviation. Exit 3 means the artifact type refuses a rollback;
exit 4 means follow the printed steps.

## E. Ship an urgent fix everywhere

Only for an `image` artifact whose rollback is a pin revert.

```bash
goatbuilder fastlane <component> --ref sha-<short> --reason "<why it cannot wait>" \
  --ack "<your name>" --dry-run
goatbuilder fastlane <component> --ref sha-<short> --reason "<why it cannot wait>" \
  --ack "<your name>" --yes
goatbuilder promote review                                    # shows the backfill owed
```

The hotfix record stays open until the same ref passes the ladder's gates and
`promote graduate` closes it.

## F. Stand up a new goatnet's founder site

```bash
goatbuilder secrets init --env <goatnet>         # zero-SaaS posture, or use your store
goatbuilder doctor --site <site>                 # every missing input, at once
goatbuilder standup <goatnet> --site <site> --dry-run
goatbuilder standup <goatnet> --site <site> --ack "<your name>" --parallel --yes
# stops at phase 8 (enrol the host) and 11 (first browser login): do the step, then
goatbuilder standup <goatnet> --site <site> --ack "<your name>" --from 9 --parallel --yes
```

Done when phase 13's `assay` reports every applicable row PASS and phase 14 has converged
the rescue nodes the site declares. The guided version, with every step
explained, is tutor lesson 10 and [Tutorial 1](../../tutorials/01-new-goatnet.md). Its
worked example, rehearsable end to end, is the repository
`USAREUR-AF/hello-galaxy-goatenv`.

## G. Add a joiner site, then a node

```bash
goatbuilder standup <goatnet> --site <joiner site> --dry-run   # a joiner applies no ACL and seeds from the founder
goatbuilder standup <goatnet> --site <joiner site> --ack "<your name>" --yes
goatbuilder join-node <goatnet> --site <site> --node <name> --class obs --dry-run
goatbuilder join-node <goatnet> --site <site> --node <name> --class obs --address <LAN address> --yes
```

The guided version is tutor lesson 12 and
[Tutorial 2](../../tutorials/02-first-federated-node.md).

## H. Take over an environment nobody declared

```bash
goatbuilder diff --env <goatnet>               # what differs from main
goatbuilder adopt --env <goatnet>              # proposed deviations, one per component
goatbuilder adopt --env <goatnet> --yes        # write them into a fresh worktree of main
# review and land the pull request; then
goatbuilder diff --env <goatnet>               # same everywhere
```

## I. Re-prove a built goatnet

```bash
goatbuilder assay <site> --plan              # which rows apply, by layer
goatbuilder assay <site>                     # exit 0 only when every applicable row passes
goatbuilder assay <site> --only <row id>     # re-run one row after fixing it
goatbuilder diff --env <goatnet>             # what differs from main, if the deploy layer failed
```

## J. Peer with another sovereign goatnet

The one decision is the partner (a peering entry in `goatbuilder.yaml`) and the
release sets it receives (owned by `federation.toml`, which the peering entry
may only narrow).

```bash
goatbuilder federate allocate <partner>                 # the peering's values and files; review and commit them
goatbuilder federate <partner> --env <goatnet> --dry-run
goatbuilder federate <partner> --env <goatnet>          # at a terminal: asks for the keys, the plan and the admission
```

Done when phase 14's acceptance shows the edge carries the agreed data,
refuses everything else, and round-trips. Without a terminal, run the four
stages, each also runnable alone:

```bash
goatbuilder federate plan <partner>      # validate and explain; nothing that carries traffic changes
goatbuilder federate deploy <partner> --approve-plan <hash> --approve-admission <digest>
goatbuilder federate test <partner>      # acceptance; evidence in <work>/evidence/acceptance/
goatbuilder federate status <partner>    # health and the last evidence, with its age
```

See [the runbook, step 1](../../operations/sovereign-federation-runbook.md#step-1--run-goatbuilder-federate-partner).

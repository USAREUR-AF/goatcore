# 7. Checking and reading state

These verbs change nothing. Run them before any deploy, after any deploy, and
whenever you need to know what an environment runs. Each section gives the
verb's purpose, when to use it, the commands, what success looks like, the
common refusals and the script equivalent.

## The words on a terminal

A terminal shows plain words. Pipes, `-o json`, the error catalog and the design
docs keep the terms below, so a pasted log or an `explain` page can use the term
where your screen used the plain word.

| On a terminal | In pipes, JSON, the catalog and the docs |
|---|---|
| stage | rung |
| wait before prod | soak |
| test version | candidate |
| release (`promote review`, KIND column) | candidate |
| hotfix (`promote review`, KIND column) | fastlane |
| current version | fleet ref |
| goatnet overrides | deviations |
| not declared, so treated as stateful | stateful (undeclared, fail closed) |

## `validate`: the config against its schema

**Purpose.** Check `goatbuilder.yaml` against the versioned schema and the
cross-field rules, offline.

**Use it** after any change to the config or its sources, and in CI.

```bash
goatbuilder validate
goatbuilder validate path/to/goatbuilder.yaml
```

**Success:** `validate: ok`, exit 0. **Refusals:** GB-025 (the config does not
validate), GB-027 (not inside a repository). **Script:**
`ops/goatbuilder/validate.py`.

## `doctor`: everything a run needs

**Purpose.** Name every precondition a run needs before anything is cut: tools,
Python dependencies, the checkout rule, each environment's transport, each
play's target mode, the secrets each component's play templates, the secrets
store, and the live validation rows' toolchain.

**Use it** first thing in a session, before a release, and after any change to
your laptop.

```bash
goatbuilder doctor                         # every environment
goatbuilder doctor --env <goatnet>         # one environment
goatbuilder doctor --site <site>           # also scan every input a standup of <site> reaches
goatbuilder doctor --browser-tests         # also require the live rows' browser toolchain
```

`--site` runs the static required-input scan over every play the site's
standup reaches and lists each missing input at once, before any host is
touched. It takes 6 to 13 seconds per site.

**Success:** every row `ok`, exit 0. Each `FAIL` row carries a `fix:` line.

| Row | Checks |
|---|---|
| `config`, `repo` | the config validates; the source repository, image namespace and signing |
| `tool:*` | git, gh, ansible-playbook, aws-vault, go (intent signing), ssh, image-digest, cosign, crane, op |
| `python:deps` | PyYAML and jsonschema for the Python tools both avenues call |
| `checkout` | the checkout is `main` and current with origin; or the tree is an assembled tree, which only its `.goat-assembled` marker says, and the row names what the marker records it was built from (goatcore, any public layers, the overlay). An assembled tree is on a detached HEAD by construction, so that is not a failure; the row fails when the marker cannot be read, says `goatcore_ref=local`, records a `-dirty` overlay commit, or names a goatcore commit that is not the tree's HEAD. A marker that names no overlay checkout with an `assemble.sh` passes with a note: `release` and `rc.sh` refuse that tree (GB-009), while a deploy finds the overlay through `GOAT_OVERLAY_DIR`. The marker is trusted the same way `rc.sh` and `release` trust it. A detached HEAD without the marker still fails (F-586) |
| `secrets:*` | the store in force and whether it can serve |
| `env:op-refs` | no incomplete `op://` reference anywhere in the shell |
| `tool:live-rows` | the browser toolchain the live validation rows need. Only a release runs those rows, so a missing toolchain is a note (the row passes and prints its install line under `Note:`); with `--browser-tests` it is a failure |
| per environment | transport, each play's target mode, each component's templated secrets |

**Refusals:** GB-009 (not a current `main` checkout), GB-010 (a Python
dependency is missing), GB-060 to GB-063 (the required-input scan).
**Script:** none; `deploy-component.sh` and `rc.sh` run their own preflights.

## `verify`: is the declared ref running on every host

**Purpose.** Read every host's newest ledger row for one component and say
whether it names the ref `main` declares.

**Use it** after a deploy, after a host comes back, and as the proof step of
any workflow.

```bash
goatbuilder verify <component> --env <goatnet>
goatbuilder verify <component> --env <goatnet> --group <site>_hosts
goatbuilder verify <component> --env <goatnet> --dry-run   # what it would read
```

**Success:** exit 0, every host converged. **Exit 3** while any host is pending
or unknown. An unreachable host is reported *unknown*, never *not converged*.
A host whose newest row is a rollback is named as one: rolled back from the
prior ref to the current ref, at the row's time.
**Script:** `deploy-component.sh <component> --goatnet <goatnet> --verify-only`;
`goatbuilder deploy --verify-only` runs the same check.

## `diff`: what differs from main right now

**Purpose.** Compare `main`'s declared ref with each host's newest ledger row,
for one component or every component the pins place on the environment,
including a component that has no deploy play of its own.

**Use it** before a release to see what is already out of line, and after an
incident to see what changed.

```bash
goatbuilder diff --env <goatnet>               # every component, one pass per member group
goatbuilder diff --env <goatnet> <component>
```

| State | Means |
|---|---|
| same | the newest row names the declared ref |
| drift | the newest row names another ref |
| unknown | no ledger, the host is unreachable, or a ledger shared by several images whose newest row records another image |
| no coverage | the component declares no ledger (`ledger_component: null`) |

**Success:** exit 0 with every row `same`. **Exit 3** on any drift or unknown.
Measured on a twenty-component environment: one pass in 7 seconds.
**Script:** none.

## `survey`: how consistent the software is, inside and across goatnets

**Purpose.** One read-only pass over every goatnet of the release ladder, in
parallel, that says where the software is not what `main` declares and where
the goatnets differ from each other. It leads with the findings, then prints a
matrix: one row per component, one column per goatnet.

**Use it** at the start of a day of releases, before graduating a candidate, and
whenever a goatnet behaves differently from the one below it on the ladder.

`survey` answers whether the *versions* agree. Whether each service a site
declares is running, healthy and wired to the services it depends on is
`goatbuilder endstate <site>` (ADR 1206); the two read the same hosts and
answer different questions.

```bash
goatbuilder survey                                   # dev, preprod and prod, read in parallel
goatbuilder survey --env <goatnet>                  # one goatnet (repeatable)
goatbuilder survey --all                             # every goatnet fleet-pins.yaml declares
goatbuilder survey --intra                           # only what differs inside each goatnet
goatbuilder survey --inter --html ./survey.html      # the cross-goatnet view, plus a report page
goatbuilder survey -o json                           # goatbuilder.survey/v1, for a program
```

What it reads, per host, in one Ansible call per member group (the read `diff`
makes): the newest row of every ledger; every container, running or stopped,
with the image it was created from and that image's registry digests; the goat
CLI the host runs; and, on a goat-get host, the release tags its `version.json`
files serve. From the repository it reads the ladder's grade for every
component (`promote status`).

**Inside each goatnet:**

| Finding | Means |
|---|---|
| undeclared-drift | a host's ledger or containers run another ref than the one declared, and no declared deviation covers it |
| hosts-disagree | hosts of one goatnet run different refs of one component |
| ledger-vs-container | the ledger names the declared ref but a running container from that image runs another |
| not-running | declared, with no ledger and no container from that image on the host |
| no-ledger | the containers run the declared ref but the host keeps no ledger to verify or roll back from (unknown) |
| served-payload | a goat-get or goat-docs host serves another release than `main` declares — judged from the host's ledger, which records what it serves, or from its `version.json` where the ledger records no payload yet |
| goat-cli-host | a host runs another goat CLI than its own goatnet serves |
| unknown | a host or a goatnet could not be read; one line per host, naming what went unverified |

A running container from the image is the component; a stopped one beside it is
a leftover and is ignored. Stopped containers are judged only when none runs —
a warm standby that keeps its stack stopped is checked for carrying the same
images.

**Across goatnets:** each difference between rungs carries the ladder's grade.
`candidate` and `hotfix-backfill-owed` are declared and in flight, reported as
DECLARED and not failed. `UNDECLARED-SKEW` is a difference no open record
covers, and fails.

**Success:** exit 0 when nothing differs that is not declared and every host was
read. **Exit 3** on any drift or anything unread. **Refusal:** GB-070 when no
goatnet could be read at all. Budget: 90 seconds for the three ladder goatnets,
doors included; measured 40.5 seconds on 2026-10-29 (one goatnet over the mesh
in 11.5 s, one through its SSM door in 24.7 s, one through its door in 40.4 s,
most of it waiting on hosts the door does not reach).
**Script:** none. Like `verify` and `diff`, `survey` reads and changes nothing,
and its reads are the same calls the deploy script's `--verify-only` makes.

## `history`: how it got there

**Purpose.** Print the deploy ledger as a timeline, newest first, on every host
of the environment: each deploy and rollback, its prior ref, its operator, and
when the deploy finished.

```bash
goatbuilder history <component> --env <goatnet>
```

With no host reachable it prints the in-repo history, exactly as
`rollback --list` does. **Script:** `ops/scripts/rollback.sh <component> --list`.

## `status`: declared against recorded, per machine

**Purpose.** Grade every machine of an environment as converged, pending or
unknown, for every component.

**Status:** built on fixtures. On real hosts `status` refuses with GB-026 and
names `verify` and `diff`, which read real hosts today.

```bash
goatbuilder status --mock ops/goatbuilder/fixtures/basic --env dev
```

*Fixture output (excerpt, image names shortened):*

```text
FAIL  dev/dev-core-a/portal  pending: record names portal:v1.3.2@sha256:9f8e7d6c5b4a…,
                             declared portal:v1.4.0@sha256:0b2a6c1d3e4f…
FAIL  dev/dev-edge-a/portal  unknown: dev-edge-a: record unreadable
status: not converged (10 checks)
```

## `explain`: what an error means

**Purpose.** Interpret an error code, or the text of any error that goatbuilder
or a tool it ran printed, and say what happened, why, what to do and where to
read more.

```bash
goatbuilder explain GB-007
goatbuilder explain X-SSH-001
goatbuilder explain E_OFF_MESH
some-command 2>&1 | goatbuilder explain -      # read the text from stdin
goatbuilder explain --list                     # every code the catalog knows
goatbuilder explain --report -                 # save an unknown error to share
```

Chapter 12 describes the catalog and how matching works.

## `tutor`: learn every verb on throwaway fixtures

**Purpose.** Eighteen lessons. Each names the exact command to run against a
throwaway copy of a fixture, then checks what that command did, its exit and
its rows, before the next lesson unlocks. Nothing touches a goatnet.

```bash
goatbuilder tutor            # the current lesson; marks it done when your runs satisfy it
goatbuilder tutor --list
goatbuilder tutor 5
goatbuilder tutor --reset    # fresh fixtures, no progress
```

| # | Lesson |
|---|---|
| 1 | Validate the config, then ask the doctor |
| 2 | Read state: converged, pending, unknown |
| 3 | Deploy: plan it, run it, prove it from the records |
| 4 | Drift and history: what differs, and how it got there |
| 5 | Roll back one step, and see an unsafe rollback refused |
| 6 | Release up the ladder: tag, dev, rows, preprod, soak, prod |
| 7 | Fastlane, and when not to |
| 8 | Pins: find drift, fix it the right way |
| 9 | Explain an error |
| 10 | Tutorial 1: stand up a new goatnet's founder site |
| 11 | Tutorial 2: the first federated node, a joiner site |

The workspace defaults to `~/.goatbuilder/tutor`, or `$GOATBUILDER_TUTOR`, or
`--workspace <dir>`. **Refusals:** GB-040 (not a tutor workspace), GB-041 (no
such lesson).

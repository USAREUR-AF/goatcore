# Federation plan (`goatbuilder federate plan`)

## Status

**Built:** `goatbuilder federate plan <partner>` in
`crates/goatbuilder/src/federate/plan.rs`, which writes `<work>/plan.json`
(schema `goat-federation-plan/v1`); `goatbuilder federate plan <partner> --recheck`
and the function behind it, `recheck_live_prerequisites`; codes GB-200 to GB-202
in `ops/errors/catalog.yaml`. Unit tests cover the hash, the digest, the
recheck and the value resolution.
**Not built:** the deploy step that calls the recheck before it changes
anything, and approval bound to the plan's hash (both belong to the deploy
stage); the allocated per-peering values (the plan reads the group's vars and
the roles' defaults until allocation is wired in); the check of this peering
against other peerings on the same host; the call to the baseline host firewall's renderer, so a host with the
baseline on is BLOCKED. **Not measured:** no run against real
hosts.

Status: WIP — gate: on the federation test environment, a plan of a clean
two-party peering is READY with the same hash on two runs, and after phase 9
renders again the recheck refuses with GB-200 naming `rendered`.

## What it is for

A peering run changes two hosts and the core router. Before anything changes,
the plan states what the run would change and what it rests on, so a person can
approve that exact change, and the deploy step can refuse when what it is about
to do is no longer what was approved. This follows section 2 of the 9 November
federation automation proposal ("plan before mutation"). The plan reads; its
only writes are `<work>/plan.json` and phase 1's result files under
`<work>/plan/`.

## What the plan holds

| Section | Holds | Read from |
|---|---|---|
| `versions` | goatbuilder's version, the checkout's commit (`-dirty` with changes), `goat --version`, the edge router and republisher images; under `--mock` the commit and `goat` are not read, so a mock plan is the same on every machine | the binary, `git`, `$GOAT_BIN`, the group's vars and the roles' defaults |
| `inputs` | every value phase 1 and the live checks read, as the run will see it: the variable when set, else the derived value (`federate/preflight.rs`), and the live-check targets only the environment sets (`FED_ROUTER_CONTAINER`, `FED_CORE_CONFIG_PATH`, `FED_SLOT_*`, `DOCKER_HOST`). A variable set to exactly the derived value counts as derived, so `federate` exporting its derived inputs before the phases is not a change. Another core router daemon or key path is a changed input | the environment and the deployment |
| `agreement` | path, digest, transport, parties and flows; absent before phase 6 | `<work>/agreement.json`; the digest is sha256 over its bytes, which are the canonical form, so it equals what `goat federate compose` prints |
| `rendered` | every file under `<work>/applied` with its sha256; empty before phase 9 | `goat federate apply`'s output |
| `pki` | every file under `<work>/pki` with its sha256: a re-minted certificate is a changed input | phase 8's output |
| `certificates` | subject and expiry of each certificate under `<work>/pki`. An expired one is a FAIL in readiness; with files present and no `openssl`, expiry is SKIP | `openssl x509` |
| `resources` | what this peering owns: its WireGuard interface, tunnel port, edge router port, edge router container, republisher metrics port and container, the core access-control fragment, the working directory, the certificate directory (`<work>/pki`), the link subnet (from the agreement; empty before phase 6) and (with `--as`) the republisher identity. The deploy step copies its owned-resources list from here. The containers are the group's `goat_zenoh_edge_stack` and `goat_fed_republisher_stack`, else the roles' `<role>-<partner id>`; the core group is `<peering>_core`, the group phase 12 deploys to | the same sources phase 1 reads, in the same order (`PORT_INPUTS` in `preflight_live.py`): the variable, the group's vars, its parent group's, `all`, then the role's default. A templated value is not guessed: its name is empty and its source says so |
| `restarts` | the core router (phase 10), this peering's WireGuard interface (11), edge router (12) and republisher (12, only when we offer a release set) | the phases; the agreement's flows decide the republisher |
| `acceptance` | the runner, whether `FED_ACCEPTANCE_STRICT=1` is set, and for each flow a reception and a refusal that acceptance must show | the agreement's flows |
| `readiness` | each phase-1 check's outcome and code | `preflight.py`, run with the derived inputs into `<work>/plan/preflight.json`; `--no-live` skips the live reads (each SKIP) |
| `host_firewall` | one entry per host of the edge and core groups, with its address. With `goat_host_firewall_baseline` not true (the default): NOT APPLICABLE, "host firewall baseline: off", nothing rendered or applied. With it true: BLOCKED, and a BLOCKED readiness row, because the plan does not render the rule set yet and must show it before anything is applied. A host that moves to another address is a changed input | the peering inventory, the group's vars |
| `verdict` | `READY`, `READY, <n> check(s) not verified` (SKIP is never a pass), or `BLOCKED` (a FAIL or BLOCKED) | the readiness |
| `hash` | sha256 over the plan without `hash` and without each readiness `detail` | the plan |

The hash is stable: the plan serializes its fields in one fixed order, and
nothing in the hashed part carries a time. A readiness `detail` is left out
because it can name a process id; the outcome and code stay in. Any changed
input changes the hash.

Phase 1 runs with the inputs `federate/preflight.rs` derives, set only in its
own environment; the plan exports nothing into the operator's shell. A plan
under `--mock` runs nothing and writes nothing: phase 1 is SKIP and the plan is
printed with its hash.

## The recheck before a change

A saved plan is not proof that the environment stayed the same. The deploy
step calls `recheck_live_prerequisites(repo, plan_path)` immediately before it
changes anything, and goes ahead only when `Recheck::allows_change()` is true:

1. The plan file is read and its hash recomputed. A missing file, another
   schema or a hash that does not match is GB-201. So is a plan for another
   peering, or one that is not in the working directory it names (a plan
   copied from another peering's directory). The hash is not a signature: it
   catches an edited file, not a forged one.
2. A plan that was not ready cannot be approved later: a FAIL or BLOCKED in
   its readiness, or a SKIP of a check step 4 does not rerun, is listed in
   `unready`.
3. Every input section is built again from the current state and compared
   with the saved plan. Each section that differs is listed in `changed`
   (GB-200): a new plan is needed, and a new approval.
4. The live phase-1 checks (`edge-ports`, `core-ports`, `slot-anchors`,
   `core-router`, `ingress-stores`) run again into `<work>/plan/recheck.json`,
   removed first so an older answer never stands in. Each must answer, and be
   PASS or NOT APPLICABLE; anything else is GB-202 beside the check's own code.
   A SKIP does not pass here. The certificates are read again too: one that
   expired since the plan blocks.

`goatbuilder federate plan <partner> --recheck` runs the same function on
demand and exits 0 only when a change may go ahead. It refuses `--mock` and
`--no-live` (it exists to read the hosts), and an `--edge-group` or `--as` that
differs from the saved plan's.

## Reverting this

Reverting removes the `plan` subcommand, `<work>/plan.json` and codes GB-200 to
GB-202. `goatbuilder federate <partner>` and every phase run as before: no phase
reads the plan, and the driver's dry run and the parity gate do not change.

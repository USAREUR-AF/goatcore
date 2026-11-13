# Federation plan (`goatbuilder federate plan`)

## Status

**Built:** `goatbuilder federate plan <partner>` in
`crates/goatbuilder/src/federate/plan.rs`, which writes `<work>/plan.json`
(schema `goat-federation-plan/v1`); `goatbuilder federate plan <partner> --recheck`
and the function behind it, `recheck_live_prerequisites`; codes GB-200 to GB-202
in `ops/errors/catalog.yaml`; the host-level allocation check in
`crates/goatbuilder/src/federate/hostalloc.rs` (GB-203); the host firewall
entries from the baseline's renderer in `crates/goatbuilder/src/federate/hostfw.rs`. Unit tests cover the
hash, the digest, the recheck, the value resolution and the host check.
**Not built:** the deploy step that calls the recheck before it changes
anything, and approval bound to the plan's hash (both belong to the deploy
stage); the allocated per-peering values (the plan reads the group's vars and
the roles' defaults until allocation is wired in); the host firewall input when a value resolves only inside a play (that host is BLOCKED). **Not measured:** no run against real
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
| `pki` | every file under `<work>/pki` with its sha256, except private keys (`*-key.pem`, `*.key`): a re-minted certificate is a changed input, and a key's fingerprint in a reviewable file buys nothing | phase 8's output |
| `certificates` | subject and expiry of each certificate under `<work>/pki`. An expired one is a FAIL in readiness; with files present and no `openssl`, expiry is SKIP | `openssl x509` |
| `resources` | what this peering owns: its WireGuard interface, tunnel port, edge router port, edge router container, republisher metrics port and container, the core access-control fragment, the working directory, the certificate directory (`<work>/pki`), the link subnet (from the agreement; empty before phase 6) and (with `--as`) the republisher identity. The deploy step copies its owned-resources list from here. The containers are the group's `goat_zenoh_edge_stack` and `goat_fed_republisher_stack`, else the roles' `<role>-<partner id>`; the core group is `<peering>_core`, the group phase 12 deploys to | the same sources phase 1 reads, in the same order (`PORT_INPUTS` in `preflight_live.py`): the variable, the group's vars, its parent group's, `all`, then the role's default. A templated value is not guessed: its name is empty and its source says so |
| `restarts` | the core router (phase 10), this peering's WireGuard interface (11), edge router (12) and republisher (12, only when we offer a release set) | the phases; the agreement's flows decide the republisher |
| `acceptance` | the runner, whether `FED_ACCEPTANCE_STRICT=1` is set, and for each flow a reception and a refusal that acceptance must show | the agreement's flows |
| `readiness` | each phase-1 check's outcome and code | `preflight.py`, run with the derived inputs into `<work>/plan/preflight.json`; `--no-live` skips the live reads (each SKIP) |
| `host_firewall` | one entry per host of the edge and core groups, with its address. With `goat_host_firewall_baseline` not true (the default): NOT APPLICABLE, "host firewall baseline: off", nothing rendered. With it true: PASS with the `ruleset` text and its `sha256` from the baseline's own renderer, or BLOCKED (and a BLOCKED readiness row with the renderer's GB-21x code) naming why it cannot be shown (below). A host that moves to another address is a changed input | the peering inventory, the vars the role reads, the renderer |
| `conflicts` | each value this peering and another declared peering would bind on one machine (below) | the peering inventory, each peering's group vars and agreement |
| `verdict` | `READY`, `READY, <n> check(s) not verified` (SKIP is never a pass), or `BLOCKED` (a FAIL or BLOCKED) | the readiness |
| `created_at` | when the plan was made (RFC 3339, UTC); outside the hash, so the same inputs give the same hash | the clock |
| `hash` | sha256 over the plan without `hash`, `created_at` and each readiness `detail` | the plan |

The hash is stable: the plan serializes its fields in one fixed order, and
nothing in the hashed part carries a time. A readiness `detail` is left out
because it can name a process id; the outcome and code stay in. Any changed
input changes the hash.

Phase 1 runs with the inputs `federate/preflight.rs` derives, set only in its
own environment; the plan exports nothing into the operator's shell. A plan
under `--mock` runs nothing and writes nothing: phase 1 is SKIP and the plan is
printed with its hash.

## Conflicts with other peerings

Allocation keeps the peerings of one environment apart in the declaration. The
host-level check looks at machines. It maps every declared peering's edge
group (`<peering>_edge`; this run's `--edge-group`) and core group
(`<peering>_core`) to the hosts the peering inventory names. The inventory is
read as Ansible reads it for this purpose: `[group:children]` expanded,
`[group:vars]` skipped, `ansible_host` from any line naming the host (quotes
removed) or from `host_vars/`, else the name. Host ranges are not expanded.
Machines are compared by address, so two inventory names for one address are
one machine. A peering is `<with>/<environment>`: the same partner on two
environments is two peerings. Then the check finds what two peerings would
share on one machine:

| Shared on one machine | Why it breaks | Compared when |
|---|---|---|
| an inventory name in both peerings' groups | Ansible merges both groups' variables onto that host, so one peering's values overwrite the other's | always |
| WireGuard interface, tunnel UDP port | two tunnels cannot bind one | the peering's agreement chose wireguard, or it has none yet; an ipsec-fips peering binds neither |
| carrier interface, carrier UDP port | the same, for the carrier to a forwarder | the peering's group sets `peering_wg_via_forwarder: true` |
| link subnet (overlap, IPv4 or IPv6) | two links with one address | from `transport.link_subnet` of each peering's agreement |
| republisher metrics port, on the core | two republishers cannot bind one | always |

This peering's values are the plan's resolved resources (and its edge group for
the carrier values). Another peering's are its group's vars, then the role
defaults, never this run's environment variables; its agreement is looked for
beside our working directory, then under `/var/lib/goat-federation/<peering>`.
The edge router's partner-facing port is not compared: `goat federate apply`
binds each edge router to its own link address, so the port is shared by
design, and the link subnets keep those addresses apart.

Outcomes: a conflict this peering is part of is a FAIL with GB-203, naming the
machine, the resource and each peering's value. The check is SKIP when this
peering has no inventory host, when one of its own values does not resolve
(unset or templated), or when another peering on a machine we share has a value
that does not resolve or (once our agreement exists) no agreement to compare
subnets with. Otherwise PASS. Only the conflicts this peering is part of go into
its plan, so a conflict between two other peerings does not change this plan's
hash. Ports held by something that is not a declared peering (NetBird on UDP
51820, any other service) are phase 1's live port check.

## Host firewall

For each host of the edge and core groups with `goat_host_firewall_baseline`
true, the plan runs the baseline's own renderer
(`ops/ansible/roles/goat-host-firewall/files/goat-host-firewall-render.py --input - --format json`,
[peering-host-firewall.md](peering-host-firewall.md)) and records the ruleset
text and its sha256. `crates/goatbuilder/src/federate/hostfw.rs` builds the
renderer's input the way the deploy builds it:

- **Variables**, in Ansible's precedence, highest first: apply's rendered values
  (the WireGuard and core plays load `<work>/applied/ops/ansible/group_vars/<edge group>`
  with `include_vars`, and stop when it is missing, so the plan blocks then; the
  ipsec-fips play loads none); the playbook's `host_vars/`, the inventory's
  `host_vars/`, the host's inventory line; then, for every group the host is in
  other than `all` (children expanded, deeper groups first), all of their
  playbook `group_vars/`, then all of their inventory `group_vars/`, then all of
  their `[group:vars]`; then the same three for `all`; then the defaults of the
  roles that play has in scope (goat-host-firewall and the play's own role).
  Vars directories are read recursively, hidden names left out, as Ansible does. A vars file that does not parse blocks
  the host rather than being skipped.
- **Templates**: a value that is `{{ var }}` or `{{ var | default(x) }}`
  (optionally `| int`, `| bool`, `| trim`, `| string`), or a string holding such
  expressions, is evaluated; `default(x)` takes a literal or a variable. Any
  other template, a template inside a list or mapping, an undefined variable, or
  a declared null is an error, and the host is BLOCKED. An error in reading
  `goat_host_firewall_baseline` itself is BLOCKED, never "off".
- **Fields**: the role's `_hf_input` (`roles/goat-host-firewall/tasks/main.yml`),
  this run's peering entry as `peering-wg.yml` or `peering-edge.yml` builds it,
  and the core host's SPIRE bundle port as `peering-republisher.yml` sets it
  (only on `peering_spire_server_host` with `peering_spire_enabled`; no sources).
- **Peerings on an edge**: the role renders every peering stored on the host
  (`/etc/goat-host-firewall/peerings/*.json`) with this run's entry in place of
  its stored copy. Phase 1's read-only probe (`ops/ansible/peering-preflight.yml`)
  now reads those stored entries into its answer (`host_firewall_peerings`), and
  the plan uses them the same way. The plan runs phase 1 before it builds the
  inputs, and removes earlier probe answers first; with no answer for a host
  (`--no-live`, `--mock` (which reads no answer at all), or the host did not
  answer), the host is BLOCKED.

The fields the role reads from the live host (bridge interfaces, the ephemeral
port range, uplinks, the managing session, the listening sockets) are left out:
they are checks only and never change the ruleset or its hash.

How the equality is tested: a synthetic fixture with three variants
(`ops/federation/standup/fixtures/host-firewall/`: WireGuard, ipsec-fips, and
WireGuard behind a forwarder; its README lists what each covers). A Rust test
requires the plan to build exactly the recorded input for each. A Python test
(`tests/test_host_firewall_plan_input.py`, run where ansible-playbook is
installed, as in the CI python job) has Ansible build the deploy's input for the
same variants from the role's and plays' own tasks and variables, copied out of
their YAML, renders both, and requires the same sha256. Another Rust test fails
when the role, the plays or the core play's variables gain a field the plan does
not build. The fixture is evidence for those declarations, not a proof for
every possible one; what the plan cannot evaluate it blocks.

A PASS is the ruleset the deploy will try to load, not a promise that it loads:
the deploy also refuses on what it reads live (a peering unit on the host with no
stored entry, GB-210; the managing session or a listening port the ruleset would
cut, GB-213 and GB-215).

A host is BLOCKED, with the reason, when a value cannot be evaluated outside a
play (above), there is no agreement yet (the tunnel is not known), apply has not
rendered the far link address, the counterparty package is missing, the stored
peerings were not read, the renderer is not in the checkout, or the renderer
refuses. A refusal puts the renderer's code (GB-210 to GB-215) on the entry and
its readiness row.

Not covered: hosts whose inventory uses host ranges (`edge[01:02]`), inventory
sources other than `ops/ansible/inventory/peering-edges.ini`, and templates
beyond the forms above (they are BLOCKED, not guessed).

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
   `unready`. So is a plan older than `FED_PLAN_MAX_AGE_HOURS` (default 24) by
   its `created_at`, or one without it: an old plan with unchanged inputs is
   aged out. `created_at` is outside the hash, so, like the hash itself, it
   catches a mistake, not a forgery.
3. Every input section is built again from the current state and compared
   with the saved plan. Each section that differs is listed in `changed`
   (GB-200): a new plan is needed, and a new approval.
4. The live phase-1 checks (`edge-ports`, `core-ports`, `slot-anchors`,
   `core-router`, `ingress-stores`, `clock`) run again into `<work>/plan/recheck.json`,
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

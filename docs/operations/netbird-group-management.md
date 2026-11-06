# NetBird group management — every group, who goes in it, and how membership changes

| Field | Value |
|---|---|
| Status | Runbook. |
| What it is | The one page to open before putting a device in a NetBird group, or changing one. It lists every group family, the rule for assigning it, where the source of truth is, and how to change membership without the dashboard. |
| Long-form reference | `managing-netbird-groups.html` — the same material as a 15-page reference with the reach grid, every family as a definition block, the rule shapes with ports, worked devices, and the verification and drift chapters; render to PDF with `ops/scripts/render-netbird-groups-reference.sh`. |
| Canon it points at | `mesh-and-ory-roles.md` (vocabulary, both planes), `mesh-acl.md` (the rules), [`p2p-graduation.md`](p2p-graduation.md) (the capability), `mesh-identity-operations.md` §2 (deviations, reconcile), [`warrant-runbook.md`](warrant-runbook.md) (the ACL is generated) |

## Two facts that decide everything else

**A NetBird group is per device, and it answers one question: what sockets may this peer open, and who may open sockets to it.** It says nothing about what a person may do inside a service; that is the Ory plane, per person, and the two never share a name except `PLATFORM-ADMIN` (the name-match rule).

**The ACL is generated, and membership is declared.** Rules come from `ops/warrant/rule-intents/netbird-acl.yaml` through `ops/warrant/generate.py` into `ops/ansible/netbird_acl_matrix.yml`; the vocabulary of groups per scope is `ops/ansible/netbird_acl_taxonomy.yml`. What a device is minted with comes from its person's manifest, `ops/operators/<env>/<handle>.yml`, field `netbird.auto_groups`. A group added in the dashboard is a deviation: it is either recorded in `ops/ci/membership-declared-deviations.yaml` with a reason and a revisit date, or the daily reconcile check on the mgmt host reports it until someone removes it. A dashboard edit is never the fix.

## The group families, and the assignment rule for each

Every group is `<PREFIX>-<SUFFIX>`. The prefix is the scope (a goatnet, a site, or an exercise `EX-<NAME>`); the suffix is one of the families below. A machine holds exactly one scope's groups; reach into another scope is a `HOST:` rule, not a second membership (ADR 1124).

### Person roles — one per person device, the narrowest that fits

| Suffix | Who | What it reaches | Do not use it for |
|---|---|---|---|
| `OBSERVER` | someone who browses: web surfaces, the portal, panoscope, external services | substrate service ports (rule A) and external-infra (rule C), one way | a machine; anyone who publishes to the fabric |
| `DEVELOPER` | a full participant who builds on the fabric: pub/sub through their own Zenoh slot | OBSERVER's reach plus the router on 7447 | granting lateral reach; that is P2P |
| `MONITOR` | observability-support staff without identity-admin rights | participant-grade reach plus diagnostics targets | a stand-in for PLATFORM-ADMIN |
| `PLATFORM-ADMIN` | a platform operator | any port to the substrate, gear and pods | a machine, ever; a shortcut to reach a colleague's laptop (the substrate does that) |

A person's phone or tablet is a person device and takes a person role, usually `OBSERVER`. A person device never holds an infra tier or a node class.

### Infra tiers — machines we run, or consume

| Suffix | What goes in it | Reach |
|---|---|---|
| `INFRA` | our substrate: mgmt hosts, mesh-dns, relay-health, site-hosted service peers | reaches everyone, reached by everyone on service ports (rule A) |
| `EXTERNAL-INFRA` | a service peer we consume but do not run: a partner's or vendor's server | reached by every role group one way, any port (rule C); initiates only toward INFRA |
| `BACKEND-INFRA` | hidden backend tier behind an INFRA service: postgres, object stores | reached only by INFRA |
| `PUBLIC-INFRA` | ours, doing infra work, and exposed to the public internet (ADR 1162) | as INFRA, with the exposure declared |

The most common mis-assignment: an external service minted as `OBSERVER` + `P2P`. It looks like it works, because clique members reach it, and then every new user has to be added to the clique. The tier is `EXTERNAL-INFRA`.

### Node classes — machines with a wire role, scope-local

| Suffix | What goes in it | Reach |
|---|---|---|
| `NETWORK` | bearer gear: routers, switches, radios (ADR 1113) | managed by this scope's PLATFORM-ADMIN and INFRA; initiates only to INFRA management ports; never P2P |
| `NETWORK-JUMP` | the one peer that opens interactive sessions to NETWORK gear (ADR 1131) | outbound only; reached by nothing |
| `BREAKGLASS` | reach-in of last resort, dual-homed (ADR 1114) | reached by PLATFORM-ADMIN and INFRA; initiates to INFRA and NETWORK on 22 only |
| `MOON` | uplink of a foreign-stack goatnet (ADR 1092) | source only; nothing initiates toward it |
| `POD` | a moon-pod appliance inside our identity planes (ADR 1092) | substrate-grade reach; a probe and P2P destination; no external-infra reach |

### Gear classes — tactical systems, by wire behaviour (ADR 1116)

Pick the class whose traffic shape matches the device, not the label that sounds right.

| Suffix | Behaviour | Reach |
|---|---|---|
| `SENSOR-FEED` | publishes observations | publish to INFRA on 7447; nothing initiates toward it |
| `SENSOR-TASKING` | accepts tasking | destination only, from an enumerated source list |
| `SENSOR-TELEMETRY` | publishes its own health | publish only |
| `SENSOR-ARBITER` | turns tasking and state into a plan | subscribe and publish |
| `KID` | our appliance fronting devices that do not speak fabric | as SENSOR-FEED plus its local sensor side |
| `EXTERNAL-KID` | same function, egress we do not control | own tier; never a lateral destination |
| `EXTERNAL-INTERMEDIARY` | someone else's box between platforms, never a terminus | as declared per rule |
| `FUSION` | publishes derived data, not observation | publish, marked as inference |
| `SENSOR`, `EFFECTOR` | reserved and empty: the sensing or effecting element itself | none until one carries a radio |

Gear talks to gear on the site scope through the `gear → gear` and `gear → POD` rules on 7447/7448 (ADR 1124). That is the designed path for systems that must keep talking with no backhaul. Gear does not hold P2P by default; see the third reason below.

### Satellites — lifecycle lanes, never assigned by hand

| Suffix | What | Who moves peers in and out |
|---|---|---|
| `PROBATION-<role>`, `PROBATION-GEAR` | dormant holding lane; INFRA 443 and DNS only | the shed command (`members move --filter 'last_seen>14d'`); the detection cron and the explainer page are designed, not built (ADR 1089) |
| `QUARANTINED` | security hold, flat | an admin, through `goat admin members move ... --set-marker quarantine` (ADR 1091) |
| `EX-<NAME>-<suffix>` | an exercise's own copy of the set | stamp and teardown through the cutover playbook; onboarding with `--env <exercise>` |

### The capability — P2P

`<SCOPE>-P2P` is not a role. It is an additive grant: members may initiate to other members, any port, one way. It is never minted by a template, never a peer's only group, and it is granted per device for one of three reasons and no other:

1. **Development clique.** A person past the basics doing device-to-device work with named counterparts already in the clique. This is the host goatnet's P2P.
2. **Tactical site continuity.** Nodes at a site that must keep reaching each other after WAN loss. This is the site scope's P2P, among the site's named nodes, with the site's own INFRA node so rule A survives the drop.
3. **Denied or disconnected autonomous systems.** A gear set that must keep communicating with nothing upstream. Gear classes and the port-scoped gear rules first; the site scope's P2P among that gear set only where the protocol cannot be port-scoped.

Reaching a service is not a reason (rule C). Reaching a colleague's machine for diagnostics is not a reason (the substrate does that, audited). The request, approval, apply and record steps are in [`p2p-graduation.md`](p2p-graduation.md).

## Deciding the group for a new device

1. **Is it a person's device?** Then one person role, the narrowest that fits their job. Phones and tablets are usually `OBSERVER`. Stop here.
2. **Do we run it, or consume it?** Run it as substrate: `INFRA`. Consume it: `EXTERNAL-INFRA`. Behind a service: `BACKEND-INFRA`.
3. **Is it bearer gear, a jump box, a break-glass node, a pod or an uplink?** One node class.
4. **Is it a tactical system?** One gear class by wire behaviour, on the site's scope.
5. **Does it need lateral reach?** Only through a P2P request with one of the three reasons. Never as the answer to "it can't reach X".

If none of these fits, the device is not ready to be enrolled. Write down what it does on the wire and ask on the bus.

## Changing membership

| Change | How | Never |
|---|---|---|
| A new person's devices | `goat admin invite --env <scope> --tier browse\|developer\|iam-admin\|platform-admin`; the candidate accepts in the console and the onboarding runner mints the key from the tier's `auto_groups`; `ops/scripts/sync-onboarded-manifests.py` writes the manifest. The direct `goat admin onboard` enrol leg is fenced on live goatnets and refuses | mint a key in the dashboard with hand-picked groups |
| Move a peer between groups **on a warrant-governed goatnet** (its `ops/warrant/membership/<scope>/_scope.yaml` says `converge: enforce`; on a scope still in `report`, the row below applies) | add or change the peer's entry in that file and open a pull request; `goat admin members move ... --write-entry` writes the entry for you. The converger applies it ([warrant-runbook](warrant-runbook.md) §4A, [ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md)) | any edit in the NetBird dashboard, which is read-only there |
| Move a peer between groups | `goat admin members move --from <A> --to <B> --handle <who> --apply` (dry-run without `--apply`): mirror-first, coverage-gated, audited. `move` is the only verb; shed and absorb are the same command with the groups swapped | dashboard drag and drop |
| A grant outside the manifest that must stand | apply it, then record it in `ops/ci/membership-declared-deviations.yaml` with `decided_by`, `decided_on`, `reason`, `revisit_when` | leave it for the reconcile to keep reporting |
| A removal that must stay removed | record it as `absent:` in the same file; a re-add then prints REVERTED | rely on memory |
| A new scope | one row under `netbird_exercises` in the taxonomy, rules and assertions in the rule intents, `ops/warrant/generate.py`, then `netbird-acl-bootstrap.yml` and `netbird-acl-cutover.yml -e target_site=<scope>` | create groups by hand |
| A new rule | a rule intent plus an assertion, regenerate, cutover in `propose` then `apply` | edit a policy in the dashboard |

## Verifying

| Question | Command |
|---|---|
| Does the declared ACL still hold every assertion for a scope? | `goat acl test <scope>` (offline, the same evaluator as netvis and the PR gate). To check an edit you haven't merged, run `cargo run -p goat-cli -- acl test <scope>` from the checkout's `crates/`: an installed `goat` checks the declaration it was built from. A scope with no assertions fails (nothing was checked); pass `--none-expected` when that is intended |
| Why can A not reach B on a port? | `goat warrant why-blocked --scope <scope> --from <GROUP> --to <GROUP> --port <n>` |
| Who holds which mesh groups? | `goat admin peer ls` (mesh IP and groups per peer); `goat who --once` is an identity roster and shows no mesh groups |
| What is actually applied on the wire? | netvis: `/?level=g` for the group matrix, machine level for a group's members |
| Which peers hold something their key never granted? | the daily reconcile report on the mgmt host (`check-membership-reconcile.py`); read it weekly |
| Is a peer reachable at all? | from the mgmt host, `ping <mesh-ip>`; a P2P-only peer answers nobody, which is a membership fault, not a rule gap |

## The mistakes this page exists to stop

- A service minted as `OBSERVER` + `P2P`, so every user gets added to the clique.
- A person's laptop in `EXTERNAL-INFRA`, reachable by every role group on any port.
- `PLATFORM-ADMIN` on a machine, or on a person who needs one more path.
- `P2P` as a peer's only group: it can reach the clique and be reached by nothing.
- A dashboard edit with no deviation entry and no manifest change, which the reconcile reports every day until it is undone.

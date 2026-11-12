# Federating two sovereign environments — operator runbook

**Audience: an operator of a goat environment who wants to exchange data with another
environment run by someone else.** You need no relationship with the authors of this software and
no access to anyone else's systems. Everything here is done on machines you control, using
material both sides are content to publish.

**Read this with your counterpart.** Both sides run the same command, in the same order, against
each other. Nothing in this procedure is asymmetric, and neither side needs privileged access to
the other.

Design behind it: `cross-sovereign-federation.md`.
Decision: [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md); the two
avenues, command and script: [ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md);
federation in goatbuilder: ADR 1225 (`federation-belongs-to-goatbuilder-one-command-one-decision`).

Status: measured — the federation end-to-end test drives both sides with goatbuilder federate alone (run up-4b1adc18cc3b-20261111T102642).

Step 1 describes the procedure: `goatbuilder federate <partner>`, and its stages `plan`, `deploy`,
`test` and `status`. This page teaches only `goatbuilder federate`. The earlier method is described
in one place, [The earlier method](#the-earlier-method).

**Status of the procedure itself: it has not yet been executed between two real environments
through these stages.** The certificate behaviour it relies on is proven
(`ops/federation-smoke/assert-pki.sh`, 7/7). Treat a first peering as a first-run procedure, not a
routine one.

---

## What you are building

One extra data router on each side, dedicated to the other party, and one small process beside
each core that names your data for the crossing.

```
 release/<set>/**                                                      release/<set>/**
       │ republisher (on your core host)          republisher (on their core host) │
       ▼                                                                           ▼
 from/<you>/<set>/**                                                 from/<them>/<set>/**
your core fabric ── your edge router ══ tunnel ══ their edge router ── their core fabric
   trusts YOUR CA       trusts THEIR CA              trusts YOUR CA        trusts THEIR CA
```

Your edge router is the only thing that trusts their certificate authority. Your core fabric
never does. Their side mirrors it exactly.

**The republisher** runs on your core host, the host your edge dials, and both of its sessions
dial your core router. It subscribes to each release set you offer, `release/<set>/**`, and
publishes every sample under `from/<you>/<set>/**`, the wire name. It is the only writer of
`from/<you>/**` on your side; your core admits it by its own certificate name, and your edge
carries the wire name out. Several sets share the one wire prefix, the set kept as a path
segment: `release/REL-ALL/<rest>` crosses as `from/<you>/REL-ALL/<rest>`.

## Before you start

- [ ] Both sides run this software with **their own** certificate authority. If one side does not
      have its own CA, stop — you do not have two sovereign environments, and a different pattern
      applies.
- [ ] **One** side can accept inbound UDP on one port, directly or through a forwarder. The other
      side only dials out. If both edges sit behind NAT with no inbound port, one side runs a
      peering forwarder first, because its address is a value that side's package carries:
      [`peering-forwarder.md`](../design/peering-forwarder.md) ([ADR 1218](../adr/1218-peering-forwarder-when-neither-edge-accepts-inbound.md)).
      No other reachability is required.
- [ ] Both sides reach the same **peering directory** (saltlick) over HTTPS. The command exchanges
      your packages and CA certificates through it, so neither side needs a public host and
      nothing is sent by email. It grants nothing:
      [`peering-directory-service.md`](../design/peering-directory-service.md).
- [ ] You have agreed, in writing, **what data flows** and **in which direction**. Everything
      below is mechanical; that is the decision. The names each router presents are derived from
      the agreement, not agreed by hand (Step 2).
- [ ] A host you control to run the edge router on. It does not need to be powerful. It must not
      be a machine that already runs your core fabric. It is a peer of your own mesh in node class
      `PEERING-EDGE`, which lets it reach only your core router's fabric port and mesh DNS (ADR 1219).
- [ ] The machine you run `goatbuilder` on reaches your edge host and your core host over SSH, as
      the ansible phases already require. Each operator runs everything on their own systems
      only; no step needs access to the other side's.

---

## Step 1 — Run `goatbuilder federate <partner>`

`goatbuilder federate <partner>` runs four stages in order, and each is also a command of its own:

```bash
goatbuilder federate <partner>           # at a terminal: phases 1 to 14, asking at its prompts
goatbuilder federate plan <partner>      # phase 1's checks and the plan of what is in <work>; changes no host
goatbuilder federate deploy <partner>    # phases 10 to 13 of the approved plan, through the existing roles
goatbuilder federate test <partner>      # phase 14, acceptance, alone
goatbuilder federate status [<partner>]  # what the run journal and the last acceptance evidence say
```

Phases 1 to 9 (the exchange, the agreement, the certificates, the rendered configuration) run
under `goatbuilder federate <partner>`. `plan` does not run them: it runs phase 1's checks and
writes the plan from what those phases left in the working directory. Without a terminal, the
same procedure is:

```bash
goatbuilder federate <partner> --yes --partner-key <base64> --partner-ca-fingerprint <sha256> \
  --agreement-digest <sha256>            # phases 1 to 9; stops at 10
goatbuilder federate plan <partner>
goatbuilder federate deploy <partner> --approve-plan <plan hash> --approve-admission <digest>
goatbuilder federate test <partner>
```

A run that stops for a value it does not have yet (their key at phase 5, the agreement digest at
phase 6, the admission digest at 10) says which, and the same command with it carries on.

You make one decision and answer the run's prompts at phases 5, 6 and 10; the commands do, check
and record the rest. At phase 5 you type the verifying key the partner reads aloud and confirm
that they read their CA fingerprint; at phase 6 you confirm that both agreement digests are the
same (together, confirmation 1 below); at phase 10 you approve the plan's hash and then what your
core admits (confirmation 2). A third human act ends the procedure: publishing `exchanging` is a
reviewable change that both operators approve.

**Do not follow the phases by hand.** A peering stood up by hand on 2026-09-07 needed about a dozen
improvised interventions, and four defects were invisible until the GENERATED artifacts were the
thing deployed. The tables below say what each stage does so you can check it.

### The one decision: who, and what they receive

The decision has two parts, each with one owner:

- **What you offer** is the `release` list in the `[data]` table of your deployment's
  `federation.toml`, the file `init` already reads. It is the only place the offered release sets
  are owned.

  ```toml
  [data]
  release = ["release/all/**", "release/xab/**"]
  ```

- **Who** is a peering entry in your deployment's `goatbuilder.yaml`. It names the partner and
  where your edge runs. It may narrow what this partner receives to a subset of what
  `federation.toml` offers (`offer:` in the entry, or `--offer <SET>[,<SET>...]` on the command);
  `plan` checks that it is a subset and refuses anything wider. It is never a second owner of the
  offer.

  ```yaml
  peerings:
    - with: xab                     # the partner's sovereign id
      environment: dev
      site: alpha
      edge_class: edge
      # offer: [REL-ALL]            # optional: only narrows what federation.toml offers
      # tunnel: wireguard           # or ipsec; the agreement picks the transport from both packages
      # values: {}                  # pins for the per-peering values
  ```

The verified agreement determines what may flow, and which transport carries it: it picks the
transport from both packages' measured capabilities, and phase 11 runs the play for that transport.
There is no `transport` key on a peering. The optional `tunnel` key (`wireguard` or `ipsec`) is this
side's local choice; a `tunnel` that disagrees with the agreement is a configuration to correct,
never an override. Your configuration may narrow what flows or demand stronger checks; it never
widens it, and there is no second hand-kept access list or lane list.
`init`, `lint`, `compose`, `diff`, `verify`, `apply`, `plan`, `deploy`, `test` and `status` are
verbs and can never be a partner name.

### `plan` — validate and explain the change

`plan` checks every input, measures readiness on your own hosts, and prints the change it proposes,
bounded and complete:

- the source and image versions, the agreement digest and the hash of each rendered configuration
  file;
- which resources the peering owns (tunnel interface, ports, link subnet, certificate directory)
  and which services restart;
- the acceptance policy: which checks are required, generated from the agreement's flows;
- conflicts: ports already in use on the edge (the tunnel port NetBird already holds, 51820 by
  default), subnets, interfaces, certificate names and expiry, receive stores on your core that are
  wider than the edge grants (`from/**`), and anything that would touch an existing peering.

The per-peering values are **proposed** as a change to your deployment's own configuration, for
you to review and commit like any other change. Once committed they never move on a re-run; `plan`
validates them.

To produce the agreement, `goatbuilder federate <partner>` runs phases 1 to 9 (`plan` then
reads what they leave in the working directory): it prepares the federation interface with no
peer on it so that it can be measured, measures your half over SSH, exchanges packages and CA
certificates through the directory, composes and verifies the agreement, validates or mints the
peering certificates, and renders the configuration. `apply` stays a renderer: it writes files and
changes no host.

| # | Kind | Phase | Worked when |
|---|---|---|---|
| 1 | GATE | preflight, fail-closed | prints `READY`; a failure names the input, where it was looked for, and its catalog code. When the declaration offers a release set, the inventory group `<peering>_core` names the core host and sits under `[peering_cores:children]` |
| 2 | A | prepare what the package measures: the federation interface and the SPIRE bundle | the interface and its key exist with no peer on them; the bundle is exported when this side runs SPIRE |
| 3 | T | `init` — our half, measured | a signed package; exit 23 means a value could not be derived and the list is the output |
| 4 | T | `lint` | every required member present, signature shape valid |
| 5 | **CONFIRM** | exchange through the directory; read the key and CA fingerprint **aloud** | the directory step uploaded ours and placed theirs and their CA on disk with no FAIL row; on one call each side read its verifying key and CA fingerprint and the other read them back; both keys are recorded |
| 6 | T | `compose` | one digest, and argument order does not change it |
| 7 | T | `verify` | the derived names re-derive; the ingress prefix and republish target are stated |
| 8 | T | validate or mint the peering leaves | four certificates, names derived from the agreement, SANs covering every address that will be dialled: outward, inward, core, and `republisher.pem` with CN `fed.<us>.<them>.republisher.01` |
| 9 | T | `apply --confirm` | the partner declaration, the edge group_vars, the republisher vars (`federation-<them>-republish.yml`) and the ACL fragment, which carries the edge's subject and the republisher's `fed-rp-<them>` subject |

**Confirmation 1 — read the keys aloud, and compare the agreement digest.** After the exchange,
`plan` prints both verifying keys and both CA fingerprints in groups of four and a compatibility
report, and stops for one call with your counterparty:

1. Each side reads its own verifying key (11 groups) and CA fingerprint (16 groups) aloud, group
   count first; the other reads them back against its own screen. Every group must match. A key
   taken from the package it verifies, or from the directory that carried it, proves nothing.
2. The compatibility report has no FAIL row.
3. When both keys match, confirm. `plan` records both keys and composes the agreement; read the
   **agreement digest** to each other the same way. A digest that differs is answered by
   `goatbuilder federate diff`.

If a key does not match, answer no: nothing is recorded or composed. A mismatched key is the attack
this call exists to catch. If the partner's entry is not in the directory yet, `plan` says so; run it
again after they have run theirs.

### Confirmation 2 — approve the plan

Approval is bound to the hash of the plan you read. What to check, beyond the list above, in the
access fragment your **core** will take:

1. Exactly two new subjects and nothing else changed: the edge, by the name derived from the
   agreement (`fed.<you>.<them>.01`), and the republisher, `fed-rp-<them>`
   (`fed.<you>.<them>.republisher.01`).
2. The republisher reads only the release sets you offer, `release/<set>/**`, and writes only
   `from/<you>/**`.
3. The edge receives into `from/<them>/**` and nothing wider; no rule names `**`, `from/**` or
   another partner's prefix.
4. The partner CA the edge will trust has the fingerprint you read back in confirmation 1.
5. The configuration change `plan` proposed (the per-peering values and the rendered
   configuration) has been reviewed and merged through your deployment's normal change process.
   That review is part of this confirmation, and the approval covers it.

If anything the plan covers changes after you approve, the approval no longer applies: run `plan`
again and approve the new hash.

The router's dry run is not something to check before you approve: it runs after the approval.
In order, stopping at the first failure, admission places the staged files (keeping a backup), runs
the core router's dry run (its play with `--check --diff`, or the deployment's own command), checks
the committed slot list against the registry, and restarts the core router. A failed dry run or slot
check puts every placed file back and says so; nothing on the core is live before the restart.

### `deploy` — carry out the approved plan

`deploy` runs phases 10 to 13 through the existing roles and plays. Immediately before it changes
anything it checks the live prerequisites again; if an input changed since the plan, it stops and
asks for a new plan. It takes a lock for this peering, so two runs never act on one peering at once,
and it never removes or changes another peering's resources.

| # | Phase | What `deploy` does | Worked when |
|---|---|---|---|
| 10 | admit the edge on our core | splices the approved fragment into the core's access control, gives the core its peering certificate, restarts the core router and checks it came back | the core runs on the new access configuration |
| 11 | open the transport the AGREEMENT selected | `wireguard` runs `peering-wg.yml`, `ipsec-fips` runs `peering-edge.yml`, with the peering's values; through a forwarder, its entry too | the link is up. It never substitutes one transport for the other, and any other transport refuses |
| 12 | deploy the edge, then the outbound republisher (`peering-zenoh-edge.yml`, then `peering-republisher.yml` with `--limit <peering>_core`) | the roles install the peering certificates on the edge and core host | the edge dials our core and accepts the partner's CN; the container `goat-fed-republisher-<them>` runs on the core host, its `/metrics` on loopback reads `goat_fed_republisher_local_subscribed 1` and `goat_fed_republisher_edge_linked 1`, and its log has one `release release/<set>/** -> wire from/<us>/<set>/**` line per offered set |
| 13 | federate the SPIRE trust domains (ADR 1157 step 2) | runs `peering-spire.yml` | our server refreshes their trust domain across the link, or the play says the step was not done for a side with no SPIRE server |

**Where phase 10 admits.** On a core the goat-zenoh role deploys, admission takes the core
router's inventory group, its `site_id`, the role's play, the org fragment's path and the router
leaf's location from the role's own declarations; nothing needs to be set. For any other core,
the deployment names them in the environment, and admission stops (GB-194), naming the missing
ones, before anything changes:

| Setting | What it names |
|---|---|
| `FED_CORE_ACL_ORG` | the core's org fragment (required): the partner's block is merged into it, and every other vendor and partner is kept |
| `FED_CORE_ACL_SEED` with `FED_CORE_ACL_OUT` | when the core's configuration is assembled: the seed it is assembled from, and where the assembled configuration goes |
| `FED_CORE_CERT`, `FED_CORE_KEY` | where phase 8's core certificate and key are placed; each only when it is set |
| `FED_CORE_PLAY` (on `FED_CORE_GROUP`, default `<partner>_core`) | the play that deploys the core router; its `--check --diff` run is the dry run |
| `FED_CORE_RESTART` with `FED_CORE_DRY_RUN` | instead of a play: the restart command, and the dry-run command (or `skip`) |
| `FED_SLOT_REGISTRY_DB` or `FED_SLOT_PG_SSH` (or `FED_SLOT_CHECK=skip`) | the slot registry the committed slot list is checked against before the restart; not needed, and the check NOT APPLICABLE, when the core keeps no committed slot list |

Each of these also overrides what a goat-zenoh role core would take from the role.

`deploy` keeps three things apart: the intent (your reviewed configuration and the verified
agreement), the progress (the run journal in the working directory: the phase attempts and the
resources this run owns) and the evidence (`<work>/evidence/`). Interrupted, run it again: it
resumes by inspecting the actual state of your hosts, guided by the run journal in the working
directory. It never skips a phase because a file says the phase passed.

### `test` — a bounded acceptance session with the partner

`test` is phase 14. Each operator runs it locally, against their own core and edge only:

1. Both import the same **session manifest**. It binds the agreement digest, the acceptance-policy
   hash, both parties, a unique session id, the schedule, the nonce challenges, limits and an
   expiry. Each side checks it before running anything.
2. Each side runs the checks the agreement's flows generate. Send-only, receive-only and
   asymmetric agreements each get their own set; four lanes is one example, not a rule.
3. Each side writes a result bundle into `<work>/evidence/acceptance/`, and the two operators
   exchange bundles **by file**. Your own observations and the results your partner reports are kept
   apart and labeled; a reported result is not something you observed.

The session, step by step, with `ops/federation/standup/acceptance_session.py`
([federation-acceptance.md §6](../design/federation-acceptance.md#6-a-bilateral-session-exchanged-by-file)):

```bash
S=ops/federation/standup/acceptance_session.py
# one operator writes the manifest, sends it, and both read its digest aloud
python3 $S new --agreement <work>/agreement.json --us <us> --out session.json
python3 $S digest session.json
# each side runs phase 14 in the session
FED_ACCEPTANCE_SESSION=session.json goatbuilder federate test <partner>
# each imports the partner's result.json, then combines both sides
python3 $S import --manifest session.json --us <us> --evidence <work>/evidence/acceptance <their result.json>
python3 $S combine --manifest session.json --us <us> --evidence <work>/evidence/acceptance
# after both operators reviewed the combined result: the reviewed partner-file change
python3 $S propose --manifest session.json --us <us> --evidence <work>/evidence/acceptance \
  --partner-file <overlay>/ops/federation/partners/<them>.yaml \
  --approved-by <us>=<name> --approved-by <them>=<name> --out <dir>
```

`combine` writes `<evidence>/bilateral/<session id>.json` and `.md`, with your rows and the
partner's side by side, and exits 0 only when the required scope of both sides passed. `propose`
writes the partner file with its `acceptance:` block and a diff into `<dir>`, never the
deployment's own file, and refuses unless the combined result passed
([§7](../design/federation-acceptance.md#7-publishing-exchanging)).

| Check | Required evidence |
|---|---|
| Fresh delivery, every agreed direction and set | a unique run id, nonce and payload digest, published through release staging and read back exactly at the receiving core |
| Persistent reception | a new sample arrives with every test subscriber stopped, so no diagnostic subscription can mask a missing store |
| Out-of-lane refusal | an allowed control and a prohibited write from the same test identity, matched in the receiving edge's allow and deny records; silence or an HTTP success is not evidence |
| Republisher boundary | a release set that is not offered, staged beside a valid control, is not lifted |
| Round trip | only when the agreement has a return path; it adds to delivery evidence, never replaces it |
| Identity and SPIRE | the approved negative test certificates and the SPIRE checks, when the profile requires them |
| Existing partnerships | each other peering checked before and after, with no release leaking across partners |

Each check reads one of five outcomes:

| Outcome | Means |
|---|---|
| PASS | the expected evidence was observed |
| FAIL | a violation was observed |
| BLOCKED | a prerequisite or the evidence was unavailable |
| SKIP | the check was not exercised |
| NOT APPLICABLE | excluded by a validated, agreed scope |

A required check passes only on PASS. A SKIP or BLOCKED required check means not ready, never a
pass; unavailable evidence never counts as a refusal that worked; and no one can waive a required
FAIL into PASS.

Probes are time-limited, use reserved acceptance key spaces, and are owned by the run that made
them. `test` removes only what it created; cleanup that did not finish is itself a required FAIL.
It never stops a production edge, core or tunnel.

**Publishing `exchanging`.** Only after the whole required scope passes on both sides, cleanup is
confirmed, and both operators approve the result, is `exchanging` published, and only as a
reviewable change through your normal configuration process. The two bundles are reviewed by
people; nothing gates on the partner's report automatically.

### `status` — the run journal and the evidence, with its age

`goatbuilder federate status` lists every declared peering; `goatbuilder federate status <partner>`
shows one. It reads the peering's run journal and its evidence in the working directory, and
nothing else: it writes nothing, takes no lock and reaches no host. For each peering it shows the
last run and how it ended (`in progress` or `killed` for a run with no end, by whether the process
in its lock file is alive), each phase's last outcome, the confirmations given, the resources it
holds on shared hosts, the digest of the agreement in the working directory, the last acceptance
evidence with its verdict, age and counts by outcome word, and the command that carries the
peering on.

The evidence is `stale` only when it was produced against another agreement than the one in the
working directory now, and `unknown` when either digest is missing. `status` does not measure the
deployed edge's live health, does not show the offer, and does not say whether a stop waits for
you or for the partner; the stop's own message and its fix say that.

### When a phase fails

The stage stops at the failed phase, runs nothing after it, and prints the phase, the error code
from `ops/errors/catalog.yaml` and the fix. Fix the cause and run the same command again.
`goatbuilder explain <code>` prints the catalog entry.

| Phase | It failed because | What to do |
|---|---|---|
| 1 | an input is missing or a host is not ready | supply the value it names, in the place it names. A port in use: pin another under `values:` or free it. A receive store wider than the edge grants: declare one per admitted scope, `from/<them>/**` ([receive-storage preflight](#receive-storage-preflight)) |
| 2 | the edge play failed | read the play's failed task |
| 3 | exit 23, a value could not be derived | the output lists each value and why; fix the measurement, never type the value |
| 4 | exit 21, the package does not lint | the rule is named; it is a defect in the package |
| 5 | the partner's entry is missing, or a FAIL row | wait for them, or fix the row on the side that owns it |
| 6 | exit 20, composition refused | a key is not bound or the packages disagree; compare with `goatbuilder federate diff` |
| 7 | the agreement does not verify from your side | the names it cannot re-derive are listed; do not continue |
| 8 | a certificate pair is invalid or changed | inspect it ([phase 8 reruns](#phase-8-reruns-and-deliberate-certificate-replacement)); do not delete files to get past it |
| 9 | apply refused | its message names the field |
| 10 | the router's dry run refused, or the core did not come back | if the dry run refused, nothing restarted; otherwise the core's previous access configuration is the way back |
| 11 | refused: no play for the selected transport | the fix is in the agreement, never the play |
| 12 | the edge does not link to your core, or the republisher is not subscribed | read the edge log on your side; the [symptom table](#if-something-does-not-work) |
| 13 | the SPIRE play failed | check the link from phase 11 is up |
| 14 | a required check is not PASS | the bundle names the check and its evidence; read the router log on the **receiving** side |

The six package verbs, `goatbuilder federate init|lint|compose|diff|verify|apply`, keep the exit
codes of `goat federate`: `20` refused · `21` malformed · `22` digest mismatch · `23` underived ·
`24` not conformant. Packages and agreements are identical whichever tool made them; see
[The earlier method](#the-earlier-method) for `goat federate <verb>`.

## The earlier method

Before 0.5.0, federation ran through `goat federate <verb>` and by hand around it. The operator decided on
11 November 2026:

1. **`goat federate <verb>` is sunset.** In 0.5.0 it still runs and prints one line naming the
   `goatbuilder federate` command. It is removed from `goat` 30 days after the 0.5.0 release, and not
   before one peering between two standing environments has been completed with
   `goatbuilder federate`. After that, `goat federate` answers only with an error that names
   `goatbuilder federate`.
2. **The script driver stays**, as the script avenue below describes: documented, not supported
   ([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)), running the same
   fourteen phases and held identical by the parity gate.
3. **One method is taught.** This runbook, the tutorials and the reference teach only
   `goatbuilder federate`.

### The script avenue

[ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md) keeps a second avenue:
`ops/scripts/peering-standup.sh` runs the same fourteen phases with the same numbers, kept identical
by the parity gate `ops/goatbuilder/parity-federate.sh`. It is documented, and it is not the
supported path. It stops at phases 5 and 10 and resumes by number; you record the keys by hand.

```bash
ops/scripts/peering-standup.sh <peering> --plan     # prints every phase, runs nothing
ops/scripts/peering-standup.sh <peering>            # exchanges through the directory, stops at phase 5
ops/scripts/peering-standup.sh <peering> --from 6   # resume after the call; stops at phase 10
ops/scripts/peering-standup.sh <peering> --from 11  # resume after the review
```

It reads its inputs from the environment; phase 1 refuses without the first three:

| Variable | Value |
|---|---|
| `FED_DIRECTORY_URL` | the peering directory, `https://…`, the same one your counterparty uses |
| `FED_COUNTERPARTY_ID` | the counterparty's `sovereign_id` (ISO 3166-1 alpha-3) |
| `FED_SIGNING_KEY` | the key that signs your package; it also signs your directory entry and uploads |
| `FED_CA_CERT` | your CA certificate (PEM), the one phase 8 signs the peering leaves with; unset, the profile's `ca-roots.pem` |
| `FED_ROUTER_DOCKER_HOST` | Docker endpoint for a separate core, e.g. `ssh://operator@core.example.org`; phase 3 sets `DOCKER_HOST` for the router reads while interface measurements remain on the operator/edge host |
| `FED_ROUTER_CONTAINER` | optional exported core container name or ID for multiple cores or older custom layouts; unset, phase 3 discovers one running labeled, standard-named or legacy core |
| `FED_PACKAGE_URL` | **leave unset**: the directory hosts your package. Set it only to serve the package from a public host of your own, behind TLS, never on a mesh |
| `FED_NO_DIRECTORY` | `1` only for a counterparty that can reach no directory; phase 5 then names the files to hand over |

At the phase 5 stop, read the keys aloud as in confirmation 1, write both to `$FED_WORK/keys.txt`,
one `sovereign_id=key` per line, and resume with `--from 6`. At the phase 10 stop, review what apply
rendered as in confirmation 2, splice `<work>/applied/ops/federation/acl/<them>-fragment.json5` into
your core's access control with `roles/goat-zenoh/files/acl-assemble.py`, give the core
`<work>/pki/core.pem` and its key, restart it, and resume with `--from 11`.

## Step 2 — The names and prefixes come from the agreement

None of these is asked of the partner. Each is in a package or derived by `compose`, and both
sides derive the same values from the same agreement.

| Value | Example | Whose |
|---|---|---|
| Your sovereign identifier | `xab` | yours, in your package |
| The name your edge router presents to them | `fed.xab.xaa.01` | derived from the agreement (`peering_cn`) |
| Where their data lands on your side | `from/xaa/**` | derived from the agreement |
| What you will send them | `release/all/**` — the **staging** name, internal to you | yours |
| The name that data carries once it crosses | `from/<your sovereign>/**` — the **wire** name. Your republisher, on your core host, lifts `release/<set>/<rest>` onto `from/<your sovereign>/<set>/<rest>` before anything reaches the edge, every set under the one prefix; it is what the counterparty's ingress prefix admits | derived |

Their values mirror these. **The name is load-bearing**: the other side's permission rules
will match it exactly, character for character, which is why it is derived rather than typed. A
change to a `sovereign_id` changes every derived name and means a new agreement on both sides.

Do **not** choose a name that already exists in your own fabric. Their router presenting a name
your own routers use is the failure mode this whole design is built to prevent.

## Step 3 — Exchange packages and CA certificates through the directory

Phase 5 does the exchange. Neither side sends a file, and no private key is exchanged in any
step of this procedure. If anyone asks you to send a private key, the procedure is being done
wrong.

The directory step (`ops/federation/standup/directory.py`):

1. publishes your pointer entry, signed with your package key; unless you serve the package from
   a public host of your own (`FED_PACKAGE_URL`), it uploads your package to
   `<directory>/v1/parties/<you>/package` and the pointer names that;
2. uploads your CA certificate to `<directory>/v1/parties/<you>/ca`: only the certificate whose
   SHA-256 is your package's `identity.ca_fingerprint`;
3. looks up the counterparty's pointer, fetches their package, checks its sha256 against the
   pointer and its signatures, and places it where phase 6 reads it; fetches their CA
   certificate, checks it against their `identity.ca_fingerprint`, and places it at
   `<work>/theirs-ca.pem`;
4. prints both verifying keys and both CA fingerprints **in groups of four**, for the call;
5. prints a **compatibility report** from the two packages: sovereign ids, spec, fabric and tool
   version, the transport the agreement selects, FIPS fabric image, PQC capability, trust
   domains, inbound reachability and expiry, each PASS, WARN, FAIL or UNKNOWN with the fix.
   Nothing in it is asked of the counterparty by message.

Today the directory holds one package per party, so a party's second peering replaces the first
one's package. Keeping one package per peering is part of this program and arrives with its own
default switch; until then, stand up one peering per party at a time.

If their entry is not there yet, run the command again after they run theirs. A directory that is
down or disagrees with a package is reported, never trusted: `compose` checks their package
against the key you read back, never against anything the directory served.

**The call.** This is confirmation 1 ([Step 1, `plan`](#plan--validate-and-explain-the-change)).
Each side reads its verifying key (11 groups) and CA fingerprint (16 groups) aloud, group count
first; the other reads them back against what phase 5 printed. When both match, you confirm and
the command records both keys and composes. After `compose`, read the agreement digest to each
other the same way. Those reads, the review in confirmation 2, and the decision about what data
flows are the only acts that are not machine-driven.

When both sides run SPIRE federation (phase 13), phase 5 also uploads the SPIFFE trust bundle
phase 2 exported and fetches theirs to `<work>/theirs.spiffe-bundle`. Nothing in the package
names the bundle, so the step checks it against the signature of the upload that stored it,
under their verifying key (saltlick `SPEC.md`, "Checking a hosted file against the key read
aloud"), and places it only when that holds. After that first exchange, refresh is automatic:
each side re-pulls the other's published bundle on a timer, so **the other side replacing their
certificate authority does not break the link**.

**No directory.** For a counterparty that can reach no directory (`FED_NO_DIRECTORY=1`), phase 5
names the files to hand over directly; the call is unchanged.

## Step 4 — Stand up your edge router

One process, dedicated to this partner. It is the same router software you already run, started
with a different configuration.

Set these, and nothing else:

| Setting | What it is |
|---|---|
| `goat_zenoh_edge_partner_ca_path` | **their** CA material — and only theirs |
| `goat_zenoh_edge_partner_cn` | the name from Step 2 that they will present |
| `goat_zenoh_edge_ingress_key_exprs` | where their data may land, e.g. `from/xab/**` |
| `goat_zenoh_edge_egress_key_exprs` | what they may receive, named as it appears **on the wire**: `from/<our sovereign>/**` — never the `release/**` staging name |
| `goat_zenoh_edge_core_endpoints` | your own core, which this edge dials inward to |
| `goat_zenoh_edge_outward_cert` / `_key` | what you present to them |
| `goat_zenoh_edge_inward_cert` / `_key` | what you present to your own core |

**Or let the role install the certificates.** With `goat_zenoh_edge_install_certs: true`, leave
the three certificate rows above unset: phase 12 checks the phase 8 leaves and their CA from
`$FED_WORK` (keys match, leaves chain to your CA, their CA's fingerprint equals the agreement's)
and installs them in the peering's own directory on the edge, `goat_zenoh_edge_cert_dir`. Nothing
is copied by hand. See [roles/goat-zenoh-edge/README.md](../../ops/ansible/roles/goat-zenoh-edge/README.md).

**Do not point the trust refresh at your core router.** It must write to the edge router's trust
file. Pointing it at the core puts a foreign certificate authority on the machine that holds
everything — the single most damaging mistake available in this procedure.

**Do not reuse your core router's configuration as a starting point.** Use the edge template. The
permission engine stops at the first rule that allows something, so a permissive rule inherited
from your core cannot be taken back by adding a narrower one afterwards.

### Phase 8 reruns and deliberate certificate replacement

Phase 8 validates and reuses existing certificate/key pairs. A successful rerun
preserves their bytes and modification times; the CA private key may remain
offline when every requested pair already exists and validates. Missing complete
pairs are issued only when the own CA signing key is available.

Validation binds the own CA fingerprint to the agreement and covers the validity period, exact derived CN and SANs, client
and server EKUs, non-CA status, key usage, matching public keys and private-key
mode 0600. The default remaining-validity minimum is 86400 seconds; configure
`FED_LEAF_MIN_VALID_SECONDS` explicitly if the deployment requires another window.
An incomplete, invalid or changed pair refuses the run before any leaf is
replaced. Inspect the discrepancy rather than deleting files to bypass it.

For intentional reissuance, including expiry, invoke the helper explicitly with
`--rotate`, using the same reviewed agreement, own CA and core SAN inputs:

```bash
ops/federation/standup/mint-peering-leaves.sh \
  "$FED_WORK/agreement.json" "$FED_WORK/pki" --as "$OUR_ID" \
  --core-cn "$FED_CORE_CN" --core-san "$FED_CORE_SAN" --rotate
```

The signing inputs come from `FED_CA_CERT` and `FED_CA_KEY`, or, without a key
file, from the deployment's age-encrypted key (`FED_CA_KEY_AGE` with
`FED_AGE_IDENTITY`), which the script opens for this run only when the
deployment opts in (`FED_CA_KEY_FROM_AGE=1`). Rotation
replaces every requested pair with a new key; keep that action separate from an
ordinary phase rerun. Previous material is retained in a private `.previous-*`
directory under the output directory. Protect and retire it through the local key
retention policy. This command prepares files; it does not deploy them or restart
services. Plan and review the corresponding deployment separately.

Operations on one output directory are locked. All candidates are staged and
validated before installation, with rollback on ordinary installation errors.
Installation across several files is not crash-atomic: after abrupt interruption,
an outstanding `.rotation-pending` marker blocks further operations. Restore or
complete the reviewed set, verify it, and only then remove the marker. Incomplete
or mismatched pairs are also refused. See [Federation certificate lifecycle](../design/federation-certificate-lifecycle.md).

### The republisher, on your core host

Phase 12 deploys it with `peering-republisher.yml` on the inventory group `<peering>_core`, which
names the core host your edge dials. The agreement's values (`goat_fed_republisher_sovereign_id`,
`_partner_id`, `_cn`, `_publish_under`, `_release`) come from `apply`. Set these in
`group_vars/<peering>_core/`, and nothing else:

| Setting | What it is |
|---|---|
| `goat_fed_republisher_image` | the image, digest-pinned: `components.goat-fed-republisher.fleet` in `ops/pins/fleet-pins.yaml` |
| `goat_fed_republisher_core_endpoint` | your core router as this host dials it, e.g. `tls/<core address>:7447`, with no `#` fragment |
| `goat_fed_republisher_core_ca_src` | **your** CA, the roots your core's certificate chains to, as a path on the machine running the driver |

Its certificate is phase 8's `republisher.pem`, and the agreement is phase 6's; the role copies both
from the working directory. Your core admits it through the fragment's `fed-rp-<them>` subject: read
the offered `release/<set>/**`, write `from/<you>/**`, nothing else. A core configured from
`roles/goat-zenoh` carries the same grant through `zenoh_sovereign_peerings[].republisher_cn` and
`republisher_release_key_exprs`.

Its `edge_linked` and `dropped_total{reason="edge_down"}` metrics describe its session to **your
core**. They do not see the partner link; phase 14's round trip does.

To stop your data crossing at once without touching the edge, stop the container
`goat-fed-republisher-<them>` on the core host: nothing else writes your wire name.

## Step 5 — Open the tunnel

One encrypted tunnel between the two edge hosts. It belongs to neither side's internal network:
do not enrol the other party's machine into your own network management, and do not join theirs.

Firewall it to exactly this, on the edge host:

- **inbound**: the tunnel port, from their address only. Everything else denied and logged.
- **outbound**: the tunnel port, to their address only.
- **nothing else crosses** — no management, no shell, no administrative interface.

**Behind a forwarder** (status: built, not yet run between two goatnets), the side that listens
receives the tunnel port on its carrier interface (`wg-carry0`), not from the partner's address,
and its only outbound is the carrier port to the forwarder. The side that dials sends to the
forwarder's address. The forwarder is up before step 1
([`peering-forwarder.md`](../design/peering-forwarder.md) §7):

| Host | Play | Variables |
|---|---|---|
| Forwarder (group `peering_forwarders`) | `ops/ansible/peering-forwarder.yml` (`roles/goat-peering-forwarder`) | `peering_forwarder_enabled: true`; `peering_forwarder_peerings`, one entry per peering: `name`, `edge_carrier_public_key`, optional `partner_sources`. Ports 51820/51821 and carrier 169.254.30.0/31 are defaults |
| Our edge | `peering-prepare.yml` and `peering-wg.yml` (`roles/goat-peering-wg`) | `peering_wg_via_forwarder: true`, `peering_wg_forwarder_carrier_public_key`, `peering_wg_forwarder_endpoint: <forwarder>:51821` |

With `peering_wg_via_forwarder: true` the edge's firewall binds the tunnel port to `wg-carry0`,
and phase 11 refuses unless this side listens and the link MTU is at most 1340. Offer
`measured_mtu = 1300`. Declare the forwarder under `forwarder:` in the partner file.

## Step 6 — Sealed data for this partner, if they need it

**Skip this unless a release set has to be readable only by a named audience.** Unsealed exchange
is the default: the lane the agreement grants is already partitioned and mutually authenticated, and
`grant-keyed-encryption.md` §0a is explicit that using the mechanism is the exception rather than
the norm. Naming no audience renders no rule, which is a deliberate configuration and not a gap.

When a partner does need it, three things have to line up.

**1. Admit their trust domain.** A federated partner's identity is in THEIR SPIFFE trust domain, so
`grant-keyd` refuses it until the domain is named:

```json
"federated_trust_domains": ["xaa.fed.lab"],
"release_sets": { "weather": ["member/a", "spiffe://xaa.fed.lab/member/partner"] }
```

A member id written as a full SPIFFE URI is a federated identity, used verbatim; a bare id is one of
ours. Their certificate lives at `<cert_dir>/federated/<their domain>/<path>.pem`. Admitting a
domain lifts exactly one check — the SVID still has to chain to a root in the trust bundle, still
has to be time-valid, and still has to match its id exactly.

**Admission is deliberate and separate from federating trust bundles.** The bundle carries their
root because two SPIRE servers federate, which authenticates the LINK. It says nothing about who may
read sealed data, and ADR 1169 keeps that decision entirely ours.

**2. Name the audience when you apply.**

```bash
goatbuilder federate apply <agreement> --as <us> --edge-group <group> --partner-audience weather
```

The verb takes the same flags `goat federate apply` had.

This renders `goat_zenoh_edge_key_bundle_key_exprs: ["keys/weather/**"]` — scoped per release set,
never `keys/**`, which would hand them every audience's bundle. Withdrawing the scope is re-running
apply without that set.

**3. Narrow the core's key storage to match.** This is the one that will catch you. A queryable
declared wider than the granted lane is refused **silently**, and the natural configuration stores
every set under one `keys/**` storage:

| Core storage | Partner fetch |
|---|---|
| `keys/**` | `[]` — the declaration is wider than the grant and never reaches them |
| `keys/weather/**` | the bundle |

A goatnet serving a federated partner needs a key storage per released release-set
(F-499).

**Worked when:** the partner queries `keys/<set>/**` and gets a bundle whose `members` include their
SPIFFE id. With no audience named the same query returns `[]`.

## Step 7 — Prove it before you trust it

Run all five. **Four of them are supposed to fail**, and a failure that does not happen is the
finding.

| # | Do this | Expected |
|---|---|---|
| 1 | They publish to the agreed landing prefix | **Arrives**, and is stored |
| 2 | They publish somewhere else | **Refused** |
| 3 | They present a name you did not agree | **Refused** |
| 4 | They present a name one of *your* routers uses | **Refused** |
| 5 | Their side subscribes to something not in the agreed list | **Nothing delivered** |

> **Check the router's log, not the absence of data.** When this software refuses something it
> does so silently — the sender is told nothing. "Nothing arrived" and "it was refused" look
> identical from outside, so only the log distinguishes a working control from a broken link.

Case 4 is the important one. Nothing stops your counterpart's certificate authority issuing a
certificate bearing one of your router's names, and the design assumes they can. Prove it is
refused.

## Step 8 — The round trip, and it is the last thing you do

**Every check in Step 7 needs shell on both sides.** You read their store, you read their log. A
real publisher has neither, and none of those checks is available to them.

The round trip is the one check the sending side can make alone. Stage a nonce under one of your
release sets, let your republisher lift it onto your wire name, and wait for their acknowledgement
to come back under theirs:

```bash
# on their side, once: watch for your probes, stage the ack under one of THEIR sets
python3 ops/federation/roundtrip/responder.py from/<you> release/<one of their sets>

# on yours, once per set you offer (phase 14 does this for every set)
ops/federation/roundtrip/fedcheck.sh release/<one of your sets> from/<them>
```

A pass means the sample crossed, **their own router delivered it to a process on their side**, the
payload arrived with the digest you sent, and the answer came back over the same agreement. It uses
no access the agreement does not already grant — the probe goes out on your wire name and the ack
returns on their landing prefix, which are the two lanes you just built. Because the probe is
staged, a pass also proves your republisher lifted that set; with the republisher stopped it fails.

Run it last, because it is the only stage that fails when an earlier one silently half-works. A
peering can be ESTABLISHED, fully authenticated, and carrying nothing at all with no denial logged
anywhere (F-493,
F-494,
F-177). One-way checks pass
in some of those states. This one does not.

Measured on a live two-region peering 2026-09-07: **PASS in 538 ms** healthy; **FAIL in 8 s** with
the partner's responder stopped; **FAIL in 8 s** with the edge stopped; **PASS again** on restore.
The failures are the point — a check that cannot fail is not a check
(F-478).

## Step 9 — Ending it

Remove them from the tunnel on your own edge host. It takes effect immediately, cuts sessions
already open, and needs no cooperation from them.

Then, in this order:

1. Remove their trust domain from the identity service **first** — otherwise the refresh timer
   re-adds their certificate authority within minutes and quietly undoes you.
2. Remove their CA from the edge router's configuration and re-apply.
3. Restart the edge router.
4. On a core configured from `roles/goat-zenoh`, review the orphan-store policy,
   remove their entry from `zenoh_sovereign_peerings`, and apply the reviewed
   core change. The per-partner directory `<storage path>/sovereign_from_<them>`
   remains on disk and is listed in `.undeclared-stores`; the role never deletes
   it. Historical copies can also remain in the shared legacy `sovereign_from`
   store. Follow the agreed retention/deletion procedure for all copies, while
   preserving other partners' data. With `zenoh_orphan_stores_fail: true`, an
   approved disposition or migration is needed before the deploy can proceed.

There is no certificate revocation list and no way to revoke a single certificate. Cutting the
tunnel is the control that actually works; the rest is cleanup.

### Step 9 as one command

`goatbuilder federate backout <them> --dry-run` lists, per host, every resource of this one
peering that the steps above remove, and the digest of the core configuration it would restart
on. Nothing changes. Without `--dry-run` it does Step 9 in the order above, then withdraws our
package for them from the directory and deletes the generated partner record for review. Other
peerings on the same hosts are left as they were. It does not touch stored partner data: the
retention step in item 4 is still yours. The `peerings:` entry, a forwarder's
`peering_forwarder_peerings` entry and a `zenoh_sovereign_peerings` entry are configuration you
remove by review. [goatbuilder.md](../design/goatbuilder.md#federation-modules-and-their-seams)
describes the steps and how each resource is matched to this one peering.

---

## Receive-storage preflight

The edge role checks each configured `goat_zenoh_edge_ingress_key_exprs` value
against the core's storage declarations. It reads the core router's configuration
on the edge host, or on our core host, the one host in inventory group
`<partner>_core` (`goat_zenoh_edge_core_group`).
Every required scope must have an exact matching receive store. This accepts the
legacy-plus-scoped configuration introduced by #235 and preserves its existing
history. It does not remove stores, migrate data, alter ACLs or restart the core.

A readable configuration with missing scopes or an unsupported format fails with
an actionable message. A core that cannot be found or read fails the play as
**could not run**; it is never a pass. Setting
`goat_zenoh_edge_ingress_scope_check: false` reports SKIP and does not inspect the
core: use it only to accept an unverified receive store deliberately. Actual fresh delivery and persistence still require the acceptance
checks; a configured store is not evidence that traffic arrived.

The implementation and its limits are described in
[Federation ingress storage preflight](../design/federation-ingress-storage-preflight.md).

## If something does not work

| Symptom | Most likely cause |
|---|---|
| Connection refused immediately | Their certificate does not chain to the CA you installed. Re-check Step 3, and check you installed *their* CA and not your own. |
| Connects, but nothing arrives | The name they present does not exactly match `goat_zenoh_edge_partner_cn`. Matching is exact — check case, hyphens and whitespace. |
| Arrives but is not stored | Your core has no storage declared for the landing prefix. Data is being routed and discarded. |
| Session up, they publish, nothing reaches your core, and nothing is logged on their side | Check for a receive store matching each admitted ingress scope. A lone `from/**` store declares a subscriber wider than a scoped edge grant and is refused. A matching `from/<them>/**` store, or matching per-lane stores for per-lane grants, supplies the required declarations. A retained legacy store can coexist with them. The edge preflight checks an inspectable local core and explicitly reports SKIP when it cannot read one; inspect a separate core through its own approved workflow. |
| Worked, then stopped after weeks | A certificate expired, or their CA rotated and the refresh is not running. Check the refresh timer first. |
| Works in one direction only | The two sides' agreed prefixes do not mirror. Compare Step 2 values side by side. |

**When diagnosing, look at the router log on the receiving side.** A refusal is only visible
there. This is the single most common way to waste an afternoon on this procedure.

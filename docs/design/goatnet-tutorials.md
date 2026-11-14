# Goatnet tutorials — a goatnet from nothing, its first federated node, its boundary node

| Field | Value |
|---|---|
| Status | **All three tutorials built and rehearsed on the fixture** (1 and 2 on 2026-10-28, 3 on 2026-10-30); no tutorial yet run on hardware |
| Decision | [ADR 1202](../adr/1202-goatnet-tutorial-program.md) |
| Readers | Platform admins standing up or extending a goatnet; teams outside the project once the repository is open |
| Tools taught | `goatbuilder` (supported) with the scripts beside it ([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)) |

## What this is

Three tutorials, taken in order, that take a platform admin from nothing to a
goatnet, preceded by a tutorial 0 for a learner's first LAN goatnet (goatlite), that can share data with an ally or partner. Each is rehearsed first on
a worked example that runs on a laptop in under a minute, then run on hardware
with the same verbs. Each ends at checkpoints that are the same proofs the
deployment e2e harness reads: the hosts' own records, never a play's report.

| # | Tutorial | Page | Status |
|---|---|---|---|
| 0 | goatlite: a first goatnet on one LAN box, from one file (the learner's rung before the founder site) | `docs/tutorials/00-goatlite.md` | **written** 2026-11-02; rehearsal on the fixture `goatnet-from-scratch/00-goatlite`; hardware drill not run; WIP ([goatlite.md](goatlite.md), ADR 1213) |
| 1 | A new goatnet, from nothing: its founder site | `docs/tutorials/01-new-goatnet.md` | **built**; rehearsal **measured** on the fixture |
| 2 | The first federated node: a joiner site attached to the founder | `docs/tutorials/02-first-federated-node.md` | **built**; rehearsal **measured** on the fixture |
| 3 | A boundary node for ally and partner data sharing across environments | `docs/tutorials/03-boundary-node.md` | **built**; rehearsal **measured** on the fixture |
| 4 | Two sovereigns, end to end: run both sides of a peering yourself | `docs/tutorials/04-two-sovereigns.md` | **written** 2026-11-02; **measured** in CI and on two real hosts |

## Where each part lives

The operator's direction: "the core of this should live in this monorepo (and
the forthcoming open source repo) but the particular example implementation
that is proven e2e will live in a hello galaxy repo".

| Part | Home | Why there |
|---|---|---|
| Tutorial text | `docs/tutorials/` in this repository; served in the goatbuilder book at `docs.<site>/goatbuilder/`; cited from the gym's platform-operator lane | It teaches the tool this repository ships and versions with it |
| The fixtures' shape and the rehearsal semantics | `goatbuilder standup --mock` (`crates/goatbuilder/src/standup.rs`) | The tool's own conformance surface |
| The checks that keep the text true | `ops/tutorials/check.sh` and the `goatnet_from_scratch_*` tests | They gate this repository's pull requests |
| The worked example | the Hello Galaxy series: `USAREUR-AF/hello-galaxy-goatenv` (`./run` rehearses the three tutorials against its checkpoints). This repository keeps the same directories at `ops/goatbuilder/fixtures/goatnet-from-scratch/` for its own checks; `ops/tutorials/sync-goatnet-from-scratch.sh` keeps the two equal | The series is where every runnable, end-to-end proven example lives; a reader clones one example without the whole platform |

The worked example holds the goatnet's declaration (`goatbuilder.yaml`), its
digest-pinned refs, the scripted converger, the records each step starts from,
and the recorded output of every rehearsal (`checkpoints/`). Tutorial 2's
directory starts from tutorial 1's end state, so the example is one goatnet
growing, not two unrelated fixtures.

## The tutorials

### 1. A new goatnet, from nothing

Plain: the reader stands up the founder site, the one that originates the
goatnet's account, certificate authority and access policy.

| | |
|---|---|
| Audience | A platform admin standing up a goatnet for the first time |
| Prerequisites | Hardware per the host-site baseline (one core host is enough to start); the site's configuration written from the nearest existing site; the goatnet's pins row, ladder rung and door; secret references; the tools `doctor` names |
| Commands | `goatbuilder validate`; `doctor --env <env>`; `standup <env> --site <site> --dry-run`; the same with `--yes` under `ops/scripts/with-secrets` (the declared secrets posture, ADR 1204); `--from <phase>` to resume after phases 10 and 12m. Scripts: `goat-standup.sh <site> --plan`, then without `--plan` |
| Checkpoints | (1) the rehearsal matches `checkpoints/01-new-goatnet.txt`, the assay's plan for the founder included; (2) `goatbuilder assay <site>` (phase 13) has every applicable row `PASS`, layer by layer ([goatnet-acceptance-suite.md](goatnet-acceptance-suite.md) §"The assay"); (3) `goatbuilder status --env <env>` shows every record naming its declared ref, and the new goatnet's card appears on another goatnet's overview |
| Done | Checkpoints 2 and 3 on hardware |
| Undo | Re-run from the failed phase; retirement runs the last phase in reverse |

### 2. The first federated node

Plain: a second site joins the same goatnet as a **joiner**. It receives the
founder's account by replication, and its standup adds the founder's half of
the replication so writes flow both ways.

| | |
|---|---|
| Audience | A platform admin who has finished tutorial 1 |
| Prerequisites | A founder whose gate is green; the joiner's configuration with `goatnet_founder: false` and `join_founder_site`; the founder's configuration naming the joiner as a replication peer; the pins row's member group; signal federation declared on both sides, the founder's relay pin and the federation DNS records ([signal-federation-pair.md](signal-federation-pair.md)) |
| Commands | `standup <env> --site <joiner-site> --dry-run` then `--yes`; `join-node <env> --site <site> --node <n> --class <c>` for each further machine. Scripts: `goat-standup.sh <joiner-site>`; the node-addition runbook |
| Checkpoints | (1) the rehearsal matches `checkpoints/02-first-federated-node.txt`, including the founder-side mirror recorded on the founder's machines; (2) the joiner's gate green from the joiner's network; (3) every subscription `replicating` both ways, a founder write visible on the joiner, a deletion replicated back; `signal-federation-verify.yml` passing; (4) `status` across both sites |
| Done | Checkpoints 2–4 on hardware |
| Undo | Re-run from the failed phase; removing a joiner reverses the founder-side mirror in the same change |

Federating with a goatnet someone else owns is not this tutorial; it is
tutorial 3's ground.

### 3. A boundary node for ally and partner data sharing

Plain: a boundary node lets a goatnet release chosen data to an ally or partner
who runs their own goatnet, under their own authority, without either side
joining the other's network or vouching for the other's identities. It is the
sovereign edge router — one dedicated router process per partner, deny-by-default,
trusting only that partner's bundle, never the core router with a foreign root
added ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md),
[ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md),
F-424).

| | |
|---|---|
| Audience | A platform admin who has finished tutorial 2, with a counterparty ready to peer |
| Prerequisites | A goatnet whose gate is green; an edge host in a `--edge-group`; the peering declared when `goatbuilder.yaml` keeps peerings; a peering directory both sides reach (`FED_DIRECTORY_URL`); a FIPS-mode fabric image on both edge routers |
| Commands | `federate <peering> --env <env> --as <ours> --dry-run` then without `--dry-run`; it stops at phases 5 and 10 (the exchange through the peering directory with the key read-back, and the review of what was rendered) and resumes with `--from 6` and `--from 11`. Scripts: `peering-standup.sh <peering> --plan`, then without `--plan` |
| Checkpoints | (1) the rehearsal matches `checkpoints/03-boundary-node.txt` — the plan of all fourteen phases; (2) phase 14 acceptance green: a sample **carries** in the agreed direction, a write outside the agreed prefix is **refused** (live fire, not by reading config — F-177), and a nonce **round-trips** through the counterparty's own router |
| Done | Checkpoint 2 on a live peering |
| Undo | Cut the peering on your own edge host; the partner's certificate-authority file is the revocation instrument (`cross-sovereign-federation.md` §7) |

It wraps `goatbuilder federate` (`crates/goatbuilder/src/federate/`), the
fourteen-phase peering verb that subsumes `ops/scripts/peering-standup.sh`. It
needs no new standup class: the boundary node is the router `federate` already
deploys. What crosses is release-set namespaces the owner deliberately shares; a
partner writes only under `from/<their-own-code>/` and reads only the declared
release prefixes, and a message meant for certain readers is sealed to them by
grant encryption so a mis-forwarded copy stays unreadable (ADR 1208).

**Two things differ from tutorials 1 and 2.** Its rehearsal is a *planned* run,
`federate --dry-run`: a peering's far side is another sovereign's goatnet, so
there is no mock converger to stand one up. The worked example is still a flat
`03-boundary-node/` fixture like 01 and 02 — it declares the `hello-ally`
peering, and `federate` reads it through `--mock` (which it now honors, like
every other verb), so the plan is the one that declaration produces. And the
`Done` step is a live peering, which needs a counterparty — the one tutorial a
reader cannot finish alone.

**Honest state.** The peering flow is proven between two purpose-built
environments and fired both ways (M-113,
2026-09-05); it has not run against a live goatnet, the sovereign-federation
runbook has not been executed against one, and grant encryption across the
boundary is tracked under F-481.
The tutorial states this so a reader's first live run holds no surprises.

This tutorial is the light `federate` step of the builder arc, rehearsed on the
fixture; the full cross-environment federation exercise — a second goatnet, real
package exchange, a live gate — is the standalone repository
`hello-galaxy-cross-env-federation`, which runs the same flow end to
end. They coexist: the builder arc proves the verb, the standalone repo proves
the whole exchange.

## How a tutorial is kept true

1. **On every pull request.** `ops/tutorials/check.sh <goatbuilder>` runs each
   tutorial's "Rehearse it on the fixture" section exactly as written, from the
   repository root, and diffs the output against the worked example's
   checkpoint, after normalising milliseconds and scratch paths. A page that
   drifted from the tool, or a tool that drifted from the page, fails. The
   Rust tests `goatnet_from_scratch_*` hold the standup rehearsals (1 and 2) inside the
   crate, and the `federate` `--mock` test holds tutorial 3's against the same
   fixture.
   Both run in `goatbuilder-pr.yml`, which since #2052 also runs when the
   scripts avenue changes.
2. **The hermetic tier.** The deployment e2e harness (ADR 1200, pull request
   #2049) runs operator pages' fenced commands against container hosts
   (`ops/e2e/doc-as-test.py`). When it lands, the tutorials' "Run it on your
   hardware" sections join it with `--only-dry-run` and the harness's own
   goatnet substitutions, so the hardware commands are checked as well as the
   rehearsal. Until then those sections are checked by hand against
   `goatbuilder --help`.
3. **The live canary.** Where a runner can reach a goatnet, the nightly canary
   re-proves the worked example's verbs live. No such runner exists today; the
   canary is the harness's `live-canary.sh`, run from an operator laptop.

## What the rehearsal proves, and what it does not

The rehearsal runs every verb for real against files: the phase order, the
classes each phase touches (the founder's included, for a joiner), the
publication serials, every record verified, and `status` across sites. It does
not install software or replicate a database; the hardware checkpoints prove
that. The two rehearsal features tutorial 2 needed are part of `goatbuilder`
since that change: a phase may name the founder's class as `<site>/<class>`
(only the site this one joins), and a fixture may list the records an earlier
standup left (`carried:`), so `status` still expects them. Tutorial 3 needed one
more: `federate` honors `--mock <dir>` and reads that directory's flat
`goatbuilder.yaml`, so a worked-example peering is planned against a declaration
in the fixture rather than the repository's own config — the same `--mock` every
other verb already took.

What a hardware run of tutorials 2 and 3 has to show, and what tutorial 3
needs from its counterparty, is in
[goatbuilder-hardware-measurement.md](goatbuilder-hardware-measurement.md#the-tutorials-on-hardware).

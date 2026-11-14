# ADR 1238 — The learning program: ten rungs to sharing sealed data with another sovereign, then fourteen stages beyond

| Field | Value |
|---|---|
| Status | Proposed — in use now as the working program. It is the maintainers' best guess, and it is open for comment |
| Date | 2026-11-14 |
| Repo scope | Multi (the program is the goat gym registry in goatstuff; this record states it) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1132](1132-goat-gym-training-surface.md) (the goat gym), [ADR 1213](1213-goatlite-preset-one-box-one-file.md) (goatlite), [`../design/goatlite.md`](../design/goatlite.md) §8 |

## TL;DR (for a layperson)

This is the path a person takes to learn goat. Ten rungs go from "I have a welcome package" to "I share sealed data with another sovereign". Fourteen further stages cover running a goatnet well, proving its value on real work, and things nobody has done yet. The order is a guess. If you walk it and it is wrong for you, say so and it changes.

## Context

A person arriving at goat needs to know what to do first, what to do next, and how to tell each step worked. The goat gym ([ADR 1132](1132-goat-gym-training-surface.md)) holds the lessons; this record states the order they are taken in and why.

## Decision

### The ladder: ten rungs

Each rung has one thing to do and one way to tell it worked. The ladder is kept to ten rungs so a newcomer sees a short path.

| Rung | The learner | Why it is here | State |
|---|---|---|---|
| 1 | Uses a goatnet someone else runs: enrolls a laptop, reads what is published, uses the CLI | Use it before building it. Everything later assumes you have seen a working goatnet from the inside | Built |
| 2 | Builds a moon-pod and joins a goatnet with it | The smallest thing you can build yourself: one box that joins an environment someone else runs | Reference implementation; the lesson does not yet have a build-and-join drill |
| 3 | Rehearses a new goatnet on the fixture | Stand one up with nothing at stake, before touching a real machine | Built, checked on every pull request |
| 4 | Stands up a goatlite and uses it | A first goatnet of your own: one box, one file of four facts | Run once on a fresh machine; the drill by someone outside the team is under way |
| 5 | Grows it so machines can enroll from the internet | The same goatnet, made reachable. Nothing is thrown away | Built; not yet stood up against real relays |
| 6 | Breaks it, reads the error, fixes it, tears it down and rebuilds | A goatnet you can only stand up is one you do not yet run | Commands built; lesson written |
| 7 | Federates a first node inside it | A second machine, before a second organization | Rehearsed on the fixture; not run on hardware |
| 8 | Adds a boundary node | The machine that will face another environment | Written and rehearsed |
| 9 | Federates across the boundary to another sovereign | Two environments, each run by its own party | Built and run between test environments; not yet with a real counterparty |
| 10 | Releases data across the boundary and seals it to one receiver | The point of the whole path: sharing with another sovereign, on purpose and to a named reader | Release measured on a live goatnet; across sovereigns unmeasured |

The order follows four rules: use before build, small before large, your own before shared, and break it before you depend on it.

### Beyond the ladder: fourteen stages

For a team that has climbed the ten rungs. None of them is needed to finish the ladder.

**Run it well: the daily work of a team that runs a goatnet.**

| Stage | Why | State |
|---|---|---|
| Change network access from code | Access rules are generated from a registry; a hand edit is lost at the next render | Lesson written |
| Host an outside service behind your goatnet | Most teams bring a service of their own within weeks | Capability built; lesson not written |
| Use direct peer-to-peer links as a capability | Direct links are granted, not assumed; using them badly costs bandwidth | Lesson written |
| Quality of service: reserve, reveal, enforce | A fabric with no broker has no central queue to tune | Lesson written |
| Find a quality-of-service fault and fix the right thing | The fault is found from what each side reveals, and fixed at the publisher or the link | Capability built; lesson not written |

**Prove its value: real work on the fabric, in any field.**

| Stage | Why | State |
|---|---|---|
| Publish what a model at the edge sees as it happens, for example a camera on a factory line reading barcodes | The first thing most teams want from the fabric: events known elsewhere the moment they happen | Publishing built; no worked example |
| Update the model running at a site or a tactical node | A model that cannot be replaced in place goes stale where it matters most | Frontier: no path for model files is designed |
| Let autonomous systems coordinate when no central station is reachable | The case the mesh and the brokerless fabric were built for | Frontier: transport built; no worked example or drill |
| Fill in one shared picture from two sovereigns, with no loops | Two parties each see part of the same thing | Frontier: the rules that stop a loop are not designed |

**The frontier: stages the project's own team has not reached.**

| Stage | Why | State |
|---|---|---|
| Cut a site off and keep working | Every site should keep doing everything while cut off ([ADR 1165](1165-island-mode-multi-writer-every-site-admits.md)) | Frontier: decided; the admin surfaces are not yet built at a joined site |
| Prove the hub overrules and can sever a site | One compromised site must not be able to wreck the rest ([ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md)) | Frontier: mechanism written; live nowhere |
| Burn a tactical pod down and rebuild it from its identities | Rebuilding should be routine, and every account is checked again on the way back ([ADR 1237](1237-a-site-is-rebuilt-from-nothing-and-a-combed-extract-of-its-identities.md)) | Frontier: proposed |
| The same for a host site | As above, for a core pair and an observability pair | Frontier: proposed |
| The same for a whole environment, one site at a time | As above, with the hub rebuilt last | Frontier: proposed |

"Frontier" means the tooling the stage would teach is not built yet. The gym shows such a stage as a named gap with no page.

### Where it lives, and how to change it

The program is one file, [`ops/gym/gym.yaml`](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/gym.yaml) in goatstuff: the lanes `learner` and `beyond-the-ladder`. That file is current; this record is the program as proposed on its date.

It is in use now because nobody can learn with a program that does not exist, and it stays Proposed because the people who walk it know better than the maintainers what it should be. Walk a rung and report where you stopped, as an [issue on goatstuff](https://github.com/usareur-af/goatstuff/issues). Propose a different order, a missing stage or a better drill as a pull request against that file.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Stand up your own goatnet first, and join someone else's later | A newcomer who has never seen a working goatnet cannot tell whether the one they built is right |
| Put the moon-pod after goatlite | Building one box that joins is less to learn than building a whole goatnet. It also needs a goatnet to join, which rung 1 already gave the learner |
| Leave break-and-rebuild off the ladder | It is the rung that turns standing a goatnet up into running one, and the one-week troubleshooting path is the hardest test of whether the tool is usable |
| End the ladder at operating a goatnet, with federation beyond it | Sharing with another sovereign is what goat is for. A ladder that stops before it teaches the tool and not its purpose |
| Add the fourteen further stages to the ladder | A newcomer shown twenty-four rungs sees a wall |
| Leave the frontier stages off until their tooling exists | They are where the project is going. Marked as not built, they tell a reader what is missing and who might help build it |

## Consequences

**Positive.** A newcomer has ten steps in a stated order with a reason for each. A team that has finished them can see what comes next, and which stages are not built.

**Negative.** Five rungs depend on things not yet proven on real machines or with a real counterparty, and say so. Rung 2's lesson does not yet deliver its rung. Eight stages beyond the ladder name tooling that does not exist.

## Assessor view (SCA-R)

This record changes no control and no deployed system. It orders training material that supports role-based awareness and training (NIST SP 800-53 AT-2, AT-3). Each drill in the gym states whether it only reads, writes, or needs an admin, and no lesson carries a private name, address or place. No waiver is involved.

## Trigger conditions for revisit

- A learner cannot climb the rungs in this order, or a rung turns out to need a later one.
- A stage beyond the ladder turns out to be needed to finish the ladder.
- A frontier stage's tooling is built: its state changes here and in the gym.
- A person outside the maintainers has climbed all ten rungs from the published documents: the record is then revised with what they found, and its Status reconsidered.

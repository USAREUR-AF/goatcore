# ADR 1238 — The learning program: ten rungs, then fourteen stages beyond

| Field | Value |
|---|---|
| Status | Proposed — in use now. It is a best guess, open for comment |
| Date | 2026-11-14 |
| Repo scope | Multi |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1132](1132-goat-gym-training-surface.md) |

## TL;DR (for a layperson)

This is the path a person takes to learn goat: ten rungs from first use to sharing data with another sovereign, then fourteen further stages. The order is our best guess. Walk it, and tell us where it is wrong.

## Context

A person arriving at goat needs to know what to do first and what to do next. Until now there was a list, with no reasons given and no stated way to change it.

## Decision

### The ladder: ten rungs

| Rung | You | Why here |
|---|---|---|
| 1 | Use a goatnet someone else runs: enroll a laptop, read what is published, use the CLI | Use it before you build it |
| 2 | Build a moon-pod and join a goatnet with it | The smallest thing you can build yourself |
| 3 | Rehearse a new goatnet on a practice fixture | Stand one up with nothing at stake |
| 4 | Stand up a goatlite: a first goatnet of your own, on one box | Small before large |
| 5 | Grow it so machines can enroll from the internet | The same goatnet, made reachable |
| 6 | Break it, read the error, fix it, tear it down and rebuild | You do not run a goatnet until you have repaired one |
| 7 | Add a second node inside it | A second machine before a second organization |
| 8 | Add a boundary node | The machine that will face another environment |
| 9 | Federate across the boundary to another sovereign | Two environments, each run by its own party |
| 10 | Release data across the boundary and seal it to one receiver | The purpose of the whole path |

### Beyond the ladder: fourteen stages

For a team that has climbed the ladder. None is needed to finish it.

**Run it well**

1. Change network access from code.
2. Host an outside service behind your goatnet.
3. Use direct peer-to-peer links.
4. Reserve and enforce quality of service.
5. Find a quality-of-service fault and fix the right thing.

**Prove its value**

6. Publish what a model at the edge sees as it happens, for example a camera on a factory line reading barcodes.
7. Update the model running at a site. *Frontier.*
8. Let autonomous systems coordinate when no central station is reachable. *Frontier.*
9. Fill in one shared picture from two sovereigns, with no loops. *Frontier.*

**The frontier**

10. Cut a site off and keep working. *Frontier.*
11. Prove that one site can overrule the others and cut a compromised site out. *Frontier.*
12. Burn a tactical pod down and rebuild it from its identities. *Frontier.*
13. The same for a host site. *Frontier.*
14. The same for a whole environment, one site at a time. *Frontier.*

*Frontier* means the tooling for that stage is not built yet.

### Tell us where it is wrong

The program is in use now, because nobody can learn with a program that does not exist. It stays "Proposed" because the people who walk it know better than we do what it should be.

- Walk a rung and report where you stopped: [open an issue](https://github.com/usareur-af/goatstuff/issues).
- Propose a different order, a missing stage or a better exercise: open a pull request against [the program file](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/gym.yaml).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Build your own goatnet first, and use someone else's later | Someone who has never seen a working goatnet cannot tell whether the one they built is right |
| End the ladder at running a goatnet, and put sharing with another sovereign beyond it | Sharing is what goat is for |
| One ladder of twenty-four rungs | A newcomer shown twenty-four rungs sees a wall |
| Leave out the stages whose tooling is not built | They show where the project is going, and who might help build it |

## Consequences

**Positive.** A newcomer has ten steps in order, with a reason for each, and can see what comes after.

**Negative.** Eight of the later stages are not built yet. The order is untested by anyone outside the team.

## Assessor view (SCA-R)

Training material only. It changes no control and no deployed system.

## Trigger conditions for revisit

A learner cannot climb the rungs in this order, or a frontier stage is built.

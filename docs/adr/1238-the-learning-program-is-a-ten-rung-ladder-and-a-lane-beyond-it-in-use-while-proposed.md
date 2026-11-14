# ADR 1238 — The learning program is a ten-rung ladder and a lane beyond it, in use while it is proposed; the people learning decide what it becomes

| Field | Value |
|---|---|
| Status | Proposed — in use as the working program while it is proposed. It moves to Accepted on the conditions in decision 6 |
| Date | 2026-11-14 |
| Deciders | the people who learn with it, through their reports; the maintainers keep the gate |
| Repo scope | Multi (goat core records the decision; the program itself is in goatstuff) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1132](1132-goat-gym-training-surface.md) (the goat gym), [ADR 1213](1213-goatlite-preset-one-box-one-file.md) (goatlite is rung 4 of the ladder), [ADR 1231](1231-one-development-branch-and-a-stable-line-that-follows-releases.md) (answering outside contributors), [`../design/goatlite.md`](../design/goatlite.md) §8, [`../design/goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) row C4 |

## TL;DR (for a layperson)

People learn goat by walking a set of stages, each with something to do and a way to tell it worked. Until now the stages were a list the maintainers kept, with no record of why it has the shape it has and no stated way for anyone else to change it. This record fixes the shape: a ladder of ten rungs that takes a newcomer from a welcome package to sharing sealed data with another sovereign, and a separate lane beyond it for running a goatnet well, proving its value on real work, and the stages nobody has reached yet. The maintainers do not know the right program; the people learning do. So the program is put to use now as a best guess, it stays "Proposed" while it is in use, and the reports of people who walk it decide what it becomes.

## Context

The goat gym ([ADR 1132](1132-goat-gym-training-surface.md)) holds lanes of modules, each with a drill. One lane, `learner`, is the ladder a newcomer climbs; [ADR 1213](1213-goatlite-preset-one-box-one-file.md) placed goatlite on it as rung 4, and [`goatlite.md`](../design/goatlite.md) §8 lists its ten rungs.

Three things were missing.

1. **Nothing said what comes after the ladder.** A team that had federated and released data had no named next stage: changing network access from code, hosting an outside service, finding a quality-of-service fault, running a model at a site, coordinating machines with no central station, keeping a site working when it is cut off.
2. **Adding those stages to the ladder would bury it.** A newcomer shown twenty-eight stages sees a wall. The ten rungs are the part that must stay short.
3. **There was a circle to break.** A learning program should be shaped by the people who learn with it. Nobody can learn with a program that does not exist. So the maintainers must guess first, and the guess must be easy to overrule.

A fourth point follows from the third. Some later stages teach things whose tooling is not built. Listing them as ordinary unwritten lessons would mislead a reader about what exists.

## Decision

1. **The program has two parts, and they stay separate.**
   - **The ladder:** ten rungs, the gym lane `learner`. It ends at releasing data across a sovereign boundary and sealing it to one receiver. The ladder is not lengthened. A proposal to add an eleventh rung is a proposal to replace one of the ten.
   - **Beyond the ladder:** the gym lane `beyond-the-ladder`, for someone who has climbed the ten rungs. Nothing in it is needed to finish the ladder. It has three groups: *Run it well* (daily operation), *Prove its value* (real work on the fabric, in any field), and *The frontier* (stages the project's own team has not reached). The ladder's page points to it once, at its end.

2. **The program has one home, and this record does not copy it.** The lanes, their order, every module, its drill and its status are in one file, [`ops/gym/gym.yaml`](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/gym.yaml) in the goatstuff repository, checked by [`check.py`](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/check.py) and rendered by [`generate.py`](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/generate.py) beside it ([README](https://github.com/usareur-af/goatstuff/blob/main/ops/gym/README.md)). [`goatlite.md`](../design/goatlite.md) §8 describes the ten rungs and what each runs on. A stage is added, moved or removed by changing that file. This record states the shape and the rules; it lists no stage, so it cannot disagree with the file.

3. **Every stage carries one of three marks, and each mark means one thing.**

   | Mark | Means | The reader gets |
   |---|---|---|
   | written | The lesson exists and has a drill with a stated pass condition | A page |
   | planned | What the lesson would teach is built; the lesson is not written | A named gap in the lane, no page |
   | frontier | The tooling the lesson would teach is not built yet | A named gap marked "frontier", no page |

   A planned or frontier stage says in its summary what is built today and what opens it. A stage becomes written only with a drill. A frontier stage becomes written only after its drill has passed on a standing goatnet. `check.py` refuses the frontier mark on a written stage.

4. **The program is in use now, as the maintainers' best guess.** Its order, its grouping and its choice of stages are a guess made so that there is something to learn with. The Status of this record stays `Proposed` for as long as that is true, and the program is used, taught from and built on while it is. `Proposed` here does not mean "not yet in effect".

5. **The people learning decide what it becomes.**
   - **A place where a learner stopped is a defect in the program**, not in the learner. Report it as an issue on [goatstuff](https://github.com/usareur-af/goatstuff/issues) naming the stage and what happened. The maintainers treat it as they treat a failing test.
   - **Anyone may propose a change** to the order, the grouping, a stage or its drill, as a pull request against `ops/gym/gym.yaml`. A change to the shape in decision 1 or the marks in decision 3 is proposed as a change to this record.
   - **Where a learner's report and the maintainers' guess disagree about order or content, the report wins** unless the change would break the gate in the next line.
   - **The maintainers keep the gate, and only the gate:** a lesson that opens must teach, a drill states its pass condition and how safe it is to run, and no lesson carries a private name, address or place (`check.py`). The first reply to an outside contributor follows the aim in [ADR 1231](1231-one-development-branch-and-a-stable-line-that-follows-releases.md).

6. **This record moves to Accepted when learners have shaped the program**, which means all three of:
   - a person outside the maintainers has climbed the ten rungs using only the published documents ([gate row C4](../design/goatbuilder-v1-gate.md));
   - every place they stopped is a merged change or an open issue;
   - at least one change to the program proposed by someone outside the maintainers has been merged.

   The pull request that records those three moves the Status.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One ladder holding every stage | A newcomer sees twenty-eight stages. The short ladder is what gets a person started, and it is the part that must not grow |
| Wait for learners before writing a program | Nobody can learn with a program that does not exist. The circle never closes |
| Publish the program as Accepted | It would state as decided what is a guess, and a learner would have to argue against a settled record to change it |
| Keep the program only in `gym.yaml`, with no record | The file says what the stages are. It does not say why the ladder is ten rungs, what a mark means, or who may change it |
| List every stage in this record | Two homes for one fact. The record and the file would drift, and the record is the one a reader trusts |
| Leave unbuilt stages off the program until their tooling exists | The stages are where the project is going. Showing them, marked honestly, tells a reader what is not there yet and invites the people who need them to help build them |

## Consequences

**Positive.**
- A newcomer sees ten rungs. A team that has finished them sees where to go next.
- A reader can tell a lesson that is only unwritten from one whose subject does not exist yet.
- There is a stated way for anyone to change the program, and a stated rule for whose view wins.
- The program can be used today without anyone claiming it is right.

**Negative.**
- A record that stays Proposed while in use is unusual, and a reader used to "Proposed means not in effect" must read the Status line. It says so.
- The maintainers accept changes they would not have chosen. That is the intent.
- Stages marked frontier advertise what is not built. Each must keep saying so until it opens.
- Decision 6 depends on people outside the maintainers doing the work. Until they do, the record stays Proposed.

## Assessor view (SCA-R)

This record changes no control and no deployed system. It governs training material: which stages exist, how their state is marked, and how they change. It supports the awareness and training family (NIST SP 800-53 AT-2, AT-3) by giving role-based practice a fixed, public structure with a drill and a pass condition per stage, and a `safety` field per drill. The residual risk is a learner running a drill that writes to a live system; the compensating control is the gym's existing rule that every drill states whether it is read-only, writes, or needs an admin, and the gate that refuses private names in any lesson. No waiver is involved.

## Trigger conditions for revisit

- A learner's report shows the ten rungs cannot be climbed in their order, or that a rung depends on a later one.
- A stage beyond the ladder turns out to be needed to finish the ladder.
- A frontier stage has stayed frontier through two minor releases after its tooling was built.
- The conditions of decision 6 are met: this record is then revised into its Accepted form.
- The gym's registry moves out of `ops/gym/gym.yaml`, or the lanes are renamed.

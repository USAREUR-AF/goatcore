# goatlite — human guide

> The human doorway to the `goatlite` skill. It points at the canonical source and does not restate it.

## When you're here

You want a real goatnet on one Ubuntu 24.04 host or VM, and you want a coding agent to do the work with you.

## Where the real instructions live

- The tutorial: [docs/tutorials/00-goatlite.md](../../../docs/tutorials/00-goatlite.md). It is the procedure, for a person and for an agent.
- The design, and what the preset decides for you: [docs/design/goatlite.md](../../../docs/design/goatlite.md).
- `goatbuilder --help` and `goatbuilder <command> --help`: the command reference, from the binary.

## What the agent needs from you

- Four facts: a short name for the goatnet, the host's address, the SSH user, and your email address as the administrator.
- Whether to encrypt the host's data disk and, if so, which disk. Encrypting a disk destroys what is on it; you name the disk, not the agent.
- One sign-in in a browser, at phase 11 of the standup. The agent pauses there and gives you the steps.
- Your agreement to the installs on your laptop, some of which use `sudo`.

## The agent's version

The `SKILL.md` beside this file is the same task written for an agent: what to ask first, where each command runs, the rehearsal, the step that needs you, resuming a failed phase, and how it tells that the goatnet is finished.

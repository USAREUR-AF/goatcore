---
name: goatlite
description: "Stand up a goatlite for a person: a real goatnet on one Ubuntu 24.04 host or VM, from one file of four facts, by following docs/tutorials/00-goatlite.md with goatbuilder. Use when someone asks to stand up, try, install or rehearse a goatlite or their first goatnet, hands over a host and asks for goat on it, or asks what goatlite needs. Covers what to ask the person first, where each command runs, the rehearsal that needs no host, the one step that needs a person, resuming a failed phase, and how to tell that the goatnet is finished. Not for a multi-site goatnet (docs/tutorials/01-new-goatnet.md) and not for production."
allowed-tools: Bash, Read, Grep, Glob, Edit, Write
audience: both
---

# Stand up a goatlite

The procedure is [`docs/tutorials/00-goatlite.md`](../../../docs/tutorials/00-goatlite.md).
Read all of it before the first command, and run its commands as written. This skill does
not repeat the tutorial. It says how an agent works through it: what to ask first, what
needs the person, and when to stop.

**`goatbuilder <command> --help` is authoritative** for flags. When the tutorial and the
help text of the binary you built differ, follow the help text and tell the person.

A goatlite is for learning and for a first goatnet, on one host. It is not a production
deployment. The design is [`docs/design/goatlite.md`](../../../docs/design/goatlite.md).

## 1. Ask the person first, once

Collect these before any command, in one message. Do not guess any of them.

| Fact | Goes into `goatlite.yaml` as | Note |
|---|---|---|
| A short name for the goatnet | `lite.name` | The person's own choice. Do not reuse the tutorial's `pasture` |
| The host's LAN address or hostname | `lite.host` | Ubuntu 24.04, 4 cores, 16 GB, reachable from this machine |
| The user the SSH key logs in as | `lite.ssh_user` | `ssh <user>@<host> true` must return without a password |
| The administrator's email address | `lite.admin` | The first and only administrator |
| Encrypt the data disk? | `lite.disk_encryption` | Defaults to `true`. For a VM that will be deleted, `false` |
| The data disk, when encrypting | `lite.data_disk` | See the warning below |

Also confirm two things about this machine: it is Linux or macOS (Windows is not supported;
WSL is untested), and the person agrees to the installs in section 1 of the tutorial, several
of which use `sudo`.

**Disk encryption destroys what is on the named disk.** With `disk_encryption: true`, do
not choose `data_disk` yourself. Show the person the output of `lsblk` from the host and
have them name the disk. If they do not answer, stop.

## 2. Where each command runs

The tutorial marks every command block with its place. Keep to it.

- **On the laptop** (the machine you run on), at the root of the goatcore checkout:
  `goatbuilder`, `goat`, and every script. Never run `goatbuilder` on the host.
- **On the host over SSH**: only `lsblk` and reading the one-time password.
- **In the browser**: the sign-in step of phase 11.

Every terminal activates the tutorial's Python environment first:
`. ~/.venvs/goatlite/bin/activate`. Each Bash call you make is a new shell, so start each
call that runs `goatbuilder`, `python3` or `ansible-playbook` with that line, and with
`export PATH="$HOME/.local/bin:$PATH"` if `goatbuilder` is not found.

## 3. Order of work

1. **Tools, checkout, the two programs** (tutorial section 1). Build with the tutorial's
   command; the first build takes several minutes. Finish with `goatbuilder --version` and
   `goat --version`.
2. **Rehearse on the worked example** (section 3) before touching the host. It needs no
   host and no network:

   ```bash
   WORK="$(mktemp -d)/pasture"
   cp -R ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite "$WORK"
   goatbuilder standup pasture --mock "$WORK" --ack "A Learner" --yes
   goatbuilder status --env pasture --mock "$WORK"
   ```

   Compare the output with
   `ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/00-goatlite.txt`. It must match
   apart from the milliseconds. If the rehearsal fails, stop and report: the build or the
   checkout is wrong, and the real run would fail the same way.
3. **Write `goatlite.yaml`** at the root of the checkout from the person's facts (section 2).
4. **`goatbuilder doctor`** (section 4). Fix each row it names and run it again until every
   row passes.
5. **`goatbuilder standup <name>`** (section 5). The run takes an hour or more. Do not put
   a short timeout on it, and do not start a second run while one is going.
   `goatbuilder standup <name> --dry-run` prints every phase and command and runs nothing;
   show the person that output first if they want to see the plan. If the run waits for a
   confirmation that the person has already given you, `--yes` confirms without prompting
   and `--ack "<their name>"` records who is declaring.
6. **The browser step**, then `goatbuilder standup <name> --from 12` (see below).
7. **Enroll the laptop** (section 6), only if the person asked for it (see below).
8. **Prove it** (section 7): `python3 ops/validation/generate-values.py <name>`,
   `ops/scripts/create-assay-probe-identity.sh <name>`, then `goatbuilder assay <name>`.

## 4. The step that needs the person

Phase 11 pauses the run and prints its steps with the goatnet's names filled in. The first
sign-in creates the administrator's account, and it is the person's account, password and
browser. Give the person the printed steps and wait. What you can do for them:

- Read the one-time password from the host, with the command that the run prints, and pass
  it to the person.
- After the person has signed in, run the re-run of phase 10 that the tutorial gives:
  `goatbuilder standup <name> --site <name> --from 10 --to 10`.
- After the person has created the service token, store it where the run says.
  `goatbuilder secrets hint <NAME>_NB_API_TOKEN` names the file. Do not print the token.

The hosts-file line in that step, and the root certificate in section 6, change this
machine with `sudo`. Run them only if the person agreed to that in step 1; otherwise hand
them the exact line.

Then resume: `goatbuilder standup <name> --from 12`.

## 5. Enrolling the laptop: never a bare `netbird up`

`netbird up` without `--profile` acts on whichever profile was used last. If this machine
is already on another mesh, a bare `netbird up` enrolls that profile into the new goatnet
and overwrites its login. Every `netbird` command you run names the profile, exactly as
section 6 of the tutorial writes it. Run `netbird profile list` first and record which
profile was selected, so that the person can return to it.

If the person did not ask for this machine to join the goatnet, skip section 6 and say so.
The acceptance rows that reach the host through the mesh then cannot run from here.

## 6. When a phase fails

A failed phase prints the phase number, the state of the host, and the resume command.

1. Read the error. `goatbuilder explain <code>` or `goatbuilder explain "<the error text>"`
   gives what happened, the likely cause, what to do and where to read.
2. Fix the cause. Running a phase again is safe; it changes only what is missing.
3. Resume from the failed phase: `goatbuilder standup <name> --from <phase>`.

On a first run, phase 5 stops while it waits for a database schema, and phase 6 completes
it. The tutorial's phase table says which outputs are expected. Do not treat an expected
stop as a failure, and do not work around a real failure by editing generated files or by
skipping a phase. If the same phase fails twice with the same error after a fix, stop and
give the person the error and the output of `goatbuilder explain`.

## 7. Finished means the assay says so

`goatbuilder assay <name>` checks the goatnet layer by layer. Finished is: every applicable
row is `PASS`. A row that could not run is `UNKNOWN`, which is not a pass, and the command
exits 3 while any remain. Read the table, not only the exit code, and give the person the
table.

Report to the person: the goatnet's name and zone, the assay table, any row that is not
`PASS` with its reason, which steps they did themselves, and every change made to this
machine (packages installed, the hosts-file line, the trusted root certificate, the NetBird
profile). Section 8 of the tutorial shows what to do next with `goat`; the
[goat-cli skill](../goat-cli/SKILL.md) covers that binary.

## 8. Do not

- Do not run `goatbuilder teardown` unless the person asks. `goatbuilder teardown <name>
  --site <name> --dry-run` prints the order and changes nothing.
- Do not commit `goatlite.yaml`, the generated configuration, or the secrets file to any
  repository, and do not print the secrets file or the setup key into a log that leaves this
  machine.
- Do not change the host outside what `goatbuilder` does over SSH.

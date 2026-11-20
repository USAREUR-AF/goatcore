---
heading: For someone deploying
title: "Changed: join-node's phase 1 checks the node answers and goes on, so one run goes from phase 1 to 15"
---

Phase 1 of `goatbuilder join-node` used to be a person's step, "reach the box
on the site LAN". Every run stopped there and went on only with `--from 2`.
It is now a check: the node answers over SSH at `--address`, or a phone answers
over USB (`adb get-state`). The run goes on to phase 15 with no operator step.

A box that refuses the login still counts as answering. Before phase 2 seeds
your key, a box that takes only its first-login password refuses every batch
login, and phase 2 proves the login.

A box that does not answer stops the run at phase 1 with GB-314 and the step
for a person. Resume with `--from 1`. So does a Jetson's USB bridge address,
192.168.55.0/24: its sshd answers, but it is reachable only over a direct
USB-C cable. Pass the box's real LAN address.

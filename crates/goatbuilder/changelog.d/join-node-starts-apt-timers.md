---
heading: For someone deploying
title: "Fixed: a joined node takes its automatic updates again without a reboot"
---

`join-node` phase 4 masks the node's `apt-daily` and `apt-daily-upgrade` timers
for the run, and phase 14 unmasked them. Unmasking alone leaves a timer stopped
until the next boot, so a joined node took no automatic updates until it
rebooted. Phase 14 now enables and starts both after unmasking them.

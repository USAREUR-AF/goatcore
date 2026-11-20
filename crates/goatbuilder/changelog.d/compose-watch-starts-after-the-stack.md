---
heading: For someone deploying
title: Fixed: the stack's watcher starts only after a deploy has brought the stack up, and is paused during it
---

`mgmt-host.yml`, which `site-bootstrap.yml` runs in `join-node`'s phase 13 and
`standup`'s phase 5, installed and started `mgmt-host-compose-watch` before it
wrote the new compose files and ran `up -d`: a play's roles run before its
tasks. On a joiner's re-initialisation the watcher then started the old
`postgres-a` from the old compose file on the emptied volume, so Postgres
initialised without the founder's encoding. The watcher now starts after the
stack is up, and the deploy pauses it from its first step to its last
(`/run/mgmt-host-compose-watch.paused`, reason `mgmt-host-deploy`, kept if
another pause is set). A deploy that fails in between keeps the pause and says
so, with the command to clear it, and the new alert `ComposeWatchPaused` fires
when a pause lasts over 30 minutes. A reboot clears it.

---
heading: For someone deploying
title: Fixed: the stack's watcher no longer starts a joiner's old Postgres during its re-initialisation
---

A joiner whose existing Postgres cluster has another encoding than its
founder's is refused by `site-bootstrap.yml` (the play `goatbuilder join-node`
and `standup` run at phase 13 and 6), which says to stop the stack, remove the
`pgdata-a` volume and re-run. While the stack was stopped,
`mgmt-host-compose-watch` started the old `postgres-a` again within 30 s. The
watcher now honours a pause file, `/run/mgmt-host-compose-watch.paused`: while
it exists it revives nothing, logs that it is paused, and reports
`compose_watch_paused` 1. The refusal says to pause it first with the reason
`postgres-reinit`, and the re-run clears that pause, and only that one, once the
cluster has the founder's encoding.

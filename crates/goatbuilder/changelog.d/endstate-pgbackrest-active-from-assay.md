---
heading: For someone deploying
title: "Changed: `goatbuilder endstate` grades pgBackRest active from its acceptance rows, and the unused sidecar is removed"
---

pgBackRest runs inside the database container, and nothing ran the
`pgbackrest-runner` sidecar. `goatbuilder endstate` used to grade pgBackRest
active when that sidecar container was running. A service with no container of
its own is now active only when its acceptance rows pass (pgBackRest's stanza
`info`). It is FAIL when one fails, and UNKNOWN when none ran. An empty
container list is never read as running.

To take it off a host, deploy the pgbackrest role; it removes the retired sidecar
there: the container, running or stopped, the overlay
`docker-compose.pgbackrest.yml`, and the `PGBACKREST_IMAGE` line in `.env`. The
bind-mounted repository is not touched. Before that, the role
stops and names any file in the repository, log or spool directory that the
database user does not own. A pgbackrest command run as root, as the sidecar did
when exec'd into, leaves such files, and backups and restore then fail. The
role changes no ownership itself.

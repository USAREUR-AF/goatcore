---
heading: For someone deploying
title: "Changed: standup passes secrets through the environment, not the command line"
---

A standup phase that needs the Postgres superuser password, the NetBird
datastore key, the pgBackRest cipher pass, the relay secret or the Cloudflare
token now gives the play `-e 'name={{ lookup("env", "<SITE>_NAME") }}'`, as
`join-node` already did. Before, the shell expanded `$(printenv <SITE>_NAME)`
into the `ansible-playbook` command, and the phase-7 relay line wrote its
secrets to a temporary vars file. The play now reads each value from its own
environment, so the same variables must be set as before. A phase whose
`lookup("env", ...)` names an empty variable is refused before it runs, naming
the variable, as an empty `$(printenv ...)` was. `goat-standup.sh` prints the
same lines. A dry run shows the new form, and the recorded fixture plans are
regenerated.

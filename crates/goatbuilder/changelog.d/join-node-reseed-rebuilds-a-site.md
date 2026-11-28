---
heading: For someone deploying
title: "Changed: join-node --reseed rebuilds a site whose Spock slots were lost, keeping the rows only that site holds"
---

`goatbuilder join-node <env> --site <site> --node <node> --class <core|phone> --reseed [--peer <site> ...] [--accept <digest>]`
now runs eight steps instead of six. Before anything is truncated, the new
`export` step compares each replicated table with the founder by primary key
and row digest. It writes the rows only the node holds to a file beside the
journal (mode 0600) and refuses (GB-388) until run with `--accept` and that
file's digest. After the copy, the new `replay` step writes the node's own
rows back, so they replicate to the founder. A row it cannot write is handed
back, with Postgres's reason (GB-391). Lost subscriptions are created again in
both directions. The founder's subscription from the node is included when
its slot on the node is lost, and the `caught-up` step then passes with a
note. The node's other subscriptions pause during the copy. Only the tables that
differ from the founder's, and the replicated tables that reference them, are
copied. They are truncated once, in repair mode, before each is copied again:
`spock.sub_resync_table`'s own truncate cascades to child tables, which are
then not copied again. A table that is not replicated and that the truncate
empties, such as a site's sessions, is named in the export, and accepted with
it. Each step refuses while a client other than the
rebuild is connected (GB-389), and when a provider is not one of the sides
given or a table has no primary key (GB-390). A journal from the six-step
helper is refused; finish that run with the helper it began with. The runbook
is `docs/operations/spock-site-rebuild.md`.

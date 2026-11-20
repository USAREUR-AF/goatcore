---
heading: For someone deploying
title: "Fixed: join-node refuses before phase 1 when a phase would read an unset variable (GB-369)"
---

A phase whose command expands a variable that is not set, such as a founder's
`OWNER_EMAIL` in phase 13, stopped on bash's own `unbound variable` partway
through the node, and goatbuilder said only "No known error in its output".
join-node now checks every phase it will run before the first one and
refuses with GB-369, naming the phase and each variable. `goatbuilder explain`
and a failed phase's `interpreted` row name bash's line GB-369 too.

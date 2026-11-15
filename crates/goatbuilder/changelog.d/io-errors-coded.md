---
heading: For someone deploying
title: Fixed: the last uncoded file, process and write errors of standup, tutor and federate carry their code
---

Twelve places passed a file, process or write error up with no catalog code,
so a log of a run that met one held only `goatbuilder: <text>`. Each now
carries one: GB-254 names the file for a step's log that could not be opened
a second time (standup's phase-12 waves, its image pull and its
observability leg) and for a directory `tutor` could not read while copying
its workspace; GB-322 for waiting on a step's bash (the same three), for the
reference `goatbuilder docs` could not write, for the report phase 3's edge
half could not write, and for the deploy baseline that could not be
serialized. The standup and `docs` places that now carry GB-322 exit 1, where
they exited 2; the federate ones exited 1 already. The GB-254 ones still exit 2.

---
heading: For someone deploying
title: "Fixed: a run record's probe log names only the variables the run reads"
---

join-node, standup, deploy and doctor ask the secrets posture which of their
variables are available. That probe printed the name of every variable with a
value on the machine (never a value), and a run keeps each probe's output, so
every run record's `logs/NNN-probe.log` listed every environment variable's
name. The probe now asks for the variables the verb reads and answers for
those only. `goatbuilder secrets available NAME…` takes names the same way;
with none it still lists every name, for your own terminal.

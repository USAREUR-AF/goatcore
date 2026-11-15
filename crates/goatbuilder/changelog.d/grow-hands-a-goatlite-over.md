---
heading: For someone deploying
title: Added: goatbuilder grow hands a goatlite over to the full preset
---

`goatbuilder grow` rewrites `goatlite.yaml` as a full configuration with the
same values and shows the difference, leaf by leaf; `--dry-run` stops there.
A real run (`--yes` without a terminal) writes
`ops/goatbuilder/goatbuilder.yaml`, takes the generated header off every file
the goatlite's render wrote, so each is yours, and removes `goatlite.yaml`.
It refuses, and changes nothing, when `goatlite.yaml` is not a goatlite
(GB-304), when `ops/goatbuilder/goatbuilder.yaml` already holds another
configuration (GB-305), or when the grown configuration is incomplete
(GB-306: a four-fact file names no `repo` or `components`). It is step 3 of
the growth design; the other steps are not built, and it has not run on
hardware.

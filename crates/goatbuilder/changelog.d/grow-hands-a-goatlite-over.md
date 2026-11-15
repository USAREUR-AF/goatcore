---
heading: For someone deploying
title: Added: goatbuilder grow hands a goatlite over to the full preset
---

`goatbuilder grow` rewrites `goatlite.yaml` as a full configuration with the
same values. Before it changes anything, it shows:
- the difference, leaf by leaf;
- the configuration file it writes;
- each file whose generated header it takes off;
- the removal of `goatlite.yaml`;
- the renderer command it runs, exactly as it runs it.

`--dry-run` stops there. At a terminal it then asks, and goes on only when
you type yes. It asks only when stdin and stderr are both a terminal; otherwise,
including `goatbuilder grow 2>file`, it needs `--yes`. With `--yes` it does not ask. A run writes
`ops/goatbuilder/goatbuilder.yaml`, takes the generated header off every file
the goatlite's render wrote, so each is yours, and removes `goatlite.yaml`.
It changes files in the checkout only, never a host. Before you commit, git
undoes it: restore `goatlite.yaml` and the files it handed over, and delete
`ops/goatbuilder/goatbuilder.yaml`.
It refuses, and changes nothing, when `goatlite.yaml` is not a goatlite
(GB-304), when `ops/goatbuilder/goatbuilder.yaml` already holds another
configuration (GB-305, which says so when the file is grow's own from an earlier run, since edited), or when the grown configuration is incomplete
(GB-306: a four-fact file names no `repo` or `components`). A renderer that
fails or refuses stops it with the renderer's own words (GB-258). Run again
after it finished, it says the goatlite was already handed over. It is step 3 of
the growth design; the other steps are not built, and it has not run on
hardware.

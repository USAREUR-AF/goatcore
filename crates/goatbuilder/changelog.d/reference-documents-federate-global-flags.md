---
heading: For someone deploying
title: Fixed: the command reference lists federate's own flags
---

`goatbuilder docs --markdown` and `docs/reference/goatbuilder/command-reference.md`
documented a global flag only on the root command. The flags `federate`
declares for all its verbs (`--over-ssh`, `--side`, `--half-out`,
`--edge-half`, `--edge-half-sha`, the host, group and router-container flags,
and the goat context flags) appeared on no command. They are now documented
once, on `goatbuilder federate`, so the compatibility check also holds them
against the last release.

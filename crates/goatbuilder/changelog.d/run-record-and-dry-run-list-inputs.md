---
heading: For someone deploying
title: Added: a run lists every value it uses and where each came from
---

`goatbuilder standup`, `join-node`, `deploy` and `federate` now record every
value their commands use in the run record's `inputs` (`goatbuilder show <run>`,
`goatbuilder runs diff`), each with its source: a file and line, the
declaration and rule it was derived from, an environment variable, a default,
an allocation or an argument. A dry run prints the same list, as `input` rows
under `-o json` and as a section of the plan page, and names the secrets
posture probes it ran to read them. A secret is shown by its source only,
never its value.

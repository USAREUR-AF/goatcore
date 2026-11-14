---
heading: For someone contributing
title: Added: a deprecated command, flag or value warns on stderr when a run uses it
---

`goatbuilder` now reads the deprecation list it ships
(`ops/goatbuilder/compat/deprecations.yaml`) on every run. A run that uses a
command, a flag, or one value of a flag or positional argument that the list
names still works. It prints one line on stderr naming what it used, the
release that first warned, and what to use instead. A flag renamed with its
old name kept as an alias warns only when the old name is typed. An entry can
now deprecate one value of a positional argument (`flag: <COMPONENT>` with
`value:`), as the compatibility check already allowed. The list is empty
today, so no run prints a warning yet. The run's `-o json` document does not
carry the warnings yet.

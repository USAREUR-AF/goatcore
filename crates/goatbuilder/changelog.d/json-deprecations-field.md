---
heading: For someone deploying
title: "Added: the -o json document lists the deprecated items a run used"
---

A run that uses a deprecated command, flag or value now lists each one in a
`deprecations` field of its `-o json` or `-o yaml` document, a report's or a
refused run's error document, so a pipeline sees it without reading stderr.
Each entry names the command, the flag or value, the replacement, the release
that first warned and the warning line printed on stderr. A run that used none
has no such field, so every document is unchanged until the list in
`ops/goatbuilder/compat/deprecations.yaml` names something (#818).

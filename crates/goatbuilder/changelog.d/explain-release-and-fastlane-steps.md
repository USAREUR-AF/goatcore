---
heading: For someone deploying
title: "Added: `goatbuilder explain release` and `explain fastlane` answer for each of their steps"
---

The phase catalog now describes `release`'s eight numbered steps (preflight,
tag, dev pin, dev deploy, dev rows, preprod, soak, prod) and `fastlane`'s
eight (component, graduation set, digest, dry run, move fleet, record the
debt, land, deploy). `goatbuilder explain release` lists them, and
`goatbuilder explain release 6` (or `explain release preprod`) says what the
step does, why, what it changes, how to tell it worked, the command by hand
and what to do when it failed, as `explain` already did for `standup`,
`join-node`, `deploy` and `federate`. A test holds each verb's entries to the
step lines it prints, so a new step cannot ship without one. Nothing a run
does changes.

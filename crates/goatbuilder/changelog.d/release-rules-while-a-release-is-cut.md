---
heading: For someone contributing
title: Added: the rules that hold while a release is cut
---

The release procedure (`docs/reference/goatbuilder/02-install.md`) now says
when the tag goes on, and three rules from the version branch's cut-off to
the tag: merges freeze except for the version pull request and the changes it
names; a run on `main` cancelled by a later merge is run again once and is not
a failure; and an engineering question that comes with a recommended option
does not hold the release once 30 minutes pass without an answer, while a
question of policy, cost or people, or about an act that cannot be undone,
waits (#673).

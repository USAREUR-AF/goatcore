---
heading: For someone deploying
title: Fixed: a release re-run on a later day opens no dev-pin PR
---

`goatbuilder release` declares a candidate on the dev rung with a reason that
carries the day. Re-running a dev step that had already landed, on a later
day, rewrote that reason and landed a pin PR that changed nothing but the
date. A rung that already declares the candidate now keeps its deviation as
it stands, reason and date included (#622).

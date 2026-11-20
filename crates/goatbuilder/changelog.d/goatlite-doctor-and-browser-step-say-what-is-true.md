---
heading: For someone deploying
title: "Fixed: under goatlite, the doctor and the browser step name the right phase and command"
---

`goatbuilder doctor` says a goatlite's missing secrets file is created by
phase 1 of the standup, the phase whose "create the local secrets store" step
writes it; it said phase 3. The browser step that pauses a LAN standup at
phase 11 names `goatbuilder secrets hint <SITE>_NB_API_TOKEN` as the command
that says where to store the token, instead of the posture script behind it.

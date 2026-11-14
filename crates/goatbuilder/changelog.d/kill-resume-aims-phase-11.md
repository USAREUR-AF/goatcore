---
heading: For someone contributing
title: Changed: the federation test environment's kill-resume aims phase 11's kill
---

The kill-resume scenario kills phase 11 once the WireGuard role has written
the link's files and before its handler restarts the link, instead of a
fixed delay after the phase starts. The gap that 0.6.1 fixed (#597) is now
found on every run, not by chance (#620).

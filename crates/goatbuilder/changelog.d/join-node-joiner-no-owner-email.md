---
heading: For someone deploying
title: "Fixed: a joiner's join-node needs no OWNER_EMAIL"
---

A joiner's phase 13 passed `-e bootstrap_owner_email="$OWNER_EMAIL"` to its
bootstrap passes. A joiner's account arrives from the founder, so the value
was never used, but a run with no `OWNER_EMAIL` set stopped at phase 13
(unbound variable). A joiner's phase 13 no longer passes it. A founder site's
join-node still does.

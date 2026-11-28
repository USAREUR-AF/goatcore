---
heading: For someone deploying
title: "Fixed: a join-node resumed from phase 13 after Spock is wired no longer refuses at the seed"
---

When phase 13 stopped after pass 3 had wired Spock, a resume with `--from 13`
ran the seed again, and any write the founder made in the meantime made the
seed tables differ, so the seed refused ("already seeded or already
federated"). Each database is now read on its own: one whose subscription from
the founder is replicating is past its seed and is compared once it has caught
up; one with no subscription must still equal the founder, as before (GB-367).

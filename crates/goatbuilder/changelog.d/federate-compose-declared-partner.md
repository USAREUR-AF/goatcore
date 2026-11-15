---
heading: For someone deploying
title: "Changed: phase 6 composes the agreement only with the peering's declared partner"
---

Phase 6 now tells `compose` which two parties the agreement is between: us
and the peering's declared partner (`FED_COUNTERPARTY_ID`, from
`goatbuilder.yaml`). A counterparty package of any other party is refused
(GB-191) and no agreement is written, even when the keyring binds that party's
key, as a keyring shared by several peerings can. Only the two parties' keyring
lines are passed to compose. `goat federate compose` takes the parties as
`--party <sovereign_id>`, once or twice; without it, compose behaves as before.

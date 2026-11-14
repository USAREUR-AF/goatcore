---
heading: For someone deploying
title: Fixed: a phase 11 link that did not come up is named as GB-271
---

When phase 11 of `goatbuilder federate` fails because the peering's
WireGuard interface does not hold the agreed peer and address, the failure's
`interpreted` row now names GB-271 instead of X-ANSIBLE-003, which pointed
at an empty secret and `deploy --dry-run`. The likely causes, most likely
first: the link unit is still running as it was before phase 11 and was not
restarted; the unit was restarted but its link script stopped partway; or
something else changed the interface afterwards. The next step: restart
`goat-peering-wg@<interface>.service` on the edge and check the interface
again, read the unit's journal if it still does not match, then resume with
`goatbuilder federate <peering> --from 11`. `goatbuilder explain GB-271`
gives the same. Every other failed assertion still reads X-ANSIBLE-003
(#646).

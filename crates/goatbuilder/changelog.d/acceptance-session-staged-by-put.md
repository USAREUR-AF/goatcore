---
heading: For someone deploying
title: "Fixed: phase 14 in a bilateral session no longer fails each side's own staging row"
---

In a bilateral acceptance session (`FED_ACCEPTANCE_SESSION` or
`FED_ACCEPTANCE_ATTEMPT`), each side checked that its own samples were staged
by reading them back from `release/<set>/**` on its own core. A core keeps no
store on that staging name, so the read found nothing and both verdicts
failed on `publish/outbound/<set>`, although the partner saw the samples
arrive. The row now passes when our core accepts both samples (HTTP 2xx),
fails when it refuses one and is BLOCKED when it does not answer; the
partner's `delivery/inbound/<set>` still observes the arrival. (#789)

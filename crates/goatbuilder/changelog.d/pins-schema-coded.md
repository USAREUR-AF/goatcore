---
heading: For someone deploying
title: Fixed: a fleet-pins.yaml that breaks the pins schema is refused with GB-337
---

`goatbuilder pins`, and the verbs that run its check (`release`, the
preflight, the fast lane), refused a fleet-pins.yaml, its sidecar or a pins
file that broke the pins schema with no catalog code. Each of those
refusals now carries the new code GB-337 and names the file and the rule.
It still exits 2.

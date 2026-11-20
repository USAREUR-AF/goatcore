---
heading: For someone deploying
title: "Changed: a member pin deviation names the goatnet value it overrides, or `goatbuilder pins` refuses (GB-357, GB-358)"
---

A host group's own deviation in `ops/pins/fleet-pins.yaml` used to keep winning
after its goatnet moved, for example to a hotfix, and that group silently kept
the old image (ADR 1256). A member deviation with a `value:` now carries
`over:`, the value its goatnet resolves the component to without it: a ref,
`absent` or `none`.

**What refuses after you move your goatcore pin.** `goatbuilder pins`,
`ops/pins/generate-pins.py` and `ops/pins/report.py` stop with:

- **GB-358**, for each member value deviation with no `over:`. The message
  prints the line to paste, for example:
  `over: "ghcr.io/example/signal-server@sha256:<the goatnet's value>"`.
  Paste it under that member deviation to keep the member's pin, or delete the
  deviation so the member takes the goatnet's value.
- **GB-357**, when `over:` no longer matches because the goatnet's value changed.
  Delete the deviation to take the new value, or set the printed `over:` line to
  keep the member's pin deliberately.

An `absent: true` member and a goatnet-scope deviation need no `over:`, and
`over:` on either is refused. A member deviation that pins its goatnet's own
value prints a warning. Nothing stamps `over:` for you: deciding each one is
the point. `graduate.py hotfix` now names the members a hotfix would miss.

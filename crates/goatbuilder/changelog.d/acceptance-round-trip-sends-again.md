---
heading: For someone deploying
title: Fixed: phase 14's round trip sends a fresh probe until one is acknowledged, so one dropped probe does not fail it
---

In a bilateral acceptance session both sides run the round trip at once.
Our probe could reach the partner's edge just behind their responder's
acknowledgement of their own probe, and the edge's downsampling dropped it
with nothing logged, so the round trip on that set failed after its
timeout although the peering worked. `fedcheck.sh` now sends a probe with
a fresh nonce until one is acknowledged: the first resend one gap after the
first probe (phase 14's sample gap), each later one twice as long after
the last, up to 4 seconds. Any acknowledged probe passes, and the check
ends within its timeout. A first probe our core refuses fails the check at
once with its HTTP status; a later refusal is counted and the check keeps
waiting. `responder.py` no
longer stops when one acknowledgement cannot be written; it tries it again (#808).

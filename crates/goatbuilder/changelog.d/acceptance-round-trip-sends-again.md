---
heading: For someone deploying
title: Fixed: phase 14's round trip sends a fresh probe once per gap, so one dropped probe does not fail it
---

In a bilateral acceptance session both sides run the round trip at once.
Our probe could reach the partner's edge just behind their responder's
acknowledgement of their own probe, and the edge's downsampling dropped it
with nothing logged, so the round trip on that set failed after its
timeout although the peering worked. `fedcheck.sh` now sends a probe with
a fresh nonce once per gap (phase 14's sample gap, never faster) until one
is acknowledged, and any acknowledged probe passes. A probe our core
refuses fails the check at once with its HTTP status. `responder.py` no
longer stops when one acknowledgement cannot be written; it tries it again.

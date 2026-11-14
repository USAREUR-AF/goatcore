---
heading: For someone contributing
title: Added: a test that backing out a role-admitted core gives its org fragment back
---

A unit test holds the back-out to the federation test environment's real
fixtures: admission's `merge_org` turns the core's org fragment from before
the peering into the admitted one, and `backout_core.py plan` turns the
admitted one back into the fragment from before, byte for byte
(`crates/goatbuilder/tests/fixtures/backout-role-core/`) (#547).

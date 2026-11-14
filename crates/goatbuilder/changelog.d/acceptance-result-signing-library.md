---
heading: For someone contributing
title: Added: a library that signs a side's acceptance result and verifies the partner's
---

`crates/goat-federation/src/acceptance_sig.rs` signs a phase 14 result with
the side's federation signing key and verifies a partner's under the key
bound to the partner at phase 5. The signature covers a fixed-format
statement whose first line is `goat-acceptance-result/v1` and which names
the agreement, session, attempt, both parties, the echoed challenge and the
sha256 of the exact result bytes. Nothing else the key signs begins that
way, and the package and directory signers refuse it. A result that does
not verify is GB-272, and one from another session is GB-273. No command
reaches the library yet; it is the first part of gate row B2 (#654).

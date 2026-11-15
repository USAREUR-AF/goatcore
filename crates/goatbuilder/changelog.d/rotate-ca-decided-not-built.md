---
heading: For someone deploying
title: "Changed: `federate rotate <partner> ca` says the CA rotation is decided, not built"
---

`goatbuilder federate rotate <partner> ca` is still refused with GB-282, and
nothing runs. Its message used to say that the CA rotation needed a decision
first. That decision is now made: ADR 1242 announces our next CA in package v2,
so the partner trusts both CAs for an overlap. The message now says the
rotation is decided and not yet built in this goatbuilder. Rotate the leaves
under the same CA with `rotate <partner> leaves` meanwhile.

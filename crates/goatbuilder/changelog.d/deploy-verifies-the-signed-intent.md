---
heading: For someone deploying
title: "Changed: deploy verifies the goatnet's signed intent before any host"
---

`goatbuilder deploy` and `deploy-component.sh` verify the goatnet's signed
intent from the checkout, with no network, before any host is touched: the
intent is there, its `content_hash` is its body's, its signature verifies
against the committed public key of its CA, it is what the tree renders, and
each member group's `pins.yml` is what the pins table renders. These are the
checks `goatbuilder pins --check` makes, for this one goatnet. One that does not
hold is refused (new code GB-383, exit 1); a dry run reports it after the whole
plan. `--verify-only` is not judged.

**A goatnet whose intent was never signed, or whose pins moved without signing
again, can no longer be deployed with either tool** until its intent is signed
and committed. `deploy-component.sh` needs the Python module `jsonschema`
beside PyYAML, as `ops/pins/generate-pins.py` does. The public key is read from
the checkout, so this check proves the pins are what that key signed, and not
that the checkout's key is the right one.

`ops/pins/generate-pins.py --check --goatnet <name>` is the script's half.

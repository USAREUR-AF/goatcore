---
heading: For someone deploying
title: Changed: a checkout file that is not well formed is refused with GB-329, exit 1
---

A YAML file goatbuilder reads from the checkout that does not parse (a pin
table, the ladder, a record, a workflow), and an intent schema
(`ops/consistency/schema/intent-v1.schema.json`) that is not JSON or does not
compile, now stop with GB-329, "a file goatbuilder reads from the checkout is
not well formed", and exit 1: refused, a precondition failed. The message names
the file and the parser's reason.

#### Compatibility for scripts

- A YAML file that does not parse, and a schema that does not compile, were
  GB-254, "cannot be read or written", with exit 2. They are now GB-329 with
  exit 1.
- An intent schema that is not JSON had no code and exited 2. It is now
  GB-329 with exit 1.

A script that tests for exit 2, or matches GB-254, after one of these sees
exit 1 and GB-329. A file that is missing or cannot be read stays GB-254.

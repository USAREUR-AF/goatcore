---
heading: For someone contributing
title: Fixed: a changelog entry title written in quotes renders without them
---

`ops/release/changelog.py` read an entry's `title:` and `heading:` verbatim,
so a title written YAML-style, `title: "Added: …"`, rendered as a heading
with its quotes, and a quoted heading was refused as not one of the three.
A value wrapped in one matching pair of `"` or `'` is now read without that
pair; quotes inside a title are kept. The entries directory's README says a
title needs no quotes for its colon.

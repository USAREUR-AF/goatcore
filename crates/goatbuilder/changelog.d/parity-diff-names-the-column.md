---
heading: For someone contributing
title: Changed: the standup and teardown parity gates say at which column two lines differ
---

`ops/goatbuilder/parity-standup.sh` and `parity-teardown.sh` compare the
script's plan with goatbuilder's. On a difference they printed a `diff` cut at
220 characters, so two long lines that differed further along printed the same.
They now print, for each differing line, its line number, the column of the
first different character, both lengths and the text around that column on
both sides (`ops/goatbuilder/parity-line-diff.py`) (#1181).

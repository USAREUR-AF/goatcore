---
heading: For someone contributing
title: Added: from a 1.0 baseline, a kept exit code no longer moves with a reason
---

`ops/goatbuilder/compat/compat.py` lets a pull request change a kept
invocation's exit code with its reason in `exit_changed:` only while the
last release is 0.x. Once the baseline is `goatbuilder/v1.0.0` or later, it
refuses `exit_changed:` unless the build's version in
`crates/goatbuilder/Cargo.toml` is a later major release, since a kept exit
code is part of the 1.0 promise (ADR 1228). Today's 0.7.0 baseline is
unaffected. (#813)

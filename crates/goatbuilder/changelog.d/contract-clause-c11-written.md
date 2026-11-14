---
heading: For someone contributing
title: Added: contract clause C11, legibility, is written beside C1 to C10
---

Clause C11 is now in the deploy contract, in `docs/design/goatbuilder.md`
and the contract chapter (`docs/reference/goatbuilder/06-contract.md`):
for every action the tool takes, its command, its inputs with their
sources, and its reason can be read before, during and after the run. Both
name its conformance test, `crates/goatbuilder/tests/legibility.rs`, and
say what each of its four checks requires. C11 does not hold yet: the
checks that fail today are listed in
`crates/goatbuilder/tests/legibility-known-failing.txt` (#647).

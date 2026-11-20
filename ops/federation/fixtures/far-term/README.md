# far-term: a federation agreement test fixture that does not expire in practice

Not normative, and not a conformance vector. The normative vectors are in
[`../../vectors/`](../../vectors/), are evaluated on their own `now.txt`, and
are never re-dated.

`goat federate verify` and `apply` judge an agreement's term on today's UTC
date and take no other date, so a test that runs them through a binary needs
an agreement still in its term on the day it runs. This is that agreement:
two packages between the test parties `xta` and `xtb`, both expiring
2099-12-31, and the agreement the binary composes from them.

- **Test keys.** Each package is signed with an Ed25519 key whose seed is a
  fixed public name in [`generate.py`](generate.py). Anyone can sign with it;
  a signature by one proves nothing about who made a package. Only the public
  halves are committed (`keys/`). Because anyone can derive the private
  halves, CI refuses the verifying keys and the CA fingerprints, listed in
  `refused-outside-fixtures.txt`, in any tracked file outside
  `ops/federation/fixtures/` and `tests/` directories.
- **Made, not edited.** `python3 generate.py <goat or goatbuilder>` writes
  every file here; `generate.py --check <binary>` makes it again in a scratch
  directory and fails unless it matches byte for byte, or once the fixture is
  within a year of 2099-12-31. The parity gate
  (`ops/goatbuilder/parity-federate-verbs.sh`) runs the check.
- **Renewal.** `ops/ci/check-fixture-expiry.py` also judges these packages on
  the wall clock and fails 60 days before they expire.

# E_PROFILE_DENIED: the profile exists but this user cannot read it

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_PROFILE_DENIED` is printed by `goat`. The same text offline: `goat explain E_PROFILE_DENIED` or `goatbuilder explain E_PROFILE_DENIED`.

## What it means

the profile directory is there and permission was denied reading it, typically a root-owned 0700 ~/.goat probed without sudo, or bare sudo that reset $HOME to /root.

## Why it happens

- the profile was created as root
- bare `sudo goat …` looked in /root/.goat

## What to do

One line per step or alternative:

```text
sudo chown -R $USER ~/.goat
sudo -E goat … (keeps your $HOME)
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 3.

# E_ACL_DECLARATION_MISSING: no ACL declaration was supplied

Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. `E_ACL_DECLARATION_MISSING` is printed by `goat`. The same text offline: `goat explain E_ACL_DECLARATION_MISSING` or `goatbuilder explain E_ACL_DECLARATION_MISSING`.

## What it means

`goat acl test` or `goat warrant` needs the goatnet's netbird_acl_taxonomy.yml and netbird_acl_matrix.yml and found them nowhere. goat carries no deployment's declaration, so it stops rather than check other rules.

## Why it happens

- the command ran outside a checkout and the context holds no declaration
- the goatnet does not serve its declaration at get.&lt;site>/acl/, so `goat context refresh` fetched none

## What to do

One line per step or alternative:

```text
goat acl test --declaration <dir> (a directory holding both files)
export GOAT_ACL_DECLARATION_DIR=<dir>
set acl_declaration_dir = "<dir>" in ~/.goat/contexts/<name>/profile.toml
goat context refresh (fills ~/.goat/contexts/<name>/acl/ when the goatnet serves it)
run from a checkout that carries ops/ansible/netbird_acl_taxonomy.yml and netbird_acl_matrix.yml
```

## Read more

- [`docs/design/goat-cli-error-codes.md`](../../../docs/design/goat-cli-error-codes.md)
- [Every error code](../errors.md)
- [How errors are interpreted](../../design/error-interpretation.md)

## Exit code

`goat` exits with 1.

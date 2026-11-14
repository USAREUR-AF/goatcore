#!/usr/bin/env bash
# goatbuilder-pr-scope.sh: decide whether a pull request changes only documents
# that no goatbuilder gate reads beyond the documents' configuration examples.
#
#   git diff --name-status <base> <head> | ops/ci/goatbuilder-pr-scope.sh
#   ops/ci/goatbuilder-pr-scope.sh --self-test
#
# Prints `docs-only` or `full`, and one line on stderr saying why. goatbuilder-pr
# runs only tests/doc_configs.rs for `docs-only` and every gate for `full`.
#
# docs-only means every changed path is a document that was added or edited:
#   - under docs/, or a *.md file outside crates/, ops/, tests/ and .github/;
#   - not under docs/tutorials/ (the tutorials' rehearsals run as written, and
#     tests/goatlite_budget.rs reads 00-goatlite.md);
#   - not under docs/reference/goatbuilder/ (the command reference is
#     regenerated and compared, and its pages hold the configuration reference).
# A document deleted, renamed, copied or changed in type is `full`: an error
# catalog entry, a phase catalog entry or a page may name its path. An empty or
# unreadable diff is `full`. When in doubt, the answer is `full`.
#
# Bash 3.2 portable.
set -euo pipefail

# One `git diff --name-status` line on stdin per change; the answer on stdout.
scope() {
  local status path rest n=0
  while IFS=$'\t' read -r status path rest; do
    [ -n "$status" ] || continue
    n=$((n + 1))
    case "$status" in
      M|A) ;;
      *) echo "full: $path is ${status} (only an added or edited document is docs-only)" >&2; echo full; return 0 ;;
    esac
    case "$path" in
      docs/tutorials/*|docs/reference/goatbuilder/*)
        echo "full: $path is read by a goatbuilder gate" >&2; echo full; return 0 ;;
      crates/*|ops/*|tests/*|.github/*)
        echo "full: $path is not a document" >&2; echo full; return 0 ;;
      docs/*|*.md) ;;
      *) echo "full: $path is not a document" >&2; echo full; return 0 ;;
    esac
  done
  if [ "$n" -eq 0 ]; then
    echo "full: no change was read" >&2
    echo full
    return 0
  fi
  echo "docs-only: $n document(s), none read by a goatbuilder gate beyond the configuration examples" >&2
  echo docs-only
}

self_test() {
  local fail=0 got want input
  check() {
    want=$1; input=$2
    got=$(printf '%b' "$input" | scope 2>/dev/null)
    if [ "$got" != "$want" ]; then
      echo "self-test: FAIL want $want got $got for: $input" >&2
      fail=1
    fi
  }
  check docs-only 'M\tdocs/adr/1238-x.md\nM\tdocs/adr/INDEX.md\n'
  check docs-only 'A\tdocs/design/new.md\n'
  check docs-only 'M\tREADME.md\n'
  check docs-only 'M\tdocs/images/figure.svg\n'
  check full 'M\tdocs/tutorials/00-goatlite.md\n'
  check full 'M\tdocs/reference/goatbuilder/command-reference.md\n'
  check full 'D\tdocs/design/old.md\n'
  check full 'R100\tdocs/a.md\tdocs/b.md\n'
  check full 'M\tdocs/adr/INDEX.md\nM\tcrates/goatbuilder/src/main.rs\n'
  check full 'M\tcrates/goatbuilder/CHANGELOG.md\n'
  check full 'M\tops/goatbuilder/compat/README.md\n'
  check full 'M\t.github/workflows/goatbuilder-pr.yml\n'
  check full 'M\tops/goatbuilder/schema/goatbuilder-v1.schema.json\n'
  check full 'M\tMakefile\n'
  check full ''
  if [ "$fail" -eq 0 ]; then
    echo "self-test: ok"
  fi
  return "$fail"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") scope ;;
  *) echo "usage: goatbuilder-pr-scope.sh [--self-test] < name-status" >&2; exit 2 ;;
esac

#!/usr/bin/env bash
# require-secret.sh — refuse a step whose secret is not there to use.
#
# Run it through ops/scripts/with-secrets, so the declared store has resolved
# references first:
#
#   ops/scripts/with-secrets ops/scripts/require-secret.sh <CODE> <VAR> <what it is for>
#
# Exit 0 when VAR holds a value. Exit 1, printing "<CODE>: ..." on stderr, when
# VAR is unset or empty, or still holds a secret store's reference (op://,
# bao://, bw://, or GOAT_SECRET_REF_PREFIX) after the injector ran. The message
# names the variable and what it is for, never its value. Exit 64 on a usage
# error. join-node and standup gate their mesh DNS step on it (GB-281).
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: require-secret.sh <CODE> <VAR> <what it is for>" >&2
  exit 64
fi
code="$1"; var="$2"; what="$3"
if ! [[ "$var" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
  echo "$code: '$var' is not a variable name" >&2
  exit 1
fi
value="${!var:-}"
if [[ -z "$value" ]]; then
  echo "$code: $var is not set: $what. Store it under the declared secrets posture, then re-run." >&2
  exit 1
fi
for scheme in op:// bao:// bw:// ${GOAT_SECRET_REF_PREFIX:+"$GOAT_SECRET_REF_PREFIX"}; do
  if [[ "$value" == "$scheme"* ]]; then
    echo "$code: $var still holds a secret store's reference ($scheme) after the injector ran: sign in to the store's tool, then re-run." >&2
    exit 1
  fi
done

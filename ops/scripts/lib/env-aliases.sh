# shellcheck shell=bash
# SPDX-License-Identifier: Apache-2.0
# ops/scripts/lib/env-aliases.sh — let a deployment keep its own environment
# variable names for the values goat's scripts read.
#
# A script that reads, say, NETHOST_HOST sources this file first:
#
#   . "$REPO/ops/scripts/lib/env-aliases.sh"
#
# It sources every ops/env-aliases.d/*.sh present in the tree, in lexical
# order. goat itself ships none; a deployment's overlay adds files whose lines
# are `goat_alias <NAME the script reads> <NAME the deployment exports>`, so an
# existing environment keeps working with no change. A name already set wins:
# an alias never overwrites a value.

goat_alias() {
  # goat_alias NEW OLD — set NEW from OLD when NEW is unset and OLD is set.
  local new="$1" old="$2"
  if [ -z "${!new+x}" ] && [ -n "${!old+x}" ]; then
    export "$new=${!old}"
  fi
}

_goat_aliases_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/ops/env-aliases.d"
if [ -d "$_goat_aliases_dir" ]; then
  for _goat_alias_file in "$_goat_aliases_dir"/*.sh; do
    [ -e "$_goat_alias_file" ] || continue
    # shellcheck source=/dev/null
    . "$_goat_alias_file"
  done
  unset _goat_alias_file
fi
unset _goat_aliases_dir

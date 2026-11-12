#!/usr/bin/env bash
# units/rollback.sh — a pair deployed with the ref's tools (avenue goatbuilder, phases 1-14),
# then goat and goatbuilder replaced on both sides with the previous release and phases
# 11-14 driven again: the peering still passes strict acceptance, with its keys and its
# agreement digest unchanged. Sourced by lib/unit.sh. The scenario reads: rollback-tools.
#
# The previous release comes from env (catalog units.rollback.previous_release): the newest
# published goat and goatbuilder older than the ref's crate versions.
#   FE2E_PREV_GOAT, FE2E_PREV_GB                  paths, or unavailable:<reason>
#   FE2E_PREV_GOAT_VERSION, FE2E_PREV_GB_VERSION  their versions
#   FE2E_PREV_GOAT_FROM, FE2E_PREV_GB_FROM        release-asset or built-from-tag
#   FE2E_REF_GOAT_VERSION, FE2E_REF_GB_VERSION    the ref's own

# What the peering works with on one side: the tunnel's public key, the fingerprint of every
# key file under the peering's pki, and the agreement's digest.
rb_state() { # $1 us, $2 them
  docker exec "$HP-$1-edge" sh -c "
    wg show $(iface "$1" "$2") public-key
    for f in /var/lib/goat-federation/$2/pki/*.pem; do case \"\$f\" in *key*) openssl pkey -in \"\$f\" -pubout 2>/dev/null | sha256sum | sed \"s|-|\$f|\" ;; esac; done
    sha256sum /var/lib/goat-federation/$2/agreement.json"
}
# Phase 14 with the previous goatbuilder, strict, and the counts read from its output.
rb_accept() { # $1 us, $2 them, $3 the previous goatbuilder
  local out; out="$(GB="$3" accept_as "$1" "$2" goatbuilder 2>&1)"; local rc=$?
  printf '%s\n' "$out" | tail -15
  printf '%s\n' "$out" | grep -q 'acceptance-counts:' && printf '%s\n' "$out" | grep -q 'skip=0' \
    && printf '%s\n' "$out" | grep -q 'fail=0' && ! printf '%s\n' "$out" | grep -qE 'ACCEPTANCE (FAILED|INCOMPLETE)' && [ "$rc" = 0 ]
}
export -f rb_accept

unit_main() {
  local n=5 s t p
  # Deployed with the ref's own tools: a --goat-version pin given to the run is for other
  # units (mixed versions), not for this one.
  for s in $PARTIES; do unset "GOAT_$s"; done
  for t in GOAT GB; do
    p="FE2E_PREV_$t"
    case "${!p:-}" in
      "") die "no previous release resolved for $t (catalog units.rollback.previous_release)" ;;
      unavailable:*) die "$(echo "${!p#unavailable:}" | tr _ ' ')" ;;
    esac
  done
  printf 'ref: goat %s, goatbuilder %s\nprevious: goat %s (%s), goatbuilder %s (%s)\n' \
    "${FE2E_REF_GOAT_VERSION:-}" "${FE2E_REF_GB_VERSION:-}" "$FE2E_PREV_GOAT_VERSION" "${FE2E_PREV_GOAT_FROM:-}" \
    "$FE2E_PREV_GB_VERSION" "${FE2E_PREV_GB_FROM:-}" > "$RECORD/rb-versions.txt"

  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  setup_hosts "$n"
  peer_up "$n" "$A" "$B" goatbuilder
  rec "rb-deployed" -- python3 -c '
import json, sys
want = set(sys.argv[2:]); seen = {}
for line in open(sys.argv[1]):
    r = json.loads(line)
    if r.get("name") in want: seen[r["name"]] = r.get("status")
for w in sorted(want): print(w, seen.get(w, "not recorded"))
sys.exit(0 if all(seen.get(w) == "pass" for w in want) else 1)' "$RECORD/report.jsonl" \
    "s$n-acceptance-as-$A-with-$B" "s$n-acceptance-as-$B-with-$A" "s$n-$A-$B-phases-11-to-12" "s$n-$B-$A-phases-11-to-12" \
    "s$n-$A-$B-phase-13" "s$n-$B-$A-phase-13"
  # A control, not a check: phases 11-13 again with the ref's own tools, so that a failure
  # after the rollback can be told apart from a rerun the ref itself does not survive.
  rec "rb-control-phases-11-to-13-with-the-ref-tools" --expect-in "federate the SPIRE trust domains" -- bash -c \
    "rc=0; drive '$A' '$B' goatbuilder 11 13 || rc=1; drive '$B' '$A' goatbuilder 11 13 || rc=1; exit \$rc"
  # What the peering works with just before the rollback.
  for s in "$A" "$B"; do rb_state "$s" "$(other "$s")" > "$RECORD/rb-$s-before.txt" 2>&1; done

  for s in "$A" "$B"; do
    replace_tools "$HP-$s-edge" "$FE2E_PREV_GOAT" "$FE2E_PREV_GB"
    replace_tools "$HP-$s-core" "$FE2E_PREV_GOAT" "$FE2E_PREV_GB"
  done
  rec "rb-rolled-back-versions" --expect-in "$A goat $FE2E_PREV_GOAT_VERSION" --expect-in "$A goatbuilder $FE2E_PREV_GB_VERSION" \
    --expect-in "$B goat $FE2E_PREV_GOAT_VERSION" --expect-in "$B goatbuilder $FE2E_PREV_GB_VERSION" -- bash -c \
    "for s in $A $B; do for t in goat goatbuilder; do echo \"\$s \$(docker exec $HP-\$s-edge /usr/local/bin/\$t --version)\"; done; done"

  rec "rb-phases-11-to-13-after-rollback" --expect-in "federate the SPIRE trust domains" -- bash -c \
    "rc=0; drive '$A' '$B' goatbuilder 11 13 || rc=1; drive '$B' '$A' goatbuilder 11 13 || rc=1; exit \$rc"
  rec "rb-acceptance-after-rollback" -- bash -c \
    "echo '== as $A'; rb_accept '$A' '$B' '$FE2E_PREV_GB' || exit 1; echo '== as $B'; rb_accept '$B' '$A' '$FE2E_PREV_GB'"
  for s in "$A" "$B"; do rb_state "$s" "$(other "$s")" > "$RECORD/rb-$s-after.txt" 2>&1; done
  rec "rb-keys-unchanged" -- bash -c \
    "diff '$RECORD/rb-$A-before.txt' '$RECORD/rb-$A-after.txt' && diff '$RECORD/rb-$B-before.txt' '$RECORD/rb-$B-after.txt' && cat '$RECORD/rb-$A-after.txt' '$RECORD/rb-$B-after.txt'"
  collect "$n"
}

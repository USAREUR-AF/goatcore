#!/usr/bin/env bash
# units/upgrade.sh — gate row E2, acceptance half (docs/design/goatbuilder-v1-gate.md): a
# peering finished by a released goatbuilder is accepted by the build under test.
#
# Both sovereigns are stood up by the released binary named in FE2E_UPGRADE_FROM (a file;
# its sha256 must equal FE2E_UPGRADE_FROM_SHA256), each by `goatbuilder federate <partner>`
# through phase 14, as units/onecmd.sh drives it. Then each driver host's goatbuilder is
# replaced by the build under test, nothing of phases 1-13 runs again, and phase 14 runs once
# more on each side against the same working directory, state.json, journal and agreement.
# The release half runs on a deployment checked out at the release: each sovereign's checkout
# is cloned from FE2E_UPGRADE_FROM_REF (a tag in this repository), as a deployment pinned to
# that release holds it. At the swap, each driver host's checkout takes the build's tree, its
# own declarations kept, as an upgrading operator updates it, and goatbuilder is replaced.
# Sourced by lib/unit.sh. Scenario: finished-on-release-accepted-by-build.
# shellcheck source=/dev/null
. "$HERE/units/onecmd.sh"

: "${FE2E_UPGRADE_FROM:?units/upgrade.sh: FE2E_UPGRADE_FROM names the released goatbuilder binary to stand up with}"
: "${FE2E_UPGRADE_FROM_SHA256:?units/upgrade.sh: FE2E_UPGRADE_FROM_SHA256 is that binary s published sha256}"
: "${FE2E_UPGRADE_FROM_REF:?units/upgrade.sh: FE2E_UPGRADE_FROM_REF names the release tag the checkouts start at (goatbuilder/v0.5.1)}"
UP_SRC_BUILD="$SRC"
UP_BUILD="$GB"

up_work() { echo "/var/lib/goat-federation/$1"; } # $1 them
# A deployment's checkout moves to the build: the build's tree laid over it, the files the
# deployment changed since its first commit (its declarations, what the command wrote) kept,
# then committed. $1 driver host.
# The files a deployment changed since its checkout's first commit: its declarations and what
# the command wrote, committed or not. $1 driver host -> one path a line.
up_kept_list() {
  local root; root="$(docker exec "$1" git -C /opt/goat rev-list --max-parents=0 HEAD | tail -1)"
  docker exec "$1" sh -c "cd /opt/goat && { git diff --name-only $root HEAD; git ls-files --others --exclude-standard; git diff --name-only; } | sort -u"
}

# Every path of the host's checkout against a source tree: a path whose blob differs, or that
# only one side has, must be one the deployment changed. $1 host, $2 source repo, $3 kept list.
up_only_kept_differ() {
  python3 - <(docker exec "$1" git -C /opt/goat ls-tree -r HEAD) <(git -C "$2" ls-tree -r HEAD) "$3" <<'PY'
import sys
def tree(p):
    out = {}
    for l in open(p):
        meta, path = l.rstrip("\n").split("\t", 1); out[path] = meta.split()[2]
    return out
host, src = tree(sys.argv[1]), tree(sys.argv[2])
kept = {l.strip() for l in open(sys.argv[3]) if l.strip()}
differ = sorted(p for p in set(host) | set(src) if host.get(p) != src.get(p))
extra = [p for p in differ if p not in kept]
print(f"{len(host)} paths on the host, {len(differ)} differ from the source, all of them the deployment's own" if not extra
      else f"{len(extra)} path(s) differ that the deployment did not change: {extra[:12]}")
sys.exit(1 if extra else 0)
PY
}
export -f up_kept_list up_only_kept_differ

up_checkout_to_build() {
  local h="$1" keep
  keep="$(up_kept_list "$h")"
  printf '%s\n' "$keep" > "$RECORD/up-$h-kept-files.txt"
  docker exec "$h" sh -c "cd /opt/goat && mkdir -p /tmp/up-keep && { printf '%s\n' \"$keep\" | while IFS= read -r f; do [ -n \"\$f\" ] && [ -e \"\$f\" ] && mkdir -p \"/tmp/up-keep/\$(dirname \"\$f\")\" && cp -p \"\$f\" \"/tmp/up-keep/\$f\"; done; }"
  # Files the release tracked that the build no longer has go; then the build's tree.
  git -C "$UP_SRC_BUILD" ls-tree -r --name-only HEAD | sort > "$WORK/up-build-files.txt"
  docker exec "$h" sh -c "cd /opt/goat && git ls-files | sort" | comm -23 - "$WORK/up-build-files.txt" \
    | grep -vxF -f "$RECORD/up-$h-kept-files.txt" > "$RECORD/up-$h-removed-files.txt" || true
  docker exec -i "$h" sh -c "cd /opt/goat && xargs -r git rm -q --" < "$RECORD/up-$h-removed-files.txt" || true
  git -C "$UP_SRC_BUILD" archive HEAD | docker exec -i "$h" tar -x -C /opt/goat
  docker exec "$h" sh -c "cd /tmp/up-keep && find . -type f | while IFS= read -r f; do cp -p \"\$f\" \"/opt/goat/\$f\"; done; rm -rf /tmp/up-keep"
  docker exec "$h" sh -c "cd /opt/goat && git add -A && git commit -q -m 'upgrade the checkout to goatcore $(git -C "$UP_SRC_BUILD" rev-parse --short=12 HEAD)'"
}

# The product trees of a driver host's checkout, as git tree ids: crates, ops/federation,
# ops/ansible/roles and ops/scripts. $1 host -> one line each.
up_product_trees() { docker exec "$1" sh -c "cd /opt/goat && for d in crates ops/federation ops/ansible/roles ops/scripts; do printf '%s %s\n' \"\$d\" \"\$(git rev-parse HEAD:\$d 2>/dev/null)\"; done"; }
src_product_trees() { local d; for d in crates ops/federation ops/ansible/roles ops/scripts; do printf '%s %s\n' "$d" "$(git -C "$1" rev-parse "HEAD:$d" 2>/dev/null)"; done; }
export -f up_product_trees src_product_trees


unit_main() {
  local s them h got
  got="$(sha256sum "$FE2E_UPGRADE_FROM" | cut -d' ' -f1)"
  [ "$got" = "$FE2E_UPGRADE_FROM_SHA256" ] || die "units/upgrade.sh: $FE2E_UPGRADE_FROM has sha256 $got, not the published $FE2E_UPGRADE_FROM_SHA256"
  [ "$(sha256sum "$UP_BUILD" | cut -d' ' -f1)" != "$got" ] || die "units/upgrade.sh: the build under test is the released binary itself; nothing would be upgraded"
  # The deployment's checkout at the release: SRC is what each sovereign's repository is
  # cloned from (lib/engine.sh, gen_sovereign).
  # From the build's own source tree, a git repository that carries the release tags (REPO is
  # the run's copy of the harness, which is not one).
  # shellcheck source=ops/federation-e2e/lib/srctree.sh
  . "$HERE/lib/srctree.sh"
  src_clone_ref "$UP_SRC_BUILD" "$FE2E_UPGRADE_FROM_REF" "$WORK/src-release" \
    || die "units/upgrade.sh: no clone of $UP_SRC_BUILD at $FE2E_UPGRADE_FROM_REF (a shallow source, or no such tag)"
  SRC="$WORK/src-release"; export SRC
  # The hosts mount $GB: the released binary until the swap.
  cp "$FE2E_UPGRADE_FROM" "$WORK/goatbuilder-released"; chmod 0755 "$WORK/goatbuilder-released"
  GB="$WORK/goatbuilder-released"; export GB
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  export -f oc_passed
  export OC_PREFIX HP ZENOH_IMAGE GB
  : > "$OUTSIDE"
  say "finished-on-release: $A and $B stood up by $("$GB" --version), each by \`goatbuilder federate <partner>\`"
  setup_hosts "$OC_PREFIX"
  prepare_edge "$A" "$B" true
  prepare_edge "$B" "$A" false
  for s in "$A" "$B"; do oc_core_staging "$s"; done
  if [ "$FE2E_CORE" = role ]; then oc_core_declare_role "$A" "$B"; oc_core_declare_role "$B" "$A"; fi
  for s in "$A" "$B"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  done
  oc_drive "$A" "$B"
  oc_status "$A" "$B"; oc_status "$B" "$A"
  for s in "$A" "$B"; do
    them="$(other "$s")"; h="$(drv_host "$s")"
    # By digest: a version string does not tell the release from a build of the same version.
    rec "up-$s-stood-up-by-the-release" --expect-in "$FE2E_UPGRADE_FROM_SHA256" -- docker exec "$h" sha256sum /usr/local/bin/goatbuilder
    rec "up-$s-checkout-is-the-release-s" -- bash -c "diff <(up_product_trees '$h') <(src_product_trees '$WORK/src-release')"
    up_kept_list "$h" > "$RECORD/up-$s-kept-before-the-swap.txt"
    rec "up-$s-only-its-own-files-differ-from-the-release" -- bash -c "up_only_kept_differ '$h' '$WORK/src-release' '$RECORD/up-$s-kept-before-the-swap.txt'"
    rec "up-$s-release-passed-phases-1-to-13" -- bash -c "oc_passed '$RECORD/status-$s.json' 1 2 3 4 5 6 7 8 9 10 11 12 13"
  done
  # Phase 14 by the release, each side: its result is kept beside the build's.
  for s in "$A" "$B"; do
    them="$(other "$s")"; local arc=0
    oc_accept "$s" "$them" > "$RECORD/acceptance-by-release-as-$s.out" 2>&1 || arc=$?
    rec "up-$s-accepted-by-the-release" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "cat '$RECORD/acceptance-by-release-as-$s.out'; exit $arc"
    host_exec "$(drv_host "$s")" sh -c "cd $(up_work "$them") && sha256sum state.json agreement.json 2>/dev/null; ls -1 evidence/acceptance 2>/dev/null" > "$RECORD/up-$s-work-before-the-swap.txt" || true
  done

  for s in "$A" "$B"; do
    docker exec "$(drv_host "$s")" cat "$(up_work "$(other "$s")")/state.json" > "$RECORD/up-$s-state-before-the-swap.json" 2>/dev/null || true
  done
  # The swap, as an upgrading operator does it: the checkout to the build's tree with the
  # deployment's own files kept, and goatbuilder replaced by the build.
  GB="$UP_BUILD"; export GB; SRC="$UP_SRC_BUILD"; export SRC
  for s in "$A" "$B"; do
    up_checkout_to_build "$(drv_host "$s")"
    replace_tools "$(drv_host "$s")" "$(goat_of "$s")" "$UP_BUILD"
  done
  for s in "$A" "$B"; do
    them="$(other "$s")"; h="$(drv_host "$s")"
    rec "up-$s-driver-runs-the-build" --expect-in "$(sha256sum "$UP_BUILD" | cut -d' ' -f1)" --expect-not-in "$FE2E_UPGRADE_FROM_SHA256" -- \
      docker exec "$h" sha256sum /usr/local/bin/goatbuilder
    rec "up-$s-checkout-is-the-build-s" -- bash -c "diff <(up_product_trees '$h') <(src_product_trees '$UP_SRC_BUILD')"
    rec "up-$s-only-its-own-files-differ-from-the-build" -- bash -c "up_only_kept_differ '$h' '$UP_SRC_BUILD' '$RECORD/up-$h-kept-files.txt'"
    rec "up-$s-state-is-the-release-s" --expect-in "goatbuilder.federate.state/v1" -- docker exec "$h" sh -c "head -c 400 $(up_work "$them")/state.json"
  done

  # The control: the read check can fail. A copy of the release's state.json with a schema
  # this build does not read is refused (GB-182), so a pass below is the build reading it.
  rec "up-$A-control-newer-schema-is-refused" --expect-in "GB-182" -- bash -c \
    "h=\$(drv_host '$A'); w=\$(echo /var/lib/goat-federation/$B); docker exec \$h sh -c \"rm -rf /tmp/up-ctl && cp -a \$w /tmp/up-ctl && sed -i 's#goatbuilder.federate.state/v1#goatbuilder.federate.state/v99#' /tmp/up-ctl/state.json && cd /opt/goat && FED_WORK=/tmp/up-ctl goatbuilder -C /opt/goat federate status $B 2>&1; rm -rf /tmp/up-ctl\"; true"
  # Phase 14 by the build, against the release's working directory.
  for s in "$A" "$B"; do
    them="$(other "$s")"; local brc=0 out="$RECORD/acceptance-by-build-as-$s.out"
    oc_accept "$s" "$them" > "$out" 2>&1 || brc=$?
    rec "up-$s-accepted-by-the-build" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
      --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "BLOCKED" -- bash -c "cat '$out'; exit $brc"
    # A refusal that names the journal, the lock or a schema means the build did not read
    # what the release wrote (GB-182: a state.json it does not read).
    rec "up-$s-build-read-the-release-s-record" -- bash -c \
      "l=\$(sed 's/\x1b\[[0-9;]*m//g' '$out' | grep -iE 'GB-182|refus[a-z]*.*(journal|state\.json|lock|schema|evidence)|(journal|state\.json|lock|schema).*refus' | head -5)
       [ -z \"\$l\" ] && { echo 'no refusal names the journal, the lock, the state or the evidence schema'; exit 0; }; echo \"\$l\"; exit 1"
  done
  for s in "$A" "$B"; do
    docker exec "$(drv_host "$s")" cat "$(up_work "$(other "$s")")/state.json" > "$RECORD/up-$s-state-after-phase-14.json" 2>/dev/null || true
    rec "up-$s-phases-1-to-13-not-run-again" -- python3 - "$RECORD/up-$s-state-before-the-swap.json" "$RECORD/up-$s-state-after-phase-14.json" <<'PY'
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:3])
pa, pb = a.get("phases", {}), b.get("phases", {})
changed = [n for n in range(1, 14) if pa.get(str(n)) != pb.get(str(n))]
seen = {(x.get("run"), x.get("phase"), x.get("at")) for x in a.get("attempts", [])}
new = [x for x in b.get("attempts", []) if (x.get("run"), x.get("phase"), x.get("at")) not in seen and int(x.get("phase", 0)) <= 13]
print("phase records 1-13 changed:", changed, "; new attempts on phases 1-13:", [(x.get("phase"), x.get("event")) for x in new])
sys.exit(1 if changed or new else 0)
PY
  done
  collect "$OC_PREFIX"
}

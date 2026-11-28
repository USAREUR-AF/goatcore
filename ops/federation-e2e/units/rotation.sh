#!/usr/bin/env bash
# units/rotation.sh — gate row B3 (docs/design/goatbuilder-v1-gate.md), the proving scenario of
# docs/design/federation-key-rotation.md §6: each key rotated in turn on a running peering,
# the command run again on both sides, and a continuous flow of samples throughout.
#
# The peering is units/forwarder.sh's ({A} behind NAT, listening through its forwarder, so
# the carrier key is covered). Then a flow of one numbered sample a second runs each way
# (lib/flow.sh), with a reader on each receiving core that records each number's arrival
# time (lib/samples.py), for the whole scenario. For each key in turn (leaves, carrier,
# transport, signing), {A} rotates it and then {B}: the material is read before and after,
# `goatbuilder federate rotate <partner> <key>` runs on the rotating side, and the checks of
# §6 read what changed, what did not, what was retired, what a rerun of the ordinary command
# does, and which numbers each reader missed during the key's step (samples-<key>).
#
# A key the goatbuilder under test refuses to rotate (GB-189, designed and not built,
# GB-282, the CA rotation, decided in ADR 1242 and not built yet,
# GB-312, a carrier whose forwarder is declared by hand,
# GB-317, a carrier on a side with no forwarder,
# or one of the IPsec rotation's refusals, GB-298, GB-303, GB-309 and GB-328; a
# build before GB-317 refused that with GB-189) is not a failure of the run: its checks are marked
# BLOCKED with the refusal's own words, and the unit goes on to the next key. A rotation that
# is attempted and fails stops the unit. The partner's step for transport and signing waits
# on §5; the unit names it where it is reached. The forced SPIRE CA rotation that §6 puts
# last is not driven yet, and its check reads BLOCKED.
# shellcheck source=/dev/null
. "$HERE/units/pair.sh"
# shellcheck source=/dev/null
. "$HERE/units/forwarder.sh"
eval "$(declare -f unit_main | sed '1s/^unit_main /rot_peering_up /')"
# shellcheck source=/dev/null
. "$HERE/lib/flow.sh"

# FE2E_ROT_KEYS: a subset of the keys, in this order, for a run that measures some of them
# (a key left out is not rotated and its checks are not recorded).
ROT_KEYS="${FE2E_ROT_KEYS:-leaves carrier transport signing}"

# drive with more arguments for the command (DRIVE_EXTRA, split on spaces): the partner's
# step passes --accept-partner-package with it.
# shellcheck disable=SC2016 # $DRIVE_EXTRA is written into drive_x's body, expanded when it runs
eval "$(declare -f drive | sed '1s/^drive /drive_x /; s/\(goatbuilder -C \/opt\/goat --yes federate .*--to "\$to"\)$/\1 $DRIVE_EXTRA/')"
export -f drive_x
# drive's environment for `goatbuilder federate rotate <partner> <key>` (ROT_KEY): the rotation
# re-runs phases (8 for the leaves, 3 after a carrier or transport key), which read the same
# values as the command does (FED_CA_CERT/KEY, FED_WORK, GOAT_CONTEXT, the signing key, the
# directory). drive() is copied with only its goatbuilder command swapped, and the copy is
# refused when the swap finds no line or changes any line but that one.
make_drive_rotate() {
  eval "$(declare -f drive | sed '1s/^drive /drive_rotate /; s/goatbuilder -C \/opt\/goat --yes federate "\$them" --as "\$s" --from "\$from" --to "\$to"$/goatbuilder -C \/opt\/goat --yes federate rotate "$them" "$ROT_KEY" --as "$s"/')"
  # shellcheck disable=SC2016 # the literal text of the swapped line, not an expansion
  declare -f drive_rotate | grep -q 'federate rotate "\$them" "\$ROT_KEY"' \
    || die "rotation.sh: drive() no longer has the command line drive_rotate swaps; update the sed above"
  local changed
  changed="$(diff <(declare -f drive | sed 1d) <(declare -f drive_rotate | sed 1d) | grep -c '^[<>]' || true)"
  [ "$changed" = 2 ] \
    || die "rotation.sh: drive_rotate differs from drive() in $changed diff lines, not the one swapped command line (2); update the sed above"
}
make_drive_rotate
export -f drive_rotate

# Every stop goes through here: the flows are stopped first, so none keeps releasing.
rot_die() {
  local l; for l in a2b b2a; do [ -e "$RECORD/flow-$l.pid" ] && touch "$RECORD/flow-$l.stop"; done
  die "$@"
}

# The material each key is, as the checks compare it. $1 side, $2 key.
rot_material() {
  local s="$1" k="$2" them h; them="$(other "$s")"; h="$(drv_host "$s")"
  case "$k" in
    leaves) docker exec "$h" sh -c "for f in /var/lib/goat-federation/$them/pki/peering-*.pem /var/lib/goat-federation/$them/pki/republisher.pem /var/lib/goat-federation/$them/pki/core.pem; do case \$f in *-key.pem) ;; *) [ -f \$f ] && printf '%s %s\n' \"\$(basename \$f)\" \"\$(openssl x509 -in \$f -noout -serial)\" ;; esac; done" ;;
    transport) docker exec "$HP-$s-edge" wg show "$(iface "$s" "$them")" public-key ;;
    carrier) if [ "$s" = "$NATED" ]; then docker exec "$HP-$s-edge" wg show wg-carry0 public-key; else echo "no carrier on $s"; fi ;;
    signing)
      # What a signing rotation changes: the seed this side signs for this peering with (this
      # peering's edge group's goat_federation_signing_key, else FED_SIGNING_KEY as the drive
      # passes it), by digest,
      # and this party's verifying key as the partner's keyring holds it (spec §5.1).
      docker exec "$h" sh -c "k=\$(grep -hoE '^goat_federation_signing_key:[[:space:]]*\\S+' /opt/goat/ops/ansible/group_vars/${them}_edge/*.yml 2>/dev/null | head -1 | sed 's/^[^:]*:[[:space:]]*//; s/[\"\x27]//g'); k=\${k:-/etc/fed-e2e/signing.key}; printf 'seed %s %s\n' \"\$k\" \"\$(sha256sum \"\$k\" | cut -d' ' -f1)\""
      docker exec "$(drv_host "$them")" sh -c "grep -E '^$s=' /var/lib/goat-federation/$s/keys.txt | sed 's/^/in the partner keyring: /'" ;;
  esac
}
export -f rot_material

rot_all_material() { # $1 side, $2 file
  local k; for k in $ROT_KEYS; do echo "## $k"; rot_material "$1" "$k"; done > "$2" 2>&1
}

# The checks of one key on one side, and of the key itself, as the catalog names them.
# What a refusal of key $1 on side $2 (first|second) marks, one "<check>\t<why>" per line:
# "refused" for the refused side's own checks and the key's (its samples and old-retired
# checks need both sides rotated). The other side's checks are never marked: it runs its own
# rotate and records its own result, and a mark outranks a passing step.
rot_refused_marks() { # $1 key, $2 first|second
  local k="$1" o="$2" c
  for c in $(rot_checks "$k" "$o"); do printf '%s\trefused\n' "$c"; done
  for c in $(rot_checks "$k" first); do case "$c" in *-first|*-first-ran) ;; *) [ "$o" = first ] || printf '%s\trefused\n' "$c" ;; esac; done
  return 0
}

rot_checks() { # $1 key, $2 first|second
  local k="$1" o="$2" c
  for c in "rotate-$k-$o-ran" "rotated-$k-$o" "kept-$k-$o" "partner-kept-$k-$o" "acceptance-$k-$o" "rerun-after-$k-$o"; do echo "$c"; done
  [ "$k" = signing ] && echo "partner-keyring-$o"
  [ "$o" = first ] && { echo "samples-$k"; case "$k" in leaves|carrier|transport) echo "old-retired-$k" ;; esac; }
  return 0
}

# The keys this run refused, each with the refusal's first line.
ROT_REFUSED=""

# One snapshot file of a side's material. A side's own snapshot is named for it alone;
# the partner's, taken while the other side rotates, carries both names, so the second
# rotation never overwrites the first side's own before-file.
rot_snap() { # $1 key, $2 whose material, $3 the rotating side, $4 before|after
  if [ "$2" = "$3" ]; then echo "$RECORD/rot-$1-$2-$4.txt"
  else echo "$RECORD/rot-$1-$3-partner-$2-$4.txt"; fi
}

rot_one() { # $1 key, $2 side
  local k="$1" s="$2" them h out rc ord c o why; them="$(other "$s")"; h="$(drv_host "$s")"
  ord="$([ "$s" = "$NATED" ] && echo first || echo second)"
  # A side runs its own rotate even after the other side's refusal: a refusal changes
  # nothing, and each side's own answer is the evidence (GB-312 on one side, GB-317 on the other).
  local sb sa pb pa
  sb="$(rot_snap "$k" "$s" "$s" before)"; sa="$(rot_snap "$k" "$s" "$s" after)"
  pb="$(rot_snap "$k" "$them" "$s" before)"; pa="$(rot_snap "$k" "$them" "$s" after)"
  rot_all_material "$s" "$sb"
  rot_all_material "$them" "$pb"
  out="$RECORD/rot-$k-$s-rotate.out"
  rc=0; ROT_KEY="$k" drive_rotate "$s" "$them" goatbuilder 0 0 > "$out" 2>&1 || rc=$?
  why="$(grep -m1 -oE '(GB-189|GB-282|GB-298|GB-303|GB-309|GB-312|GB-317|GB-328)[^|]*' "$out" || true)"
  if [ "$rc" != 0 ] && [ -n "$why" ]; then
    # A refusal changes nothing: both sides' material is read again and compared with the
    # material before it, the edge's live carrier pair is shown, and no carrier key was set
    # aside (no .previous-carrier-* directory on the edge).
    rot_all_material "$s" "$RECORD/rot-$k-$s-after-refusal.txt"
    rot_all_material "$them" "$RECORD/rot-$k-$them-after-refusal.txt"
    rec "rot-refused-$k-$s-changed-nothing" -- bash -c \
      "cat '$out' | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'GB-[0-9]+' | head -3
       diff '$sb' '$RECORD/rot-$k-$s-after-refusal.txt' && diff '$pb' '$RECORD/rot-$k-$them-after-refusal.txt' || exit 1
       docker exec $HP-$s-edge sh -c 'wg show wg-carry0 2>/dev/null | grep -E \"public key|peer:|latest handshake\"; ls -d /etc/goat-peering-wg/.previous-carrier-* 2>/dev/null && exit 1; exit 0'"
    # Refused, by this build's own word: the refused side's checks and the key's wait for a
    # build that rotates it; the other side runs its own rotate and records its own result.
    ROT_REFUSED="$ROT_REFUSED $k"
    local ord c kind
    ord="$([ "$s" = "$NATED" ] && echo first || echo second)"
    while IFS="$(printf '\t')" read -r c kind; do
      case "$kind" in
        refused) mark "key-rotation.$c" BLOCKED "\`rotate $them $k\` is refused by the goatbuilder under test: $why" ;;
      esac
    done <<EOF_MARKS
$(rot_refused_marks "$k" "$ord")
EOF_MARKS
    say "rotation: $k refused on $s ($why); its checks are BLOCKED, and the next key goes on"
    return 0
  fi
  local f0="$FAILS"
  # Not refused: the refusal's changed-nothing check does not apply to this side's key.
  mark "key-rotation.refused-changed-nothing-$k-$ord" "NOT APPLICABLE" "\`rotate $them $k\` ran on $s: nothing was refused"
  rec "rot-rotate-$k-$s-ran" -- bash -c "tail -30 '$out'; exit $rc"
  [ "$FAILS" = "$f0" ] || rot_die "\`goatbuilder federate rotate $them $k\` failed on $s (exit $rc): $out"
  rot_all_material "$s" "$sa"
  if [ "$k" = signing ]; then
    # Signing: the seed must change here; the partner's keyring is its own check below.
    rec "rot-rotated-$k-$s" -- bash -c \
      "a=\$(grep '^seed ' '$sb'); b=\$(grep '^seed ' '$sa'); echo \"before: \$a\"; echo \"after:  \$b\"; [ -n \"\$a\" ] && [ -n \"\$b\" ] && [ \"\$a\" != \"\$b\" ]"
  else
  rec "rot-rotated-$k-$s" -- bash -c \
    "diff <(sed -n '/^## $k\$/,/^## /p' '$sb') <(sed -n '/^## $k\$/,/^## /p' '$sa') && { echo 'the $k material did not change'; exit 1; } || exit 0"
  fi
  rec "rot-kept-$k-$s" -- python3 - "$sb" "$sa" "$k" <<'PY'
import re, sys
def parts(p):
    out, cur = {}, None
    for l in open(p):
        if l.startswith("## "): cur = l[3:].strip(); out[cur] = []
        elif cur: out[cur].append(l)
    return out
a, b, k = parts(sys.argv[1]), parts(sys.argv[2]), sys.argv[3]
bad = re.compile(r"error|no such|not found|unable|cannot|permission denied|usage:", re.I)
problems = []
for x in a:
    if x == k: continue
    body = "".join(a[x]).strip()
    if not body or bad.search(body): problems.append(f"{x}: no material read before ({body[:80]!r})")
    elif a.get(x) != b.get(x): problems.append(f"{x}: changed")
print("other keys:", sorted(x for x in a if x != k), "; problems:", problems or "none")
sys.exit(1 if problems else 0)
PY
  rot_all_material "$them" "$pa"
  rec "rot-partner-kept-$k-$s" -- diff "$pb" "$pa"
  if [ "$k" = signing ]; then
    # Our new verifying key in the partner's keyring is the partner's step (§5), not built yet.
    local ord; ord="$([ "$s" = "$NATED" ] && echo first || echo second)"
    mark "key-rotation.partner-keyring-$ord" BLOCKED "the partner's step that takes our new verifying key into its keyring waits on the operator's decision in docs/design/federation-key-rotation.md §5"
  fi
  case "$k" in
    # LAB TRIAL (ADR 1229, local only): the partner's step after a transport rotation, as
    # §5 and the K4 row give it: the partner's phase 5 takes our new package once its digest
    # is confirmed (--accept-partner-package, the digest read from our side as on the call),
    # and its phases to 14 run on it; its phase 11 replaces our peer key, and the tunnel
    # comes back. The times bound the tunnel gap: our rotation ends, their step starts, the
    # first handshake after it.
    transport)
      local d t0 pub fp ad
      d="$(docker exec "$(drv_host "$s")" sha256sum "/var/lib/goat-federation/$them/ours.canonical.json" | cut -d' ' -f1)"
      # The lab binds the keyring by hand (lib/engine.sh), so no confirmation 1 is
      # journaled on the partner's side: its phase 5 asks the keys with the package. The
      # values are the ones the call would read: our verifying key and our CA fingerprint.
      pub="$(cat "$WORK/$s/keys/signing.pub")"
      fp="$(docker exec "$(drv_host "$s")" python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["identity"]["ca_fingerprint"])' "/var/lib/goat-federation/$them/ours.canonical.json")"
      # Nor is confirmation 2 journaled through the command: the agreement digest is
      # passed too. The agreement carries no WireGuard key, so it is the one held now.
      ad="$(docker exec "$(drv_host "$them")" sha256sum "/var/lib/goat-federation/$s/agreement.json" | cut -d' ' -f1)"
      t0="$(date +%s.%N)"; printf '%s\tpartner-step-start\n' "$t0" >> "$RECORD/rot-$k-$s.times"
      rec "rot-partner-step-$k-$s" -- bash -c "DRIVE_EXTRA='--accept-partner-package $d --partner-key $pub --partner-ca-fingerprint $fp --agreement-digest $ad' drive_x '$them' '$s' '$AVENUE' 5 14"
      # drive_x stops at phase 10's hand-off; phase 11 is where the partner replaces our
      # peer key. It stops at 13: phase 14 through drive has no acceptance surfaces, and
      # the acceptance step below covers 14.
      # rec.py starts its command as a program; drive is a function, exported to child
      # shells by lib/engine.sh's export_helpers, so it runs through bash -c.
      rec "rot-partner-step-from-11-$k-$s" -- bash -c "drive '$them' '$s' '$AVENUE' 11 13"
      printf '%s\tpartner-step-end\n' "$(date +%s.%N)" >> "$RECORD/rot-$k-$s.times"
      rec "rot-tunnel-back-$k-$s" -- bash -c \
        "i=\$(iface '$s' '$them'); for _ in \$(seq 1 30); do t=\$(docker exec $HP-$s-edge wg show \$i latest-handshakes | cut -f2); [ \"\${t:-0}\" -gt '${t0%%.*}' ] 2>/dev/null && break; sleep 2; done
         docker exec $HP-$s-edge wg show \$i latest-handshakes; printf 'first handshake after the partner step began: %s (step began %s)\\n' \"\${t:-none}\" '${t0%%.*}'
         [ \"\${t:-0}\" -gt '${t0%%.*}' ] 2>/dev/null"
      ;;
    signing) rot_die "the partner's step after a $k rotation waits on the operator's decision in docs/design/federation-key-rotation.md §5" ;;
  esac
  for x in "$s" "$them"; do
    rec "rot-acceptance-$k-$x" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
      --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" -- bash -c "accept_as '$x' '$(other "$x")' '$AVENUE'"
  done
  # The ordinary command again, from phase 1 with nothing changed: the resume skips 3, 6
  # and 9 (and the stops at 5 and 10, unchanged since they passed), and nothing rotates.
  local ro="$RECORD/rot-rerun-after-$k-$s.out"
  rec "rot-rerun-after-$k-$s" -- bash -c \
    "drive '$s' '$them' '$AVENUE' 1 14 > '$ro' 2>&1; rc=\$?; tail -40 '$ro'
     [ \$rc = 0 ] || exit \$rc
     for n in 3 6 9; do grep -q \"phase \$n unchanged since it passed\" '$ro' || { echo \"phase \$n was not skipped\"; exit 1; }; done
     ! grep -qF -- '--rotate' '$ro' || { echo 'the ordinary command rotated something'; exit 1; }"
}

# What a key's rotation retires, once both sides have rotated it (§6 old-retired-<key>).
rot_retired() { # $1 key
  local k="$1" s them old
  case " $ROT_REFUSED " in *" $k "*) return 0 ;; esac
  case "$k" in
    transport)
      for s in "$NATED" "$PARTNER"; do them="$(other "$s")"
        old="$(sed -n '/^## transport$/,/^## /p' "$(rot_snap "$k" "$s" "$s" before)" | grep -vE '^## ' | head -1)"
        printf '%s %s\n' "$s" "$old" >> "$RECORD/rot-$k-old-keys.txt"
      done
      rec "rot-old-retired-$k" -- bash -c \
        "while read -r s old; do them=\$(other \$s); p=\$(docker exec $HP-\$them-edge wg show \$(iface \$them \$s) peers) || { echo \"could not read \$them's peers\"; exit 1; }
           [ -n \"\$old\" ] || { echo \"no old key was read for \$s\"; exit 1; }
           ! printf '%s\\n' \"\$p\" | grep -qxF \"\$old\" || { echo \"\$them still holds \$s's old key\"; exit 1; }; done < '$RECORD/rot-$k-old-keys.txt'; echo 'no old key in a peer list'" ;;
    carrier)
      old="$(sed -n '/^## carrier$/,/^## /p' "$(rot_snap "$k" "$NATED" "$NATED" before)" | grep -vE '^## ' | head -1)"
      rec "rot-old-retired-$k" -- bash -c \
        "p=\$(docker exec $FWD_HOST wg show wg-carry0 peers) || { echo 'could not read the forwarder peers'; exit 1; }
         [ -n '$old' ] || { echo 'no old carrier key was read'; exit 1; }
         ! printf '%s\\n' \"\$p\" | grep -qxF '$old' || { echo 'the forwarder still holds the old carrier key'; exit 1; }; echo 'the old carrier key is in no peer list on the forwarder'" ;;
    leaves)
      mark "key-rotation.old-retired-$k" BLOCKED "a session presenting the old leaf until its notAfter (the exposure §3 states) is not driven by the harness yet" ;;
  esac
}

unit_main() {
  rot_peering_up
  say "rotation: a flow each way for the whole scenario, then each key in turn on each side"
  flow_start "$NATED" "$PARTNER" a2b reader
  flow_start "$PARTNER" "$NATED" b2a reader
  local k s
  for k in $ROT_KEYS; do
    printf '%s\tstart\n' "$(date +%s.%N)" >> "$RECORD/rot-$k.window"
    # Each side's window, from its rotation to the end of its partner's step and the
    # acceptance after it: the samples check takes a lossy gap only inside one.
    for s in "$NATED" "$PARTNER"; do
      printf '%s\tside-start\t%s\n' "$(date +%s.%N)" "$s" >> "$RECORD/rot-$k.window"
      rot_one "$k" "$s"
      printf '%s\tside-end\t%s\n' "$(date +%s.%N)" "$s" >> "$RECORD/rot-$k.window"
    done
    printf '%s\tend\n' "$(date +%s.%N)" >> "$RECORD/rot-$k.window"
    rot_retired "$k"
  done
  mark "key-rotation.spire-ca-rotation" BLOCKED "the forced SPIRE CA rotation (localauthority x509 prepare, then activate, then a phase 13 rerun) is not driven by the harness yet"
  sleep 20 # values after the last rotation, before the flows stop
  flow_stop a2b; flow_stop b2a
  { flow_report a2b; flow_report b2a; } > "$RECORD/rot-samples.json"
  # Decidable now: data resumes. Values are stored inside and after every rotation's window,
  # and the last five of each flow are stored.
  # The keys rotated, without those this build refused: a refused key has no step to measure.
  local rotated="" r
  for r in $ROT_KEYS; do case " $ROT_REFUSED " in *" $r "*) ;; *) rotated="$rotated $r" ;; esac; done
  # shellcheck disable=SC2086
  rec "rot-samples-resume" -- python3 - "$RECORD" $rotated <<'PY'
import json, os, sys
d, keys = sys.argv[1], sys.argv[2:]
def rows(l): return [r.rstrip("\n").split("\t") for r in open(os.path.join(d, f"flow-{l}.read")) if r.strip()]
def window(k):
    t = [float(r.split("\t")[0]) for r in open(os.path.join(d, f"rot-{k}.window")) if r.strip()]
    return min(t), max(t)
bad = []
for l in ("a2b", "b2a"):
    rs = rows(l); ok = [r[3] == "stored" for r in rs]; t = [float(r[0]) for r in rs]
    if len(ok) < 5 or not all(ok[-5:]): bad.append(f"{l}: the last five values are not all stored")
    for k in keys:
        lo, hi = window(k)
        inside = [o for o, x in zip(ok, t) if lo <= x <= hi]; after = [o for o, x in zip(ok, t) if x > hi]
        if not any(inside): bad.append(f"{l}: nothing stored during the {k} rotation")
        if not any(after): bad.append(f"{l}: nothing stored after the {k} rotation")
print(open(os.path.join(d, "rot-samples.json")).read().strip()); print("problems:", bad or "none")
sys.exit(1 if bad else 0)
PY
  # Per key, at both readers: no number missing across the step, and for transport and
  # carrier, whose step has a gap by design (§3), every number re-delivered after it.
  for k in $ROT_KEYS; do
    case " $ROT_REFUSED " in *" $k "*) continue ;; esac
    local rots
    rots="$(awk -F'\t' '$2 == "side-start" { a[$3] = $1 } $2 == "side-end" && ($3 in a) { printf "--rotation %s %s ", a[$3], $1 }' "$RECORD/rot-$k.window")"
    rec "rot-samples-$k" -- bash -c \
      "lo=\$(cut -f1 '$RECORD/rot-$k.window' | sort -n | head -1); hi=\$(cut -f1 '$RECORD/rot-$k.window' | sort -n | tail -1)
       python3 '$HERE/lib/samples.py' check --from \$lo --to \$hi $(case "$k" in transport|carrier) echo --gap ;; esac) $rots \
         --sent '$RECORD/flow-a2b.sent' --arrived '$RECORD/flow-a2b.arrived' --sent '$RECORD/flow-b2a.sent' --arrived '$RECORD/flow-b2a.arrived'"
  done
  # Not decidable yet: how much may be lost across a rotation (the same question as B4's).
  mark "key-rotation.samples-loss" BLOCKED "loss tolerance across a rotation awaits the operator's decision (as B4); the flow's figures are in rot-samples.json"
  rec "rot-no-pending" -- bash -c "for s in $NATED $PARTNER; do o=\$(docker exec \$(drv_host \$s) find /var/lib/goat-federation -name .rotation-pending) || { echo \"could not search \$s\"; exit 1; }; [ -z \"\$o\" ] || { echo \"\$o\"; exit 1; }; done; echo none"
}

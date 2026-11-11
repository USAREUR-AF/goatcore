#!/usr/bin/env bash
# units/killresume.sh — a run killed inside each phase, then resumed with --from P --to P.
# Sourced by lib/unit.sh. Scenario: kill-resume (scenarios/catalog.yaml).
#
# Two parties through one avenue, in the order of the engine's peer_up: phases 1-5 each
# side, the exchange, 6-10 each side, the review and admission, 11-12, 13, then phase 14
# acceptance from both sides. The first side (the listener: {A} under goatbuilder, {B}
# under script, as in the pair unit) never runs a phase straight through. For each phase
# P from 1 to 13 it starts `drive ... P P`, waits until the phase is doing work, waits a
# little more so that the kill lands inside the phase, kills -9 the driver's whole process
# tree inside the edge host (goatbuilder or peering-standup.sh and everything under it:
# bash, goat, ansible-playbook, its workers) and the docker exec client, and then runs
# the phase again with --from P --to P. The second side runs uninterrupted: it is the
# clean run the first side's end state is compared with.
#
# The drivers keep no journal: resume is --from/--to over phases they claim idempotent.
# A killed phase that leaves state its rerun cannot recover is a FAIL, with the killed
# run's output and the state it left in RECORD (kr-phase-<P>-killed.out,
# kr-phase-<P>-after-kill.txt) and the timing of every kill in RECORD/kills.tsv.
#
# Steps:
#   kr-phase-<P>-killed-mid-phase  the driver was still inside phase P when it was killed
#   kr-phase-<P>-resumed           the rerun of phase P exits 0
#   kr-all-phases-resumed          every kr-phase-<P>-resumed passed
#   kr-* end state                 keys, nothing duplicated, the same shape as the clean
#                                  side, one agreement digest, a handshake, strict acceptance

kr_first() { [ "$AVENUE" = goatbuilder ] && echo "$A" || echo "$B"; }

# Seconds to wait after the phase starts doing work, before the kill. Chosen per phase from
# measured durations (the rerun's duration_s in report.jsonl is the phase's full length) so
# that the kill lands inside the phase. A phase shorter than its delay ends before the kill,
# and kr-phase-<P>-killed-mid-phase says so.
kr_delay() {
  case "$1" in
    1) echo 0.05 ;; 2) echo 8 ;; 3) echo 0.4 ;; 4) echo 0.02 ;; 5) echo 0.05 ;;
    6) echo 0.02 ;; 7) echo 0.02 ;; 8) echo 0.2 ;; 9) echo 0.05 ;; 10) echo 0 ;;
    11) echo 8 ;; 12) echo 15 ;; 13) echo 3 ;; *) echo 0 ;;
  esac
}

# Runs inside the edge host, started just before the driver: it waits for the driver's
# process, then for the phase to start doing work (a process under the driver running the
# phase's command: preflight.py, with-aws or ansible, `goat federate`, mint-peering-leaves,
# directory.py), waits $1 seconds more, and kills the driver's whole process tree -9, all
# inside the edge so that no docker exec round trip sits between the decision and the kill.
# (A bare `bash -c <one command>` execs that command, so the shell goatbuilder starts per
# phase is not itself a reliable sign.) What is not a descendant (a container dockerd
# started, a systemd unit, a daemonized ssh master) is left running, as it would be when
# an operator's session dies. Bracketed patterns keep pgrep from matching its own shell.
KR_WATCH='
# Only ever inside a lab host: on the machine itself every lab'"'"'s drivers share the
# process table, and a match by name there would kill another run.
[ -f /.dockerenv ] || { echo "refused: not inside a lab container"; exit 2; }
delay="$1"
roots="goatbuilde[r] -C /opt/goat --yes federate|ops/scripts/peering-standu[p].sh"
work="preflight\.py|with-aws|ansible|/goat federate|mint-peering|directory\.py"
t0="$(date +%s.%N)"
since() { awk -v a="$t0" -v b="$(date +%s.%N)" "BEGIN { printf \"%.2f\", b - a }"; }
tree() { # $1 a pid: it and every descendant
  all="$1"; f="$1"
  while [ -n "$f" ]; do
    n=""; for q in $f; do n="$n $(ps -o pid= --ppid "$q" 2>/dev/null | tr "\n" " ")"; done
    n="$(echo $n)"; [ -z "$n" ] || all="$all $n"; f="$n"
  done
  echo "$all"
}
root=""; i=0
while [ -z "$root" ] && [ "$i" -lt 600 ]; do root="$(pgrep -o -f "$roots" || true)"; [ -n "$root" ] || sleep 0.05; i=$((i + 1)); done
if [ -z "$root" ]; then echo "work never"; echo "alive 0"; echo "kill none: no driver process appeared"; exit 0; fi
echo "driver $root at $(since)s"
seen=""
while kill -0 "$root" 2>/dev/null; do
  if ps -o args= -p "$(tree "$root" | tr " " ",")" 2>/dev/null | grep -qE "$work"; then seen="$(since)"; break; fi
  sleep 0.02
done
echo "work ${seen:-never}"
[ -z "$seen" ] || sleep "$delay"
if kill -0 "$root" 2>/dev/null; then echo "alive 1"; else echo "alive 0"; fi
echo "kill at $(since)s"
all="$(tree "$root")"
echo "killing:"; ps -o pid=,args= -p "$(echo $all | tr " " ",")" 2>/dev/null | cut -c1-200
kill -STOP $all 2>/dev/null || true
kill -9 $all 2>/dev/null || true
# Anything of the driver that was reparented before the walk.
pkill -9 -f "ansible-playboo[k]|goatbuilde[r] -C /opt/goat|peering-standu[p].sh|/usr/local/bin/goa[t] federate" 2>/dev/null || true
sleep 0.5
echo "left:"; pgrep -a -f "ansible-playboo[k]|goatbuilde[r] -C /opt/goat|peering-standu[p].sh|/usr/local/bin/goa[t] federate" || echo none
'

# What a peering leaves on the hosts, with the two sovereign ids replaced by US and PEER,
# so that the killed side's print compares with the clean side's.
kr_shape() { # $1 us, $2 them
  local s="$1" them="$2"
  {
    docker exec "$HP-$s-edge" sh -c "
      echo '# files in the work directory'; find /var/lib/goat-federation/$them -type f | sed 's|^/var/lib/goat-federation/$them/||' | sort
      echo '# edge containers'; docker ps -a --format '{{.Names}} {{.State}}' | grep -v '^fed-probe-' | sort
      echo '# wireguard interfaces'; ip -o link show type wireguard | awk '{print \$2}' | sort
      echo '# nft tables'; nft list tables | sort
      echo '# peering units'; ls /etc/systemd/system | grep -E 'goat-peering|goat-zenoh-edge' | sort
      echo '# peering configuration'; ls /etc/goat-peering-wg /etc/goat-zenoh-edge-$them 2>&1 | sort" 2>&1
    docker exec "$HP-$s-core" sh -c "
      echo '# core containers'; docker ps -a --format '{{.Names}} {{.State}}' | sort
      echo '# republisher configuration'; ls /etc/goat-fed-republisher-$them 2>&1 | sort" 2>&1
  } | sed "s/$them/PEER/g; s/$s/US/g"
}

# One phase on the first side: started, killed partway, run again.
kr_phase() { # $1 us, $2 them, $3 phase
  local s="$1" them="$2" p="$3" out kf dpid wpid tw landed alive why delay rc=0
  out="$RECORD/kr-phase-$p-killed.out"; kf="$RECORD/kr-phase-$p-kill.txt"; delay="$(kr_delay "$p")"
  docker exec -i "$HP-$s-edge" bash -s -- "$delay" <<<"$KR_WATCH" > "$kf" 2>&1 &
  wpid=$!
  drive "$s" "$them" "$AVENUE" "$p" "$p" > "$out" 2>&1 &
  dpid=$!
  wait "$wpid" 2>/dev/null || echo "the watcher inside the edge exited non-zero" >> "$kf"
  # The docker exec client, as an operator's terminal going away.
  kill -9 "$dpid" 2>/dev/null || true
  wait "$dpid" 2>/dev/null || rc=$?
  tw="$(sed -n 's/^work //p' "$kf" | head -1)"; alive="$(sed -n 's/^alive //p' "$kf" | head -1)"
  landed="$(sed -n 's/^kill at //p' "$kf" | head -1)"
  kr_shape "$s" "$them" > "$RECORD/kr-phase-$p-after-kill.txt" || true
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$p" "${tw:-never}" "$delay" "${landed:-none}" "${alive:-0}" "$rc" >> "$RECORD/kills.tsv"
  if [ "$alive" = 1 ] && [ -n "$tw" ] && [ "$tw" != never ]; then
    why="killed at $landed after the start, $delay s after phase $p began work at ${tw}s"
  elif [ -z "$tw" ] || [ "$tw" = never ]; then
    why="the driver exited (code $rc) before phase $p showed any work; nothing was interrupted"
  else
    why="phase $p ended (code $rc) within $delay s of starting work at ${tw}s, before the kill; nothing was interrupted"
  fi
  rec "kr-phase-$p-killed-mid-phase" --note "$why" -- bash -c "echo '$why'; cat '$kf'; tail -20 '$out'; [ '$alive' = 1 ] && [ '${tw:-never}' != never ]"
  local f0="$FAILS"
  rec "kr-phase-$p-resumed" --note "after: $why" -- bash -c "drive '$s' '$them' '$AVENUE' $p $p"
  if [ "$FAILS" -gt "$f0" ]; then
    # The resume failed: that is the finding, and kr-phase-<P>-resumed stays FAIL. To learn
    # whether the damage was only transient (work the killed run started outside the
    # driver's process tree, such as apt-get under the edge's own sshd, still running) and
    # to measure the phases after this one, wait until no such work is left on either host
    # and run the phase once more.
    rec "kr-phase-$p-leftover-work-ended" -- bash -c \
      "for _ in \$(seq 1 90); do l=\$(for h in $HP-$s-edge $HP-$s-core; do docker exec \$h pgrep -a -f 'apt-ge[t]|dpk[g]|ansible-playboo[k]|AnsiballZ_' 2>/dev/null; done); [ -z \"\$l\" ] && { echo none left; exit 0; }; echo \"\$l\" | tail -3; sleep 2; done; exit 1"
    rec "kr-phase-$p-second-rerun" -- bash -c "drive '$s' '$them' '$AVENUE' $p $p"
  fi
}

kr_phases() { # $1 us, $2 them, $3 from, $4 to
  local p
  for p in $(seq "$3" "$4"); do kr_phase "$1" "$2" "$p"; done
}

unit_main() {
  local first second s them i
  first="$(kr_first)"; second="$(other "$first")"
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  printf 'phase\twork_seen_s\tdelay_s\tkill_at_s\tdriver_alive\tkilled_exit\n' > "$RECORD/kills.tsv"
  setup_hosts kr
  i="$(iface "$first" "$second")"
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false

  say "kill-resume: $first phases 1-5, each killed and resumed; $second straight through ($AVENUE)"
  kr_phases "$first" "$second" 1 5
  rec "kr-$second-phases-1-to-5" --expect-in "exchange through the directory" -- bash -c "drive '$second' '$first' '$AVENUE' 1 5"
  exchange "$first" "$second" "$AVENUE" kr

  say "kill-resume: phases 6-10"
  kr_phases "$first" "$second" 6 10
  rec "kr-$second-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$second' '$first' '$AVENUE' 6 10"
  rec "kr-one-agreement-digest" -- bash -c \
    "a=\$(docker exec $HP-$first-edge sha256sum /var/lib/goat-federation/$second/agreement.json | cut -d' ' -f1); b=\$(docker exec $HP-$second-edge sha256sum /var/lib/goat-federation/$first/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  review_and_admit "$first" "$second"
  review_and_admit "$second" "$first"

  say "kill-resume: phases 11-12"
  kr_phases "$first" "$second" 11 12
  rec "kr-$second-phases-11-to-12" --expect-in "goat-fed-republisher-$first is running" -- bash -c "drive '$second' '$first' '$AVENUE' 11 12"

  say "kill-resume: phase 13"
  kr_phases "$first" "$second" 13 13
  rec "kr-$second-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "drive '$second' '$first' '$AVENUE' 13 13"

  rec "kr-all-phases-resumed" -- python3 -c '
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
st = {r["name"]: r["status"] for r in rows}
bad = [p for p in range(1, 14) if st.get("kr-phase-%d-resumed" % p) != "pass"]
for p in range(1, 14): print("phase %d: %s" % (p, st.get("kr-phase-%d-resumed" % p, "not run")))
sys.exit(1 if bad else 0)' "$RECORD/report.jsonl"

  say "kill-resume: the end state, against the clean side"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    kr_shape "$s" "$them" > "$RECORD/kr-$s-shape.txt" || true
  done
  rec "kr-same-shape-as-the-clean-side" -- diff "$RECORD/kr-$second-shape.txt" "$RECORD/kr-$first-shape.txt"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "kr-$s-wireguard-key-is-the-one-the-partner-holds" -- bash -c \
      "k=\$(docker exec $HP-$s-edge wg show $(iface "$s" "$them") public-key); p=\$(docker exec $HP-$them-edge wg show $(iface "$them" "$s") peers); echo \"ours=\$k partner-holds=\$p\"; [ -n \"\$k\" ] && [ \"\$k\" = \"\$p\" ]"
    rec "kr-$s-peering-keys-match-their-certificates" -- docker exec "$HP-$s-edge" sh -c "
      n=0; bad=0
      for k in /var/lib/goat-federation/$them/pki/*-key.pem; do
        c=\"\${k%-key.pem}.pem\"; n=\$((n + 1))
        a=\$(openssl pkey -in \"\$k\" -pubout 2>/dev/null | sha256sum | cut -c1-16)
        b=\$(openssl x509 -in \"\$c\" -noout -pubkey 2>/dev/null | sha256sum | cut -c1-16)
        echo \"\$k \$a \$c \$b\"; [ -s \"\$c\" ] && [ \"\$a\" = \"\$b\" ] || bad=\$((bad + 1))
      done
      [ \$n -gt 0 ] && [ \$bad = 0 ]"
  done
  rec "kr-$first-duplicated-nothing" -- bash -c \
    "docker exec $HP-$first-edge sh -c 'docker ps -a --format {{.Names}} | sort | uniq -d; ip -o link | grep -c wg-fed' | tee /dev/stderr | tail -1 | grep -qx 1 && docker exec $HP-$first-core sh -c 'docker ps -a --format {{.Names}} | grep -c goat-fed-republisher-' | grep -qx 1"
  rec "kr-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2; [ \"\${t:-0}\" != 0 ]"

  say "kill-resume: phase 14 from both sides, strict"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
    start_probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$(probe_port "$s" "$them" 8101)"
    start_probe "$s" "$them" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem "$(probe_port "$s" "$them" 8102)"
    start_probe "$s" "$them" impersonate "/etc/fed-e2e/impersonate-$them.pem" "/etc/fed-e2e/impersonate-$them-key.pem" "$(probe_port "$s" "$them" 8103)"
  done
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "kr-acceptance-as-$s-with-$them" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "accept_as '$s' '$them' '$AVENUE'"
  done
  collect kr
}

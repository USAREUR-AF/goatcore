#!/usr/bin/env bash
# units/onecmd.sh — two lab sovereigns, each driven by `goatbuilder federate <partner>` alone,
# every switch in its released position: no --from, --to, --as, --yes or --confirm. The
# command runs on a terminal (lib/answer.py), so it asks its confirmations there, and the
# operator answers from what the partner reads aloud on the call: the partner's verifying
# key, its CA fingerprint and its agreement digest, each read from the partner's own files,
# never from what arrived over the wire. Each operator runs the same command again when it
# stops (the partner was not ready, or a person had to answer), as the command says.
# Sourced by lib/unit.sh. Scenario: goatbuilder-only-both-sides (scenarios/catalog.yaml).
#
# What the operators exchange is what the product says they exchange: the packages through
# the peering directory, the keys and the digest by voice. Anything else the harness has to
# do for the run to go on is written to RECORD/outside.txt, one line naming the phase, what
# the command did instead and what the harness did, and check oc-nothing-outside-the-command
# fails with every line as its reason. The drill's own infrastructure (the hosts, the
# directory, the responders a drill starts, phase 14's probe launcher) is not such a step:
# federation-acceptance.md §2 and §2.4 name it the drill's.
#
# Records: RECORD/oc-<side>-<run>.log (each run's terminal transcript), RECORD/typed-<side>.jsonl
# (each question, exactly what was typed, and why), RECORD/oc-runs.txt (each run: exit status,
# the phases passed after it and the command's own `next`), RECORD/status-<side>.json.

OC_PREFIX="${OC_PREFIX:-oc}"
OUTSIDE="$RECORD/outside.txt"

# The operator's environment for `goatbuilder federate` on the driver host, as -e words in
# OC_ENV. drive()'s environment (lib/engine.sh), and the deployment's settings for the two
# steps the command now does itself: phase 10's admission on our core (admit.rs: where the
# core's org fragment, seed and assembled configuration live, our core certificate, and the
# command that places them and restarts the router) and phase 14 on this side's own surfaces.
oc_env() { # $1 us, $2 them
  local s="$1" them="$2"
  OC_ENV=(
    -e "GOAT_CONTEXT=$(ctx_of "$s" "$them")"
    -e "FED_WORK=/var/lib/goat-federation/$them"
    -e "FED_SIGNING_KEY=/etc/fed-e2e/signing.key"
    -e "FED_CA_CERT=/etc/fed-e2e/ca.pem" -e "FED_CA_KEY=/etc/fed-e2e/ca-key.pem"
    -e "FED_CORE_CN=core-$s" -e "FED_CORE_SAN=$(core_ip "$s")"
    -e "FED_ROUTER_DOCKER_HOST=ssh://goat@$(core_ip "$s")"
    -e GOAT_AWS_MODE=none -e GOAT_BIN=/usr/local/bin/goat -e HOME=/root -e GOAT_ANSIBLE_TRUNK_REMOTE=origin
    -e "FED_COUNTERPARTY_ID=$them" -e SSL_CERT_FILE=/etc/fed-e2e/directory-ca.pem
    -e "FED_DIRECTORY_URL=https://$DIRECTORY_IP:8443"
    -e FED_SLOT_CHECK=skip
    -e "FED_OURS_REST=http://$(core_ip "$s"):8000"
    -e "FED_EDGE_LOG_OURS=docker logs goat-zenoh-edge-$them"
    -e "FED_SPIRE_OURS=ssh goat@$(core_ip "$s") docker exec spire-server /opt/spire/bin/spire-server"
  )
  # Phase 14's own surfaces: a tree that derives them from the core and edge declarations
  # (acceptance_surfaces.py) runs without them unless FE2E_ACC_VALUES=set; a unit may give
  # one back once a run says it was not derived (OC_ACC_ADDED, units/nohand.sh).
  if [ "${FE2E_ACC_VALUES:-omit}" != set ] && [ -f "$WORK/$s/clone/ops/federation/standup/acceptance_surfaces.py" ]; then
    local keep=() j=0 m=${#OC_ENV[@]} nm
    while [ "$j" -lt "$m" ]; do
      if [ "${OC_ENV[$j]}" = -e ]; then
        nm="${OC_ENV[$((j + 1))]%%=*}"
        case "$nm" in FED_OURS_REST|FED_EDGE_LOG_OURS|FED_SPIRE_OURS)
          case " ${OC_ACC_ADDED:-} " in *" $nm "*) ;; *) j=$((j + 2)); continue ;; esac ;;
        esac
      fi
      keep+=("${OC_ENV[$j]}"); j=$((j + 1))
    done
    OC_ENV=("${keep[@]}")
  fi
  if [ "$FE2E_CORE" = role ]; then
    # The core deployed by the goat-zenoh role: the router leaf where the role reads it
    # (the controller's state directory), the org fragment as a copy on the driver host that
    # fe2e-core-role carries to the role's path on the core host before it runs the role's
    # play (zenoh-deploy.yml) to dry-run or to deploy. The role assembles the configuration
    # itself, so no seed and no assembled output are named.
    OC_ENV+=(
      -e "FED_CORE_ACL_ORG=/var/lib/goat-core-admission/acl-org-fragment.json5"
      -e "FED_CORE_CERT=/root/.goat/state/$s-leaves/router-cert.pem" -e "FED_CORE_KEY=/root/.goat/state/$s-leaves/router-key.pem"
      -e "FED_CORE_RESTART=/usr/local/sbin/fe2e-core-role deploy" -e "FED_CORE_DRY_RUN=/usr/local/sbin/fe2e-core-role check"
    )
  else
    OC_ENV+=(
      -e FED_CORE_CONFIG_PATH=/etc/goat-core/config.json5
      -e FED_CORE_ACL_ORG=/srv/goat-core/acl-org-fragment.json5
      -e FED_CORE_ACL_SEED=/srv/goat-core/seed.json5 -e FED_CORE_ACL_OUT=/srv/goat-core/config.json5
      -e FED_CORE_CERT=/srv/goat-core/core.pem -e FED_CORE_KEY=/srv/goat-core/core-key.pem
      -e FED_CORE_RESTART=/srv/goat-core/restart -e FED_CORE_DRY_RUN=/srv/goat-core/dry-run
    )
  fi
  # A unit may add or drop words (units/nohand.sh starts from less).
  if declare -F oc_env_more >/dev/null; then oc_env_more "$s" "$them"; fi
}

# The hand-run core's configuration as its operator keeps it on the driver host, where
# phase 10's admission stages and places what the core loads: the seed, the org fragment
# (empty: this core admits no partner yet), the assembled configuration and the core's
# certificate; `restart` carries them to the core host and restarts the router.
oc_core_staging() { # $1 sovereign
  local s="$1" h; h="$(drv_host "$1")"
  if [ "$FE2E_CORE" = role ]; then oc_core_staging_role "$s"; return 0; fi
  host_exec "$h" install -d -m 0700 /srv/goat-core
  docker cp -q "$WORK/$s/core-seed.json5" "$h:/srv/goat-core/seed.json5"
  docker cp -q "$WORK/$s/core.json5" "$h:/srv/goat-core/config.json5"
  docker cp -q "$WORK/$s/keys/core.pem" "$h:/srv/goat-core/core.pem"
  docker cp -q "$WORK/$s/keys/core-key.pem" "$h:/srv/goat-core/core-key.pem"
  host_exec "$h" sh -c ': > /srv/goat-core/acl-org-fragment.json5'
  host_exec "$h" sh -c "cat > /srv/goat-core/restart" <<SH
#!/bin/sh
# Carry the core's configuration and certificate to the core host and restart the router.
set -e
tar -C /srv/goat-core -cf - config.json5 core.pem core-key.pem \\
  | ssh goat@$(core_ip "$s") "sudo tar -C /etc/goat-core -xf - && sudo chmod 0644 /etc/goat-core/core-key.pem && docker restart $s-zenoh-router >/dev/null"
SH
  host_exec "$h" sh -c "cat > /srv/goat-core/dry-run" <<'SH'
#!/bin/sh
# The assembled configuration is there and whole JSON5 (the router reads nothing else).
set -e
test -s /srv/goat-core/config.json5
grep -q '"*access_control"*' /srv/goat-core/config.json5
SH
  host_exec "$h" chmod 0755 /srv/goat-core/restart /srv/goat-core/dry-run
}

# The role-deployed core (FE2E_CORE=role): the org fragment the role keeps on the core host
# (goat_zenoh_acl_org_path, /etc/<site>-zenoh/acl-org-fragment.json5), copied to the driver
# host for admission to merge into, and fe2e-core-role, which carries it back and runs the
# role's play from the sovereign's checkout as core_deploy (lib/engine.sh) does: with
# --check --diff for the dry run, without for the restart.
oc_core_staging_role() { # $1 sovereign
  local s="$1" h; h="$(drv_host "$1")"
  host_exec "$h" install -d -m 0700 /var/lib/goat-core-admission
  host_exec "$h" sh -c "ssh goat@$(core_ip "$s") 'sudo cat /etc/$s-zenoh/acl-org-fragment.json5' > /var/lib/goat-core-admission/acl-org-fragment.json5"
  host_exec "$h" sh -c "cat > /usr/local/sbin/fe2e-core-role" <<SH
#!/bin/sh
# fe2e-core-role check|deploy: place the org fragment where the goat-zenoh role reads it on
# the core host, then run the role's play against the core (a dry run, or the deploy).
set -e
ssh goat@$(core_ip "$s") "sudo tee /etc/$s-zenoh/acl-org-fragment.json5 >/dev/null" < /var/lib/goat-core-admission/acl-org-fragment.json5
extra=""; [ "\$1" = check ] && extra="--check --diff"
cd /opt/goat/ops/ansible
GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin HOME=/root ../scripts/with-aws none -- ../scripts/ansible-preflight.sh \
  -i inventory/core-router.ini -e target=core_router -e site_id=$s -e goat_zenoh_rest_host=0.0.0.0 \$extra zenoh-deploy.yml
SH
  host_exec "$h" chmod 0755 /usr/local/sbin/fe2e-core-role
}

# The role core's admission of the partner in the role's own declarations: the peering in
# group_vars/core_router (zenoh_sovereign_peerings renders its landing store; the subject's
# CN is our edge's inward leaf, fed.<them>.<us>.01), merged to the sovereign's trunk.
oc_core_declare_role() { # $1 us, $2 them
  local s="$1" them="$2" f="$WORK/$1/core-peerings.yml"
  printf '%s\n' "# Each admitted peering (the goat-zenoh role's core admission)." "zenoh_sovereign_peerings:" \
    "  - sovereign: $them" "    cn: fed.$them.$s.01" > "$f"
  core_declare "$s" ops/ansible/group_vars/core_router/peerings.yml "$f" "$s: admit the peering with $them on the core router"
}

# One run of the command on the driver host, on a terminal, answering its questions.
oc_run() { # $1 us, $2 them, $3 run label, then `goatbuilder federate` arguments
  local s="$1" them="$2" label="$3"; shift 3
  oc_env "$s" "$them"
  python3 "$HERE/lib/answer.py" --label "$label" --transcript "$RECORD/$OC_PREFIX-$s-$label.log" --typed "$RECORD/typed-$s.jsonl" \
    --value "partner_key=cat '$WORK/$them/keys/signing.pub'" \
    --value "partner_ca_fp=openssl x509 -in '$WORK/$them/keys/ca.pem' -outform DER | sha256sum | cut -d' ' -f1" \
    --value "partner_agreement=docker exec $(drv_host "$them") sha256sum /var/lib/goat-federation/$s/agreement.json | cut -d' ' -f1" \
    --value "partner_package=docker exec $(drv_host "$them") sha256sum /var/lib/goat-federation/$s/ours.canonical.json | cut -d' ' -f1" \
    -- docker exec -it "${OC_ENV[@]}" -w /opt/goat "$(drv_host "$s")" goatbuilder -C /opt/goat federate "$@" > /dev/null 2>&1
}

# The command's own record of the peering: `goatbuilder federate status <them> -o json`.
oc_status() { # $1 us, $2 them -> RECORD/status-<us>.json
  local s="$1" them="$2"
  oc_env "$s" "$them"
  docker exec "${OC_ENV[@]}" -w /opt/goat "$(drv_host "$s")" goatbuilder -C /opt/goat -o json federate status "$them" > "$RECORD/status-$s.json" 2>"$RECORD/status-$s.err" || true
}

# From a status document: "<passed phases> | <next>", or "unreadable".
oc_summary() { # $1 status file
  python3 - "$1" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    p = d["peerings"][0]
except Exception:
    print("unreadable"); sys.exit(0)
done = sorted({x["phase"] for x in p.get("phases", []) if x.get("outcome") == "passed"})
last = p.get("last_run") or {}
print(" ".join(str(n) for n in done) + " | last run " + str(last.get("outcome", "none")) + " | next: " + (p.get("next") or ""))
PY
}
oc_passed() { # $1 status file, $2..: phases; true when every one is recorded passed
  python3 - "$@" <<'PY'
import json, sys
try:
    p = json.load(open(sys.argv[1]))["peerings"][0]
except Exception:
    sys.exit(1)
done = {x["phase"] for x in p.get("phases", []) if x.get("outcome") == "passed"}
sys.exit(0 if all(int(n) in done for n in sys.argv[2:]) else 1)
PY
}
oc_outside() { grep -qxF -- "$*" "$OUTSIDE" 2>/dev/null || printf '%s\n' "$*" >> "$OUTSIDE"; }
# `--as <us>` for a side whose run took the wrong party (OC_AS, set by oc_drive), else nothing.
OC_AS=" "
oc_as() { case "$OC_AS" in *" $1 "*) echo "--as $1" ;; esac; }

# Phase 10 when the one command stopped for it: the route the product offers instead, then
# the hand admission when that does not admit either. Each is a step outside the command.
oc_admit_another_way() { # $1 us, $2 them, $3 round
  local s="$1" them="$2" r="$3" next
  next="$(oc_summary "$RECORD/status-$s.json" | sed 's/.*| next: //')"
  oc_outside "phase 10 ($s): \`goatbuilder federate $them\` stopped before admitting $them's edge on $s's core (its next: $next); ran \`goatbuilder federate plan $them\` and \`goatbuilder federate deploy $them\` instead (W10)"
  oc_run "$s" "$them" "r$r-plan" plan "$them" || true
  # shellcheck disable=SC2046
  oc_run "$s" "$them" "r$r-deploy" deploy "$them" $(oc_as "$s") || true
  oc_status "$s" "$them"
  echo "round $r $s deploy route: $(oc_summary "$RECORD/status-$s.json")" >> "$RECORD/oc-runs.txt"
  oc_passed "$RECORD/status-$s.json" 10 && return 0
  # A deploy that named a value the unit withheld (units/nohand.sh) is tried again on the
  # next round with it supplied, before anything is admitted by hand.
  if declare -F oc_after_run >/dev/null && oc_after_run "$s" "$them" "$RECORD/$OC_PREFIX-$s-r$r-deploy.log"; then return 1; fi
  oc_outside "phase 10 ($s): \`goatbuilder federate deploy $them\` did not admit either; the harness admitted by hand (review_and_admit: their CA checked and placed, the fragment spliced with acl-assemble.py, the core restarted on phase 8's certificate) and ran \`goatbuilder federate $them --from 11\` (W10)"
  review_and_admit "$s" "$them" "$OC_PREFIX"
  # shellcheck disable=SC2046
  oc_run "$s" "$them" "r$r-from-11" "$them" --from 11 $(oc_as "$s") || true
}

# Both operators run the one command until each has passed every phase, or a round moves
# neither of them. $1 first side, $2 second side.
oc_drive() {
  local a="$1" b="$2" r s them before after moved a_done=0 b_done=0 rc admitted=" "
  for r in 1 2 3 4 5 6 7 8 9 10; do
    moved=0
    for s in "$a" "$b"; do
      them="$(other "$s")"
      if [ "$s" = "$a" ] && [ "$a_done" = 1 ]; then continue; fi
      if [ "$s" = "$b" ] && [ "$b_done" = 1 ]; then continue; fi
      oc_status "$s" "$them"; before="$(oc_summary "$RECORD/status-$s.json")"
      rc=0
      # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
      oc_run "$s" "$them" "r$r" "$them" $(oc_as "$s") || rc=$?
      oc_status "$s" "$them"; after="$(oc_summary "$RECORD/status-$s.json")"
      echo "round $r $s: exit $rc; phases passed: $after" >> "$RECORD/oc-runs.txt"
      [ "$before" = "$after" ] || moved=1
      # A unit may supply, after a run, what the run named as missing (units/nohand.sh).
      if declare -F oc_after_run >/dev/null && oc_after_run "$s" "$them" "$RECORD/$OC_PREFIX-$s-r$r.log"; then moved=1; fi
      if oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9 10 11 12 13; then
        if [ "$s" = "$a" ]; then a_done=1; else b_done=1; fi
      fi
    done
    [ "$a_done" = 1 ] && [ "$b_done" = 1 ] && break
    if [ "$moved" = 0 ]; then
      # Neither moved. A side held at phase 10 is admitted another way, once, so that the
      # later phases are measured too; otherwise the run ends here.
      for s in "$a" "$b"; do
        them="$(other "$s")"
        case "$admitted" in *" $s "*) continue ;; esac
        if oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9 && ! oc_passed "$RECORD/status-$s.json" 10; then
          if oc_admit_another_way "$s" "$them" "$r"; then admitted="$admitted$s "; fi; moved=1
        fi
        # A run that took the partner for us (mint-peering-leaves.sh: "agreement names us
        # '<them>'") is given --as from then on, once, so that the later phases are measured.
        case "$OC_AS" in *" $s "*) ;; *)
          if grep -q "agreement names us '$them'" "$RECORD/$OC_PREFIX-$s-r$r.log" 2>/dev/null; then
            oc_outside "phase 8 ($s): \`goatbuilder federate $them\` minted as $them (mint-peering-leaves.sh: \"agreement names us '$them'\", then \"own CA fingerprint differs from the agreement\"): the plan passes --as to phases 7-9 only when given; the harness added --as $s to every later run (W10, federate/mod.rs)"
            OC_AS="$OC_AS$s "; moved=1
          fi ;;
        esac
      done
      [ "$moved" = 1 ] || break
    fi
  done
  # Phase 14 on one side needs the other side's edge and republisher up. A side that got
  # through phase 13 before its partner did runs the same command once more now, so its
  # phase 14 in the command is measured with both sides deployed.
  for s in "$a" "$b"; do
    them="$(other "$s")"
    oc_passed "$RECORD/status-$s.json" 13 && oc_passed "$RECORD/status-$them.json" 13 || continue
    rc=0
    # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
    oc_run "$s" "$them" "final" "$them" $(oc_as "$s") || rc=$?
    oc_status "$s" "$them"
    if declare -F oc_after_run >/dev/null && oc_after_run "$s" "$them" "$RECORD/$OC_PREFIX-$s-final.log"; then
      rc=0
      # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
      oc_run "$s" "$them" "final-2" "$them" $(oc_as "$s") || rc=$?
      oc_status "$s" "$them"
    fi
    echo "final $s: exit $rc; phases passed: $(oc_summary "$RECORD/status-$s.json")" >> "$RECORD/oc-runs.txt"
  done
}

# Strict acceptance from one side, where an operator runs it: on the driver host, through
# `goatbuilder federate test <them>` (phase 14 alone), in the peering's own working
# directory (/var/lib/goat-federation/<them>, the allocated cert_dir), with every surface
# configured and the probes started by the run itself through the drill's launcher.
# Strict acceptance also reads the partner's core, SPIRE server and edge: only the drill
# party, which owns both sides, can give it that, so for the length of the run the driver
# host joins the partner's internal network and our operator key is authorized on the
# partner's core and edge. Both are taken away again after the run.
oc_accept() { # $1 us, $2 them
  local s="$1" them="$2" h pl far rc=0 stage
  h="$(drv_host "$s")"; pl="$WORK/$s/launcher-$them"
  mkdir -p "$pl"
  # The partner's identities for the probes: its agreed leaf, a name nobody agreed, a leaf
  # naming our core; and our CA, which the partner's probe trusts our edge by.
  docker cp -q "$(drv_host "$them"):/var/lib/goat-federation/$s/pki/peering-outward.pem" "$pl/agreed-cert.pem"
  docker cp -q "$(drv_host "$them"):/var/lib/goat-federation/$s/pki/peering-outward-key.pem" "$pl/agreed-key.pem"
  cp "$WORK/$them/keys/forged.pem" "$pl/forged-cert.pem"; cp "$WORK/$them/keys/forged-key.pem" "$pl/forged-key.pem"
  cp "$WORK/$them/keys/impersonate-$s.pem" "$pl/impersonate-cert.pem"; cp "$WORK/$them/keys/impersonate-$s-key.pem" "$pl/impersonate-key.pem"
  cp "$WORK/$s/keys/ca.pem" "$pl/far-ca.pem"; chmod 0644 "$pl"/*
  docker cp -q "$(drv_host "$s"):/var/lib/goat-federation/$them/agreement.json" "$pl/agreement.json"
  far="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$pl/agreement.json" "$s")"
  host_exec "$h" install -d -m 0700 /root/fe2e-launcher
  for f in "$pl"/*-cert.pem "$pl"/*-key.pem "$pl/far-ca.pem"; do docker cp -q "$f" "$h:/root/fe2e-launcher/"; done
  docker cp -q "$HERE/lib/probe-launcher.sh" "$h:/usr/local/sbin/fe2e-probe-launcher"
  docker cp -q "$HERE/probe.json5" "$h:/root/fe2e-launcher/probe.json5"
  host_exec "$h" chmod 0755 /usr/local/sbin/fe2e-probe-launcher
  # The drill's reach into the partner's side, for this run only.
  docker network connect "$NP-$them" "$h" >/dev/null 2>&1 || true
  local pub; pub="$(cat "$WORK/$s/keys/id_ed25519.pub")"
  for t in "$HP-$them-core" "$HP-$them-edge"; do
    docker exec "$t" sh -c "printf '%s\n' '$pub' >> /home/goat/.ssh/authorized_keys"
  done
  stage=(test "$them"); "$GB" federate --help 2>/dev/null | grep -q 'test <partner>' || stage=("$them" --from 14 --to 14)
  oc_env "$s" "$them"
  # shellcheck disable=SC2046 # oc_as gives nothing or two words (--as <us>)
  docker exec "${OC_ENV[@]}" -e FED_ACCEPTANCE_STRICT=1 \
    -e "FED_THEIRS_REST=http://127.0.0.1:8000" -e "FED_SSH_THEIRS=ssh://goat@$(core_ip "$them")" \
    -e "FED_SPIRE_THEIRS=ssh goat@$(core_ip "$them") docker exec spire-server /opt/spire/bin/spire-server" \
    -e FED_PROBE_LAUNCHER=/usr/local/sbin/fe2e-probe-launcher \
    -e "FE2E_PL_SSH=goat@$(edge_ip "$them")" -e "FE2E_PL_REST_IP=$(edge_ip "$them")" -e "FE2E_PL_FAR_IP=$far" \
    -e FE2E_PL_DIR=/root/fe2e-launcher -e FE2E_PL_PORT=8201 -e FE2E_PL_TEMPLATE=/root/fe2e-launcher/probe.json5 -e "FE2E_PL_IMAGE=$ZENOH_IMAGE" \
    -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate "${stage[@]}" $(oc_as "$s") || rc=$?
  for t in "$HP-$them-core" "$HP-$them-edge"; do
    docker exec "$t" sh -c "grep -vxF '$pub' /home/goat/.ssh/authorized_keys > /home/goat/.ssh/ak.tmp; cat /home/goat/.ssh/ak.tmp > /home/goat/.ssh/authorized_keys; rm -f /home/goat/.ssh/ak.tmp" || true
  done
  docker network disconnect "$NP-$them" "$h" >/dev/null 2>&1 || true
  return "$rc"
}

# The check ids each side's steps answer in the catalog: OC_SCENARIO, and OC_ID_<side>
# (the id prefix for that side's checks; units/nohand.sh names them differently).
OC_SCENARIO="${OC_SCENARIO:-goatbuilder-only-both-sides}"
oc_id() { # $1 side, $2 check name -> <scenario>.<id>
  local v; if [ "$1" = "$A" ]; then v="${OC_ID_A-first-}"; else v="${OC_ID_B-second-}"; fi
  echo "$OC_SCENARIO.$v$2"
}
# A check the command never reached is BLOCKED, naming where the run stopped: the earlier
# FAIL is the finding, not this one.
oc_stopped_at() { # $1 side -> where its last status says the command stopped
  python3 - "$RECORD/status-$1.json" <<'PY'
import json, sys
try:
    p = json.load(open(sys.argv[1]))["peerings"][0]
except Exception:
    print("no status could be read"); sys.exit(0)
done = {x["phase"] for x in p.get("phases", []) if x.get("outcome") == "passed"}
first = next((n for n in range(1, 15) if n not in done), None)
det = next((x.get("detail", "") for x in p.get("phases", []) if x["phase"] == first), "")
print(f"the command stopped at phase {first}: {det[:240]} (its next: {p.get('next', '')})")
PY
}

oc_checks() { # $1 first, $2 second
  local a="$1" b="$2" s them p blocked name
  for s in "$a" "$b"; do
    them="$(other "$s")"; blocked=""
    for p in "1-to-5:phases-1-to-5:1 2 3 4 5" "6-to-9:phases-6-to-9:6 7 8 9" "10:phase-10-admission:10" "11-to-12:phases-11-to-12:11 12" "13:phase-13:13" "14:phase-14-in-the-command:14"; do
      name="${p#*:}"; name="${name%%:*}"
      if [ -n "$blocked" ]; then mark "$(oc_id "$s" "$name")" BLOCKED "$blocked"; continue; fi
      rec "$OC_PREFIX-$s-$them-phases-${p%%:*}" -- bash -c \
        "python3 -c 'import json,sys; p=json.load(open(sys.argv[1]))[\"peerings\"][0]; [print(x[\"phase\"], x[\"outcome\"], x.get(\"run\",\"\"), x.get(\"detail\",\"\")[:300]) for x in p.get(\"phases\",[])]; print(\"next:\", p.get(\"next\"))' '$RECORD/status-$s.json'; oc_passed '$RECORD/status-$s.json' ${p##*:}"
      oc_passed "$RECORD/status-$s.json" ${p##*:} || blocked="$(oc_stopped_at "$s")"
    done
    if oc_passed "$RECORD/status-$s.json" 6; then
      rec "$OC_PREFIX-$s-$them-confirmed-at-the-terminal" -- bash -c \
        "cat '$RECORD/typed-$s.jsonl'; python3 -c 'import json,sys
rows=[json.loads(l) for l in open(sys.argv[1])]
bad=[r for r in rows if str(r.get(\"why\",\"\")).startswith(\"unrecognized\")]
have=lambda w: any(w in (r.get(\"question\") or \"\") and r.get(\"typed\") for r in rows)
need=[\"CA fingerprint aloud\", \"verifying key\", \"Is theirs the same\"]
miss=[w for w in need if not have(w)]
print(\"unrecognized:\", len(bad), \"not confirmed:\", miss)
sys.exit(1 if bad or miss else 0)' '$RECORD/typed-$s.jsonl'"
    else
      mark "$(oc_id "$s" confirmed-at-the-terminal)" BLOCKED "the agreement digest is asked after phase 6, which did not pass: $(oc_stopped_at "$s")"
    fi
  done
  if oc_passed "$RECORD/status-$a.json" 6 && oc_passed "$RECORD/status-$b.json" 6; then
    rec "$OC_PREFIX-$a-$b-one-agreement-digest" -- bash -c \
      "x=\$(docker exec $(drv_host "$a") sha256sum /var/lib/goat-federation/$b/agreement.json | cut -d' ' -f1); y=\$(docker exec $(drv_host "$b") sha256sum /var/lib/goat-federation/$a/agreement.json | cut -d' ' -f1); echo \"\$x \$y\"; [ -n \"\$x\" ] && [ \"\$x\" = \"\$y\" ]"
  else
    mark "$OC_SCENARIO.one-agreement" BLOCKED "phase 6 did not pass on both sides: $a: $(oc_stopped_at "$a"); $b: $(oc_stopped_at "$b")"
  fi
  if oc_passed "$RECORD/status-$a.json" 11 && oc_passed "$RECORD/status-$b.json" 11; then
    rec "$OC_PREFIX-$a-$b-a-wireguard-handshake" -- bash -c \
      "for _ in \$(seq 1 30); do t=\$(docker exec $HP-$a-edge wg show all latest-handshakes | awk '\$3 > 0 {print \$3}' | head -1); [ -n \"\$t\" ] && break; sleep 2; done; docker exec $HP-$a-edge wg show all latest-handshakes; [ -n \"\$t\" ]"
  else
    mark "$OC_SCENARIO.handshake" BLOCKED "phase 11 (the transport) did not pass on both sides: $a: $(oc_stopped_at "$a"); $b: $(oc_stopped_at "$b")"
  fi
}

oc_acceptance_checks() { # $1 first, $2 second
  local a="$1" b="$2" s them id
  for s in "$a" "$b"; do
    them="$(other "$s")"
    if [ "$OC_SCENARIO" = goatbuilder-only-both-sides ]; then
      if [ "$s" = "$a" ]; then id="$OC_SCENARIO.acceptance-first"; else id="$OC_SCENARIO.acceptance-second"; fi
    else
      [ "$s" = "$A" ] || continue
      id="$OC_SCENARIO.acceptance"
    fi
    if ! oc_passed "$RECORD/status-$s.json" 13 || ! oc_passed "$RECORD/status-$them.json" 13; then
      mark "$id" BLOCKED "phase 13 did not pass on both sides: $s: $(oc_stopped_at "$s"); $them: $(oc_stopped_at "$them")"
      continue
    fi
    # Run here (it needs this shell's settings), recorded from its output and exit status.
    local out="$RECORD/acceptance-as-$s.out" arc=0
    oc_accept "$s" "$them" > "$out" 2>&1 || arc=$?
    # A unit may supply what acceptance named as missing, redeploy by the command's own
    # route, and have acceptance run once more (units/nohand.sh); its result is the record.
    if [ "$arc" != 0 ] && declare -F oc_accept_retry >/dev/null && oc_accept_retry "$s" "$them" "$out"; then
      mv "$out" "$out.first"; arc=0
      oc_accept "$s" "$them" > "$out" 2>&1 || arc=$?
      { echo "--- acceptance before the retry (full output in $(basename "$out").first):"; grep -E 'acceptance-counts|ACCEPTANCE|trace verdict' "$out.first" | head -5; } > "$out.retry-note"
    fi
    rec "$OC_PREFIX-acceptance-as-$s-with-$them" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" \
      -- bash -c "cat '$out'; exit $arc"
  done
}

# One check per step the harness had to do instead of the command, so that each defect
# reads FAIL on its own: the plain run admits on our core (phase 10), deploy carries on past
# the admission it made (phase 11), and phase 8 mints as us without --as.
oc_defect_checks() { # $1 first, $2 second
  local s them c
  for s in "$1" "$2"; do
    them="$(other "$s")"
    if ! oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9; then
      for c in plain-run-admits deploy-carries-past-its-admission; do mark "$(oc_id "$s" "$c")" BLOCKED "phase 10 was never reached: $(oc_stopped_at "$s")"; done
    else
    rec "$OC_PREFIX-$s-$them-plain-run-admits" -- bash -c \
      "if grep -F 'phase 10 ($s): \`goatbuilder federate $them\` stopped' '$OUTSIDE'; then exit 1; fi; echo 'the plain run did not stop for the admission'"
    if grep -qF "phase 10 ($s): \`goatbuilder federate $them\` stopped" "$OUTSIDE"; then
      rec "$OC_PREFIX-$s-$them-deploy-carries-past-its-admission" --expect-not-in "what the plan was made from has changed" -- \
        bash -c "cat '$RECORD/$OC_PREFIX-$s-'r*-deploy.log 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'Phase 1[0-3] |✗|federate: '"
    else
      mark "$(oc_id "$s" deploy-carries-past-its-admission)" "NOT APPLICABLE" "the plain run admitted by itself, so deploy was not run"
    fi
    fi
    if ! oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7; then
      mark "$(oc_id "$s" mints-as-us-without-as)" BLOCKED "phase 8 was never reached: $(oc_stopped_at "$s")"; continue
    fi
    rec "$OC_PREFIX-$s-$them-mints-as-us-without-as" -- bash -c \
      "if grep -F 'phase 8 ($s)' '$OUTSIDE'; then exit 1; fi; echo 'phase 8 minted as $s with no --as'"
  done
}

oc_outside_check() {
  [ -s "$OUTSIDE" ] || : > "$OUTSIDE"
  local args=() l
  while IFS= read -r l; do [ -n "$l" ] && args+=(--expect-not-in "$l"); done < "$OUTSIDE"
  rec "$OC_PREFIX-nothing-outside-the-command" ${args[@]+"${args[@]}"} -- bash -c "echo 'steps done outside \`goatbuilder federate <partner>\`:'; cat '$OUTSIDE'; [ -s '$OUTSIDE' ] || echo none"
}

unit_main() {
  local s them
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  export -f oc_passed
  export OC_PREFIX HP ZENOH_IMAGE GB
  : > "$OUTSIDE"
  say "goatbuilder-only-both-sides: $A and $B, each by \`goatbuilder federate <partner>\` alone"
  setup_hosts "$OC_PREFIX"
  prepare_edge "$A" "$B" true
  prepare_edge "$B" "$A" false
  for s in "$A" "$B"; do oc_core_staging "$s"; done
  if [ "$FE2E_CORE" = role ]; then oc_core_declare_role "$A" "$B"; oc_core_declare_role "$B" "$A"; fi
  # The responders a drill keeps on each core for the round trip (acceptance §2: started by
  # whoever runs the drill).
  for s in "$A" "$B"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  done
  oc_drive "$A" "$B"
  oc_status "$A" "$B"; oc_status "$B" "$A"
  oc_checks "$A" "$B"
  oc_acceptance_checks "$A" "$B"
  oc_defect_checks "$A" "$B"
  oc_outside_check
  collect "$OC_PREFIX"
}

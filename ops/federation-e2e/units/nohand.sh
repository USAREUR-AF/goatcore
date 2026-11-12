#!/usr/bin/env bash
# units/nohand.sh — a lab sovereign ({A}) with no peering to its partner ({B}) at all, and
# exactly the operator's input for one: the peering entry in goatbuilder.yaml (who,
# environment, site, edge class), the offer in its environment's federation.toml
# (ops/federation/<environment>/federation.toml), and the two confirmations, answered at
# the command's prompts. overlay.py --no-peering writes no peering group_vars, no peering
# inventory and no `peerings:` entry for it; the site as it stands before any peering (its
# goatbuilder environment and site, its site inventory naming the core and the edge, the
# operator's enrolled goat profile) is the sovereign's own and is not the peering's.
#
# Every other file and value the peering needs must come from the command: `goatbuilder
# federate allocate` (the per-peering values, the inventory, the deployment halves, the
# peering's federation.toml) and `goatbuilder federate <partner>` (certificates, admission,
# acceptance). Whenever the run cannot go on without one, this unit writes it the minimal
# way, appends one line to RECORD/hand-written.txt (what was written, and the command that
# should have produced it), and goes on, so that later gaps are found too. Check
# nh-nothing-hand-written fails with every line as its reason; an empty list is the pass.
# {B} is an ordinary lab sovereign (overlay.py as every unit has it), and both sides are
# driven by the one command as in units/onecmd.sh. Scenario: nothing-hand-written.
# Sourced by lib/unit.sh.
# shellcheck source=onecmd.sh
. "$HERE/units/onecmd.sh"
OC_PREFIX=nh
# Read by units/onecmd.sh.
export OC_SCENARIO=nothing-hand-written OC_ID_A="" OC_ID_B="partner-"
HAND="$RECORD/hand-written.txt"
hand() { grep -qxF -- "$*" "$HAND" 2>/dev/null || printf '%s\n' "$*" >> "$HAND"; }
# The reviewed lab-only lines, and nothing else: the lab's locally built republisher image
# (a deployment's pin), the FIPS waiver of a stock Ubuntu host, and phase 14's counterparty
# surfaces (only a drill owns both sides; docs/design/federation-acceptance.md).
nh_lab_only() { # $1 line
  case "$1" in
    "value goat_fed_republisher_image in ops/ansible/group_vars/all.yml (block lab-site): lab-only: "*) return 0 ;;
    "value peering_wg_fips_waived_reason in ops/ansible/group_vars/all.yml (block lab-site): lab-only: "*) return 0 ;;
    "values FED_THEIRS_REST, FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER and the drill's reach into the partner's network (phase 14's counterparty surfaces): given only to the separate acceptance step; lab-only: only a drill owns both sides; "*) return 0 ;;
    "values FED_THEIRS_REST, FED_SSH_THEIRS, FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER and the drill's reach into the partner's network, given to phase 14 (\`goatbuilder federate "*" --from 14\`) on "*": lab-only: only a drill owns both sides; a production operator holds only their own (docs/design/federation-acceptance.md)") return 0 ;;
  esac
  return 1
}
# Merge {A}'s clone to its trunk and fast-forward the driver host's checkout.
nh_push() { # $1 commit message
  local c="$WORK/$A/clone"
  git -C "$c" add -A
  git -C "$c" diff --cached --quiet || git -C "$c" commit -q -m "$1"
  git -C "$c" push -q origin HEAD:main
  host_exec "$(drv_host "$A")" git -C /opt/goat pull -q --ff-only origin main
}
# Put one marked block of {A}'s site values into ops/ansible/group_vars/all.yml, replacing
# an earlier block of the same name. Not a group_vars/all/ directory: Ansible loads either
# that directory or all.yml, never both (DataLoader.find_vars_files stops at the first
# match), so a directory would hide every value all.yml holds (ghcr_org among them).
nh_all_block() { # $1 block name, YAML on stdin
  python3 -c '
import re, sys
p, name = sys.argv[1], sys.argv[2]
body = sys.stdin.read()
try:
    text = open(p).read()
except FileNotFoundError:
    text = "---\n"
b, e = "# BEGIN fe2e " + name + "\n", "# END fe2e " + name + "\n"
text = re.sub(re.escape(b) + ".*?" + re.escape(e), "", text, flags=re.S)
if not text.endswith("\n"):
    text += "\n"
open(p, "w").write(text + b + body + e)
' "$WORK/$A/clone/ops/ansible/group_vars/all.yml" "$1"
}
# Add one field to {A}'s declaration of the peering with $1: hosts (from the site's two
# machines; $3 the core address) or a switch set to $3.
nh_declare() { # $1 them, $2 field, $3 value
  python3 - "$WORK/$A/clone/ops/goatbuilder/goatbuilder.yaml" "$1" "$2" "$3" <<'PY'
import sys, yaml
p, them, field, val = sys.argv[1:5]
text = open(p).read(); cfg = yaml.safe_load(text)
for e in cfg["peerings"]:
    if e["with"] == them:
        e[field] = {"edge": "127.0.0.1", "core": val, "user": "goat"} if field == "hosts" else val
open(p, "w").write(text.split("\nschema:")[0] + "\n" + yaml.safe_dump(cfg, sort_keys=False))
PY
}

# The values the operator's environment is first run WITHOUT on {A}, because a command
# could derive each from what the deployment already declares. Each is added back, and
# listed, only when a run of the command names it as missing.
NH_WITHHELD="FED_CORE_SAN FED_ROUTER_DOCKER_HOST FED_COUNTERPARTY_ID FED_WORK"
NH_ADDED=" "
NH_OPT=" "
# Values the operator declares in goatbuilder.yaml where the tree reads the declaration.
NH_DECLARED="FED_SIGNING_KEY FED_CA_CERT FED_CA_KEY FED_DIRECTORY_URL SSL_CERT_FILE"
# Admission's values on a goat-zenoh role core, withheld where the tree derives them.
NH_ADMIT="FED_CORE_ACL_ORG FED_CORE_CERT FED_CORE_KEY FED_CORE_RESTART FED_CORE_DRY_RUN FED_SLOT_CHECK"
NH_ADMIT_TREE=0
# Both sides are treated alike: the same values withheld, given back only in survey mode.
oc_env_more() { # $1 us, $2 them
  local out=() i=0 n=${#OC_ENV[@]} name
  while [ "$i" -lt "$n" ]; do
    if [ "${OC_ENV[$i]}" = -e ]; then
      name="${OC_ENV[$((i + 1))]%%=*}"
      case " $NH_WITHHELD " in
        *" $name "*) case "$NH_ADDED" in *" $name "*) ;; *) i=$((i + 2)); continue ;; esac ;;
      esac
    fi
    out+=("${OC_ENV[$i]}"); i=$((i + 1))
  done
  OC_ENV=("${out[@]}")
  # FED_CORE_CN is left out (phase 8's own default, or what it derives), and given only
  # once a run shows it is needed (NH_CORE_CN, set by oc_after_run): then the CN the
  # rendered edge half pins.
  out=(); i=0; n=${#OC_ENV[@]}
  while [ "$i" -lt "$n" ]; do
    if [ "${OC_ENV[$i]}" = -e ] && [ "${OC_ENV[$((i + 1))]%%=*}" = FED_CORE_CN ]; then i=$((i + 2)); continue; fi
    out+=("${OC_ENV[$i]}"); i=$((i + 1))
  done
  OC_ENV=("${out[@]}")
  [ -z "${NH_CORE_CN:-}" ] || OC_ENV+=(-e "FED_CORE_CN=$NH_CORE_CN")
}
# The CN the rendered edge half pins for our core (goat_zenoh_edge_core_cn), or nothing.
nh_rendered_cn() { # $1 them
  python3 - "$WORK/$A/clone/ops/ansible/group_vars/${1}_edge" <<'PY'
import os, sys, yaml
d, cn = sys.argv[1], ""
for f in sorted(os.listdir(d)) if os.path.isdir(d) else []:
    if f.endswith((".yml", ".yaml")):
        cn = (yaml.safe_load(open(os.path.join(d, f))) or {}).get("goat_zenoh_edge_core_cn", cn)
print(cn or "")
PY
}
# After each run of the command on {A}: a withheld value it names is added back and listed.
oc_after_run() { # $1 us, $2 them, $3 transcript
  [ "$1" = "$A" ] || return 1
  local v added=1
  for v in $NH_WITHHELD; do
    case "$NH_ADDED" in *" $v "*) continue ;; esac
    case " $NH_ADMIT " in
      *" $v "*)
        # Admission's own values: given back only when a line names one as missing.
        if grep -qiE "${v}[^\n]*(unset|not set|missing|required|is empty|no value)|(unset|not set|missing|required|no value|[Ss]et)[^\n]*$v" "$3" 2>/dev/null; then
          NH_ADDED="$NH_ADDED$v "; added=0
          hand "value $v (environment of \`goatbuilder federate $2\`): admission named it as missing on the goat-zenoh role core: $(grep -m1 -E "$v" "$3" | sed 's/\x1b\[[0-9;]*m//g; s/^ *//' | cut -c1-200); admission should take it from the role (admission, W10)"
        fi
        continue ;;
    esac
    case " $NH_DECLARED " in
      *" $v "*)
        # Declared in goatbuilder.yaml: added back only when a line names it as missing.
        if grep -qiE "${v}[^\n]*(unset|not set|missing|required|is empty|no value)|(unset|not set|missing|required|no value)[^\n]*$v" "$3" 2>/dev/null; then
          NH_ADDED="$NH_ADDED$v "; added=0
          hand "value $v (environment of \`goatbuilder federate $2\`): the run named it as missing although goatbuilder.yaml declares it (secrets.federation, environments[].peering_directory); the run should read the declaration (V-keys/V-dir)"
        fi
        continue ;;
    esac
    # Named as missing, not merely mentioned (help text names FED_WORK, for one).
    if grep -qiE "${v}[^\n]*(unset|not set|missing|required|is empty|no value|could not|cannot)|(unset|not set|missing|required|no value|[Ss]et )[^\n]*$v" "$3" 2>/dev/null; then
      NH_ADDED="$NH_ADDED$v "; added=0
      hand "value $v (environment of \`goatbuilder federate $2\`): the run named it as missing; it is derivable from the deployment's declarations (the core's inventory and group_vars, the peering's declaration) and should be resolved by \`goatbuilder federate $2\` itself (W6/W10)"
    fi
  done
  # Phase 14's own surfaces, left out (units/onecmd.sh oc_env): given back, and listed, only
  # when acceptance says it could not derive one.
  for v in FED_OURS_REST FED_EDGE_LOG_OURS FED_SPIRE_OURS; do
    case " ${OC_ACC_ADDED:-} " in *" $v "*) continue ;; esac
    # Only acceptance's own "<value> not derived" counts: its skip hints ("Set FED_SPIRE_OURS
    # and FED_SPIRE_THEIRS") also fire when only the partner's, lab-only, surface is missing.
    if grep -qE "acceptance: $v not derived" "$3" 2>/dev/null; then
      OC_ACC_ADDED="${OC_ACC_ADDED:-} $v"; added=0
      hand "value $v (phase 14 on this side's own surfaces): acceptance could not derive it: $(grep -m1 -oE "$v not derived: .{0,200}" "$3" | tr -d '\r'); this run gave it back (acceptance, W7)"
    fi
  done
  # Our core's certificate, once phase 8 minted it, must carry the CN the edge pins. Only
  # when it does not (or a run names FED_CORE_CN) is FED_CORE_CN given, phase 8 run again,
  # and the value listed.
  if [ -z "${NH_CORE_CN:-}" ]; then
    local want got
    want="$(nh_rendered_cn "$2")"
    got="$(host_exec "$(drv_host "$A")" sh -c "openssl x509 -in /var/lib/goat-federation/$2/pki/core.pem -noout -subject 2>/dev/null" | sed -n 's/.*CN *= *\([^,/]*\).*/\1/p')"
    if { [ -n "$want" ] && [ -n "$got" ] && [ "$got" != "$want" ]; } || grep -q "FED_CORE_CN" "$3" 2>/dev/null; then
      NH_CORE_CN="${want:-core-$A}"
      hand "value FED_CORE_CN=$NH_CORE_CN (environment of \`goatbuilder federate $2\`): phase 8 minted our core's certificate with CN '${got:-?}', but the rendered edge half pins goat_zenoh_edge_core_cn '$want'; the run should derive FED_CORE_CN from that group_var (w3-measure)"
      oc_run "$A" "$2" "core-cn-phase-8" "$2" --from 8 --to 8 || true
      added=0
    fi
  fi
  # Phase 3 reads the peering's federation.toml from the goat context. Only when phase 3
  # refuses for want of it is the generated one copied there by hand, and listed.
  local ctxf="/root/.goat/contexts/default/federation.toml" gen_ft="$WORK/$A/clone/ops/federation/$A/peerings/$2/federation.toml"
  local ctxline
  ctxline="$(sed 's/\x1b\[[0-9;]*m//g' "$3" 2>/dev/null | grep -m1 -E "no federation\.toml at|federation\.toml.*(GB-[0-9]+|missing|not found)" || true)"
  if ! host_exec "$(drv_host "$A")" test -s "$ctxf" && [ -n "$ctxline" ] && ! oc_passed "$RECORD/status-$A.json" 3 && [ -f "$gen_ft" ]; then
    docker cp -q "$gen_ft" "$(drv_host "$A"):$ctxf"
    hand "file $ctxf: the run refused without it ($(echo "$ctxline" | sed 's/^ *//' | cut -c1-160)); copied by hand from the generated ops/federation/$A/peerings/$2/federation.toml; preflight and phase 3 should read the generated file itself (federation.toml, W15/W3)"
    added=0
  fi
  # The edge role asserts where the partner's CA goes (goat_zenoh_edge_partner_ca_path, or
  # goat_zenoh_edge_install_certs); deploy's admission refuses GB-194 without it.
  local gv="ops/ansible/group_vars/${2}_edge/partner-ca.yml" cl="$WORK/$A/clone"
  local d6line
  d6line="$(sed 's/\x1b\[[0-9;]*m//g' "$3" 2>/dev/null | grep -m1 -E 'nothing says where their CA goes|goat_zenoh_edge_partner_ca_path \| length > 0' || true)"
  if [ -n "$d6line" ] && [ ! -f "$cl/$gv" ] && ! grep -qs '^goat_zenoh_edge_partner_ca_path:' "$cl/ops/ansible/group_vars/${2}_edge/peering-deployment.yml"; then
    printf '%s\n' "# Written by hand: where the edge router reads the partner's CA and ours." \
      "goat_zenoh_edge_partner_ca_path: /var/lib/goat-federation/$2/pki/partner-ca.pem" \
      "goat_zenoh_edge_our_ca_path: /var/lib/goat-federation/$2/pki/our-ca.pem" > "$cl/$gv"
    git -C "$cl" add -A && git -C "$cl" commit -q -m "$A: the edge's CA paths for $2, by hand" && git -C "$cl" push -q origin HEAD:main
    host_exec "$(drv_host "$A")" git -C /opt/goat pull -q --ff-only origin main
    hand "file $gv (goat_zenoh_edge_partner_ca_path, goat_zenoh_edge_our_ca_path): the rendered edge half names neither, and the run refused without them: $(echo "$d6line" | sed 's/^ *//' | cut -c1-200); the deployment half should render them from the allocated cert_dir (deployment halves, W15)"
    added=0
  fi
  # The optional edge values the rendered half leaves unset: each is declared (in a block of
  # group_vars/all.yml) and listed only when a run fails without it, naming the failure.
  local ov oline oval
  for ov in goat_zenoh_edge_trust_mode goat_zenoh_edge_acl_trace peering_wg_extra_ports peering_spire_outbound_listen; do
    case "$NH_OPT" in *" $ov "*) continue ;; esac
    oline="$(sed 's/\x1b\[[0-9;]*m//g' "$3" 2>/dev/null | grep -m1 -E "$ov" | grep -iE 'fatal|fail|refus|missing|assert|blocked|not set|unset|undefined' || true)"
    if [ -z "$oline" ] && [ "$ov" = goat_zenoh_edge_acl_trace ]; then
      oline="$(sed 's/\x1b\[[0-9;]*m//g' "$3" 2>/dev/null | grep -m1 -E "the ACL trace was not read|trace verdict '<none>'" || true)"
    fi
    [ -n "$oline" ] || continue
    case "$ov" in
      goat_zenoh_edge_trust_mode) oval="fragment" ;;
      goat_zenoh_edge_acl_trace) oval="true" ;;
      peering_wg_extra_ports) oval="[8443]" ;;
      peering_spire_outbound_listen) oval="\"$(edge_ip "$A")\"" ;;
    esac
    printf '%s: %s\n' "$ov" "$oval" | nh_all_block "optional-$ov"
    nh_push "$A: $ov, which a run failed without"
    NH_OPT="$NH_OPT$ov "
    hand "value $ov: $oval in ops/ansible/group_vars/all.yml (block optional-$ov): the rendered half leaves it unset and a run failed without it: $(echo "$oline" | sed 's/^ *//' | cut -c1-200) (deployment halves, W15)"
    added=0
  done
  # Phase 12 reads our CA where the edge's values name it.
  if grep -q "our-ca.pem" "$3" 2>/dev/null && ! host_exec "$(drv_host "$A")" test -f "/var/lib/goat-federation/$2/pki/our-ca.pem" -a -s "/var/lib/goat-federation/$2/pki/our-ca.pem"; then
    host_exec "$(drv_host "$A")" sh -c "install -d -m 0755 /var/lib/goat-federation/$2/pki && install -m 0644 /etc/fed-e2e/ca.pem /var/lib/goat-federation/$2/pki/our-ca.pem"
    hand "file /var/lib/goat-federation/$2/pki/our-ca.pem (our CA, where the edge's and the republisher's values read it): no phase placed it; the run named it missing; mint-peering-leaves or allocate should (W10/W15)"
    added=0
  fi
  return "$added"
}

# Acceptance failed: when it names something the unit can supply (oc_after_run: the ACL
# trace the receiving edge must keep, among the optional values), supply it, redeploy the
# edge by the command's own route (the plain run, whose phase 12 runs the edge play again),
# and let acceptance run once more. Anything else stays as acceptance reported it.
oc_accept_retry() { # $1 us, $2 them, $3 acceptance output
  [ "$1" = "$A" ] || return 1
  oc_after_run "$1" "$2" "$3" || return 1
  # shellcheck disable=SC2046
  oc_run "$1" "$2" "after-acceptance" "$2" $(oc_as "$1") || true
  oc_status "$1" "$2"
  echo "after acceptance $1: phases passed: $(oc_summary "$RECORD/status-$1.json")" >> "$RECORD/oc-runs.txt"
  return 0
}

# Commit what is in the driver host's checkout as the operator's reviewed change: copied
# into the sovereign's clone, committed, pushed to its trunk, and the host fast-forwarded.
nh_commit_host_changes() { # $1 sovereign, $2 message
  local s="$1" msg="$2" h c f; h="$(drv_host "$1")"; c="$WORK/$1/clone"
  host_exec "$h" git -C /opt/goat status --porcelain --untracked-files=all > "$WORK/$s/host-changes.txt" || true
  while read -r _ f; do
    [ -n "$f" ] || continue
    mkdir -p "$(dirname "$c/$f")"
    docker cp -q "$h:/opt/goat/$f" "$c/$f"
  done < "$WORK/$s/host-changes.txt"
  git -C "$c" add -A
  git -C "$c" diff --cached --quiet || git -C "$c" commit -q -m "$msg"
  git -C "$c" push -q origin HEAD:main
  host_exec "$h" sh -c 'cd /opt/goat && git checkout -q -- . && git clean -fdq && git pull -q --ff-only origin main'
}

# What a hand-written deployment.yml would have to set for the peering, minus what the
# command already set for it: overlay.py's peering files, rendered aside and compared.
nh_hand_halves() { # $1 us, $2 them
  local s="$1" them="$2" c="$WORK/$1/clone" d="$WORK/$1/overlay-aside"
  rm -rf "$d"; mkdir -p "$d/ops/ansible/inventory"
  python3 "$HERE/overlay.py" "$d" "$s" "$them" "$(octet "$s")" "$ZENOH_IMAGE" --republisher-image "$RP_IMAGE" \
    --core-addr "$(core_ip "$s")" --spire-listen "$(edge_ip "$s")"
  python3 - "$c" "$d" "$them" "$HAND" <<'PY'
import os, sys, yaml
clone, aside, them, hand = sys.argv[1:5]
def setvars(group):
    got = {}
    g = os.path.join(clone, "ops/ansible/group_vars", group)
    for root in (g, os.path.join(clone, "ops/ansible/group_vars/all")):
        if not os.path.isdir(root):
            continue
        for f in sorted(os.listdir(root)):
            if f.endswith((".yml", ".yaml")):
                try:
                    got.update(yaml.safe_load(open(os.path.join(root, f))) or {})
                except Exception:
                    pass
    return got
for group, name, cmd in ((f"{them}_edge", "deployment.yml", "the edge half"), (f"{them}_core", "republisher.yml", "the core half")):
    want = yaml.safe_load(open(os.path.join(aside, "ops/ansible/group_vars", group, name))) or {}
    have = setvars(group)
    missing = {k: v for k, v in want.items() if k not in have}
    if not missing:
        continue
    os.makedirs(os.path.join(clone, "ops/ansible/group_vars", group), exist_ok=True)
    path = f"ops/ansible/group_vars/{group}/{name}"
    open(os.path.join(clone, path), "w").write(f"# Written by hand: {cmd} of the peering, which no command rendered.\n" + yaml.safe_dump(missing, sort_keys=True))
    with open(hand, "a") as h:
        h.write(f"file {path} ({cmd}: {', '.join(sorted(missing))}): should be rendered by `goatbuilder federate allocate` from the declaration and the site's values (deployment halves, W15)\n")
PY
}

# One side's peering declared from nothing: the operator's profile in the goat context, the
# peering entry and the offer, and every other file from `goatbuilder federate allocate`. A
# and B are this side's own within the function (bash's dynamic scope), so each helper
# called here works on this side.
nh_side_from_nothing() { # $1 us, $2 them, $3 "true" when this side listens
  local A="$1" B="$2" listener="$3"
  local c="$WORK/$A/clone" h s them gen inv f declares="${NH_DECLARES:-0}"
  h="$(drv_host "$A")"
  rec "nh-$A-starts-with-no-peering" -- bash -c \
    "cd '$c' && ls ops/ansible/group_vars/ ops/ansible/inventory/ 2>&1; grep -n 'peerings' ops/goatbuilder/goatbuilder.yaml; ! ls -d ops/ansible/group_vars/${B}_edge ops/ansible/group_vars/${B}_core ops/ansible/inventory/peering-edges.ini 2>/dev/null && ! grep -q '^peerings:' ops/goatbuilder/goatbuilder.yaml"
  # The operator's profile, the sovereign's own before any peering: the goat context holds
  # the enrolled certificate and the CA roots, and nothing of the peering.
  local p="/root/.goat/contexts/default"
  host_exec "$h" sh -c "[ -d /opt/goat/.git ] || git clone -q /srv/goat.git /opt/goat"
  host_exec "$h" sh -c "git -C /opt/goat config user.name '$A operator'; git -C /opt/goat config user.email 'ops@$A.test'"
  host_exec "$h" install -d -m 0700 /root/.goat /root/.goat/contexts "$p"
  host_exec "$h" install -m 0644 /etc/fed-e2e/profile.pem "$p/mtls.cert.pem"
  host_exec "$h" install -m 0644 /etc/fed-e2e/ca.pem "$p/ca-roots.pem"
  # One group configures the core router, as in a deployment: the site overlay's [core]
  # (the site inventory's hosts, which allocate reads) keeps no goat-zenoh variable of its
  # own, else admission finds two role groups for one machine (GB-236).
  if [ "${NH_ADMIT_TREE:-0}" = 1 ] && [ -f "$c/ops/ansible/group_vars/core/zenoh.yml" ]; then
    git -C "$c" rm -q ops/ansible/group_vars/core/zenoh.yml
    git -C "$c" commit -q -m "$A: the core router is configured by its core_router group only"
    git -C "$c" push -q origin HEAD:main
    host_exec "$h" git -C /opt/goat pull -q --ff-only origin main
  fi
  # THE OPERATOR'S INPUT, and nothing else: the peering entry, and the environment's offer.
  # NH_OFFER_<side> (comma-separated release sets): this side's peering offers only those,
  # narrowing its environment's release (the peering entry's `offer:`; units/offer.sh, B5).
  local offer=""; eval "offer=\${NH_OFFER_$A:-}"
  python3 - "$c/ops/goatbuilder/goatbuilder.yaml" "$B" "$A" "$declares" "https://$DIRECTORY_IP:8443" "$offer" <<'PY'
import sys, yaml
p, them, env, declares, url, offer = sys.argv[1:7]
text = open(p).read()
cfg = yaml.safe_load(text)
cfg["peerings"] = [{"with": them, "environment": env, "site": "home", "edge_class": "edge"}]
if offer:
    cfg["peerings"][0]["offer"] = [o for o in offer.split(",") if o]
if declares == "1":
    cfg.setdefault("secrets", {})["federation"] = {
        "signing_key": "/etc/fed-e2e/signing.key", "ca_cert": "/etc/fed-e2e/ca.pem", "ca_key": "/etc/fed-e2e/ca-key.pem"}
    for e in cfg["environments"]:
        if e["name"] == env:
            e["peering_directory"] = {"url": url, "ca_cert": "/etc/fed-e2e/directory-ca.pem"}
open(p, "w").write(text.split("\nschema:")[0] + "\n" + yaml.safe_dump(cfg, sort_keys=False))
PY
  mkdir -p "$c/ops/federation/$A"
  cat > "$c/ops/federation/$A/federation.toml" <<TOML
# $A: what this environment offers every partner (the operator's input).
sovereign_id = "$A"
term_days    = 30
cut_contact  = "fed-ops@$A.test"

[transport]
endpoint     = "$(boundary_ip "$A")"
listener     = $listener
measured_mtu = 1380

[data]
release                  = ["release/all/**", "release/$B/**"]
accept_hz                = 100
accept_bytes_per_message = 1048576
accept_distinct_keys     = 5000
TOML
  # Each side's input under its own name: both sides are declared into this one record.
  mkdir -p "$RECORD/operator-input/$A"
  cp "$c/ops/goatbuilder/goatbuilder.yaml" "$c/ops/federation/$A/federation.toml" "$RECORD/operator-input/$A/"
  git -C "$c" add -A && git -C "$c" commit -q -m "$A: declare the peering with $B"
  git -C "$c" push -q origin HEAD:main
  host_exec "$h" git -C /opt/goat pull -q --ff-only origin main

  # EVERYTHING THE PRODUCT IS SUPPOSED TO DERIVE IS LEFT OUT FIRST: peerings[].hosts, the
  # deployment and federation_toml switches, the site values. A dry run of allocate shows
  # what this tree still needs; each thing is then added the minimal way and listed, and
  # the dry run is repeated, so the list is what THIS tree needs, never what an older one
  # did. The lab's own pin and choices are declared first (lab-site.yml): they are the
  # deployment's, not derivable, and are listed as such.
  local rendered_tree=0
  has_text crates/goatbuilder/src/config.rs "pub deployment: Option<String>" && rendered_tree=1
  if [ "$rendered_tree" = 1 ]; then
    nh_all_block lab-site <<YAML
# $A: the lab's own pin and choices for every peering (ops/federation-e2e/units/nohand.sh).
goat_fed_republisher_image: "$RP_IMAGE"
peering_wg_fips_waived_reason: "federation e2e on stock Ubuntu, DBB-WVR-001; no FIPS claim"
YAML
    hand "value goat_fed_republisher_image in ops/ansible/group_vars/all.yml (block lab-site): lab-only: locally built image, a deployment pin"
    hand "value peering_wg_fips_waived_reason in ops/ansible/group_vars/all.yml (block lab-site): lab-only: stock Ubuntu with no FIPS module, the deployment's own recorded waiver"
    nh_push "$A: the lab's pin and choices for peering halves"
  else
    mark nothing-hand-written.rendered-halves-need-no-hand-site-values BLOCKED "the tree under test offers no peering deployment: rendered (config.rs), so no deployment half can be rendered"
  fi
  local try out named added declared=" " first=1
  for try in 1 2 3 4 5; do
    oc_env "$A" "$B"
    out="$WORK/$A/allocate-dry-run-$try.out"
    docker exec "${OC_ENV[@]}" -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate allocate --dry-run > "$out" 2>&1 || true
    cp "$out" "$RECORD/allocate-dry-run-$try.out"
    if [ "$first" = 1 ]; then
      first=0
      # Defect checks on what the tree derives with nothing but the operator's input.
      if [ "$rendered_tree" = 1 ]; then
        rec "nh-$A-rendered-halves-need-no-hand-site-values" --expect-not-in "GB-232" -- cat "$out"
      fi
      rec "nh-$A-site-hosts-unambiguous" --expect-not-in "GB-234" -- cat "$out"
    fi
    added=""
    # hosts: first, since the machines' addresses are what the core endpoint and the SPIRE
    # address derive from. Needed when allocate is ambiguous about them (GB-234), or takes
    # none from the site inventory (no `hosts` row) and renders no inventory for the
    # peering, or cannot derive the core's address (GB-232 naming the core endpoint or the
    # SPIRE server address) with no hosts taken.
    local need_hosts=0
    if grep -q 'GB-234' "$out"; then need_hosts=1
    elif ! grep -qE '^[a-z]+ +hosts ' "$out"; then
      if grep -q 'GB-232' "$out"; then
        grep 'GB-232' "$out" | grep -qE 'goat_fed_republisher_core_endpoint|peering_spire_server_addr' && need_hosts=1
      elif ! grep -qE '(would write|written|unchanged): ops/ansible/inventory/peering-' "$out"; then
        need_hosts=1
      fi
    fi
    if has_text crates/goatbuilder/src/config.rs "pub hosts: Option<PeeringHosts>" && [ "$need_hosts" = 1 ] \
       && case "$declared" in *" hosts "*) false ;; *) true ;; esac; then
      nh_declare "$B" hosts "$(core_ip "$A")"; declared="$declared hosts "
      hand "value peerings[].hosts {edge: 127.0.0.1, core: $(core_ip "$A"), user: goat} in goatbuilder.yaml: allocate took no hosts from the site inventory ($A-site.ini names both machines) and, without them, rendered no inventory or could not derive the core's address (inventory, W15/W16)"
      nh_push "$A: the peering's hosts, which allocate's dry run needed"
      continue
    fi
    # GB-232: exactly the site values it named.
    named="$(grep 'GB-232' "$out" | grep -oE '\b(goat_fed_republisher_core_endpoint|goat_fed_republisher_image|goat_zenoh_edge_core_cn|goat_zenoh_edge_image|peering_spire_server_addr)\b' | sort -u | tr '\n' ' ' || true)"
    if [ -n "$named" ]; then
      local v; { [ -f "$WORK/$A/federation-site.body" ] && cat "$WORK/$A/federation-site.body" || true
        echo "# $A: the site values allocate could not derive (GB-232), written by hand."
        for v in $named; do
          case "$v" in
            goat_fed_republisher_core_endpoint) echo "$v: \"tls/$(core_ip "$A"):7447\"" ;;
            goat_fed_republisher_image) echo "$v: \"$RP_IMAGE\"" ;;
            goat_zenoh_edge_core_cn) echo "$v: \"$A-core-$B\"" ;;
            goat_zenoh_edge_image) echo "$v: \"$ZENOH_IMAGE\"" ;;
            peering_spire_server_addr) echo "$v: \"$(core_ip "$A")\"" ;;
          esac
        done; } | grep -v '^#' > "$WORK/$A/federation-site.new"
      mv "$WORK/$A/federation-site.new" "$WORK/$A/federation-site.body"
      nh_all_block federation-site < "$WORK/$A/federation-site.body"
      hand "values in ops/ansible/group_vars/all.yml (block federation-site: $(echo $named | sed 's/ /, /g')): the site values allocate refused without (GB-232); it should derive them from their owners (deployment halves, W15)"
      added="$added site"
    fi
    # The switches, where an absent one does not render for a new peering.
    if [ "$rendered_tree" = 1 ] && case "$declared" in *" deployment "*) false ;; *) true ;; esac && ! grep -q 'peering-deployment.yml' "$out" && ! grep -q 'GB-' "$out"; then
      nh_declare "$B" deployment rendered; declared="$declared deployment "; added="$added deployment"
      hand "value peerings[].deployment: rendered in goatbuilder.yaml: allocate rendered no deployment half for the new peering without it; rendering should be the default (deployment halves, W15/W17)"
    fi
    if has_text crates/goatbuilder/src/config.rs "pub federation_toml: Option<String>" && case "$declared" in *" federation_toml "*) false ;; *) true ;; esac \
       && ! grep -q "peerings/$B/federation.toml" "$out" && ! grep -q 'GB-' "$out"; then
      nh_declare "$B" federation_toml rendered; declared="$declared federation_toml "; added="$added federation_toml"
      hand "value peerings[].federation_toml: rendered in goatbuilder.yaml: allocate generated no federation.toml for the new peering without it; generating should be the default (federation.toml, W15/W17)"
    fi
    [ -n "$added" ] || break
    nh_push "$A: what allocate's dry run named as missing:$added"
  done
  case "$declared" in *" hosts "*) ;; *) has_text crates/goatbuilder/src/config.rs "pub hosts: Option<PeeringHosts>" || mark nothing-hand-written.named-allocate-renders-the-inventory BLOCKED "the tree under test offers no peering hosts: (config.rs), so allocate renders no inventory" ;; esac
  # Every declared peering (this sovereign declares one). The named form, allocate <peering>,
  # renders no inventory even when every peering declares hosts: (measured on the stage 2
  # candidate), so the unit uses the whole form.
  local alloc=(allocate)
  oc_env "$A" "$B"
  rec "nh-$A-allocate" -- docker exec "${OC_ENV[@]}" -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate "${alloc[@]}"
  rec "nh-$A-allocate-wrote" -- bash -c "docker exec $h git -C /opt/goat status --porcelain --untracked-files=all; docker exec $h git -C /opt/goat status --porcelain --untracked-files=all | grep -q ."
  # Defect check: a new peering's files are rendered, never left on hand (allocate names
  # such a peering in a `hand` row).
  rec "nh-$A-allocate-leaves-nothing-on-hand" -- bash -c \
    "f=\$(ls '$RECORD'/*-nh-$A-allocate.log | head -1); grep -E '^[a-z]+ +hand +' \"\$f\" && exit 1; echo 'no hand row'"
  nh_commit_host_changes "$A" "$A: what goatbuilder federate allocate wrote for $B"
  # Defect check: the named form renders the same inventory when every peering declares
  # hosts: (a dry run on the committed tree, which writes nothing).
  if has_text crates/goatbuilder/src/config.rs "pub hosts: Option<PeeringHosts>"; then
    rec "nh-$A-named-allocate-renders-the-inventory" --expect-in "inventory/peering-" -- \
      docker exec "${OC_ENV[@]}" -w /opt/goat "$h" goatbuilder -C /opt/goat --yes federate allocate "$B" --dry-run
  fi
  gen="# GENERATED by \`goatbuilder federate allocate\`"

  # The inventory of the peering's groups.
  inv=""
  for f in "ops/ansible/inventory/peering-${B}_edge.ini" ops/ansible/inventory/peering-edges.ini; do
    [ -f "$c/$f" ] && grep -q "^\[${B}_edge\]" "$c/$f" && { inv="$f"; break; }
  done
  rec "nh-$A-inventory-from-the-command" -- bash -c "echo 'inventory: ${inv:-none}'; [ -n '$inv' ] && head -1 '$c/$inv' | grep -qF '$gen'"
  if [ -z "$inv" ]; then
    printf '%s\n' "[peering_edges:children]" "${B}_edge" "" "[${B}_edge]" "$A-edge-$B ansible_host=127.0.0.1 ansible_user=goat" "" \
      "[peering_cores:children]" "${B}_core" "" "[${B}_core]" "$A-core-$B ansible_host=$(core_ip "$A") ansible_user=goat" "" > "$c/ops/ansible/inventory/peering-edges.ini"
    hand "file ops/ansible/inventory/peering-edges.ini ([${B}_edge] $A-edge-$B, [${B}_core] $A-core-$B): should be rendered by \`goatbuilder federate allocate\` from the site inventory (inventory, W15)"
  fi
  # The per-peering values, and the deployment halves.
  rec "nh-$A-edge-values-from-the-command" -- bash -c "ls '$c/ops/ansible/group_vars/${B}_edge/' 2>&1; head -1 '$c/ops/ansible/group_vars/${B}_edge/peering-values.yml' | grep -qF '$gen'"
  rec "nh-$A-edge-half-from-the-command" -- bash -c "grep -l 'goat_zenoh_edge_image' '$c/ops/ansible/group_vars/${B}_edge/'* 2>/dev/null | xargs -r head -1 | grep -qF '$gen'"
  rec "nh-$A-core-half-from-the-command" -- bash -c "grep -l 'goat_fed_republisher_image' '$c/ops/ansible/group_vars/${B}_core/'* 2>/dev/null | xargs -r head -1 | grep -qF '$gen'"
  # Halves the command rendered are its own; only a group it rendered none for is written by hand.
  if ! grep -qs "goat_zenoh_edge_image" "$c/ops/ansible/group_vars/${B}_edge/peering-deployment.yml" \
     || ! grep -qs "goat_fed_republisher_image" "$c/ops/ansible/group_vars/${B}_core/peering-deployment.yml"; then
    nh_hand_halves "$A" "$B"
  fi
  # The peering's federation.toml, and where phase 3 reads it: the goat context.
  local ft="ops/federation/$A/peerings/$B/federation.toml"
  rec "nh-$A-federation-toml-from-the-command" -- bash -c "cat '$c/$ft' 2>&1; head -1 '$c/$ft' 2>/dev/null | grep -qF '$gen'"
  git -C "$c" add -A
  git -C "$c" diff --cached --quiet || { git -C "$c" commit -q -m "$A: what no command wrote for $B, by hand"; git -C "$c" push -q origin HEAD:main; host_exec "$h" git -C /opt/goat pull -q --ff-only origin main; }

}

nh_main() {
  local s them gen inv
  : > "$HAND"; : > "$OUTSIDE"
  for s in "$A" "$B"; do eval "FE2E_OVERLAY_ARGS_$s=--no-peering"; export "FE2E_OVERLAY_ARGS_$s"; done
  # The core of each sovereign: deployed by the ref's goat-zenoh role (default), or the
  # hand-run core for comparison (FE2E_NH_CORE=hand).
  FE2E_CORE="${FE2E_NH_CORE:-role}"; export FE2E_CORE
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  export_helpers
  export -f oc_passed
  export OC_PREFIX HP ZENOH_IMAGE GB
  say "nothing-hand-written: $A and $B each declare their peering from nothing; every other file from the command"
  setup_hosts nh

  for s in "$A" "$B"; do oc_core_staging "$s"; done
  # A tree whose admission works a goat-zenoh role core itself (GB-237: it writes
  # zenoh_sovereign_peerings; it runs the role's play with site_id from the core group, places
  # the org fragment and the router leaf where the role reads them, and judges the slot check):
  # {A} declares only what a deployment declares for its core (site_id in the core group), and
  # admission's values are withheld; one is given back, and listed, only when a run names it
  # as missing.
  local admit_tree=0
  [ "$FE2E_CORE" = role ] && has_text ops/errors/catalog.yaml "code: GB-237" && admit_tree=1
  if [ "$admit_tree" = 1 ]; then
    NH_ADMIT_TREE=1
    for s in "$A" "$B"; do
      printf '%s\n' "# $s's core router site (the goat-zenoh role's mandatory site_id)." "site_id: $s" > "$WORK/$s/core-site.yml"
      core_declare "$s" ops/ansible/group_vars/core_router/site.yml "$WORK/$s/core-site.yml" "$s: the core router's site"
    done
    NH_WITHHELD="$NH_WITHHELD $NH_ADMIT"
  elif [ "$FE2E_CORE" = role ]; then
    oc_core_declare_role "$B" "$A"
    oc_core_declare_role "$A" "$B"
    hand "file ops/ansible/group_vars/core_router/peerings.yml (zenoh_sovereign_peerings: [{sovereign: $B, cn: fed.$B.$A.01}]): the core's admission of the partner in the goat-zenoh role's own declarations, written by hand and merged; phase 10's admission should write it into the reviewed change from the agreement (admission, W10)"
    hand "file /usr/local/sbin/fe2e-core-role and values FED_CORE_RESTART, FED_CORE_DRY_RUN (place the org fragment on the core host, then zenoh-deploy.yml -i inventory/core-router.ini -e target=core_router -e site_id=$A, with --check --diff for the dry run): admission's play route FED_CORE_PLAY runs only \`-i inventory/peering-edges.ini --limit <with>_core\`, which cannot give zenoh-deploy.yml its mandatory target and site_id, and it writes the org fragment on the driver host, never where the role reads it; admission should run the goat-zenoh role from the core_router inventory group with site_id from it and place the fragment at the role's goat_zenoh_acl_org_path on that host (admission, W10)"
    hand "value FED_CORE_ACL_ORG (/var/lib/goat-core-admission/acl-org-fragment.json5, a driver-host copy fetched from the core host): the role keeps the org fragment at goat_zenoh_acl_org_path (/etc/$A-zenoh/acl-org-fragment.json5) on the core host; admission should read and write it there (admission, W10)"
    hand "values FED_CORE_CERT, FED_CORE_KEY (/root/.goat/state/$A-leaves/router-cert.pem and router-key.pem): where the goat-zenoh role reads the router leaf (goat_leaf, the controller's state directory); admission should take the location from the role, not from the operator (admission, W10)"
    hand "value FED_SLOT_CHECK=skip: this core declares no lazy slot storage (preflight reports the slot check NOT APPLICABLE: no zenoh-lazy-storage.yml), yet admission refuses without FED_SLOT_REGISTRY_DB, FED_SLOT_PG_SSH or skip; it should judge the slot check NOT APPLICABLE the same way (admission, W10)"
  else
    hand "files /srv/goat-core/{seed.json5,acl-org-fragment.json5,config.json5,core.pem,core-key.pem,restart,dry-run} and values FED_CORE_CONFIG_PATH, FED_CORE_ACL_ORG, FED_CORE_ACL_SEED, FED_CORE_ACL_OUT, FED_CORE_CERT, FED_CORE_KEY, FED_CORE_RESTART, FED_CORE_DRY_RUN, FED_SLOT_CHECK: lab-only: the hand-run core (FE2E_NH_CORE=hand, kept for comparison) is started with docker run, so no deployment declaration exists for any command to read; with the role core (the default) these lines are replaced by the role-core gaps"
  fi
  # The operator's keys and the peering directory: declared in goatbuilder.yaml where the
  # tree reads the declarations (secrets.federation, environments[].peering_directory),
  # which is the operator's input like the peering entry; the environment values are then
  # left out, and given (and listed) only when a run names one as missing.
  NH_DECLARES=0
  has_text ops/goatbuilder/schema/goatbuilder-v1.schema.json '"peering_directory"' && NH_DECLARES=1
  local declares="$NH_DECLARES"
  if [ "$declares" = 1 ]; then
    NH_WITHHELD="$NH_WITHHELD $NH_DECLARED"
  else
    hand "values FED_SIGNING_KEY, FED_CA_CERT, FED_CA_KEY (paths of the operator's signing key and CA): should be resolved from the goat context or goatbuilder.yaml secrets (W6 ca-key-resolve)"
    hand "values FED_DIRECTORY_URL, SSL_CERT_FILE (the peering directory and its CA): should come from the environment's declaration in goatbuilder.yaml"
  fi
  if [ "${FE2E_ACC_VALUES:-omit}" = set ] || ! has_text ops/federation/standup/acceptance_surfaces.py "def "; then
    hand "values FED_OURS_REST, FED_EDGE_LOG_OURS, FED_SPIRE_OURS (phase 14 on this side's own surfaces): should be derived from the core and edge declarations (acceptance, W7)"
  fi

  # Phase 14's counterparty surfaces: only the separate acceptance step (oc_accept) is given
  # them; phase 14 inside the command runs without them and is BLOCKED ("no counterparty
  # surface (FED_THEIRS_REST)").
  hand "values FED_THEIRS_REST, FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER and the drill's reach into the partner's network (phase 14's counterparty surfaces): given only to the separate acceptance step; lab-only: only a drill owns both sides; a production operator holds only their own, so phase 14 inside \`goatbuilder federate $B\` is BLOCKED, exit 3, without them (no counterparty surface (FED_THEIRS_REST)), the designed reading (docs/design/federation-acceptance.md); production proves the partner's half through the bilateral session exchanged by file"

  # Each side declares its peering from nothing, the same way (nh_side_from_nothing).
  nh_side_from_nothing "$A" "$B" false
  nh_side_from_nothing "$B" "$A" true

  # The responders a drill keeps on each core (acceptance §2), then both operators run the
  # one command, as in units/onecmd.sh.
  for s in "$A" "$B"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  done
  oc_drive "$B" "$A"
  # Admission through plan -> deploy: {B}'s route on its own role core (units/nohand-loop.sh
  # drives {B} by `federate plan` and `federate deploy` from phase 10), while {A} admits by the
  # plain run. Each route is exercised on a role core with admission's values withheld.
  if [ "$NH_ADMIT_TREE" != 1 ]; then
    mark nothing-hand-written.plan-deploy-admits BLOCKED "the tree under test does not admit on a goat-zenoh role core (no GB-237), so neither side admits through plan -> deploy there"
  elif oc_passed "$RECORD/status-$B.json" 1 2 3 4 5 6 7 8 9; then
    rec "nh-$B-plan-deploy-admits" -- bash -c \
      "cat '$RECORD/$OC_PREFIX-$B-'*deploy.log 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r' | grep -E 'Phase 10|core router|admi|✗|GB-[0-9]+' | head -40;
       cat '$RECORD/$OC_PREFIX-$B-'*deploy.log 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | grep -qE 'our core router restarted through'"
  else
    mark nothing-hand-written.plan-deploy-admits BLOCKED "phase 9 was never passed on $B, so no plan could be deployed: $(oc_stopped_at "$B")"
  fi
  oc_status "$A" "$B"
  oc_status "$A" "$B"; oc_status "$B" "$A"
  sed "s/^/step (outside the command): /" "$OUTSIDE" | grep -E "\(($A|$B)\)" >> "$HAND" || true
  oc_checks "$A" "$B"
  oc_acceptance_checks "$A" "$B"
  oc_defect_checks "$A" "$B"
  local args=() l
  # Every line is printed. Lab-only is a fixed, reviewed set (nh_lab_only); any other line,
  # labelled lab-only or not, fails the check.
  while IFS= read -r l; do
    [ -n "$l" ] || continue
    nh_lab_only "$l" || args+=(--expect-not-in "$l")
  done < "$HAND"
  rec "nh-nothing-hand-written" ${args[@]+"${args[@]}"} -- bash -c "echo 'written by a person besides the operator input:'; cat '$HAND'; [ -s '$HAND' ] || echo none"
  collect nh
}

# The fix-and-resume loop (steer: one integrator on the lab): units/nohand-loop.sh.
# shellcheck source=/dev/null
# FE2E_NH_MODE: strict (default: nothing given back, nothing done by hand, the first refusal
# fails the unit), loop (strict, and wait for a fixer at each refusal), or survey (the old
# omit-first pass: each missing value is given back and listed, to see every gap at once).
FE2E_NH_MODE="${FE2E_NH_MODE:-strict}"; [ "${FE2E_NH_LOOP:-0}" = 1 ] && FE2E_NH_MODE=loop
case "$FE2E_NH_MODE" in strict|loop) . "$HERE/units/nohand-loop.sh" ;; survey) ;; *) echo "nohand: FE2E_NH_MODE must be strict, loop or survey" >&2; exit 2 ;; esac
unit_main() { nh_main; }

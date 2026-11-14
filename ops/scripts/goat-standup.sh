#!/usr/bin/env bash
# goat-standup.sh — THE single entrypoint for a sandbox-tier goatnet standup.
#
# THE DEFECT THIS CLOSES: there was no one command and no one procedure. The
# ordering of ~15 phases lived in prose and in the executor's head, stitched
# from per-env playbooks, so a standup took many retries and an expert in the
# loop. This script IS the ordering,
# executable and inspectable, chaining the built tools + playbooks + gates and
# pausing at the genuinely operator-fired steps with the exact instruction.
#
# It follows the canonical sequence in docs/operations/goatnet-standup.md
# (which is the human/AI-readable form of this same procedure — one doc, one
# script, one order).
#
# SCOPE: any goatnet built around ONE core mgmt host (a mini PC, a Jetson or
# similar), WITH OR WITHOUT a separate
# observability host. The obs host (the ADR 0826 "Orin-C": Prometheus +
# Alertmanager + Grafana, plus Forgejo — the pub/sub state-of-record primary) is
# part of THIS path, not a prod-only extra: when the site declares one and it is
# identified + prepped (see "The obs leg" below) the driver builds it in
# PARALLEL with the core host, automatically. A SECOND HOST SITE joining an
# existing goatnet (goatnet_founder:false, ADR 1084) IS part of this path: when
# the site's group_vars declare it a joiner the driver runs the three-pass Spock
# join (pass 1 bootstrap without Spock, pass 2 seed-from-peer, pass 3 bootstrap)
# and the founder-side mirror, in place of seeding a fresh account. What remains
# a follow-on hand step is a same-site core A/B streaming standby (intra_site_pair)
# on an already-joined site — that is not the join itself.
#
# STATUS: v2 sequencer. Every step it RUNS is an already-used tool/playbook; the
# value it adds is the order + the gates + the manual-step prompts. The phase
# commands are transcribed from the validated command crib sheet in
# docs/operations/goatnet-standup.md (the "corrected forms" that run on a fresh
# site). It has NOT yet passed a cold-read drill (a non-author, no AI, this doc
# set only, standing up a goatnet with zero improvisation) — that drill is the
# definition of done (docs/project/standup-remediation-plan.md), and the
# internet-profile chaining in particular (the phase 5 -> 6 schema-wait hand-off and
# the post-login re-runs) has not been driven end to end. Run --plan first,
# always, and read the handbook alongside it.
#
# Usage:
#   ops/scripts/goat-standup.sh <site-id> [--plan] [--profile lan|internet]
#                                         [--from <phase>] [--to <phase>]
#                                         [--skip-obs] [--obs-only] [--check]
#     --plan        print the ordered phase plan and the exact command per
#                   phase; execute NOTHING. Offline-safe; start here.
#     --profile     topology (default: auto-detected from the config — a group
#                   vars file with `dashboard_cert_delivery_mode: acme` is
#                   internet, otherwise lan). Override when auto-detect is wrong.
#     --from/--to   run a sub-range (resume after a manual step, or after a
#                   failed phase once you have fixed its cause — every phase is
#                   idempotent, so re-running from the failed number is safe).
#     --check       on a goatlite, run the acceptance phase (13), which is off by
#                   default there; `goatbuilder assay <site>` runs it on its own.
#                   Every other goatnet runs phase 13 always.
#
#   Environment:
#     GOAT_AWS_PROFILE   aws-vault profile the ansible phases run under
#                        (has a default). A sovereign-owned goatnet whose
#                        cloud lives in its OWN AWS account sets this to its own
#                        profile; a LAN-only goatnet that needs no AWS still
#                        needs a profile aws-vault can resolve for GHCR pulls.
#
# Two topology profiles, one order. Phases are numbered 1 to 14, the same for
# both profiles, with no gaps and no lettered sub-phases; a LAN site skips the
# internet-only phases (--from/--to over a skipped phase runs nothing), and
# phase 10 differs by profile:
#
#   Phase  What                                    lan       internet
#   1      gates: config, inputs, ACL, door, hosts  yes       yes
#   2      host baseline + LUKS                    yes       yes
#   3      per-instance secrets                    yes       yes
#   4      leaf certs + name snapshot              yes       yes
#   5      mgmt-stack (site-bootstrap, the         yes       yes
#          stack part), then relay-connect
#   6      identity stack (Ory), then the rest     yes       yes
#          of site-bootstrap (account seed); a
#          joiner seeds from its founder and
#          mirrors here
#   7      public TLS + relay tier                 --        yes
#   8      mesh-enroll host + re-mint router       --        yes (MANUAL)
#   9      zenoh router + SPIRE (founder)          yes       yes
#   10     LAN services  /  dashboard + OIDC       lan svc   dashboard+oidc
#   11     first login + admin promote +           both (MANUAL)
#          the bootstrap NetBird token             lan: only while the posture
#                                                  does not hold <SITE>_NB_API_TOKEN
#   12     declared ACL, NetBird service           yes       yes
#          identities, account baseline, mesh
#          CoreDNS (internet), portal,
#          onboarding, grant-keyd, add-ons
#   13     ACCEPTANCE GATE                         yes       yes
#   14     rescue nodes + cross-goatnet surfaces   yes       yes
#
# Account-seed is not its own phase: it runs inside site-bootstrap (phases 5
# and 6). The numbering previously had gaps and letters; the old-to-new
# table is in docs/operations/goatnet-standup.md §"Phase numbers".
#
# goatlite (ADR 1213, docs/design/goatlite.md): when <repo>/goatlite.yaml declares
# `preset: lite` and its lite.name is this site, the profile defaults to lan (to
# internet when goatlite.yaml declares lite.internet, goatlite.md §6), the
# obs leg is skipped, phase 1 opens with the render of the config surface from
# goatlite.yaml (ops/scripts/goatlite-render.py), and phase 4 mints the goatnet's
# own CA when none exists (mint-site-certs.sh --mint-ca-if-missing). Same fourteen
# phases, same plays, same gates — a preset, not a second procedure (doctrine P1).
#
# The observability leg — runs in PARALLEL with the phases above, automatically,
# when ALL of these hold ("identified and prepped"):
#   - group_vars/<site>_hosts/vars.yml says `has_obs_host: true`
#   - group_vars/<site>_obs_hosts/ exists (the obs host's config surface —
#     the config preflight is run over it too, with --obs)
#   - the inventory group [<site>_obs_hosts] has at least one host
#   - the host preflight can measure it (it is in the phase-1 host set)
# Otherwise the leg is skipped with a one-line reason and nothing else changes.
#
#   Leg    What                                        starts with   joins before
#   host   obs host baseline + LUKS (own nodes only),  phase 2       phase 12
#          then utility-host-bootstrap (Forgejo, the                (mirror)
#          pub/sub state-of-record PRIMARY tier)
#   stack  obs-host-bootstrap (Prometheus/Alertmanager phase 9       phase 13
#          /Grafana; needs obs_scrape_targets = the                 (gate)
#          core host's addresses, so it waits for 8)
#
# The host leg needs nothing from the core host, so it starts with phase 2 and
# runs while the core host goes through 2..6. The stack leg scrapes the core
# host's mesh (or LAN) addresses, so it starts once the router is up and runs
# while 10..12 deploy the service surface. Each leg logs to
# <state dir>/standup-<site>-obs-<leg>.log and is joined (rc checked,
# tail printed) at the phase that needs it. A failed leg fails the run at the
# join, with the same recovery message as a failed phase; the leg itself is
# idempotent, so the resume is `--obs-only` then `--from <join phase>`.
set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi

SITE="${1:-}"; [ -n "$SITE" ] || { echo "usage: goat-standup.sh <site-id> [--plan] [--from N] [--to N]"; exit 1; }
shift || true
PLAN=0; FROM=1; TO=14; PROFILE_OVERRIDE=""; SKIP_OBS=0; OBS_ONLY=0; PARALLEL=0; CHECK=0
while [ $# -gt 0 ]; do case "$1" in
  --plan) PLAN=1; shift ;;
  --profile) PROFILE_OVERRIDE="$2"; shift 2 ;;
  --from) FROM="$2"; shift 2 ;;
  --to)   TO="$2"; shift 2 ;;
  --skip-obs) SKIP_OBS=1; shift ;;
  --obs-only) OBS_ONLY=1; shift ;;
  --parallel) PARALLEL=1; shift ;;   # phase 12's independent steps in waves (ops/goatbuilder/standup-dag.yaml)
  --check) CHECK=1; shift ;;         # a goatlite runs phase 13 (acceptance) only when asked
  --offline) export GOAT_DEPLOY_OFFLINE=1; shift ;;   # an islanded site: judged against the cached trunk (C3)
  *) echo "unknown arg: $1"; exit 1 ;;
esac; done

# Phases are 1..14, whole numbers. A lettered id (6b, 7b, 12m, ...) is from the
# numbering previously : refuse it and name the phase it became.
phase_arg() {  # phase_arg <flag> <value>
  case "$2" in
    0b|0) echo "goat-standup: $1 $2 — phases start at 1 now; the gates are phase 1, the host baseline phase 2" >&2; exit 2 ;;
    6b) echo "goat-standup: $1 6b — phase 6b is now part of phase 5" >&2; exit 2 ;;
    7a|7b|7c) echo "goat-standup: $1 $2 — phase $2 is now part of phase 6" >&2; exit 2 ;;
    9b|9c) echo "goat-standup: $1 $2 — phase $2 is now part of phase 7" >&2; exit 2 ;;
    12m) echo "goat-standup: $1 12m — the first-login step is now phase 11" >&2; exit 2 ;;
  esac
  case "$2" in ''|*[!0-9]*) echo "goat-standup: $1 $2 — want a phase number 1..14 (docs/operations/goatnet-standup.md §\"Phase numbers\")" >&2; exit 2 ;; esac
  if [ "$2" -lt 1 ] || [ "$2" -gt 14 ]; then echo "goat-standup: $1 $2 — want a phase number 1..14" >&2; exit 2; fi
}
phase_arg --from "$FROM"; phase_arg --to "$TO"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# A declared value as one shell word in a planned command, the way goatbuilder's
# sh_quote / sh_word write it (standup.rs): sh_quote always single-quotes, each '
# written as '\''; sh_word leaves a value of letters, digits and _ . / : = @ % + , -
# as it is, keeps a leading ~/ meaning the home directory, and quotes the rest.
sh_quote() { local q="'\\''"; printf "'%s'" "${1//\'/$q}"; }
sh_word() {
  local plain='^[A-Za-z0-9_./:=@%+,-]+$'
  # shellcheck disable=SC2088  # a literal ~/ on purpose: the planned word keeps it for the phase shell to expand
  case "$1" in
    "~/"*) if [[ "${1#\~/}" =~ $plain ]]; then printf '%s' "$1"; else printf '~/%s' "$(sh_quote "${1#\~/}")"; fi ;;
    *) if [[ "$1" =~ $plain ]]; then printf '%s' "$1"; else sh_quote "$1"; fi ;;
  esac
}
GROUP="${SITE//-/_}_hosts"

# goatlite (ADR 1213): is THIS site the one <repo>/goatlite.yaml declares? Read as
# YAML with python3 + PyYAML, the way gv_value reads group_vars below, and judged
# the way `goatbuilder standup` judges it (standup.rs resolve_lite): no file, or a
# file that names another goatnet, means "not lite" and changes nothing; a file that
# cannot be read as YAML refuses the run, and so does one that names this site and
# does not validate (ops/goatbuilder/validate.py).
LITE=0; LITE_INTERNET_AWS=""
if [ -f "$REPO/goatlite.yaml" ]; then
  lite_rc=0
  lite_name="$(python3 - "$REPO/goatlite.yaml" <<'PY'
import sys
try:
    import yaml
except ImportError:
    print("goat-standup: reading goatlite.yaml needs PyYAML (pip install pyyaml)", file=sys.stderr)
    sys.exit(3)
try:
    d = yaml.safe_load(open(sys.argv[1])) or {}
except Exception as e:
    print(f"goat-standup: goatlite.yaml is not readable as YAML: {e}", file=sys.stderr)
    sys.exit(3)
if isinstance(d, dict) and d.get("preset") == "lite":
    lite = d.get("lite") or {}
    # name on the first line, the declared administrator on the second, and on the
    # third the AWS profile of a goatlite grown to the internet (lite.internet, its
    # aws_profile or else the name), empty for a LAN goatlite
    print(str(lite.get("name", "")).strip())
    print(str(lite.get("admin", "")).strip())
    inet = lite.get("internet")
    print(str((inet.get("aws_profile") or lite.get("name", "")) if isinstance(inet, dict) else "").strip())
PY
)" || lite_rc=$?
  [ "$lite_rc" = 0 ] || { echo "goat-standup: refusing to plan: fix $REPO/goatlite.yaml (or move it aside), then re-run" >&2; exit 2; }
  lite_admin="$(printf '%s\n' "$lite_name" | sed -n 2p)"
  lite_aws="$(printf '%s\n' "$lite_name" | sed -n 3p)"
  lite_name="$(printf '%s\n' "$lite_name" | sed -n 1p)"
  if [ -n "$lite_name" ] && [ "$lite_name" = "$SITE" ]; then
    python3 "$REPO/ops/goatbuilder/validate.py" "$REPO/goatlite.yaml" >&2 \
      || { echo "goat-standup: goatlite.yaml does not validate; fix it, then re-run" >&2; exit 2; }
    LITE=1; LITE_INTERNET_AWS="$lite_aws"
  fi
fi

# Phase 12's dependency graph, the same file `goatbuilder standup` reads
# (ops/goatbuilder/standup-dag.yaml). Every plan prints each step's needs; with
# --parallel the steps run in waves. Without python3 + PyYAML the graph is empty
# and every step runs serially, as it always did.
DAG_N=0
eval "$(python3 "$REPO/ops/goatbuilder/standup-dag.py" --shell 2>/dev/null)" || DAG_N=0
dag_index() {  # dag_index <label> -> index into DAG_*, or nothing
  local l="$1" i=0
  l="${l#\[internet\] }"; l="${l#\[lan\] }"; l="${l#\[joiner\] }"
  while [ "$i" -lt "$DAG_N" ]; do
    case "$l" in "${DAG_LABEL[$i]}"*) echo "$i"; return 0 ;; esac
    i=$((i+1))
  done
}
Q_N=0
SITE_UPPER="$(printf '%s' "${SITE//-/_}" | tr '[:lower:]' '[:upper:]')"  # env-var prefix, e.g. DEMO
INV=(-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini)
OBS_GROUP="${SITE//-/_}_obs_hosts"
# Where this machine keeps the project's local state (run logs, CA keys): the
# deploy.state_dir the checkout declares in ops/goatbuilder/goatbuilder.yaml
# (read by project-setting.sh), else ~/.goat/state. The builder reads the same
# (crate::settings), so a run resumed with the other avenue finds its files.
# STATE_DECL keeps the `~` for the commands a phase prints and runs; STATE_DIR
# is the same path expanded, for what this script opens itself.
# Only an empty answer with exit 0 means undeclared; any other exit of
# project-setting.sh (its refusal, exit 3, or a file or value it cannot read)
# stops the run with its message kept.
ps_rc=0; STATE_DECL="$("$REPO/ops/scripts/project-setting.sh" --repo "$REPO" deploy state_dir)" || ps_rc=$?
[ "$ps_rc" = 0 ] || exit 2
# shellcheck disable=SC2088  # the literal `~` is kept on purpose, see above
[ -n "$STATE_DECL" ] || STATE_DECL='~/.goat/state'
case "$STATE_DECL" in \~/*) STATE_DIR="$HOME/${STATE_DECL#\~/}" ;; *) STATE_DIR="$STATE_DECL" ;; esac
mkdir -p "$STATE_DIR"
# The audit log the signing tools append to: ca-audit.jsonl beside the state
# directory, passed as --audit-log so the file is the project's and not a
# default built into the tool. The builder derives the same path
# (ops/scripts/project-setting.sh ca-audit-log prints it).
case "${STATE_DECL%/}" in */*) AUDIT_DECL="${STATE_DECL%/}"; AUDIT_DECL="${AUDIT_DECL%/*}/ca-audit.jsonl" ;; *) AUDIT_DECL=./ca-audit.jsonl ;; esac

# Is there an obs host to build, and is it identified + prepped? All four
# conditions, or the leg is skipped and says why. A declared-but-absent obs
# host is NOT an error here: has_obs_host: true with no [<site>_obs_hosts] host
# yet is the normal state of a site whose third box has not arrived — the
# acceptance gate's obs rows then read N/A, not PASS.
OBS=0; OBS_WHY=""
if [ "$LITE" = 1 ]; then OBS_WHY="goatlite (ADR 1213): one box, no obs host";
elif [ "$SKIP_OBS" = 1 ]; then OBS_WHY="--skip-obs";
elif ! grep -qE '^[[:space:]]*has_obs_host:[[:space:]]*true' "$REPO/ops/ansible/group_vars/$GROUP/vars.yml" 2>/dev/null; then
  OBS_WHY="has_obs_host is not true in group_vars/$GROUP/vars.yml"
elif [ ! -d "$REPO/ops/ansible/group_vars/$OBS_GROUP" ]; then
  OBS_WHY="has_obs_host: true but no config surface at group_vars/$OBS_GROUP/ (scaffold: standup-config-preflight.py $OBS_GROUP --scaffold-plan)"
elif [ -z "$(python3 "$REPO/ops/scripts/inventory-hosts.py" "$OBS_GROUP" 2>/dev/null)" ]; then
  OBS_WHY="has_obs_host: true but no host in inventory group [$OBS_GROUP] yet"
else OBS=1; fi
if [ "$OBS_ONLY" = 1 ] && [ "$OBS" = 0 ]; then echo "--obs-only: no obs leg to run — $OBS_WHY"; exit 1; fi
# Refuse here rather than at phase 2, after the core leg has started beside it.
if [ "$OBS" = 1 ] && [ ! -f "$REPO/ops/ansible/utility-host-bootstrap.yml" ]; then
  echo "has_obs_host: true, but ops/ansible/utility-host-bootstrap.yml is not in this tree, so the obs leg cannot run. Re-run with --skip-obs to stand up the site without its obs host." >&2
  exit 1
fi
# The ansible phases run under an aws-vault profile. It defaults to the
# platform's hosted account, but a sovereign goatnet owns its own account —
# so it is overridable (GOAT_AWS_PROFILE) rather than baked in. A hardcoded
# profile here was one of the reasons the guides route around this driver.
# GOAT_AWS_PROFILE, then the project's own deploy.aws_profile, then the name the
# AWS tools fall back to themselves. The builder reads the same three
# (crate::settings, the portability track): this driver held one organisation's
# profile name as its default, which is what that track exists to remove. No
# python3, no PyYAML or no declaration all mean the tools' own fallback. A goatlite
# grown to the internet declares its own (lite.internet.aws_profile, else its name),
# read after GOAT_AWS_PROFILE and before the project's.
AWS_PROFILE_STANDUP="${GOAT_AWS_PROFILE:-$LITE_INTERNET_AWS}"
if [ -z "$AWS_PROFILE_STANDUP" ] && [ -f "$REPO/ops/goatbuilder/goatbuilder.yaml" ]; then
  if python3 -c 'import yaml' >/dev/null 2>&1; then
    # With PyYAML the file is read as YAML, every form of it, as goatbuilder reads it.
    AWS_PROFILE_STANDUP="$(python3 - "$REPO/ops/goatbuilder/goatbuilder.yaml" <<'PY' 2>/dev/null || true
import sys, yaml
try:
    d = yaml.safe_load(open(sys.argv[1])) or {}
except Exception:
    sys.exit(0)
if isinstance(d, dict):
    print(str((d.get("deploy") or {}).get("aws_profile", "")).strip())
PY
)"
  else
    # Without it, read as deploy.state_dir is (project-setting.sh): any exit but 0
    # stops the run with its message kept.
    ps_rc=0; AWS_PROFILE_STANDUP="$("$REPO/ops/scripts/project-setting.sh" --repo "$REPO" deploy aws_profile)" || ps_rc=$?
    [ "$ps_rc" = 0 ] || exit 2
  fi
fi
AWS_PROFILE_STANDUP="${AWS_PROFILE_STANDUP:-default}"
# Fresh source (contract C3): once, before phase 1 touches a host. Every play
# then goes through ansible-preflight.sh judged against the same trunk commit,
# so a merge that lands during a long standup does not stop it half way.
if [ "$PLAN" = 0 ]; then
  GOAT_FRESH_SOURCE_TRUNK="$("$REPO/ops/scripts/fresh-source.sh" --repo "$REPO")" \
    || { echo "goat-standup: refusing before any host is touched: this checkout is not current with the source of record (above)" >&2; exit 1; }
  export GOAT_FRESH_SOURCE_TRUNK
fi

# Topology profile (lan | internet). Explicit --profile wins; otherwise derive
# from the config: an internet-enrollable site delivers its dashboard cert via
# ACME, a LAN-only site hand-mints it. A site with no config surface yet is
# treated as lan for --plan purposes (the phase-1 gate refuses for real anyway).
PROFILE="$PROFILE_OVERRIDE"
if [ -z "$PROFILE" ]; then
  if [ "$LITE" = 1 ] && [ -n "$LITE_INTERNET_AWS" ]; then
    PROFILE=internet   # goatlite.yaml declares lite.internet: grown out of lite (goatlite.md §6)
  elif [ "$LITE" = 1 ]; then
    PROFILE=lan   # goatlite is a LAN goatnet (ADR 1213 D1); its surface may not be rendered yet
  elif grep -qE '^[[:space:]]*dashboard_cert_delivery_mode:[[:space:]]*acme' \
        "$REPO/ops/ansible/group_vars/$GROUP/vars.yml" 2>/dev/null; then
    PROFILE=internet
  else
    PROFILE=lan
  fi
fi
case "$PROFILE" in lan|internet) ;; *) echo "bad --profile '$PROFILE' (want lan|internet)"; exit 1 ;; esac
# A LAN goatlite reaches its one box directly at its inventory address and declares
# no cloud at all, so no play in the run needs AWS. with-aws defaults to
# aws-vault whenever aws-vault is installed (ADR 1204), so without this every
# play asked the operator's keychain for credentials it would never use, once
# per play. A goatlite on the internet profile reaches its relays through AWS
# (SSM, Secrets Manager), so it keeps the profile. The builder does the same
# (crates/goatbuilder/src/standup.rs).
AWS_MODE_STANDUP=""
[[ "${LITE}" == 1 && "$PROFILE" == lan ]] && AWS_MODE_STANDUP="GOAT_AWS_MODE=none "
ANSIBLE_PRE="OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ${AWS_MODE_STANDUP}../scripts/with-aws $(sh_word "$AWS_PROFILE_STANDUP") -- ../scripts/ansible-preflight.sh"

# Founder or joiner (ADR 1084). goatnet_founder:false in the site's group_vars
# makes site-bootstrap auto-skip the account seed (the account arrives by Spock
# replication instead); this driver then adds the three-pass join + founder-side
# mirror below. Read it from config, never the command line — setting it wrong on
# a joiner seeds a second account into a replicated schema. join_founder_site is
# the founder's site-id (the goatnet being joined) — the peer to seed
# from and to mirror onto.
JOINER=0; JOIN_FOUNDER_SITE=""; JOIN_FOUNDER_GROUP=""
GV_FILE="$REPO/ops/ansible/group_vars/$GROUP/vars.yml"
if grep -qE '^[[:space:]]*goatnet_founder:[[:space:]]*false' "$GV_FILE" 2>/dev/null; then
  JOINER=1
  # `|| true` is not decoration. Under `set -e` a command substitution whose
  # pipeline exits non-zero kills the script, so a joiner site that declares
  # goatnet_founder:false and no join_founder_site made this driver EXIT
  # silently, printing no phases and returning success. A driver that does nothing must never look like a
  # driver that did everything.
  JOIN_FOUNDER_SITE="$(grep -E '^[[:space:]]*join_founder_site:' "$GV_FILE" 2>/dev/null | head -1 | awk '{print $2}' | tr -d '"'\''' || true)"
  [ -n "$JOIN_FOUNDER_SITE" ] && JOIN_FOUNDER_GROUP="${JOIN_FOUNDER_SITE//-/_}_hosts"
fi

# The NetBird ACL this standup applies and checks (ADR 1185). A founder applies
# its own declared ACL; a joiner shares the founder's NetBird account through
# replication, so it applies nothing and is checked against the founder's ACL.
# The mgmt API is the site's public_mgmt_fqdn (a joiner's is the founder's).
ACL_SITE="$SITE"; [ "$JOINER" = 1 ] && ACL_SITE="$JOIN_FOUNDER_SITE"
# The ACL site names the token variable phase 12 reads (<SITE>_NB_API_TOKEN), so a
# founder site that cannot make a variable name is refused (standup.rs read_site).
ACL_UPPER="$(printf '%s' "${ACL_SITE//-/_}" | tr '[:lower:]' '[:upper:]')"
if [ -n "$ACL_SITE" ] && ! [[ "$ACL_UPPER" =~ ^[A-Z0-9_]+$ ]]; then
  # A joiner's ACL site is its founder's (join_founder_site); a founder's is itself.
  if [ "$JOINER" = 1 ]; then ACL_SOURCE="join_founder_site '$ACL_SITE' in group_vars/$GROUP/vars.yml"; else ACL_SOURCE="the site '$ACL_SITE'"; fi
  echo "goat-standup: $ACL_SOURCE is not a site id: phase 12 reads the token variable ${ACL_UPPER}_NB_API_TOKEN named after it, so it may hold only letters, digits, - and _" >&2
  exit 2
fi
MGMT_FQDN="$(grep -E '^[[:space:]]*public_mgmt_fqdn:' "$GV_FILE" 2>/dev/null | head -1 | awk '{print $2}' | tr -d '"'\''' || true)"
# The group_vars values above as one shell word each in a planned command
# (sh_quote / sh_word, as standup.rs writes them).
ACL_SITE_Q="$(sh_quote "$ACL_SITE")"
MGMT_URL_W="$(sh_word "https://$MGMT_FQDN")"
JFS_Q="$(sh_quote "$JOIN_FOUNDER_SITE")"; JFG_Q="$(sh_quote "$JOIN_FOUNDER_GROUP")"; JFG_W="$(sh_word "$JOIN_FOUNDER_GROUP")"
RELAY_HOSTS_W="$(sh_word "inventory/hosts-$JOIN_FOUNDER_SITE.ini")"
RELAY_ALIAS_W="$(sh_word "inventory/$JOIN_FOUNDER_SITE-goat-relay-alias.ini")"
FOUNDER_SITE_W="$(sh_word "founder_site=$JOIN_FOUNDER_SITE")"
RELAY_MISSING_Q="$(sh_quote "founder relay inventory inventory/hosts-$JOIN_FOUNDER_SITE.ini missing — generate it first: inventory/generate-relays.sh $JOIN_FOUNDER_SITE terraform-$JOIN_FOUNDER_SITE")"

# intra_site_pair: an A/B core pair runs A->B streaming replication (inside
# site-bootstrap.yml),
# so the standby IS a replicated copy — a durability story on its own. The
# pass-1 durability override below must NOT force independent backups
# (pgBackRest) on such a site: doing so runs pgBackRest on a goatnet that defers
# it, and stanza-create then fails on the standby's rebuilt cluster. The
# durability guard in site-bootstrap.yml now credits intra_site_pair directly.
INTRA_PAIR=0
grep -qE '^[[:space:]]*intra_site_pair:[[:space:]]*true' "$GV_FILE" 2>/dev/null && INTRA_PAIR=1

# A value store's values are read afresh per phase in a child process
# (standup-phase-run); a reference store keeps this shell's own eval, as before.
phase_in_subshell() {
  case "${GOAT_SECRET_STORE:-$(python3 "$REPO/ops/secrets/posture.py" store 2>/dev/null || echo 1password)}" in
    env-file|sops|git-crypt) return 0 ;;
  esac
  return 1
}

step() {   # step <n> <kind> <label> <command...>
  local n="$1" kind="$2" label="$3"; shift 3
  [ "$n" -lt "$FROM" ] && return 0
  [ "$n" -gt "$TO" ] && return 0
  printf '\n=== Phase %s [%s] %s ===\n' "$n" "$kind" "$label"
  printf '    %s\n' "$*"
  local di=""
  if [ "$n" = 12 ] && [ "$DAG_N" -gt 0 ]; then
    di="$(dag_index "$label")"
    [ -n "$di" ] && printf '    needs: %s\n' "${DAG_NEEDS[$di]:-nothing}"
  fi
  if [ "$PLAN" = 1 ]; then return 0; fi
  if [ "$PARALLEL" = 1 ] && [ -n "$di" ]; then
    Q_ID[$Q_N]="${DAG_ID[$di]}"; Q_NEEDS[$Q_N]="${DAG_NEEDS[$di]}"; Q_LABEL[$Q_N]="$label"; Q_CMD[$Q_N]="$*"
    Q_N=$((Q_N+1)); echo "    (queued for the phase-12 waves)"; return 0
  fi
  case "$kind" in
    MANUAL)
      echo "    ^ OPERATOR STEP — run the above, then re-run with --from $((n+1))"
      exit 0 ;;
    *)
      # Run the phase. On failure, do not just die on errexit with a bare
      # non-zero — tell the operator what state the site is in, where the
      # phase's end-state and troubleshooting live, and the one command that
      # resumes. This is the whole point of a driver over a wall of commands:
      # a stuck run is recoverable without reverse-engineering the script.
      # Capture the status BEFORE negating it. `if ! cmd; then rc=$?` reads the
      # status of the NEGATION, which is 0 exactly when the command failed — so
      # every failed phase reported "FAILED — exit 0", and the one number an
      # operator would use to tell a refusal from a crash was always wrong.
      # Every run measures itself: the phase's wall-clock goes to the screen and
      # to a per-site timings file, so the one-hour target is a number read
      # from runs, not a guess (docs/design/goatbuilder-speed.md).
      # Through ops/scripts/standup-phase-run, as the builder does: an empty
      # $(printenv NAME) refuses and names NAME; under a value store each phase
      # reads the store afresh, so phase 3's values reach the phases after it.
      # Otherwise the phase still runs in this shell, as it always has.
      local t0; t0=$(date +%s)
      if phase_in_subshell; then "$REPO/ops/scripts/standup-phase-run" "$*"; rc=$?
      else "$REPO/ops/scripts/standup-phase-run" --check "$*" && eval "$*"; rc=$?; fi
      local dt=$(( $(date +%s) - t0 ))
      printf '    phase %s took %ss (exit %s)\n' "$n" "$dt" "$rc"
      printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%Y%m%dT%H%M%SZ)" "$n" "$label" "$dt" "$rc" >> "$STATE_DIR/standup-$SITE-timings.tsv" 2>/dev/null || true
      if [ "$rc" -ne 0 ]; then
        cat >&2 <<RECOVER

  !! Phase $n ("$label") FAILED — exit $rc.
     No later phase has run; the site is left partway through phase $n.
     To get unstuck and reach this phase's end state:
       1. Read the error above.
       2. Open docs/operations/goatnet-standup.md: "The run" table has this
          phase's "Worked when" (the end state you are aiming for), and the
          Troubleshooting table lists the common failures by symptom + fix.
       3. Fix the cause, then resume — every phase is idempotent:
            ops/scripts/goat-standup.sh $SITE --from $n $( [ "$PROFILE" != lan ] && echo "--profile $PROFILE" )
RECOVER
        exit "$rc"
      fi ;;
  esac
}

# ---- phase 12 in waves (--parallel) ----------------------------------------
# Each queued step's wave is one more than the latest wave among the queued steps
# it needs (a need the site never queued is met). A wave's steps run together,
# each logged to its own file; the run stops after a wave in which any failed,
# naming every failure, so one resume fixes them all.
dag_flush() {
  [ "$Q_N" -gt 0 ] || return 0
  local i j changed max=0 w
  i=0; while [ "$i" -lt "$Q_N" ]; do Q_W[$i]=1; i=$((i+1)); done
  changed=1
  while [ "$changed" = 1 ]; do
    changed=0; i=0
    while [ "$i" -lt "$Q_N" ]; do
      for need in ${Q_NEEDS[$i]}; do
        j=0
        while [ "$j" -lt "$Q_N" ]; do
          if [ "${Q_ID[$j]}" = "$need" ] && [ "${Q_W[$i]}" -le "${Q_W[$j]}" ]; then Q_W[$i]=$(( Q_W[$j] + 1 )); changed=1; fi
          j=$((j+1))
        done
      done
      [ "${Q_W[$i]}" -gt "$max" ] && max="${Q_W[$i]}"
      i=$((i+1))
    done
  done
  w=1
  while [ "$w" -le "$max" ]; do
    local ids="" failed=""
    i=0; while [ "$i" -lt "$Q_N" ]; do [ "${Q_W[$i]}" = "$w" ] && ids="$ids ${Q_ID[$i]}"; i=$((i+1)); done
    printf '\n=== Phase 12 wave %s of %s:%s ===\n' "$w" "$max" "$ids"
    i=0
    while [ "$i" -lt "$Q_N" ]; do
      if [ "${Q_W[$i]}" = "$w" ]; then
        local log="$STATE_DIR/standup-$SITE-12-${Q_ID[$i]}.log" rcf="$STATE_DIR/standup-$SITE-12-${Q_ID[$i]}.rc"
        rm -f "$rcf"
        ( t0=$(date +%s); "$REPO/ops/scripts/standup-phase-run" "${Q_CMD[$i]}"; rc=$?; echo "$rc $(( $(date +%s) - t0 ))" > "$rcf" ) >"$log" 2>&1 &
        Q_PID[$i]=$!
      fi
      i=$((i+1))
    done
    i=0
    while [ "$i" -lt "$Q_N" ]; do
      if [ "${Q_W[$i]}" = "$w" ]; then
        wait "${Q_PID[$i]}" || true
        local rcf="$STATE_DIR/standup-$SITE-12-${Q_ID[$i]}.rc" r="1 0"
        [ -f "$rcf" ] && r="$(cat "$rcf")"
        local rc="${r%% *}" dt="${r#* }"
        printf '    phase 12 %s took %ss (exit %s)\n' "${Q_ID[$i]}" "$dt" "$rc"
        printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%Y%m%dT%H%M%SZ)" "12" "${Q_LABEL[$i]}" "$dt" "$rc" >> "$STATE_DIR/standup-$SITE-timings.tsv" 2>/dev/null || true
        [ "$rc" = 0 ] || failed="$failed ${Q_ID[$i]}"
      fi
      i=$((i+1))
    done
    if [ -n "$failed" ]; then
      for id in $failed; do
        printf '\n  !! phase 12 %s FAILED — last lines of %s:\n' "$id" "$STATE_DIR/standup-$SITE-12-$id.log" >&2
        tail -n 15 "$STATE_DIR/standup-$SITE-12-$id.log" | sed 's/^/    | /' >&2
      done
      cat >&2 <<RECOVER

  !! Phase 12 wave $w of $max FAILED:$failed. Steps in this wave that passed stay
     done; no later wave has run. Every phase-12 step is idempotent: fix each
     failure named above, then resume:
       ops/scripts/goat-standup.sh $SITE --from 12 --parallel $( [ "$PROFILE" != lan ] && echo "--profile $PROFILE" )
RECOVER
      exit 1
    fi
    w=$((w+1))
  done
  Q_N=0
}

# ---- the obs leg -----------------------------------------------------------
# Three sub-legs, each a plain command string like a phase. They run in the
# background (leg_start) and are joined (leg_join) at the phase that needs
# them; --obs-only runs all three in the foreground and exits. Every leg is
# idempotent, so a re-run is always safe.
OBS_LEG_BASELINE="cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} mgmt-host-baseline.yml -e target=$OBS_GROUP -e site_id=$SITE && $ANSIBLE_PRE ${INV[*]} tang-server.yml -e target=$OBS_GROUP && $ANSIBLE_PRE ${INV[*]} mgmt-host-luks.yml -e target=$OBS_GROUP -e site_id=$SITE"
# utility-host-bootstrap pins the docker data-root on the NVMe itself (its
# first play step, the same docker-engine role obs-host-bootstrap runs) BEFORE Forgejo,
# so Forgejo's containers are born on the encrypted root and a later data-root
# move cannot orphan them.
OBS_LEG_UTILITY="cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} utility-host-bootstrap.yml -e target=$OBS_GROUP"
OBS_LEG_STACK="cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} obs-host-bootstrap.yml -e target=$OBS_GROUP"

leg_start() {  # leg_start <leg> <label> <command...>  — background, logged
  local leg="$1" label="$2"; shift 2
  local log="$STATE_DIR/standup-$SITE-obs-$leg.log"
  printf '\n=== Observability leg "%s" [A ∥] %s ===\n    %s\n    (log: %s)\n' "$leg" "$label" "$*" "$log"
  [ "$PLAN" = 1 ] && return 0
  ( "$REPO/ops/scripts/standup-phase-run" "$*" ) >"$log" 2>&1 &
  eval "LEG_PID_$leg=$!"
}
leg_join() {   # leg_join <leg> <join-phase>  — wait, print the tail, fail like a phase
  local leg="$1" at="$2" pid rc
  [ "$PLAN" = 1 ] && { printf '\n=== join observability leg "%s" before phase %s ===\n' "$leg" "$at"; return 0; }
  eval "pid=\${LEG_PID_$leg:-}"; [ -n "$pid" ] || return 0
  printf '\n=== join observability leg "%s" before phase %s (waiting) ===\n' "$leg" "$at"
  wait "$pid"; rc=$?
  tail -n 25 "$STATE_DIR/standup-$SITE-obs-$leg.log" | sed 's/^/    | /'
  if [ "$rc" -ne 0 ]; then
    cat >&2 <<RECOVER

  !! Observability leg "$leg" FAILED — exit $rc (full log: $STATE_DIR/standup-$SITE-obs-$leg.log).
     The core phases up to $at have run; phase $at needs this leg, so the run stops here.
     Fix the cause (the log names it; obs_scrape_targets unset is the usual one for the stack leg),
     then: ops/scripts/goat-standup.sh $SITE --obs-only && ops/scripts/goat-standup.sh $SITE --from $at
RECOVER
    exit "$rc"
  fi
}
# ---- the image pull ----------------------------------------------------------
# Every pinned image the site's hosts deploy, pulled on those hosts in parallel
# (ops/ansible/standup-image-pull.yml; docs/design/goatlite.md §11 measure 2).
# Started after phase 2, when the container engine and the registry login are in
# place, and joined before phase 5, the first phase that runs an image. Only when
# phase 5 is in the run's range: a resume past it pulls inside its plays as before.
PULL_LOG="$STATE_DIR/standup-$SITE-images.log"; PULL_PID=""
pull_start() {  # pull_start <label> <command...>  — background, logged
  local label="$1"; shift
  [ "$FROM" -le 5 ] && [ "$TO" -ge 5 ] || return 0
  printf '\n=== Image pull [A ∥] %s ===\n    %s\n    (log: %s)\n' "$label" "$*" "$PULL_LOG"
  [ "$PLAN" = 1 ] && { PULL_PID=planned; return 0; }
  ( "$REPO/ops/scripts/standup-phase-run" "$*" ) >"$PULL_LOG" 2>&1 &
  PULL_PID=$!
}
pull_join() {   # pull_join <join-phase>  — wait, print the tail, fail like a phase
  local at="$1" rc
  [ -n "$PULL_PID" ] || return 0
  [ "$PLAN" = 1 ] && { printf '\n=== join the image pull before phase %s ===\n' "$at"; return 0; }
  printf '\n=== join the image pull before phase %s (waiting) ===\n' "$at"
  wait "$PULL_PID"; rc=$?; PULL_PID=""
  tail -n 25 "$PULL_LOG" | sed 's/^/    | /'
  if [ "$rc" -ne 0 ]; then
    cat >&2 <<RECOVER

  !! Image pull FAILED — exit $rc (full log: $PULL_LOG).
     Phase $at and every later phase run these images, so the run stops here; the
     log names each image that could not be pulled. A private package needs
     GHCR_READ_TOKEN on the laptop (phase 2 logs the box in with it); a pin naming
     an image the registry no longer has needs a new pin in ops/pins/.
     Fix the cause, then resume: ops/scripts/goat-standup.sh $SITE --from $at $( [ "$PROFILE" != lan ] && echo "--profile $PROFILE" )
RECOVER
    exit "$rc"
  fi
}
run_obs_leg_fg() {  # --obs-only: the three legs, in order, foreground, phase-style
  step 2 A "[obs] observability host: baseline + LUKS" "$OBS_LEG_BASELINE"
  step 2 A "[obs] observability host: utility (Forgejo)" "$OBS_LEG_UTILITY"
  step 9 A "[obs] observability host: stack (Prometheus/Alertmanager/Grafana)" "$OBS_LEG_STACK"
}

echo "goat-standup: site=$SITE  group=$GROUP  profile=$PROFILE  obs=$( [ "$OBS" = 1 ] && echo "$OBS_GROUP (parallel leg)" || echo "none — $OBS_WHY" )  $( [ "$PLAN" = 1 ] && echo '(PLAN — nothing executes)')"
if [ "$OBS_ONLY" = 1 ]; then FROM=1; TO=14; run_obs_leg_fg; echo; echo "obs leg complete for $SITE."; exit 0; fi

# A site whose config surface does not exist yet is the NORMAL state for a new
# goatnet, not an error — but say so, because every phase below assumes it. The
# phase-1 preflight refuses for real at execute time; this is the early warning so a
# --plan reader is not misled into thinking the config half is done.
# Printed UNINDENTED on purpose: the parity gate (ops/goatbuilder/parity-standup.sh)
# keeps every line that starts with four spaces as a command line, and the binary
# prints this same fact as a banner row. A goatlite before its first render is
# exactly this state, and its render is phase 1's first gate.
if [ ! -d "$REPO/ops/ansible/group_vars/$GROUP" ]; then
  cat <<BANNER

goat-standup: !! ops/ansible/group_vars/$GROUP does not exist.
goat-standup: This goatnet has no config surface yet, so nothing below can run. Author it
goat-standup: first — it is a FILE INVENTORY (~30 files), not a single vars.yml. Start from
goat-standup: the reference env and re-thread every value:
goat-standup:   ops/scripts/standup-config-preflight.py $GROUP --scaffold-plan
goat-standup: You also need an inventory group [$GROUP] and a cert manifest
goat-standup: (ops/enrollment/manifests/$SITE-certs.yaml — copy TEMPLATE-certs.yaml).
goat-standup: A goatlite (ADR 1213) renders all of that from goatlite.yaml in phase 1.
goat-standup: Full pre-phase list: docs/operations/goatnet-standup.md §"Before phase 1".

BANNER
fi

# goatlite: the derivation step first (docs/design/goatlite.md §4). The learner
# wrote four facts; the engines below read a file set. Render it — idempotently,
# never over a file the learner has edited — then gate it like any other site.
if [ "$LITE" = 1 ]; then
step 1 GATE "goatlite: render the config surface from goatlite.yaml (idempotent; an edited file is reported, never overwritten)" \
  "python3 '$REPO/ops/scripts/goatlite-render.py' --site $SITE"
# Then the env-file store every later phase reads (it needs the rendered groups);
# standup-phase-run runs these two lines without loading the store.
step 1 GATE "goatlite: create the local secrets store (strong random values; a value already there is kept)" \
  "python3 '$REPO/ops/secrets/posture.py' init --if-env-file --env $SITE"
fi

# Pre-flight gate (fail-closed): config surface + values ready before any host.
step 1 GATE "config preflight (fail-closed)" \
  "python3 '$REPO/ops/scripts/standup-config-preflight.py' $GROUP $( [ "$OBS" = 1 ] && echo "--obs $OBS_GROUP" )"

# Every input the run will reach, scanned at once (sink 1 of
# docs/design/goatbuilder-speed.md): the role asserts, `| mandatory` and the
# secrets the reached plays template, read from the plays themselves, so a
# missing variable is found here in seconds instead of one per failed run.
# Up to the manual step only. Phase 11 issues the NetBird service token that
# phase 12 reads, so a scan of all fourteen phases here refused every first run
# of a new goatnet: it asked for a secret the run itself makes. The phase-12
# gate scans what follows, where it is a real blocker. The builder does the
# same (crates/goatbuilder/src/standup.rs).
step 1 GATE "every input the phases before the manual step will reach is set (one static scan of the plays; changes nothing)" \
  "python3 '$REPO/ops/scripts/standup-required-inputs.py' $SITE --profile $PROFILE --to 11"

# Establish the env-door: render this goatnet's transport rows (env-door.sh arm +
# doors.ini + doors-groupvars.ini) from the door registry, so it is deployable
# off-profile — without flipping the laptop's shared netbird profile — by
# construction, not by a hand-rolled ad-hoc inventory. Fail-closed: if the
# goatnet has no doors.yaml entry the generator stops and says what to declare.
# ADR 1117 / docs/design/env-doors.md. The finish gate re-asserts it (--check).
step 1 GATE "the NetBird ACL for ${ACL_SITE:-?} is declared, with no Default All->All (ADR 1185)" \
  "python3 '$REPO/ops/scripts/netbird-policy-posture.py' $ACL_SITE_Q --declared"
step 1 GATE "establish the env-door (off-profile transport)" \
  "python3 '$REPO/ops/scripts/generate-door-inventory.py' $SITE"

# Phase 1, last gate — MEASURE THE HOSTS. The gate above reads group_vars and contacts no
# host, by design; this reads the hosts and no group_vars. Together they are
# phase 1, and the split is deliberate: a configuration can be complete and
# correct for a machine that does not match what it assumes.
#
# Every invariant it checks is one this repo already documents and that currently
# fails LATE — during a play, after deployment, or not at all: both container
# data-roots on the Tier-3 NVMe, non-local bind on both families,
# chrony-presync ordered before chrony and docker. It measures and
# changes nothing.
#
# Skipped automatically when the site has no hosts in inventory yet, which is the
# normal state before phase 1 has ever run.
#
# It is told what this run already knows: phase 2 still ahead (--baseline-pending
# when FROM is 1 or 2: the sysctls, presync and container engine phase 2 sets
# are then "set by phase 2 of this run", not failures) and the site's own
# disk-encryption declaration (luks_volumes_mgmt_host: [] in its vars.yml, the
# goatlite VM shape, makes an unencrypted root the declared exclusion). The
# binary computes the same two from the same facts (parity gate).
step 1 GATE "host preflight (measure the targets; changes nothing)" \
  "hosts=\$(python3 '$REPO/ops/scripts/inventory-hosts.py' $GROUP $( [ "$OBS" = 1 ] && echo "$OBS_GROUP" )); \
   enc=\$(grep -qE '^luks_volumes_mgmt_host:[[:space:]]*\\[\\]' '$REPO/ops/ansible/group_vars/$GROUP/vars.yml' 2>/dev/null && echo '--disk-encryption declared-off'); \
   if [ -z \"\$hosts\" ]; then echo '    no hosts in inventory for $GROUP yet — nothing to measure (normal before phase 1)'; \
   else python3 '$REPO/ops/scripts/standup-host-preflight.py' \$hosts \$enc$( [ "$FROM" -le 2 ] && echo ' --baseline-pending' ) \
        --inventory '$REPO/ops/ansible/inventory/mgmt-hosts.ini' \
        --inventory '$REPO/ops/ansible/inventory/mgmt-hosts-mesh.ini'; fi"

# Phase 2. Host baseline + LUKS (on-prem chain; seed-ssh-key first, done once).
# `-e target=$GROUP` is load-bearing, not decoration: both playbooks
# default to the umbrella group `mgmt_hosts` — every goatnet's mgmt hosts,
# production included — and mgmt-host-baseline.yml never reads site_id at all.
# tang-server.yml runs BEFORE mgmt-host-luks.yml: the LUKS role binds
# each volume to its siblings' Tang servers, and previously nothing in
# this chain installed those servers — they were set up by hand, and the hand
# config carried the boot ordering cycle that can lock a goatnet for days.
# Hosts declaring no Tang listener are skipped by the playbook itself.
step 2 A "host baseline + LUKS (this site's nodes only)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} mgmt-host-baseline.yml -e target=$GROUP -e site_id=$SITE && $ANSIBLE_PRE ${INV[*]} tang-server.yml -e target=$GROUP && $ANSIBLE_PRE ${INV[*]} mgmt-host-luks.yml -e target=$GROUP -e site_id=$SITE"

# The obs leg's host-level half starts NOW, in parallel with everything below:
# baseline + LUKS on the obs host, then Forgejo. Neither needs the core host.
if [ "$OBS" = 1 ]; then
  leg_start host "observability host: baseline + LUKS, then utility (Forgejo)" "$OBS_LEG_BASELINE && $OBS_LEG_UTILITY"
fi

# Every pinned image, pulled on the box in parallel with phases 3 and 4; joined
# before phase 5. The binary's lines are the same (parity gate).
pull_start "every pinned image for $GROUP, pulled on its hosts in parallel (joined before phase 5)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} standup-image-pull.yml -e target=$GROUP"

# Phase 3. Per-instance secrets (operator machine; idempotent; store values + local references).
step 3 T "generate per-instance secrets" \
  "'$REPO/ops/scripts/newsite-secrets.sh' $SITE"

# Phase 4. Mint every leaf cert from the manifest (operator machine; idempotent).
#    goatlite mints the goatnet's own CA first when none exists (ADR 1213 D4,
#    goatlite.md §5 row 5): the dev blob is no longer out-of-band from anyone.
MINT_CA_FLAG=""; [ "$LITE" = 1 ] && MINT_CA_FLAG=" --mint-ca-if-missing"
step 4 T "mint leaf certs" \
  "'$REPO/ops/scripts/mint-site-certs.sh' $SITE$MINT_CA_FLAG"

# Phase 4. name-fallback snapshot (ADR 1082) — generate + sign when the goatnet opts
#     in via name_snapshot_enabled. Signs with this goatnet's offline CA (same
#     key family as the leaf certs; ca_id is read from the snapshot). goat-get (phase 12)
#     then serves it. Skips cleanly when not opted in or the CA key is absent.
CA_KEY_WORD="$(sh_word "${STATE_DECL}/${SITE}-ca/ca.key.age")"
AUDIT_WORD="$(sh_word "$AUDIT_DECL")"
NO_KEY_MSG="$(sh_quote "name_snapshot: no ${STATE_DECL}/${SITE}-ca/ca.key.age — generate ran, SIGN manually with ops/enrollment/cmd/name-snapshot-sign")"
step 4 T "name-fallback snapshot (generate + sign)" \
  "if grep -qE 'name_snapshot_enabled:[[:space:]]*true' '$REPO/ops/ansible/group_vars/${GROUP}/vars.yml' 2>/dev/null; then \
     python3 '$REPO/ops/dns/generate-mesh-hosts.py'; \
     if [ -f ${CA_KEY_WORD} ]; then \
       ( cd '$REPO/ops/enrollment' && go run ./cmd/name-snapshot-sign --snapshot '$REPO/ops/dns/${SITE}/name-snapshot.json' --root-key ${CA_KEY_WORD} --age-identity ~/.config/age/keys.txt --audit-log ${AUDIT_WORD} ); \
     else echo ${NO_KEY_MSG}; fi; \
   else echo 'name_snapshot_enabled not true for ${GROUP} — skipping (ADR 1082)'; fi"

# Phase 5. mgmt-stack (postgres/signal/management/traefik/admit-audit). Needs FIVE
#    extra-vars, not just site_id — bootstrap_owner_email plus three secrets from
#    the site's env (loaded by direnv). Without them the play fails on a missing
#    var (this is the "missing extra-vars" defect the guides routed around).
#    Phase 5 runs the `stack` plays only (--skip-tags after-ory): the account seed
#    waits for the netbird schema, which management-a writes only once it can reach
#    Hydra, so it and every play after it run in phase 6, after ory-deploy
#    (--skip-tags stack). Each play runs once; no phase stops on purpose.
OWNER_EMAIL_STANDUP="\$OWNER_EMAIL"
OWNER_EMAIL_ARG="\"\$OWNER_EMAIL\""
if [[ "${LITE}" == 1 && -n "${lite_admin:-}" ]]; then
  # The declared address reaches the bootstrap phases as data: exported, and read
  # on the command line inside double quotes with printenv, so the phase's shell
  # expands it once and never parses the address, and standup-phase-run refuses
  # it by name when it is empty (goatbuilder's owner_word does the same).
  OWNER_EMAIL_STANDUP="$lite_admin"
  export GOAT_BOOTSTRAP_OWNER_EMAIL="$lite_admin"
  OWNER_EMAIL_ARG="\"\$(printenv GOAT_BOOTSTRAP_OWNER_EMAIL)\""
fi
BOOTSTRAP_SECRETS_EV="-e bootstrap_owner_email=$OWNER_EMAIL_ARG -e postgres_superuser_password=\"\$(printenv ${SITE_UPPER}_PG_PASS)\" -e netbird_encryption_key=\"\$(printenv ${SITE_UPPER}_NB_ENCKEY)\" -e pgbackrest_cipher_pass=\"\$(printenv ${SITE_UPPER}_BKP_KEY)\""
# A joiner's bootstrap carries site_id as a PLAY variable: its own
# $SITE-bootstrap.yml when it keeps one (that play sets site_id), else the
# generic wrapper site-bootstrap-for.yml -e bootstrap_site=$SITE. Never
# site-bootstrap.yml -e site_id=$SITE for a joiner: an extra var reaches every
# inventory host, and the Spock roles' walk over the other sites' hostvars then
# finds no peer to wire. A founder keeps the plain form. (goatbuilder's
# bootstrap_play; the parity gate holds the two to the same plan.)
if [ "$JOINER" = 1 ]; then
  if [ -f "$REPO/ops/ansible/${SITE}-bootstrap.yml" ]; then SB="${SITE}-bootstrap.yml"; else SB="site-bootstrap-for.yml -e bootstrap_site=$SITE"; fi
  BOOTSTRAP_EV="$BOOTSTRAP_SECRETS_EV"
else
  SB="site-bootstrap.yml"
  BOOTSTRAP_EV="-e site_id=$SITE $BOOTSTRAP_SECRETS_EV"
fi
# A joiner's pass-1 bootstrap must NOT enable Spock yet — the account + schema
# arrive by the seed in pass 2 (phase 6). participates_in_spock also defaults
# from group_vars; this pins it for the first pass. A founder uses BOOTSTRAP_EV.
# Pass 1 brings the local stack up BEFORE the founder is wired in, so Spock is
# off. A joiner declares has_durable_independent_state:false (its durability
# arrives via Spock replication, not local backups) — but with Spock off in
# pass 1 the "no independent backups AND no replica" durability guard
# (site-bootstrap.yml) would trip. How pass 1 satisfies it depends on whether the
# site has a LOCAL replica:
#   - intra_site_pair (an A/B core pair): site-bootstrap's A->B streaming standby IS
#     the replicated copy, so the guard is satisfied by intra_site_pair alone.
#     Do NOT force independent backups — forcing them runs pgBackRest on a goatnet
#     that defers it, and stanza-create fails on the standby's rebuilt
#     cluster. Keep has_durable_independent_state at its group_vars
#     value (false for a joiner).
#   - single node, no pair: assert independent durability for THIS transient pass
#     only; pass 3 (BOOTSTRAP_EV, Spock on) restores the real posture. Measured
#     without this override such a pass-1 fails the guard.
BOOTSTRAP_EV_P1="$BOOTSTRAP_EV"
if [ "$JOINER" = 1 ]; then
  if [ "$INTRA_PAIR" = 1 ]; then
    BOOTSTRAP_EV_P1="$BOOTSTRAP_EV -e participates_in_spock=false"
  else
    BOOTSTRAP_EV_P1="$BOOTSTRAP_EV -e participates_in_spock=false -e has_durable_independent_state=true"
  fi
fi
pull_join 5
# Any refusal here is a real one and fails the phase. The binary's line is
# the same (parity gate).
step 5 A "mgmt-stack (site-bootstrap up to the account seed; the seed needs Ory and runs in phase 6)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} $SB $BOOTSTRAP_EV_P1 --skip-tags after-ory"

# Phase 5. [internet] Connect the mgmt host to its relays over the rathole tunnel.
#     site-rathole-client.yml is the generic play; a site that keeps its own
#     $SITE-rathole-client.yml gets that one instead. Uses the relay inventory too.
if [ "$PROFILE" = internet ]; then
RATHOLE_PLAY=site-rathole-client.yml
[ -f "$REPO/ops/ansible/${SITE}-rathole-client.yml" ] && RATHOLE_PLAY="${SITE}-rathole-client.yml"
step 5 A "[internet] relay-connect (rathole-client, $RATHOLE_PLAY)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE -i inventory/hosts-${SITE}.ini ${INV[*]} $RATHOLE_PLAY -e rathole_client_target=$GROUP"
fi

# Phase 6. Ory (must precede account-seed). ory-deploy.yml is `hosts: all` and reads
#    no `target` var, so it narrows with `--limit`, NOT `-e target=` (which
#    would silently deploy Ory to every mgmt host in inventory — the F-376
#    hazard, in a play the F-376 assert does not cover). It also needs the
#    Postgres superuser password from the site's <SITE>_PG_PASS env var.
step 6 A "ory (kratos/hydra/ui)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} ory-deploy.yml --limit $GROUP -e postgres_superuser_password=\"\$(printenv ${SITE_UPPER}_PG_PASS)\""

# Phase 6. [joiner] Pass 2 of the three-pass join — seed-from-peer. A data-only copy
#     of identity + mesh state from the founder into the fresh schema (Ory is up
#     now, so the tables exist), then it removes the migration marker. Guarded:
#     fails loud if the account is already populated (override -e force_reseed=true
#     deliberately). Founder sites skip this. site-seed-from-peer.yml is the
#     generic play; a site that keeps its own $SITE-seed-from-peer.yml gets that
#     one instead.
if [ "$JOINER" = 1 ]; then
SEED_PLAY=site-seed-from-peer.yml
[ -f "$REPO/ops/ansible/${SITE}-seed-from-peer.yml" ] && SEED_PLAY="${SITE}-seed-from-peer.yml"
step 6 A "[joiner] seed-from-peer (pass 2 — data-only copy from founder ${JOIN_FOUNDER_SITE:-?}; guarded)" \
  "cd '$REPO/ops/ansible' && { [ -n $JFS_Q ] || { echo 'joiner: group_vars/$GROUP/vars.yml must set join_founder_site (the founder site-id to seed from)'; exit 1; }; } && $ANSIBLE_PRE ${INV[*]} $SEED_PLAY -e site_id=$SITE"
fi

# Phase 6. The rest of site-bootstrap, now that Ory is up: the account seed waits
#     for the netbird schema (bounded: netbird_schema_wait_seconds, default 300 s,
#     and the rescue names the cause), seeds the account and prints the single-use
#     bootstrap setup key — SAVE IT (phase 11 needs it; phase 8 mints its own); then the plays after
#     it. A founder runs only that part (--skip-tags stack). A joiner runs the whole
#     file again: its pass 3 turns Spock on, which the stack plays configure.
#     Ory (Phase 7D) is staged across both sides so each step passes: pass 3
#     builds the joiner's Ory node and repset only (ory_spock_wire_subscriptions=
#     false), the founder mirror below subscribes without reconciling history,
#     then the joiner's spock-mesh-wire subscribes and reconciles both ways.
if [ "$JOINER" = 1 ]; then
step 6 A "[joiner] re-run site-bootstrap with Spock on (pass 3; Ory node + repset only; prints + saves the setup key)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} $SB $BOOTSTRAP_EV -e ory_spock_wire_subscriptions=false"
else
step 6 A "the rest of site-bootstrap: wait for the netbird schema, seed the account, print + save the setup key" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} site-bootstrap.yml $BOOTSTRAP_EV --skip-tags stack"
fi

# Phase 6. [joiner] The founder-side mirror — the SAME change, not a follow-up
#     (node-addition.md §5.4a). spock-mesh-wire.yml on the FOUNDER creates the
#     netbird_mgmt + ory repset subscriptions in BOTH directions; without it the
#     join replicates one way and the daily reconcile shows drift. PREREQUISITE
#     (config, must exist before this runs): the founder's group_vars must name
#     this joiner as a peer (inter_site_spock_peers + spock_mesh_publisher_host =
#     the joiner's mesh IP). That founder-side config is part of the joiner's
#     repo change (node-addition.md §5.5), not something -e can supply here.
#
#     SCOPE: wire ONLY this joiner ($SITE), by overriding the founder's peer
#     list to just this site. The founder's OTHER edges were established when it
#     and those peers were stood up; a NEW join only needs founder<->joiner. Not
#     scoping meant the founder's whole mesh got re-verified on every join, so a
#     single unrelated DOWN edge (e.g. a standby being rebuilt) made spock-mesh-
#     wire block on that edge's `until replicating` and the join could not finish
#     — a joiner held hostage by an edge it has nothing to do with.
#     synchronize_data stays false (role default): the joiner is pre-seeded by
#     the seed step above, so wiring carries incremental writes only and never truncates.
if [ "$JOINER" = 1 ]; then
step 6 A "[joiner] founder-side mirror (spock-mesh-wire on ${JOIN_FOUNDER_GROUP:-?}, peer=$SITE only — both-direction subscriptions)" \
  "cd '$REPO/ops/ansible' && { [ -n $JFG_Q ] || { echo 'joiner: join_founder_site not set in group_vars/$GROUP/vars.yml'; exit 1; }; } && $ANSIBLE_PRE ${INV[*]} spock-mesh-wire.yml --limit $JFG_W -e '{\"inter_site_spock_peers\":[\"$SITE\"],\"ory_spock_reconcile_history\":false}'"
step 6 A "[joiner] the joiner's Spock wiring again, now that the founder's half exists (Ory subscription + history reconcile)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} spock-mesh-wire.yml --limit $GROUP"
fi

# Phase 7. [internet] Public TLS via ACME (traefik-acme.yml). DNS-01 through Cloudflare;
#    needs the site's CF API token from the env.
if [ "$PROFILE" = internet ]; then
step 7 A "[internet] public TLS certificate (traefik-acme)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} traefik-acme.yml -e target=$GROUP -e cf_api_token=\"\$(printenv CLOUDFLARE_API_TOKEN_${SITE_UPPER})\""
fi

# Phase 7. [internet] RELAY TIER — inventory, then deploy. The phase this script did
#     not have, which is how a goatnet reaches "done" with relay VPSes that no
#     play can reach.
#
#     A goatnet can have relay VPSes with coturn running on them and NO ansible
#     inventory covering any of them, so neither relay-server.yml nor
#     goat-relay-deploy.yml can target them and the netbird relay tier is never
#     deployable. Nothing reports that, because
#     an absent phase emits nothing. The relay tier is what every peer pair
#     that cannot form direct P2P falls back to; without it a goatnet works
#     until two peers happen to sit behind unfriendly NAT, and then does not.
#
#     Two steps on purpose. generate-relays.sh reads the terraform `relays`
#     output and writes inventory/hosts-<site>.ini; relay-server.yml then
#     installs coturn, rathole and the certbot renewal path onto those hosts.
#     Running the second without the first silently matches zero hosts.
#
#     LAN goatnets have no relay tier and skip this whole block: enrollment is
#     on the LAN and there is nothing to relay through.
if [ "$PROFILE" = internet ]; then
step 7 T "[internet] generate relay inventory from terraform" \
  "'$REPO/ops/ansible/inventory/generate-relays.sh' $SITE terraform-$SITE"

# The token is asserted by the play itself (>= 30 chars, F-500) rather than
# checked here — `$(printenv X)` on an unset variable yields an empty string
# with no error, which is precisely how a relay fleet ends up with credentials
# that cannot renew and certificates that expire ninety days later.
# Secrets go through a mode-0600 vars file, NOT `-e key=value`. Anything on an
# ansible command line lands in the process table, where `ps` shows it to every
# process on the machine for the life of the run, and into any shell history or
# session log that captures the invocation. A live Cloudflare API token would be
# readable in `ps` output. The file is written
# under umask 077 and removed on the way out.
step 7 A "[internet] relay tier (coturn + rathole + TLS renewal)" \
  "cd '$REPO/ops/ansible' && umask 077 && VF=\$(mktemp -t goat-relay-vars.XXXXXX) && printf 'nb_relay_auth_secret: \"%s\"\nrelay_secret: \"%s\"\ncloudflare_api_token: \"%s\"\n' \"\$(printenv ${SITE_UPPER}_RELAY_SECRET)\" \"\$(printenv ${SITE_UPPER}_RELAY_SECRET)\" \"\$(printenv CLOUDFLARE_API_TOKEN_${SITE_UPPER})\" > \"\$VF\" && $ANSIBLE_PRE -i inventory/hosts-$SITE.ini relay-server.yml -e @\"\$VF\"; rc=\$?; rm -f \"\$VF\"; exit \$rc"
fi

# Phase 7. [joiner+internet] Wire this site into the FOUNDER's relay ingress dispatch
#     (ADR 1085) so the joiner's mgmt host is reachable through the founder's
#     relay fleet. This is the ONE bring-up action the join used to do by hand:
#     edit the founder's <founder>-goat-relay-alias.ini to add a
#     netbird-mgmt-<site> backend, then redeploy the relays live — with no gate
#     and no post-check .
#     relay-ingress-add-site.yml replaces it on rails: fail-closed gate -> merge
#     the backend into the committed founder alias file -> re-render every
#     founder relay (the existing goat-relay-rathole.yml) -> post-check the
#     backend is live. Idempotent + re-runnable; the reverse op (remove a site)
#     is in the play header.
#
#     A joiner has NO relay tier of its own — it SHARES the founder's — so this
#     runs against the FOUNDER's relay inventory, in the founder's cloud account
#     (the standup already runs under GOAT_AWS_PROFILE=$AWS_PROFILE_STANDUP,
#     which for a founder is the account the relays live in; aws-vault
#     sets AWS_VAULT so the play's SSM/SM calls use the ambient creds). Placed
#     after public TLS so the joiner's Traefik already serves the apex + its
#     federation hostname (ADR 1085 §"serve the apex before it takes weight"),
#     and before mesh-enroll so ingress exists before peers dial the public apex.
#
#     The OTHER half — the joiner's mgmt host dialing the relays so the backend
#     goes UP, not just wired — is the existing client play:
#       <founder>-rathole-client.yml -e rathole_client_target=<site>_hosts
if [ "$JOINER" = 1 ] && [ "$PROFILE" = internet ]; then
step 7 A "[joiner] wire $SITE into the founder (${JOIN_FOUNDER_SITE:-?}) relay ingress + re-render" \
  "cd '$REPO/ops/ansible' && { [ -n $JFS_Q ] || { echo 'joiner: join_founder_site not set in group_vars/$GROUP/vars.yml'; exit 1; }; } && { ls $RELAY_HOSTS_W >/dev/null 2>&1 || { echo $RELAY_MISSING_Q; exit 1; }; } && $ANSIBLE_PRE -i $RELAY_HOSTS_W -i $RELAY_ALIAS_W ${INV[*]} relay-ingress-add-site.yml -e target=goat_relay_hosts -e $FOUNDER_SITE_W -e join_site=$SITE"
fi

# Phase 8. [internet] Mesh-enroll the hosts onto their own goatnet, scripted
#     (docs/design/goatbuilder-speed.md): a single-use key minted where the
#     NetBird database is, `netbird up` against the site's declared
#     mesh_enrollment_url (its port checked against where the name resolves,
#     F-188; rosenpass flags from rosenpass_enabled), a wait for the management
#     connection and a mesh IP. Then the IPs go into group_vars, the mesh
#     inventory and the cert manifest, and the router pair is re-minted against
#     them. A host already enrolled mints nothing. LAN skips this: its router
#     cert is already final from phase 4 (bound to the LAN IP).
if [ "$PROFILE" = internet ]; then
MESH_IPS="$STATE_DIR/standup-$SITE-mesh-ips.json"
step 8 A "[internet] mesh-enroll the hosts (a single-use key each, the declared enrolment URL)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} mesh-enroll-host.yml -e target=$GROUP -e mesh_ips_out=$(sh_quote "$MESH_IPS")"
step 8 T "[internet] record the mesh IPs (group_vars, mesh inventory, cert manifest) and re-mint the router cert" \
  "python3 '$REPO/ops/scripts/standup-mesh-record.py' $SITE --ips $(sh_quote "$MESH_IPS")"
fi

# Phase 9. Per-uuid Zenoh router (binds the mesh IP on internet, LAN IP on lan; cert from phase 4/8).
step 9 A "zenoh router" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} zenoh-deploy.yml -e target=$GROUP"

# Phase 9. SPIRE workload identity (core tier, service-criticality-tiers.md). The
#      founder is the goatnet's SPIRE server: it mints the SVIDs that the portal
#      bridge, id-console, onboard-runner and grant-keyd authenticate with. The
#      role does nothing on a host not declared the CA holder, so the founder is
#      declared here rather than relying on a host_vars line a new goatnet may
#      not have (SPIRE was reached only by a hand-run play).
step 9 A "SPIRE server on the founder (workload identity)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} spire-deploy.yml -e target=$GROUP -l 'mgmt-${SITE}-01*' -e spire_enabled=true -e postgres_superuser_password=\"\$(printenv ${SITE_UPPER}_PG_PASS)\""

# The obs STACK starts now: the router is up, and on an internet site phase 8
# has just produced the core host's mesh address that obs_scrape_targets names.
# Runs in parallel with 10..12; joined before the acceptance gate.
if [ "$OBS" = 1 ]; then
  leg_start stack "observability host: stack (Prometheus/Alertmanager/Grafana) — needs obs_scrape_targets" "$OBS_LEG_STACK"
fi

# Phase 10. Service surface — this is where the profiles diverge.
#   LAN: site-services.yml itself, scoped to its LAN-surface roles (lan-coredns,
#        traefik, coturn) by tag — no per-site copy (#392: the old "awk it out
#        of hobby-bootstrap, make a per-site file" path is gone; site-services.yml
#        is the one file every goatnet runs). The tags also keep this call from
#        reaching site-services.yml's own zenoh-router play (already run as its
#        own step, phase 9 above) and its phase-12/14-equivalent imports early.
#   internet: the dashboard on its own, then the OIDC clients (per-env file).
if [ "$PROFILE" = lan ]; then
step 10 A "[lan] LAN service surface (dashboard + DNS + TLS routes + STUN)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} -l 'mgmt-${SITE}-01*' site-services.yml --tags lan-dns,traefik,stun -e target=$GROUP -e site_id=$SITE -e postgres_superuser_password=\"\$(printenv ${SITE_UPPER}_PG_PASS)\" -e netbird_encryption_key=\"\$(printenv ${SITE_UPPER}_NB_ENCKEY)\""

step 10 A "[lan] TLS routes for the mgmt stack (file certs)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} -l 'mgmt-${SITE}-01*' lan-traefik-routes.yml -e target=$GROUP -e site_id=$SITE"
step 10 A "[lan] dashboard" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} -l 'mgmt-${SITE}-01*' dashboard-deploy.yml -e site_id=$SITE"

# Phase 11. [lan] The same browser step the internet profile has, for EVERY LAN
#      site (goatlite.md §5 row 6): the bootstrap NetBird token exists only once
#      someone has signed in to the dashboard, and phase 12 mints everything else
#      from it. Printed ONLY while the declared secrets posture does not already
#      hold <SITE>_NB_API_TOKEN (posture.py available), so a resumed or re-run
#      standup walks straight into phase 12. On a LAN site the dashboard names
#      live under the site's own zone, served by the host's LAN CoreDNS.
if ! python3 "$REPO/ops/secrets/posture.py" available 2>/dev/null | grep -qx "${SITE_UPPER}_NB_API_TOKEN"; then
step 11 MANUAL "[lan] first login, admin-promote, the bootstrap NetBird token" \
  "handbook §'Phase 11' has the exact block; on a LAN site the dashboard is the dashboard.* name under this site's zone (site_dns_zone in group_vars/$GROUP/vars.yml), resolved by the host's LAN CoreDNS: point the laptop's DNS at the host or pin the names in /etc/hosts. Read the one-time password from /opt/mgmt-host/.bootstrap-passwords/$OWNER_EMAIL_STANDUP.txt on the host, log in; re-run the phase-10 LAN service line ONCE (promotes you to admin — the UI 403s until then); create service user svc-${SITE}-netbird-infra (Team > Service Users, role admin), create its token, and store it as ${SITE_UPPER}_NB_API_TOKEN under the declared secrets posture (posture.py hint ${SITE_UPPER}_NB_API_TOKEN says where). THEN resume: ops/scripts/goat-standup.sh $SITE --from 12"
fi
else
step 10 A "[internet] dashboard" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} -l 'mgmt-${SITE}-01*' dashboard-deploy.yml -e site_id=$SITE"
# site-oidc-clients.yml is the generic play; a site that keeps its own
# $SITE-oidc-clients.yml gets that one instead.
OIDC_PLAY=site-oidc-clients.yml
[ -f "$REPO/ops/ansible/${SITE}-oidc-clients.yml" ] && OIDC_PLAY="${SITE}-oidc-clients.yml"
step 10 A "[internet] register OIDC clients ($OIDC_PLAY)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} $OIDC_PLAY -e target=$GROUP"

# Phase 11. [internet] The browser step: first login, the admin promotion, and the ONE
#      NetBird token a human has to mint — the bootstrap service identity, which
#      only exists once someone can sign in to the dashboard. Every other NetBird
#      identity is minted from it by phase 12, and every phase-12 service reads
#      one, which is why this pause is phase 11 and not later.
step 11 MANUAL "[internet] first login, admin-promote, the bootstrap NetBird token" \
  "handbook §'Phase 11' has the exact block. In short: enroll YOUR laptop (netbird up), pin the three dashboard hosts in /etc/hosts, read the one-time password from /opt/mgmt-host/.bootstrap-passwords/$OWNER_EMAIL_STANDUP.txt, log in; re-run the phase-10 dashboard-deploy line ONCE (promotes you to admin — the UI 403s until then); create service user svc-${SITE}-netbird-infra (Team > Service Users, role admin), create its token, add 'export ${SITE_UPPER}_NB_API_TOKEN=<token>' to .envrc.local.d/30-site-services.envrc and store it in 1Password as ${SITE}-netbird / api_token. THEN resume: ops/scripts/goat-standup.sh $SITE --from 12"
fi

# Phase 12. THE DECLARED ACL, FIRST (ADR 1185). NetBird's own Default All->All is off
#     (management.json DisableDefaultPolicy) and nothing seeds one, so until the
#     declared policies exist enrolled peers reach nothing — and every later
#     phase that crosses the mesh depends on them. Groups, then policies, then
#     enforce (which deletes any Default All->All it finds), as ONE ansible-playbook
#     run of the two plays in that order: netbird-acl-bootstrap.yml makes the
#     groups, netbird-acl-cutover.yml mode=enforce creates every declared policy
#     and then applies the declared Default-policy action (enforce does all that
#     mode=apply does first), each play's asserts unchanged. Uses the bootstrap
#     token from phase 11. A joiner applies nothing: its account IS the founder's.
if [ "$JOINER" = 0 ]; then
step 12 A "declared NetBird ACL: groups, then policies, then enforce (one play run)" \
  "cd '$REPO/ops/ansible' && ansible-playbook -i localhost, --connection local netbird-acl-bootstrap.yml netbird-acl-cutover.yml -e target_site=$SITE -e mode=enforce -e confirm_cutover=YES -e netbird_mgmt_url=$MGMT_URL_W"
fi
step 12 GATE "the live NetBird account holds only the declared ACL for $ACL_SITE, and no All-group rule" \
  "python3 '$REPO/ops/scripts/netbird-policy-posture.py' $ACL_SITE_Q --url $MGMT_URL_W --token-env $(sh_word "${ACL_UPPER}_NB_API_TOKEN")"

# Phase 12. NetBird service identities (ops/netbird/service-identities.yaml). This
#     goatnet's group_vars decide which identities it needs; mint every missing
#     one from the bootstrap token (writes the operator envrc + 1Password, never
#     prints a token), then refuse to go on unless each required token exists and
#     authenticates as the service user the registry names. Every consumer role
#     below also refuses to deploy without its token, so a skipped mint stops
#     the run here instead of deploying services that run without their
#     capability — a portal without it refuses every machine enrollment.
step 12 T "mint the NetBird service identities this goatnet needs (creates only what is missing)" \
  "python3 '$REPO/ops/scripts/netbird-service-identities.py' $SITE --group $GROUP --mint"
step 12 GATE "every required NetBird service identity is present and verified" \
  "python3 '$REPO/ops/scripts/netbird-service-identities.py' $SITE --group $GROUP --live"

# Phase 12. netbird account baseline: the mesh-DNS nameserver group, and no policy
#     (ADR 1185). FOUNDERS ONLY — a joiner's NetBird store is the founder's
#     account, replicated, and this play writes rows into it: the
#     play would put an enabled All->All into the live founder account. The play
#     itself also refuses a joiner. Like Ory it is
#     `hosts: all` and reads no `target` — narrow it with `--limit`.
if [ "$JOINER" = 0 ]; then
step 12 A "netbird account baseline (mesh-DNS nameserver group; no policy)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} goat-netbird-account-init.yml --limit $GROUP"
fi

# Phase 12. [internet] Mesh CoreDNS, after the account baseline created the mesh-DNS
#      group it attaches to. goat-coredns.yml is `hosts: all` and reads no
#      `target`, so it is narrowed with --limit. Afterwards, drop the laptop's
#      /etc/hosts pin from phase 11.
#
#      ONLY for a site that has a mesh zone. generate-mesh-hosts.py renders a
#      zone (ops/dns/<site>/mesh-hosts) only for a site with a non-empty
#      isitup_services; a joiner that declares none
#      has no zone and resolves through
#      the FOUNDER's DNS plane — its only local name, the zenoh router, is
#      reached by mesh IP (its cert carries the IP SAN, not a DNS name). Running
#      mesh CoreDNS there rendered a Corefile and then died in the drift-guard's
#      awk on the never-generated zone file. Skip it when no zone exists.
if [ "$PROFILE" = internet ]; then
step 12 A "[internet] mesh CoreDNS (only if this site has a mesh zone; joiners use the founder's DNS)" \
  "cd '$REPO' && python3 ops/dns/generate-mesh-hosts.py && { if [ -f ops/dns/$SITE/mesh-hosts ]; then cd ops/ansible && $ANSIBLE_PRE ${INV[*]} goat-coredns.yml --limit $GROUP; else echo '    no ops/dns/$SITE/mesh-hosts — this site declares no mesh zone (a joiner on the founder DNS plane); skipping mesh CoreDNS'; fi; }"
fi

# Phase 12. goat-zenoh-portal — the registry, certificate minting and the grant rows
#      the rest of phase 12 depends on (onboarding intake + runner, grant-keyd).
#      It was reached only by a hand-run portal-deploy.yml, so a
#      standup that followed this script got a goatnet with no portal.
step 12 A "portal (registry, certificate minting, grant rows)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} portal-deploy.yml -e target=$GROUP -l 'mgmt-${SITE}-01*' -e postgres_superuser_password=\"\$(printenv ${SITE_UPPER}_PG_PASS)\""

# Phase 12. pub/sub state-of-record Repl (Block 86) — mirror the portal registry to
#      Forgejo (+ MinIO shadow) so a rebuild restores fabric STRUCTURE, not just
#      an empty fabric. Requires a Forgejo for this goatnet (goatcore ships no
#      Forgejo role) + the mirror bot minted once
#      (ops/scripts/forgejo-mint-pubsub-bot.sh <forgejo-ssh-target>).
#      The mint and the mirror both skip cleanly when the goatnet declares no
#      pubsub-state group_var.
#      Design: docs/design/pubsub-state-as-code.md §Deployment topologies.
# Forgejo must be up (leg O1O2) and the mirror bot minted before the mirror
# deploys. The mint is once-per-goatnet and writes FORGEJO_PUBSUB_TOKEN to the
# operator envrc; it is skipped when the token is already in the environment.
if [ "$OBS" = 1 ]; then
  leg_join host 12
fi
# The bot lives on the goatnet's Forgejo, which goatcore does not deploy: a
# goatnet that declares no pub/sub state forge skips the mint and goes on.
if [ "$OBS" = 1 ] && [ ! -f "$REPO/ops/ansible/group_vars/${GROUP}/pubsub-state.yml" ]; then
  if [ 12 -ge "$FROM" ] && [ 12 -le "$TO" ]; then
    printf '\n=== Phase 12 [T] %s ===\n    SKIPPED — %s\n    %s\n' "mint the pubsub-state mirror bot on the obs host's Forgejo (once; skipped when FORGEJO_PUBSUB_TOKEN is set)" \
      "no group_vars/${GROUP}/pubsub-state.yml: this goatnet declares no Forgejo for the pub/sub state mirror" \
      "goatcore ships no Forgejo role; to turn the mirror on, run a Forgejo for this goatnet and declare it (ops/ansible/roles/pubsub-state-repo/README.md)"
  fi
elif [ "$OBS" = 1 ]; then
  # user@address of the obs host, from the same two inventories the plays use
  # (the mesh override wins when the host has enrolled).
  OBS_SSH_TARGET="$(python3 "$REPO/ops/scripts/inventory-hosts.py" "$OBS_GROUP" --ssh-target --inventory "$REPO/ops/ansible/inventory/mgmt-hosts.ini" --inventory "$REPO/ops/ansible/inventory/mgmt-hosts-mesh.ini" 2>/dev/null | head -1)"
step 12 T "mint the pubsub-state mirror bot on the obs host's Forgejo (once; skipped when FORGEJO_PUBSUB_TOKEN is set)" \
  "if [ -n \"\${FORGEJO_PUBSUB_TOKEN:-}\" ]; then echo '    FORGEJO_PUBSUB_TOKEN already set — bot minted; skipping'; \
   else '$REPO/ops/scripts/forgejo-mint-pubsub-bot.sh' $(sh_quote "$OBS_SSH_TARGET") \
     && eval \"\$('$REPO/ops/scripts/envrc-var.sh' FORGEJO_PUBSUB_TOKEN)\" \
     && '$REPO/ops/scripts/standup-keep-secret' FORGEJO_PUBSUB_TOKEN \
     && echo '    token loaded from the operator envrc; the run continues'; fi"
fi
step 12 GATE "every input the phases after the manual step will reach is set (the NetBird service token phase 11 issues)" \
  "python3 '$REPO/ops/scripts/standup-required-inputs.py' $SITE --profile $PROFILE --from 12"

step 12 A "pubsub-state Repl mirror (Block 86)" \
  "cd '$REPO/ops/ansible' && if ls group_vars/${GROUP}/pubsub-state.yml >/dev/null 2>&1; then $ANSIBLE_PRE ${INV[*]} pubsub-state-mirror-deploy.yml -e target=$GROUP; else echo 'no group_vars/${GROUP}/pubsub-state.yml — the pub/sub state mirror is not configured for this goatnet; skipping. It needs a Forgejo you run for this goatnet (goatcore ships no Forgejo role): see ops/ansible/roles/pubsub-state-repo/README.md, then re-run --from 12'; fi"

# Phase 12. Onboarding PUBLIC intake front desk (ADR 1062 stage 1) — the public web
#      page an off-mesh candidate opens to submit their onboarding form. Without
#      it, `goat admin invite` hands out links to a host that does not exist and
#      the CANONICAL three-stage onboarding cannot run at all (there is no other
#      way to put a candidate into the review queue). Skipping this blocks
#      onboarding a whole cohort. Respects
#      onboard_intake_enabled; needs intake.<public> DNS — an A round-robin over
#      the relay EIPs (Cloudflare), NOT a CNAME onto the netbird apex (GH #151).
step 12 A "onboarding intake front desk (public, ADR 1062 stage 1)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} onboard-intake-deploy.yml -e target=$GROUP -e site_id=$SITE"

# Phase 12. onboard-runner (ADR 1062 stage 3) — the daemon that turns an admin
#      approval into an automatic mint + welcome email. Respects
#      onboard_runner_enabled. Without it, stage-3 approvals never mint.
step 12 A "onboard-runner (auto-mint on approval, ADR 1062 stage 3)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} onboard-runner-deploy.yml -e target=$GROUP -e site_id=$SITE"

# Phase 12. grant-keyd — grant-keyed encryption key distribution (ADRs 1094/1106).
#      Turns each `goat access grant` into a wrapped key bundle on the fabric.
#      Needs the portal (above) and SPIRE (phase 9). Its token is resolved on the
#      host and registered in the portal's key directory; the play fails unless
#      the portal accepts it. Single-homed on the founder.
step 12 A "grant-keyd (encryption key distribution)" \
  "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} grant-keyd-deploy.yml -e target=$GROUP -l 'mgmt-${SITE}-01*'"

# Phase 12. notional publishers/subscribers (synthetic demo traffic) — OPTIONAL per
#      goatnet. A dev/demo goatnet opts in with synth_vendors_enabled: true in
#      its group_vars so the fabric has real traffic for panoscope/portal; a
#      real goatnet leaves it off and this phase skips cleanly. Deploy needs the
#      per-vendor mTLS certs (ops/zenoh-trial-peers/bootstrap-portal-vendors.sh)
#      staged first when the router endpoint is tls/.
#      Uses the per-uuid-slot-pool onboarder (the slot-pool synth-vendors deploy play)
#      that mints a slot anchor + cert per synth peer via the portal — the model
#      every modern goatnet uses. (A flat-vendor goatnet uses
#      zenoh-synth-vendors-deploy.yml instead.) Needs PORTAL_OPERATOR_API_KEY.

step 12 A "notional pub/sub (synthetic vendors — opt-in via synth_vendors_enabled)" \
  "cd '$REPO/ops/ansible' && { [ ! -e group_vars/${GROUP}/synth-vendors.yml ] || [ -r group_vars/${GROUP}/synth-vendors.yml ] || { echo 'cannot read group_vars/${GROUP}/synth-vendors.yml' >&2; false; }; } && if grep -qE '^[[:space:]]*synth_vendors_enabled:[[:space:]]*.?(true|True|TRUE|yes|Yes|YES|on|On|ON|1).?[[:space:]]*(#.*)?$' group_vars/${GROUP}/synth-vendors.yml 2>/dev/null; then $ANSIBLE_PRE ${INV[*]} zenoh-synth-vendors-slot-pool-deploy.yml -e target=$GROUP; else echo 'synth_vendors_enabled not true for ${GROUP} — notional pub/sub not requested, skipping'; fi"

# --parallel: run the queued phase-12 steps in their waves, before anything of phase 13.
dag_flush

# Obs stack joined before the gate (its observability rows are part of the verdict).
if [ "$OBS" = 1 ]; then leg_join stack 13; fi

# Phase 13. ACCEPTANCE GATE — prove every provisioned aspect is live + functional.
#     First DERIVE the value set from this goatnet's own config (generate-values.py):
#     without it every row resolves to <UNRESOLVED> and the gate renders no verdict
#     and the goatnet stays unverifiable. Then run the gate against it.
#     Read the report; the exit code is 0 unless a row FAILs. An UNKNOWN/expected
#     list is normal — the handbook §"Phase 13" says which rows that is.
#     On a goatlite the phase is off by default (goatlite.md §11 measure 4):
#     --check runs it here, `goatbuilder assay <site>` at any time. The binary
#     prints the same skip.
ACCEPTANCE_LABEL="acceptance suite (values derived from config; all applicable rows green)"
if [ "$LITE" = 1 ] && [ "$CHECK" = 0 ]; then
  if [ 13 -ge "$FROM" ] && [ 13 -le "$TO" ]; then
    printf '\n=== Phase 13 [GATE] %s ===\n    SKIPPED — %s\n    %s\n' "$ACCEPTANCE_LABEL" \
      "goatlite (ADR 1213): the acceptance phase is off by default; --check runs it in the standup" \
      "run it on its own at any time: goatbuilder assay $SITE"
  fi
else
step 13 GATE "$ACCEPTANCE_LABEL" \
  "python3 '$REPO/ops/goatbuilder/endstate/endstate.py' model $SITE > $(sh_quote "$STATE_DIR/endstate-$SITE.json") && python3 '$REPO/ops/validation/generate-values.py' $SITE && python3 '$REPO/ops/validation/run-acceptance.py' --env $SITE --values '$REPO/ops/validation/values-$SITE.yaml' --endstate $(sh_quote "$STATE_DIR/endstate-$SITE.json") --all-eligible --by-layer"
fi

# Phase 14. JOIN THE CROSS-GOATNET SURFACES. The goatnet overview on every relay-health
#     box (goatnets.html) and the summary publish behind it are rendered from
#     ops/goatnets/cross-env.yaml at deploy time, so a new goatnet is on no
#     sibling's overview until every box is redeployed. Refuses when the site has no
#     cross-env.yaml entry (a fleet-pins goatnet without one already fails CI),
#     or when the committed generated config is stale. Retiring a goatnet runs the
#     same script after its entries are deleted.
# ---- 15a. the two rescue classes (ADR 1183) --------------------------------
# BREAKGLASS (ADR 1114) is the way in when the mesh and the control plane are
# both down; NETWORK-JUMP (ADR 1131) is the reach to the switch, the router and
# out-of-band gear management. Both existed as classes with playbooks that no
# phase ever called, so a live break-glass node had no config scope on its own
# goatnet and the jump class had zero peers anywhere. The site's group_vars say
# which it has (the phase-1 config gate refuses silence), and this phase
# converges what is declared and says out loud what it skipped and why.
#
# Declared-but-not-inventoried is NOT an error — it is the normal state of a
# site whose box has not arrived, same as has_obs_host. It skips with a note.
gv_value() {  # gv_value <key> — one group_vars value, folded scalars included
  # Read it as YAML, not with grep: `reason: >-` puts the sentence on the
  # following lines, and a grep of the key line prints the fold marker.
  python3 - "$REPO/ops/ansible/group_vars/$GROUP/vars.yml" "$1" <<'PY' 2>/dev/null || true
import sys
try:
    import yaml
    d = yaml.safe_load(open(sys.argv[1])) or {}
except Exception:
    sys.exit(0)
v = d.get(sys.argv[2])
if v is not None:
    print(" ".join(str(v).split()))
PY
}

rescue_phase() {  # rescue_phase <flag> <reason-key> <group-suffix> <extra-inv> <label> <command...>
  local flag="$1" reason_key="$2" suffix="$3" extra_inv="$4" label="$5"; shift 5
  local rgroup="${SITE//-/_}_${suffix}_hosts"
  local declared why
  declared="$(gv_value "$flag")"
  if [ "$declared" != "True" ] && [ "$declared" != "true" ]; then
    why="$(gv_value "$reason_key")"
    printf '\n=== Phase 14 [A] %s ===\n    SKIPPED — %s is not true in group_vars/%s/vars.yml\n    declared reason: %s\n' \
      "$label" "$flag" "$GROUP" \
      "${why:-(none recorded — the phase-1 config gate refuses this; ADR 1183)}"
    return 0
  fi
  if [ -z "$(python3 "$REPO/ops/scripts/inventory-hosts.py" "$rgroup" 2>/dev/null)" ]; then
    printf '\n=== Phase 14 [A] %s ===\n    SKIPPED — %s: true but no host in inventory group [%s] yet.\n    Declared and not yet built is a real state; add the host when the box arrives.\n' \
      "$label" "$flag" "$rgroup"
    return 0
  fi
  local inv_extra=""
  [ -n "$extra_inv" ] && [ -f "$REPO/ops/ansible/$extra_inv" ] && inv_extra="-i $extra_inv"
  step 14 A "$label" "cd '$REPO/ops/ansible' && $ANSIBLE_PRE ${INV[*]} $inv_extra $* -e target=$rgroup"
}

rescue_phase has_breakglass_node breakglass_absent_reason breakglass \
  "inventory/${SITE}-breakglass-nodes.ini" \
  "break-glass node (ADR 1114 — the way in when mesh + control plane are both down)" \
  "breakglass-nodes.yml"

rescue_phase has_network_jump_node network_jump_absent_reason network_jump \
  "inventory/${SITE}-network-jump.ini" \
  "network-jump host (ADR 1131 — reach to switch, router and out-of-band gear; herd-connector has no public image: set herd_connector_image in group_vars to an image you supply, or the play refuses)" \
  "herd-connector-deploy.yml -e herd_connector_target_hosts=${SITE//-/_}_network_jump_hosts"

# G-20: cross-env-apply.sh redeploys relay-health on EVERY box in
# cross-env.yaml, which is other goatnets' machines. A goatlite must not reach past
# its one box (ADR 1213), and a fresh checkout's roster names goatnets whose
# credentials a learner does not hold.
if [ "$LITE" = 1 ]; then
  printf '\n=== Phase 14 [A] %s ===\n    SKIPPED — %s\n    %s\n' \
    "join the cross-goatnet surfaces (relay-health overview on every box)" \
    "goatlite (ADR 1213): one box on its own LAN; this step reaches every relay-health box in ops/goatnets/cross-env.yaml, none of which belong to this goatnet" \
    "declared reason: cross-env.yaml carries this goatnet as relay_health absent; the overviews pick its roster row up on their next deploy"
else
step 14 A "join the cross-goatnet surfaces (relay-health overview on every box)" \
  "'$REPO/ops/goatnets/cross-env-apply.sh' --require $SITE"
fi

if [ "$PLAN" = 1 ]; then
  echo; echo "PLAN complete ($PROFILE profile). Remove --plan to execute. The one manual step"
  echo "(phase 11 first login, both profiles, until the posture holds ${SITE_UPPER}_NB_API_TOKEN)"
  echo "pauses the run; resume with --from <next>."
  echo "Read docs/operations/goatnet-standup.md first."
fi

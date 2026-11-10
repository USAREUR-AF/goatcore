#!/usr/bin/env bash
# kanidm-door-audit.sh — who was invited through a Kanidm door and never got in.
#
# The gap this closes: issuing a credential-setup code is fire-and-forget. The
# door records that a person EXISTS; nothing records that they ever finished
# setup. So a person whose code expired, or whose one-shot link was spent in
# transit by a mail-security scanner, is stranded silently — and stays
# stranded until they happen to email somebody. that was the only
# reason anyone learned that 83 of 98 persons on one door carried no
# credential at all.
#
# The measurable fact, and the only one this reports as fact: a person with
# neither `primary_credential` nor `passkeys` in their attrs has never completed
# credential setup on THIS door.
#
# What that does NOT prove is that they cannot log in — and whether it does
# depends on the DOOR, which is why the fallback is per-plane rather than a
# constant. Verified by fetching each login page:
#
#   public (relay-health / consistency, via the public sign-in broker)
#       "Log in with Google" + "Log in with Kanidm"  -> gmail/googlemail is a
#       certain second way in; those accounts are not stranded.
#   collab (chat / docs / files, via the collaboration sign-in broker)
#       Kanidm ONLY — the request goes straight to the collab Kanidm with no
#       connector chooser. There is NO fallback. On this door, no credential
#       means no access, gmail or not.
#
# Assuming the public door's shape on the collab door would have under-reported
# collab strands by 15 people on the day this was written. Any domain that is
# not certainly Google-backed is reported AT RISK, never as locked out — a
# person may hold a Workspace account on their own domain, unknowable from here.
#
#   ops/scripts/kanidm-door-audit.sh
#   ops/scripts/kanidm-door-audit.sh --plane collab
#   ops/scripts/kanidm-door-audit.sh --format tsv
#   ops/scripts/kanidm-door-audit.sh --prom /var/lib/node_exporter/door.prom
#   ops/scripts/kanidm-door-audit.sh --reinvite-plan
#
# READ-ONLY. Two GETs (auth, then /v1/person). It mints nothing, writes nothing
# to the door, and can never be the cause of a change you are investigating.
#
# PII: handles and DOMAINS only. The local-part of an address never reaches
# stdout, a metric or a report file — the same discipline the collab reconcile
# keeps (docs/design/collab-onboarding-provisioner.md). A domain is what makes
# the Google-fallback call, so a domain is what it prints.
#
# The door and its credential come from the environment (direnv, or the unit's
# EnvironmentFile), never arguments:
#   public plane (dashboard door) → PUBLIC_DOOR_KANIDM_URL, and
#     GOAT_IDM_ADMIN_PW (run from a checkout, the variable the checkout names
#     as public_door_idm_admin_pw_env when GOAT_IDM_ADMIN_PW is unset),
#     or a read-scoped service-account bearer in DOOR_AUDIT_TOKEN (preferred;
#     mint with ops/scripts/create-door-audit-identity.sh)
#   collab plane                  → COLLAB_KANIDM_URL, COLLAB_KANIDM_IDM_ADMIN_PW
# The URL is the base of that door's Kanidm (https://idm.<zone>); there is no
# built-in door. Run from a checkout with the URL variable unset, the address
# is what the checkout declares as public_door_kanidm_url / collab_door_kanidm_url
# in ops/ansible/group_vars/all.yml.
#
# Exit codes: 0 = the audit ran (findings are a REPORT, not a failure — a guard
# that exits non-zero on "found something" trains people to mute it). 2 = usage
# or config error. 3 = the door could not be read, which is the one case where
# the number this prints would be a lie.
set -euo pipefail

PLANE=public
FORMAT=text
PROM_PATH=""
REINVITE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --plane)         PLANE="$2"; shift 2 ;;
    --format)        FORMAT="$2"; shift 2 ;;
    --prom)          PROM_PATH="$2"; shift 2 ;;
    --reinvite-plan) REINVITE=1; shift ;;
    -h|--help)       sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# One value this checkout declares in ops/ansible/group_vars/all.yml (through
# project-setting.sh). Prints nothing where the script runs outside a checkout.
declared_var() {
  local helper
  helper="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-setting.sh"
  [ -x "$helper" ] || return 0
  "$helper" var "$1" 2>/dev/null || true
}

# The variable the public door's idm_admin password is read from:
# GOAT_IDM_ADMIN_PW. A checkout whose secret references carry another name
# declares it as public_door_idm_admin_pw_env, and that variable is read when
# GOAT_IDM_ADMIN_PW is unset.
public_pw_var() {
  local name=""
  [ -n "${GOAT_IDM_ADMIN_PW:-}" ] || name="$(declared_var public_door_idm_admin_pw_env)"
  case "$name" in ""|[0-9]*|*[!A-Za-z0-9_]*) name="" ;; esac
  if [ -n "$name" ] && [ -n "${!name:-}" ]; then printf '%s' "$name"; else printf 'GOAT_IDM_ADMIN_PW'; fi
}

case "$PLANE" in
  public) HOSTVAR=PUBLIC_DOOR_KANIDM_URL; HOSTKEY=public_door_kanidm_url; PWVAR="$(public_pw_var)" ;;
  collab) HOSTVAR=COLLAB_KANIDM_URL;      HOSTKEY=collab_door_kanidm_url; PWVAR=COLLAB_KANIDM_IDM_ADMIN_PW ;;
  *) echo "unknown --plane '$PLANE' (want: public | collab)" >&2; exit 2 ;;
esac
PW="${!PWVAR:-}"
HOST="${!HOSTVAR:-}"; [ -n "$HOST" ] || HOST="$(declared_var "$HOSTKEY")"; HOST="${HOST%/}"
[ -n "$HOST" ] || { echo "$HOSTVAR is not set and the checkout declares no $HOSTKEY — the base URL of the $PLANE door's Kanidm (https://idm.<zone>)." >&2; exit 2; }
case "$FORMAT" in text|tsv) : ;; *) echo "unknown --format '$FORMAT' (want: text | tsv)" >&2; exit 2 ;; esac

TOKEN="${DOOR_AUDIT_TOKEN:-}"

if [ -z "$TOKEN" ]; then
  if [ -z "$PW" ]; then
    echo "no credential: set DOOR_AUDIT_TOKEN (read-scoped, preferred) or $PWVAR." >&2
    echo "  Mint the scoped one: ops/scripts/create-door-audit-identity.sh --plane $PLANE" >&2
    exit 2
  fi
  hdrs=$(mktemp); trap 'rm -f "$hdrs"' EXIT
  curl -sf -D "$hdrs" -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
    -H 'Content-Type: application/json' -d '{"step":{"init":"idm_admin"}}' \
    || { echo "cannot reach $HOST" >&2; exit 3; }
  SID=$(grep -i '^x-kanidm-auth-session-id:' "$hdrs" | tr -d '\r' | cut -d' ' -f2)
  [ -n "$SID" ] || { echo "no auth session id from $HOST" >&2; exit 3; }
  curl -sf -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
    -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
    -d '{"step":{"begin":"password"}}' || { echo "auth begin failed on $HOST" >&2; exit 3; }
  TOKEN=$(curl -s --max-time 20 -X POST "$HOST/v1/auth" \
    -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
    --data-binary "{\"step\":{\"cred\":{\"password\":\"$PW\"}}}" \
    | python3 -c 'import sys,json
raw=sys.stdin.read().strip()
try: print(json.loads(raw).get("state",{}).get("success","") or "")
except Exception: print("")')
  if [ -z "$TOKEN" ]; then
    echo "idm_admin auth failed on $HOST — $PWVAR is probably stale." >&2
    exit 3
  fi
fi

BODY=$(curl -s -w '\n%{http_code}' --max-time 40 -H "Authorization: Bearer $TOKEN" "$HOST/v1/person")
STATUS=${BODY##*$'\n'}
JSON=${BODY%$'\n'*}
if [ "$STATUS" != "200" ]; then
  echo "GET $HOST/v1/person -> HTTP $STATUS; refusing to report a count from a failed read." >&2
  exit 3
fi

# The person list goes to the reader via a FILE, not a pipe: the analysis below
# is a heredoc, and a heredoc IS stdin — piping the JSON in as well silently
# loses it and the script reports zero persons on a door full of them.
PERSONS_JSON=$(mktemp); trap 'rm -f "$PERSONS_JSON" ${hdrs:-}' EXIT
printf '%s' "$JSON" > "$PERSONS_JSON"

HOST="$HOST" PLANE="$PLANE" FORMAT="$FORMAT" PERSONS_JSON="$PERSONS_JSON" \
  PROM_PATH="$PROM_PATH" REINVITE="$REINVITE" python3 - <<'PY'
import json, os, sys, time

host    = os.environ["HOST"]
plane   = os.environ["PLANE"]
fmt     = os.environ["FORMAT"]
prom    = os.environ["PROM_PATH"]
replan  = os.environ["REINVITE"] == "1"

# Domains where a Google login is CERTAINLY available, so no Kanidm credential
# is not a strand. Everything else may or may not be a Workspace domain; that is
# unknowable from this side of the door, so it is reported as at-risk, never as
# locked out.
#
# EMPTY on a door with no second connector. The collab door brokers Kanidm
# alone (verified its sign-in goes straight through, no connector
# chooser), so nothing there is a fallback and every credential-less person is
# stranded. Carrying the public door's assumption across would have quietly
# discounted 15 real strands.
GOOGLE_CERTAIN = {"gmail.com", "googlemail.com"} if plane == "public" else set()

try:
    with open(os.environ["PERSONS_JSON"]) as fh:
        persons = json.load(fh)
except Exception as e:
    print("could not parse the person list: %s" % e, file=sys.stderr)
    raise SystemExit(3)

rows = []
for p in persons:
    a = (p or {}).get("attrs") or {}
    name = (a.get("name") or [""])[0]
    # Built-in service identities are not people and have no setup to finish.
    if not name or name.startswith("idm_"):
        continue
    mail = (a.get("mail") or [""])[0]
    domain = mail.split("@")[-1].lower() if "@" in mail else "(no-mail)"
    has_cred = ("primary_credential" in a) or bool(a.get("passkeys")) or bool(a.get("attested_passkeys"))
    rows.append({"handle": name, "domain": domain, "credential": has_cred})

rows.sort(key=lambda r: (r["credential"], r["domain"], r["handle"]))
total     = len(rows)
done      = [r for r in rows if r["credential"]]
nocred    = [r for r in rows if not r["credential"]]
google_ok = [r for r in nocred if r["domain"] in GOOGLE_CERTAIN]
at_risk   = [r for r in nocred if r["domain"] not in GOOGLE_CERTAIN]

if fmt == "tsv":
    print("handle\tdomain\tcredential\tclass")
    for r in rows:
        cls = ("complete" if r["credential"]
               else "google-fallback" if r["domain"] in GOOGLE_CERTAIN
               else "at-risk")
        print("%s\t%s\t%s\t%s" % (r["handle"], r["domain"], "yes" if r["credential"] else "no", cls))
else:
    print("Kanidm door audit — %s (%s plane)" % (host, plane))
    print("")
    print("  persons on this door           %4d" % total)
    print("  completed credential setup     %4d" % len(done))
    print("  never completed setup          %4d" % len(nocred))
    if GOOGLE_CERTAIN:
        print("    of which gmail/googlemail    %4d   (Log in with Google works — not stranded)" % len(google_ok))
        print("    of which any other domain    %4d   (AT RISK: no proven second way in)" % len(at_risk))
    else:
        print("    all of them AT RISK          %4d   (this door brokers Kanidm ONLY —" % len(at_risk))
        print("                                        there is no second way in)")
    print("")
    if at_risk:
        print("  AT RISK — invited, never finished setup, no certain fallback:")
        by_domain = {}
        for r in at_risk:
            by_domain.setdefault(r["domain"], []).append(r["handle"])
        for dom in sorted(by_domain, key=lambda d: (-len(by_domain[d]), d)):
            print("    %-28s %3d  %s" % (dom, len(by_domain[dom]), " ".join(sorted(by_domain[dom]))))
        print("")
        print("  These are handles, not verdicts. Some are synthetic/bridge accounts and")
        print("  some people may hold a Google Workspace account on their own domain; the")
        print("  audit cannot tell from here. What it can tell you is that none of them")
        print("  has ever set a credential on this door.")
    else:
        print("  No at-risk accounts: everyone without a credential has a Google fallback.")
    if replan and at_risk:
        print("")
        print("  Re-invite each with a fresh code (paste the two lines it prints, never a link):")
        for r in at_risk:
            print("    ops/scripts/kanidm-reissue-code.sh %s --plane %s" % (r["handle"], plane))

if prom:
    # A last-run timestamp ships with the counts on purpose: a timer that
    # silently died then reads as a number that stopped moving, rather than as
    # the absence of a problem (the mesh-membership-reconcile lesson, F-335).
    lines = [
        '# HELP goat_door_persons Persons present on this Kanidm door.',
        '# TYPE goat_door_persons gauge',
        'goat_door_persons{plane="%s"} %d' % (plane, total),
        '# HELP goat_door_persons_no_credential Persons who have never completed credential setup.',
        '# TYPE goat_door_persons_no_credential gauge',
        'goat_door_persons_no_credential{plane="%s"} %d' % (plane, len(nocred)),
        '# HELP goat_door_persons_at_risk No credential on this door AND no certain second login method.',
        '# TYPE goat_door_persons_at_risk gauge',
        'goat_door_persons_at_risk{plane="%s"} %d' % (plane, len(at_risk)),
        '# HELP goat_door_audit_last_run_seconds Unix time of the last completed door audit.',
        '# TYPE goat_door_audit_last_run_seconds gauge',
        'goat_door_audit_last_run_seconds{plane="%s"} %d' % (plane, int(time.time())),
        '',
    ]
    tmp = prom + ".tmp"
    with open(tmp, "w") as fh:
        fh.write("\n".join(lines))
    os.replace(tmp, prom)   # atomic: node_exporter must never read a half file
    os.chmod(prom, 0o644)
PY

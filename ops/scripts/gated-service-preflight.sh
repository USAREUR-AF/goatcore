#!/usr/bin/env bash
# gated-service-preflight.sh — run BEFORE deploying an OIDC-gated service to a
# goatnet. Every check below is a real failure mode of standing up a gated
# service, in the order it bites; each one costs a deploy round-trip or an outage.
#
#   ./ops/scripts/gated-service-preflight.sh <goatnet> <service> [--fqdn <public-fqdn>] [--issuer <url>]
#
# --issuer is the OIDC issuer the gate signs in against (check 8). Left out, it
# is read from the goatnet's group_vars (<service>_public_oidc_issuer).
#
# e.g. ./ops/scripts/gated-service-preflight.sh site-a relay_health \
#          --fqdn relay-health.example.net
#
# Exit 0 = every check passed. Non-zero = at least one FAIL; each names the
# failure it prevents so the fix is obvious rather than archaeological.
#
# WHY A SCRIPT AND NOT A CI CHECK: over half of these are only answerable
# against LIVE state — is the secret in *this shell*, does the running Dex know
# this callback, is there EIP headroom. CI cannot see any of that, which is
# precisely why they kept reaching production. CI covers the repo-side half
# (ops/ci/check-group-vars-parity.sh); this covers the half CI structurally
# cannot.
set -uo pipefail

GOATNET="${1:-}"; SERVICE="${2:-}"; shift 2 2>/dev/null || true
FQDN=""; ISSUER=""
while [ $# -gt 0 ]; do case "$1" in --fqdn) FQDN="$2"; shift 2;; --issuer) ISSUER="$2"; shift 2;; *) shift;; esac; done

if [ -z "$GOATNET" ] || [ -z "$SERVICE" ]; then
  echo "usage: $0 <goatnet> <service> [--fqdn <public-fqdn>] [--issuer <url>]" >&2
  echo "  e.g. $0 site-a relay_health --fqdn relay-health.example.net" >&2
  exit 2
fi

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GV_DIR="$REPO/ops/ansible/group_vars"
UPPER="$(echo "$GOATNET" | tr 'a-z-' 'A-Z_')"
FAIL=0; WARN=0
ok(){   printf '  \033[32mok\033[0m    %s\n' "$1"; }
fail(){ printf '  \033[31mFAIL\033[0m  %s\n     ↳ %s\n' "$1" "$2"; FAIL=$((FAIL+1)); }
warn(){ printf '  \033[33mwarn\033[0m  %s\n     ↳ %s\n' "$1" "$2"; WARN=$((WARN+1)); }
sec(){  printf '\n\033[1m%s\033[0m\n' "$1"; }

echo "gated-service preflight — goatnet=$GOATNET service=$SERVICE${FQDN:+ fqdn=$FQDN}"

# ---------------------------------------------------------------------------
sec "1. enablement declared (absent means DESTROY, not skip)"
# The role's sunset path fires on a FALSY flag and ends the play with
# `meta: end_host`, BEFORE the asserts that would fail loudly. A surface stood
# up with `-e` overrides therefore gets deleted by the next routine re-run.
HITS="$(grep -rl "^${SERVICE}_enabled:" "$GV_DIR" 2>/dev/null | grep -i "$GOATNET" || true)"
if [ -n "$HITS" ]; then ok "${SERVICE}_enabled declared in group_vars"
else fail "${SERVICE}_enabled NOT declared for $GOATNET" \
     "A re-run will take the teardown path and remove the containers. Declare it in group_vars — never with -e."; fi

# ---------------------------------------------------------------------------
sec "2. secrets resolvable IN THIS SHELL (not merely present in an envrc)"
# Writing a value into .envrc.local.d does NOT put it in an already-loaded
# shell. Three deploys failed one assert at a time on this.
SECRET_VARS="$(grep -rhoE "lookup\('env', *'${UPPER}_[A-Z0-9_]+'" "$GV_DIR" 2>/dev/null \
                | grep -oE "${UPPER}_[A-Z0-9_]+" | sort -u || true)"
if [ -z "$SECRET_VARS" ]; then warn "no ${UPPER}_* env lookups found" "Nothing to check, or the goatnet prefix differs."
else
  for v in $SECRET_VARS; do
    if [ -n "${!v:-}" ]; then ok "$v resolvable"
    else fail "$v is EMPTY in this shell" \
         "It may exist in .envrc.local.d and simply not be loaded. Re-source it, or the deploy fails an assert mid-run."; fi
  done
fi

# ---------------------------------------------------------------------------
sec "3. cookie secret length (oauth2-proxy crash-loops otherwise)"
# Must be EXACTLY 16/24/32 BYTES. `openssl rand -base64 32` gives 44 chars and
# the container crash-loops with a message that does not mention base64.
for v in $SECRET_VARS; do
  case "$v" in *COOKIE_SECRET*)
    VAL="${!v:-}"; N=${#VAL}
    if [ "$N" -eq 16 ] || [ "$N" -eq 24 ] || [ "$N" -eq 32 ]; then ok "$v is $N bytes"
    elif [ "$N" -eq 0 ]; then : # already reported above
    else fail "$v is $N bytes (needs 16/24/32)" "Use \`openssl rand -hex 16\`, not \`rand -base64 32\`."; fi;;
  esac
done

# ---------------------------------------------------------------------------
sec "4. gate allowlist non-empty AND verified against a real login"
# Fail-closed by design. But non-empty is not the same as CORRECT: an allowlist
# seeded with a plausible-looking address that is not the one the IdP
# asserts, which 403s AFTER a successful login — indistinguishable from a
# broken gate unless you read the proxy log.
AL_VAR="$(echo "$SECRET_VARS" | grep -E 'ALLOWED_EMAILS' | head -1 || true)"
if [ -z "$AL_VAR" ]; then warn "no *_ALLOWED_EMAILS var found" "If this service has a public gate, it needs one."
elif [ -z "${!AL_VAR:-}" ]; then fail "$AL_VAR empty" "The gate refuses to deploy a surface admitting any authenticated user."
else
  ok "$AL_VAR set ($(echo "${!AL_VAR}" | tr ',' '\n' | grep -c . ) entr(y|ies))"
  warn "allowlist addresses are NOT verified here" \
       "They must be the address the IdP ASSERTS, not the person's usual email. Confirm with one real login; on a 403 read the proxy log for the actual address before editing."
fi

# ---------------------------------------------------------------------------
sec "5. OIDC-discovery host entries do not pin a mesh IP"
# A mesh IP frozen into extra_hosts goes stale on re-enrollment; the token
# exchange then dials an address belonging to no peer and every login 403s
# after a 60s timeout. Correct where the issuer is on ANOTHER host; wrong where
# it is co-resident.
BAD="$(grep -rn "host_entry:.*mesh_ip" "$GV_DIR" 2>/dev/null | grep -i "$GOATNET" || true)"
if [ -z "$BAD" ]; then ok "no mesh-IP-pinned host entries for $GOATNET"
else warn "mesh-IP-pinned host entry present" \
     "Correct only if the OIDC issuer is on a DIFFERENT host. Co-resident ⇒ use host-gateway. Offenders:
$BAD"; fi

# ---------------------------------------------------------------------------
sec "6. publisher/ingest pairs — both halves"
# Each band is a two-part contract and NEITHER half validates the other. The
# allowlist is fail-closed, so a name mismatch is indistinguishable from
# silence: the panel is simply blank.
# relay_metrics is the pair I MISSED when first writing this check, and it cost
# a blank throughput column that survived every other fix: the ingest allowlist
# was set on the box while the relays themselves had publish_enabled=false.
# Its publisher half lives in the RELAY inventory, not group_vars — which is
# exactly why eyeballing group_vars did not reveal it.
RELAY_PUB="$(grep -rh 'relay_metrics_publish_enabled' "$REPO"/ops/ansible/inventory/*"$GOATNET"* 2>/dev/null | tail -1 || true)"
if [ -n "$RELAY_PUB" ]; then
  case "$RELAY_PUB" in *=true) ok "relays self-publish metrics ($GOATNET)";;
    *) fail "relay_metrics_publish_enabled is FALSE for $GOATNET" \
       "The dashboard's peer-count and throughput columns render EMPTY. Set it in ops/ansible/inventory/*${GOATNET}*-alias.ini and redeploy the relays.";;
  esac
fi

for pair in "zenoh_health_ingest_urls:zenoh_health_allowed_names" \
            "host_net_ingest_urls:host_net_allowed_names"; do
  U="${pair%%:*}"; A="${pair##*:}"
  HAS_U="$(grep -rl "^${U}:" "$GV_DIR" 2>/dev/null | grep -i "$GOATNET" || true)"
  HAS_A="$(grep -rl "^${A}:" "$GV_DIR" 2>/dev/null | grep -i "$GOATNET" || true)"
  if [ -n "$HAS_U" ] && [ -n "$HAS_A" ]; then ok "$U + $A both declared"
  elif [ -z "$HAS_U" ] && [ -z "$HAS_A" ]; then ok "$U pair not used by $GOATNET"
  else fail "only one half of $U / $A declared" \
       "The band renders BLANK, not an error. Declare both, and make the published name appear in the allowlist."; fi
done

# ---------------------------------------------------------------------------
sec "7. no environment-specific role default silently inherited"
# A role default naming ONE environment's artifact is a bug, not a default: the
# fixed relays.conf makes one goatnet's dashboard render another's relays under
# its own vantage — a populated panel describing the wrong infrastructure,
# The goatnet says which relay list is its own by declaring
# relay_health_relays_conf in its group_vars; one that declares nothing would
# inherit the role default, which is one goatnet's fleet.
if [ "$SERVICE" = "relay_health" ]; then
  CONF="$(grep -rh "^relay_health_relays_conf:" "$GV_DIR"/*"$(echo "$GOATNET" | tr '-' '_')"* 2>/dev/null | awk '{print $2}' | head -1)"
  if [ -n "$CONF" ]; then ok "relays conf declared ($CONF)"
  else fail "relay_health_relays_conf not declared for $GOATNET" \
       "It will probe the fleet the role default names and label the results with THIS goatnet's vantage."; fi
fi

# ---------------------------------------------------------------------------
sec "8. Dex knows the callback — ask DEX, not the dashboard"
# A dashboard that 302s to the IdP proves NOTHING: the redirect URI lives in the
# infra repo's terraform, and committing it is not applying it. Landing only the commit gives a surface with
# committing it is not applying it. Landing only the commit gives a surface with
# a valid cert and a clean redirect that dead-ends at "Unregistered
# redirect_uri" after the user clicks.
# Which Dex: the issuer this goatnet declares for the service's public gate
# (<service>_public_oidc_issuer in its group_vars), or --issuer.
if [ -z "$ISSUER" ]; then
  ISSUER="$(grep -rh "^${SERVICE}_public_oidc_issuer:" "$GV_DIR"/*"$(echo "$GOATNET" | tr '-' '_')"* 2>/dev/null | head -1 | cut -d' ' -f2- | tr -d '"' || true)"
fi
ISSUER="${ISSUER%/}"
if [ -z "$FQDN" ]; then warn "no --fqdn given" "Cannot check the Dex callback registration; pass --fqdn to enable it."
elif [ -z "$ISSUER" ]; then warn "no ${SERVICE}_public_oidc_issuer declared for $GOATNET" "Cannot check the Dex callback registration; declare it in group_vars or pass --issuer <url>."
else
  CB="https://${FQDN}/oauth2/callback"
  ENC="$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$CB")"
  RESP="$(curl -s --max-time 20 "${ISSUER}/auth?client_id=oauth2-proxy&response_type=code&scope=openid&redirect_uri=${ENC}" 2>/dev/null || true)"
  if [ -z "$RESP" ]; then warn "could not reach ${ISSUER}" "Network, or Dex is down. Re-run before deploying."
  elif echo "$RESP" | grep -qi 'Unregistered redirect_uri'; then
    fail "Dex does NOT know $CB" \
      "Add it to oauth2_proxy_redirect_uris in the infra repository's identity terraform (environments/aws/prod/main_identity.tf), then APPLY IT:
       terraform -chdir=environments/aws/prod apply -target=module.dex
       Use -target: that shared tree routinely carries unrelated pending work (it wanted to replace the compliance-dashboard Lambda on 2026-08-07)."
  else ok "Dex accepts $CB"; fi
fi

# ---------------------------------------------------------------------------
sec "9. public DNS + cert (only when a public FQDN is claimed)"
if [ -n "$FQDN" ]; then
  IP="$(dig +short A "$FQDN" @1.1.1.1 2>/dev/null | head -1)"
  if [ -n "$IP" ]; then ok "$FQDN resolves ($IP)"
  else warn "$FQDN does not resolve" "Expected before the terraform apply that creates the record; a FAIL afterwards."; fi
fi

# ---------------------------------------------------------------------------
sec "10. Elastic IP headroom (only if this deploy needs one)"
# An apply that allocates an EIP fails HALFWAY on AddressLimitExceeded, leaving
# an instance with no address — recoverable, but a surprise mid-apply.
if command -v aws >/dev/null 2>&1 && [ -n "${AWS_REGION:-eu-central-1}" ]; then
  Q="$(aws service-quotas get-service-quota --region "${AWS_REGION:-eu-central-1}" --service-code ec2 \
        --quota-code L-0263D0A3 --query 'Quota.Value' --output text 2>/dev/null || echo "")"
  U="$(aws ec2 describe-addresses --region "${AWS_REGION:-eu-central-1}" --query 'length(Addresses)' --output text 2>/dev/null || echo "")"
  if [ -n "$Q" ] && [ -n "$U" ]; then
    if [ "${U%.*}" -lt "${Q%.*}" ]; then ok "EIP headroom ${U}/${Q%.*}"
    else fail "EIP quota exhausted (${U}/${Q%.*})" \
         "An apply needing an EIP dies halfway. Request an increase first: aws service-quotas request-service-quota-increase --service-code ec2 --quota-code L-0263D0A3 --desired-value <n>"; fi
  else warn "could not read EIP quota" "No AWS credentials in this shell, or the call failed."; fi
fi

# ---------------------------------------------------------------------------
sec "11. terraform state is not split across open branches"
# Two open PRs each holding PART of one tree's config share ONE state, so a plan
# from either alone proposes DESTROYING the other's resources. This nearly
# deleted a live Google Search Console verification record mid-review.
BRANCHES="$(git -C "$REPO" branch -r --list 'origin/track/*' 2>/dev/null | wc -l | tr -d ' ')"
if [ "${BRANCHES:-0}" -gt 1 ]; then
  warn "$BRANCHES track branches exist" \
    "If more than one touches the SAME terraform tree, plan from a worktree MERGING them — never from one alone. Verify 'destroy: 0' before applying."
else ok "no split-state risk detected"; fi

# ---------------------------------------------------------------------------
sec "12b. every ingest URL has had its PUBLISHER redeployed"
# Changing an ingest URL in group_vars moves NOTHING until the publisher that
# reads it is redeployed. A dashboard can serve empty band files for hours while
# publishers post to the old one, because the URLs were repointed in the repo
# and the publisher host was not re-run.
# The symptom is a dashboard that renders with most panels blank — easy to read
# as "not built yet" rather than "built and pointed elsewhere".
for u in zenoh_health_ingest_urls host_net_ingest_urls relay_peer_names_ingest_urls; do
  V="$(grep -rh "^${u}:" "$GV_DIR"/*"$(echo "$GOATNET" | tr '-' '_')"* 2>/dev/null | head -1 | cut -d' ' -f2- | tr -d '"' || true)"
  if [ -z "$V" ]; then warn "$u not set for $GOATNET" "That band will be blank on the dashboard."
  else ok "$u -> $V"; fi
done
warn "confirm each publisher was REDEPLOYED after the URL last changed" \
  "Repointing group_vars is not a deploy. Check the band files are non-trivial on the RECEIVER:
     ls -la <www>/ingest/   # a ~40-byte hosts.json or zenoh-health.json is an EMPTY SHELL, not data"

# ---------------------------------------------------------------------------
sec "12. running process actually holds the config on disk"
# The failure that outlasted every other fix. A bind-mounted config file changes
# on disk, `docker compose up -d` sees no DEFINITION change and leaves the
# container running, and the process keeps serving the OLD value. oauth2-proxy
# even logs "watching <file> for updates" — but ansible writes atomically by
# renaming a temp file over the target, which swaps the inode out from under the
# watch. Symptom: a 403 AFTER a successful IdP login, indistinguishable from a
# broken gate. Tell: the container's uptime predates your last deploy.
if [ -n "$FQDN" ] && command -v aws >/dev/null 2>&1; then
  warn "verify container uptime post-deploy" \
    "After changing any bind-mounted config (allowlists, ingest scripts), confirm the container actually restarted:
       docker ps --format '{{.Names}}\t{{.Status}}' | grep oauth2
     An uptime older than your deploy means the process is still serving the previous config. Restart it explicitly."
fi

# ---------------------------------------------------------------------------
printf '\n'
if [ "$FAIL" -gt 0 ]; then
  printf '\033[31mPREFLIGHT FAILED\033[0m — %d fail, %d warn. Fix the FAILs before deploying.\n' "$FAIL" "$WARN"; exit 1
fi
printf '\033[32mPREFLIGHT PASSED\033[0m — 0 fail, %d warn. Warnings are judgement calls, not blockers.\n' "$WARN"

#!/usr/bin/env bash
# generate-relays.sh — Generic Ansible inventory generator for VPS relays.
#
# Cloud-env-agnostic: takes the env name + Terraform tree path + AWS profile
# as args and emits a per-env inventory file. Replaces per-env generators
# (generate-prod.sh, etc.) with a single parameterized script that scales
# to any future deployment without per-env clones.
#
# Usage:
#   ops/ansible/inventory/generate-relays.sh <env> <terraform_dir> [aws_auth_mode]
#
# Args:
#   <env>             Env identifier (e.g. legacy, prod, future-name).
#                     Used to name the inventory file + SSH key dir +
#                     relay-env.sh + AWS Secrets Manager prefix.
#   <terraform_dir>   Path to the Terraform tree (relative to repo root)
#                     that owns the relays. The tree must expose a `relays`
#                     output map of {name → {public_ip, region, instance_id,
#                     ssh_key_secret_name, rathole_secret_name, relay_url}}.
#   [aws_auth_mode]   Optional. One of:
#                       envvars   — read AWS_ACCESS_KEY_<ENV-UPPER> + secret
#                                   from .envrc.local (default if those exist)
#                       aws-vault — use `aws-vault exec <profile> --no-session`
#                                   where <profile> resolves in this order:
#                                     (1) AWS_VAULT_PROFILE_<ENV_UPPER> env var
#                                     (2) GOAT_AWS_PROFILE env var
#                                     (3) deploy.aws_profile in the checkout's
#                                         ops/goatbuilder/goatbuilder.yaml
#                                     (4) hardcoded mapping (prod → usareur-af)
#                                     (5) env name (legacy → legacy)
#                     Default: envvars if present, else aws-vault.
#
# Examples:
#   ops/ansible/inventory/generate-relays.sh prod terraform-prod
#   ops/ansible/inventory/generate-relays.sh legacy terraform aws-vault
#   ops/ansible/inventory/generate-relays.sh region-b terraform-region-b
#
# Outputs:
#   inventory/hosts-<env>.ini    Ansible inventory with [relay_servers] group
#   <state_dir>/relay-ssh-keys/<env>/
#                                SSH private keys per relay (dir 0700, files
#                                0600). state_dir = deploy.state_dir in
#                                ops/goatbuilder/goatbuilder.yaml, else
#                                ~/.goat/state; RELAY_SSH_KEY_DIR overrides.
#                                Never under /tmp: a copy an earlier run left in
#                                /tmp/<env>-ssh-keys/ is removed.
#   ops/relay-env-<env>.sh       Bash-friendly relay metadata
#
# Pre-requisites:
#   - <terraform_dir>/  has been `terraform apply`-ed (relays exist + outputs work)
#   - AWS auth available (env vars per .envrc.local OR aws-vault profile)
#   - the env has a declared goat-relay TLS posture in relay-tls-posture.sh.
#     There is NO default: an undeclared env exits 71 rather than inherit the
#     role's self-signed fallback, which the netbird relay client rejects
#     against the system trust store (Relays 0/N — F-188). Override for one
#     run with RELAY_TLS_POSTURE_<ENV_UPPER>=le-dns01.
#
# Exit codes: 64 usage · 65 missing terraform dir · 66 missing AWS env creds ·
#   67 unknown auth mode · 68/69 unreadable terraform output ·
#   70 missing posture map · 71 undeclared posture · 72 invalid posture.
#
# Tests: ops/ansible/inventory/tests/relay-tls-posture-test.sh (hermetic; stubs
# terraform/aws/aws-vault, diffs the generated inventory against goldens).
#
# Idempotent — safe to re-run. Overwrites the inventory + key files.

set -euo pipefail

if [ $# -lt 2 ]; then
  cat >&2 <<USAGE
Usage: $0 <env> <terraform_dir> [aws_auth_mode]
       $0 prod terraform-prod
       $0 legacy terraform aws-vault
       $0 region-b terraform-region-b
USAGE
  exit 64
fi

ENV_NAME="$1"
TF_DIR_RELATIVE="$2"
AUTH_MODE="${3:-auto}"

ENV_UPPER=$(echo "$ENV_NAME" | tr '[:lower:]-' '[:upper:]_')

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ANSIBLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$ANSIBLE_DIR/../.." && pwd)"
TF_DIR="$REPO_DIR/$TF_DIR_RELATIVE"
INVENTORY_FILE="$SCRIPT_DIR/hosts-${ENV_NAME}.ini"
PROJECT_SETTING="$REPO_DIR/ops/scripts/project-setting.sh"
project_setting() {  # project_setting <section> <key> — empty when undeclared
  [ -x "$PROJECT_SETTING" ] || [ -f "$PROJECT_SETTING" ] || return 0
  bash "$PROJECT_SETTING" --repo "$REPO_DIR" "$@" 2>/dev/null || true
}
STATE_DIR="$(project_setting deploy state_dir)"
STATE_DIR="${STATE_DIR:-~/.goat/state}"
# shellcheck disable=SC2088  # expand a leading ~ the declaration may carry
case "$STATE_DIR" in "~"|"~/"*) STATE_DIR="$HOME${STATE_DIR#\~}" ;; esac
KEY_DIR="${RELAY_SSH_KEY_DIR:-${STATE_DIR%/}/relay-ssh-keys/${ENV_NAME}}"
LEGACY_TMP_KEY_DIR="/tmp/${ENV_NAME}-ssh-keys"
RELAY_ENV_FILE="$REPO_DIR/ops/relay-env-${ENV_NAME}.sh"

if [ ! -d "$TF_DIR" ]; then
  echo "ERROR: terraform dir not found: $TF_DIR" >&2
  exit 65
fi

# --- goat-relay TLS posture — DECLARED per env, never inferred ---------------
#
# The netbird relay client validates the rels:// server cert against the SYSTEM
# TRUST STORE, so a self-signed cert on a peer-facing tier is rejected and no
# peer ever connects (Relays 0/N — F-188). The goat-relay role's own default is
# self-signed, which is right for a bench and wrong for a goatnet. This used to
# be a hardcoded `if [[ "$ENV_NAME" == "zagros" ]]` branch further down, so every
# other env — including every future one — inherited the bench default silently.
#
# Postures live in relay-tls-posture.sh, one row per env with the reasoning on
# the row. Resolution order: per-run env-var override, else the declared row.
# An env with neither is a HARD FAILURE — the fail-closed half of the fix.
#
# Resolved HERE, before the first AWS or terraform call, so an undeclared env
# costs nothing and leaves no half-written inventory behind. The emission
# itself is at the bottom, with the other per-env inventory var blocks.
POSTURE_FILE="$SCRIPT_DIR/relay-tls-posture.sh"
if [ ! -f "$POSTURE_FILE" ]; then
  echo "ERROR: relay TLS posture map not found: $POSTURE_FILE" >&2
  exit 70
fi
# shellcheck source=relay-tls-posture.sh
source "$POSTURE_FILE"

POSTURE_OVERRIDE_VAR="RELAY_TLS_POSTURE_${ENV_UPPER}"
RELAY_TLS_POSTURE_RESOLVED=""
if [ -n "${!POSTURE_OVERRIDE_VAR:-}" ]; then
  RELAY_TLS_POSTURE_RESOLVED="${!POSTURE_OVERRIDE_VAR}"
  POSTURE_SOURCE="env override \$$POSTURE_OVERRIDE_VAR"
elif RELAY_TLS_POSTURE_RESOLVED=$(relay_tls_posture_for "$ENV_NAME"); then
  POSTURE_SOURCE="relay-tls-posture.sh [$ENV_NAME]"
else
  cat >&2 <<POSTURE_UNDECLARED
ERROR: env '$ENV_NAME' declares no goat-relay TLS posture.

  There is no default, deliberately. The goat-relay role falls back to
  self-signed, which the netbird relay client rejects against the system trust
  store — an environment whose containers are all up and whose peers all read
  "Relays 0/N".

  Declare it, with a reason, in:
    ops/ansible/inventory/relay-tls-posture.sh   ->   $ENV_NAME) echo le-dns01 ;;

  Values: le-dns01 (publicly-trusted LE via Cloudflare DNS-01; what a
  peer-facing goatnet needs) or self-signed (bench / first-light only).

  For a single run without editing the map:
    $POSTURE_OVERRIDE_VAR=le-dns01 $0 $ENV_NAME $TF_DIR_RELATIVE
POSTURE_UNDECLARED
  exit 71
fi

case "$RELAY_TLS_POSTURE_RESOLVED" in
  le-dns01|self-signed) ;;
  *)
    echo "ERROR: invalid relay TLS posture '$RELAY_TLS_POSTURE_RESOLVED' for env '$ENV_NAME'" >&2
    echo "       (from $POSTURE_SOURCE). Expected: le-dns01 or self-signed." >&2
    exit 72
    ;;
esac

echo "==> goat-relay TLS posture: $RELAY_TLS_POSTURE_RESOLVED (from $POSTURE_SOURCE)"

# Source .envrc.local for env-var-based AWS auth
if [ -f "$REPO_DIR/.envrc.local" ]; then
  set -a
  source "$REPO_DIR/.envrc.local" 2>/dev/null || true
  set +a
fi

# Resolve AWS auth mode
ACCESS_KEY_VAR="AWS_ACCESS_KEY_${ENV_UPPER}"
SECRET_KEY_VAR="AWS_SECRET_ACCESS_KEY_${ENV_UPPER}"
if [ "$AUTH_MODE" = "auto" ]; then
  if [ -n "${!ACCESS_KEY_VAR:-}" ] && [ -n "${!SECRET_KEY_VAR:-}" ]; then
    AUTH_MODE="envvars"
  else
    AUTH_MODE="aws-vault"
  fi
fi

if [ "$AUTH_MODE" = "envvars" ]; then
  if [ -z "${!ACCESS_KEY_VAR:-}" ] || [ -z "${!SECRET_KEY_VAR:-}" ]; then
    echo "ERROR: $ACCESS_KEY_VAR / $SECRET_KEY_VAR not set in .envrc.local" >&2
    exit 66
  fi
  export AWS_ACCESS_KEY_ID="${!ACCESS_KEY_VAR}"
  export AWS_SECRET_ACCESS_KEY="${!SECRET_KEY_VAR}"
  export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-eu-central-1}"
  # Terraform's S3 backend reads AWS_REGION, not AWS_DEFAULT_REGION; without
  # this, backend falls back to provider region resolution and emits a
  # deprecation warning to stdout that corrupts JSON-capturing assignments.
  export AWS_REGION="${AWS_REGION:-$AWS_DEFAULT_REGION}"
  AWS_WRAP=""
elif [ "$AUTH_MODE" = "aws-vault" ]; then
  # Resolve aws-vault profile name. The script's original assumption — that
  # `<env-name>` is also the aws-vault profile name — holds for legacy
  # (env=legacy, profile=legacy) but breaks for prod (env=prod,
  # profile=usareur-af per CLAUDE.md "usareur-af prod creds: aws-vault add
  # usareur-af"). The fix layers two resolution paths:
  #
  #   1. AWS_VAULT_PROFILE_<ENV_UPPER> env var (per .envrc.local) — explicit
  #      override. Set this for any env whose profile name diverges from
  #      env name. Example: AWS_VAULT_PROFILE_PROD=usareur-af.
  #   2. Hardcoded known-mappings fallback for the named-historical envs.
  #      Currently: prod → usareur-af. Future env-vs-profile drift adds an
  #      env-var override per (1) — DO NOT extend this fallback further;
  #      it's load-bearing-by-historical-accident only.
  #   3. Otherwise: env name = profile name (the original assumption).
  #   Between (1) and (2): GOAT_AWS_PROFILE, then the checkout's declared
  #   deploy.aws_profile (ops/goatbuilder/goatbuilder.yaml), the same profile
  #   goatbuilder runs every ansible call under (crates/goatbuilder/src/settings.rs).
  AWS_VAULT_PROFILE_VAR="AWS_VAULT_PROFILE_${ENV_UPPER}"
  DECLARED_AWS_PROFILE="$(project_setting deploy aws_profile)"
  # The source is recorded so a failure can say WHERE the profile name came
  # from, which is the difference between "you set it wrong" and "nobody set
  # it at all".
  if [ -n "${!AWS_VAULT_PROFILE_VAR:-}" ]; then
    AWS_VAULT_PROFILE="${!AWS_VAULT_PROFILE_VAR}"
    AWS_AUTH_SOURCE="\$$AWS_VAULT_PROFILE_VAR"
  elif [ -n "${GOAT_AWS_PROFILE:-}" ]; then
    AWS_VAULT_PROFILE="$GOAT_AWS_PROFILE"
    AWS_AUTH_SOURCE="\$GOAT_AWS_PROFILE"
  elif [ -n "$DECLARED_AWS_PROFILE" ]; then
    AWS_VAULT_PROFILE="$DECLARED_AWS_PROFILE"
    AWS_AUTH_SOURCE="deploy.aws_profile in ops/goatbuilder/goatbuilder.yaml"
  elif [ "$ENV_NAME" = "prod" ]; then
    AWS_VAULT_PROFILE="usareur-af"
    AWS_AUTH_SOURCE="the prod->usareur-af fallback"
  else
    AWS_VAULT_PROFILE="$ENV_NAME"
    AWS_AUTH_SOURCE="the env name, because \$$AWS_VAULT_PROFILE_VAR, \$GOAT_AWS_PROFILE and deploy.aws_profile are unset"
  fi
  AWS_WRAP="aws-vault exec ${AWS_VAULT_PROFILE} --no-session --"
  echo "==> aws-vault profile: $AWS_VAULT_PROFILE (from $AWS_AUTH_SOURCE)"
else
  echo "ERROR: unknown aws_auth_mode '$AUTH_MODE' (expected envvars or aws-vault)" >&2
  exit 67
fi

# Resolve provider tokens that the Terraform tree's providers will expect
# during init. Even read-only `terraform output` may initialize providers; an
# unconfigured provider can emit error chatter to stdout that corrupts our
# JSON-capturing assignments. Generic pattern: <PROVIDER>_<ENV_UPPER> →
# canonical <PROVIDER> env var if the canonical isn't already set.
CLOUDFLARE_TOKEN_VAR="CLOUDFLARE_API_TOKEN_${ENV_UPPER}"
if [ -z "${CLOUDFLARE_API_TOKEN:-}" ] && [ -n "${!CLOUDFLARE_TOKEN_VAR:-}" ]; then
  export CLOUDFLARE_API_TOKEN="${!CLOUDFLARE_TOKEN_VAR}"
fi

echo "==> env=$ENV_NAME tf=$TF_DIR auth=$AUTH_MODE"

# Read relays output (canonical map shape).
#
# Use -json for both the map output (RELAY_JSON) AND the scalar bucket
# (SSM_BUCKET-via-jq-lite-python), so that any provider-init chatter terraform
# emits to stdout cannot pollute the captured value — we always parse JSON
# and only accept correctly-shaped data. -raw mode would silently embed any
# stdout pollution into the variable.
echo "==> Reading 'relays' output from $TF_DIR_RELATIVE..."
cd "$TF_DIR"
RELAY_JSON=$($AWS_WRAP terraform output -json relays 2>/dev/null \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d))" 2>/dev/null || echo "")
SSM_BUCKET=$($AWS_WRAP terraform output -json ansible_ssm_bucket 2>/dev/null \
  | python3 -c "import sys,json; print(json.load(sys.stdin))" 2>/dev/null || echo "")

# Re-run the SAME command with stderr KEPT, only to have something true to say
# if the read came back empty. Both reads above send stderr to /dev/null so a
# noisy-but-successful terraform does not corrupt the JSON — which also throws
# away the one line that says WHY an empty result happened.
# The capture ends in `|| true`: under `set -euo pipefail` a failing aws-vault
# or terraform would otherwise end the script right here, silently, which is
# the one moment its error is the answer.
TF_STDERR=""
if [ -z "$RELAY_JSON" ] || [ "$RELAY_JSON" = "{}" ] || [ -z "$SSM_BUCKET" ]; then
  TF_STDERR=$($AWS_WRAP terraform output -json relays 2>&1 >/dev/null | head -8 || true)
fi

if [ -z "$RELAY_JSON" ] || [ "$RELAY_JSON" = "{}" ]; then
  echo "ERROR: could not read the 'relays' output from $TF_DIR_RELATIVE." >&2
  echo >&2
  if [ -n "$TF_STDERR" ]; then
    echo "  ${AWS_WRAP:+aws-vault / }terraform said:" >&2
    printf '%s\n' "$TF_STDERR" | sed 's/^/    /' >&2
    echo >&2
  else
    echo "  (${AWS_WRAP:+aws-vault and }terraform printed no error.)" >&2
    echo >&2
  fi
  # The old text said only "Apply first, or confirm the tree exposes a 'relays'
  # map output". On 2026-09-08 that was printed for zagros-dev while the tree WAS
  # applied and the output WAS perfect — the real cause was that no
  # AWS_VAULT_PROFILE_ZAGROS_DEV was set, so this script wrapped terraform in
  # `aws-vault exec zagros-dev`, a profile that does not exist. Naming the wrong
  # cause sent the reader to check the one thing that was already fine.
  echo "  Checked in this order:" >&2
  if [ "$AUTH_MODE" = "aws-vault" ]; then
  echo "    1. AWS profile — this run used '$AWS_VAULT_PROFILE' (from $AWS_AUTH_SOURCE)." >&2
  echo "       If that profile does not exist, terraform cannot reach the state" >&2
  echo "       backend and every output reads empty. Declare deploy.aws_profile in" >&2
  echo "       ops/goatbuilder/goatbuilder.yaml, or set" >&2
  echo "       AWS_VAULT_PROFILE_${ENV_UPPER}=<profile> for this env." >&2
  else
  echo "    1. AWS credentials — this run used \$$ACCESS_KEY_VAR / \$$SECRET_KEY_VAR." >&2
  fi
  echo "    2. terraform init — an uninitialised tree cannot read remote state." >&2
  echo "    3. apply — the tree may genuinely have no relays yet." >&2
  echo "    4. the output itself — confirm a 'relays' map output exists." >&2
  exit 68
fi

if [ -z "$SSM_BUCKET" ]; then
  echo "ERROR: 'ansible_ssm_bucket' output empty or unparseable in $TF_DIR_RELATIVE." >&2
  echo "       Apply first, or confirm the tree exposes 'ansible_ssm_bucket'." >&2
  if [ -n "$TF_STDERR" ]; then
    printf '%s\n' "$TF_STDERR" | sed 's/^/    /' >&2
  fi
  exit 69
fi

# Parse relay names + values
RELAY_NAMES=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(' '.join(sorted(json.load(sys.stdin).keys())))")
echo "==> Found relays: $RELAY_NAMES"

# Fetch SSH keys from Secrets Manager into a private directory under the
# user's state dir. A copy an earlier version of this script left in /tmp is
# removed: private keys do not stay in a world-listable directory.
if [ -d "$LEGACY_TMP_KEY_DIR" ] && [ -O "$LEGACY_TMP_KEY_DIR" ]; then
  rm -rf -- "$LEGACY_TMP_KEY_DIR"
  echo "==> Removed the old relay key copy in $LEGACY_TMP_KEY_DIR/"
fi
(umask 077 && mkdir -p "$KEY_DIR")
chmod 700 "$KEY_DIR"

for name in $RELAY_NAMES; do
  region=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['region'])")
  secret_name=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['ssh_key_secret_name'])")
  key_file="$KEY_DIR/${name}.pem"

  echo "==> Fetching SSH key for $name (region $region, secret $secret_name)"
  ( umask 077
    $AWS_WRAP aws secretsmanager get-secret-value \
      --secret-id "$secret_name" --region "$region" \
      --query SecretString --output text > "$key_file" 2>/dev/null ) || {
      echo "  WARNING: could not fetch $secret_name; key file empty"
    }
  chmod 600 "$key_file"
done

# Write inventory.
#
# NOTE: Heredoc interpolates $vars but NOT backticks (we want literal text
# referencing terraform commands, not command-substitution invocations of
# them). Use \$( ) or single-quoted phrasing for any command references.
cat > "$INVENTORY_FILE" <<HEADER
# Auto-generated by ops/ansible/inventory/generate-relays.sh
# env=$ENV_NAME  terraform=$TF_DIR_RELATIVE
# DO NOT EDIT — regenerated on every run.
#
# Connection: AWS SSM (SSH ports closed by default).
# Re-generate when relays change or after a 'terraform apply'.

[relay_servers]
HEADER

for name in $RELAY_NAMES; do
  instance_id=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['instance_id'])")
  region=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['region'])")
  rathole_secret=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['rathole_secret_name'])")
  relay_url=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['relay_url'])")
  key_file="$KEY_DIR/${name}.pem"

  # goat_relay_advertised_addr is also emitted here (not only in the
  # [goat_relay_hosts] block below) so an all-Rust relay tier that consumes
  # [relay_servers] via the per-env alias (e.g. inventory/zagros-goat-relay-
  # alias.ini: [goat_relay_hosts:children] relay_servers) carries the
  # advertised addr goat-relay-deploy.yml's preflight requires. Without it
  # the Rust relay could not deploy on ZAGROS — only the colocated rathole
  # ingress ran, leaving :443 enrollment working but :33073 dead.
  # Stock Go-relay tiers (nova-a, prod) ignore this var, so it is harmless there.
  echo "relay-${name} ansible_host=${instance_id} ansible_aws_ssm_region=${region} aws_region=${region} ansible_connection=community.aws.aws_ssm ansible_aws_ssm_s3_addressing_style=virtual ansible_aws_ssm_bucket_name=${SSM_BUCKET} ansible_ssh_private_key_file=${key_file} rathole_secret_name=${rathole_secret} relay_dns_name=${relay_url} goat_relay_advertised_addr=${relay_url}:33073" >> "$INVENTORY_FILE"
done

cat >> "$INVENTORY_FILE" <<RELAYVARS

[relay_servers:vars]
ansible_user=ubuntu
ansible_python_interpreter=/usr/bin/python3
# SSM session user lacks a writable home dir; default ~/.ansible/tmp triggers
# 'mkdir: cannot create directory /home/ubuntu: Permission denied' before
# become escalates. Forcing a /tmp-rooted remote_tmp dodges that.
ansible_remote_tmp=/tmp/.ansible/tmp
RELAYVARS

# Per-env relay-server flags. Legacy dev/test uses rathole for
# mgmt-API forwarding; usareur-af prod uses Block 22 wg-cp0 silent
# control plane (ADR 0313 / 0314), which replaces rathole's role.
# relay-server.yml gates the rathole stack via deploy_rathole (default
# true). See ops/ansible/relay-server-rathole.yml.
if [[ "$ENV_NAME" == "prod" ]]; then
  cat >> "$INVENTORY_FILE" <<RATHOLE_OFF

# usareur-af prod relays use Block 22 wg-cp0 instead of rathole.
deploy_rathole=false

# Block 22N — prod tier doesn't need the relay-side dnsmasq. Per-site
# mesh-internal DNS is per-site CoreDNS on Orin-A; relay-side dnsmasq
# only ever served the legacy single-NETHOST topology and is vestigial
# on prod (its addn-hosts zone resolves *.netbird.legacy.example to
# the legacy NETHOST at 100.127.1.23 — unreachable + irrelevant for
# prod peers). relay-server.yml fires the explicit stop+disable+purge
# path when this flag is false.
deploy_dnsmasq=false
RATHOLE_OFF
fi

# DNS-01 relays (first built for nova, now every goatnet whose SG closes
# port 80): validate the relay LE cert via Cloudflare DNS-01, not HTTP-01. The nova SG posture closes port 80 (http_port_open=false,
# DNS-01-only — docs/design/nova-public-posture.md), so certbot HTTP-01
# standalone can't validate. relay-server.yml branches on relay_cert_dns01
#. The nova deploy already passes -e cloudflare_api_token=...
# (Step 5), which the DNS-01 path consumes. Makes "add a relay region" turnkey.
# Emitted from the RESOLVED TLS POSTURE, not from a hardcoded env name. This
# was `if ENV_NAME == "nova"`, which was wrong in both directions:
# nova's declared posture is self-signed and it got the flag anyway, while
# zagros-dev, zagros, zagros-prod and ibex are all le-dns01 and got nothing. Without
# the flag relay-server.yml falls through to certbot HTTP-01 standalone, and
# every one of these goatnets closes port 80 by security-group posture, so the
# challenge times out — "Timeout during connect (likely firewall problem)".
#
# Measured 2026-09-08: zagros-dev's four relays failed exactly that way, which is
# why zagros-dev had no relay certificates and therefore no rels:// tier at all.
# The posture map already knew the right answer; nothing consumed it.
if [[ "$RELAY_TLS_POSTURE_RESOLVED" == "le-dns01" ]]; then
  cat >> "$INVENTORY_FILE" <<LE_DNS01_RELAY_VARS

# Posture le-dns01 ($POSTURE_SOURCE): validate the relay LE cert via Cloudflare
# DNS-01, because port 80 is closed by SG posture and HTTP-01 cannot validate.
# Consumed by relay-server.yml; the deploy passes
# -e cloudflare_api_token=\$CLOUDFLARE_API_TOKEN_<ENV_UPPER>.
relay_cert_dns01=true
LE_DNS01_RELAY_VARS
fi

# goat-relay TLS posture — resolved before any cloud call (see top of file).
#
# self-signed emits nothing: it IS the role default, so writing it would only
# restate the fallback. The protection is at declaration time, above — not here.
# That is also what keeps this refactor a byte-for-byte no-op: the emitted text
# below is the former zagros block with the env name substituted into the slot
# that used to read the literal "ZAGROS".
if [[ "$RELAY_TLS_POSTURE_RESOLVED" == "le-dns01" ]]; then
  cat >> "$INVENTORY_FILE" <<RELAY_TLS_VARS

# ${ENV_UPPER} all-Rust goat-relay: publicly-trusted LE cert via Cloudflare DNS-01
# (self-signed is rejected by netbird relay clients — F-188). Deploy passes
# -e cloudflare_api_token=\$CLOUDFLARE_API_TOKEN_${ENV_UPPER}.
goat_relay_le_dns01=true
goat_relay_self_signed_cert=false
RELAY_TLS_VARS
fi

# goat-relay tier (Block 33 / ADR 0712) — emit [goat_relay_hosts] group
# when the terraform tree exposes a `goat_relays` output. Consumed by
# ops/ansible/goat-relay-deploy.yml + goat-relay-tune.yml.
echo ""
echo "==> Reading 'goat_relays' output from $TF_DIR_RELATIVE..."
GOAT_RELAY_JSON=$($AWS_WRAP terraform output -json goat_relays 2>/dev/null \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d))" 2>/dev/null || echo "")

if [ -n "$GOAT_RELAY_JSON" ] && [ "$GOAT_RELAY_JSON" != "{}" ]; then
  GOAT_RELAY_NAMES=$(echo "$GOAT_RELAY_JSON" | python3 -c "import sys,json; print(' '.join(sorted(json.load(sys.stdin).keys())))")
  echo "==> Found goat-relays: $GOAT_RELAY_NAMES"

  echo "" >> "$INVENTORY_FILE"
  echo "[goat_relay_hosts]" >> "$INVENTORY_FILE"

  for name in $GOAT_RELAY_NAMES; do
    instance_id=$(echo "$GOAT_RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['instance_id'])")
    region=$(echo "$GOAT_RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['region'])")
    public_ip=$(echo "$GOAT_RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['public_ip'])")
    relay_url=$(echo "$GOAT_RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['relay_url'])")

    echo "goat-relay-${name} ansible_host=${instance_id} ansible_aws_ssm_region=${region} aws_region=${region} goat_relay_public_ip=${public_ip} goat_relay_advertised_addr=${relay_url}:33073" >> "$INVENTORY_FILE"
  done

  cat >> "$INVENTORY_FILE" <<GOATRELAYVARS

[goat_relay_hosts:vars]
ansible_connection=community.aws.aws_ssm
ansible_aws_ssm_s3_addressing_style=virtual
ansible_aws_ssm_bucket_name=${SSM_BUCKET}
ansible_remote_tmp=/tmp/.ansible/tmp
ansible_user=ubuntu
ansible_python_interpreter=/usr/bin/python3
ansible_aws_ssm_timeout=300
ansible_command_timeout=300
GOATRELAYVARS
else
  echo "==> No 'goat_relays' output (Block 33 tier not provisioned in $TF_DIR_RELATIVE). Skipping [goat_relay_hosts]."
fi

# Generate relay-env-<env>.sh — bash-friendly metadata
echo "==> Generating $RELAY_ENV_FILE"
{
  echo "# Relay metadata for env=$ENV_NAME — auto-generated by generate-relays.sh."
  echo "# Source from any bash script that needs relay names / IPs / regions."
  echo
  echo "RELAY_ENV_NAME=\"$ENV_NAME\""
  echo "RELAY_NAMES=($(for n in $RELAY_NAMES; do printf "'%s' " "$n"; done))"
  printf "RELAY_IPS=("
  for n in $RELAY_NAMES; do
    ip=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$n']['public_ip'])")
    printf "'%s' " "$ip"
  done
  echo ")"
  printf "RELAY_REGIONS=("
  for n in $RELAY_NAMES; do
    region=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$n']['region'])")
    printf "'%s' " "$region"
  done
  echo ")"
  printf "RELAY_INSTANCE_IDS=("
  for n in $RELAY_NAMES; do
    instance_id=$(echo "$RELAY_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['$n']['instance_id'])")
    printf "'%s' " "$instance_id"
  done
  echo ")"
  echo "RELAY_KEY_DIR=\"$KEY_DIR\""
  echo "RELAY_TF_DIR=\"$TF_DIR_RELATIVE\""
  echo "RELAY_AUTH_MODE=\"$AUTH_MODE\""
} > "$RELAY_ENV_FILE"

echo ""
echo "==> Inventory:  $INVENTORY_FILE"
echo "==> Relay env:  $RELAY_ENV_FILE"
echo "==> SSH keys:   $KEY_DIR/ (mode 0700; private keys, delete with the inventory)"
echo ""
echo "Next: ansible-playbook -i $INVENTORY_FILE relay-server.yml"

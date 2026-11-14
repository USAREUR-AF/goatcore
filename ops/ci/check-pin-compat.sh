#!/usr/bin/env bash
# ops/ci/check-pin-compat.sh
#
# Pin-regression guard. When docker-compose.yml pins an image to a digest,
# verify the pinned image's CLI surface is compatible with the compose
# `command:` block that consumes it.
#
# Caught failures this is designed to catch:
#   - signal-server pinned to sha-9df2a27 removed the
#     -federation-peers flag; compose still passed --federation-peers=,
#     signal-a crashlooped. Not discovered until live deploy.
#   - postgres-spock pinned to sha-cca1950 shipped an incomplete
#     pg_hba.conf missing host rules management-a needs; management-a
#     crashlooped with "no pg_hba.conf entry".
#
# What this script does:
#   1. Parses ops/compose/mgmt-host/docker-compose.yml
#   2. For each service with image: + command:, extracts flags from command:
#   3. Pulls the image, runs it with --help
#   4. Asserts every flag in command: appears in --help output
#   5. Image-specific contract checks (postgres-spock pg_hba rule count etc.)
#
# Which image. A service whose compose line carries a fallback
# (`${VAR:-<ref>}`) is checked against that fallback. A service whose line
# carries none (`${VAR:?...}`) is checked against, in order: the environment
# variable VAR; then the `fleet:` value of the pin-table row whose `var:` is
# VAR in lower case (ops/pins/fleet-pins.yaml, with `{{ ghcr_org }}` taken
# from GHCR_ORG or the default in ops/ansible/group_vars/all.yml). A tree with
# a pin table and no value for VAR fails. A tree with no pin table and VAR
# unset skips that service and says so: the pins live elsewhere, and so does
# this check.
#
# Exit 0 on all checks pass, 1 on any failure.
#
# Run locally:
#   ./ops/ci/check-pin-compat.sh
#
# Wire in CI:
#   See .github/workflows/compose-lint.yml (pin-compat job).

set -euo pipefail

COMPOSE_FILE=${COMPOSE_FILE:-ops/compose/mgmt-host/docker-compose.yml}

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "ERROR: $COMPOSE_FILE not found (run from repo root)" >&2
  exit 1
fi

for tool in python3 docker; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "ERROR: required tool '$tool' not found" >&2
    exit 1
  fi
done

# Extract services with (image, command) into newline-separated "name|image|flag1 flag2 ..." records.
parse_compose() {
  python3 - "$COMPOSE_FILE" <<'PY'
import sys
import yaml
import re
import os

path = sys.argv[1]
with open(path) as f:
    data = yaml.safe_load(f)

PIN_TABLE = "ops/pins/fleet-pins.yaml"
ALL_VARS = "ops/ansible/group_vars/all.yml"


def ghcr_org():
    org = os.environ.get("GHCR_ORG")
    if org:
        return org
    try:
        with open(ALL_VARS) as f:
            for line in f:
                if line.startswith("ghcr_org:"):
                    m = re.search(r"or\s+'([^']+)'", line)
                    return m.group(1) if m else None
    except OSError:
        return None
    return None


def pinned_image(var):
    """(image, None) for a compose variable with no fallback, else (None, why)."""
    if os.environ.get(var):
        return os.environ[var], None
    if not os.path.isfile(PIN_TABLE):
        return None, "no-table"
    with open(PIN_TABLE) as f:
        table = yaml.safe_load(f) or {}
    for cname, spec in (table.get("components") or {}).items():
        if (spec or {}).get("var") == var.lower():
            ref = spec.get("fleet")
            if not ref:
                return None, f"pin-table row {cname} has no fleet value"
            if "{{ ghcr_org }}" in ref:
                org = ghcr_org()
                if not org:
                    return None, f"cannot read ghcr_org from {ALL_VARS}; set GHCR_ORG"
                ref = ref.replace("{{ ghcr_org }}", org)
            return ref, None
    return None, f"no pin-table row has var: {var.lower()}"


services = data.get("services", {}) or {}
for name, spec in services.items():
    if not spec:
        continue
    image = spec.get("image", "")
    command = spec.get("command")
    if not image or not command:
        continue
    # Resolve ${VAR:-default} → default (the pinned fallback)
    image = re.sub(r"\$\{[^:}]+:-([^}]+)\}", r"\1", image)
    # ${VAR:?message} / ${VAR:-} carry no reference: the environment names
    # the image, else the pin table does.
    m = re.fullmatch(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::\?[^}]*|:-)?\}", image.strip())
    if m:
        var = m.group(1)
        image, why = pinned_image(var)
        if not image:
            if why == "no-table":
                print(f"SKIP|{name}|{var}")
                continue
            print(f"UNRESOLVED|{name}|{var}: {why}")
            continue
    # command can be a list or a string
    if isinstance(command, list):
        tokens = command
    else:
        tokens = command.split()
    # Extract long flags (--foo, -flag) — ignore bare args and =values
    flags = set()
    for t in tokens:
        t = str(t).strip()
        m = re.match(r"^(--?[A-Za-z0-9][A-Za-z0-9_-]*)", t)
        if m:
            flags.add(m.group(1))
    if not flags:
        continue
    print(f"{name}|{image}|{' '.join(sorted(flags))}")
PY
}

# Run image and collect --help / -h / (empty) output — whatever it offers.
get_help_text() {
  local image=$1
  # Try a set of conventional help-invocations. signal-server + pgbackrest
  # both support --help; postgres supports --help. If none work, we return
  # whatever stderr the image dumped.
  local out
  for args in "--help" "-h" "help" ""; do
    out=$(timeout 30 docker run --rm "$image" $args 2>&1 || true)
    # If we got meaningful output (>200 chars or contains a dash-prefixed flag), use it
    if [[ ${#out} -gt 200 ]] || echo "$out" | grep -qE '^\s*-[A-Za-z]'; then
      echo "$out"
      return 0
    fi
  done
  # Last resort — try --entrypoint override for images whose entrypoint is a
  # shell script that needs help-arg discovery.
  echo "$out"
}

FAIL=0

# Image-specific deep contract checks.
# Run image briefly, assert specific properties. Called after generic flag checks.
check_postgres_spock() {
  local image=$1
  echo "    postgres-spock contract: pg_hba rule count + pgbackrest presence"
  local cid
  cid=$(docker run -d --rm \
          -e POSTGRES_PASSWORD=ci-smoke \
          "$image" \
          postgres -c config_file=/etc/postgresql/postgresql.conf 2>&1)
  if [[ -z "$cid" || "$cid" =~ "Error" ]]; then
    echo "      FAIL: could not start container ($cid)"
    return 1
  fi
  # Give postgres a moment
  for _ in $(seq 1 30); do
    if docker exec "$cid" pg_isready -U postgres >/dev/null 2>&1; then break; fi
    sleep 1
  done
  local rule_count
  rule_count=$(docker exec "$cid" grep -cE '^host' /etc/postgresql/pg_hba.conf 2>&1 || echo 0)
  local has_pgbackrest
  has_pgbackrest=$(docker exec "$cid" bash -c 'command -v pgbackrest' 2>&1 || echo "")
  local has_postgres_rule
  has_postgres_rule=$(docker exec "$cid" grep -cE '^host\s+all\s+postgres\s+0\.0\.0\.0' /etc/postgresql/pg_hba.conf 2>&1 || echo 0)
  docker kill "$cid" >/dev/null 2>&1 || true

  local ok=0
  if [[ "$rule_count" -lt 10 ]]; then
    echo "      FAIL: pg_hba has $rule_count host rules, expected >= 10"
    ok=1
  else
    echo "      ok  pg_hba has $rule_count host rules"
  fi
  if [[ -z "$has_pgbackrest" ]]; then
    echo "      FAIL: pgbackrest not found in image"
    ok=1
  else
    echo "      ok  pgbackrest present at $has_pgbackrest"
  fi
  if [[ "$has_postgres_rule" -lt 1 ]]; then
    echo "      FAIL: missing 'host all postgres 0.0.0.0/0' rule — management-a will crashloop"
    ok=1
  else
    echo "      ok  'host all postgres' rule present"
  fi
  return $ok
}

# Digest-pin guard. ADR 0703: production images must be pinned by digest in
# their compose default. Tag-only refs (no `@sha256:`) regress the pin
# discipline because the underlying image can be re-tagged silently.
# The allowlist below carves out images for which a digest default is not
# achievable today (e.g. private images that get re-pushed by our own CI on
# every main commit — production deployments override the env var with a
# digest from group_vars, so the in-tree default is documented as dev-only).
# Any new entry to the allowlist needs a comment in the relevant compose
# file naming this script and explaining why a tag-only default is OK.
DIGEST_ALLOWLIST=(
  # mgmt-host-bootstrap: rebuilt by CI on every main push, and run one-shot
  # (`compose run --rm`), never long-lived. A goatnet names its own reference
  # in .env. See the compose comment in ops/compose/mgmt-host/docker-compose.yml.
  "ghcr.io/usareur-af/mgmt-host-bootstrap"

)

# Scan every *.yml in the compose directory; report tag-only image defaults
# that aren't on the allowlist.
check_digest_pins() {
  local compose_dir
  compose_dir=$(dirname "$COMPOSE_FILE")
  python3 - "$compose_dir" "${DIGEST_ALLOWLIST[@]}" <<'PY'
import os
import re
import sys
import yaml

compose_dir = sys.argv[1]
allowlist = set(sys.argv[2:])
fail = 0
seen = []

for fn in sorted(os.listdir(compose_dir)):
    if not fn.endswith(('.yml', '.yaml')):
        continue
    path = os.path.join(compose_dir, fn)
    try:
        with open(path) as f:
            data = yaml.safe_load(f) or {}
    except Exception as e:
        # archive subdirectories or inert YAML — skip silently
        continue
    services = data.get('services', {}) or {}
    for name, spec in services.items():
        if not spec:
            continue
        raw = spec.get('image') or ''
        if not raw:
            continue
        # A line with no fallback (${VAR:?message} or ${VAR:-}) names no
        # image in this tree: the pin that feeds VAR is governed where it
        # lives (ops/pins/generate-pins.py refuses a bare tag as a fleet
        # value), so there is nothing here to judge.
        if re.fullmatch(r'\$\{[A-Za-z_][A-Za-z0-9_]*(?::\?[^}]*|:-)?\}', raw.strip()):
            print(f"  none  {fn}:{name} = {raw.strip()[:40]}  (no in-tree reference)")
            continue
        # Resolve ${VAR:-default}; if no default is present we can't judge,
        # so fall back to the raw string and let the check still run on a
        # plain `image: foo:tag` line.
        m = re.search(r'\$\{[^:}]+:-([^}]+)\}', raw)
        effective = m.group(1) if m else raw
        # Strip a tag-only ref to a normalized "<repo>" for allowlist match
        repo = effective.split('@', 1)[0].rsplit(':', 1)[0]
        seen.append((fn, name, effective, repo))
        if '@sha256:' in effective:
            continue
        if repo in allowlist:
            print(f"  allow {fn}:{name} = {effective}  (allowlisted: {repo})")
            continue
        print(f"  FAIL  {fn}:{name} = {effective}  (tag-only default; ADR 0703)")
        fail = 1

print('')
print(f"  scanned {len(seen)} image refs across {len(set(s[0] for s in seen))} compose file(s)")
sys.exit(fail)
PY
}

echo ""
echo "=== digest-pin guard (ADR 0703): $(dirname "$COMPOSE_FILE")/*.yml ==="
if ! check_digest_pins; then
  FAIL=1
fi

echo ""
echo "=== pin-compat guard: $COMPOSE_FILE ==="
while IFS='|' read -r service image flags; do
  [[ -z "$service" ]] && continue
  if [[ "$service" == "SKIP" ]]; then
    echo ""
    echo "[$image] skipped: $flags is unset and this tree has no pin table"
    echo "  set $flags to check the image a goatnet pins"
    continue
  fi
  if [[ "$service" == "UNRESOLVED" ]]; then
    echo ""
    echo "[$image] FAIL: no image for $flags"
    FAIL=1
    continue
  fi
  echo ""
  echo "[$service] image=$image"
  echo "  pulling..."
  if ! docker pull -q "$image" >/dev/null 2>&1; then
    echo "  FAIL: could not pull $image"
    FAIL=1
    continue
  fi

  # Generic flag check
  if [[ -n "$flags" ]]; then
    # Read flags into an array for correct counting
    read -ra flag_arr <<< "$flags"
    echo "  flags in compose command (${#flag_arr[@]}): $flags"
    help=$(get_help_text "$image")
    missing=()
    for f in "${flag_arr[@]}"; do
      fkey="${f%=}"
      # Go's flag package shows flags as `-foo` in help but accepts both
      # `-foo` and `--foo` on CLI. Strip leading dashes and match the
      # flag name preceded by one OR two dashes at a word boundary.
      fname="${fkey##--}"
      fname="${fname##-}"
      pattern=$(printf '%s' "$fname" | sed 's/[.[\*^$()+?{}|]/\\&/g')
      # Here-string instead of `echo | grep` — under `set -o pipefail`,
      # `grep -q` exits as soon as it finds a match, sending SIGPIPE to
      # echo. Bash's pipefail then surfaces echo's broken-pipe exit as
      # the pipeline status, which flips `if !` and falsely reports the
      # flag as missing. Race is timing-dependent on `$help` size; bit
      # the traefik image (large help output) .
      if ! grep -qE "(^|[^A-Za-z0-9_-])-{1,2}${pattern}([^A-Za-z0-9_-]|$)" <<< "$help"; then
        missing+=("$fkey")
      fi
    done
    if (( ${#missing[@]} > 0 )); then
      echo "  FAIL: flags not recognized by image --help: ${missing[*]}"
      FAIL=1
    else
      echo "  ok   all ${#flag_arr[@]} flag(s) recognized"
    fi
  fi

  # Image-specific contract checks
  case "$image" in
    *postgres-spock*)
      check_postgres_spock "$image" || FAIL=1
      ;;
  esac
done < <(parse_compose)

echo ""
if (( FAIL )); then
  echo "=== pin-compat guard: FAIL ==="
  exit 1
fi
echo "=== pin-compat guard: PASS ==="

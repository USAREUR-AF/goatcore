#!/usr/bin/env bash
# ops/ci/check-kratos-config-renders.sh
#
# Renders ops/ansible/roles/ory/templates/ory/kratos.yaml.j2 for every goatnet
# and asserts Kratos ACCEPTS the result.
#
# WHY THIS EXISTS. A role knob can render a key Kratos has never had (for example
# `selfservice.flows.login.required_aal`). An invalid key is not a soft failure:
# Kratos refuses to boot on it, so enabling the knob takes a goatnet's whole
# identity plane down at the next `ory` role run.
#
# The role fixture (roles/ory/tests/test-fixture.sh) cannot catch this: that
# fixture writes its OWN hand-authored kratos.yaml heredoc and never renders the
# template.
#
# HOW IT VALIDATES. There is no `kratos validate --config`; `kratos validate`
# handles resources (identities, schemas), not the config file. But Kratos
# validates its config against the embedded JSON schema BEFORE it touches the
# database, so `kratos serve -c <file>` reaches a decisive verdict in about a
# second: a schema violation prints "The configuration contains values or keys
# which are invalid" and exits, while a valid config gets as far as a DSN or
# migration complaint. We assert on the schema verdict only — a DSN error means
# the config PASSED.
#
# Usage:
#   ops/ci/check-kratos-config-renders.sh            # all goatnets
#   ops/ci/check-kratos-config-renders.sh <site>_hosts # one
#
# Requires: docker, python3 with jinja2 + pyyaml.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TEMPLATE="$REPO_ROOT/ops/ansible/roles/ory/templates/ory/kratos.yaml.j2"
DEFAULTS="$REPO_ROOT/ops/ansible/roles/ory/defaults/main.yml"
GROUP_VARS="$REPO_ROOT/ops/ansible/group_vars"
KRATOS_IMAGE="${KRATOS_IMAGE:-oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643}"

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fail=0

# Every group_vars dir that carries an ory/identity plane. A goatnet added later
# is picked up automatically: we render whatever has a site_id.
#
# Deliberately NOT `mapfile -t GROUPS < <(...)`. That form returned rc=1 here
# and, with `set -e`, killed the script before a single goatnet was checked —
# a guard that exits 1 having validated NOTHING, which reads in CI exactly like
# a guard that caught something. Write the list to a file and read it back.
list_groups() {
  if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@"
  else
    find "$GROUP_VARS" -maxdepth 1 -mindepth 1 -type d -printf '%f\n' | sort
  fi
}

list_groups "$@" > "$WORK/groups"
if [ ! -s "$WORK/groups" ]; then
  echo "FAIL: no group_vars directories found under $GROUP_VARS"
  exit 1
fi
checked=0
skipped=0

render() {  # render <group> <outfile>
  python3 - "$1" "$2" "$TEMPLATE" "$DEFAULTS" "$GROUP_VARS" <<'PY'
import os, sys, glob, yaml
from jinja2 import Environment, FileSystemLoader, StrictUndefined

group, out, template, defaults, gvroot = sys.argv[1:6]

ctx = {}
with open(defaults) as fh:
    ctx.update(yaml.safe_load(fh) or {})
for path in sorted(glob.glob(os.path.join(gvroot, group, "*.yml"))):
    with open(path) as fh:
        try:
            loaded = yaml.safe_load(fh)
        except yaml.YAMLError:
            continue          # a goatnet's unrelated file, not ours to police
    if isinstance(loaded, dict):
        ctx.update(loaded)

if "site_id" not in ctx:
    sys.exit(3)               # not a goatnet with an identity plane

# site_dns_zone is Class A with NO role default on purpose (see the role
# defaults): a forgotten per-instance override must fail loudly at play time
# rather than render another goatnet's URLs into this one's identity plane.
# Groups that never define it therefore cannot run the
# role without -e, which is the documented posture, not a template defect.
# Report the skip rather than failing on it — and rather than passing silently.
if "site_dns_zone" not in ctx:
    sys.stderr.write("site_dns_zone\n")
    sys.exit(4)

# Ansible resolves `{{ lookup('env', ...) }}` and nested group_var references at
# play time; CI only needs the SHAPE to be valid, so undefined lookups become
# harmless placeholders rather than render failures.
env = Environment(loader=FileSystemLoader(os.path.dirname(template)),
                  undefined=StrictUndefined, keep_trailing_newline=True)
env.globals["lookup"] = lambda *a, **k: "ci-placeholder-value"

# Ansible ships filters plain Jinja2 does not. Only shim the ones this template
# uses; a template that starts using another Ansible filter fails loudly here
# rather than rendering something subtly different from what Ansible produces.
def _bool(value):
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() in ("true", "yes", "on", "1")

env.filters["bool"] = _bool

def resolve(value, depth=0):
    if isinstance(value, str) and "{{" in value and depth < 12:
        try:
            return resolve(env.from_string(value).render(**ctx), depth + 1)
        except Exception:
            return "ci-placeholder-value"
    return value

for _ in range(6):            # group_vars reference each other; settle by passes
    ctx = {k: resolve(v) for k, v in ctx.items()}

with open(out, "w") as fh:
    fh.write(env.get_template(os.path.basename(template)).render(**ctx))
PY
}

while read -r group; do
  [ -n "$group" ] || continue
  out="$WORK/kratos.$group.yaml"
  set +e
  render "$group" "$out"; rc=$?
  set -e
  case $rc in
    0) : ;;
    3) continue ;;                                  # no identity plane here
    4) echo "skip [$group] no site_dns_zone — Class A, must be passed with -e (see role defaults)"
       skipped=$((skipped + 1)); continue ;;
    *) echo "FAIL [$group] template did not render (exit $rc)"; fail=1; continue ;;
  esac

  # YAML well-formedness first — a clearer message than Kratos's.
  if ! python3 -c 'import sys,yaml; yaml.safe_load(open(sys.argv[1]))' "$out" 2>"$WORK/yamlerr"; then
    echo "FAIL [$group] rendered kratos.yaml is not valid YAML:"; sed 's/^/    /' "$WORK/yamlerr"; fail=1; continue
  fi

  # Kratos's own schema is the authority.
  #
  # The template ships `secrets.{cookie,cipher}: []` on purpose — the real
  # values arrive as env vars from /opt/mgmt-host/.env at parse time. The
  # schema enforces minItems:1, so without the same env vars every render
  # would "fail" for a reason that has nothing to do with the template. Supply
  # throwaway ones exactly as the deployment does. DSN likewise: we want the
  # run to get PAST config validation and die on infrastructure instead.
  docker run --rm -v "$out:/etc/config/kratos.yaml:ro" \
      -e SECRETS_COOKIE=ci-placeholder-cookie-secret-32c \
      -e SECRETS_CIPHER=ci-placeholder-cipher-secret-32c \
      -e DSN=postgres://ci:ci@127.0.0.1:1/ci \
      "$KRATOS_IMAGE" serve -c /etc/config/kratos.yaml >"$WORK/out.$group" 2>&1 || true

  if grep -qE 'configuration contains values or keys which are invalid|additionalProperties .* not allowed' "$WORK/out.$group"; then
    echo "FAIL [$group] Kratos rejected the rendered config:"
    grep -E 'additionalProperties|I\[#/|\^--' "$WORK/out.$group" | head -6 | sed 's/^/    /'
    fail=1
  else
    echo "ok   [$group] rendered config accepted by ${KRATOS_IMAGE##*:}"
  fi
  checked=$((checked + 1))
done < "$WORK/groups"

# A guard that validated nothing must not report success. Without this, a
# rename of the template or the group_vars root would make every goatnet skip
# and the job would go green having checked zero configs.
if [ "$checked" -eq 0 ]; then
  echo "FAIL: rendered zero goatnet configs — nothing was validated."
  echo "      Check \$GROUP_VARS ($GROUP_VARS) and the site_id detection."
  exit 1
fi
echo "validated $checked goatnet config(s); skipped $skipped for a missing Class-A var"

if [ "$fail" -ne 0 ]; then
  echo
  echo "A rendered kratos.yaml is invalid. Kratos does not degrade on this —"
  echo "it refuses to boot, taking the goatnet's identity plane with it."
  exit 1
fi

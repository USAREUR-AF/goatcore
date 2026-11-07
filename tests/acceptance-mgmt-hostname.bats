#!/usr/bin/env bats

# Acceptance tests for NetBird management hostname resolution.
#
# Verifies that the acceptance catalog rows for the NetBird mgmt API
# and signal endpoints resolve to the per-env public_mgmt_fqdn, and
# that the names-collector profile template does not embed a doubled
# "netbird." label when public_mgmt_fqdn is present or absent.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

get_python_with_jinja2() {
  # Prefer system python3 if it has jinja2.
  if command -v python3 >/dev/null 2>&1; then
    if python3 - <<'PY' >/dev/null 2>&1
import importlib
try:
    import jinja2  # type: ignore[import]
except Exception:
    raise SystemExit(1)
PY
    then
      echo python3
      return 0
    fi
  fi

  # Fallback: use the interpreter backing ansible-playbook.
  if command -v ansible-playbook >/dev/null 2>&1; then
    local ap shebang python_path
    ap="$(command -v ansible-playbook)"
    shebang="$(head -n1 "$ap" 2>/dev/null || true)"
    if [[ "$shebang" == '#!'* ]]; then
      python_path="${shebang:2}"
      if "$python_path" - <<'PY' >/dev/null 2>&1
try:
    import jinja2  # type: ignore[import]
except Exception:
    raise SystemExit(1)
PY
      then
        echo "$python_path"
        return 0
      fi
    fi
  fi

  return 1
}

@test "acceptance rows for NetBird mgmt host resolve to public_mgmt_fqdn" {
  # Reads each goatnet's generated values file; a tree without them (a public
  # goatcore checkout) has nothing to judge, and the test runs where they are.
  [ -f "$REPO_ROOT/ops/validation/values-zagros.yaml" ] || skip "no goatnet values files in this tree"
  cd "$REPO_ROOT"

  run python3 - <<'PY'
import importlib.util
import pathlib
import sys

try:
    import yaml  # type: ignore[import]
except Exception as e:  # pragma: no cover - environment guard
    print(f"PyYAML not available: {e}")
    sys.exit(1)

root = pathlib.Path(".")
runner_path = root / "ops" / "validation" / "run-acceptance.py"
spec = importlib.util.spec_from_file_location("run_acceptance", runner_path)
module = importlib.util.module_from_spec(spec)
assert spec and spec.loader, "failed to load run-acceptance.py"
spec.loader.exec_module(module)

resolve = module.resolve
load_values = module.load_values

manifest_path = root / "ops" / "validation" / "acceptance-tests.yaml"
with manifest_path.open() as f:
    manifest = yaml.safe_load(f) or {}

rows = {row["id"]: row for row in manifest.get("tests", []) if isinstance(row, dict) and "id" in row}

test_ids = [
    "mgmt-api-answers",
    "signal-tls-not-cleartext",
    "mgmt-api-refuses-an-unauthenticated-caller",
]

value_files = {
    "ops/validation/values-zagros.yaml": "zagros",
    "ops/validation/values-zagros-dev.yaml": "zagros-dev",
    "ops/validation/values-zagros-prod.yaml": "zagros-prod",
}

failures = []

for values_path, env in value_files.items():
    values = load_values(env, values_path, overrides=None)
    public_mgmt_fqdn = values.get("public_mgmt_fqdn")
    if not public_mgmt_fqdn:
        failures.append(f"{values_path}: missing public_mgmt_fqdn")
        continue

    for test_id in test_ids:
        row = rows.get(test_id)
        if row is None:
            failures.append(f"{values_path}:{test_id}: row not found in manifest")
            continue

        params = row.get("params", {})
        resolved = resolve(params, values)

        host = None
        if "host" in resolved:
            host = resolved["host"]
        elif "url" in resolved:
            host = resolved["url"]

        if host is None:
            failures.append(f"{values_path}:{test_id}: missing host/url in resolved params")
            continue

        if "netbird.netbird." in str(host):
            failures.append(f"{values_path}:{test_id}: doubled netbird label in {host!r}")
        if "<UNRESOLVED:" in str(host):
            failures.append(f"{values_path}:{test_id}: unresolved sentinel in {host!r}")

        # For the http rows the host is the public management FQDN itself;
        # for the URL rows, enforce that the hostname component equals it.
        from urllib.parse import urlparse

        h = str(host)
        if h.startswith("http://") or h.startswith("https://"):
            parsed = urlparse(h)
            hostname = parsed.hostname
        else:
            hostname = h

        if hostname != public_mgmt_fqdn:
            failures.append(
                f"{values_path}:{test_id}: {hostname!r} != public_mgmt_fqdn {public_mgmt_fqdn!r}"
            )

if failures:
    print("\n".join(failures))
    sys.exit(1)
PY

  printf '%s\n' "$output" >&2
  [ "$status" -eq 0 ]
}

@test "names-collector profile portal_endpoint uses public_mgmt_fqdn or site_dns_zone without doubled netbird label" {
  cd "$REPO_ROOT"

  local python_cmd
  if ! python_cmd="$(get_python_with_jinja2)"; then
    skip "No python interpreter with jinja2 available"
  fi

  run "$python_cmd" - <<'PY'
import pathlib
import sys

from jinja2 import Environment, FileSystemLoader  # type: ignore[import]

template_dir = pathlib.Path("ops/ansible/roles/names-collector/templates")
env = Environment(loader=FileSystemLoader(str(template_dir)))
template = env.get_template("profile.toml.j2")

site_id = "zagros-backbone"
public_mgmt_fqdn = "netbird.zagros.example"

rendered_with = template.render(site_id=site_id, public_mgmt_fqdn=public_mgmt_fqdn)
rendered_without = template.render(site_id=site_id, site_dns_zone="netbird.zagros.example")

def get_portal_endpoint(rendered: str) -> str | None:
    for line in rendered.splitlines():
        if line.strip().startswith("portal_endpoint"):
            try:
                return line.split("=", 1)[1].strip().strip('"')
            except Exception:
                return None
    return None

endpoint_with = get_portal_endpoint(rendered_with)
endpoint_without = get_portal_endpoint(rendered_without)

errors = []

if endpoint_with is None:
    errors.append("portal_endpoint line not found in rendered_with")
else:
    expected = f"https://portal.{site_id}.{public_mgmt_fqdn}"
    if endpoint_with != expected:
        errors.append(
            f"portal_endpoint with public_mgmt_fqdn is {endpoint_with!r}, expected {expected!r}"
        )
    if "netbird.netbird." in endpoint_with:
        errors.append(f"doubled netbird label in endpoint_with: {endpoint_with!r}")
    if "<UNRESOLVED:" in endpoint_with:
        errors.append(f"unresolved sentinel in endpoint_with: {endpoint_with!r}")

if endpoint_without is None:
    errors.append("portal_endpoint line not found in rendered_without")
else:
    if "netbird.netbird." in endpoint_without:
        errors.append(f"doubled netbird label in endpoint_without: {endpoint_without!r}")
    if "<UNRESOLVED:" in endpoint_without:
        errors.append(f"unresolved sentinel in endpoint_without: {endpoint_without!r}")

if errors:
    print("\n".join(errors))
    sys.exit(1)
PY

  printf '%s\n' "$output" >&2
  [ "$status" -eq 0 ]
}

@test "grep guard: no doubled netbird label against site_dns_zone in acceptance or names-collector" {
  cd "$REPO_ROOT"

  run grep -rnE 'netbird\.\{\{ *site_dns_zone' \
    ops/validation \
    ops/ansible/roles/names-collector

  printf '%s\n' "$output" >&2
  # grep must find no matches; a zero exit status here means the doubled
  # label is still present somewhere under the guarded paths.
  [ "$status" -ne 0 ]
}

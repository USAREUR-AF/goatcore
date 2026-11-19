#!/usr/bin/env bash
# ops/ci/check-bare-portal-http-guards.sh — additive-only ratchet on bare
# `PortalError::Http { status }` match arms in goat-cli (PR #365).
#
# WHY: panoscope-core's PortalError carries an HTTP refusal in two shapes,
# `Http { status }` and `HttpWithBody { status, body }`. A mapper that matches
# the bare variant by name handles only the first, so when the portal answers
# with a body the arm is skipped and the operator loses the server's own
# reason for the refusal. The rail is to dispatch on `e.http_status()`, which
# answers for both shapes, and to render the server's sentence with
# `server_line(&e)`. The arms that predate the rail are grandfathered per
# file below; the count may only go DOWN. A file above its baseline fails the
# build, and a NEW file with any bare arm fails at baseline 0.
#
# When a PR migrates an arm, lower that file's baseline here in the same PR.
# Constructors in tests (`Http { status: StatusCode::X }`) and the fallback
# arm `Http { .. } | HttpWithBody { .. }` are not matched: only a status-bound
# bare arm is the pattern this guard exists to stop.
#
# Doctrine P6: `--self-test` proves the guard goes RED on a fixture before its
# verdict on the real tree is trusted. Plain bash 3.2: no associative arrays
# (ops/ci/check-bash-portability.sh).
set -euo pipefail

PATTERN='PortalError::Http[[:space:]]*\{[[:space:]]*status([[:space:]]*,[[:space:]]*\.\.)?[[:space:]]*\}'
SCAN_DIR='crates/goat-cli/src'

# One row per grandfathered file: <path> <baseline>. Every other file is 0.
BASELINE='
crates/goat-cli/src/cmd/admin.rs 14
crates/goat-cli/src/cmd/access.rs 4
crates/goat-cli/src/cmd/replay.rs 3
crates/goat-cli/src/cmd/platform_admin.rs 1
crates/goat-cli/src/cmd/doctor.rs 0
'

baseline_for() {
  local n
  n="$(printf '%s\n' "${BASELINE}" | awk -v f="$1" '$1 == f { print $2 }')"
  printf '%s' "${n:-0}"
}

count_in() {
  if [ ! -f "$1" ]; then echo 0; return; fi
  # Rust's current variant has more fields, so match arms use `status, ..`.
  # Keep the older shape too, and exempt an explicit pair covering both
  # variants. This remains a line-oriented ratchet, not a Rust parser.
  python3 - "$1" <<'PY'
import pathlib, re, sys
bare = re.compile(r'PortalError::Http\s*\{\s*status(?:\s*,\s*\.\.)?\s*\}')
paired_after = re.compile(r'\s*\)?\s*\|\s*(?:Err\s*\()?\s*(?:\w+::)*PortalError::HttpWithBody\s*\{')
paired_before = re.compile(r'PortalError::HttpWithBody\s*\{[^{}]*\}\s*\)?\s*\|\s*(?:Err\s*\()?\s*(?:\w+::)*$')
count = 0
for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
    line = re.sub(r'"(?:\\.|[^"\\])*"', '""', line).split('//', 1)[0]
    for match in bare.finditer(line):
        if not paired_after.match(line[match.end():]) and not paired_before.search(line[:match.start()]):
            count += 1
print(count)
PY
}

scan() {
  local root="$1"
  local fail=0
  local file rel n base files summary=""

  if [ ! -d "${root}/${SCAN_DIR}" ]; then
    echo "FAIL: missing Rust source directory: ${SCAN_DIR}" >&2
    return 2
  fi
  files="$(find "${root}/${SCAN_DIR}" -type f -name '*.rs')" || return 2
  if [ -z "${files}" ]; then
    echo "FAIL: no Rust source files checked in ${SCAN_DIR}" >&2
    return 2
  fi

  while IFS= read -r file; do
    [ -n "${file}" ] || continue
    rel="${file#"${root}"/}"
    n="$(count_in "${file}")" || return 2
    base="$(baseline_for "${rel}")"
    if [ "${n}" -gt "${base}" ]; then
      echo "FAIL: ${rel} has ${n} bare \`PortalError::Http { status }\` arm(s); baseline is ${base}." >&2
      grep -n -E "${PATTERN}" "${file}" | sed 's/^/      /' >&2
      echo "      A bare arm handles \`Http\` and misses \`HttpWithBody\`, so the operator loses the" >&2
      echo "      server's reason. Dispatch on the status instead and render the server line:" >&2
      echo "        match e.http_status().map(|s| s.as_u16()) { Some(403) => ..., _ => ... }" >&2
      echo "        ... server_line(&e) ..." >&2
      echo "      (PR #365). Migrating an existing arm: lower that file's baseline in" >&2
      echo "      ops/ci/check-bare-portal-http-guards.sh in the same PR." >&2
      fail=1
    fi
  done <<<"${files}"

  while read -r rel base; do
    [ -n "${rel}" ] || continue
    summary="${summary} ${rel##*/}=$(count_in "${root}/${rel}")/${base}"
  done <<<"${BASELINE}"

  if [ "${fail}" -eq 0 ]; then
    echo "ok: bare PortalError::Http arms within baseline:${summary}"
  fi
  return "${fail}"
}

if [ "${1:-}" = "--self-test" ]; then
  fixture="$(mktemp -d)"
  trap 'rm -rf "${fixture}"' EXIT
  mkdir -p "${fixture}/${SCAN_DIR}/cmd"
  arm='        Err(PortalError::Http { status, .. }) if status.as_u16() == 409 => {}'

  # Case 1: a grandfathered file one arm over its baseline goes RED.
  : > "${fixture}/${SCAN_DIR}/cmd/admin.rs"
  for _ in $(seq 15); do echo "${arm}" >> "${fixture}/${SCAN_DIR}/cmd/admin.rs"; done
  if out="$(scan "${fixture}" 2>&1)"; then
    echo "SELF-TEST FAIL: an over-baseline file passed" >&2; exit 2
  else
    rc=$?
  fi
  if [ "${rc}" -ne 1 ]; then echo "SELF-TEST FAIL: scan did not return refusal status" >&2; exit 2; fi
  case "$out" in
    *"admin.rs has 15 bare"*) ;;
    *) echo "SELF-TEST FAIL: a file over its baseline did not fire" >&2; echo "$out" >&2; exit 2 ;;
  esac

  # Case 2: the same file at its baseline is GREEN.
  : > "${fixture}/${SCAN_DIR}/cmd/admin.rs"
  for _ in $(seq 14); do echo "${arm}" >> "${fixture}/${SCAN_DIR}/cmd/admin.rs"; done
  if ! scan "${fixture}" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: a file at its baseline was refused" >&2; exit 2
  fi

  # Case 3: a new file with a single bare arm goes RED at baseline 0.
  echo "${arm}" > "${fixture}/${SCAN_DIR}/cmd/new_verb.rs"
  if out="$(scan "${fixture}" 2>&1)"; then
    echo "SELF-TEST FAIL: a new bare arm passed" >&2; exit 2
  else
    rc=$?
  fi
  if [ "${rc}" -ne 1 ]; then echo "SELF-TEST FAIL: scan did not return refusal status" >&2; exit 2; fi
  case "$out" in
    *"new_verb.rs has 1 bare"*) ;;
    *) echo "SELF-TEST FAIL: a new file with a bare arm did not fire" >&2; echo "$out" >&2; exit 2 ;;
  esac

  # Case 4: a test constructor and the fallback arm are not counted.
  printf '%s\n' 'PortalError::Http { status: StatusCode::FORBIDDEN }' \
    'PortalError::Http { status, .. } | PortalError::HttpWithBody { status, .. } => {}' \
    'PortalError::HttpWithBody { status, .. } | PortalError::Http { status, .. } => {}' \
    '// PortalError::Http { status, .. } if status.as_u16() == 409 => {}' \
    'let text = "PortalError::Http { status, .. }";' \
    > "${fixture}/${SCAN_DIR}/cmd/new_verb.rs"
  if ! scan "${fixture}" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: a constructor or the fallback arm was counted as bare" >&2; exit 2
  fi

  # Case 5: keep detecting the older status-only shape too.
  echo 'Err(PortalError::Http { status }) if status.as_u16() == 409 => {}' > "${fixture}/${SCAN_DIR}/cmd/new_verb.rs"
  if scan "${fixture}" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: the older bare shape was not refused" >&2; exit 2
  fi

  # No source is missing coverage, not a successful zero-match scan.
  if scan "${fixture}/missing" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: missing source was accepted" >&2; exit 2
  else
    rc=$?
  fi
  if [ "${rc}" -ne 2 ]; then echo "SELF-TEST FAIL: missing source returned the wrong status" >&2; exit 2; fi
  mkdir -p "${fixture}/empty/${SCAN_DIR}"
  if scan "${fixture}/empty" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: empty source was accepted" >&2; exit 2
  else
    rc=$?
  fi
  if [ "${rc}" -ne 2 ]; then echo "SELF-TEST FAIL: empty source returned the wrong status" >&2; exit 2; fi

  echo "OK: self-test — guard goes RED over baseline and on a new file, GREEN at baseline and on non-bare shapes."
  exit 0
fi

scan "$(git rev-parse --show-toplevel)"
